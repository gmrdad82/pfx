const FORBIDDEN_METHODS: &[&str] = &[
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "exp",
    "exp2",
    "exp_m1",
    "ln",
    "ln_1p",
    "log",
    "log2",
    "log10",
    "powf",
    "powi",
    "cbrt",
    "hypot",
    "sin_cos",
    "mul_add",
    "to_degrees",
    "to_radians",
    "gamma",
    "ln_gamma",
    "erf",
    "fma",
    "rem_euclid",
    "div_euclid",
];

const FORBIDDEN_TEXT: &[&str] = &[
    "libm",
    "intrinsics",
    "fmaf",
    "std::f64::",
    "std::f32::",
    "target_feature",
    "fast-math",
    "fadd_fast",
    "fmul_fast",
    "unsafe",
    "SystemTime",
    "Instant",
    "thread_rng",
];

const SOURCES: &[(&str, &str)] = &[
    ("mod.rs", include_str!("mod.rs")),
    ("math.rs", include_str!("math.rs")),
    ("rng.rs", include_str!("rng.rs")),
];

fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source.lines().enumerate().map(|(i, l)| (i + 1, l))
}

#[test]
fn sim_sources_call_no_std_transcendentals() {
    for (file, source) in SOURCES {
        for (line_no, line) in code_lines(source) {
            for name in FORBIDDEN_METHODS {
                for prefix in [".", "::"] {
                    let call = format!("{prefix}{name}(");
                    assert!(
                        !line.contains(&call),
                        "{file}:{line_no} calls {name}: {line}"
                    );
                }
            }
        }
    }
}

#[test]
fn sim_sources_avoid_other_nondeterminism() {
    for (file, source) in SOURCES {
        for (line_no, line) in code_lines(source) {
            for text in FORBIDDEN_TEXT {
                assert!(
                    !line.contains(text),
                    "{file}:{line_no} mentions {text}: {line}"
                );
            }
        }
    }
}

#[test]
fn sim_sources_never_take_a_float_remainder() {
    for (file, source) in SOURCES {
        for (line_no, line) in code_lines(source) {
            assert!(
                !(line.contains(" % ") && line.contains("f64")),
                "{file}:{line_no}: {line}"
            );
        }
    }
}

#[test]
fn the_guard_catches_what_it_scans_for() {
    let sample = "let y = x.sin() + f64::exp(x) + a.mul_add(b, c);";
    let mut hits = 0;
    for name in FORBIDDEN_METHODS {
        for prefix in [".", "::"] {
            if sample.contains(&format!("{prefix}{name}(")) {
                hits += 1;
            }
        }
    }
    assert_eq!(hits, 3);
}
