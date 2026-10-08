use std::f64::consts::TAU;
use std::time::Instant;

use pfx_geom::icon::{Icon, Part, PartShape};
use pfx_geom::mark::{Form, Mark};

const SAMPLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><circle cx="32" cy="32" r="24" fill="#808080"/><rect x="26" y="14" width="12" height="36" fill="#404040"/></svg>"##;

fn gear() -> String {
    let teeth = 24;
    let mut d = String::new();
    for i in 0..teeth {
        let a = TAU * i as f64 / teeth as f64;
        let b = TAU * (i as f64 + 0.5) / teeth as f64;
        let (r0, r1) = (40.0, 46.0);
        let p = |r: f64, t: f64| (50.0 + r * t.cos(), 50.0 + r * t.sin());
        let (x0, y0) = p(r0, a);
        let (cx, cy) = p(r1 + 4.0, (a + b) * 0.5);
        let (x1, y1) = p(r0, b);
        d.push_str(&format!(
            "{}{x0:.3} {y0:.3} Q{cx:.3} {cy:.3} {x1:.3} {y1:.3} ",
            if i == 0 { "M" } else { "L" }
        ));
    }
    d.push('Z');
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path fill="#808080" fill-rule="evenodd" d="{d} M50 30 A20 20 0 1 0 50 70 A20 20 0 1 0 50 30 Z"/><path d="M30 50 C30 30 70 30 70 50 S50 70 50 60" fill="none" stroke="#404040" stroke-width="4"/><rect x="44" y="44" width="12" height="12" rx="3" fill="#202020" stroke="#202020" stroke-width="2"/></svg>"##
    )
}

fn icons() -> Vec<(&'static str, Icon)> {
    let ball = |x: f32, y: f32, r: f32| {
        Part::new(PartShape::Ball {
            at: [x, y],
            z: 0.0,
            r,
        })
    };
    let mut flat = Icon::new(vec![ball(0.0, 0.0, 0.4), Part::new(PartShape::torus())]);
    flat.fuse = false;
    vec![
        ("dot", Icon::new(vec![ball(0.0, 0.0, 0.4)])),
        (
            "ring",
            Icon::new(vec![Part::new(PartShape::Torus {
                at: [0.0, 0.0],
                z: 0.0,
                major: 0.5,
                minor: 0.1,
            })]),
        ),
        (
            "pair",
            Icon::new(vec![ball(-0.2, 0.0, 0.3), ball(0.2, 0.0, 0.3)]),
        ),
        (
            "node",
            Icon::new(vec![
                ball(-0.6, -0.4, 0.2),
                ball(0.6, -0.4, 0.2),
                ball(0.0, 0.5, 0.25),
                Part::new(PartShape::tube(vec![[-0.6, -0.4], [0.0, 0.5], [0.6, -0.4]])),
                Part::new(PartShape::cube()),
            ]),
        ),
        (
            "drop",
            Icon::new(vec![
                Part::new(PartShape::drop()),
                Part::new(PartShape::capsule([-0.5, 0.0], [0.5, 0.0])),
            ]),
        ),
        ("unfused", flat),
    ]
}

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut out = f();
    for _ in 0..runs {
        let start = Instant::now();
        out = f();
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    (best, out)
}

fn main() {
    let form = Form::default();
    let gear = gear();
    for (name, svg) in [("sample", SAMPLE), ("gear", gear.as_str())] {
        let (ms, mark) = time(5, || Mark::build(svg, &form).unwrap());
        let triangles: usize = mark.solids.iter().map(|s| s.mesh.triangle_count()).sum();
        println!(
            "mark {name}: {ms:.2} ms, {} solids, {triangles} triangles",
            mark.solids.len()
        );
    }
    for (name, icon) in icons() {
        let (ms, pieces) = time(5, || icon.build().unwrap());
        let triangles: usize = pieces.iter().map(|p| p.mesh.triangle_count()).sum();
        println!(
            "icon {name}: {ms:.2} ms, {} pieces, {triangles} triangles",
            pieces.len()
        );
    }
}
