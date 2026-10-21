use glam::{DMat4, DVec3};
use pfx_load::scene::Scene;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("scatter_scene")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn recipe(scale: &str, extra: &str) -> String {
    format!(
        "recipe = 1\nname = \"field\"\nseed = 4\n\n[output]\nscene = \"field.scene.toml\"\n\n[[object]]\nname = \"ground\"\nkind = \"plane\"\nsize = [6.0, 4.0]\n\n[[object]]\nname = \"shelf\"\nkind = \"empty\"\nat = [3.0, 2.0, -1.0]\nrotate = [0.0, 30.0, 0.0]\n\n[[object]]\nname = \"stone\"\nkind = \"box\"\nsize = [0.2, 0.1, 0.3]\nhidden = true\nat = [1.0, -5.0, 2.0]\nrotate = [0.0, 0.0, 90.0]\nscale = {scale}\n{extra}\n[[scatter]]\nname = \"stones\"\nobject = \"stone\"\non = \"ground\"\nspacing = 0.6\nrotate_max = [0.0, 360.0, 0.0]\nscale_min = 0.8\nscale_max = 1.2\n\n[[camera]]\nname = \"main\"\nposition = [0.0, 5.0, 5.0]\ntarget = [0.0, 0.0, 0.0]\nfov_y_deg = 40.0\n"
    )
}

fn world(matrix: DMat4, positions: &[[f32; 3]]) -> Vec<DVec3> {
    positions
        .iter()
        .map(|p| matrix.transform_point3(DVec3::new(p[0].into(), p[1].into(), p[2].into())))
        .collect()
}

#[test]
fn scattered_copies_drop_their_sources_position_in_the_glb_and_the_scene_alike() {
    let even = "[1.5, 1.5, 1.5]";
    let uneven = "[2.0, 1.0, 1.0]";
    for (name, scale, extra, tall) in [
        ("baked", even, "", 0.3),
        ("kept", even, "keep_transform = true\n", 0.3),
        ("parented", even, "parent = \"shelf\"\n", 0.3),
        ("uneven", uneven, "keep_transform = true\n", 0.4),
        ("uneven-baked", uneven, "", 0.4),
    ] {
        let dir = scratch(name);
        let recipe = pfx_recipe::parse(&recipe(scale, extra)).unwrap();
        let built = pfx_build::build(&recipe, &dir).unwrap();
        pfx_build::write(&built, &recipe, &dir, None).unwrap();
        let scene = Scene::open(dir.join("field.scene.toml")).unwrap();
        let draws = scene.draws();
        let copies: Vec<_> = built
            .parts
            .iter()
            .filter(|part| part.node.name.starts_with("stones."))
            .collect();
        assert!(copies.len() > 20, "{name}: {}", copies.len());
        assert!(
            draws.items.iter().all(|draw| draw.object != "stone"),
            "{name}: the hidden source is drawn"
        );
        for part in copies {
            let placed = world(part.node.transform, &part.node.mesh.positions);
            let low = placed.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
            let high = placed.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
            assert!(
                (low + high).abs() < 1e-5,
                "{name} {}: the copy sits on the ground, not at the source's height: {low} to {high}",
                part.node.name
            );
            let height = high - low;
            assert!(
                (tall * 0.8 - 1e-5..=tall * 1.2 + 1e-5).contains(&height),
                "{name} {}: the source's turn and scale stay on the copy: {height}",
                part.node.name
            );
            let draw = draws
                .items
                .iter()
                .find(|draw| draw.object == part.node.name)
                .unwrap_or_else(|| panic!("{name}: no draw for {}", part.node.name));
            let model = DMat4::from_cols_array_2d(&draw.model.map(|c| c.map(f64::from)));
            let live = world(model, &draw.geometry.positions);
            assert_eq!(live.len(), placed.len());
            for (a, b) in live.iter().zip(&placed) {
                assert!(
                    a.distance(*b) < 1e-4,
                    "{name} {}: live {a} against the glb's {b}",
                    part.node.name
                );
            }
        }
        let again = pfx_build::build(&recipe, &dir).unwrap();
        assert_eq!(again.glb(), built.glb(), "{name}");
    }
}
