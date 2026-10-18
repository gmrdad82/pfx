use crate::image::{ColorSpace, Image, Pixels};
use crate::mesh::Mesh;
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace as ZuneSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

const DECODED: [(&str, &[u8]); 14] = [
    (
        "baseline420.jpg",
        include_bytes!("../../tests/data/baseline420.jpg"),
    ),
    (
        "baseline422.jpg",
        include_bytes!("../../tests/data/baseline422.jpg"),
    ),
    (
        "baseline440.jpg",
        include_bytes!("../../tests/data/baseline440.jpg"),
    ),
    (
        "baseline444.jpg",
        include_bytes!("../../tests/data/baseline444.jpg"),
    ),
    ("grey.jpg", include_bytes!("../../tests/data/grey.jpg")),
    (
        "progressive420.jpg",
        include_bytes!("../../tests/data/progressive420.jpg"),
    ),
    (
        "progressive444.jpg",
        include_bytes!("../../tests/data/progressive444.jpg"),
    ),
    (
        "progressive_restart.jpg",
        include_bytes!("../../tests/data/progressive_restart.jpg"),
    ),
    (
        "restart420.jpg",
        include_bytes!("../../tests/data/restart420.jpg"),
    ),
    (
        "small420.jpg",
        include_bytes!("../../tests/data/small420.jpg"),
    ),
    ("rgb.jpg", include_bytes!("../../tests/data/rgb.jpg")),
    (
        "tall420.jpg",
        include_bytes!("../../tests/data/tall420.jpg"),
    ),
    (
        "tall422.jpg",
        include_bytes!("../../tests/data/tall422.jpg"),
    ),
    (
        "tallgrey.jpg",
        include_bytes!("../../tests/data/tallgrey.jpg"),
    ),
];

const REFUSED: [(&str, &[u8], &str); 4] = [
    (
        "arithmetic.jpg",
        include_bytes!("../../tests/data/arithmetic.jpg"),
        "arithmetic-coded",
    ),
    (
        "twelve.jpg",
        include_bytes!("../../tests/data/twelve.jpg"),
        "12-bit",
    ),
    (
        "lossless.jpg",
        include_bytes!("../../tests/data/lossless.jpg"),
        "lossless",
    ),
    (
        "cmyk.jpg",
        include_bytes!("../../tests/data/cmyk.jpg"),
        "CMYK",
    ),
];

const TRIANGLE: &str = include_str!("../../tests/data/triangle.gltf");
const TRIANGLE_BIN: &[u8] = include_bytes!("../../tests/data/triangle.bin");

fn zune(bytes: &[u8], out: ZuneSpace) -> (usize, usize, Vec<u8>) {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(out);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels = decoder.decode().unwrap();
    let (width, height) = decoder.dimensions().unwrap();
    (width, height, pixels)
}

fn reference(name: &str, bytes: &[u8]) -> (usize, usize, Vec<u8>) {
    if name == "rgb.jpg" {
        let (width, height, rgb) = zune(bytes, ZuneSpace::RGB);
        let rgba = rgb
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        (width, height, rgba)
    } else {
        zune(bytes, ZuneSpace::RGBA)
    }
}

#[test]
fn the_port_decodes_every_fixture_as_zune_jpeg_does() {
    for (name, bytes) in DECODED {
        let image = Image::jpeg(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let (width, height, expected) = reference(name, bytes);
        assert_eq!(
            (image.width as usize, image.height as usize),
            (width, height),
            "{name}"
        );
        assert_eq!(image.space, ColorSpace::Srgb, "{name}");
        let Pixels::Eight(pixels) = &image.pixels else {
            panic!("{name}: expected 8-bit pixels");
        };
        let first = pixels.iter().zip(&expected).position(|(a, b)| a != b);
        assert_eq!(pixels.len(), expected.len(), "{name}");
        assert!(
            first.is_none(),
            "{name}: first difference at byte {first:?}"
        );
    }
}

#[test]
fn unsupported_jpegs_are_refused_by_name() {
    for (name, bytes, reason) in REFUSED {
        let error = Image::jpeg(bytes).unwrap_err();
        assert!(error.starts_with("jpeg: "), "{name}: {error}");
        assert!(error.contains(reason), "{name}: {error}");
    }
    let error = Image::jpeg(&[0xFF, 0xD8, 0xFF]).unwrap_err();
    assert!(error.starts_with("jpeg: "), "{error}");
}

#[test]
fn a_header_past_the_edge_limit_is_refused_before_allocating() {
    let mut bytes = DECODED[0].1.to_vec();
    let sof = bytes
        .windows(2)
        .position(|pair| pair == [0xFF, 0xC0])
        .unwrap();
    bytes[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let error = Image::jpeg(&bytes).unwrap_err();
    assert!(error.contains("the limit is 16384 a side"), "{error}");
}

fn with_image(image: &str) -> String {
    TRIANGLE.replace(
        "\"materials\":[{\"name\":\"ink\"}]",
        &format!(
            "\"materials\":[{{\"name\":\"ink\",\"pbrMetallicRoughness\":{{\"baseColorTexture\":{{\"index\":0}}}}}}],\"textures\":[{{\"source\":0}}],\"images\":[{image}]"
        ),
    )
}

#[test]
fn a_gltf_refuses_an_unsupported_jpeg_by_its_name() {
    let gltf = with_image("{\"uri\":\"maps/arithmetic.jpg\"}");
    let error = Mesh::parse_with(gltf.as_bytes(), |uri| match uri {
        "triangle.bin" => Ok(TRIANGLE_BIN.to_vec()),
        "maps/arithmetic.jpg" => Ok(REFUSED[0].1.to_vec()),
        other => Err(format!("{other}: not found")),
    })
    .unwrap_err();
    assert!(error.contains("maps/arithmetic.jpg"), "{error}");
    assert!(error.contains("arithmetic-coded"), "{error}");

    let photo = REFUSED[3].1;
    let gltf = with_image("{\"bufferView\":5,\"mimeType\":\"image/jpeg\",\"name\":\"ink\"}")
        .replace(
            "{\"buffer\":0,\"byteOffset\":144,\"byteLength\":6}]",
            &format!(
                "{{\"buffer\":0,\"byteOffset\":144,\"byteLength\":6}},{{\"buffer\":0,\"byteOffset\":152,\"byteLength\":{}}}]",
                photo.len()
            ),
        )
        .replace(
            "\"byteLength\":150}",
            &format!("\"byteLength\":{}}}", 152 + photo.len()),
        );
    let mut bin = TRIANGLE_BIN.to_vec();
    bin.resize(152, 0);
    bin.extend_from_slice(photo);
    let error = Mesh::parse_with(gltf.as_bytes(), |uri| match uri {
        "triangle.bin" => Ok(bin.clone()),
        other => Err(format!("{other}: not found")),
    })
    .unwrap_err();
    assert!(error.contains("image 0 (ink)"), "{error}");
    assert!(error.contains("CMYK"), "{error}");
}

#[test]
fn a_gltf_with_an_embedded_jpeg_gets_its_base_map() {
    let photo = DECODED[0].1;
    let gltf = with_image("{\"bufferView\":5,\"mimeType\":\"image/jpeg\"}")
        .replace(
            "{\"buffer\":0,\"byteOffset\":144,\"byteLength\":6}]",
            &format!(
                "{{\"buffer\":0,\"byteOffset\":144,\"byteLength\":6}},{{\"buffer\":0,\"byteOffset\":152,\"byteLength\":{}}}]",
                photo.len()
            ),
        )
        .replace(
            "\"byteLength\":150}",
            &format!("\"byteLength\":{}}}", 152 + photo.len()),
        );
    let mut bin = TRIANGLE_BIN.to_vec();
    bin.resize(152, 0);
    bin.extend_from_slice(photo);
    let mesh = Mesh::parse_with(gltf.as_bytes(), |uri| match uri {
        "triangle.bin" => Ok(bin.clone()),
        other => Err(format!("{other}: not found")),
    })
    .unwrap();
    let base = mesh.surfaces[0].maps.base.as_ref().expect("a base map");
    assert_eq!((base.width, base.height), (33, 17));
    assert_eq!(base.space, ColorSpace::Srgb);
    let Pixels::Eight(pixels) = &base.pixels else {
        panic!("expected 8-bit pixels");
    };
    assert!(pixels.chunks_exact(4).any(|p| p[..3] != [255, 255, 255]));
    assert_eq!(**base, Image::jpeg(photo).unwrap());
}

#[test]
fn truncated_and_corrupted_jpegs_never_panic() {
    for (_, bytes) in [DECODED[0], DECODED[7]] {
        for end in 0..bytes.len() {
            let _ = Image::jpeg(&bytes[..end]);
        }
        for at in 0..bytes.len() {
            let mut broken = bytes.to_vec();
            broken[at] ^= 0xFF;
            let _ = Image::jpeg(&broken);
        }
    }
}
