use super::{Board, Painted, read, read_mark};
use crate::flat::{Fill, Srgba};

fn doc(defs: &str, body: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\" width=\"100\" height=\"100\"><defs>{defs}</defs>{body}</svg>"
    )
}

fn painted(source: &str, scale: f32) -> (Board, Painted) {
    let board = read_mark(source, Srgba::WHITE).unwrap();
    let raster = &board.rasters[0];
    let pixels = raster.pixels(scale, 4096);
    let out = raster.paint(&board.icons, pixels).unwrap();
    (board, out)
}

fn at(board: &Board, out: &Painted, p: [f32; 2]) -> [u8; 4] {
    let r = board.rasters[0].region;
    let x = ((p[0] - r[0]) / (r[2] - r[0]) * out.size[0] as f32) as usize;
    let y = ((p[1] - r[1]) / (r[3] - r[1]) * out.size[1] as f32) as usize;
    let i = (y.min(out.size[1] as usize - 1) * out.size[0] as usize
        + x.min(out.size[0] as usize - 1))
        * 4;
    [
        out.rgba[i],
        out.rgba[i + 1],
        out.rgba[i + 2],
        out.rgba[i + 3],
    ]
}

const SQUARE: &str =
    "<rect x=\"40\" y=\"40\" width=\"20\" height=\"20\" fill=\"#ff0000\" filter=\"url(#f)\"/>";

#[test]
fn a_blur_spreads_the_shape_evenly_and_keeps_its_colour_at_any_scale() {
    let source = doc(
        "<filter id=\"f\" x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\"><feGaussianBlur stdDeviation=\"4\"/></filter>",
        SQUARE,
    );
    let (board, out) = painted(&source, 1.0);
    assert_eq!(board.rasters[0].region, [30.0, 30.0, 70.0, 70.0]);
    assert_eq!(out.size, [40, 40]);
    assert!(matches!(board.draws[0].fill, Fill::Image(_)));
    assert!(at(&board, &out, [50.0, 50.0])[3] > 235);
    let edge = at(&board, &out, [40.0, 50.0])[3] as i32;
    assert!((edge - 128).abs() < 24, "{edge}");
    assert!(at(&board, &out, [31.0, 31.0])[3] < 8);
    let left = at(&board, &out, [36.0, 50.0]);
    let right = at(&board, &out, [63.0, 50.0]);
    assert!(
        (left[3] as i32 - right[3] as i32).abs() <= 4,
        "{left:?} {right:?}"
    );
    assert_eq!(&left[..3], &[255, 0, 0]);
    let mass: f32 = out.rgba.chunks(4).map(|p| p[3] as f32 / 255.0).sum();
    assert!((mass - 400.0).abs() < 16.0, "{mass}");
    let (big_board, big) = painted(&source, 4.0);
    assert_eq!(big.size, [160, 160]);
    for p in [[50.0, 50.0], [40.0, 50.0], [36.0, 50.0], [33.0, 45.0]] {
        let (a, b) = (
            at(&board, &out, p)[3] as i32,
            at(&big_board, &big, p)[3] as i32,
        );
        assert!((a - b).abs() <= 12, "{p:?}: {a} at 1x, {b} at 4x");
    }
}

#[test]
fn an_offset_moves_the_shape_in_user_units() {
    let source = doc(
        "<filter id=\"f\" filterUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"100\" height=\"100\"><feOffset dx=\"20\" dy=\"-10\"/></filter>",
        SQUARE,
    );
    let (board, out) = painted(&source, 2.0);
    assert_eq!(out.size, [200, 200]);
    assert_eq!(at(&board, &out, [45.0, 55.0])[3], 0);
    assert_eq!(at(&board, &out, [65.0, 45.0]), [255, 0, 0, 255]);
    assert_eq!(at(&board, &out, [79.0, 31.0]), [255, 0, 0, 255]);
    assert_eq!(at(&board, &out, [65.0, 55.0])[3], 0);
}

#[test]
fn a_merge_lays_its_nodes_in_order() {
    let source = doc(
        "<filter id=\"f\" x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\"><feGaussianBlur stdDeviation=\"3\" result=\"b\"/><feMerge><feMergeNode in=\"b\"/><feMergeNode in=\"SourceGraphic\"/></feMerge></filter>",
        SQUARE,
    );
    let (board, out) = painted(&source, 1.0);
    assert_eq!(at(&board, &out, [41.0, 41.0]), [255, 0, 0, 255]);
    assert_eq!(at(&board, &out, [50.0, 50.0]), [255, 0, 0, 255]);
    let glow = at(&board, &out, [37.0, 50.0]);
    assert!(glow[3] > 20 && glow[3] < 200, "{glow:?}");
}

#[test]
fn a_flood_composited_in_the_source_alpha_recolours_the_shape() {
    let source = doc(
        "<filter id=\"f\"><feFlood flood-color=\"#00ff00\" flood-opacity=\"0.5\"/><feComposite in2=\"SourceAlpha\" operator=\"in\"/></filter>",
        SQUARE,
    );
    let (board, out) = painted(&source, 1.0);
    assert_eq!(board.rasters[0].region, [38.0, 38.0, 62.0, 62.0]);
    let inside = at(&board, &out, [50.0, 50.0]);
    assert_eq!(&inside[..3], &[0, 255, 0]);
    assert!((inside[3] as i32 - 128).abs() <= 1, "{inside:?}");
    assert_eq!(at(&board, &out, [39.0, 39.0])[3], 0);
}

#[test]
fn a_composite_over_keeps_the_source_on_a_flood_clipped_to_its_subregion() {
    let source = doc(
        "<filter id=\"f\" filterUnits=\"userSpaceOnUse\" x=\"20\" y=\"20\" width=\"60\" height=\"60\"><feFlood flood-color=\"#0000ff\" x=\"20\" y=\"20\" width=\"30\" height=\"60\" result=\"c\"/><feComposite in=\"SourceGraphic\" in2=\"c\" operator=\"over\"/></filter>",
        SQUARE,
    );
    let (board, out) = painted(&source, 1.0);
    assert_eq!(at(&board, &out, [25.0, 25.0]), [0, 0, 255, 255]);
    assert_eq!(at(&board, &out, [45.0, 45.0]), [255, 0, 0, 255]);
    assert_eq!(at(&board, &out, [55.0, 45.0]), [255, 0, 0, 255]);
    assert_eq!(at(&board, &out, [70.0, 25.0])[3], 0);
}

#[test]
fn a_colour_matrix_desaturates_and_swaps_channels() {
    let grey = doc(
        "<filter id=\"f\"><feColorMatrix type=\"saturate\" values=\"0\"/></filter>",
        SQUARE,
    );
    let (board, out) = painted(&grey, 1.0);
    let p = at(&board, &out, [50.0, 50.0]);
    assert_eq!(p[0], p[1]);
    assert_eq!(p[1], p[2]);
    assert!(p[0] > 0 && p[0] < 255 && p[3] == 255, "{p:?}");
    let swap = doc(
        "<filter id=\"f\" color-interpolation-filters=\"sRGB\"><feColorMatrix type=\"matrix\" values=\"0 0 0 0 0  0 0 0 0 0  1 0 0 0 0  0 0 0 1 0\"/></filter>",
        SQUARE,
    );
    let (board, out) = painted(&swap, 1.0);
    assert_eq!(at(&board, &out, [50.0, 50.0]), [0, 0, 255, 255]);
}

#[test]
fn a_filtered_group_rasterises_its_children_together() {
    let source = doc(
        "<filter id=\"f\" filterUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"100\" height=\"100\"><feOffset dx=\"10\"/></filter>",
        "<circle cx=\"10\" cy=\"10\" r=\"5\"/><g filter=\"url(#f)\" opacity=\"0.5\"><rect x=\"20\" y=\"20\" width=\"10\" height=\"10\" fill=\"#ff0000\"/><circle cx=\"60\" cy=\"60\" r=\"8\" fill=\"#0000ff\"/></g>",
    );
    let (board, out) = painted(&source, 1.0);
    assert_eq!(board.draws.len(), 2);
    assert_eq!(board.rasters.len(), 1);
    assert_eq!(board.rasters[0].draw, 1);
    assert_eq!(board.draws[1].opacity, 0.5);
    assert!(board.groups.is_empty());
    assert_eq!(at(&board, &out, [35.0, 25.0]), [255, 0, 0, 255]);
    assert_eq!(at(&board, &out, [70.0, 60.0]), [0, 0, 255, 255]);
    assert_eq!(at(&board, &out, [25.0, 25.0])[3], 0);
}

#[test]
fn filter_regions_follow_their_units() {
    let user = doc(
        "<filter id=\"f\" filterUnits=\"userSpaceOnUse\" x=\"10\" y=\"15\" width=\"50\" height=\"40\"><feOffset/></filter>",
        SQUARE,
    );
    let board = read_mark(&user, Srgba::WHITE).unwrap();
    assert_eq!(board.rasters[0].region, [10.0, 15.0, 60.0, 55.0]);
    let fractions = doc(
        "<filter id=\"f\" x=\"0\" y=\"0.5\" width=\"1\" height=\"0.5\"><feOffset/></filter>",
        SQUARE,
    );
    let board = read_mark(&fractions, Srgba::WHITE).unwrap();
    assert_eq!(board.rasters[0].region, [40.0, 50.0, 60.0, 60.0]);
    let bounded = doc(
        "<filter id=\"f\" primitiveUnits=\"objectBoundingBox\" x=\"-1\" y=\"-1\" width=\"3\" height=\"3\"><feOffset dx=\"0.5\"/></filter>",
        SQUARE,
    );
    let (board, out) = painted(&bounded, 1.0);
    assert_eq!(board.rasters[0].region, [20.0, 20.0, 80.0, 80.0]);
    assert_eq!(at(&board, &out, [55.0, 50.0]), [255, 0, 0, 255]);
    assert_eq!(at(&board, &out, [45.0, 50.0])[3], 0);
}

#[test]
fn unsupported_primitives_are_refused_by_name() {
    let source = doc(
        "<filter id=\"f\"><feTurbulence baseFrequency=\"0.1\"/></filter>",
        SQUARE,
    );
    let e = read_mark(&source, Srgba::WHITE).unwrap_err();
    assert_eq!(e, "the filter primitive <feTurbulence> is unsupported");
    let blend = doc(
        "<filter id=\"f\"><feBlend mode=\"multiply\" in2=\"SourceGraphic\"/></filter>",
        SQUARE,
    );
    let e = read_mark(&blend, Srgba::WHITE).unwrap_err();
    assert_eq!(e, "<feBlend mode=\"multiply\"> is unsupported");
    let unused = doc(
        "<filter id=\"f\"><feTile/></filter>",
        "<circle cx=\"5\" cy=\"5\" r=\"5\"/>",
    );
    assert!(read_mark(&unused, Srgba::WHITE).is_ok());
}

#[test]
fn named_filters_keep_their_meaning_over_a_defined_one() {
    let source = doc(
        "<filter id=\"halo\"><feGaussianBlur stdDeviation=\"8\"/></filter>",
        "<circle cx=\"50\" cy=\"50\" r=\"10\" fill=\"#ff0000\" filter=\"url(#halo)\"/>",
    );
    let board = read(&source, &[("halo", super::Filter::Glow { sigma: 8.0 })]).unwrap();
    assert!(board.rasters.is_empty());
    assert!(matches!(
        board.draws[0].shadow,
        crate::flat::Shadow::Glow(_)
    ));
    let board = read(&source, &[]).unwrap();
    assert_eq!(board.rasters.len(), 1);
}
