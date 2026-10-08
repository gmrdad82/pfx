use std::ops::Range;

use wasm_encoder::reencode::{Error, Reencode, RoundtripReencoder};
use wasm_encoder::{
    CodeSection, ComponentSectionId, ConstExpr, Function, FunctionSection, GlobalSection,
    GlobalType, Instruction, Module, RawSection, SectionId, TypeSection, ValType,
};
use wasmparser::{Chunk, FunctionBody, Operator, Parser, Payload, TypeRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Site {
    pub func: u32,
    pub offset: usize,
}

pub(crate) struct Instrumented {
    pub bytes: Vec<u8>,
    pub sites: Vec<Site>,
}

pub(crate) fn instrument(component: &[u8], limit: u32) -> Result<Instrumented, String> {
    let mut found = Vec::new();
    let bytes = walk(component, limit, &mut found)?;
    let mut starts = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::ModuleSection {
            unchecked_range, ..
        } = payload.map_err(|e| e.to_string())?
        {
            starts.push(unchecked_range.start as usize);
        }
    }
    if starts.len() != found.len() {
        return Err("the instrumented component lost a module".to_string());
    }
    let mut sites: Vec<Site> = found
        .into_iter()
        .zip(starts)
        .filter_map(|(site, start)| {
            site.map(|site| Site {
                func: site.func,
                offset: start + site.offset,
            })
        })
        .collect();
    sites.sort();
    sites.dedup();
    Ok(Instrumented { bytes, sites })
}

fn span(range: Range<u64>, len: usize) -> Result<Range<usize>, String> {
    let range = range.start as usize..range.end as usize;
    if range.end > len || range.start > range.end {
        return Err("a section runs past the end of the component".to_string());
    }
    Ok(range)
}

fn walk(bytes: &[u8], limit: u32, sites: &mut Vec<Option<Site>>) -> Result<Vec<u8>, String> {
    let mut out = wasm_encoder::Component::new();
    let mut parser = Parser::new(0);
    let mut at = 0;
    loop {
        let (payload, consumed) = match parser
            .parse(&bytes[at..], true)
            .map_err(|e| e.to_string())?
        {
            Chunk::Parsed { payload, consumed } => (payload, consumed),
            Chunk::NeedMoreData(_) => return Err("the component is cut short".to_string()),
        };
        at += consumed;
        match payload {
            Payload::Version { .. } => {}
            Payload::End(_) => break,
            Payload::ModuleSection {
                unchecked_range, ..
            } => {
                let range = span(unchecked_range, bytes.len())?;
                let module = module(&bytes[range.clone()], limit, sites)?;
                out.section(&RawSection {
                    id: ComponentSectionId::CoreModule as u8,
                    data: &module,
                });
                at += range.len();
            }
            Payload::ComponentSection {
                unchecked_range, ..
            } => {
                let range = span(unchecked_range, bytes.len())?;
                let nested = walk(&bytes[range.clone()], limit, sites)?;
                out.section(&RawSection {
                    id: ComponentSectionId::Component as u8,
                    data: &nested,
                });
                at += range.len();
            }
            other => {
                if let Some((id, range)) = other.as_section() {
                    let range = span(range, bytes.len())?;
                    out.section(&RawSection {
                        id,
                        data: &bytes[range],
                    });
                }
            }
        }
    }
    Ok(out.finish())
}

#[derive(Default)]
struct Scan {
    params: Vec<u32>,
    imported_funcs: u32,
    imported_globals: u32,
    funcs: Vec<u32>,
    globals: u32,
    has_globals: bool,
}

fn scan(bytes: &[u8]) -> Result<Scan, String> {
    let mut scan = Scan::default();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(|e| e.to_string())? {
            Payload::TypeSection(reader) => {
                for ty in reader.into_iter_err_on_gc_types() {
                    scan.params
                        .push(ty.map_err(|e| e.to_string())?.params().len() as u32);
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    match import.map_err(|e| e.to_string())?.ty {
                        TypeRef::Func(_) | TypeRef::FuncExact(_) => scan.imported_funcs += 1,
                        TypeRef::Global(_) => scan.imported_globals += 1,
                        _ => {}
                    }
                }
            }
            Payload::FunctionSection(reader) => {
                for ty in reader {
                    scan.funcs.push(ty.map_err(|e| e.to_string())?);
                }
            }
            Payload::GlobalSection(reader) => {
                scan.has_globals = true;
                scan.globals = reader.count();
            }
            _ => {}
        }
    }
    Ok(scan)
}

struct Depth {
    limit: u32,
    global: u32,
    trap_type: u32,
    trap_func: u32,
    params: Vec<u32>,
    funcs: Vec<u32>,
    next: usize,
    has_globals: bool,
}

type Fail = Error<std::convert::Infallible>;

fn rank(id: SectionId) -> u8 {
    match id {
        SectionId::Type => 1,
        SectionId::Import => 2,
        SectionId::Function => 3,
        SectionId::Table => 4,
        SectionId::Memory => 5,
        SectionId::Tag => 6,
        SectionId::Global => 7,
        SectionId::Export => 8,
        SectionId::Start => 9,
        SectionId::Element => 10,
        SectionId::DataCount => 11,
        SectionId::Code => 12,
        SectionId::Data => 13,
        _ => 0,
    }
}

impl Depth {
    fn depth_global(&self, globals: &mut GlobalSection) {
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
    }

    fn leave(&self, f: &mut Function) {
        f.instruction(&Instruction::GlobalGet(self.global));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::GlobalSet(self.global));
    }

    fn leave_if(&self, f: &mut Function) {
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        self.leave(f);
        f.instruction(&Instruction::End);
    }

    fn enter(&self, f: &mut Function) {
        f.instruction(&Instruction::GlobalGet(self.global));
        f.instruction(&Instruction::I32Const(self.limit as i32));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Call(self.trap_func));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::GlobalGet(self.global));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::GlobalSet(self.global));
    }

    fn body(&mut self, body: &FunctionBody<'_>) -> Result<Function, Fail> {
        let ty = self.funcs.get(self.next).copied().unwrap_or(0);
        self.next += 1;
        let mut scratch = self.params.get(ty as usize).copied().unwrap_or(0);
        let mut locals = Vec::new();
        for pair in body.get_locals_reader()? {
            let (count, ty) = pair?;
            scratch += count;
            locals.push((count, RoundtripReencoder.val_type(ty)?));
        }
        locals.push((1, ValType::I32));
        let mut f = Function::new(locals);
        self.enter(&mut f);
        let mut open = 0u32;
        let mut reader = body.get_operators_reader()?;
        while !reader.eof() {
            let op = reader.read()?;
            match &op {
                Operator::Block { .. }
                | Operator::Loop { .. }
                | Operator::If { .. }
                | Operator::TryTable { .. }
                | Operator::Try { .. } => open += 1,
                Operator::End | Operator::Delegate { .. } => {
                    if open == 0 {
                        self.leave(&mut f);
                    } else {
                        open -= 1;
                    }
                }
                Operator::Return
                | Operator::ReturnCall { .. }
                | Operator::ReturnCallIndirect { .. }
                | Operator::ReturnCallRef { .. } => self.leave(&mut f),
                Operator::Br { relative_depth } if *relative_depth == open => {
                    self.leave(&mut f);
                }
                Operator::BrIf { relative_depth } if *relative_depth == open => {
                    f.instruction(&Instruction::LocalTee(scratch));
                    self.leave_if(&mut f);
                    f.instruction(&Instruction::LocalGet(scratch));
                }
                Operator::BrTable { targets } => {
                    let mut outs = Vec::new();
                    for (index, target) in targets.targets().enumerate() {
                        if target? == open {
                            outs.push(index as u32);
                        }
                    }
                    let default = targets.default() == open;
                    if default || !outs.is_empty() {
                        f.instruction(&Instruction::LocalSet(scratch));
                        f.instruction(&Instruction::I32Const(0));
                        for index in outs {
                            f.instruction(&Instruction::LocalGet(scratch));
                            f.instruction(&Instruction::I32Const(index as i32));
                            f.instruction(&Instruction::I32Eq);
                            f.instruction(&Instruction::I32Or);
                        }
                        if default {
                            f.instruction(&Instruction::LocalGet(scratch));
                            f.instruction(&Instruction::I32Const(targets.len() as i32));
                            f.instruction(&Instruction::I32GeU);
                            f.instruction(&Instruction::I32Or);
                        }
                        self.leave_if(&mut f);
                        f.instruction(&Instruction::LocalGet(scratch));
                    }
                }
                _ => {}
            }
            f.instruction(&RoundtripReencoder.instruction(op)?);
        }
        Ok(f)
    }
}

impl Reencode for Depth {
    type Error = std::convert::Infallible;

    fn parse_type_section(
        &mut self,
        types: &mut TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), Fail> {
        wasm_encoder::reencode::utils::parse_type_section(self, types, section)?;
        types.ty().function([], []);
        Ok(())
    }

    fn parse_function_section(
        &mut self,
        functions: &mut FunctionSection,
        section: wasmparser::FunctionSectionReader<'_>,
    ) -> Result<(), Fail> {
        wasm_encoder::reencode::utils::parse_function_section(self, functions, section)?;
        functions.function(self.trap_type);
        Ok(())
    }

    fn parse_global_section(
        &mut self,
        globals: &mut GlobalSection,
        section: wasmparser::GlobalSectionReader<'_>,
    ) -> Result<(), Fail> {
        wasm_encoder::reencode::utils::parse_global_section(self, globals, section)?;
        self.depth_global(globals);
        Ok(())
    }

    fn parse_code_section(
        &mut self,
        code: &mut CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), Fail> {
        wasm_encoder::reencode::utils::parse_code_section(self, code, section)?;
        let mut trap = Function::new([]);
        trap.instruction(&Instruction::Unreachable);
        trap.instruction(&Instruction::End);
        code.function(&trap);
        Ok(())
    }

    fn parse_function_body(
        &mut self,
        code: &mut CodeSection,
        func: FunctionBody<'_>,
    ) -> Result<(), Fail> {
        let f = self.body(&func)?;
        code.function(&f);
        Ok(())
    }

    fn intersperse_section_hook(
        &mut self,
        module: &mut Module,
        after: Option<SectionId>,
        before: Option<SectionId>,
    ) -> Result<(), Fail> {
        let global = rank(SectionId::Global);
        let past = after.is_none_or(|after| rank(after) < global);
        let ahead = before.is_none_or(|before| rank(before) > global);
        if !self.has_globals && past && ahead {
            let mut globals = GlobalSection::new();
            self.depth_global(&mut globals);
            module.section(&globals);
            self.has_globals = true;
        }
        Ok(())
    }
}

fn module(bytes: &[u8], limit: u32, sites: &mut Vec<Option<Site>>) -> Result<Vec<u8>, String> {
    let scan = scan(bytes)?;
    if scan.funcs.is_empty() {
        sites.push(None);
        return Ok(bytes.to_vec());
    }
    let trap_func = scan.imported_funcs + scan.funcs.len() as u32;
    let mut depth = Depth {
        limit,
        global: scan.imported_globals + scan.globals,
        trap_type: scan.params.len() as u32,
        trap_func,
        params: scan.params,
        funcs: scan.funcs,
        next: 0,
        has_globals: scan.has_globals,
    };
    let mut out = Module::new();
    depth
        .parse_core_module(&mut out, Parser::new(0), bytes)
        .map_err(|e| e.to_string())?;
    let out = out.finish();
    sites.push(Some(Site {
        func: trap_func,
        offset: trap_offset(&out)?,
    }));
    Ok(out)
}

fn trap_offset(module: &[u8]) -> Result<usize, String> {
    let mut last = None;
    for payload in Parser::new(0).parse_all(module) {
        if let Payload::CodeSectionEntry(body) = payload.map_err(|e| e.to_string())? {
            last = Some(body);
        }
    }
    let body = last.ok_or_else(|| "the instrumented module lost its code".to_string())?;
    let mut reader = body.get_operators_reader().map_err(|e| e.to_string())?;
    let offset = reader.original_position();
    reader.read().map_err(|e| e.to_string())?;
    Ok(offset as usize)
}
