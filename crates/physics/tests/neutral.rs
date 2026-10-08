mod fixture;

use pfx_physics::Chain;

const DT: f32 = 1.0 / 120.0;

fn hang(frames: usize) -> Chain {
    let mut chain = Chain::new(fixture::chain());
    let anchor = [100.0, 40.0];
    let beads = [[140.0, 120.0], [200.0, 180.0]];
    for frame in 0..frames {
        chain.step(DT, frame as f32 * DT, anchor, &beads, &[], 1.0);
    }
    chain
}

#[test]
fn a_chain_on_the_neutral_preset_hangs_the_same_way_each_run() {
    let first = hang(1200);
    let second = hang(1200);
    assert_eq!(first.nodes(), second.nodes());
    assert!(!first.nodes().is_empty());
    assert!(first.nodes().iter().flatten().all(|v| v.is_finite()));
    assert_eq!(first.nodes()[0], [100.0, 40.0]);
    let last = *first.nodes().last().unwrap();
    let gap = ((last[0] - 200.0).powi(2) + (last[1] - 180.0).powi(2)).sqrt();
    assert!(gap < 3.0, "{last:?}");
}

#[test]
fn the_neutral_fluid_names_its_own_table_and_tuning() {
    let desc = fixture::fluid();
    assert_eq!((desc.width, desc.height), (960, 540));
    assert!(desc.max_particles.is_power_of_two());
    assert!(desc.settle > 0.0);
    let params = desc.params;
    assert!(
        [
            params.pressure,
            params.contain,
            params.cohesion,
            params.damping,
            params.viscosity,
            params.gather,
            params.life_rate
        ]
        .iter()
        .all(|v| v.is_finite() && *v > 0.0)
    );
}
