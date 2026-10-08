use super::*;
use pfx_materials::Library;

const ROOM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/box-room");
const SKY: &[u8] = pfx_load::fixture::SKY_HDR;
const FIXTURE_BYTES: &[u8] = pfx_materials::FIXTURE.as_bytes();
const HEAD: &str = "scene = \"scene.gltf\"\nanchors = [9.0, 16.0]\nsamples = 1\nseed = 5\n";
const VOLUME: &str =
    "\n[[volumes]]\nname = \"v\"\nmin = [0.0, 0.0, 0.0]\nmax = [0.0, 0.0, 0.0]\nspacing = 1.0\n";
const ROOM_SKY: &str = "\n[sky.room]\nwidth = 32\nfloor = [0.3, 0.3, 0.3]\nwall = [0.5, 0.5, 0.5]\nceiling = [0.8, 0.8, 0.8]\n\n[[sky.room.lights]]\naz = 10.0\nel = 20.0\npower = 3.0\n";
const AUTHORED: &str = "toward = [0.3, 0.7, 0.5]\ncolor = [1.0, 0.9, 0.8]\nirradiance = 2.5\n";
const SHORT_CLOCK: &str = "\n[clock]\ntoward = [0.25, 0.85, 0.45]\nsun = 1.5\ntint = 0.4\nelevation = 2.0\nazimuth = -3.0\n";
const CLOCK: &str = "\n[clock]\ntoward = [0.25, 0.85, 0.45]\nsun = 2.5\ntint = 0.4\nelevation = 2.0\nazimuth = -3.0\nskywarm = 0.1\nlamp = 3.0\nexposure = 1.0\nwarmth = 0.5\nturn = 0.6\n\n[clock.arc]\nstart = 5.0\nspan = 15.0\nelevation = 50.0\nazimuth = -70.0\nsweep = 140.0\nreference = 10.0\n";

fn scratch(label: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp")
        .join(format!("bake-keys-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn scene_gltf(materials: &[&str]) -> String {
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    let data: Vec<u8> = positions
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .chain([0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    let encoded = {
        const SET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for k in 0..4 {
                out.push(if k <= chunk.len() {
                    SET[(n >> (18 - 6 * k) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    };
    let names: Vec<String> = materials
        .iter()
        .map(|name| format!("{{\"name\":\"{name}\"}}"))
        .collect();
    let meshes: Vec<String> = (0..materials.len())
        .map(|i| format!("{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1,\"material\":{i}}}]}}"))
        .chain(std::iter::once(
            "{\"primitives\":[{\"attributes\":{\"POSITION\":0},\"indices\":1}]}".to_owned(),
        ))
        .collect();
    let nodes: Vec<String> = (0..meshes.len())
        .map(|i| format!("{{\"name\":\"n{i}\",\"mesh\":{i}}}"))
        .collect();
    let roots: Vec<String> = (0..nodes.len()).map(|i| i.to_string()).collect();
    format!(
        "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[{}]}}],\"nodes\":[{}],\"materials\":[{}],\"meshes\":[{}],\"buffers\":[{{\"uri\":\"data:application/octet-stream;base64,{encoded}\",\"byteLength\":42}}],\"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36}},{{\"buffer\":0,\"byteOffset\":36,\"byteLength\":6}}],\"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":[0,0,0],\"max\":[1,0,1]}},{{\"bufferView\":1,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}}]}}",
        roots.join(","),
        nodes.join(","),
        names.join(","),
        meshes.join(",")
    )
}

fn folder(label: &str, recipe: &str, materials: &[&str], files: &[(&str, &[u8])]) -> PathBuf {
    let path = scratch(label);
    fs::write(path.join("bake.toml"), recipe).unwrap();
    fs::write(path.join("scene.gltf"), scene_gltf(materials)).unwrap();
    for (name, bytes) in files {
        let file = path.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }
    path
}

fn load(label: &str, recipe: &str, materials: &[&str], files: &[(&str, &[u8])]) -> SceneSource {
    let path = folder(label, recipe, materials, files);
    let source = SceneSource::load(&path).unwrap_or_else(|e| panic!("{label}: {e}"));
    fs::remove_dir_all(path).unwrap();
    source
}

fn refused(label: &str, recipe: &str, files: &[(&str, &[u8])], wants: &str) {
    let path = folder(label, recipe, &["glass"], files);
    let error = SceneSource::load(&path).err().unwrap_or_default();
    fs::remove_dir_all(path).unwrap();
    assert!(error.contains(wants), "{label}: {error:?} lacks {wants:?}");
}

fn same(old: &SceneSource, new: &SceneSource) {
    let spec = GridSpec {
        min: [0.0; 3],
        max: [0.0; 3],
        spacing: 1.0,
    };
    assert_eq!(old.scene.triangles, new.scene.triangles);
    assert_eq!(old.scene.materials, new.scene.materials);
    assert_eq!(old.emitters, new.emitters);
    assert_eq!(old.scene.anchors.len(), new.scene.anchors.len());
    for (a, b) in old.scene.anchors.iter().zip(&new.scene.anchors) {
        assert_eq!(a.hour, b.hour);
        assert_eq!(a.sun.direction, b.sun.direction);
        assert_eq!(a.sun.color, b.sun.color);
        assert_eq!(a.sun.intensity, b.sun.intensity);
        assert_eq!(a.sky, b.sky);
    }
    assert_eq!(
        scene_hash(&old.scene, spec, 1, 1).unwrap(),
        scene_hash(&new.scene, spec, 1, 1).unwrap()
    );
    let hours = [6.5, 12.0, 20.25];
    let (a, b) = (old.with_hours(&hours), new.with_hours(&hours));
    if let (Ok(a), Ok(b)) = (a, b) {
        for (a, b) in a.iter().zip(&b) {
            assert_eq!(a.sky, b.sky);
            assert_eq!(a.sun.direction, b.sun.direction);
            assert_eq!(a.sun.intensity, b.sun.intensity);
        }
    }
}

fn alias(label: &str, old: &str, new: &str, materials: &[&str], files: &[(&str, &[u8])]) {
    let a = load(&format!("{label}-old"), old, materials, files);
    let b = load(&format!("{label}-new"), new, materials, files);
    same(&a, &b);
    assert_eq!(a.recipe.product, b.recipe.product);
}

#[test]
fn app_takes_any_slug_and_product_stays_its_alias() {
    let room = fs::read_to_string(Path::new(ROOM).join("bake.toml")).unwrap();
    let app = room.replace("app = \"test\"", "app = \"box-room-2\"");
    let recipe = Recipe::parse(app.as_bytes()).unwrap();
    assert_eq!(recipe.product, "box-room-2");
    assert_eq!(recipe.sun, SunModel::Solar);
    assert_eq!(recipe.sky.kind, SkyKind::Daylight);
    let parse = |text: String| Recipe::parse(text.as_bytes()).map(|_| ()).unwrap_err();
    assert!(parse(room.replace("app = \"test\"", "")).contains("app"));
    assert!(parse(room.replace("app = \"test\"", "app = \"Box\"")).contains("slug"));
    assert!(parse(room.replace("app = \"test\"", "app = \"\"")).contains("slug"));
    assert!(
        parse(room.replace("app = \"test\"", "app = \"box\"\nproduct = \"box\""))
            .contains("keep app")
    );
    alias(
        "product-some-game",
        &format!("{HEAD}product = \"some-game\"\n{VOLUME}"),
        &format!("{HEAD}app = \"some-game\"\n{VOLUME}"),
        &["glass"],
        &[],
    );
    assert!(
        Recipe::parse(
            format!("{HEAD}product = \"some-game\"\n{VOLUME}\n[sun]\n{AUTHORED}").as_bytes()
        )
        .unwrap_err()
        .contains("needs model")
    );
}

#[test]
fn a_library_names_the_materials_its_fallback_tints_poses_and_emitters() {
    let fixture = Library::fixture();
    let lib: [(&str, &[u8]); 1] = [("materials.toml", FIXTURE_BYTES)];
    let recipe = format!(
        "{HEAD}app = \"test\"\nmaterials = \"materials.toml\"\nfallback = \"grey\"\n{VOLUME}"
    );
    let source = load("library", &recipe, &["glass", "wood.002", "nothing"], &lib);
    let m = &source.scene.materials;
    assert_eq!(m.len(), 4);
    assert_eq!(m[0], *fixture.get("glass").unwrap());
    assert_eq!(m[1], *fixture.get("wood").unwrap());
    assert_eq!(m[2], *fixture.get("grey").unwrap());
    assert_eq!(m[3], *fixture.get("grey").unwrap());
    let bare = recipe.replace("fallback = \"grey\"\n", "");
    let source = load("library-bare", &bare, &["glass", "nothing"], &lib);
    assert_eq!(source.scene.materials[1], Material::default());
    assert_eq!(source.scene.materials[2], Material::default());
    let tinted = format!("{recipe}\n[tint]\nglass = [0.5, 1.0, 0.25]\ngrey = [2.0, 2.0, 2.0]\n");
    let source = load("library-tint", &tinted, &["glass", "nothing"], &lib);
    let glass = fixture.get("glass").unwrap();
    assert_eq!(
        source.scene.materials[0].base,
        [glass.base[0] * 0.5, glass.base[1], glass.base[2] * 0.25]
    );
    assert_eq!(source.scene.materials[0].ior, glass.ior);
    let grey = fixture.get("grey").unwrap();
    assert_eq!(source.scene.materials[1].base, grey.base.map(|v| v * 2.0));
    let posed = format!(
        "{recipe}\n[[pose]]\nnode = \"n0\"\ntranslation = [0.0, 1.0, 0.0]\nmaterials = {{ glass = \"felt\" }}\n\n[[emitter]]\nposition = [0.0, 2.0, 0.0]\nradius = 0.1\ncolor = [1.0, 1.0, 1.0]\nintensity = 1.0\npreset = \"metal\"\n\n[[emitter]]\nposition = [0.0, 3.0, 0.0]\nradius = 0.1\ncolor = [1.0, 1.0, 1.0]\nintensity = 1.0\n"
    );
    let source = load("library-pose", &posed, &["glass"], &lib);
    let posed_glass = source
        .scene
        .triangles
        .iter()
        .find(|t| t.vertices[0][1] == 1.0)
        .unwrap();
    let felt = fixture.get("felt").unwrap();
    assert_eq!(source.scene.materials[posed_glass.material as usize], *felt);
    assert!(!source.scene.materials.contains(glass));
    let metal = fixture.get("metal").unwrap();
    let first = source.scene.materials[source.emitters[0]];
    assert_eq!(first.metalness, metal.metalness);
    assert_eq!(first.family, metal.family);
    assert_eq!(first.base, [0.0; 3]);
    let second = source.scene.materials[source.emitters[1]];
    assert_eq!(second.family, grey.family);
    assert_eq!(second.specular, grey.specular);
}

#[test]
fn a_library_is_hashed_and_can_live_beside_the_recipe_folder() {
    let shared = scratch("library-shared");
    fs::write(shared.join("shared.toml"), FIXTURE_BYTES).unwrap();
    let name = shared.file_name().unwrap().to_str().unwrap().to_owned();
    let recipe = format!(
        "{HEAD}app = \"test\"\nmaterials = \"../{name}/shared.toml\"\nfallback = \"grey\"\n{VOLUME}"
    );
    let path = folder("library-parent", &recipe, &["glass"], &[]);
    let source = SceneSource::load(&path).unwrap();
    assert_eq!(
        source.scene.materials[0],
        *Library::fixture().get("glass").unwrap()
    );
    let spec = source.recipe.volumes[0].grid;
    let first = source.hash(spec, 1).unwrap();
    assert_eq!(
        SceneSource::load(&path).unwrap().hash(spec, 1).unwrap(),
        first
    );
    let edited = String::from_utf8(FIXTURE_BYTES.to_vec()).unwrap() + "\n[materials.extra]\n";
    fs::write(shared.join("shared.toml"), edited).unwrap();
    let after = SceneSource::load(&path).unwrap();
    assert_eq!(after.scene.materials, source.scene.materials);
    assert_ne!(after.hash(spec, 1).unwrap(), first);
    fs::remove_dir_all(path).unwrap();
    fs::remove_dir_all(shared).unwrap();
}

#[test]
fn library_keys_are_refused_when_they_name_nothing() {
    let lib: [(&str, &[u8]); 1] = [("materials.toml", FIXTURE_BYTES)];
    let base = format!("{HEAD}app = \"test\"\nmaterials = \"materials.toml\"\n{VOLUME}");
    refused(
        "fallback-missing",
        &base.replace(
            "app = \"test\"\n",
            "app = \"test\"\nfallback = \"marble\"\n",
        ),
        &lib,
        "fallback marble",
    );
    refused(
        "tint-missing",
        &format!("{base}\n[tint]\nmarble = [1.0, 1.0, 1.0]\n"),
        &lib,
        "tint names marble",
    );
    refused(
        "tint-negative",
        &format!("{base}\n[tint]\nglass = [1.0, -1.0, 1.0]\n"),
        &lib,
        "tint",
    );
    refused(
        "pose-missing",
        &format!("{base}\n[[pose]]\nnode = \"n0\"\nmaterials = {{ glass = \"marble\" }}\n"),
        &lib,
        "pose material marble",
    );
    refused(
        "pose-stock",
        &format!("{base}\n[[pose]]\nnode = \"n0\"\n\n[pose.stock]\nroughness = 0.5\n"),
        &lib,
        "unknown field `stock`",
    );
    refused(
        "emitter-missing",
        &format!(
            "{base}\n[[emitter]]\nposition = [0.0, 1.0, 0.0]\nradius = 0.1\ncolor = [1.0, 1.0, 1.0]\nintensity = 1.0\npreset = \"marble\"\n"
        ),
        &lib,
        "emitter preset marble",
    );
    refused("library-absent", &base, &[], "materials.toml");
    refused(
        "library-broken",
        &base,
        &[("materials.toml", b"[materials.a]\nbsae = 1\n")],
        "bsae",
    );
    refused(
        "library-absolute",
        &base.replace("\"materials.toml\"", "\"/materials.toml\""),
        &lib,
        "relative",
    );
    refused(
        "library-name",
        &base.replace("\"materials.toml\"", "\"materials.json\""),
        &lib,
        "library .toml",
    );
    refused(
        "set-refused",
        &base.replace("\"materials.toml\"", "\"old\""),
        &lib,
        "library .toml",
    );
    let plain = format!("{HEAD}app = \"test\"\n{VOLUME}");
    refused(
        "fallback-alone",
        &plain.replace("app = \"test\"\n", "app = \"test\"\nfallback = \"grey\"\n"),
        &lib,
        "fallback needs materials",
    );
    refused(
        "tint-alone",
        &format!("{plain}\n[tint]\ngrey = [1.0, 1.0, 1.0]\n"),
        &lib,
        "[tint] needs materials",
    );
    refused(
        "stock-library",
        &format!("{base}\n[stock]\nroughness = 0.5\n"),
        &lib,
        "unknown field `stock`",
    );
    refused(
        "pose-materials-alone",
        &format!("{plain}\n[[pose]]\nnode = \"n0\"\nmaterials = {{ glass = \"grey\" }}\n"),
        &lib,
        "pose materials need",
    );
}

#[test]
fn every_sun_model_lights_by_its_values() {
    alias(
        "sun-solar",
        &format!("{HEAD}app = \"some-game\"\n{VOLUME}"),
        &format!("{HEAD}app = \"some-game\"\n{VOLUME}\n[sun]\nmodel = \"solar\"\n"),
        &["glass"],
        &[],
    );
    let fixed = load(
        "sun-fixed",
        &format!("{HEAD}app = \"test\"\n{VOLUME}\n[sun]\nmodel = \"fixed\"\n{AUTHORED}"),
        &["glass"],
        &[],
    );
    let length = (0.3f64 * 0.3 + 0.7 * 0.7 + 0.5 * 0.5).sqrt();
    for anchor in &fixed.scene.anchors {
        let daylight = Daylight {
            hour: f64::from(anchor.hour),
            day: Daylight::DAY,
            latitude: Daylight::LATITUDE,
            heading: Daylight::HEADING,
        };
        assert_eq!(
            anchor.sun.direction,
            [0.3, 0.7, 0.5].map(|v: f64| (v / length) as f32)
        );
        assert_eq!(anchor.sun.color, [1.0, 0.9, 0.8]);
        assert_eq!(
            anchor.sun.intensity,
            2.5 * daylight.light(REFERENCE_HOUR).intensity as f32
        );
    }
    let source = load(
        "sun-clock",
        &format!("{HEAD}app = \"test\"\n{VOLUME}\n[sun]\nmodel = \"clock\"\n{CLOCK}"),
        &["glass"],
        &[],
    );
    let SunModel::Clock { clock, base } = source.recipe.sun else {
        panic!("{:?}", source.recipe.sun);
    };
    assert_eq!(base, [0.25, 0.85, 0.45]);
    assert_eq!((clock.sun, clock.turn, clock.skywarm), (2.5, 0.6, 0.1));
    for anchor in &source.scene.anchors {
        let damped = clock.at(f64::from(anchor.hour), base);
        assert_eq!(anchor.sun.color, damped.colour.map(|v| v as f32));
        assert_eq!(
            anchor.sun.direction,
            compose::unit(damped.toward.map(|v| v as f32))
        );
    }
}

#[test]
fn sun_and_clock_keys_are_refused_when_malformed() {
    let base = format!("{HEAD}app = \"test\"\n{VOLUME}");
    let parse = |extra: &str| Recipe::parse(format!("{base}{extra}").as_bytes()).unwrap_err();
    assert!(parse("\n[sun]\nmodel = \"moon\"\n").contains("not 'moon'"));
    assert!(parse(&format!("\n[sun]\n{AUTHORED}")).contains("needs model"));
    assert!(parse(SHORT_CLOCK).contains("model = 'clock'"));
    assert!(parse(&format!("\n[sun]\nmodel = \"solar\"\n{AUTHORED}")).contains("takes no"));
    assert!(
        parse("\n[sun]\nmodel = \"authored\"\ntoward = [0.0, 1.0, 0.0]\n")
            .contains("needs toward, color and irradiance")
    );
    assert!(
        parse("\n[sun]\nmodel = \"fixed\"\ntoward = [0.0, 0.0, 0.0]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 1.0\n")
            .contains("finite toward")
    );
    assert!(parse("\n[sun]\nmodel = \"clock\"\n").contains("[clock] table"));
    assert!(
        parse(&format!("\n[sun]\nmodel = \"clock\"\n{SHORT_CLOCK}"))
            .contains("needs toward, sun, tint")
    );
    assert!(
        parse(&format!(
            "\n[sun]\nmodel = \"clock\"\n{AUTHORED}{SHORT_CLOCK}"
        ))
        .contains("takes no")
    );
    assert!(
        parse(&format!(
            "\n[sun]\nmodel = \"authored\"\n{AUTHORED}{SHORT_CLOCK}"
        ))
        .contains("cannot both")
    );
    let clocked = format!("\n[sun]\nmodel = \"clock\"\n{CLOCK}");
    assert!(Recipe::parse(format!("{base}{clocked}").as_bytes()).is_ok());
    assert!(parse(&clocked.replace("lamp = 3.0", "lamp = -3.0")).contains("clock needs"));
    assert!(parse(&clocked.replace("turn = 0.6", "turn = nan")).contains("clock needs"));
    let arcless = &clocked[..clocked.find("\n[clock.arc]").unwrap()];
    assert!(parse(arcless).contains("[clock.arc]"));
    assert!(parse(&clocked.replace("span = 15.0", "span = 0.0")).contains("positive span"));
    assert!(parse(&clocked.replace("sweep = 140.0", "sweep = 140.0\nnoon = 1.0")).contains("noon"));
    assert!(parse("\n[sun]\nmodel = \"solar\"\nsky_fill = 1.0\n").contains("takes no"));
    assert!(
        parse("\n[sun]\nmodel = \"fixed\"\ntoward = [0.0, 1.0, 0.0]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 1.0\nsky_fill = 1.0\n")
            .contains("sky_fill belongs")
    );
    assert!(
        parse("\n[sun]\nmodel = \"authored\"\ntoward = [0.0, 1.0, 0.0]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 1.0\nsky_fill = -1.0\n")
            .contains("sky_fill must be")
    );
    let late = format!("{HEAD}app = \"test\"\nreference_hour = 25.0\n{VOLUME}");
    assert!(
        Recipe::parse(late.as_bytes())
            .unwrap_err()
            .contains("reference_hour = 25")
    );
    let named = format!("{HEAD}product = \"some-game\"\n{VOLUME}");
    assert!(
        Recipe::parse(format!("{named}{SHORT_CLOCK}").as_bytes())
            .unwrap_err()
            .contains("model = 'clock'")
    );
    assert!(Recipe::parse(format!("{named}{clocked}").as_bytes()).is_ok());
}

#[test]
fn the_sky_kinds_name_their_skies_and_prepare_chains() {
    let hdr: [(&str, &[u8]); 1] = [("sky.hdr", SKY)];
    for (old, new) in [
        ("", "\n[sky]\nkind = \"daylight\"\n"),
        ("\n[sky]\nkind = \"analytic\"\n", ""),
    ] {
        alias(
            "sky-daylight",
            &format!("{HEAD}product = \"some-game\"\n{VOLUME}{old}"),
            &format!("{HEAD}app = \"some-game\"\n{VOLUME}{new}"),
            &["glass"],
            &hdr,
        );
    }
    let authored = format!("\n[sun]\nmodel = \"authored\"\n{AUTHORED}");
    let source = load(
        "sky-room",
        &format!(
            "{HEAD}app = \"test\"\n{VOLUME}{authored}\n[sky]\nkind = \"room\"\nrotation_deg = 40.0\nintensity = 1.5\nambient = \"authored\"\n{ROOM_SKY}"
        ),
        &["glass"],
        &hdr,
    );
    assert_eq!(source.recipe.sky.kind, SkyKind::Room);
    assert_eq!(source.recipe.sky.ambient, Some(Ambient::Authored));
    let chained = load(
        "sky-prepare-chain",
        &format!(
            "{HEAD}app = \"test\"\n{VOLUME}{authored}\n[sky]\nkind = \"hdr\"\npath = \"sky.hdr\"\nrotation_deg = 120.0\nintensity = 3.0\nambient = \"authored\"\n\n[sky.prepare]\ncap = 20.0\nbalance = [1.0, 0.9, 0.7]\nmean = 0.4\n"
        ),
        &["glass"],
        &hdr,
    );
    assert_eq!(
        chained.recipe.sky.prepare,
        Some(Prepare {
            cap: Some(20.0),
            balance: Some([1.0, 0.9, 0.7]),
            mean: Some(0.4),
        })
    );
    let base_sky = Sky::parse(SKY)
        .unwrap()
        .capped_luminance(20.0)
        .balanced_to([1.0, 0.9, 0.7])
        .scaled_to_mean_luminance(0.4)
        .rotated(-120.0_f32.to_radians())
        .exposed(3.0);
    let light = Daylight {
        hour: 9.0,
        day: Daylight::DAY,
        latitude: Daylight::LATITUDE,
        heading: Daylight::HEADING,
    }
    .authored(
        &Authored {
            toward: [0.3, 0.7, 0.5],
            colour: [1.0, 0.9, 0.8],
            sky_fill: 0.0,
        },
        REFERENCE_HOUR,
    );
    assert_eq!(
        chained.scene.anchors[0].sky,
        base_sky.exposed(light.ambient as f32)
    );
    let source = load(
        "sky-prepare-steps",
        &format!(
            "{HEAD}app = \"test\"\n{VOLUME}\n[sky]\nkind = \"hdr\"\npath = \"sky.hdr\"\nrotation_deg = 30.0\n\n[sky.prepare]\ncap = 2.0\nmean = 0.5\n"
        ),
        &["glass"],
        &hdr,
    );
    let expected = Sky::parse(SKY)
        .unwrap()
        .capped_luminance(2.0)
        .scaled_to_mean_luminance(0.5)
        .rotated(-30.0_f32.to_radians());
    assert!(
        source
            .scene
            .anchors
            .iter()
            .all(|anchor| anchor.sky == expected)
    );
}

#[test]
fn sky_keys_are_refused_when_malformed() {
    let base = format!("{HEAD}app = \"test\"\n{VOLUME}");
    let authored = format!("\n[sun]\nmodel = \"authored\"\n{AUTHORED}");
    let clock = format!("\n[sun]\nmodel = \"clock\"\n{CLOCK}");
    let hdr = "\n[sky]\nkind = \"hdr\"\npath = \"sky.hdr\"\n";
    let parse = |extra: String| Recipe::parse(format!("{base}{extra}").as_bytes()).unwrap_err();
    assert!(parse("\n[sky]\nkind = \"cloud\"\n".into()).contains("not 'cloud'"));
    assert!(parse("\n[sky]\nkind = \"hdr\"\n".into()).contains("needs a path"));
    assert!(parse("\n[sky]\nkind = \"room\"\n".into()).contains("[sky.room]"));
    assert!(
        parse(format!(
            "\n[sky]\nkind = \"room\"\n\n[sky.prepare]\nmean = 1.0\n{ROOM_SKY}"
        ))
        .contains("takes no prepare")
    );
    assert!(parse("\n[sky]\n\n[sky.prepare]\nmean = 1.0\n".into()).contains("kind = 'hdr'"));
    assert!(parse(format!("{hdr}\n[sky.prepare]\n")).contains("needs cap, balance or mean"));
    assert!(parse(format!("{hdr}\n[sky.prepare]\ncap = -1.0\n")).contains("finite and positive"));
    assert!(
        parse(format!("{hdr}\n[sky.prepare]\nbalance = [1.0, 0.0, 1.0]\n"))
            .contains("finite and positive")
    );
    assert!(parse(format!("{hdr}\n[sky.prepare]\nmeen = 1.0\n")).contains("meen"));
    assert!(
        parse(format!(
            "{hdr}balance = [1.0, 1.0, 1.0]\n\n[sky.prepare]\nmean = 1.0\n"
        ))
        .contains("own table")
    );
    assert!(parse(format!("{hdr}prepare = \"bright\"\n")).contains("not 'bright'"));
    assert!(parse(format!("{clock}{hdr}mean = 0.44\n")).contains("mean"));
    assert!(parse(format!("{hdr}prepare = \"old\"\n")).contains("[sky.prepare]"));
    assert!(parse(format!("{hdr}balance = [1.0, 1.0, 1.0]\n")).contains("[sky.prepare]"));
    assert!(parse("\n[sky]\nkind = \"old\"\n".into()).contains("'room'"));
    assert!(parse(format!("{hdr}ambient = \"old\"\n")).contains("'authored'"));
    assert!(parse(format!("{hdr}ambient = \"authored\"\n")).contains("model = 'authored'"));
    assert!(parse(format!("{authored}\n[sky]\nambient = \"authored\"\n")).contains("HDR or room"));
    assert!(parse(format!("{hdr}ambient = \"clock\"\n")).contains("model = 'clock'"));
    assert!(parse(format!("{clock}{hdr}ambient = \"clock\"\n")).contains("mean alone"));
    assert!(
        parse(format!(
            "{clock}{hdr}ambient = \"clock\"\n\n[sky.prepare]\ncap = 9.0\nmean = 1.0\n"
        ))
        .contains("mean alone")
    );
    assert!(
        parse(format!(
            "{clock}{hdr}ambient = \"clock\"\nintensity = 2.0\n\n[sky.prepare]\nmean = 1.0\n"
        ))
        .contains("no rotation or intensity")
    );
    assert!(parse(format!("{authored}{hdr}ambient = \"sunny\"\n")).contains("not 'sunny'"));
    assert!(
        parse(format!(
            "{authored}{hdr}ambient = \"clock\"\n\n[sky.prepare]\nmean = 1.0\n"
        ))
        .contains("model = 'clock'")
    );
    assert!(
        Recipe::parse(format!("{base}{authored}{hdr}ambient = \"authored\"\n").as_bytes()).is_ok()
    );
    assert!(
        Recipe::parse(
            format!("{base}{clock}{hdr}ambient = \"clock\"\n\n[sky.prepare]\nmean = 1.0\n")
                .as_bytes()
        )
        .is_ok()
    );
}

fn files_of(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(name, fs::read(&path).unwrap());
            }
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn product_and_app_bake_the_same_bytes() {
    let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
    let room = fs::read_to_string(Path::new(ROOM).join("bake.toml")).unwrap();
    let gltf = fs::read(Path::new(ROOM).join("room.gltf")).unwrap();
    let authored = format!("\n[sun]\nmodel = \"authored\"\n{AUTHORED}");
    let news = [
        room.clone() + "\n[sun]\nmodel = \"solar\"\n\n[sky]\nkind = \"daylight\"\n",
        room.clone()
            + &format!(
                "{authored}\n[sky]\nkind = \"hdr\"\npath = \"sky.hdr\"\nrotation_deg = 120.0\nintensity = 3.0\nambient = \"authored\"\n\n[sky.prepare]\ncap = 20.0\nbalance = [1.0, 0.9, 0.7]\nmean = 0.4\n"
            ),
        room.clone()
            + &format!(
                "{authored}\n[sky]\nkind = \"room\"\nambient = \"authored\"\nintensity = 1.5\n{ROOM_SKY}"
            ),
    ];
    let pairs: Vec<(String, String)> = news
        .into_iter()
        .map(|new| (new.replace("app = \"test\"", "product = \"test\""), new))
        .collect();
    for (index, (old, new)) in pairs.iter().enumerate() {
        let mut written = Vec::new();
        for (side, recipe) in [("old", old), ("new", new)] {
            let dir = scratch(&format!("gpu-{index}-{side}"));
            fs::write(dir.join("bake.toml"), recipe).unwrap();
            fs::write(dir.join("room.gltf"), &gltf).unwrap();
            fs::write(dir.join("sky.hdr"), SKY).unwrap();
            let source = SceneSource::load(&dir).unwrap();
            let spec = GridSpec {
                min: [-0.5, 0.5, -0.5],
                max: [0.5, 1.5, 0.5],
                spacing: 0.5,
            };
            let grids = crate::bake(&gpu, &source.scene, spec, 4, source.recipe.seed).unwrap();
            let out = dir.join("tmp");
            fs::create_dir_all(&out).unwrap();
            crate::write_artifact(&out, Path::new("probes"), &source.scene, &grids, 4, 73).unwrap();
            written.push(files_of(&out.join("probes")));
            fs::remove_dir_all(dir).unwrap();
        }
        assert!(!written[0].is_empty());
        assert_eq!(written[0], written[1], "pair {index}");
    }
}
