use super::particles::{
    Body, Emitter, Gradient, Ground, IDENTITY, Matrix, PREWARM_MAX, Placed, Recipe, RecipeKeys,
    STEP, rotate,
};
use super::wake::{Recipe as WakeRecipe, Wake};

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

#[test]
fn prewarm_starts_where_an_emitter_started_that_long_before_with_its_parent_held() {
    let moving = |t: f32| {
        let mut m = IDENTITY;
        m[3][0] = 3.0 + 2.0 * t;
        m
    };
    let held = |_: f32| moving(0.0);
    let base = Recipe {
        rate: 50.0,
        lifetime: [0.5, 3.0],
        speed: [0.5, 2.0],
        cone: 40.0,
        gravity: [0.0, 0.4, 0.0],
        drag: 0.7,
        burst: 12,
        max: 2_000,
        seed: 11,
        ..Recipe::default()
    };
    let warmed = Recipe {
        prewarm: 2.0,
        keys: RecipeKeys {
            rate: Some(Gradient::new(vec![(0.0, 50.0), (1.0, 500.0)], 0.0)),
            ..RecipeKeys::default()
        },
        ..base.clone()
    };
    let mut early = Emitter::new(base, [0.0, 0.5, 0.0]);
    early.seek(2.0, STEP, held);
    let mut opened = Emitter::new(warmed.clone(), [0.0, 0.5, 0.0]);
    opened.seek(0.0, STEP, moving);
    assert!(opened.live() > 50, "{}", opened.live());
    assert_eq!(opened.particles(), early.particles());
    assert_eq!(opened.time(), 0.0);
    opened.seek(1.5, STEP, moving);
    let mut straight = Emitter::new(warmed.clone(), [0.0, 0.5, 0.0]);
    straight.seek(1.5, STEP, moving);
    assert_eq!(opened.particles(), straight.particles());
    opened.seek(0.5, STEP, moving);
    opened.seek(1.5, STEP, moving);
    assert_eq!(opened.particles(), straight.particles());
    let capped = Recipe {
        prewarm: 1e9,
        ..warmed
    };
    assert_eq!(capped.prewarm_steps(), (PREWARM_MAX / STEP).round() as u64);
}

fn leaves(seed: u64) -> Recipe {
    Recipe {
        burst: 200,
        max: 200,
        lifetime: [4.0, 5.0],
        speed: [0.0, 0.5],
        cone: 180.0,
        gravity: [0.0, -9.81, 0.0],
        size: [1.0, 1.0],
        alpha: Gradient::constant(1.0),
        seed,
        body: Some(Body {
            ground: Some(Ground::default()),
            ..Body::default()
        }),
        ..Recipe::default()
    }
}

fn rise(m: Matrix, y: f32) -> Matrix {
    let mut m = m;
    m[3][1] = y;
    m
}

#[test]
fn mesh_particles_settle_on_the_ground_and_stay_past_their_life() {
    let mut fall = Emitter::new(leaves(3), [0.0; 3]);
    fall.seek(10.0, STEP, |_| rise(IDENTITY, 1.5));
    assert_eq!(fall.live(), 200);
    for particle in fall.particles() {
        assert!(particle.resting, "{particle:?}");
        assert_eq!(particle.position[1], 0.0);
        assert_eq!(particle.velocity, [0.0; 3]);
        let up = rotate(particle.orientation, [0.0, 1.0, 0.0]);
        assert!(up[1].abs() > 0.9999, "{up:?}");
    }
    let rested = fall.particles().to_vec();
    fall.seek(20.0, STEP, |_| rise(IDENTITY, 1.5));
    assert_eq!(fall.live(), 200);
    for (now, then) in fall.particles().iter().zip(&rested) {
        assert_eq!(now.position, then.position);
        assert_eq!(now.orientation, then.orientation);
        assert_eq!(now.age, then.age);
    }
}

#[test]
fn drag_by_area_and_mass_slows_a_light_card_more_than_a_heavy_one() {
    let fallen = |mass: f32| {
        let recipe = Recipe {
            burst: 50,
            speed: [0.0, 0.0],
            body: Some(Body {
                mass,
                tumble: [0.0, 2.0],
                ..Body::default()
            }),
            ..leaves(5)
        };
        let mut emitter = Emitter::new(recipe, [0.0; 3]);
        emitter.seek(1.5, STEP, |_| rise(IDENTITY, 20.0));
        let heights: f32 = emitter.particles().iter().map(|p| p.position[1]).sum();
        heights / emitter.live() as f32
    };
    let heavy = fallen(1.0);
    let light = fallen(0.001);
    assert!(
        (heavy - (20.0 - 0.5 * 9.81 * 1.5 * 1.5)).abs() < 0.2,
        "{heavy}"
    );
    assert!(light > heavy + 3.0, "light {light}, heavy {heavy}");
}

#[test]
fn mesh_particles_in_a_wake_are_the_same_from_the_same_seed() {
    let car = |t: f32| rise(moved(15.0 * t - 10.0), 0.4);
    let run = |seed: u64, seconds: f32| {
        let recipe = Recipe {
            burst: 0,
            placed: (0..40)
                .map(|i| Placed {
                    position: [i as f32 * 0.25 - 5.0, 0.0, 0.3 * (i % 3) as f32],
                    yaw: i as f32 * 17.0,
                })
                .collect(),
            lifetime: [30.0, 30.0],
            rate: 20.0,
            ..leaves(seed)
        };
        let mut wake = Wake::new(WakeRecipe::default());
        let mut emitter = Emitter::new(recipe, [0.0; 3]);
        let steps = (seconds / STEP).round() as u32;
        for n in 0..steps {
            let t = n as f32 * STEP;
            wake.step(STEP, car(t), car(t + STEP));
            emitter.step_in(STEP, IDENTITY, IDENTITY, Some(&wake));
        }
        let mut pieces = Vec::new();
        emitter.pieces(&mut pieces);
        pieces
    };
    let a = run(9, 3.0);
    let b = run(9, 3.0);
    assert_eq!(a, b);
    assert!(a.len() > 40);
    assert_ne!(a, run(10, 3.0));
    let lifted = a
        .iter()
        .take(40)
        .enumerate()
        .filter(|(i, piece)| piece.position != [*i as f32 * 0.25 - 5.0, 0.0, 0.3 * (i % 3) as f32])
        .count();
    assert!(lifted > 0, "the wake moved no placed piece");
}

struct Breeze([f32; 3]);

impl super::particles::Field for Breeze {
    fn velocity(&self, _: [f32; 3]) -> [f32; 3] {
        self.0
    }
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5)
}

#[test]
fn placement_rotates_scales_and_moves_and_back_steps_along_the_motion() {
    use super::particles::{Piece, UPRIGHT, placement, point, turned};
    let quarter = turned(UPRIGHT, [0.0, std::f32::consts::FRAC_PI_2, 0.0]);
    assert!(close(rotate(quarter, [1.0, 0.0, 0.0]), [0.0, 0.0, -1.0]));
    let m = placement([1.0, 2.0, 3.0], quarter, 2.0);
    assert!(close(point(m, [1.0, 0.0, 0.0]), [1.0, 2.0, 1.0]));
    assert!(close(point(m, [0.0, 0.0, 0.0]), [1.0, 2.0, 3.0]));
    assert_eq!(turned(quarter, [0.0; 3]), quarter);
    assert_eq!(turned(quarter, [f32::NAN; 3]), quarter);
    let piece = Piece {
        position: [1.0, 0.0, 0.0],
        previous: [0.0; 3],
        velocity: [10.0, 0.0, 0.0],
        orientation: UPRIGHT,
        previous_orientation: UPRIGHT,
        angular: [0.0; 3],
        size: 1.0,
        colour: [1.0; 3],
        alpha: 1.0,
        resting: false,
    };
    assert!(close(point(piece.back(0.1), [0.0; 3]), [0.0, 0.0, 0.0]));
    assert_eq!(piece.back(0.0), piece.matrix());
    assert_eq!(piece.back(f32::NAN), piece.matrix());
    let rested = Piece {
        resting: true,
        ..piece
    };
    assert_eq!(rested.back(0.1), rested.matrix());
    assert!(close(point(piece.previous_matrix(), [0.0; 3]), [0.0; 3]));
}

#[test]
fn laid_keeps_the_heading_and_lies_face_up_or_down_whichever_is_nearer() {
    use super::particles::{UPRIGHT, laid, turned};
    let yawed = turned(UPRIGHT, [0.0, 0.7, 0.0]);
    let tilted = turned(yawed, [0.3, 0.0, 0.2]);
    let flat = laid(tilted);
    assert!(rotate(flat, [0.0, 1.0, 0.0])[1] > 0.9999);
    let heading = |q| {
        let x = rotate(q, [1.0, 0.0, 0.0]);
        (-x[2]).atan2(x[0])
    };
    assert!((heading(flat) - heading(yawed)).abs() < 0.2);
    let upside = turned(yawed, [std::f32::consts::PI - 0.2, 0.0, 0.0]);
    assert!(rotate(laid(upside), [0.0, 1.0, 0.0])[1] < -0.9999);
    let edge = turned(UPRIGHT, [0.0, 0.0, std::f32::consts::FRAC_PI_2]);
    let up = rotate(laid(edge), [0.0, 1.0, 0.0])[1];
    assert!(up.abs() > 0.9999, "{up}");
}

#[test]
fn an_emitter_without_a_body_or_a_field_steps_as_it_always_did() {
    let mut plain = Emitter::new(spray(), [0.2, 0.1, -1.5]);
    let mut fielded = Emitter::new(spray(), [0.2, 0.1, -1.5]);
    for n in 0..120 {
        let t = n as f32 * STEP;
        plain.step(STEP, moved(20.0 * t), moved(20.0 * (t + STEP)));
        fielded.step_in(STEP, moved(20.0 * t), moved(20.0 * (t + STEP)), None);
    }
    assert_eq!(plain.particles(), fielded.particles());
    assert!(
        plain
            .particles()
            .iter()
            .all(|p| p.orientation == super::particles::UPRIGHT && !p.resting)
    );
}

#[test]
fn a_sprite_drifts_with_the_wind_only_through_its_drag() {
    let still = |drag: f32| Recipe {
        burst: 1,
        lifetime: [10.0, 10.0],
        speed: [0.0, 0.0],
        drag,
        ..Recipe::default()
    };
    let breeze = Breeze([3.0, 0.0, 0.0]);
    let mut dragged = Emitter::new(still(4.0), [0.0; 3]);
    let mut slick = Emitter::new(still(0.0), [0.0; 3]);
    for _ in 0..240 {
        dragged.step_in(STEP, IDENTITY, IDENTITY, Some(&breeze));
        slick.step_in(STEP, IDENTITY, IDENTITY, Some(&breeze));
    }
    assert!((dragged.particles()[0].velocity[0] - 3.0).abs() < 0.01);
    assert!(dragged.particles()[0].position[0] > 3.0);
    assert_eq!(slick.particles()[0].velocity, [0.0; 3]);
    assert_eq!(slick.particles()[0].position, [0.0; 3]);
}

fn resting_leaf(keep: bool, lift: f32) -> Emitter {
    let recipe = Recipe {
        burst: 0,
        lifetime: [1.0, 1.0],
        placed: vec![Placed {
            position: [0.0, 0.0, 0.0],
            yaw: 45.0,
        }],
        body: Some(Body {
            ground: Some(Ground {
                keep,
                lift,
                ..Ground::default()
            }),
            ..Body::default()
        }),
        gravity: [0.0, -9.81, 0.0],
        ..Recipe::default()
    };
    Emitter::new(recipe, [0.0; 3])
}

#[test]
fn a_placed_piece_rests_where_placed_and_ages_out_without_keep() {
    let mut kept = resting_leaf(true, 2.0);
    let mut aged = resting_leaf(false, 2.0);
    for _ in 0..360 {
        kept.step(STEP, IDENTITY, IDENTITY);
        aged.step(STEP, IDENTITY, IDENTITY);
    }
    assert_eq!(kept.live(), 1);
    let leaf = kept.particles()[0];
    assert!(leaf.resting);
    assert_eq!(leaf.position, [0.0; 3]);
    assert_eq!(leaf.age, 0.0);
    let up = rotate(leaf.orientation, [0.0, 1.0, 0.0]);
    assert!(up[1] > 0.9999);
    assert_eq!(aged.live(), 0);
}

#[test]
fn wind_above_lift_raises_a_resting_piece_and_below_it_does_not() {
    let gust = Breeze([1.5, 4.0, 0.0]);
    let calm = Breeze([1.5, 0.5, 0.0]);
    let mut lifted = resting_leaf(true, 2.0);
    let mut held = resting_leaf(true, 2.0);
    lifted.step(STEP, IDENTITY, IDENTITY);
    held.step(STEP, IDENTITY, IDENTITY);
    for _ in 0..30 {
        lifted.step_in(STEP, IDENTITY, IDENTITY, Some(&gust));
        held.step_in(STEP, IDENTITY, IDENTITY, Some(&calm));
    }
    assert!(!lifted.particles()[0].resting);
    assert!(lifted.particles()[0].position[1] > 0.0);
    assert!(lifted.particles()[0].age > 0.0);
    assert!(held.particles()[0].resting);
    assert_eq!(held.particles()[0].position, [0.0; 3]);
}

#[test]
fn a_lump_takes_no_lift_and_falls_straight_while_a_card_glides_aside() {
    let drop = |flat: f32| {
        let recipe = Recipe {
            burst: 1,
            lifetime: [10.0, 10.0],
            speed: [0.0, 0.0],
            gravity: [0.0, -9.81, 0.0],
            body: Some(Body {
                flat,
                tumble: [0.0, 0.0],
                ..Body::default()
            }),
            seed: 21,
            ..Recipe::default()
        };
        let mut emitter = Emitter::new(recipe, [0.0; 3]);
        emitter.seek(2.0, STEP, |_| rise(IDENTITY, 20.0));
        emitter.particles()[0]
    };
    let lump = drop(0.0);
    let card = drop(1.0);
    assert!(
        lump.position[0].abs() < 1e-4 && lump.position[2].abs() < 1e-4,
        "{lump:?}"
    );
    let aside = (card.position[0].powi(2) + card.position[2].powi(2)).sqrt();
    assert!(aside > 0.01, "{card:?}");
    assert!(card.angular.iter().all(|w| w.is_finite()));
    assert!(super::particles::length(card.angular) <= super::particles::SPIN_MAX + 1e-3);
}

#[test]
fn the_pool_cap_counts_placed_pieces_it_drops() {
    let recipe = Recipe {
        max: 2,
        placed: (0..5)
            .map(|i| Placed {
                position: [i as f32, 0.0, 0.0],
                yaw: 0.0,
            })
            .collect(),
        ..leaves(1)
    };
    let mut emitter = Emitter::new(Recipe { burst: 0, ..recipe }, [0.0; 3]);
    emitter.step(STEP, IDENTITY, IDENTITY);
    assert_eq!(emitter.live(), 2);
    assert_eq!(emitter.dropped(), 3);
}
