#![cfg(feature = "schema")]

use pfx_scene::{FileKind, json_schema};

#[test]
fn the_published_schemas_match_the_types() {
    for (kind, name) in [
        (FileKind::Project, "project"),
        (FileKind::Scene, "scene"),
        (FileKind::Prefab, "prefab"),
        (FileKind::Materials, "materials"),
        (FileKind::Proxies, "proxies"),
        (FileKind::Finish, "finish"),
    ] {
        let path = format!(
            "{}/../../docs/schema/{name}.schema.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let published = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\r\n", "\n");
        assert!(
            published == json_schema(kind),
            "{path} is stale: cargo run -p pfx-scene --features schema --example scene-schema -- docs/schema"
        );
    }
}
