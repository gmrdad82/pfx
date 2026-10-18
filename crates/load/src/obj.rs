use std::collections::HashMap;
use std::fmt;
use std::str::{FromStr, SplitWhitespace};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<f32>,
    pub vertex_color: Vec<f32>,
    pub normals: Vec<f32>,
    pub texcoords: Vec<f32>,
    pub indices: Vec<u32>,
    pub face_arities: Vec<u32>,
    pub material_id: Option<usize>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LoadOptions {
    pub triangulate: bool,
    pub ignore_points: bool,
    pub ignore_lines: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub mesh: Mesh,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Texture {
    pub name: String,
    pub bump_multiplier: Option<f32>,
    pub offset: Option<[f32; 3]>,
    pub scale: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Material {
    pub name: String,
    pub ambient: Option<[f32; 3]>,
    pub diffuse: Option<[f32; 3]>,
    pub specular: Option<[f32; 3]>,
    pub emissive: Option<[f32; 3]>,
    pub shininess: Option<f32>,
    pub dissolve: Option<f32>,
    pub optical_density: Option<f32>,
    pub roughness: Option<f32>,
    pub metallic: Option<f32>,
    pub clearcoat: Option<f32>,
    pub clearcoat_roughness: Option<f32>,
    pub ambient_texture: Option<String>,
    pub diffuse_texture: Option<String>,
    pub specular_texture: Option<String>,
    pub normal_texture: Option<String>,
    pub shininess_texture: Option<String>,
    pub dissolve_texture: Option<String>,
    pub norm_texture: Option<String>,
    pub emissive_texture: Option<String>,
    pub roughness_texture: Option<String>,
    pub metallic_texture: Option<String>,
    pub illumination_model: Option<u8>,
    pub unknown_param: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    ReadError,
    PositionParseError,
    ScalingByZeroError,
    ColorParseError,
    NormalParseError,
    TexcoordParseError,
    FaceParseError,
    MaterialParseError,
    InvalidObjectName,
    InvalidPolygon,
    FaceVertexOutOfBounds,
    FaceTexCoordOutOfBounds,
    FaceNormalOutOfBounds,
    FaceColorOutOfBounds,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let msg = match *self {
            LoadError::ReadError => "read error",
            LoadError::PositionParseError => "position parse error",
            LoadError::ScalingByZeroError => "scaling (w) by zero",
            LoadError::ColorParseError => "color parse error (too many or too few components)",
            LoadError::NormalParseError => "normal parse error",
            LoadError::TexcoordParseError => "texcoord parse error",
            LoadError::FaceParseError => "face parse error",
            LoadError::MaterialParseError => "material parse error",
            LoadError::InvalidObjectName => "invalid object name",
            LoadError::InvalidPolygon => "invalid polygon",
            LoadError::FaceVertexOutOfBounds => "face vertex index out of bounds",
            LoadError::FaceTexCoordOutOfBounds => "face texcoord index out of bounds",
            LoadError::FaceNormalOutOfBounds => "face normal index out of bounds",
            LoadError::FaceColorOutOfBounds => "face vertex color index out of bounds",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for LoadError {}

pub type LoadResult = Result<(Vec<Model>, Result<Vec<Material>, LoadError>), LoadError>;

pub type MtlLoadResult = Result<(Vec<Material>, HashMap<String, usize>), LoadError>;

#[derive(Hash, Eq, PartialEq, Debug, Copy, Clone)]
struct VertexIndices {
    v: usize,
    vt: usize,
    vn: usize,
}

const MISSING_INDEX: usize = usize::MAX;

impl VertexIndices {
    fn parse(face_str: &str, pos_sz: usize, tex_sz: usize, norm_sz: usize) -> Option<Self> {
        let mut indices = [MISSING_INDEX; 3];
        for (slot, text) in face_str.split('/').enumerate() {
            if !text.is_empty() {
                let x = isize::from_str(text).ok()?;
                *indices.get_mut(slot)? = if x < 0 {
                    let size = [pos_sz, tex_sz, norm_sz][slot] as isize;
                    (size + x) as usize
                } else {
                    (x - 1) as usize
                };
            }
        }
        Some(VertexIndices {
            v: indices[0],
            vt: indices[1],
            vn: indices[2],
        })
    }
}

#[derive(Debug)]
enum Face {
    Point(VertexIndices),
    Line(VertexIndices, VertexIndices),
    Triangle(VertexIndices, VertexIndices, VertexIndices),
    Quad(VertexIndices, VertexIndices, VertexIndices, VertexIndices),
    Polygon(Vec<VertexIndices>),
}

fn parse_floatn(val_str: &mut SplitWhitespace, vals: &mut Vec<f32>, n: usize) -> bool {
    let sz = vals.len();
    for p in val_str.take(n) {
        match f32::from_str(p) {
            Ok(x) => vals.push(x),
            Err(_) => return false,
        }
    }
    sz + n == vals.len()
}

fn parse_float3(val_str: SplitWhitespace) -> Result<[f32; 3], LoadError> {
    let values = val_str
        .take(3)
        .map(f32::from_str)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| LoadError::MaterialParseError)?;
    values.try_into().map_err(|_| LoadError::MaterialParseError)
}

fn parse_float(val_str: Option<&str>) -> Result<f32, LoadError> {
    val_str
        .map(f32::from_str)
        .map_or(Err(LoadError::MaterialParseError), |v| {
            v.map_err(|_| LoadError::MaterialParseError)
        })
}

fn parse_face(
    face_str: SplitWhitespace,
    faces: &mut Vec<Face>,
    pos_sz: usize,
    tex_sz: usize,
    norm_sz: usize,
) -> bool {
    let mut indices = Vec::new();
    for f in face_str {
        match VertexIndices::parse(f, pos_sz, tex_sz, norm_sz) {
            Some(v) => indices.push(v),
            None => return false,
        }
    }
    match indices.len() {
        0 => return false,
        1 => faces.push(Face::Point(indices[0])),
        2 => faces.push(Face::Line(indices[0], indices[1])),
        3 => faces.push(Face::Triangle(indices[0], indices[1], indices[2])),
        4 => faces.push(Face::Quad(indices[0], indices[1], indices[2], indices[3])),
        _ => faces.push(Face::Polygon(indices)),
    }
    true
}

struct Attributes<'a> {
    pos: &'a [f32],
    v_color: &'a [f32],
    texcoord: &'a [f32],
    normal: &'a [f32],
}

fn add_vertex(
    mesh: &mut Mesh,
    index_map: &mut HashMap<VertexIndices, u32>,
    vert: &VertexIndices,
    from: &Attributes,
) -> Result<(), LoadError> {
    if let Some(&i) = index_map.get(vert) {
        mesh.indices.push(i);
        return Ok(());
    }
    let v = vert.v;
    if v.saturating_mul(3).saturating_add(2) >= from.pos.len() {
        return Err(LoadError::FaceVertexOutOfBounds);
    }
    mesh.positions
        .extend_from_slice(&from.pos[v * 3..v * 3 + 3]);
    if !from.texcoord.is_empty() && vert.vt != MISSING_INDEX {
        let vt = vert.vt;
        if vt.saturating_mul(2).saturating_add(1) >= from.texcoord.len() {
            return Err(LoadError::FaceTexCoordOutOfBounds);
        }
        mesh.texcoords
            .extend_from_slice(&from.texcoord[vt * 2..vt * 2 + 2]);
    }
    if !from.normal.is_empty() && vert.vn != MISSING_INDEX {
        let vn = vert.vn;
        if vn.saturating_mul(3).saturating_add(2) >= from.normal.len() {
            return Err(LoadError::FaceNormalOutOfBounds);
        }
        mesh.normals
            .extend_from_slice(&from.normal[vn * 3..vn * 3 + 3]);
    }
    if !from.v_color.is_empty() {
        if v * 3 + 2 >= from.v_color.len() {
            return Err(LoadError::FaceColorOutOfBounds);
        }
        mesh.vertex_color
            .extend_from_slice(&from.v_color[v * 3..v * 3 + 3]);
    }
    let next = index_map.len() as u32;
    mesh.indices.push(next);
    index_map.insert(*vert, next);
    Ok(())
}

fn export_faces(
    from: &Attributes,
    faces: &[Face],
    mat_id: Option<usize>,
    load_options: &LoadOptions,
) -> Result<Mesh, LoadError> {
    let mut index_map = HashMap::new();
    let mut mesh = Mesh {
        material_id: mat_id,
        ..Default::default()
    };
    let mut is_all_triangles = true;
    let mut add =
        |mesh: &mut Mesh, vert: &VertexIndices| add_vertex(mesh, &mut index_map, vert, from);
    for f in faces {
        match f {
            Face::Point(a) => {
                if !load_options.ignore_points {
                    add(&mut mesh, a)?;
                    if load_options.triangulate {
                        add(&mut mesh, a)?;
                        add(&mut mesh, a)?;
                    } else {
                        is_all_triangles = false;
                        mesh.face_arities.push(1);
                    }
                }
            }
            Face::Line(a, b) => {
                if !load_options.ignore_lines {
                    add(&mut mesh, a)?;
                    add(&mut mesh, b)?;
                    if load_options.triangulate {
                        add(&mut mesh, b)?;
                    } else {
                        is_all_triangles = false;
                        mesh.face_arities.push(2);
                    }
                }
            }
            Face::Triangle(a, b, c) => {
                add(&mut mesh, a)?;
                add(&mut mesh, b)?;
                add(&mut mesh, c)?;
                if !load_options.triangulate {
                    mesh.face_arities.push(3);
                }
            }
            Face::Quad(a, b, c, d) => {
                add(&mut mesh, a)?;
                add(&mut mesh, b)?;
                add(&mut mesh, c)?;
                if load_options.triangulate {
                    add(&mut mesh, a)?;
                    add(&mut mesh, c)?;
                    add(&mut mesh, d)?;
                } else {
                    add(&mut mesh, d)?;
                    is_all_triangles = false;
                    mesh.face_arities.push(4);
                }
            }
            Face::Polygon(indices) => {
                if load_options.triangulate {
                    let a = indices.first().ok_or(LoadError::InvalidPolygon)?;
                    let mut b = indices.get(1).ok_or(LoadError::InvalidPolygon)?;
                    for c in indices.iter().skip(2) {
                        add(&mut mesh, a)?;
                        add(&mut mesh, b)?;
                        add(&mut mesh, c)?;
                        b = c;
                    }
                } else {
                    for i in indices {
                        add(&mut mesh, i)?;
                    }
                    is_all_triangles = false;
                    mesh.face_arities.push(indices.len() as u32);
                }
            }
        }
    }
    if is_all_triangles {
        mesh.face_arities = Vec::new();
    }
    Ok(mesh)
}

struct TmpModels {
    models: Vec<Model>,
    pos: Vec<f32>,
    v_color: Vec<f32>,
    texcoord: Vec<f32>,
    normal: Vec<f32>,
    faces: Vec<Face>,
    name: String,
    mat_id: Option<usize>,
}

impl Default for TmpModels {
    fn default() -> Self {
        Self {
            models: Vec::new(),
            pos: Vec::new(),
            v_color: Vec::new(),
            texcoord: Vec::new(),
            normal: Vec::new(),
            faces: Vec::new(),
            name: "unnamed_object".to_owned(),
            mat_id: None,
        }
    }
}

impl TmpModels {
    fn pop_model(&mut self, load_options: &LoadOptions) -> Result<(), LoadError> {
        let from = Attributes {
            pos: &self.pos,
            v_color: &self.v_color,
            texcoord: &self.texcoord,
            normal: &self.normal,
        };
        let mesh = export_faces(&from, &self.faces, self.mat_id, load_options)?;
        self.models.push(Model {
            mesh,
            name: self.name.clone(),
        });
        self.faces.clear();
        Ok(())
    }
}

#[derive(Debug, Default)]
struct TmpMaterials {
    materials: Vec<Material>,
    mat_map: HashMap<String, usize>,
    mtlerr: Option<LoadError>,
}

impl TmpMaterials {
    fn push(&mut self, material: Material) {
        self.mat_map
            .insert(material.name.clone(), self.materials.len());
        self.materials.push(material);
    }

    fn merge(&mut self, mtl_load_result: MtlLoadResult) {
        match mtl_load_result {
            Ok((mut mats, map)) => {
                let mat_offset = self.materials.len();
                self.materials.append(&mut mats);
                for (name, index) in map {
                    self.mat_map.insert(name, index + mat_offset);
                }
            }
            Err(e) => self.mtlerr = Some(e),
        }
    }

    fn into_materials(self) -> Result<Vec<Material>, LoadError> {
        if !self.materials.is_empty() {
            Ok(self.materials)
        } else if let Some(mtlerr) = self.mtlerr {
            Err(mtlerr)
        } else {
            Ok(Vec::new())
        }
    }
}

enum ParseReturnType {
    LoadMaterial(String),
    None,
}

fn parse_obj_line(
    line: &str,
    load_options: &LoadOptions,
    models: &mut TmpModels,
    materials: &TmpMaterials,
) -> Result<ParseReturnType, LoadError> {
    let mut words = line.split_whitespace();
    match words.next() {
        Some("#") | None => Ok(ParseReturnType::None),
        Some("v") => {
            if !parse_floatn(&mut words, &mut models.pos, 3) {
                return Err(LoadError::PositionParseError);
            }
            let Some(first) = words.next() else {
                return Ok(ParseReturnType::None);
            };
            match (words.next(), words.next()) {
                (None, _) => {
                    let w = f32::from_str(first).map_err(|_| LoadError::PositionParseError)?;
                    if w == 0.0 {
                        return Err(LoadError::ScalingByZeroError);
                    }
                    for coordinate in models.pos.iter_mut().rev().take(3) {
                        *coordinate /= w;
                    }
                }
                (Some(_), None) => return Err(LoadError::ColorParseError),
                (Some(second), Some(third)) => {
                    if words.next().is_some() {
                        return Err(LoadError::ColorParseError);
                    }
                    for rgb_component in [first, second, third] {
                        match f32::from_str(rgb_component) {
                            Ok(x) => models.v_color.push(x),
                            Err(_) => break,
                        }
                    }
                }
            }
            Ok(ParseReturnType::None)
        }
        Some("vt") => {
            if parse_floatn(&mut words, &mut models.texcoord, 2) {
                Ok(ParseReturnType::None)
            } else {
                Err(LoadError::TexcoordParseError)
            }
        }
        Some("vn") => {
            if parse_floatn(&mut words, &mut models.normal, 3) {
                Ok(ParseReturnType::None)
            } else {
                Err(LoadError::NormalParseError)
            }
        }
        Some("f") | Some("l") => {
            if parse_face(
                words,
                &mut models.faces,
                models.pos.len() / 3,
                models.texcoord.len() / 2,
                models.normal.len() / 3,
            ) {
                Ok(ParseReturnType::None)
            } else {
                Err(LoadError::FaceParseError)
            }
        }
        Some("o") | Some("g") => {
            if !models.faces.is_empty() {
                models.pop_model(load_options)?;
            }
            models.name = line.trim_start()[1..].trim().to_owned();
            if models.name.is_empty() {
                models.name = "unnamed_object".to_owned();
            }
            Ok(ParseReturnType::None)
        }
        Some("mtllib") => {
            let mtllib = rest(line, "mtllib");
            Ok(ParseReturnType::LoadMaterial(mtllib.to_owned()))
        }
        Some("usemtl") => {
            let mat_name = rest(line, "usemtl");
            if mat_name.is_empty() {
                return Err(LoadError::MaterialParseError);
            }
            let new_mat = materials.mat_map.get(mat_name).cloned();
            if models.mat_id != new_mat && !models.faces.is_empty() {
                models.pop_model(load_options)?;
            }
            models.mat_id = new_mat;
            Ok(ParseReturnType::None)
        }
        Some(_) => Ok(ParseReturnType::None),
    }
}

fn rest<'a>(line: &'a str, keyword: &str) -> &'a str {
    line.trim_start()[keyword.len()..].trim()
}

fn texture(line: &str, keyword: &str) -> Result<String, LoadError> {
    match line.get(keyword.len()..).map(str::trim) {
        Some("") | None => Err(LoadError::MaterialParseError),
        Some(tex) => Ok(tex.to_owned()),
    }
}

pub fn texture_options(raw: &str) -> Option<Texture> {
    let mut out = Texture::default();
    let mut token = raw;
    loop {
        token = token.trim_start_matches([' ', '\t']);
        if token.is_empty() {
            return None;
        }
        let (word, after) = token.split_once([' ', '\t']).unwrap_or((token, ""));
        let count = match word {
            "-blendu" | "-blendv" | "-clamp" | "-boost" | "-bm" | "-type" | "-texres"
            | "-imfchan" | "-colorspace" => 1,
            "-mm" => 2,
            "-o" | "-s" | "-t" => 3,
            _ => {
                out.name = token.trim_end().to_owned();
                return Some(out);
            }
        };
        let mut args = after.trim_start_matches([' ', '\t']);
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            let (value, next) = args.split_once([' ', '\t']).unwrap_or((args, ""));
            values.push(value);
            args = next.trim_start_matches([' ', '\t']);
        }
        let real = |k: usize, fallback: f32| -> f32 {
            values
                .get(k)
                .and_then(|v| f32::from_str(v).ok())
                .unwrap_or(fallback)
        };
        match word {
            "-bm" => out.bump_multiplier = Some(real(0, 1.0)),
            "-o" => out.offset = Some([real(0, 0.0), real(1, 0.0), real(2, 0.0)]),
            "-s" => out.scale = Some([real(0, 1.0), real(1, 1.0), real(2, 1.0)]),
            _ => {}
        }
        token = args;
    }
}

fn parse_mtl_line(
    line: &str,
    materials: &mut TmpMaterials,
    mut cur_mat: Material,
    has_d: &mut bool,
) -> Result<Material, LoadError> {
    let line = line.trim();
    let mut words = line.split_whitespace();
    match words.next() {
        Some("#") | None => {}
        Some("newmtl") => {
            if !cur_mat.name.is_empty() {
                materials.push(cur_mat);
            }
            cur_mat = Material {
                name: line[6..].trim().to_owned(),
                ..Material::default()
            };
            *has_d = false;
            if cur_mat.name.is_empty() {
                return Err(LoadError::InvalidObjectName);
            }
        }
        Some("Ka") => cur_mat.ambient = Some(parse_float3(words)?),
        Some("Kd") => cur_mat.diffuse = Some(parse_float3(words)?),
        Some("Ks") => cur_mat.specular = Some(parse_float3(words)?),
        Some("Ke") => cur_mat.emissive = Some(parse_float3(words)?),
        Some("Ns") => cur_mat.shininess = Some(parse_float(words.next())?),
        Some("Ni") => cur_mat.optical_density = Some(parse_float(words.next())?),
        Some("d") => {
            cur_mat.dissolve = Some(parse_float(words.next())?);
            *has_d = true;
        }
        Some("Tr") => {
            let tr = parse_float(words.next())?;
            if !*has_d {
                cur_mat.dissolve = Some(1.0 - tr);
            }
        }
        Some("Pr") => cur_mat.roughness = Some(parse_float(words.next())?),
        Some("Pm") => cur_mat.metallic = Some(parse_float(words.next())?),
        Some("Pc") => cur_mat.clearcoat = Some(parse_float(words.next())?),
        Some("Pcr") => cur_mat.clearcoat_roughness = Some(parse_float(words.next())?),
        Some("map_Ka") => cur_mat.ambient_texture = Some(texture(line, "map_Ka")?),
        Some("map_Kd") => cur_mat.diffuse_texture = Some(texture(line, "map_Kd")?),
        Some("map_Ks") => cur_mat.specular_texture = Some(texture(line, "map_Ks")?),
        Some("map_Bump") | Some("map_bump") => {
            cur_mat.normal_texture = Some(texture(line, "map_Bump")?)
        }
        Some("map_Ns") | Some("map_ns") | Some("map_NS") => {
            cur_mat.shininess_texture = Some(texture(line, "map_Ns")?)
        }
        Some("bump") => cur_mat.normal_texture = Some(texture(line, "bump")?),
        Some("norm") => cur_mat.norm_texture = Some(texture(line, "norm")?),
        Some("map_d") => cur_mat.dissolve_texture = Some(texture(line, "map_d")?),
        Some("map_Pr") => cur_mat.roughness_texture = Some(texture(line, "map_Pr")?),
        Some("map_Pm") => cur_mat.metallic_texture = Some(texture(line, "map_Pm")?),
        Some("map_Ke") => cur_mat.emissive_texture = Some(texture(line, "map_Ke")?),
        Some("illum") => {
            let p = words.next().ok_or(LoadError::MaterialParseError)?;
            cur_mat.illumination_model =
                Some(u8::from_str(p).map_err(|_| LoadError::MaterialParseError)?);
        }
        Some(unknown) => {
            let param = line[unknown.len()..].trim().to_owned();
            cur_mat.unknown_param.insert(unknown.to_owned(), param);
        }
    }
    Ok(cur_mat)
}

pub fn load_obj_text<ML>(text: &str, load_options: &LoadOptions, material_loader: ML) -> LoadResult
where
    ML: Fn(&str) -> MtlLoadResult,
{
    let mut models = TmpModels::default();
    let mut materials = TmpMaterials::default();
    for line in text.lines() {
        match parse_obj_line(line, load_options, &mut models, &materials)? {
            ParseReturnType::LoadMaterial(mat_file) => materials.merge(material_loader(&mat_file)),
            ParseReturnType::None => {}
        }
    }
    models.pop_model(load_options)?;
    Ok((models.models, materials.into_materials()))
}

pub fn load_mtl_text(text: &str) -> MtlLoadResult {
    let mut materials = TmpMaterials::default();
    let mut cur_mat = Material::default();
    let mut has_d = false;
    for line in text.lines() {
        cur_mat = parse_mtl_line(line, &mut materials, cur_mat, &mut has_d)?;
    }
    if !cur_mat.name.is_empty() {
        materials.push(cur_mat);
    }
    Ok((materials.materials, materials.mat_map))
}

pub fn text(bytes: &[u8]) -> Result<&str, LoadError> {
    std::str::from_utf8(bytes).map_err(|_| LoadError::ReadError)
}
