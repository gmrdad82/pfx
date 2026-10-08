use pfx_gpu::floor::SAMPLED_TEXTURES_PER_STAGE;
use pfx_gpu::{layout_shortfall, live_floor, stage_bindings, wgpu};
use pfx_live::{canopy, deform, flat, frame, glass, maps, plates, shadow, text};
use std::collections::BTreeSet;
use std::path::Path;

type Entries = Vec<wgpu::BindGroupLayoutEntry>;

fn layouts() -> Vec<(&'static str, Vec<Entries>)> {
    vec![
        (
            "live frame layout",
            vec![
                frame::scene_entries(),
                frame::shadow_entries(),
                deform::deformer_entries(),
                frame::lighting_entries(),
            ],
        ),
        (
            "live shadow layout",
            vec![
                frame::scene_entries(),
                frame::cascade_entries(),
                deform::deformer_entries(),
            ],
        ),
        (
            "live cut shadow layout",
            vec![
                frame::scene_entries(),
                frame::cascade_entries(),
                deform::deformer_entries(),
                frame::lighting_entries(),
            ],
        ),
        (
            "live cards layout",
            vec![
                frame::scene_entries(),
                frame::shadow_entries(),
                canopy::cookie_entries(),
                frame::lighting_entries(),
            ],
        ),
        (
            "glass back pipeline layout",
            vec![glass::scene_entries(), glass::id_entries()],
        ),
        (
            "glass compose pipeline layout",
            vec![
                glass::scene_entries(),
                glass::id_entries(),
                glass::compose_entries(),
            ],
        ),
        (
            "creature shadow depth",
            vec![shadow::light_entries(), shadow::creature_entries()],
        ),
        (
            "lit text pipeline layout",
            vec![
                frame::scene_entries(),
                frame::shadow_entries(),
                text::lit_entries(),
                frame::lighting_entries(),
            ],
        ),
        (
            "content compute mip region",
            vec![maps::mip_compute_entries(), maps::mip_region_entries()],
        ),
        (
            "deformed text pipeline layout",
            vec![text::text_entries(), Vec::new(), text::deformer_entries()],
        ),
        (
            "flat layer composite",
            vec![flat::view_entries(), flat::layer_entries()],
        ),
        (
            "plate composite",
            vec![plates::composite_entries(), plates::cascade_entries()],
        ),
    ]
}

fn sources(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

fn groups(entries: &[Entries]) -> Vec<&[wgpu::BindGroupLayoutEntry]> {
    entries.iter().map(Vec::as_slice).collect()
}

fn stages() -> [wgpu::ShaderStages; 3] {
    [
        wgpu::ShaderStages::VERTEX,
        wgpu::ShaderStages::FRAGMENT,
        wgpu::ShaderStages::COMPUTE,
    ]
}

#[test]
fn every_layout_fits_the_floor() {
    let floor = live_floor();
    for (label, entries) in layouts() {
        assert_eq!(
            layout_shortfall(&groups(&entries), &floor.limits),
            None,
            "{label} needs more than the floor"
        );
    }
}

#[test]
fn the_floor_is_the_most_sampled_textures_any_layout_binds() {
    let most = layouts()
        .iter()
        .flat_map(|(label, entries)| {
            stages().map(|stage| {
                (
                    stage_bindings(&groups(entries), stage).sampled_textures,
                    *label,
                )
            })
        })
        .max()
        .unwrap();
    assert_eq!(
        SAMPLED_TEXTURES_PER_STAGE, most.0,
        "the floor's sampled textures per stage should be what {} binds",
        most.1
    );
    assert_eq!(
        live_floor().limits.max_sampled_textures_per_shader_stage,
        most.0
    );
}

fn without_tests(source: &str) -> &str {
    source
        .find("\n#[cfg(test)]\nmod tests")
        .map_or(source, |end| &source[..end])
}

fn single_group(list: &str) -> bool {
    let list: String = list.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(inner) = list
        .strip_prefix("&[")
        .and_then(|rest| rest.strip_suffix("],"))
    else {
        return false;
    };
    !inner.is_empty() && !inner.contains(',')
}

fn multi_group_labels(source: &str) -> Vec<String> {
    let source = without_tests(source);
    let mut labels = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("bind_group_layouts:") {
        let before = &rest[..at];
        let after = &rest[at + "bind_group_layouts:".len()..];
        let end = after
            .find("push_constant_ranges")
            .expect("a pipeline layout without push_constant_ranges");
        if !single_group(&after[..end]) {
            let label = before
                .rfind("label: Some(\"")
                .map(|start| {
                    let text = &before[start + "label: Some(\"".len()..];
                    text[..text.find('"').unwrap()].to_string()
                })
                .expect("a multi-group pipeline layout without a label");
            labels.push(label);
        }
        rest = &after[end..];
    }
    labels
}

#[test]
fn every_multi_group_layout_in_the_renderer_is_counted() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&dir, &mut files);
    files.sort();
    let found: BTreeSet<String> = files
        .iter()
        .flat_map(|path| multi_group_labels(&std::fs::read_to_string(path).unwrap()))
        .collect();
    let counted: BTreeSet<String> = layouts()
        .into_iter()
        .map(|(label, _)| label.to_string())
        .collect();
    assert!(
        found.len() >= 8,
        "the scan found only {found:?}; it no longer reads the sources"
    );
    assert_eq!(
        found, counted,
        "add every multi-group pipeline layout to layouts() in this test"
    );
}

#[test]
fn the_scan_tells_one_group_from_several() {
    assert!(single_group(" &[&layout],\n "));
    assert!(!single_group(" &[&scene, &shadow],\n "));
    assert!(!single_group(" &layouts,\n "));
    let source = "let a = x(&D {\n label: Some(\"one\"),\n bind_group_layouts: &[&a],\n push_constant_ranges: &[],\n});\nlet b = x(&D {\n label: Some(\"two\"),\n bind_group_layouts: &[&a, &b],\n push_constant_ranges: &[],\n});\n";
    assert_eq!(multi_group_labels(source), vec!["two".to_string()]);
}
