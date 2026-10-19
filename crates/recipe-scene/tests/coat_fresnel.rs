use std::path::{Path, PathBuf};

const SIZE: [u32; 2] = [320, 180];

fn recipe(power: f32) -> String {
    format!(
        r#"recipe = 1
name = "lacquer"

[[object]]
name = "slab"
kind = "box"
at = [0.0, 0.45, 1.5]
size = [1.4, 0.1, 3.0]
material = "lacquer"

[[camera]]
name = "behind"
position = [0.0, 1.15, -1.0]
target = [0.0, 0.5, 3.0]
fov_y_deg = 45.0

[sun]
model = "authored"
toward = [0.3, 0.2, 0.9]
irradiance = 0.0

[sky]
kind = "hdr"
path = "window.hdr"
source = "made by pfx for its own tests"

[materials.lacquer]
base = [0.03, 0.03, 0.03]
roughness = 0.45
metalness = 0.3
clearcoat = 1.0
clearcoat_roughness = 0.03
fresnel_power = {power:.1}
"#
    )
}

fn still(power: f32) -> Vec<f32> {
    let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("coat-fresnel")
        .join(format!("{power}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sky");
    std::fs::copy(fixtures.join("window.hdr"), root.join("window.hdr")).unwrap();
    std::fs::write(root.join("still.toml"), recipe(power)).unwrap();
    let (recipe, built) = pfx_recipe_scene::load_at(
        &root.join("still.toml"),
        None,
        &[],
        &pfx_recipe::Pick::default(),
        &pfx_build::Registry::new(),
        None,
    )
    .unwrap();
    let gpu = pfx_recipe_scene::headless().unwrap();
    pfx_recipe_scene::shoot_posed(&gpu, (&recipe, &built), "behind", SIZE, None)
        .unwrap()
        .chunks_exact(4)
        .map(|texel| {
            0.2126 * f32::from(texel[0])
                + 0.7152 * f32::from(texel[1])
                + 0.0722 * f32::from(texel[2])
        })
        .collect()
}

fn mean(image: &[f32], [x0, y0, x1, y1]: [u32; 4]) -> f32 {
    let mut total = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            total += image[(y * SIZE[0] + x) as usize];
        }
    }
    total / ((x1 - x0) * (y1 - y0)) as f32
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_steeper_fresnel_power_darkens_a_coated_slab_at_grazing_and_nothing_else() {
    const FLANK: [u32; 4] = [150, 95, 190, 107];
    const SKY: [u32; 4] = [0, 0, 320, 40];
    let five = still(5.0);
    let twelve = still(12.0);
    let (flank5, flank12) = (mean(&five, FLANK), mean(&twelve, FLANK));
    println!("grazing flank: {flank5:.1} at power 5, {flank12:.1} at power 12");
    assert!(
        flank12 < 0.8 * flank5,
        "power 12 should darken the grazing flank: {flank5:.1} at 5, {flank12:.1} at 12"
    );
    assert_eq!(mean(&five, SKY), mean(&twelve, SKY), "the sky is unchanged");
}
