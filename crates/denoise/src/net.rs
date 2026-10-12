use crate::tza::{Tensor, Tensors};

pub const RECEPTIVE_FIELD: u32 = 174;
pub const OUT_BLOCKS_PER_GROUP: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Input,
    Layer(usize),
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub src0: Source,
    pub src1: Option<Source>,
    pub upsample: bool,
    pub pool: bool,
    pub level: u32,
    pub in0: u32,
    pub in1: u32,
    pub out: u32,
    pub weights: Vec<f32>,
    pub bias: Vec<f32>,
}

impl Layer {
    pub fn in0_blocks(&self) -> u32 {
        self.in0.div_ceil(4)
    }

    pub fn in1_blocks(&self) -> u32 {
        self.in1.div_ceil(4)
    }

    pub fn out_blocks(&self) -> u32 {
        self.out.div_ceil(4)
    }

    pub fn groups(&self) -> u32 {
        self.out_blocks().div_ceil(OUT_BLOCKS_PER_GROUP)
    }

    pub fn out_level(&self) -> u32 {
        self.level + u32::from(self.pool)
    }

    pub fn macs_per_pixel(&self) -> u64 {
        u64::from(self.in0_blocks() + self.in1_blocks())
            * 4
            * u64::from(self.groups() * OUT_BLOCKS_PER_GROUP * 4)
            * 9
    }
}

pub struct Net {
    pub input_channels: u32,
    pub layers: Vec<Layer>,
}

struct Builder<'a> {
    tensors: &'a Tensors,
    input_channels: u32,
    layers: Vec<Layer>,
}

impl Builder<'_> {
    fn channels(&self, source: Source) -> u32 {
        match source {
            Source::Input => self.input_channels,
            Source::Layer(index) => self.layers[index].out,
        }
    }

    fn level(&self, source: Source) -> u32 {
        match source {
            Source::Input => 0,
            Source::Layer(index) => self.layers[index].out_level(),
        }
    }

    fn tensor(&self, name: &str) -> Result<&Tensor, String> {
        self.tensors
            .get(name)
            .ok_or_else(|| "invalid or corrupted weights blob".to_string())
    }

    fn push(
        &mut self,
        name: &str,
        src0: Source,
        src1: Option<Source>,
        pool: bool,
    ) -> Result<Source, String> {
        let weight = self.tensor(&format!("{name}.weight"))?;
        let bias = self.tensor(&format!("{name}.bias"))?;
        let in0 = self.channels(src0);
        let in1 = src1.map_or(0, |source| self.channels(source));
        let level = match src1 {
            Some(source) => self.level(source),
            None => self.level(src0),
        };
        let upsample = src1.is_some();
        if upsample && self.level(src0) != level + 1 {
            return Err(format!(
                "{name}: the upsampled source is not one level down"
            ));
        }
        if weight.dims.len() != 4 || bias.dims.len() != 1 {
            return Err("invalid convolution weight/bias".into());
        }
        let out = weight.dims[0] as u32;
        if weight.dims[1] as u32 != in0 + in1
            || weight.dims[2] != 3
            || weight.dims[3] != 3
            || bias.dims[0] as u32 != out
        {
            return Err(format!("{name}: the weights do not match the network"));
        }
        let mut layer = Layer {
            name: name.to_string(),
            src0,
            src1,
            upsample,
            pool,
            level,
            in0,
            in1,
            out,
            weights: Vec::new(),
            bias: Vec::new(),
        };
        layer.weights = reorder_weight(&layer, &weight.data);
        layer.bias = reorder_bias(&layer, &bias.data);
        self.layers.push(layer);
        Ok(Source::Layer(self.layers.len() - 1))
    }

    fn conv(&mut self, name: &str, src: Source) -> Result<Source, String> {
        self.push(name, src, None, false)
    }

    fn conv_pool(&mut self, name: &str, src: Source) -> Result<Source, String> {
        self.push(name, src, None, true)
    }

    fn upsample_concat_conv(
        &mut self,
        name: &str,
        src0: Source,
        src1: Source,
    ) -> Result<Source, String> {
        self.push(name, src0, Some(src1), false)
    }
}

fn reorder_weight(layer: &Layer, oihw: &[f32]) -> Vec<f32> {
    let blocks = layer.in0_blocks() + layer.in1_blocks();
    let groups = layer.groups();
    let inputs = (layer.in0 + layer.in1) as usize;
    let per = OUT_BLOCKS_PER_GROUP as usize;
    let mut out = vec![0.0; (groups * blocks) as usize * 9 * 4 * per * 4];
    for g in 0..groups as usize {
        for b in 0..blocks as usize {
            for tap in 0..9 {
                for lane in 0..4 {
                    let padded = b * 4 + lane;
                    let channel = if padded < (layer.in0_blocks() * 4) as usize {
                        (padded < layer.in0 as usize).then_some(padded)
                    } else {
                        let rest = padded - (layer.in0_blocks() * 4) as usize;
                        (rest < layer.in1 as usize).then_some(layer.in0 as usize + rest)
                    };
                    let Some(channel) = channel else {
                        continue;
                    };
                    for ob in 0..per {
                        for j in 0..4 {
                            let o = (g * per + ob) * 4 + j;
                            if o >= layer.out as usize {
                                continue;
                            }
                            let index =
                                ((((g * blocks as usize + b) * 9 + tap) * 4 + lane) * per + ob) * 4
                                    + j;
                            out[index] = oihw[(o * inputs + channel) * 9 + tap];
                        }
                    }
                }
            }
        }
    }
    out
}

fn reorder_bias(layer: &Layer, bias: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; (layer.groups() * OUT_BLOCKS_PER_GROUP * 4) as usize];
    out[..bias.len()].copy_from_slice(bias);
    out
}

pub fn build(tensors: &Tensors, input_channels: u32) -> Result<Net, String> {
    let mut b = Builder {
        tensors,
        input_channels,
        layers: Vec::new(),
    };
    let input = Source::Input;
    let x = b.conv("enc_conv0", input)?;
    let pool1 = b.conv_pool("enc_conv1", x)?;
    let pool2 = b.conv_pool("enc_conv2", pool1)?;
    let pool3 = b.conv_pool("enc_conv3", pool2)?;
    let pool4 = b.conv_pool("enc_conv4", pool3)?;
    let x = b.conv("enc_conv5a", pool4)?;
    let x = b.conv("enc_conv5b", x)?;
    let x = b.upsample_concat_conv("dec_conv4a", x, pool3)?;
    let x = b.conv("dec_conv4b", x)?;
    let x = b.upsample_concat_conv("dec_conv3a", x, pool2)?;
    let x = b.conv("dec_conv3b", x)?;
    let x = b.upsample_concat_conv("dec_conv2a", x, pool1)?;
    let x = b.conv("dec_conv2b", x)?;
    let x = b.upsample_concat_conv("dec_conv1a", x, input)?;
    let x = b.conv("dec_conv1b", x)?;
    b.conv("dec_conv0", x)?;
    if b.layers.last().is_none_or(|layer| layer.out != 3) {
        return Err("the network does not end in three channels".into());
    }
    Ok(Net {
        input_channels,
        layers: b.layers,
    })
}
