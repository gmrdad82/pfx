use super::*;

const MANIFEST_TEXT: &str =
    "id = \"counter\"\nversion = \"1.2.0\"\napi = \"0.3.1\"\nentry = \"counter.wasm\"\n";

#[test]
fn manifest_reads_its_four_fields() {
    let m = Manifest::parse(MANIFEST_TEXT).unwrap();
    assert_eq!(m.id, "counter");
    assert_eq!(m.version, "1.2.0");
    assert_eq!(m.api, "0.3.1");
    assert_eq!(m.entry, "counter.wasm");
}

#[test]
fn manifest_refuses_unknown_fields() {
    let text = format!("{MANIFEST_TEXT}wasi = true\n");
    assert!(matches!(Manifest::parse(&text), Err(Refusal::Manifest(_))));
}

#[test]
fn manifest_refuses_bad_ids_and_entries() {
    for id in ["", "Up", "-lead", "a/b", "a b", "..", &"x".repeat(65)] {
        let text = MANIFEST_TEXT.replace("\"counter\"", &format!("{id:?}"));
        assert!(
            matches!(Manifest::parse(&text), Err(Refusal::Id { .. })),
            "{id:?}"
        );
    }
    for entry in [
        "../x.wasm",
        "a/b.wasm",
        "a\\\\b.wasm",
        ".wasm",
        "x.wat",
        ".hidden.wasm",
        "c:x.wasm",
    ] {
        let text = MANIFEST_TEXT.replace("counter.wasm", entry);
        assert!(
            matches!(Manifest::parse(&text), Err(Refusal::Entry { .. })),
            "{entry}"
        );
    }
    assert!(valid_id("a-b_9"));
    assert!(valid_entry("mod.v2.wasm"));
}

#[test]
fn manifest_needs_semantic_versions() {
    let text = MANIFEST_TEXT.replace("\"1.2.0\"", "\"one\"");
    assert!(matches!(Manifest::parse(&text), Err(Refusal::Manifest(_))));
    let text = MANIFEST_TEXT.replace("\"0.3.1\"", "\"3\"");
    assert!(matches!(Manifest::parse(&text), Err(Refusal::Manifest(_))));
}

#[test]
fn api_versions_follow_caret_rules() {
    let api = Api::new("play", "1.4.2");
    assert!(api.accepts("1.0.0").is_ok());
    assert!(api.accepts("1.4.2").is_ok());
    assert!(api.accepts("1.5.0").is_err());
    assert!(api.accepts("2.0.0").is_err());
    let early = Api::new("play", "0.3.4");
    assert!(early.accepts("0.3.0").is_ok());
    assert!(early.accepts("0.2.0").is_err());
    assert!(early.accepts("0.4.0").is_err());
}

#[test]
fn api_mismatch_message_names_both_versions() {
    let message = Api::new("play", "0.3.4")
        .accepts("0.4.0")
        .unwrap_err()
        .to_string();
    assert!(message.contains("0.4.0"), "{message}");
    assert!(message.contains("0.3.4"), "{message}");
    assert!(message.contains("`play`"), "{message}");
}

#[test]
fn fingerprints_cover_manifest_and_component() {
    let a = Fingerprint::of_mod(b"m", b"c");
    assert_ne!(a, Fingerprint::of_mod(b"m2", b"c"));
    assert_ne!(a, Fingerprint::of_mod(b"m", b"c2"));
    assert_ne!(
        Fingerprint::of_mod(b"ab", b"c"),
        Fingerprint::of_mod(b"a", b"bc")
    );
    assert_eq!(a.hex().len(), 64);
}

#[test]
fn set_fingerprint_ignores_order_and_marks_the_empty_set() {
    let a = Fingerprint::of_mod(b"a", b"1");
    let b = Fingerprint::of_mod(b"b", b"2");
    assert_eq!(
        Fingerprint::of_set([("a", a), ("b", b)]),
        Fingerprint::of_set([("b", b), ("a", a)])
    );
    assert_ne!(Fingerprint::of_set([("a", a)]), Fingerprint::of_set([]));
    assert_ne!(
        Fingerprint::of_set([("a", a)]),
        Fingerprint::of_set([("b", a)])
    );
}

#[test]
fn log_keeps_a_rate_and_cuts_long_lines_on_char_boundaries() {
    let limits = Limits {
        log: Some(LogLimits {
            lines_per_tick: 2,
            line_bytes: 4,
        }),
        ..Limits::default()
    };
    let mut cx = ModCtx::new((), &limits);
    cx.log.push("abcdef");
    cx.log.push("ab\u{e9}z");
    cx.log.push("dropped");
    assert_eq!(
        cx.log.take(),
        vec!["abcd".to_string(), "ab\u{e9}".to_string()]
    );
    assert_eq!(cx.log.dropped(), 1);
    cx.log.tick();
    cx.log.push("x");
    assert_eq!(cx.log.take(), vec!["x".to_string()]);
}

#[test]
fn host_refuses_bad_game_versions() {
    assert!(Host::<()>::new(Api::new("play", "first"), Limits::default()).is_err());
}

#[test]
fn host_refuses_core_modules_and_oversized_parts() {
    let host = Host::<()>::new(Api::new("play", "0.3.1"), Limits::default()).unwrap();
    let core = b"\0asm\x01\0\0\0";
    assert_eq!(
        host.load(MANIFEST_TEXT.as_bytes(), core).err(),
        Some(Refusal::NotComponent)
    );
    let big = vec![b' '; Limits::default().manifest_bytes + 1];
    assert!(matches!(
        host.load(&big, core),
        Err(Refusal::Size {
            what: "manifest",
            ..
        })
    ));
    let small = Host::<()>::new(
        Api::new("play", "0.3.1"),
        Limits {
            component_bytes: 4,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        small.load(MANIFEST_TEXT.as_bytes(), core),
        Err(Refusal::Size {
            what: "component",
            ..
        })
    ));
}

#[test]
fn host_refuses_an_api_mismatch_before_compiling() {
    let host = Host::<()>::new(Api::new("play", "0.4.0"), Limits::default()).unwrap();
    assert!(matches!(
        host.load(MANIFEST_TEXT.as_bytes(), b"not wasm"),
        Err(Refusal::Api { .. })
    ));
}
