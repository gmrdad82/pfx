use pfx_build::Registry;
use pfx_recipe::Pick;
use pfx_still::{Request, View, looked_with};
use std::path::{Path, PathBuf};

const RECIPE: &str = r#"recipe = 1
name = "cutout"

[sun]
model = "authored"
toward = [0.3, 0.9, 0.2]
irradiance = 3.0

[[camera]]
name = "top"
kind = "orthographic"
position = [0.0, 2.0, 0.0]
target = [0.0, 0.0, 0.0]
up = [0.0, 0.0, -1.0]
ortho_height = 1.0

[materials.grey]
base = [0.5, 0.5, 0.5]
roughness = 0.8

[[object]]
name = "tile"
kind = "box"
size = [0.4, 0.1, 0.4]
rotate = [0.0, 30.0, 0.0]
material = "grey"
"#;

const SIZE: [u32; 2] = [64, 64];

fn scene(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/still-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cutout.toml"), RECIPE).unwrap();
    dir.join("cutout.toml")
}

fn traced(recipe: &Path, transparent: bool, denoise: bool) -> pfx_still::Rendered {
    let request = Request {
        recipe,
        camera: "top",
        size: SIZE,
        samples: 8,
        noise: None,
        seed: None,
        view: View::Lens,
        variant: None,
        set: &[],
        pick: &Pick::default(),
        crop: None,
        denoise,
        time: None,
        transparent,
    };
    pfx_still::render(&request, &Registry::new(), &mut |_| {}).unwrap()
}

fn at(x: u32, y: u32) -> usize {
    (y * SIZE[0] + x) as usize
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_transparent_trace_is_clear_where_it_misses_and_covered_where_it_hits() {
    let recipe = scene("transparent-traced");
    for denoise in [false, true] {
        let rendered = traced(&recipe, true, denoise);
        assert!(rendered.film.transparent);
        let film = &rendered.film;
        assert_eq!(film.color[at(1, 1)][3], 0.0);
        assert!(
            film.color[at(32, 32)][3] > 0.999,
            "{:?}",
            film.color[at(32, 32)]
        );
        let edges = film
            .color
            .iter()
            .filter(|p| p[3] > 0.01 && p[3] < 0.99)
            .count();
        assert!(edges > 0);
        let rgba = looked_with(film, (&rendered.finish, &rendered.overlay), denoise).unwrap();
        let pixel = |n: usize| &rgba[n * 4..n * 4 + 4];
        assert_eq!(pixel(at(1, 1)), [0; 4]);
        assert_eq!(pixel(at(SIZE[0] - 2, SIZE[1] - 2)), [0; 4]);
        assert_eq!(pixel(at(32, 32))[3], 255);
        assert!(pixel(at(32, 32))[..3].iter().any(|&c| c > 0));
        for (n, given) in film.color.iter().enumerate() {
            let alpha = pixel(n)[3];
            assert!(
                (f32::from(alpha) - given[3].clamp(0.0, 1.0) * 255.0).abs() <= 1.0,
                "{n}: {alpha} {given:?}"
            );
        }
    }
    let opaque = traced(&recipe, false, true);
    assert!(!opaque.film.transparent);
    let rgba = looked_with(&opaque.film, (&opaque.finish, &opaque.overlay), true).unwrap();
    assert!(rgba.chunks_exact(4).all(|p| p[3] == 255));
}
