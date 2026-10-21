use pfx_gpu::Gpu;

pub const KINDS: usize = 10;
pub const NAMES: [&str; KINDS] = [
    "PATHS",
    "PRIMARY",
    "BOUNCE",
    "CLOSEST",
    "SHADOW",
    "SURFACE_SHADOW",
    "CLOSEST_CANDIDATES",
    "SHADOW_CANDIDATES",
    "MOVER_QUERIES",
    "MOVER_WALKS",
];
pub(crate) const BINDING: u32 = 21;
pub(crate) const BYTES: u64 = (KINDS * 8) as u64;
pub(crate) const WGSL: &str = include_str!("counters.wgsl");
const START: &str = "fn trace_counters_start() {}\n";
const END: &str = "fn trace_counters_end() {}\n";
const SUBGROUP: &str = "fn trace_count_first() -> bool {\n    return subgroupExclusiveAdd(1u)==0u;\n}\nfn trace_count_reduce(n: u32) -> u32 {\n    return subgroupAdd(n);\n}\n";
const LANE: &str = "fn trace_count_first() -> bool {\n    return true;\n}\nfn trace_count_reduce(n: u32) -> u32 {\n    return n;\n}\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduce {
    Subgroup,
    Lane,
}

impl Reduce {
    pub fn of(features: wgpu::Features) -> Reduce {
        if features.contains(wgpu::Features::SUBGROUP) {
            Reduce::Subgroup
        } else {
            Reduce::Lane
        }
    }

    fn wgsl(self) -> &'static str {
        match self {
            Reduce::Subgroup => SUBGROUP,
            Reduce::Lane => LANE,
        }
    }
}

pub(crate) fn spliced(source: String, reduce: Option<Reduce>) -> String {
    let Some(reduce) = reduce else {
        return source;
    };
    let (head, rest) = source.split_once(START).unwrap();
    let (_, tail) = rest.split_once(END).unwrap();
    format!("{head}{WGSL}{}{tail}", reduce.wgsl())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub paths: u64,
    pub primary: u64,
    pub bounce: u64,
    pub closest: u64,
    pub shadow: u64,
    pub surface_shadow: u64,
    pub closest_candidates: u64,
    pub shadow_candidates: u64,
    pub mover_queries: u64,
    pub mover_walks: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RaysPerPath {
    pub primary: f64,
    pub bounce: f64,
    pub other_closest: f64,
    pub surface_shadow: f64,
    pub other_shadow: f64,
    pub total: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CandidatesPerRay {
    pub closest: f64,
    pub shadow: f64,
}

impl Counts {
    pub fn from_words(words: &[u32]) -> Result<Counts, String> {
        if words.len() != 2 * KINDS {
            return Err(format!(
                "the trace counters hold {} words, not {}",
                words.len(),
                2 * KINDS
            ));
        }
        let at = |kind: usize| u64::from(words[2 * kind]) | (u64::from(words[2 * kind + 1]) << 32);
        Ok(Counts {
            paths: at(0),
            primary: at(1),
            bounce: at(2),
            closest: at(3),
            shadow: at(4),
            surface_shadow: at(5),
            closest_candidates: at(6),
            shadow_candidates: at(7),
            mover_queries: at(8),
            mover_walks: at(9),
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Counts, String> {
        if !bytes.len().is_multiple_of(4) {
            return Err(format!(
                "the trace counters' {} bytes are not whole words",
                bytes.len()
            ));
        }
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        Counts::from_words(&words)
    }

    pub fn rays(&self) -> u64 {
        self.closest + self.shadow
    }

    fn per(&self, count: u64, of: u64) -> f64 {
        if of == 0 {
            return 0.0;
        }
        count as f64 / of as f64
    }

    pub fn rays_per_path(&self) -> RaysPerPath {
        let per = |count| self.per(count, self.paths);
        RaysPerPath {
            primary: per(self.primary),
            bounce: per(self.bounce),
            other_closest: per(self.closest.saturating_sub(self.primary + self.bounce)),
            surface_shadow: per(self.surface_shadow),
            other_shadow: per(self.shadow.saturating_sub(self.surface_shadow)),
            total: per(self.rays()),
        }
    }

    pub fn candidates_per_ray(&self) -> CandidatesPerRay {
        CandidatesPerRay {
            closest: self.per(self.closest_candidates, self.closest),
            shadow: self.per(self.shadow_candidates, self.shadow),
        }
    }

    pub fn mover_queries_per_path(&self) -> f64 {
        self.per(self.mover_queries, self.paths)
    }

    pub fn mover_walks_per_path(&self) -> f64 {
        self.per(self.mover_walks, self.paths)
    }

    pub fn line(&self) -> String {
        let rays = self.rays_per_path();
        let candidates = self.candidates_per_ray();
        format!(
            "{} paths, {:.2} rays a path ({:.2} primary, {:.2} bounce, {:.2} other closest, {:.2} surface shadow, {:.2} other shadow), {:.2} candidates a closest ray and {:.2} a shadow ray, {:.2} mover queries and {:.2} mover walks a path",
            self.paths,
            rays.total,
            rays.primary,
            rays.bounce,
            rays.other_closest,
            rays.surface_shadow,
            rays.other_shadow,
            candidates.closest,
            candidates.shadow,
            self.mover_queries_per_path(),
            self.mover_walks_per_path()
        )
    }
}

pub(crate) struct Counters {
    buffer: wgpu::Buffer,
    pub(crate) reduce: Reduce,
}

pub(crate) fn room(gpu: &Gpu, lit: bool) -> Result<(), String> {
    let has = gpu.device.limits().max_storage_buffers_per_shader_stage;
    let floor = pfx_gpu::floor::TRACE_STORAGE_BUFFERS_PER_STAGE;
    if lit && has <= floor {
        return Err(format!(
            "counting a lit trace's rays needs {} storage buffers a shader stage, and this GPU offers {has}",
            floor + 1
        ));
    }
    Ok(())
}

impl Counters {
    pub(crate) fn new(gpu: &Gpu) -> Counters {
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace counters"),
            size: BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Counters {
            buffer,
            reduce: Reduce::of(gpu.device.features()),
        }
    }

    pub(crate) fn entry(&self) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding: BINDING,
            resource: self.buffer.as_entire_binding(),
        }
    }

    pub(crate) fn clear(&self, gpu: &Gpu) {
        gpu.queue
            .write_buffer(&self.buffer, 0, &[0; BYTES as usize]);
    }

    pub(crate) fn take(&self, gpu: &Gpu) -> Result<Counts, String> {
        let bytes = crate::checkpoint::read_buffer(gpu, &self.buffer)?;
        self.clear(gpu);
        Counts::from_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(values: [u64; KINDS]) -> Vec<u32> {
        values
            .into_iter()
            .flat_map(|value| [value as u32, (value >> 32) as u32])
            .collect()
    }

    #[test]
    fn the_counters_read_each_kind_as_a_low_and_a_high_word() {
        let counts = Counts::from_words(&words([
            (1 << 32) + 7,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            u64::from(u32::MAX) + 1,
            10,
        ]))
        .unwrap();
        assert_eq!(counts.paths, (1 << 32) + 7);
        assert_eq!(counts.primary, 2);
        assert_eq!(counts.mover_queries, u64::from(u32::MAX) + 1);
        assert_eq!(counts.mover_walks, 10);
        let bytes: Vec<u8> = words([11, 0, 0, 0, 0, 0, 0, 0, 0, 1 << 40])
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let read = Counts::from_bytes(&bytes).unwrap();
        assert_eq!((read.paths, read.mover_walks), (11, 1 << 40));
        assert!(Counts::from_words(&[0; 2 * KINDS - 1]).is_err());
        assert!(Counts::from_bytes(&[0; 3]).is_err());
    }

    #[test]
    fn the_shader_numbers_its_counters_in_the_order_the_host_reads_them() {
        let source = include_str!("trace.wgsl");
        for (at, name) in NAMES.into_iter().enumerate() {
            let line = format!("const COUNT_{name}: u32 = {at}u;");
            assert!(source.contains(&line), "{line}");
        }
        assert!(source.contains(&format!("const COUNT_KINDS: u32 = {KINDS}u;")));
        assert!(WGSL.contains(&format!("array<u32,{KINDS}>")));
        assert!(WGSL.contains(&format!("@binding({BINDING})")));
    }

    const LANES: u32 = 1000;
    const TALLY: &str = "@compute @workgroup_size(64)\nfn main(@builtin(global_invocation_id) id: vec3u) {\n    if (id.x>=1000u) { return; }\n    trace_count(0u);\n    for (var i=0u;i<id.x%7u;i++) {\n        if (i==id.x%5u) { break; }\n        trace_count(1u);\n    }\n    if (id.x%3u==0u) { trace_count_by(2u,id.x); }\n    trace_count_by(3u,0x00ffffffu);\n    trace_count_flush();\n}\n";

    fn tallied(gpu: &Gpu, reduce: Reduce, start: &[u32]) -> Counts {
        let source = format!(
            "const COUNT_KINDS: u32 = {KINDS}u;\n{WGSL}{}{TALLY}",
            reduce.wgsl()
        );
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("trace counters tally"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("trace counters tally"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let counters = Counters::new(gpu);
        let bytes: Vec<u8> = start.iter().flat_map(|word| word.to_le_bytes()).collect();
        gpu.queue.write_buffer(&counters.buffer, 0, &bytes);
        let binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trace counters tally"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[counters.entry()],
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("trace counters tally"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("trace counters tally"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(LANES.div_ceil(64), 1, 1);
        }
        gpu.queue.submit(Some(encoder.finish()));
        let counts = counters.take(gpu).unwrap();
        assert_eq!(counters.take(gpu).unwrap(), Counts::default());
        counts
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn both_reductions_add_the_same_64_bit_totals() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let offered = gpu.adapter.features().contains(wgpu::Features::SUBGROUP);
        assert_eq!(
            Reduce::of(gpu.device.features()) == Reduce::Subgroup,
            offered
        );
        let mut start = vec![0u32; 2 * KINDS];
        start[0] = 0xffff_ff00;
        let lanes = 0..u64::from(LANES);
        let expected = Counts {
            paths: 0xffff_ff00 + u64::from(LANES),
            primary: lanes.clone().map(|x| (x % 7).min(x % 5)).sum(),
            bounce: lanes.filter(|x| x % 3 == 0).sum(),
            closest: u64::from(LANES) * 0x00ff_ffff,
            ..Counts::default()
        };
        assert!(expected.paths > u64::from(u32::MAX));
        assert_eq!(tallied(&gpu, Reduce::Lane, &start), expected);
        if offered {
            assert_eq!(tallied(&gpu, Reduce::Subgroup, &start), expected);
        }
    }

    #[test]
    fn the_counted_shaders_validate_with_either_reduction() {
        for rt in [false, true] {
            for reduce in [Reduce::Subgroup, Reduce::Lane] {
                let source = crate::gpu::counted_source(true, rt, Some(reduce));
                assert!(source.contains("fn trace_count_add("));
                assert!(!source.contains("fn trace_counters_start()"));
                let module = naga::front::wgsl::parse_str(&source).unwrap();
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .unwrap();
            }
        }
        assert!(!crate::gpu::counted_source(true, false, None).contains("trace_counters:"));
    }
}
