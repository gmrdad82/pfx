use pfx_core::sim::splitmix64;

use super::fft::Complex;
use super::stft::{Stft, std_max, std_min};

const NOISE_FLOOR: f32 = 1e-15;
const MAX_CLEAN_STRETCH: f32 = 2.0;
const DEFAULT_SEED: u64 = 0x5eed;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    #[default]
    Default,
    Cheaper,
}

impl Preset {
    fn blocks(self, rate: u32) -> (usize, usize) {
        let rate = rate as f32 as f64;
        let (block, interval) = match self {
            Preset::Default => (0.12, 0.03),
            Preset::Cheaper => (0.1, 0.04),
        };
        ((rate * block) as usize, (rate * interval) as usize)
    }
}

#[derive(Clone, Copy, Default)]
struct Band {
    input: Complex,
    prev_input: Complex,
    output: Complex,
    input_energy: f32,
}

#[derive(Clone, Copy, Default)]
struct Prediction {
    energy: f32,
    input: Complex,
}

impl Prediction {
    fn make_output(&self, phase: Complex) -> Complex {
        let mut phase = phase;
        let mut norm = phase.norm();
        if norm <= NOISE_FLOOR {
            phase = self.input;
            norm = self.input.norm() + NOISE_FLOOR;
        }
        phase.scale((self.energy / norm).sqrt())
    }
}

#[derive(Clone, Copy)]
struct Peak {
    input: f32,
    output: f32,
}

#[derive(Clone, Copy)]
struct MapPoint {
    input_bin: f32,
    freq_grad: f32,
}

#[derive(Clone, Copy)]
struct BlockState {
    samples_since_last: usize,
    new_spectrum: bool,
    reanalyse_prev: bool,
    mapped_frequencies: bool,
    process_formants: bool,
    time_factor: f32,
}

impl Default for BlockState {
    fn default() -> Self {
        BlockState {
            samples_since_last: usize::MAX,
            new_spectrum: false,
            reanalyse_prev: false,
            mapped_frequencies: false,
            process_formants: false,
            time_factor: 0.0,
        }
    }
}

pub struct Stretch {
    channels: usize,
    rate: u32,
    bands: usize,
    stft: Stft,
    block: BlockState,
    silence_counter: usize,
    silence_first: bool,
    freq_multiplier: f32,
    freq_tonality_limit: f32,
    formant_compensation: bool,
    formant_multiplier: f32,
    inv_formant_multiplier: f32,
    formant_base_freq: f32,
    prev_input_offset: i64,
    did_seek: bool,
    seek_time_factor: f32,
    channel_bands: Vec<Band>,
    peaks: Vec<Peak>,
    energy: Vec<f32>,
    smoothed_energy: Vec<f32>,
    smooth_energy_state: f32,
    output_map: Vec<MapPoint>,
    predictions: Vec<Prediction>,
    formant_metric: Vec<f32>,
    freq_estimate_weighted: f32,
    freq_estimate_weight: f32,
    freq_estimate: f32,
    scratch: Vec<f32>,
    rng: u64,
}

enum Source<'a> {
    Planar(&'a [Vec<f32>], usize),
    Zeros,
}

impl Source<'_> {
    fn at(&self, channel: usize, index: usize) -> f32 {
        match self {
            Source::Planar(data, offset) => data[channel][index + offset],
            Source::Zeros => 0.0,
        }
    }

    fn shifted(&self, by: usize) -> Source<'_> {
        match self {
            Source::Planar(data, offset) => Source::Planar(data, *offset + by),
            Source::Zeros => Source::Zeros,
        }
    }
}

impl Stretch {
    pub fn new(channels: u16, rate: u32) -> Stretch {
        Stretch::with_preset(channels, rate, Preset::Default)
    }

    pub fn with_preset(channels: u16, rate: u32, preset: Preset) -> Stretch {
        let (block, interval) = preset.blocks(rate);
        Stretch::with_blocks(channels, rate, block, interval)
    }

    pub fn with_blocks(
        channels: u16,
        rate: u32,
        block_samples: usize,
        interval_samples: usize,
    ) -> Stretch {
        let channels = (channels as usize).max(1);
        let interval = interval_samples.max(1);
        let block_samples = block_samples.max(interval * 2);
        let mut stft = Stft::new(channels, block_samples, interval + 1, interval);
        stft.reset(0.1);
        let bands = stft.bands();
        Stretch {
            channels,
            rate,
            bands,
            stft,
            block: BlockState::default(),
            silence_counter: 0,
            silence_first: true,
            freq_multiplier: 1.0,
            freq_tonality_limit: 0.5,
            formant_compensation: false,
            formant_multiplier: 1.0,
            inv_formant_multiplier: 1.0,
            formant_base_freq: 0.0,
            prev_input_offset: -1,
            did_seek: false,
            seek_time_factor: 1.0,
            channel_bands: vec![Band::default(); bands * channels],
            peaks: Vec::with_capacity(bands / 2),
            energy: vec![0.0; bands],
            smoothed_energy: vec![0.0; bands],
            smooth_energy_state: 0.0,
            output_map: vec![
                MapPoint {
                    input_bin: 0.0,
                    freq_grad: 1.0
                };
                bands
            ],
            predictions: vec![Prediction::default(); bands * channels],
            formant_metric: vec![0.0; bands + 2],
            freq_estimate_weighted: 0.0,
            freq_estimate_weight: 0.0,
            freq_estimate: 0.0,
            scratch: Vec::with_capacity(block_samples + interval),
            rng: DEFAULT_SEED,
        }
    }

    pub fn channels(&self) -> u16 {
        self.channels as u16
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn block_samples(&self) -> usize {
        self.stft.block_samples()
    }

    pub fn interval_samples(&self) -> usize {
        self.stft.interval()
    }

    pub fn input_latency(&self) -> usize {
        self.stft.analysis_latency()
    }

    pub fn output_latency(&self) -> usize {
        self.stft.synthesis_latency()
    }

    pub fn set_seed(&mut self, seed: u64) {
        self.rng = seed;
    }

    pub fn set_transpose_factor(&mut self, factor: f32, tonality_limit_hz: f32) {
        self.freq_multiplier = factor;
        self.freq_tonality_limit = if tonality_limit_hz > 0.0 {
            (tonality_limit_hz / self.rate as f32) / factor.sqrt()
        } else {
            1.0
        };
    }

    pub fn set_transpose_semitones(&mut self, semitones: f32, tonality_limit_hz: f32) {
        self.set_transpose_factor(semitone_factor(semitones), tonality_limit_hz);
    }

    pub fn set_formant_factor(&mut self, factor: f32, formant_compensation: bool) {
        self.formant_multiplier = factor;
        self.inv_formant_multiplier = 1.0 / factor;
        self.formant_compensation = formant_compensation;
    }

    pub fn set_formant_semitones(&mut self, semitones: f32, formant_compensation: bool) {
        self.set_formant_factor(semitone_factor(semitones), formant_compensation);
    }

    pub fn set_formant_base(&mut self, hz: f32) {
        self.formant_base_freq = hz / self.rate as f32;
    }

    pub fn process(&mut self, input: &[f32], output_frames: usize) -> Vec<f32> {
        let channels = self.channels;
        let frames = input.len() / channels;
        if frames == 0 || output_frames == 0 {
            return vec![0.0; output_frames * channels];
        }
        let rate = frames as f32 / output_frames as f32;
        let needed = self.output_seek_length(rate) + 1;
        let (frames_in, frames_out) = if frames < needed {
            let scaled = (needed as f64 * output_frames as f64 / frames as f64).ceil() as usize;
            (needed, scaled.max(output_frames))
        } else {
            (frames, output_frames)
        };
        let mut planar = vec![vec![0.0f32; frames_in]; channels];
        for (i, frame) in input.chunks_exact(channels).enumerate() {
            for (c, &s) in frame.iter().enumerate() {
                planar[c][i] = s;
            }
        }
        let mut out = vec![vec![0.0f32; frames_out]; channels];
        self.exact(&planar, frames_in, &mut out, frames_out);
        let mut interleaved = Vec::with_capacity(output_frames * channels);
        for i in 0..output_frames {
            for channel in &out {
                interleaved.push(channel[i]);
            }
        }
        interleaved
    }

    pub fn exact_planar(&mut self, input: &[Vec<f32>], output: &mut [Vec<f32>]) -> bool {
        let frames_in = input.first().map_or(0, Vec::len);
        let frames_out = output.first().map_or(0, Vec::len);
        self.exact(input, frames_in, output, frames_out)
    }

    fn output_seek_length(&self, rate: f32) -> usize {
        (self.input_latency() as f32 + rate * self.output_latency() as f32) as usize
    }

    fn exact(
        &mut self,
        input: &[Vec<f32>],
        frames_in: usize,
        output: &mut [Vec<f32>],
        frames_out: usize,
    ) -> bool {
        let rate = frames_in as f32 / frames_out as f32;
        let seek_length = self.output_seek_length(rate);
        if frames_in < seek_length {
            for channel in output.iter_mut() {
                channel[..frames_out].fill(0.0);
            }
            return false;
        }
        let source = Source::Planar(input, 0);
        self.output_seek(&source, seek_length);
        let output_index = (frames_out as f32 - seek_length as f32 / rate) as i64;
        let output_index = output_index.clamp(0, frames_out as i64) as usize;
        self.run(
            &source.shifted(seek_length),
            frames_in - seek_length,
            output,
            0,
            output_index,
        );
        self.flush(output, output_index, frames_out - output_index, rate);
        true
    }

    fn reset(&mut self) {
        self.stft.reset(0.1);
        self.prev_input_offset = -1;
        self.channel_bands.fill(Band::default());
        self.silence_counter = 0;
        self.did_seek = false;
        self.block = BlockState::default();
        self.freq_estimate_weighted = 0.0;
        self.freq_estimate_weight = 0.0;
    }

    fn seek(&mut self, source: &Source, frames: usize, rate: f64) {
        let length = self.stft.block_samples() + self.stft.interval();
        let mut buffer = std::mem::take(&mut self.scratch);
        buffer.clear();
        buffer.resize(length, 0.0);
        let start = frames.saturating_sub(length);
        let pad = length + start - frames;
        let mut total = 0.0f32;
        for c in 0..self.channels {
            for i in start..frames {
                let s = source.at(c, i);
                total += s * s;
                buffer[i - start + pad] = s;
            }
            self.stft.write_input(c, &buffer);
        }
        self.stft.move_input(length);
        self.scratch = buffer;
        if total >= NOISE_FLOOR {
            self.silence_counter = 0;
            self.silence_first = true;
        }
        self.did_seek = true;
        let interval = self.stft.interval();
        self.seek_time_factor = if rate * interval as f64 > 1.0 {
            (1.0 / rate) as f32
        } else {
            interval as f32
        };
    }

    fn output_seek(&mut self, source: &Source, frames: usize) {
        self.reset();
        let latency = self.output_latency();
        let surplus = frames.saturating_sub(self.input_latency());
        let rate = surplus as f32 / latency as f32;
        let seek_frames = frames - surplus;
        self.seek(source, seek_frames, rate as f64);
        let mut pre_roll = vec![vec![0.0f32; latency]; self.channels];
        self.run(
            &source.shifted(seek_frames),
            surplus,
            &mut pre_roll,
            0,
            latency,
        );
        for (c, channel) in pre_roll.iter_mut().enumerate() {
            for v in channel.iter_mut() {
                *v = -*v;
            }
            channel.reverse();
            self.stft.add_output(c, channel);
        }
    }

    fn flush(&mut self, output: &mut [Vec<f32>], at: usize, frames: usize, rate: f32) {
        let interval = self.stft.interval();
        let block = frames.saturating_sub(interval);
        if block > 0 {
            let input = (block as f32 * rate) as usize;
            self.run(&Source::Zeros, input, output, at, block);
        }
        let tail = frames - block;
        let mut buffer = std::mem::take(&mut self.scratch);
        buffer.clear();
        buffer.resize(tail, 0.0);
        self.stft.finish_output();
        for (c, channel) in output.iter_mut().enumerate().take(self.channels) {
            self.stft.read_output(c, 0, &mut buffer);
            channel[at + block..at + block + tail].copy_from_slice(&buffer);
            self.stft.read_output(c, tail, &mut buffer);
            for (i, &v) in buffer.iter().enumerate() {
                channel[at + block + tail - 1 - i] -= v;
            }
        }
        self.scratch = buffer;
        self.stft.reset(0.1);
        for band in &mut self.channel_bands {
            band.prev_input = Complex::ZERO;
            band.output = Complex::ZERO;
        }
    }

    fn copy_input(&mut self, source: &Source, to: usize, copied: &mut usize) {
        let limit = self.stft.block_samples() + self.stft.interval();
        let length = limit.min(to - *copied);
        let offset = to - length;
        let mut buffer = std::mem::take(&mut self.scratch);
        buffer.clear();
        buffer.resize(length, 0.0);
        for c in 0..self.channels {
            for (i, b) in buffer.iter_mut().enumerate() {
                *b = source.at(c, i + offset);
            }
            self.stft.write_input(c, &buffer);
        }
        self.stft.move_input(length);
        self.scratch = buffer;
        *copied = to;
    }

    fn run(
        &mut self,
        source: &Source,
        frames_in: usize,
        output: &mut [Vec<f32>],
        at: usize,
        frames_out: usize,
    ) {
        let mut copied = 0usize;
        let mut total = 0.0f32;
        for c in 0..self.channels {
            for i in 0..frames_in {
                let s = source.at(c, i);
                total += s * s;
            }
        }
        if total < NOISE_FLOOR {
            if self.silence_counter >= 2 * self.stft.block_samples() {
                if self.silence_first {
                    self.silence_first = false;
                    self.block = BlockState::default();
                    for b in &mut self.channel_bands {
                        b.input = Complex::ZERO;
                        b.prev_input = Complex::ZERO;
                        b.output = Complex::ZERO;
                        b.input_energy = 0.0;
                    }
                }
                for (c, channel) in output.iter_mut().enumerate().take(self.channels) {
                    for i in 0..frames_out {
                        channel[at + i] = if frames_in > 0 {
                            source.at(c, i % frames_in)
                        } else {
                            0.0
                        };
                    }
                }
                self.copy_input(source, frames_in, &mut copied);
                return;
            }
            self.silence_counter += frames_in;
        } else {
            self.silence_counter = 0;
            self.silence_first = true;
        }

        let interval = self.stft.interval();
        for index in 0..frames_out {
            if self.block.samples_since_last >= interval {
                self.block.samples_since_last = 0;
                let offset = ((index as f32 * frames_in as f32) / frames_out as f32).round() as i64;
                let input_interval = offset - self.prev_input_offset;
                self.prev_input_offset = offset;
                self.copy_input(source, offset.max(0) as usize, &mut copied);
                let new_spectrum = self.did_seek || input_interval > 0;
                self.block.new_spectrum = new_spectrum;
                self.block.mapped_frequencies = self.freq_multiplier != 1.0;
                if new_spectrum {
                    self.block.reanalyse_prev =
                        self.did_seek || (input_interval - interval as i64).abs() > 1;
                }
                self.block.process_formants = self.formant_multiplier != 1.0
                    || (self.formant_compensation && self.block.mapped_frequencies);
                self.block.time_factor = if self.did_seek {
                    self.seek_time_factor
                } else {
                    interval as f32 / input_interval.max(1) as f32
                };
                self.did_seek = false;
                self.process_block();
            }
            self.block.samples_since_last += 1;
            for (c, channel) in output.iter_mut().enumerate().take(self.channels) {
                channel[at + index] = self.stft.read_one(c);
            }
            self.stft.move_output(1);
        }
        self.copy_input(source, frames_in, &mut copied);
        self.prev_input_offset -= frames_in as i64;
    }

    fn process_block(&mut self) {
        let bands = self.bands;
        if self.block.new_spectrum {
            if self.block.reanalyse_prev {
                let interval = self.stft.interval();
                for c in 0..self.channels {
                    self.stft.analyse(c, interval);
                    let spectrum = self.stft.spectrum(c);
                    for (band, s) in self.channel_bands[c * bands..(c + 1) * bands]
                        .iter_mut()
                        .zip(spectrum)
                    {
                        band.prev_input = *s;
                    }
                }
            }
            for c in 0..self.channels {
                self.stft.analyse(c, 0);
                let spectrum = self.stft.spectrum(c);
                for (band, s) in self.channel_bands[c * bands..(c + 1) * bands]
                    .iter_mut()
                    .zip(spectrum)
                {
                    band.input = *s;
                }
            }
        }
        self.process_spectrum();
        for c in 0..self.channels {
            let spectrum = self.stft.spectrum_mut(c);
            for (s, band) in spectrum
                .iter_mut()
                .zip(&self.channel_bands[c * bands..(c + 1) * bands])
            {
                *s = band.output;
            }
        }
        self.stft.synthesise();
    }

    fn band(&self, channel: usize, index: i64) -> Band {
        if index < 0 || index >= self.bands as i64 {
            return Band::default();
        }
        self.channel_bands[index as usize + channel * self.bands]
    }

    fn fractional(&self, channel: usize, low: i64, frac: f32) -> (Complex, f32, Complex) {
        let a = self.band(channel, low);
        let b = self.band(channel, low + 1);
        (
            lerp(a.input, b.input, frac),
            a.input_energy + (b.input_energy - a.input_energy) * frac,
            lerp(a.prev_input, b.prev_input, frac),
        )
    }

    fn input_at(&self, channel: usize, index: f32) -> Complex {
        let low = index.floor();
        let a = self.band(channel, low as i64);
        let b = self.band(channel, low as i64 + 1);
        lerp(a.input, b.input, index - (low as i64) as f32)
    }

    fn uniform(&mut self, a: f32, b: f32) -> f32 {
        let unit = (splitmix64(&mut self.rng) >> 40) as f32 / (1u64 << 24) as f32;
        unit * (b - a) + a
    }

    fn process_spectrum(&mut self) {
        let bands = self.bands;
        let channels = self.channels;
        let interval = self.stft.interval();
        let smoothing_bins = self.stft.fft_samples() as f32 / interval as f32;
        let long_step = smoothing_bins.round() as usize;
        let time_factor = std_max(self.block.time_factor, 1.0 / MAX_CLEAN_STRETCH);
        let random = time_factor > MAX_CLEAN_STRETCH;
        let dist_a = MAX_CLEAN_STRETCH * 2.0 * (random as u8 as f32) - time_factor;

        if self.block.new_spectrum {
            let tau = (2.0 * std::f64::consts::PI) as f32;
            let first = self.stft.bin_to_freq(0.0);
            let step = self.stft.bin_to_freq(1.0) - first;
            for c in 0..channels {
                let mut rot = Complex::polar(first * interval as f32 * tau);
                let rot_step = Complex::polar(step * interval as f32 * tau);
                for bin in &mut self.channel_bands[c * bands..(c + 1) * bands] {
                    bin.output = bin.output.mul(rot);
                    bin.prev_input = bin.prev_input.mul(rot);
                    rot = rot.mul(rot_step);
                }
            }
        }
        if self.block.mapped_frequencies {
            self.smooth_energy(smoothing_bins);
            self.find_peaks();
            self.update_output_map();
        } else {
            for bin in &mut self.channel_bands {
                bin.input_energy = bin.input.norm();
            }
            for (b, point) in self.output_map.iter_mut().enumerate() {
                *point = MapPoint {
                    input_bin: b as f32,
                    freq_grad: 1.0,
                };
            }
        }
        if self.block.process_formants {
            self.update_formants();
        }

        for c in 0..channels {
            for b in 0..bands {
                let point = self.output_map[b];
                let low = point.input_bin.floor() as i64;
                let frac = point.input_bin - low as f32;
                let prev_energy = self.predictions[c * bands + b].energy;
                let (input, energy, prev_input) = self.fractional(c, low, frac);
                let energy = energy * std_max(0.0, point.freq_grad);
                let prediction = &mut self.predictions[c * bands + b];
                prediction.energy = energy;
                prediction.input = input;
                let bin = &mut self.channel_bands[c * bands + b];
                let freq_twist = input.mul_conj(prev_input);
                let phase = bin.output.mul(freq_twist);
                bin.output = phase.div(std_max(prev_energy, energy) + NOISE_FLOOR);
            }
        }

        for b in 0..bands {
            let mut max_channel = 0;
            let mut max_energy = self.predictions[b].energy;
            for c in 1..channels {
                let e = self.predictions[c * bands + b].energy;
                if e > max_energy {
                    max_channel = c;
                    max_energy = e;
                }
            }
            let base = max_channel * bands;
            let prediction = self.predictions[base + b];
            let point = self.output_map[b];
            let mut phase = Complex::ZERO;
            if b > 0 {
                let factor = if random {
                    self.uniform(dist_a, time_factor)
                } else {
                    time_factor
                };
                let down = self.input_at(max_channel, point.input_bin - factor);
                let twist = prediction.input.mul_conj(down);
                phase = phase.add(self.channel_bands[base + b - 1].output.mul(twist));
                if b >= long_step {
                    let down =
                        self.input_at(max_channel, point.input_bin - long_step as f32 * factor);
                    let twist = prediction.input.mul_conj(down);
                    phase = phase.add(self.channel_bands[base + b - long_step].output.mul(twist));
                }
            }
            if b + 1 < bands {
                let up = self.predictions[base + b + 1];
                let up_point = self.output_map[b + 1];
                let factor = if random {
                    self.uniform(dist_a, time_factor)
                } else {
                    time_factor
                };
                let down = self.input_at(max_channel, up_point.input_bin - factor);
                let twist = up.input.mul_conj(down);
                phase = phase.add(self.channel_bands[base + b + 1].output.mul_conj(twist));
                if b + long_step < bands {
                    let long_up = self.predictions[base + b + long_step];
                    let long_point = self.output_map[b + long_step];
                    let down = self.input_at(
                        max_channel,
                        long_point.input_bin - long_step as f32 * factor,
                    );
                    let twist = long_up.input.mul_conj(down);
                    phase = phase.add(
                        self.channel_bands[base + b + long_step]
                            .output
                            .mul_conj(twist),
                    );
                }
            }
            let output = prediction.make_output(phase);
            self.channel_bands[base + b].output = output;
            for c in 0..channels {
                if c != max_channel {
                    let other = self.predictions[c * bands + b];
                    let twist = other.input.mul_conj(prediction.input);
                    let channel_phase = output.mul(twist);
                    self.channel_bands[c * bands + b].output = other.make_output(channel_phase);
                }
            }
        }

        if self.block.new_spectrum {
            for bin in &mut self.channel_bands {
                bin.prev_input = bin.input;
            }
        }
    }

    fn smooth_energy(&mut self, smoothing_bins: f32) {
        let bands = self.bands;
        let slew = 1.0 / (1.0 + smoothing_bins * 0.5);
        self.energy.fill(0.0);
        for c in 0..self.channels {
            for b in 0..bands {
                let bin = &mut self.channel_bands[c * bands + b];
                let e = bin.input.norm();
                bin.input_energy = e;
                self.energy[b] += e;
            }
        }
        self.smoothed_energy.copy_from_slice(&self.energy);
        let mut e = 0.0f32;
        for _ in 0..2 {
            for s in self.smoothed_energy.iter_mut().rev() {
                e += (*s - e) * slew;
                *s = e;
            }
            for s in self.smoothed_energy.iter_mut() {
                e += (*s - e) * slew;
                *s = e;
            }
        }
        self.smooth_energy_state = e;
    }

    fn map_freq(&self, freq: f32) -> f32 {
        if freq > self.freq_tonality_limit {
            freq + (self.freq_multiplier - 1.0) * self.freq_tonality_limit
        } else {
            freq * self.freq_multiplier
        }
    }

    fn find_peaks(&mut self) {
        self.peaks.clear();
        let bands = self.bands;
        let mut start = 0;
        while start < bands {
            if self.energy[start] > self.smoothed_energy[start] {
                let mut end = start;
                let mut band_sum = 0.0f32;
                let mut energy_sum = 0.0f32;
                while end < bands && self.energy[end] > self.smoothed_energy[end] {
                    band_sum += end as f32 * self.energy[end];
                    energy_sum += self.energy[end];
                    end += 1;
                }
                let avg_band = band_sum / energy_sum;
                let avg_freq = self.stft.bin_to_freq(avg_band);
                let output = self.stft.freq_to_bin(self.map_freq(avg_freq));
                self.peaks.push(Peak {
                    input: avg_band,
                    output,
                });
                start = end;
            }
            start += 1;
        }
    }

    fn update_output_map(&mut self) {
        let bands = self.bands as i64;
        let (Some(&first), Some(&last)) = (self.peaks.first(), self.peaks.last()) else {
            for (b, point) in self.output_map.iter_mut().enumerate() {
                *point = MapPoint {
                    input_bin: b as f32,
                    freq_grad: 1.0,
                };
            }
            return;
        };
        let bottom = first.input - first.output;
        for b in 0..bands.min(first.output.ceil() as i64) {
            self.output_map[b as usize] = MapPoint {
                input_bin: b as f32 + bottom,
                freq_grad: 1.0,
            };
        }
        for pair in self.peaks.windows(2) {
            let (prev, next) = (pair[0], pair[1]);
            let range_scale = 1.0 / (next.output - prev.output);
            let out_offset = prev.input - prev.output;
            let out_scale = next.input - next.output - prev.input + prev.output;
            let grad_scale = out_scale * range_scale;
            let start = 0.max(prev.output.ceil() as i64);
            let end = bands.min(next.output.ceil() as i64);
            for b in start..end {
                let r = (b as f32 - prev.output) * range_scale;
                let h = r * r * (3.0 - 2.0 * r);
                let out_b = b as f32 + out_offset + h * out_scale;
                let grad_h = 6.0 * r * (1.0 - r);
                let grad_b = 1.0 + grad_h * grad_scale;
                self.output_map[b as usize] = MapPoint {
                    input_bin: out_b,
                    freq_grad: grad_b,
                };
            }
        }
        let top = last.input - last.output;
        for b in 0.max(last.output as i64)..bands {
            self.output_map[b as usize] = MapPoint {
                input_bin: b as f32 + top,
                freq_grad: 1.0,
            };
        }
    }

    fn inv_map_formant(&self, freq: f32) -> f32 {
        if freq * self.inv_formant_multiplier > self.freq_tonality_limit {
            freq + (1.0 - self.formant_multiplier) * self.freq_tonality_limit
        } else {
            freq * self.inv_formant_multiplier
        }
    }

    fn estimate_frequency(&mut self) -> f32 {
        let m = &self.formant_metric;
        let mut peaks = [0usize; 3];
        for b in 1..self.bands.saturating_sub(1) {
            let e = m[b];
            if e < m[b - 1] || e <= m[b + 1] {
                continue;
            }
            if e > m[peaks[0]] {
                if e > m[peaks[1]] {
                    if e > m[peaks[2]] {
                        peaks = [peaks[1], peaks[2], b];
                    } else {
                        peaks = [peaks[1], b, peaks[2]];
                    }
                } else {
                    peaks[0] = b;
                }
            }
        }
        let mut estimate = peaks[2] as i64;
        if m[peaks[1]] > m[peaks[2]] * 0.1 {
            let diff = (estimate - peaks[1] as i64).abs();
            if diff > estimate / 8 && diff < estimate * 7 / 8 {
                estimate %= diff;
            }
            if m[peaks[0]] > m[peaks[2]] * 0.01 {
                let diff = (estimate - peaks[0] as i64).abs();
                if diff > estimate / 8 && diff < estimate * 7 / 8 {
                    estimate %= diff;
                }
            }
        }
        let weight = m[peaks[2]];
        self.freq_estimate_weighted +=
            (estimate as f32 * weight - self.freq_estimate_weighted) * 0.25;
        self.freq_estimate_weight += (weight - self.freq_estimate_weight) * 0.25;
        self.freq_estimate_weighted / (self.freq_estimate_weight + 1e-30)
    }

    fn update_formants(&mut self) {
        let bands = self.bands;
        self.formant_metric.fill(0.0);
        for c in 0..self.channels {
            for b in 0..bands {
                self.formant_metric[b] += self.channel_bands[c * bands + b].input_energy;
            }
        }
        self.freq_estimate = self.stft.freq_to_bin(self.formant_base_freq);
        if self.formant_base_freq <= 0.0 {
            self.freq_estimate = self.estimate_frequency();
        }

        let mut decay = 1.0 - 1.0 / (self.freq_estimate * 0.5 + 1.0);
        let mut e = 0.0f32;
        let metric = &mut self.formant_metric[..bands];
        for _ in 0..2 {
            for m in metric.iter_mut().rev() {
                e = std_max(*m, e * decay);
                *m = e;
            }
            for m in metric.iter_mut() {
                e = std_max(*m, e * decay);
                *m = e;
            }
        }
        decay = 1.0 / decay;
        for _ in 0..2 {
            for m in metric.iter_mut().rev() {
                e = std_min(*m, e * decay);
                *m = e;
            }
            for m in metric.iter_mut() {
                e = std_min(*m, e * decay);
                *m = e;
            }
        }

        for b in 0..bands {
            let input_f = self.stft.bin_to_freq(b as f32);
            let output_f = if self.formant_compensation {
                self.map_freq(input_f)
            } else {
                input_f
            };
            let output_f = self.inv_map_formant(output_f);
            let input_e = self.formant_metric[b];
            let target_e = self.formant_at(self.stft.freq_to_bin(output_f));
            let ratio = target_e / (input_e + 1e-30);
            for c in 0..self.channels {
                self.channel_bands[c * bands + b].input_energy *= ratio;
            }
        }
    }

    fn formant_at(&self, band: f32) -> f32 {
        if band < 0.0 {
            return 0.0;
        }
        let band = std_min(band, self.bands as f32);
        let floor = band.floor();
        let index = floor as usize;
        let frac = band - floor;
        let low = self.formant_metric[index];
        let high = self.formant_metric[index + 1];
        low + (high - low) * frac
    }
}

fn lerp(a: Complex, b: Complex, frac: f32) -> Complex {
    a.add(b.sub(a).scale(frac))
}

fn semitone_factor(semitones: f32) -> f32 {
    2f64.powf((semitones / 12.0) as f64) as f32
}

fn frames_of(samples: &[f32], channels: u16) -> usize {
    samples.len() / (channels.max(1) as usize)
}

pub fn pitch_shift(samples: &[f32], channels: u16, rate: u32, semitones: f32) -> Vec<f32> {
    let mut stretch = Stretch::new(channels, rate);
    stretch.set_transpose_semitones(semitones, 0.0);
    stretch.process(samples, frames_of(samples, channels))
}

pub fn formant_shift(samples: &[f32], channels: u16, rate: u32, semitones: f32) -> Vec<f32> {
    let mut stretch = Stretch::new(channels, rate);
    stretch.set_formant_semitones(semitones, false);
    stretch.process(samples, frames_of(samples, channels))
}

pub fn time_stretch(samples: &[f32], channels: u16, rate: u32, factor: f32) -> Vec<f32> {
    let frames = frames_of(samples, channels);
    let out = (frames as f64 * factor.max(0.0) as f64).round() as usize;
    let mut stretch = Stretch::new(channels, rate);
    stretch.process(samples, out)
}
