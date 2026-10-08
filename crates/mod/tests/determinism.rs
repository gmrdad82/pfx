mod common;

use std::fmt::Debug;
use std::fs;
use std::path::Path;

use common::*;
use pfx_mod::wasmtime::Store;
use pfx_mod::{Called, Host, Limits, ModCtx, ModError};

type Case = fn(&Play, &mut Store<ModCtx<Game>>) -> pfx_mod::wasmtime::Result<u32>;

const GOLDEN: &str = include_str!("golden/determinism.txt");
const CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;

const BITS: [u64; 8] = [
    0x7ff4_0000_0000_0001,
    0xfff8_0000_0000_0000,
    0x7ff8_0000_0000_0123,
    0xffff_ffff_ffff_ffff,
    0x3ff0_0000_0000_0000,
    0xbff0_0000_0000_0000,
    0x0000_0000_0000_0000,
    0x7ff0_0000_0000_0000,
];

fn outcome<R: Debug>(result: Result<Called<R>, ModError>) -> String {
    match result {
        Ok(called) => format!("ok {:x?} fuel {}", called.value, called.fuel),
        Err(error) => {
            let at = error
                .at
                .map(|at| format!("{}+{:x?}", at.func, at.offset))
                .unwrap_or_else(|| "none".to_string());
            format!("{:?} fuel {} at {at}", error.cause, error.fuel)
        }
    }
}

fn run(host: &Host<Game>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut m = start(host, "steady", 11);
    for by in [1, 2, 3, u32::MAX] {
        let r = m.call(|play, store| play.call_bump(store, by));
        lines.push(format!("bump {by} : {}", outcome(r)));
    }
    for n in 0..3 {
        let r = m.call(|play, store| play.call_roll(store));
        lines.push(format!("roll {n} : {}", outcome(r)));
    }
    for (a, b, steps) in [
        (1.25, 0.5, 1000),
        (-3.0, 7.0e-3, 4096),
        (1.0e300, -2.5e-300, 64),
        (0.1, 0.2, 20_000),
    ] {
        let r = m
            .call(|play, store| play.call_mix(store, a, b, steps))
            .map(|c| Called {
                value: f64::to_bits(c.value),
                fuel: c.fuel,
            });
        lines.push(format!("mix {a:e} {b:e} {steps} : {}", outcome(r)));
    }
    for bits in BITS {
        for op in 0..6 {
            let r = m.call(|play, store| play.call_nan_bits(store, bits, op));
            if let Ok(called) = &r {
                let value = f64::from_bits(called.value);
                assert!(
                    !value.is_nan() || called.value == CANONICAL_NAN,
                    "op {op} on {bits:#x} gave a non-canonical NaN {:#x}",
                    called.value
                );
            }
            lines.push(format!("nan {bits:016x} op {op} : {}", outcome(r)));
        }
    }
    let r = m.call(|play, store| play.call_burn(store, 1000));
    lines.push(format!("burn 1000 : {}", outcome(r)));
    let cases: [(&str, Case); 3] = [
        ("burn max", |play, store| play.call_burn(store, u32::MAX)),
        ("fall 9", |play, store| play.call_fall(store, 9)),
        ("grow 100", |play, store| play.call_grow(store, 100)),
    ];
    for (name, case) in cases {
        let mut m = start(host, "failing", 11);
        let r = m.call(case);
        lines.push(format!("{name} : {}", outcome(r)));
        assert!(!m.running());
    }
    let depth = host.limits().depth;
    for by in [depth - 1, depth, u32::MAX] {
        let mut m = start(host, "deep", 11);
        let r = m.call(|play, store| play.call_fall(store, by));
        lines.push(format!("fall {by} : {}", outcome(r)));
    }
    lines
}

fn overflow() -> String {
    let host = host(Limits {
        depth: u32::MAX,
        ..limits()
    });
    let mut m = start(&host, "deep", 11);
    let error = m
        .call(|play, store| play.call_fall(store, u32::MAX))
        .unwrap_err();
    format!("overflow : {:?}", error.cause)
}

#[test]
fn mods_give_the_same_bits_fuel_and_trap_points_on_every_platform() {
    let host = host(limits());
    let mut lines = run(&host);
    assert_eq!(lines, run(&host), "two runs in one process differ");
    lines.push(overflow());
    let text = lines.join("\n") + "\n";
    if std::env::var_os("PFX_MOD_BLESS").is_some() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/determinism.txt");
        fs::write(path, &text).unwrap();
        return;
    }
    let golden = GOLDEN.replace("\r\n", "\n");
    for (n, (got, want)) in lines.iter().zip(golden.lines()).enumerate() {
        assert_eq!(got, want, "line {} differs from the golden", n + 1);
    }
    assert_eq!(lines.len(), golden.lines().count(), "{text}");
}
