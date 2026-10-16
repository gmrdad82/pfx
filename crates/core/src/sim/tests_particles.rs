use super::particles::{Emitter, Gradient, IDENTITY, Matrix, Recipe, RecipeKeys, STEP};

fn moved(x: f32) -> Matrix {
    let mut m = IDENTITY;
    m[3][0] = x;
    m
}

fn spray() -> Recipe {
    Recipe {
        rate: 400.0,
        burst: 20,
        max: 4096,
        lifetime: [0.4, 0.9],
        speed: [1.0, 3.0],
        direction: [0.0, 1.0, -1.0],
        cone: 25.0,
        inherit: 0.3,
        gravity: [0.0, -9.81, 0.0],
        drag: 1.5,
        spin: [0.5, 2.0],
        seed: 7,
        ..Recipe::default()
    }
}

#[test]
fn same_seed_same_particles_and_seek_matches_stepping() {
    let car = |t: f32| moved(20.0 * t);
    let mut a = Emitter::new(spray(), [0.2, 0.1, -1.5]);
    let mut b = Emitter::new(spray(), [0.2, 0.1, -1.5]);
    a.seek(1.5, STEP, car);
    for n in 0..180 {
        let t = n as f32 * STEP;
        b.step(STEP, car(t), car(t + STEP));
    }
    assert!(a.live() > 100);
    assert_eq!(a.particles(), b.particles());
    a.seek(0.5, STEP, car);
    let mut c = Emitter::new(spray(), [0.2, 0.1, -1.5]);
    c.seek(0.5, STEP, car);
    assert_eq!(a.particles(), c.particles());
    let mut other = Emitter::new(Recipe { seed: 8, ..spray() }, [0.2, 0.1, -1.5]);
    other.seek(0.5, STEP, car);
    assert_ne!(a.particles(), other.particles());
}

#[test]
fn inherit_carries_the_parent_and_gravity_pulls_down() {
    let still = Recipe {
        rate: 0.0,
        burst: 1,
        lifetime: [10.0, 10.0],
        speed: [0.0, 0.0],
        inherit: 1.0,
        ..Recipe::default()
    };
    let mut riding = Emitter::new(still.clone(), [0.0; 3]);
    riding.step(0.1, moved(0.0), moved(1.0));
    assert!((riding.particles()[0].velocity[0] - 10.0).abs() < 1e-4);
    let mut left = Emitter::new(
        Recipe {
            inherit: 0.0,
            ..still.clone()
        },
        [0.0; 3],
    );
    left.step(0.1, moved(0.0), moved(1.0));
    assert_eq!(left.particles()[0].velocity, [0.0; 3]);
    let mut falling = Emitter::new(
        Recipe {
            inherit: 0.0,
            gravity: [0.0, -10.0, 0.0],
            ..still
        },
        [0.0; 3],
    );
    for _ in 0..100 {
        falling.step(0.01, IDENTITY, IDENTITY);
    }
    let particle = falling.particles()[0];
    assert!((particle.velocity[1] + 10.0).abs() < 1e-3);
    assert!((particle.position[1] + 5.05).abs() < 1e-2);
}

#[test]
fn pool_cap_holds_and_counts_drops() {
    let mut emitter = Emitter::new(
        Recipe {
            rate: 10_000.0,
            burst: 500,
            max: 300,
            lifetime: [5.0, 5.0],
            ..Recipe::default()
        },
        [0.0; 3],
    );
    for _ in 0..60 {
        emitter.step(STEP, IDENTITY, IDENTITY);
        assert!(emitter.live() <= 300);
    }
    assert_eq!(emitter.live(), 300);
    assert!(emitter.dropped() > 0);
}

#[test]
fn a_fast_parent_leaves_no_gap_between_steps() {
    let mut emitter = Emitter::new(
        Recipe {
            rate: 1200.0,
            lifetime: [2.0, 2.0],
            speed: [0.0, 0.0],
            ..Recipe::default()
        },
        [0.0; 3],
    );
    emitter.step(1.0 / 60.0, moved(0.0), moved(1.0));
    let mut xs: Vec<f32> = emitter.particles().iter().map(|p| p.position[0]).collect();
    xs.sort_by(f32::total_cmp);
    assert_eq!(xs.len(), 20);
    assert!(xs[0] < 0.05 && xs[19] > 0.95);
    assert!(xs.windows(2).all(|pair| pair[1] - pair[0] < 0.06));
}

#[test]
fn rate_keys_drive_the_rate_deterministically() {
    let keyed = Recipe {
        rate: 1000.0,
        keys: RecipeKeys {
            rate: Some(Gradient::new(
                vec![(0.0, 0.0), (1.0, 0.0), (1.0, 600.0)],
                0.0,
            )),
            ..RecipeKeys::default()
        },
        lifetime: [5.0, 5.0],
        max: 10_000,
        seed: 3,
        ..Recipe::default()
    };
    let mut a = Emitter::new(keyed.clone(), [0.0; 3]);
    a.seek(1.0, STEP, |_| IDENTITY);
    assert_eq!(a.live(), 0);
    a.seek(2.0, STEP, |_| IDENTITY);
    assert!((a.live() as i64 - 598).abs() <= 2, "{}", a.live());
    let mut b = Emitter::new(keyed, [0.0; 3]);
    b.seek(2.0, STEP, |_| IDENTITY);
    assert_eq!(a.particles(), b.particles());
    let ramp = Gradient::new(vec![(2.0, 30.0), (0.0, 10.0)], 0.0);
    assert_eq!(ramp.sample(-1.0), 10.0);
    assert_eq!(ramp.sample(1.0), 20.0);
    assert_eq!(ramp.sample(9.0), 30.0);
}

#[test]
fn keyed_size_colour_and_alpha_follow_the_scene_time() {
    let recipe = Recipe {
        burst: 4,
        lifetime: [10.0, 10.0],
        speed: [0.0, 0.0],
        alpha: Gradient::constant(1.0),
        keys: RecipeKeys {
            size: Some(Gradient::new(
                vec![(0.0, [0.1, 0.1]), (2.0, [0.3, 0.3])],
                [0.1; 2],
            )),
            colour: Some(Gradient::new(
                vec![(0.0, [1.0, 0.0, 0.0]), (2.0, [0.0, 0.0, 1.0])],
                [1.0; 3],
            )),
            alpha: Some(Gradient::new(vec![(0.0, 1.0), (2.0, 0.5)], 1.0)),
            ..RecipeKeys::default()
        },
        ..Recipe::default()
    };
    let mut emitter = Emitter::new(recipe, [0.0; 3]);
    emitter.seek(1.0, STEP, |_| IDENTITY);
    let mut sprites = Vec::new();
    emitter.sprites(&mut sprites);
    assert_eq!(sprites.len(), 4);
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    for sprite in &sprites {
        assert!(near(sprite.size, 0.2), "{sprite:?}");
        assert!(
            near(sprite.colour[0], 0.5) && near(sprite.colour[2], 0.5),
            "{sprite:?}"
        );
        assert!(near(sprite.alpha, 0.75), "{sprite:?}");
    }
}
