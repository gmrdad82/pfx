use pfx_materials::PackedMaterial;

pub const HOT_ROWS: usize = 8;
pub const MAP_ROWS: usize = 8;
pub const LOBE_ROWS: usize = 3;
pub const LAYER_ROWS: usize = 5;
pub const LAYERS: u32 = 7;
pub const MAPS: u32 = 8;
pub const LOBES: u32 = 16;
pub const THIN: u32 = 32;
pub const ALPHA: u32 = 64;
pub const ALPHA_ROWS: usize = 1;
pub const FLAKES: u32 = 128;
pub const FLAKE_ROWS: usize = 2;

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Hot {
    pub base_roughness: [f32; 4],
    pub metal_spec_coat: [f32; 4],
    pub sheen_trans_ior_disp: [f32; 4],
    pub thick_sub_film: [f32; 4],
    pub tint_absorption: [f32; 4],
    pub emission_film: [f32; 4],
    pub extension: [f32; 4],
    pub wet: [f32; 4],
}

impl Hot {
    pub fn emission(&self) -> [f32; 3] {
        [
            self.emission_film[0],
            self.emission_film[1],
            self.emission_film[2],
        ]
    }

    pub fn flags(&self) -> u32 {
        self.extension[1].to_bits()
    }

    pub fn first(&self) -> u32 {
        self.extension[2].to_bits()
    }

    pub fn set_uvs1(&mut self, first: u32) {
        self.extension[3] = f32::from_bits(first);
    }
}

pub struct Records {
    hot: Vec<Hot>,
    extension: Vec<[f32; 4]>,
}

impl Records {
    pub fn split(materials: &[PackedMaterial], effects: Hot) -> Self {
        let mut hot = Vec::with_capacity(materials.len() + 1);
        hot.push(effects);
        let mut extension = Vec::new();
        let first = (materials.len() + 1) * HOT_ROWS;
        for packed in materials {
            let (record, rows) = hot_of(packed, (first + extension.len()) as u32);
            extension.extend(rows);
            hot.push(record);
        }
        Self { hot, extension }
    }

    pub fn hot(&self, material: usize) -> &Hot {
        &self.hot[material + 1]
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = bytemuck::cast_slice(&self.hot).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&self.extension));
        bytes
    }

    pub fn len(&self) -> usize {
        self.hot.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn maps_on(packed: &PackedMaterial) -> bool {
    packed.textures.iter().any(|&slot| slot >= 0.0) || packed.texture_extra[0] >= 0.0
}

pub fn lobes_on(packed: &PackedMaterial) -> bool {
    [packed.sheen, packed.lobes, packed.scatter]
        .iter()
        .flatten()
        .any(|value| value.to_bits() != 0)
}

pub fn thin_on(packed: &PackedMaterial) -> bool {
    packed.tail[1] != 0.0 && packed.sheen_trans_ior_disp[1] > 0.0
}

pub fn alpha_on(packed: &PackedMaterial) -> bool {
    packed.coverage[2] != 0.0
}

pub fn flakes_on(packed: &PackedMaterial) -> bool {
    packed.flakes[0] > 0.0 && packed.flakes[1] > 0.0
}

fn layer_on(row: [f32; 4]) -> bool {
    row[0] > 0.0 && row[2] != 0.0
}

fn hot_of(packed: &PackedMaterial, first: u32) -> (Hot, Vec<[f32; 4]>) {
    let mut rows = Vec::new();
    let mut flags = 0;
    if maps_on(packed) {
        flags |= MAPS;
        rows.extend([
            packed.textures,
            packed.texture_scales,
            packed.texture_extra,
            packed.texture_transform,
            packed.detail_transform,
            packed.normal_transform,
            packed.orm_transform,
            packed.map_turns,
        ]);
    }
    if lobes_on(packed) {
        flags |= LOBES;
        rows.extend([packed.sheen, packed.lobes, packed.scatter]);
    }
    if thin_on(packed) {
        flags |= THIN;
    }
    let layers = [packed.layer0, packed.layer1, packed.layer2, packed.layer3];
    for (index, layer) in layers.into_iter().enumerate() {
        if layer_on(layer) {
            flags += 1;
            rows.push(layer);
            rows.extend_from_slice(&packed.params[index * 4..index * 4 + 4]);
        }
    }
    if alpha_on(packed) {
        flags |= ALPHA;
        rows.push(packed.coverage);
    }
    if flakes_on(packed) {
        flags |= FLAKES;
        rows.extend([packed.flakes, packed.flake_color]);
    }
    let first = if rows.is_empty() { 0 } else { first };
    let hot = Hot {
        base_roughness: packed.base_roughness,
        metal_spec_coat: packed.metal_spec_coat,
        sheen_trans_ior_disp: packed.sheen_trans_ior_disp,
        thick_sub_film: packed.thick_sub_film,
        tint_absorption: packed.tint_absorption,
        emission_film: packed.emission_film,
        extension: [
            packed.age_b[3],
            f32::from_bits(flags),
            f32::from_bits(first),
            0.0,
        ],
        wet: packed.wet,
    };
    (hot, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn extension(records: &Records, material: usize) -> Vec<[f32; 4]> {
        let hot = records.hot(material);
        let flags = hot.flags();
        let rows = (flags & MAPS != 0) as usize * MAP_ROWS
            + (flags & LOBES != 0) as usize * LOBE_ROWS
            + (flags & LAYERS) as usize * LAYER_ROWS
            + (flags & ALPHA != 0) as usize * ALPHA_ROWS
            + (flags & FLAKES != 0) as usize * FLAKE_ROWS;
        let first = hot.first() as usize - records.hot.len() * HOT_ROWS;
        records.extension[first..first + rows].to_vec()
    }

    #[test]
    fn the_hot_record_is_one_cache_line_and_the_shader_reads_every_field_it_read_before() {
        assert_eq!(std::mem::size_of::<Hot>(), HOT_ROWS * 16);
        assert_eq!(std::mem::size_of::<Hot>(), 128);
        let mut plain = PackedMaterial::zeroed();
        plain.base_roughness = [0.5, 0.4, 0.3, 0.2];
        plain.emission_film = [1.0, 2.0, 3.0, 0.5];
        plain.age_b = [9.0, 9.0, 9.0, 5.0];
        plain.textures = [-1.0; 4];
        plain.texture_extra = [-1.0, 0.0, 0.0, 0.0];
        plain.wet = [0.3, 0.0, 2.0, 1.0];
        let mut rich = plain;
        rich.textures = [-1.0, 2.0, -1.0, -1.0];
        rich.texture_scales = [1.0, 0.5, 1.0, 1.0];
        rich.texture_transform = [0.0, 0.0, 2.0, 2.0];
        rich.lobes = [0.0, -0.0, 0.0, 0.0];
        rich.layer1 = [3.0, 4.0, 0.5, f32::from_bits(7)];
        rich.layer2 = [3.0, 4.0, 0.0, 0.0];
        for (row, value) in rich.params.iter_mut().enumerate() {
            *value = [row as f32; 4];
        }
        let records = Records::split(&[plain, rich], Hot::default());
        assert_eq!(records.len(), 2);
        assert_eq!(
            records.bytes().len(),
            (3 * HOT_ROWS + MAP_ROWS + LOBE_ROWS + LAYER_ROWS) * 16
        );
        let hot = records.hot(0);
        assert_eq!(hot.base_roughness, plain.base_roughness);
        assert_eq!(hot.emission(), [1.0, 2.0, 3.0]);
        assert_eq!(hot.extension[0], 5.0);
        assert_eq!(hot.wet, plain.wet);
        assert_eq!((hot.flags(), hot.first()), (0, 0));
        assert_eq!(records.hot(1).flags(), MAPS | LOBES | 1);
        let mut lens = plain;
        lens.sheen_trans_ior_disp = [0.0, 1.0, 1.5, 0.0];
        lens.tail[1] = 1.0;
        let mut opaque = lens;
        opaque.sheen_trans_ior_disp[1] = 0.0;
        let thin = Records::split(&[lens, opaque], Hot::default());
        assert_eq!((thin.hot(0).flags(), thin.hot(0).first()), (THIN, 0));
        assert_eq!(thin.hot(1).flags(), 0);
        assert_eq!(thin.bytes().len(), 3 * HOT_ROWS * 16);
        let mut decal = rich;
        decal.coverage = [0.8, 0.3, 1.0, 0.0];
        let alpha = Records::split(&[plain, decal], Hot::default());
        assert_eq!(alpha.hot(1).flags(), MAPS | LOBES | ALPHA | 1);
        let rows = extension(&alpha, 1);
        assert_eq!(rows.len(), MAP_ROWS + LOBE_ROWS + LAYER_ROWS + ALPHA_ROWS);
        assert_eq!(rows[rows.len() - 1], decal.coverage);
        assert_eq!(rows[..rows.len() - 1], extension(&records, 1)[..]);
        let mut paint = decal;
        paint.flakes = [0.3, 0.001, 0.25, 0.2];
        paint.flake_color = [0.9, 0.8, 0.6, 0.0];
        let flaked = Records::split(&[paint], Hot::default());
        assert_eq!(flaked.hot(0).flags(), MAPS | LOBES | ALPHA | FLAKES | 1);
        let rows = extension(&flaked, 0);
        assert_eq!(
            rows[rows.len() - 3..],
            [decal.coverage, paint.flakes, paint.flake_color]
        );
        let mut sized = plain;
        sized.flakes = [0.3, 0.0, 0.25, 0.2];
        assert_eq!(Records::split(&[sized], Hot::default()).hot(0).flags(), 0);
        assert_eq!(
            extension(&records, 1),
            [
                rich.textures,
                rich.texture_scales,
                rich.texture_extra,
                rich.texture_transform,
                rich.detail_transform,
                rich.normal_transform,
                rich.orm_transform,
                rich.map_turns,
                rich.sheen,
                rich.lobes,
                rich.scatter,
                rich.layer1,
                rich.params[4],
                rich.params[5],
                rich.params[6],
                rich.params[7],
            ]
        );
    }
}
