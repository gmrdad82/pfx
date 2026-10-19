use std::fmt::Write;

use pfx_geom::mark::{Form, Mark};
use pfx_glb::{DMat4, Node, Order};
use pfx_mesh::{Mesh, Slot};

pub const PLATE: f32 = 0.99;
pub const CAP: f32 = 0.36;
pub const WIDTH: f32 = 2.4;
pub const GAP: f32 = 0.15;
pub const NAME_DEPTH: f32 = 0.029;
pub const NAME_BEVEL: f32 = 0.006;
pub const PLATE_DEPTH: f32 = 0.015;
pub const PLATE_BEVEL: f32 = 0.025;
const FACE: f32 = 0.8;
const FACE_LIFT: f32 = 0.0015;
const GENERATOR: &str = "pfx new";

#[derive(Clone, Debug, PartialEq)]
pub struct Title {
    pub name: Vec<u8>,
    pub mark: Vec<u8>,
    pub name_at: [f32; 3],
    pub mark_at: [f32; 3],
    pub version_at: [f32; 3],
    pub version_size: f32,
}

struct Pen {
    path: String,
    x: f32,
}

impl ttf_parser::OutlineBuilder for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.path, "M{} {} ", self.x + x, -y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.path, "L{} {} ", self.x + x, -y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(self.path, "Q{} {} {} {} ", self.x + x1, -y1, self.x + x, -y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ = write!(
            self.path,
            "C{} {} {} {} {} {} ",
            self.x + x1,
            -y1,
            self.x + x2,
            -y2,
            self.x + x,
            -y
        );
    }

    fn close(&mut self) {
        self.path.push_str("Z ");
    }
}

struct Laid {
    svg: String,
    width: f32,
    cap: f32,
}

fn lay(font: &[u8], text: &str) -> Result<Laid, String> {
    let face = ttf_parser::Face::parse(font, 0).map_err(|error| format!("the font: {error}"))?;
    let cap = face
        .capital_height()
        .filter(|cap| *cap > 0)
        .map(f32::from)
        .unwrap_or(f32::from(face.units_per_em()) * 0.7);
    let mut pen = Pen {
        path: String::new(),
        x: 0.0,
    };
    for c in text.chars() {
        let glyph = face
            .glyph_index(c)
            .ok_or_else(|| format!("the font has no glyph for {c:?}"))?;
        face.outline_glyph(glyph, &mut pen);
        pen.x += f32::from(face.glyph_hor_advance(glyph).unwrap_or(0));
    }
    if pen.path.is_empty() {
        return Err(format!("{text:?} draws nothing in the font"));
    }
    let width = pen.x;
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 {} {width} {cap}\"><path fill=\"#ffffff\" d=\"{}\"/></svg>",
        -cap,
        pen.path.trim_end()
    );
    Ok(Laid { svg, width, cap })
}

fn solid(svg: &str, depth: f32, bevel: f32, size: f32, material: &str) -> Result<Mesh, String> {
    let form = Form {
        depth: depth / size,
        bevel: bevel / size,
        ..Form::default()
    };
    let mark = Mark::build(svg, &form)?;
    let mut mesh = Mesh {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Some(Vec::new()),
        uvs1: None,
        indices: Vec::new(),
        slots: Vec::new(),
    };
    for solid in &mark.solids {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(
            solid
                .mesh
                .positions
                .iter()
                .map(|p| p.map(|value| value * size)),
        );
        mesh.normals.extend_from_slice(&solid.mesh.normals);
        if let Some(uvs) = mesh.uvs.as_mut() {
            uvs.extend_from_slice(&solid.mesh.uvs);
        }
        mesh.indices
            .extend(solid.mesh.indices.iter().map(|index| index + base));
    }
    if mesh.indices.is_empty() {
        return Err("the outline builds no solid".into());
    }
    mesh.slots.push(Slot {
        material: material.into(),
        start: 0,
        count: mesh.indices.len() as u32,
    });
    Ok(mesh)
}

fn face(mesh: &mut Mesh, picture: [u32; 2]) {
    let [width, height] = picture.map(|side| side.max(1) as f32);
    let fit = PLATE * FACE;
    let scale = fit / width.max(height);
    let (x, y) = (width * scale * 0.5, height * scale * 0.5);
    let z = PLATE_DEPTH + PLATE_BEVEL + FACE_LIFT;
    let base = mesh.positions.len() as u32;
    mesh.positions
        .extend([[-x, y, z], [x, y, z], [x, -y, z], [-x, -y, z]]);
    mesh.normals.extend([[0.0, 0.0, 1.0]; 4]);
    if let Some(uvs) = mesh.uvs.as_mut() {
        uvs.extend([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    }
    let start = mesh.indices.len() as u32;
    mesh.indices
        .extend([base, base + 3, base + 2, base, base + 2, base + 1]);
    mesh.slots.push(Slot {
        material: "face".into(),
        start,
        count: 6,
    });
}

fn glb(name: &str, mesh: Mesh) -> Vec<u8> {
    let node = Node {
        name: name.into(),
        mesh,
        transform: DMat4::IDENTITY,
        keep_transform: false,
        targets: None,
    };
    pfx_glb::write(&[node], Order::File, GENERATOR)
}

pub fn picture_size(png: &[u8]) -> Result<[u32; 2], String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let reader = decoder
        .read_info()
        .map_err(|error| format!("the mark is not a PNG: {error}"))?;
    let info = reader.info();
    if info.width == 0 || info.height == 0 {
        return Err("the mark is an empty picture".into());
    }
    Ok([info.width, info.height])
}

pub fn build(font: &[u8], text: &str, mark: Option<[u32; 2]>) -> Result<Title, String> {
    let laid = lay(font, text)?;
    let metres = (CAP / laid.cap).min(WIDTH / laid.width);
    let size = laid.width.max(laid.cap) * metres;
    let name = solid(&laid.svg, NAME_DEPTH, NAME_BEVEL, size, "name")?;
    let plate = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"-5 -5 110 110\"><rect x=\"0\" y=\"0\" width=\"100\" height=\"100\" rx=\"16\" fill=\"#ffffff\"/></svg>";
    let side = (PLATE - 2.0 * PLATE_BEVEL) * 1.1;
    let mut plate = solid(plate, PLATE_DEPTH, PLATE_BEVEL, side, "plate")?;
    if let Some(picture) = mark {
        face(&mut plate, picture);
    }
    let width = laid.width * metres;
    let total = PLATE + GAP + width;
    let mark_at = [-total * 0.5 + PLATE * 0.5, 0.0, 0.0];
    let name_at = [total * 0.5 - width * 0.5, 0.0, 0.0];
    let version_size = 0.072;
    let version_at = [
        name_at[0],
        -PLATE * 0.5 + version_size * 0.75,
        NAME_DEPTH + NAME_BEVEL,
    ];
    Ok(Title {
        name: glb("name", name),
        mark: glb("mark", plate),
        name_at,
        mark_at,
        version_at,
        version_size,
    })
}
