mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{Folder, show};
use pfx_scene::{Diagnostic, MAX_DEPTH, Severity, code};

const HEAD: &str = "format = 1\n\n[project]\nname = \"cases\"\n";

fn run(name: &str, files: &[(&str, &str)]) -> (Folder, Vec<Diagnostic>) {
    let folder = Folder::assets(name);
    folder.write("project.toml", HEAD);
    for (file, text) in files {
        folder.write(file, text);
    }
    let diagnostics = folder.check();
    (folder, diagnostics)
}

fn find<'a>(diagnostics: &'a [Diagnostic], code: &str) -> &'a Diagnostic {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("no {code} in:\n{}", show(diagnostics)))
}

fn at(diagnostic: &Diagnostic, file: &str, line: u32, column: u32) {
    assert_eq!(
        (
            diagnostic.file.as_path(),
            diagnostic.line,
            diagnostic.column
        ),
        (Path::new(file), line, column),
        "{diagnostic}"
    );
}

const BLOCK: &str = "format = 1\n\n[mesh.b]\nid = \"me5h00000b\"\nfile = \"block.gltf\"\n\n";

#[test]
fn every_code_fires_where_it_should() {
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    let mut expect = |name: &str,
                      files: &[(&str, &str)],
                      code: &'static str,
                      file: &str,
                      line: u32,
                      column: u32|
     -> Diagnostic {
        let (_folder, diagnostics) = run(name, files);
        let found = find(&diagnostics, code).clone();
        at(&found, file, line, column);
        seen.insert(code);
        found
    };

    let syntax = expect(
        "syntax",
        &[("a.scene.toml", "format = 1\n[[object]\nname = \"x\"\n")],
        code::SYNTAX,
        "a.scene.toml",
        2,
        10,
    );
    assert!(syntax.is_error());

    let unknown = expect(
        "unknown-key",
        &[(
            "a.scene.toml",
            "format = 1\n\n[[light]]\nid = \"11ght00001\"\nname = \"a\"\nposition = [0.0, 1.0, 0.0]\ncolour = [1.0, 1.0, 1.0]\nintensity = 1.0\n",
        )],
        code::UNKNOWN_KEY,
        "a.scene.toml",
        7,
        1,
    );
    assert_eq!(unknown.key, "light.11ght00001.colour");
    assert_eq!((unknown.end_line, unknown.end_column), (7, 7));
    assert!(unknown.message.contains("colour"), "{unknown}");
    assert!(unknown.message.contains("'intensity'"), "{unknown}");

    let top = expect(
        "unknown-top",
        &[("a.scene.toml", "format = 1\nlights = 3\n")],
        code::UNKNOWN_KEY,
        "a.scene.toml",
        2,
        1,
    );
    assert!(top.message.contains("'light'"), "{top}");

    let missing = expect(
        "missing-key",
        &[(
            "a.scene.toml",
            "format = 1\n\n[[light]]\nid = \"11ght00001\"\nname = \"a\"\nintensity = 1.0\n",
        )],
        code::MISSING_KEY,
        "a.scene.toml",
        3,
        1,
    );
    assert!(missing.message.contains("position"), "{missing}");
    assert_eq!(missing.key, "light.11ght00001");

    let mesh = expect(
        "missing-mesh",
        &[(
            "a.scene.toml",
            "format = 1\n\n[[object]]\nid = \"0bj0000001\"\nname = \"x\"\n",
        )],
        code::MISSING_KEY,
        "a.scene.toml",
        3,
        1,
    );
    assert!(mesh.message.contains("'mesh'"), "{mesh}");

    let kind = expect(
        "bad-type",
        &[(
            "a.scene.toml",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\nat = \"up\"\n"
            ),
        )],
        code::BAD_TYPE,
        "a.scene.toml",
        11,
        6,
    );
    assert_eq!(kind.key, "object.0bj0000001.at");

    let variant = expect(
        "bad-variant",
        &[(
            "a.scene.toml",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\nshadow = \"maybe\"\n"
            ),
        )],
        code::BAD_VALUE,
        "a.scene.toml",
        11,
        10,
    );
    assert!(variant.message.contains("maybe"), "{variant}");

    let range = expect(
        "bad-value",
        &[(
            "a.scene.toml",
            "format = 1\n\n[[light]]\nid = \"11ght00001\"\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\nrange = 0.0\n",
        )],
        code::BAD_VALUE,
        "a.scene.toml",
        3,
        1,
    );
    assert!(range.message.contains("positive range"), "{range}");

    let id = expect(
        "bad-id",
        &[(
            "a.scene.toml",
            "format = 1\n\n[[light]]\nid = \"11ght0000i\"\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n",
        )],
        code::BAD_ID,
        "a.scene.toml",
        4,
        6,
    );
    assert!(id.message.contains("'i'"), "{id}");

    let missing_id = expect(
        "missing-id",
        &[(
            "a.scene.toml",
            "format = 1\n\n[[light]]\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n",
        )],
        code::MISSING_ID,
        "a.scene.toml",
        3,
        1,
    );
    assert_eq!(missing_id.severity, Severity::Warning);
    assert_eq!(missing_id.key, "light.0");

    let twice = expect(
        "duplicate-id",
        &[
            (
                "a.scene.toml",
                "format = 1\n\n[[light]]\nid = \"11ght00001\"\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n",
            ),
            (
                "b.scene.toml",
                "format = 1\n\n[[light]]\nid = \"11ght00001\"\nname = \"m\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n",
            ),
        ],
        code::DUPLICATE_ID,
        "b.scene.toml",
        4,
        6,
    );
    assert_eq!(twice.related.len(), 1);
    assert_eq!(twice.related[0].file, Path::new("a.scene.toml"));
    assert_eq!(twice.related[0].line, 4);

    expect(
        "bad-name",
        &[(
            "a.scene.toml",
            "format = 1\n\n[[light]]\nid = \"11ght00001\"\nname = \" l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n",
        )],
        code::BAD_NAME,
        "a.scene.toml",
        5,
        8,
    );

    let named = expect(
        "duplicate-name",
        &[(
            "a.scene.toml",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n\n[[object]]\nid = \"0bj0000002\"\nname = \"x\"\nmesh = \"b\"\n"
            ),
        )],
        code::DUPLICATE_NAME,
        "a.scene.toml",
        14,
        8,
    );
    assert!(named.message.contains("object x is named twice"), "{named}");
    assert_eq!(named.related[0].line, 9);

    let reference = expect(
        "bad-reference",
        &[(
            "a.scene.toml",
            "format = 1\n\n[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"nothing\"\n",
        )],
        code::BAD_REFERENCE,
        "a.scene.toml",
        6,
        8,
    );
    assert!(
        reference.message.contains("nothing names no mesh"),
        "{reference}"
    );

    let ambiguous = expect(
        "ambiguous-reference",
        &[(
            "a.scene.toml",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n\n[[object]]\nid = \"0bj0000002\"\nname = \"x\"\nmesh = \"b\"\n\n[[mover]]\nid = \"m0ver00001\"\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
        )],
        code::AMBIGUOUS_REFERENCE,
        "a.scene.toml",
        20,
        12,
    );
    assert_eq!(ambiguous.related.len(), 2);

    let absolute = expect(
        "bad-path",
        &[(
            "a.scene.toml",
            "format = 1\n\n[mesh.b]\nid = \"me5h00000b\"\nfile = \"/b.gltf\"\n",
        )],
        code::BAD_PATH,
        "a.scene.toml",
        5,
        8,
    );
    assert!(absolute.message.contains("absolute"), "{absolute}");

    expect(
        "outside-root",
        &[(
            "a.scene.toml",
            "format = 1\n\n[mesh.b]\nid = \"me5h00000b\"\nfile = \"../b.gltf\"\n",
        )],
        code::OUTSIDE_ROOT,
        "a.scene.toml",
        5,
        8,
    );

    let gone = expect(
        "missing-file",
        &[(
            "a.scene.toml",
            "format = 1\n\n[mesh.b]\nid = \"me5h00000b\"\nfile = \"nothing.gltf\"\n",
        )],
        code::MISSING_FILE,
        "a.scene.toml",
        5,
        8,
    );
    assert!(gone.message.contains("nothing.gltf"), "{gone}");

    let case = expect(
        "path-case",
        &[(
            "a.scene.toml",
            "format = 1\n\n[mesh.b]\nid = \"me5h00000b\"\nfile = \"Block.gltf\"\n",
        )],
        code::PATH_CASE,
        "a.scene.toml",
        5,
        8,
    );
    assert!(case.message.contains("block.gltf"), "{case}");

    expect(
        "include-cycle",
        &[
            ("a.scene.toml", "format = 1\ninclude = [\"b.scene.toml\"]\n"),
            ("b.scene.toml", "format = 1\ninclude = [\"a.scene.toml\"]\n"),
        ],
        code::INCLUDE_CYCLE,
        "b.scene.toml",
        2,
        12,
    );

    expect(
        "prefab-cycle",
        &[(
            "p.prefab.toml",
            "format = 1\n\n[[object]]\nid = \"0bj0000001\"\nname = \"again\"\nprefab = \"p.prefab.toml\"\n",
        )],
        code::PREFAB_CYCLE,
        "p.prefab.toml",
        6,
        10,
    );

    let includes: Vec<(String, String)> = (0..=MAX_DEPTH + 1)
        .map(|at| {
            let text = if at <= MAX_DEPTH {
                format!("format = 1\ninclude = [\"s{}.scene.toml\"]\n", at + 1)
            } else {
                "format = 1\n".to_string()
            };
            (format!("s{at}.scene.toml"), text)
        })
        .collect();
    let files: Vec<(&str, &str)> = includes
        .iter()
        .map(|(file, text)| (file.as_str(), text.as_str()))
        .collect();
    let deep = expect(
        "include-depth",
        &files,
        code::INCLUDE_DEPTH,
        &format!("s{MAX_DEPTH}.scene.toml"),
        2,
        12,
    );
    assert_eq!(deep.related.len(), MAX_DEPTH + 1, "{deep}");

    let mut prefabs: Vec<(String, String)> = vec![(
        "room.scene.toml".to_string(),
        "format = 1\n\n[[object]]\nid = \"p000000000\"\nname = \"next\"\nprefab = \"p1.prefab.toml\"\n"
            .to_string(),
    )];
    prefabs.extend((1..=MAX_DEPTH + 1).map(|at| {
        let text = if at <= MAX_DEPTH {
            format!(
                "format = 1\n\n[[object]]\nid = \"p{at:09}\"\nname = \"next\"\nprefab = \"p{}.prefab.toml\"\n",
                at + 1
            )
        } else {
            "format = 1\n".to_string()
        };
        (format!("p{at}.prefab.toml"), text)
    }));
    let files: Vec<(&str, &str)> = prefabs
        .iter()
        .map(|(file, text)| (file.as_str(), text.as_str()))
        .collect();
    let deep = expect(
        "prefab-depth",
        &files,
        code::PREFAB_DEPTH,
        &format!("p{MAX_DEPTH}.prefab.toml"),
        6,
        10,
    );
    assert_eq!(deep.related.len(), MAX_DEPTH + 1, "{deep}");

    expect(
        "parent-cycle",
        &[(
            "a.scene.toml",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\nparent = \"y\"\n\n[[object]]\nid = \"0bj0000002\"\nname = \"y\"\nmesh = \"b\"\nparent = \"x\"\n"
            ),
        )],
        code::PARENT_CYCLE,
        "a.scene.toml",
        11,
        10,
    );

    let held = expect(
        "held-twice",
        &[
            (
                "sky.scene.toml",
                "format = 1\n\n[sky]\nkind = \"analytic\"\n",
            ),
            (
                "main.scene.toml",
                "format = 1\ninclude = [\"sky.scene.toml\"]\n\n[sky]\nkind = \"analytic\"\n",
            ),
        ],
        code::HELD_TWICE,
        "main.scene.toml",
        4,
        2,
    );
    assert!(
        held.message.contains("sky.scene.toml sets it already"),
        "{held}"
    );
    assert_eq!(held.related[0].file, Path::new("sky.scene.toml"));

    let lamp = "format = 1\n\n[mesh.shade]\nid = \"1amp0mesh1\"\nfile = \"ball.gltf\"\n\n[[object]]\nid = \"1amp000001\"\nname = \"shade\"\nmesh = \"shade\"\n";
    let over = expect(
        "bad-override",
        &[
            ("lamp.prefab.toml", lamp),
            (
                "a.scene.toml",
                "format = 1\n\n[[object]]\nid = \"p1acement1\"\nname = \"lamp\"\nprefab = \"lamp.prefab.toml\"\n\n[object.set]\n\"nobody.at\" = [0.0, 1.0, 0.0]\n",
            ),
        ],
        code::BAD_OVERRIDE,
        "a.scene.toml",
        9,
        1,
    );
    assert!(over.message.contains("nobody"), "{over}");

    let placed = expect(
        "placement-key",
        &[
            ("lamp.prefab.toml", lamp),
            (
                "a.scene.toml",
                "format = 1\n\n[[object]]\nid = \"p1acement1\"\nname = \"lamp\"\nprefab = \"lamp.prefab.toml\"\nmesh = \"shade\"\n",
            ),
        ],
        code::PLACEMENT_KEY,
        "a.scene.toml",
        7,
        1,
    );
    assert!(placed.message.contains("not mesh"), "{placed}");

    let old = expect(
        "old-format",
        &[(
            "a.scene.toml",
            "[[light]]\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n",
        )],
        code::OLD_FORMAT,
        "a.scene.toml",
        1,
        1,
    );
    assert_eq!(old.severity, Severity::Warning);

    let newer = expect(
        "newer-format",
        &[("a.scene.toml", "format = 2\n")],
        code::NEWER_FORMAT,
        "a.scene.toml",
        1,
        10,
    );
    assert!(newer.message.contains("format 1"), "{newer}");

    let migration = expect(
        "migration",
        &[("a.scene.toml", "[body.ghost]\nkind = \"fixed\"\n")],
        code::MIGRATION,
        "a.scene.toml",
        1,
        1,
    );
    assert!(migration.message.contains("ghost"), "{migration}");

    {
        let folder = Folder::empty("not-utf8");
        folder.write("project.toml", HEAD);
        std::fs::write(folder.path("a.scene.toml"), [b'f', 0xff, b'\n']).unwrap();
        let diagnostics = folder.check();
        at(find(&diagnostics, code::NOT_UTF8), "a.scene.toml", 1, 1);
        seen.insert(code::NOT_UTF8);
    }

    {
        let folder = Folder::empty("unreadable");
        folder.write("project.toml", HEAD);
        folder.write("a.scene.toml", "format = 1\ninclude = [\"b.scene.toml\"]\n");
        folder.write("b.scene.toml", "format = 1\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = folder.path("b.scene.toml");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            if std::fs::read(&path).is_err() {
                let diagnostics = folder.check();
                at(find(&diagnostics, code::UNREADABLE), "b.scene.toml", 1, 1);
            }
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        seen.insert(code::UNREADABLE);
    }

    let all: BTreeSet<&'static str> = code::ALL.iter().copied().collect();
    assert_eq!(seen, all);
}

#[test]
fn the_engines_refusals_are_kept() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "node-shadow",
            "format = 1\n\n[mesh.b]\nid = \"me5h00000b\"\nfile = \"block.gltf\"\n\n[mesh.b.nodes.Block]\nshadow = \"maybe\"\n",
            "cast",
        ),
        (
            "sky-kind",
            "format = 1\n\n[sky]\nkind = \"cloud\"\n",
            "analytic",
        ),
        (
            "room-path",
            "format = 1\n\n[sky]\nkind = \"room\"\npath = \"x.hdr\"\n",
            "[sky.room]",
        ),
        (
            "prepare-empty",
            "format = 1\n\n[sky]\nkind = \"hdr\"\npath = \"block.gltf\"\n[sky.prepare]\n",
            "cap, balance or mean",
        ),
        (
            "sun-model",
            "format = 1\n\n[sun]\nmodel = \"moon\"\n",
            "daylight",
        ),
        (
            "sun-mixed",
            "format = 1\n\n[sun]\nmodel = \"daylight\"\nhour = 9.0\nirradiance = 2.0\n",
            "not toward",
        ),
        (
            "sun-hour",
            "format = 1\n\n[sun]\nmodel = \"daylight\"\nhour = 30.0\n",
            "outside 0 to 24",
        ),
        (
            "sun-radius",
            "format = 1\n\n[sun]\nmodel = \"daylight\"\nhour = 9.0\nradius = 12.0\n",
            "outside 0 to 10",
        ),
        (
            "camera-both",
            "format = 1\n\n[camera]\nfov = 30.0\nfocal = 50.0\n",
            "not both",
        ),
        (
            "camera-ortho",
            "format = 1\n\n[camera]\nprojection = \"orthographic\"\n",
            "positive height",
        ),
        (
            "camera-same",
            "format = 1\n\n[camera]\nat = [0.0, 0.0, 0.0]\n",
            "must differ",
        ),
        (
            "camera-focus",
            "format = 1\n\n[camera]\nfocus = 3.0\n",
            "focus needs fstop",
        ),
        (
            "camera-fstop",
            "format = 1\n\n[camera]\nfstop = 0.0\n",
            "fstop must be positive",
        ),
        (
            "camera-ortho-fstop",
            "format = 1\n\n[camera]\nprojection = \"orthographic\"\nheight = 2.0\nfstop = 2.8\n",
            "not fov, focal, sensor, fstop, focus or focus_keys",
        ),
        (
            "preset-key",
            "format = 1\n\n[camera.preset]\norbit = { spin = 1.0 }\n",
            "spin",
        ),
        (
            "preset-hush",
            "format = 1\n\n[camera.preset.hush]\nfloor = 1.0\n",
            "hush needs floor, stiffness and blocks_cursor",
        ),
        (
            "finish-key",
            "format = 1\n\n[finish]\nglow = 1.0\n",
            "unknown key 'glow'",
        ),
        (
            "finish-file",
            "format = 1\n\n[finish]\nfile = \"look.toml\"\nstyle = \"noir\"\n",
            "file alone",
        ),
        (
            "finish-style",
            "format = 1\n\n[finish]\nstyle = \"sparkle\"\n",
            "sparkle",
        ),
        (
            "finish-pass-style",
            "format = 1\n\n[finish]\nexposure = 1.0\n\n[[finish.pass]]\nwarmth = 0.1\n\n[[finish.pass]]\nstyle = \"noir\"\n",
            "not style",
        ),
        (
            "finish-pass-empty",
            "format = 1\n\n[finish]\n[[finish.pass]]\n",
            "builds no pass",
        ),
        (
            "finish-tone-steps",
            "format = 1\n\n[finish]\ntone_steps = 9\n",
            "tone_steps is 3 to 5",
        ),
        (
            "finish-lut",
            "format = 1\n\n[finish]\nlut = { size = 2, values = [0.0] }\n",
            "lut.values has 24 numbers",
        ),
        (
            "finish-bloom-key",
            "format = 1\n\n[finish]\nbloom = { glow = 1.0 }\n",
            "glow",
        ),
        (
            "finish-colour",
            "format = 1\n\n[finish]\nrim_color = \"#12345\"\n",
            "#rrggbb",
        ),
        (
            "haze-bounds",
            "format = 1\n\n[haze]\nlo = [0.0, 0.0, 0.0]\nhi = [1.0, 0.0, 1.0]\n",
            "lo must lie below hi",
        ),
        (
            "haze-amount",
            "format = 1\n\n[haze]\nlo = [0.0, 0.0, 0.0]\nhi = [1.0, 1.0, 1.0]\namount = 2.0\n",
            "outside 0 to 1",
        ),
        (
            "haze-bounces",
            "format = 1\n\n[haze]\nheight = 0.0\nbounces = 9\n",
            "haze bounces = 9 is outside 0 to 8",
        ),
        (
            "mix-one",
            "format = 1\n\n[sky]\nkind = \"mix\"\n[[sky.layer]]\nkind = \"analytic\"\n",
            "two or more",
        ),
        (
            "mix-key",
            "format = 1\n\n[sky]\nkind = \"mix\"\nintensity = 2.0\n[[sky.layer]]\nkind = \"analytic\"\n[[sky.layer]]\nkind = \"analytic\"\n",
            "only [[sky.layer]]",
        ),
        (
            "layer-not-mix",
            "format = 1\n\n[sky]\nkind = \"analytic\"\n[[sky.layer]]\nkind = \"analytic\"\n",
            "only a mix sky",
        ),
        (
            "layer-weight",
            "format = 1\n\n[sky]\nkind = \"mix\"\n[[sky.layer]]\nkind = \"analytic\"\nweight = -1.0\n[[sky.layer]]\nkind = \"analytic\"\n",
            "nonnegative",
        ),
        (
            "sky-weight",
            "format = 1\n\n[sky]\nkind = \"analytic\"\nweight = 1.0\n",
            "belongs to a [[sky.layer]]",
        ),
        (
            "room-width",
            "format = 1\n\n[sky]\nkind = \"room\"\n[sky.room]\nwidth = 1\n",
            "room width is 1",
        ),
        (
            "physics",
            "format = 1\n\n[physics]\nrate = 0.0\n",
            "rate of 1 to 1000",
        ),
        (
            "trace-clamp",
            "format = 1\n\n[trace]\nclamp_indirect = -1\n",
            "[trace] clamp_indirect = -1 is outside 0 to 10000",
        ),
        (
            "trace-clamp-high",
            "format = 1\n\n[trace]\nclamp_indirect = 10000.5\n",
            "[trace] clamp_indirect = 10000.5 is outside 0 to 10000",
        ),
        (
            "trace-clamp-nan",
            "format = 1\n\n[trace]\nclamp_indirect = nan\n",
            "[trace] clamp_indirect = NaN is outside 0 to 10000",
        ),
        (
            "trace-filter",
            "format = 1\n\n[trace]\nfilter_glossy = 1.5\n",
            "[trace] filter_glossy = 1.5 is outside 0 to 1",
        ),
        (
            "trace-filter-inf",
            "format = 1\n\n[trace]\nfilter_glossy = -inf\n",
            "[trace] filter_glossy = -inf is outside 0 to 1",
        ),
        (
            "alpha-cutoff",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\nalpha_cutoff = 1.5\n"
            ),
            "outside 0 to 1",
        ),
        (
            "clip-three",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\nclip = [[0.0, 1.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]\n"
            ),
            "at most two",
        ),
        (
            "content-missing",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\ncontent = \"tv\"\n"
            ),
            "tv names no content",
        ),
        (
            "parent-missing",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\nparent = \"y\"\n"
            ),
            "y names no object",
        ),
        (
            "fallback",
            "format = 1\nfallback = \"gold\"\n",
            "gold names no material",
        ),
        (
            "mover-object",
            &format!(
                "{BLOCK}[[mover]]\nid = \"m0ver00001\"\nname = \"m\"\nobjects = [\"ghost\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "ghost names no object",
        ),
        (
            "mover-kind",
            &format!(
                "{BLOCK}[[mover]]\nid = \"m0ver00001\"\nname = \"m\"\nobjects = [\"x\"]\nkind = \"spin\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "turn",
        ),
        (
            "mover-axis",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n[[mover]]\nid = \"m0ver00001\"\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [0.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "nonzero axis",
        ),
        (
            "mover-twice",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n[[mover]]\nid = \"m0ver00001\"\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n[[mover]]\nid = \"m0ver00002\"\nname = \"n\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [0.0, 1.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "one at most",
        ),
        (
            "mover-clips",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\nclip = [[0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]]\n[[mover]]\nid = \"m0ver00001\"\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\nclip = [[0.0, 0.0, 1.0, 0.0]]\n"
            ),
            "with its movers'",
        ),
        (
            "body-parent",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n[[object]]\nid = \"0bj0000002\"\nname = \"y\"\nmesh = \"b\"\nparent = \"x\"\n[object.body]\nkind = \"fixed\"\n"
            ),
            "a body sits on a root object",
        ),
        (
            "body-shape",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n[object.body]\nshape = \"sphere\"\nhalf = [1.0, 1.0, 1.0]\n"
            ),
            "half for a box",
        ),
        (
            "body-mover",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n[object.body]\nkind = \"dynamic\"\n[[mover]]\nid = \"m0ver00001\"\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "a body or a mover",
        ),
        (
            "sound-hit",
            "format = 1\n\n[sound.bang]\nid = \"s0nd000001\"\nfile = \"block.gltf\"\nplay = \"hit\"\n",
            "names no object",
        ),
        (
            "sound-body",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n[sound.bang]\nid = \"s0nd000001\"\nfile = \"block.gltf\"\nplay = \"hit\"\nobject = \"x\"\n"
            ),
            "has no body",
        ),
        (
            "sound-loops",
            "format = 1\n\n[sound.a]\nid = \"s0nd000001\"\nfile = \"block.gltf\"\nloop = true\n\n[sound.b]\nid = \"s0nd000002\"\nfile = \"block.gltf\"\nloop = true\n",
            "one looping sound",
        ),
        (
            "material-layers",
            "format = 1\nmaterials = [\"m.materials.toml\"]\n",
            "at most 4 noise layers",
        ),
        (
            "text-size",
            "format = 1\n\n[text.t]\nid = \"text000001\"\ntext = \"hi\"\nfont = \"block.gltf\"\nsize = 0.0\n",
            "positive size",
        ),
        (
            "placement-no-prefab-set",
            &format!(
                "{BLOCK}[[object]]\nid = \"0bj0000001\"\nname = \"x\"\nmesh = \"b\"\n[object.set]\n\"a.b\" = 1\n"
            ),
            "places a prefab",
        ),
    ];
    let layer =
        "[[materials.m.layers]]\nkind = \"fbm\"\nfrequency = 1.0\namplitude = 1.0\nseed = 1\n";
    let library = format!(
        "format = 1\n\n[materials.m]\nid = \"mat0000001\"\n{}",
        layer.repeat(5)
    );
    for (name, text, needle) in cases {
        let (_folder, diagnostics) = run(
            name,
            &[("a.scene.toml", text), ("m.materials.toml", &library)],
        );
        let errors: Vec<&Diagnostic> = diagnostics.iter().filter(|d| d.is_error()).collect();
        assert!(
            errors.iter().any(|d| d.to_string().contains(needle)),
            "{name}: no error says {needle:?} in:\n{}",
            show(&diagnostics)
        );
    }
}

#[test]
fn a_scene_file_that_is_not_found_is_named_in_a_diagnostic() {
    let folder = Folder::room("scene-not-found");
    let project = folder.project();
    let missing = folder.path("gone.scene.toml");
    let named = |diagnostic: &Diagnostic| {
        diagnostic.code == code::MISSING_FILE
            && diagnostic.file == Path::new("gone.scene.toml")
            && (diagnostic.line, diagnostic.column) == (1, 1)
            && diagnostic.message.contains("gone.scene.toml")
    };
    let scene = project.scene(&missing).unwrap_err();
    assert!(scene.iter().any(named), "{}", show(&scene));
    let fix = project.fix(&missing).unwrap_err();
    assert_eq!(fix.code, code::MISSING_FILE, "{fix}");
    assert!(fix.diagnostics.iter().any(named), "{fix}");
    let edit = pfx_scene::SceneEdit::open(&missing).err().unwrap();
    assert_eq!(edit.code, code::MISSING_FILE, "{edit}");
    assert!(edit.message.contains("gone.scene.toml"), "{edit}");
}
