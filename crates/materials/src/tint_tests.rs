use crate::{Material, Tint};

#[test]
fn no_tint_leaves_a_material_as_it_is() {
    let material = Material {
        base: [0.3, 0.4, 0.5],
        roughness: 0.6,
        metalness: 0.2,
        ..Material::default()
    };
    assert!(Tint::NONE.is_none());
    assert_eq!(Tint::default(), Tint::NONE);
    assert_eq!(Tint::NONE.apply(material), material);
    assert_eq!(Tint::NONE.base_of([0.3, 0.4, 0.5]), [0.3, 0.4, 0.5]);
}

#[test]
fn a_multiply_tint_scales_the_base_and_keeps_the_finish() {
    let material = Material {
        base: [0.8, 0.8, 0.4],
        roughness: 0.6,
        metalness: 0.2,
        ..Material::default()
    };
    let tint = Tint::multiply([1.0, 0.5, 0.25]);
    assert!(!tint.is_none());
    let tinted = tint.apply(material);
    assert_eq!(tinted.base, [0.8, 0.4, 0.1]);
    assert_eq!((tinted.roughness, tinted.metalness), (0.6, 0.2));
    assert_eq!(
        Material {
            base: material.base,
            ..tinted
        },
        material
    );
}

#[test]
fn a_replacing_tint_swaps_the_base_and_the_finish_it_names() {
    let material = Material {
        base: [0.8, 0.8, 0.8],
        roughness: 0.6,
        metalness: 0.2,
        ..Material::default()
    };
    let swap = Tint {
        base: [0.1, 0.2, 0.3],
        replace: true,
        roughness: Some(0.9),
        metalness: None,
    };
    let swapped = swap.apply(material);
    assert_eq!(swapped.base, [0.1, 0.2, 0.3]);
    assert_eq!((swapped.roughness, swapped.metalness), (0.9, 0.2));
    let metal = Tint {
        metalness: Some(1.0),
        ..Tint::NONE
    };
    let made = metal.apply(material);
    assert_eq!(
        (made.base, made.roughness, made.metalness),
        ([0.8; 3], 0.6, 1.0)
    );
}

#[test]
fn the_gpu_rows_carry_the_mode_and_mark_an_unset_finish_below_zero() {
    assert_eq!(
        Tint::NONE.gpu(),
        [[1.0, 1.0, 1.0, 0.0], [-1.0, -1.0, 0.0, 0.0]]
    );
    let swap = Tint {
        base: [0.1, 0.2, 0.3],
        replace: true,
        roughness: Some(0.0),
        metalness: Some(0.5),
    };
    assert_eq!(swap.gpu(), [[0.1, 0.2, 0.3, 1.0], [0.0, 0.5, 0.0, 0.0]]);
}
