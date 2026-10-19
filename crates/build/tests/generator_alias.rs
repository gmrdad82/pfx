use pfx_build::{Registry, gltf_io};
use pfx_recipe::{Kind, load};
use std::path::PathBuf;

fn written(name: &str, generator: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("generator_alias");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{name}.toml"));
    std::fs::write(
        &file,
        format!(
            "recipe = 1\nname = \"street\"\n\n[[route]]\nname = \"main\"\npoints = [[0.0, 0.0, 0.0], [0.0, 0.0, 40.0], [12.0, 0.0, 80.0]]\n\n[[object]]\nname = \"street\"\nkind = \"generator\"\ngenerator = \"{generator}\"\nparams = {{ route = \"main\" }}\n"
        ),
    )
    .unwrap();
    file
}

#[test]
fn an_old_manfred_generator_name_reads_as_its_pfx_name_and_builds_the_same_glb() {
    let old = written("old", "manfred/road");
    let new = written("new", "pfx/road");
    let read = load(&old).unwrap().recipe;
    let Kind::Generator { generator, .. } = &read.objects[0].kind else {
        panic!("the object is a generator");
    };
    assert_eq!(generator, "pfx/road");
    let registry = Registry::new();
    assert_eq!(
        gltf_io::export_glb(&old, &registry, None).unwrap(),
        gltf_io::export_glb(&new, &registry, None).unwrap()
    );
}
