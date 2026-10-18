static TABLE: [[u32; 256]; 16] = tables();

const fn tables() -> [[u32; 256]; 16] {
    let mut table = [[0u32; 256]; 16];
    let mut n = 0;
    while n < 256 {
        let mut crc = n as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[0][n] = crc;
        n += 1;
    }
    let mut slice = 1;
    while slice < 16 {
        let mut n = 0;
        while n < 256 {
            let prev = table[slice - 1][n];
            table[slice][n] = (prev >> 8) ^ table[0][(prev & 0xff) as usize];
            n += 1;
        }
        slice += 1;
    }
    table
}

pub fn hash(buf: &[u8]) -> u32 {
    update(0, buf)
}

pub fn update(prev: u32, mut buf: &[u8]) -> u32 {
    let mut crc = !prev;
    while buf.len() >= 64 {
        for _ in 0..4 {
            crc = TABLE[0x0][buf[0xf] as usize]
                ^ TABLE[0x1][buf[0xe] as usize]
                ^ TABLE[0x2][buf[0xd] as usize]
                ^ TABLE[0x3][buf[0xc] as usize]
                ^ TABLE[0x4][buf[0xb] as usize]
                ^ TABLE[0x5][buf[0xa] as usize]
                ^ TABLE[0x6][buf[0x9] as usize]
                ^ TABLE[0x7][buf[0x8] as usize]
                ^ TABLE[0x8][buf[0x7] as usize]
                ^ TABLE[0x9][buf[0x6] as usize]
                ^ TABLE[0xa][buf[0x5] as usize]
                ^ TABLE[0xb][buf[0x4] as usize]
                ^ TABLE[0xc][buf[0x3] as usize ^ ((crc >> 0x18) & 0xff) as usize]
                ^ TABLE[0xd][buf[0x2] as usize ^ ((crc >> 0x10) & 0xff) as usize]
                ^ TABLE[0xe][buf[0x1] as usize ^ ((crc >> 0x08) & 0xff) as usize]
                ^ TABLE[0xf][buf[0x0] as usize ^ (crc & 0xff) as usize];
            buf = &buf[16..];
        }
    }
    for &byte in buf {
        crc = TABLE[0][((crc as u8) ^ byte) as usize] ^ (crc >> 8);
    }
    !crc
}
