use super::shape::Shape;

pub const UNITS_PER_EM: u16 = 1024;
pub const UNITS_PER_PX: f32 = 16.0;
pub const FIRST_CHAR: u32 = 0xe000;
pub const FAMILY: &str = "PITO Input Glyphs";

struct Glyf {
    bytes: Vec<u8>,
    advance: u16,
    bounds: [i16; 4],
    points: u16,
    contours: u16,
}

fn units(value: f32) -> i16 {
    (value * UNITS_PER_PX)
        .round()
        .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

fn encode(shape: &Shape) -> Glyf {
    let advance = units(shape.width).max(0) as u16;
    let mut ends = Vec::new();
    let mut flags = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut count = 0u16;
    let mut bounds = [i16::MAX, i16::MAX, i16::MIN, i16::MIN];
    for contour in &shape.contours {
        if contour.len() < 3 {
            continue;
        }
        for point in contour {
            let x = units(point.x);
            let y = units(shape.height - point.y);
            bounds = [
                bounds[0].min(x),
                bounds[1].min(y),
                bounds[2].max(x),
                bounds[3].max(y),
            ];
            flags.push(u8::from(point.on));
            xs.push(x);
            ys.push(y);
            count += 1;
        }
        ends.push(count - 1);
    }
    if ends.is_empty() {
        return Glyf {
            bytes: Vec::new(),
            advance,
            bounds: [0; 4],
            points: 0,
            contours: 0,
        };
    }
    let mut bytes = Vec::new();
    push16(&mut bytes, ends.len() as u16);
    for value in bounds {
        push16(&mut bytes, value as u16);
    }
    for end in &ends {
        push16(&mut bytes, *end);
    }
    push16(&mut bytes, 0);
    bytes.extend_from_slice(&flags);
    let mut previous = 0i16;
    for x in &xs {
        push16(&mut bytes, x.wrapping_sub(previous) as u16);
        previous = *x;
    }
    previous = 0;
    for y in &ys {
        push16(&mut bytes, y.wrapping_sub(previous) as u16);
        previous = *y;
    }
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    Glyf {
        bytes,
        advance,
        bounds,
        points: count,
        contours: ends.len() as u16,
    }
}

fn push16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn push32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

pub fn build(shapes: &[Shape]) -> Vec<u8> {
    let mut glyphs = vec![Glyf {
        bytes: Vec::new(),
        advance: UNITS_PER_EM / 2,
        bounds: [0; 4],
        points: 0,
        contours: 0,
    }];
    glyphs.extend(shapes.iter().map(encode));
    let count = glyphs.len() as u16;
    let mut bounds = [i16::MAX, i16::MAX, i16::MIN, i16::MIN];
    for glyph in glyphs.iter().filter(|glyph| glyph.contours > 0) {
        bounds = [
            bounds[0].min(glyph.bounds[0]),
            bounds[1].min(glyph.bounds[1]),
            bounds[2].max(glyph.bounds[2]),
            bounds[3].max(glyph.bounds[3]),
        ];
    }
    if bounds[0] > bounds[2] {
        bounds = [0; 4];
    }
    let ascender = UNITS_PER_EM as i16;
    let descender = -(UNITS_PER_EM as i16) / 4;

    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for glyph in &glyphs {
        push32(&mut loca, glyf.len() as u32);
        glyf.extend_from_slice(&glyph.bytes);
    }
    push32(&mut loca, glyf.len() as u32);

    let mut head = Vec::new();
    push32(&mut head, 0x0001_0000);
    push32(&mut head, 0x0001_0000);
    push32(&mut head, 0);
    push32(&mut head, 0x5f0f_3cf5);
    push16(&mut head, 0x000b);
    push16(&mut head, UNITS_PER_EM);
    head.extend_from_slice(&[0; 16]);
    for value in bounds {
        push16(&mut head, value as u16);
    }
    push16(&mut head, 0);
    push16(&mut head, 8);
    push16(&mut head, 2);
    push16(&mut head, 1);
    push16(&mut head, 0);

    let max_advance = glyphs.iter().map(|glyph| glyph.advance).max().unwrap_or(0);
    let mut hhea = Vec::new();
    push32(&mut hhea, 0x0001_0000);
    push16(&mut hhea, ascender as u16);
    push16(&mut hhea, descender as u16);
    push16(&mut hhea, 0);
    push16(&mut hhea, max_advance);
    push16(&mut hhea, bounds[0] as u16);
    push16(&mut hhea, 0);
    push16(&mut hhea, bounds[2] as u16);
    push16(&mut hhea, 1);
    push16(&mut hhea, 0);
    push16(&mut hhea, 0);
    hhea.extend_from_slice(&[0; 8]);
    push16(&mut hhea, 0);
    push16(&mut hhea, count);

    let mut hmtx = Vec::new();
    for glyph in &glyphs {
        push16(&mut hmtx, glyph.advance);
        push16(&mut hmtx, glyph.bounds[0] as u16);
    }

    let mut maxp = Vec::new();
    push32(&mut maxp, 0x0001_0000);
    push16(&mut maxp, count);
    push16(
        &mut maxp,
        glyphs.iter().map(|glyph| glyph.points).max().unwrap_or(0),
    );
    push16(
        &mut maxp,
        glyphs.iter().map(|glyph| glyph.contours).max().unwrap_or(0),
    );
    push16(&mut maxp, 0);
    push16(&mut maxp, 0);
    push16(&mut maxp, 2);
    for _ in 0..8 {
        push16(&mut maxp, 0);
    }

    let last_char = FIRST_CHAR + u32::from(count) - 2;
    let mut cmap = Vec::new();
    push16(&mut cmap, 0);
    push16(&mut cmap, 1);
    push16(&mut cmap, 3);
    push16(&mut cmap, 1);
    push32(&mut cmap, 12);
    push16(&mut cmap, 4);
    push16(&mut cmap, 32);
    push16(&mut cmap, 0);
    push16(&mut cmap, 4);
    push16(&mut cmap, 4);
    push16(&mut cmap, 1);
    push16(&mut cmap, 0);
    push16(&mut cmap, last_char as u16);
    push16(&mut cmap, 0xffff);
    push16(&mut cmap, 0);
    push16(&mut cmap, FIRST_CHAR as u16);
    push16(&mut cmap, 0xffff);
    push16(&mut cmap, 1u16.wrapping_sub(FIRST_CHAR as u16));
    push16(&mut cmap, 1);
    push16(&mut cmap, 0);
    push16(&mut cmap, 0);

    let mut os2 = Vec::new();
    push16(&mut os2, 4);
    push16(&mut os2, max_advance);
    push16(&mut os2, 400);
    push16(&mut os2, 5);
    push16(&mut os2, 0);
    for value in [650, 600, 0, 75, 650, 600, 0, 350, 50, 260] {
        push16(&mut os2, value);
    }
    push16(&mut os2, 0);
    os2.extend_from_slice(&[0; 10]);
    for _ in 0..4 {
        push32(&mut os2, 0);
    }
    os2.extend_from_slice(b"PITO");
    push16(&mut os2, 0x0040);
    push16(&mut os2, FIRST_CHAR as u16);
    push16(&mut os2, last_char as u16);
    push16(&mut os2, ascender as u16);
    push16(&mut os2, descender as u16);
    push16(&mut os2, 0);
    push16(&mut os2, ascender as u16);
    push16(&mut os2, (-descender) as u16);
    push32(&mut os2, 1);
    push32(&mut os2, 0);
    push16(&mut os2, UNITS_PER_EM / 2);
    push16(&mut os2, UNITS_PER_EM * 3 / 4);
    push16(&mut os2, 0);
    push16(&mut os2, 0);
    push16(&mut os2, 1);

    let names = [
        (1u16, FAMILY.to_owned()),
        (2, "Regular".to_owned()),
        (4, format!("{FAMILY} Regular")),
        (6, "PITOInputGlyphs-Regular".to_owned()),
    ];
    let mut strings = Vec::new();
    let mut name = Vec::new();
    push16(&mut name, 0);
    push16(&mut name, names.len() as u16);
    push16(&mut name, (6 + 12 * names.len()) as u16);
    for (id, text) in &names {
        let encoded: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        push16(&mut name, 3);
        push16(&mut name, 1);
        push16(&mut name, 0x0409);
        push16(&mut name, *id);
        push16(&mut name, encoded.len() as u16);
        push16(&mut name, strings.len() as u16);
        strings.extend_from_slice(&encoded);
    }
    name.extend_from_slice(&strings);

    let mut post = Vec::new();
    push32(&mut post, 0x0003_0000);
    push32(&mut post, 0);
    push16(&mut post, (-(UNITS_PER_EM as i16) / 10) as u16);
    push16(&mut post, UNITS_PER_EM / 20);
    for _ in 0..5 {
        push32(&mut post, 0);
    }

    let tables: [(&[u8; 4], Vec<u8>); 10] = [
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    let mut font = Vec::new();
    push32(&mut font, 0x0001_0000);
    push16(&mut font, tables.len() as u16);
    push16(&mut font, 128);
    push16(&mut font, 3);
    push16(&mut font, (tables.len() * 16 - 128) as u16);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        push32(&mut font, checksum(data));
        push32(&mut font, offset as u32);
        push32(&mut font, data.len() as u32);
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        while font.len() % 4 != 0 {
            font.push(0);
        }
    }
    font
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}
