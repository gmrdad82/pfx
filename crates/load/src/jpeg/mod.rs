mod bitstream;
mod color;
mod components;
mod headers;
mod huffman;
mod idct;
mod mcu;
mod progressive;
#[cfg(test)]
mod tests;
mod upsample;

use std::cmp::max;

use components::{Component, ComponentID, SampleRatios, UpSampler};
use huffman::HuffmanTable;

const MAX_COMPONENTS: usize = 4;
const MAX_SCANS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
    Luma,
    YCbCr,
    Rgb,
    Cmyk,
    Ycck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Sof(u8),
    Dht,
    Dac,
    Rst(u8),
    Soi,
    Eoi,
    Sos,
    Dqt,
    Dnl,
    Dri,
    App(u8),
    Com,
    Unknown(u8),
}

impl Marker {
    fn from_u8(n: u8) -> Marker {
        match n {
            0xFE => Marker::Com,
            0xC0 => Marker::Sof(0),
            0xC1 => Marker::Sof(1),
            0xC2 => Marker::Sof(2),
            0xC4 => Marker::Dht,
            0xCC => Marker::Dac,
            0xD0..=0xD7 => Marker::Rst(n - 0xD0),
            0xD8 => Marker::Soi,
            0xD9 => Marker::Eoi,
            0xDA => Marker::Sos,
            0xDB => Marker::Dqt,
            0xDC => Marker::Dnl,
            0xDD => Marker::Dri,
            0xE0 => Marker::App(0),
            0xE1 => Marker::App(1),
            0xE2 => Marker::App(2),
            0xED => Marker::App(13),
            0xEE => Marker::App(14),
            _ => Marker::Unknown(n),
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl Reader<'_> {
    fn read_u8(&mut self) -> u8 {
        let byte = self.data.get(self.position).copied().unwrap_or(0);
        self.position += 1;
        byte
    }

    fn read_u8_err(&mut self) -> Result<u8, String> {
        let byte = *self.data.get(self.position).ok_or("the file ends early")?;
        self.position += 1;
        Ok(byte)
    }

    fn get_u16_be(&mut self) -> u16 {
        u16::from_be_bytes([self.read_u8(), self.read_u8()])
    }

    fn get_u16_be_err(&mut self) -> Result<u16, String> {
        let mut bytes = [0; 2];
        self.read_exact(&mut bytes)?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn read_exact(&mut self, out: &mut [u8]) -> Result<(), String> {
        let end = self.position.saturating_add(out.len());
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or("the file ends early")?;
        out.copy_from_slice(bytes);
        self.position = end;
        Ok(())
    }

    fn read4(&mut self) -> Option<u32> {
        let bytes = self
            .data
            .get(self.position..self.position.checked_add(4)?)?;
        self.position += 4;
        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn peek(&self, count: usize) -> Option<&[u8]> {
        self.data
            .get(self.position..self.position.checked_add(count)?)
    }

    fn skip(&mut self, count: usize) {
        self.position = self.position.saturating_add(count);
    }

    fn rewind(&mut self, count: usize) {
        self.position = self.position.saturating_sub(count);
    }

    fn eof(&self) -> bool {
        self.position >= self.data.len()
    }
}

pub(crate) struct Decoder<'a> {
    width: u16,
    height: u16,
    sample_ratio: SampleRatios,
    qt_tables: [Option<[i32; 64]>; MAX_COMPONENTS],
    dc_huffman_tables: [Option<HuffmanTable>; MAX_COMPONENTS],
    ac_huffman_tables: [Option<HuffmanTable>; MAX_COMPONENTS],
    components: Vec<Component>,
    h_max: usize,
    v_max: usize,
    mcu_x: usize,
    mcu_y: usize,
    is_interleaved: bool,
    input: Space,
    is_progressive: bool,
    spec_start: u8,
    spec_end: u8,
    succ_high: u8,
    succ_low: u8,
    num_scans: u8,
    scan_subsampled: bool,
    z_order: [usize; MAX_COMPONENTS],
    restart_interval: usize,
    todo: usize,
    stream: Reader<'a>,
    headers_decoded: bool,
    seen_sof: bool,
}

impl<'a> Decoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Decoder {
            width: 0,
            height: 0,
            sample_ratio: SampleRatios::None,
            qt_tables: [None; MAX_COMPONENTS],
            dc_huffman_tables: [None, None, None, None],
            ac_huffman_tables: [None, None, None, None],
            components: Vec::new(),
            h_max: 1,
            v_max: 1,
            mcu_x: 0,
            mcu_y: 0,
            is_interleaved: false,
            input: Space::YCbCr,
            is_progressive: false,
            spec_start: 0,
            spec_end: 0,
            succ_high: 0,
            succ_low: 0,
            num_scans: 0,
            scan_subsampled: false,
            z_order: [0; MAX_COMPONENTS],
            restart_interval: 0,
            todo: 0x7fff_ffff,
            stream: Reader { data, position: 0 },
            headers_decoded: false,
            seen_sof: false,
        }
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (u32::from(self.width), u32::from(self.height))
    }

    pub(crate) fn read_headers(&mut self) -> Result<(), String> {
        if self.headers_decoded {
            return Ok(());
        }
        let magic = self.stream.get_u16_be_err()?;
        if magic != 0xffd8 {
            return Err(format!("not a JPEG: it starts with 0x{magic:04X}"));
        }
        let mut last_byte = 0;
        loop {
            let mut m = self.stream.read_u8_err()?;
            if (m == 0xFF || m == 0) && last_byte == 0xFF {
                while m == 0xFF || m == 0 {
                    last_byte = m;
                    m = self.stream.read_u8_err()?;
                }
            }
            if last_byte == 0xFF {
                let marker = Marker::from_u8(m);
                self.parse_marker_inner(marker)?;
                if marker == Marker::Sos {
                    self.headers_decoded = true;
                    let is_rgb = self.components.len() == 3
                        && self
                            .components
                            .iter()
                            .zip(*b"RGB")
                            .all(|(component, id)| component.id == id);
                    if is_rgb {
                        self.input = Space::Rgb;
                    }
                    return self.check_space();
                }
            }
            last_byte = m;
        }
    }

    pub(crate) fn decode_into(&mut self, out: &mut [u8]) -> Result<(), String> {
        self.read_headers()?;
        let expected = usize::from(self.width) * usize::from(self.height) * 4;
        let out = out
            .get_mut(..expected)
            .ok_or_else(|| format!("the output needs {expected} bytes"))?;
        if self.is_progressive {
            self.decode_mcu_ycbcr_progressive(out)
        } else {
            self.decode_mcu_ycbcr_baseline(out)
        }
    }

    fn check_space(&mut self) -> Result<(), String> {
        let count = self.components.len();
        if count == 3 && self.input == Space::Cmyk {
            self.input = Space::Rgb;
        }
        if count == 3 && self.input == Space::Ycck {
            self.input = Space::YCbCr;
        }
        if count == 1 {
            self.input = Space::Luma;
        }
        match (self.input, count) {
            (Space::Luma, 1) | (Space::YCbCr | Space::Rgb, 3) => Ok(()),
            (Space::Ycck, 4) => Err("CMYK JPEGs (YCCK-coded) are not supported".into()),
            (_, 4) => Err("CMYK JPEGs are not supported".into()),
            (_, n) => Err(format!("{n}-component JPEGs are not supported")),
        }
    }

    fn skip_segment(&mut self) -> Result<(), String> {
        let length = self.stream.get_u16_be_err()?;
        if length < 2 {
            return Err(format!("a marker has an invalid length {length}"));
        }
        self.stream.skip(usize::from(length - 2));
        Ok(())
    }

    fn parse_marker_inner(&mut self, m: Marker) -> Result<(), String> {
        match m {
            Marker::Sof(kind) => {
                if kind == 2 {
                    self.is_progressive = true;
                }
                headers::parse_start_of_frame(self)?;
            }
            Marker::Unknown(n @ (0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)) => {
                return Err(unsupported_frame(n - 0xC0));
            }
            Marker::App(14) => headers::parse_app14(self)?,
            Marker::Dqt => headers::parse_dqt(self)?,
            Marker::Dht => headers::parse_huffman(self)?,
            Marker::Sos => headers::parse_sos(self)?,
            Marker::Eoi => return Err("the image ends before its data".into()),
            Marker::Dac => return Err("arithmetic-coded JPEGs are not supported".into()),
            Marker::Dnl => return Err("the DNL marker is not supported".into()),
            Marker::Dri => {
                if self.stream.get_u16_be_err()? != 4 {
                    return Err("a bad DRI length".into());
                }
                self.restart_interval = usize::from(self.stream.get_u16_be_err()?);
                self.todo = self.restart_interval;
            }
            _ => self.skip_segment()?,
        }
        Ok(())
    }

    fn set_upsampling(&mut self) -> Result<(), String> {
        if self.h_max == self.v_max && self.h_max == 1 {
            return Ok(());
        }
        for comp in &mut self.components {
            let hs = self.h_max / comp.horizontal_sample;
            let vs = self.v_max / comp.vertical_sample;
            let sampler = match (hs, vs) {
                (1, 1) => {
                    comp.sample_ratio = SampleRatios::None;
                    UpSampler::NoOp
                }
                (2, 1) => {
                    comp.sample_ratio = SampleRatios::H;
                    UpSampler::Horizontal
                }
                (1, 2) => {
                    comp.sample_ratio = SampleRatios::V;
                    UpSampler::Vertical
                }
                (2, 2) => {
                    comp.sample_ratio = SampleRatios::HV;
                    UpSampler::HV
                }
                (hs, vs) => {
                    comp.sample_ratio = SampleRatios::Generic(hs, vs);
                    UpSampler::Generic
                }
            };
            comp.setup_upsample_scanline()?;
            comp.up_sampler = sampler;
        }
        Ok(())
    }

    fn setup_component_params(&mut self) -> Result<(), String> {
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        for component in &mut self.components {
            self.h_max = max(self.h_max, component.horizontal_sample);
            self.v_max = max(self.v_max, component.vertical_sample);
            self.mcu_x = width.div_ceil(self.h_max * 8);
            self.mcu_y = height.div_ceil(self.v_max * 8);
            if self.h_max != 1 || self.v_max != 1 {
                self.is_interleaved = true;
            }
            let table = self.qt_tables[usize::from(component.quantization_table_number)]
                .ok_or_else(|| {
                    format!(
                        "no quantization table for component {:?}",
                        component.component_id
                    )
                })?;
            component.quantization_table = table;
            component.width_stride *= self.mcu_x * 8;
        }
        let sampled_luma = self
            .components
            .iter()
            .find(|c| c.component_id == ComponentID::Y)
            .is_some_and(|y| y.horizontal_sample == 2 || y.vertical_sample == 2);
        if sampled_luma {
            for comp in &mut self.components {
                if comp.component_id != ComponentID::Y
                    && (comp.horizontal_sample != 1 || comp.vertical_sample != 1)
                {
                    comp.fix_an_annoying_bug = 2;
                }
            }
        }
        Ok(())
    }

    fn reset_params(&mut self) {
        self.h_max = 1;
        self.v_max = 1;
        self.sample_ratio = SampleRatios::None;
        self.is_interleaved = false;
        self.components[0].vertical_sample = 1;
        self.components[0].width_stride = usize::from(self.width).div_ceil(8) * 8;
        self.components[0].horizontal_sample = 1;
    }
}

fn unsupported_frame(kind: u8) -> String {
    match kind {
        3 => "lossless JPEGs (SOF3) are not supported".into(),
        5..=7 => format!("hierarchical JPEGs (SOF{kind}) are not supported"),
        _ => format!("arithmetic-coded JPEGs (SOF{kind}) are not supported"),
    }
}

const SCRATCH: &str = "the decode's working memory";

fn filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>, String> {
    pfx_core::memory::filled(len, value, SCRATCH)
}

fn zeroed(len: usize) -> Result<Vec<i16>, String> {
    filled(len, 0)
}

fn calculate_padded_width(actual_width: usize, sub_sample: SampleRatios) -> usize {
    match sub_sample {
        SampleRatios::None | SampleRatios::V => actual_width.div_ceil(8) * 8,
        SampleRatios::H | SampleRatios::HV => actual_width.div_ceil(16) * 16,
        SampleRatios::Generic(h, _) => actual_width.div_ceil(h * 8) * (h * 8),
    }
}
