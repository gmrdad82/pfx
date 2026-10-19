use serde_json::{Value, json};

#[derive(Default)]
struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn floats(&mut self, values: &[f32], kind: &str, width: usize) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let offset = self.data.len();
        self.data
            .extend(values.iter().flat_map(|v| v.to_le_bytes()));
        self.views
            .push(json!({ "buffer": 0, "byteOffset": offset, "byteLength": values.len() * 4 }));
        let mut lo = vec![f32::INFINITY; width];
        let mut hi = vec![f32::NEG_INFINITY; width];
        for chunk in values.chunks(width) {
            for (i, v) in chunk.iter().enumerate() {
                lo[i] = lo[i].min(*v);
                hi[i] = hi[i].max(*v);
            }
        }
        self.accessors.push(json!({
            "bufferView": self.views.len() - 1,
            "componentType": 5126,
            "count": values.len() / width,
            "type": kind,
            "min": lo,
            "max": hi,
        }));
        self.accessors.len() - 1
    }

    fn indices(&mut self, values: &[u16]) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let offset = self.data.len();
        self.data
            .extend(values.iter().flat_map(|v| v.to_le_bytes()));
        self.views
            .push(json!({ "buffer": 0, "byteOffset": offset, "byteLength": values.len() * 2 }));
        self.accessors.push(json!({
            "bufferView": self.views.len() - 1,
            "componentType": 5123,
            "count": values.len(),
            "type": "SCALAR",
        }));
        self.accessors.len() - 1
    }
}

pub const TILTED: [f32; 3] = [0.0, 0.6, 0.8];
pub const TURN: [f32; 3] = [0.0, -0.2, 0.1];
pub const ARM_UV: [f32; 4] = [0.0, 0.0, 0.5, 0.5];
pub const CAP_UV: [f32; 4] = [0.75, 0.75, 1.0, 1.0];

pub struct Model {
    pub scale: f32,
    pub paint: [f32; 3],
    pub normals: bool,
    pub uvs: bool,
}

impl Default for Model {
    fn default() -> Model {
        Model {
            scale: 1.0,
            paint: [0.8, 0.1, 0.1],
            normals: true,
            uvs: false,
        }
    }
}

fn quad(
    bin: &mut Bin,
    half: f32,
    normal: [f32; 3],
    normals: bool,
    morph: bool,
    uv: Option<[f32; 4]>,
) -> Value {
    let positions = [
        -half, -half, 0.0, half, -half, 0.0, half, half, 0.0, -half, half, 0.0,
    ];
    let p = bin.floats(&positions, "VEC3", 3);
    let mut attributes = json!({ "POSITION": p });
    if normals {
        attributes["NORMAL"] = json!(bin.floats(&normal.repeat(4), "VEC3", 3));
    }
    if let Some([u0, v0, u1, v1]) = uv {
        attributes["TEXCOORD_0"] = json!(bin.floats(&[u0, v1, u1, v1, u1, v0, u0, v0], "VEC2", 2));
    }
    let i = bin.indices(&[0, 1, 2, 0, 2, 3]);
    let mut primitive = json!({ "attributes": attributes, "indices": i });
    if morph {
        let lift = bin.floats(&[0.0, 0.0, 0.1].repeat(4), "VEC3", 3);
        let turn = bin.floats(&TURN.repeat(4), "VEC3", 3);
        primitive["targets"] = json!([{ "POSITION": lift, "NORMAL": turn }]);
    }
    primitive
}

impl Model {
    pub fn glb(&self) -> Vec<u8> {
        let mut bin = Bin::default();
        let uv = |rect: [f32; 4]| self.uvs.then_some(rect);
        let mut lid = quad(
            &mut bin,
            0.5,
            TILTED,
            self.normals,
            true,
            uv([0.0, 0.0, 1.0, 1.0]),
        );
        lid["material"] = json!(0);
        let mut arm = quad(
            &mut bin,
            0.25,
            [0.0, 0.0, 1.0],
            self.normals,
            false,
            uv(ARM_UV),
        );
        arm["material"] = json!(1);
        let mut cap = quad(
            &mut bin,
            0.1,
            [0.0, 0.0, 1.0],
            self.normals,
            false,
            uv(CAP_UV),
        );
        cap["material"] = json!(1);
        let s = self.scale;
        let [r, g, b] = self.paint;
        let json = json!({
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [
                { "name": "Root", "scale": [s, s, s], "children": [1, 2] },
                { "name": "Lid", "mesh": 0, "translation": [0.0, 1.0, 0.0] },
                { "name": "Arm", "mesh": 1, "translation": [1.0, 0.0, 0.0], "children": [3] },
                { "name": "Lid", "mesh": 2, "translation": [0.0, 0.5, 0.0] },
            ],
            "meshes": [
                { "primitives": [lid], "weights": [0.0], "extras": { "targetNames": ["Lift"] } },
                { "primitives": [arm] },
                { "primitives": [cap] },
            ],
            "materials": [
                { "name": "Paint", "pbrMetallicRoughness": { "baseColorFactor": [r, g, b, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.3 } },
                { "name": "Steel", "pbrMetallicRoughness": { "baseColorFactor": [0.5, 0.5, 0.5, 1.0], "metallicFactor": 1.0, "roughnessFactor": 0.2 } },
            ],
            "accessors": bin.accessors,
            "bufferViews": bin.views,
            "buffers": [{ "byteLength": bin.data.len().next_multiple_of(4) }],
        });
        pfx_build::library::join_glb(&json, &bin.data)
    }
}
