use crate::{Codec, lz4};

pub(crate) const AUTO: Codec = Codec::Lz4;
const DEFLATE_LEVEL: u8 = 6;

pub(crate) fn encode(codec: Codec, raw: &[u8]) -> Vec<u8> {
    match codec {
        Codec::Auto | Codec::Store => raw.to_vec(),
        Codec::Deflate => miniz_oxide::deflate::compress_to_vec(raw, DEFLATE_LEVEL),
        Codec::Lz4 => lz4::compress(raw),
    }
}

pub(crate) fn decode(codec: Codec, stored: &[u8], mut out: Vec<u8>) -> Option<Vec<u8>> {
    match codec {
        Codec::Auto | Codec::Store => (stored.len() == out.len()).then(|| {
            out.copy_from_slice(stored);
            out
        }),
        Codec::Deflate => {
            let raw = out.len();
            let written = miniz_oxide::inflate::decompress_slice_iter_to_slice(
                &mut out,
                std::iter::once(stored),
                false,
                true,
            )
            .ok()?;
            (written == raw).then_some(out)
        }
        Codec::Lz4 => lz4::decompress_into(stored, out),
    }
}

pub(crate) fn packed(bytes: &[u8]) -> bool {
    const SIGNATURES: [&[u8]; 8] = [
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff",
        b"RIFF",
        b"\x1f\x8b",
        b"PK\x03\x04",
        b"\x28\xb5\x2f\xfd",
        b"OggS",
        b"fLaC",
    ];
    SIGNATURES
        .iter()
        .any(|signature| bytes.starts_with(signature))
}
