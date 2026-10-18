use super::{MAX_COMPONENTS, filled};

const MAX_SAMP_FACTOR: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComponentID {
    Y,
    Cb,
    Cr,
    Q,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SampleRatios {
    HV,
    V,
    H,
    Generic(usize, usize),
    None,
}

impl SampleRatios {
    pub(super) fn sample(self) -> usize {
        match self {
            SampleRatios::HV => 4,
            SampleRatios::V | SampleRatios::H => 2,
            SampleRatios::Generic(a, b) => a * b,
            SampleRatios::None => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpSampler {
    NoOp,
    Horizontal,
    Vertical,
    HV,
    Generic,
}

#[derive(Clone)]
pub(super) struct Component {
    pub(super) component_id: ComponentID,
    pub(super) vertical_sample: usize,
    pub(super) horizontal_sample: usize,
    pub(super) dc_huff_table: usize,
    pub(super) ac_huff_table: usize,
    pub(super) quantization_table_number: u8,
    pub(super) quantization_table: [i32; 64],
    pub(super) dc_pred: i32,
    pub(super) up_sampler: UpSampler,
    pub(super) width_stride: usize,
    pub(super) id: u8,
    pub(super) raw_coeff: Vec<i16>,
    pub(super) upsample_dest: Vec<i16>,
    pub(super) row_up: Vec<i16>,
    pub(super) row: Vec<i16>,
    pub(super) first_row_upsample_dest: Vec<i16>,
    pub(super) sample_ratio: SampleRatios,
    pub(super) fix_an_annoying_bug: usize,
}

impl Component {
    pub(super) fn from(a: [u8; 3], pos: u8) -> Result<Component, String> {
        let component_id = match pos {
            0 => ComponentID::Y,
            1 => ComponentID::Cb,
            2 => ComponentID::Cr,
            3 => ComponentID::Q,
            _ => return Err(format!("{} components are too many", pos + 1)),
        };
        let horizontal_sample = usize::from(a[1] >> 4);
        let vertical_sample = usize::from(a[1] & 0x0f);
        if horizontal_sample > MAX_SAMP_FACTOR {
            return Err(format!(
                "a bogus horizontal sampling factor {horizontal_sample}"
            ));
        }
        if vertical_sample > MAX_SAMP_FACTOR {
            return Err(format!(
                "a bogus vertical sampling factor {vertical_sample}"
            ));
        }
        let quantization_table_number = a[2];
        if usize::from(quantization_table_number) >= MAX_COMPONENTS {
            return Err(format!(
                "quantization table {quantization_table_number} is out of range"
            ));
        }
        if !horizontal_sample.is_power_of_two() {
            return Err(format!(
                "the horizontal sampling factor {horizontal_sample} is not a power of two"
            ));
        }
        if vertical_sample == 0 {
            return Err("the vertical sampling factor is zero".into());
        }
        Ok(Component {
            component_id,
            vertical_sample,
            horizontal_sample,
            dc_huff_table: 0,
            ac_huff_table: 0,
            quantization_table_number,
            quantization_table: [0; 64],
            dc_pred: 0,
            up_sampler: UpSampler::NoOp,
            width_stride: horizontal_sample,
            id: a[0],
            raw_coeff: Vec::new(),
            upsample_dest: Vec::new(),
            row_up: Vec::new(),
            row: Vec::new(),
            first_row_upsample_dest: Vec::new(),
            sample_ratio: SampleRatios::None,
            fix_an_annoying_bug: 1,
        })
    }

    pub(super) fn setup_upsample_scanline(&mut self) -> Result<(), String> {
        let line = self.width_stride * self.vertical_sample;
        self.row = filled(line, 0)?;
        self.row_up = filled(line, 0)?;
        self.first_row_upsample_dest = filled(line * self.sample_ratio.sample(), 128)?;
        self.upsample_dest = filled(
            self.width_stride * self.sample_ratio.sample() * self.fix_an_annoying_bug * 8,
            0,
        )?;
        Ok(())
    }
}
