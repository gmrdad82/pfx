use super::bitstream::BitStream;
use super::components::{Component, SampleRatios};
use super::idct::{idct_int, idct_int_1x1, idct4x4};
use super::upsample::upsample;
use super::{
    Decoder, MAX_COMPONENTS, Marker, Reader, Space, calculate_padded_width, color, zeroed,
};

const MAX_THREADS: usize = 16;
const MIN_BAND_ROWS: usize = 4;

pub(super) enum McuContinuation {
    Ok,
    AnotherSos,
    InterScanMarker(Marker),
    Terminate,
}

#[derive(Clone, Copy)]
struct Frame {
    width: usize,
    padded_width: usize,
    v_max: usize,
    is_interleaved: bool,
    input: Space,
    sample_ratio: SampleRatios,
}

fn convert_rows(
    pixels: &mut [u8],
    px: &mut usize,
    num_iters: usize,
    samples: [&[i16]; MAX_COMPONENTS],
    frame: &Frame,
    count: usize,
) -> Result<(), String> {
    let Some(rest) = pixels.get_mut(*px..) else {
        return Ok(());
    };
    for (pos, output) in rest
        .chunks_exact_mut(frame.width * 4)
        .take(num_iters)
        .enumerate()
    {
        let mut raw: [&[i16]; MAX_COMPONENTS] = [&[]; MAX_COMPONENTS];
        for (j, samp) in raw.iter_mut().enumerate().take(count) {
            *samp = samples[j]
                .get(pos * frame.padded_width..(pos + 1) * frame.padded_width)
                .ok_or("missing samples")?;
        }
        color::convert(&raw, frame.input, output, frame.width);
        *px += frame.width * 4;
    }
    Ok(())
}

fn post_process(
    frame: &Frame,
    components: &mut [Component],
    pixels: &mut [u8],
    i: usize,
    mcu_height: usize,
    pixels_written: &mut usize,
    scratch: &mut [i16],
) -> Result<(), String> {
    let count = components.len();
    let mut px = *pixels_written;
    let is_vertically_sampled = components
        .iter()
        .any(|c| matches!(c.sample_ratio, SampleRatios::HV | SampleRatios::V));
    if frame.is_interleaved {
        for comp in components.iter_mut() {
            upsample(comp, mcu_height, i, scratch, is_vertically_sampled)?;
        }
        if is_vertically_sampled {
            if i > 0 {
                let mut samples: [&[i16]; MAX_COMPONENTS] = [&[]; MAX_COMPONENTS];
                for (samp, component) in samples.iter_mut().zip(components.iter()) {
                    *samp = &component.first_row_upsample_dest;
                }
                convert_rows(pixels, &mut px, frame.v_max, samples, frame, count)?;
            }
            for component in components.iter_mut() {
                if component.sample_ratio != SampleRatios::H {
                    let size = component.vertical_sample
                        * component.width_stride
                        * component.sample_ratio.sample();
                    let last = component
                        .raw_coeff
                        .rchunks_exact(size)
                        .next()
                        .filter(|last| last.len() == component.first_row_upsample_dest.len())
                        .ok_or("invalid component dimensions")?;
                    component.first_row_upsample_dest.copy_from_slice(last);
                }
            }
        }
        let mut samples: [&[i16]; MAX_COMPONENTS] = [&[]; MAX_COMPONENTS];
        for (samp, component) in samples.iter_mut().zip(components.iter()) {
            *samp = if component.sample_ratio == SampleRatios::None {
                &component.raw_coeff
            } else {
                &component.upsample_dest
            };
        }
        let is_last_considered = is_vertically_sampled && i != mcu_height.saturating_sub(1);
        let num_iters = (8 - usize::from(is_last_considered)) * frame.v_max;
        convert_rows(pixels, &mut px, num_iters, samples, frame, count)?;
    } else {
        let mut samples: [&[i16]; MAX_COMPONENTS] = [&[]; MAX_COMPONENTS];
        for (samp, component) in samples.iter_mut().zip(components.iter()) {
            *samp = &component.raw_coeff;
        }
        let num_iters = if let SampleRatios::Generic(_, v) = frame.sample_ratio {
            8 * v
        } else {
            8
        };
        convert_rows(pixels, &mut px, num_iters, samples, frame, count)?;
    }
    *pixels_written = px;
    Ok(())
}

fn idct_block(tmp: &mut [i32; 64], out: &mut [i16], stride: usize) {
    if tmp[1..].iter().all(|&c| c == 0) {
        idct_int_1x1(tmp, out, stride);
    } else if tmp[32..].iter().all(|&c| c == 0)
        && (0..4).all(|row| tmp[row * 8 + 4..row * 8 + 8].iter().all(|&c| c == 0))
    {
        idct4x4(tmp, out, stride);
    } else {
        idct_int(tmp, out, stride);
    }
}

fn idct_row(
    component: &mut Component,
    plane: &[i16],
    i: usize,
    mcu_height: usize,
    tmp: &mut [i32; 64],
) -> Result<(), String> {
    let step = plane.len() / mcu_height;
    let slice = plane
        .get(i * step..(i + 1) * step)
        .ok_or("invalid component dimensions")?;
    let width_stride = component.width_stride;
    for k in 0..component.vertical_sample {
        for j in 0..width_stride / 8 {
            let start = j * 64 + k * 8 * width_stride;
            let coefficients = slice
                .get(start..start + 64)
                .ok_or("invalid component dimensions")?;
            for ((x, out), q) in coefficients
                .iter()
                .zip(tmp.iter_mut())
                .zip(&component.quantization_table)
            {
                *out = i32::from(*x).wrapping_mul(*q);
            }
            let out = component
                .raw_coeff
                .get_mut(k * 8 * width_stride + j * 8..)
                .ok_or("invalid component dimensions")?;
            idct_block(tmp, out, width_stride);
        }
    }
    Ok(())
}

struct Band<'p> {
    first: usize,
    end: usize,
    pixels: &'p mut [u8],
}

fn run_band(
    frame: &Frame,
    components: &mut [Component],
    planes: &[Vec<i16>; MAX_COMPONENTS],
    mcu_height: usize,
    band: Band,
) -> Result<(), String> {
    let scratch_size = usize::from(frame.is_interleaved) * components[0].width_stride;
    let mut scratch = zeroed(scratch_size)?;
    let mut tmp = [0_i32; 64];
    let mut px = 0;
    for i in band.first.saturating_sub(1)..band.end {
        for (component, plane) in components.iter_mut().zip(planes) {
            idct_row(component, plane, i, mcu_height, &mut tmp)?;
        }
        let out: &mut [u8] = if i < band.first {
            &mut []
        } else {
            &mut *band.pixels
        };
        post_process(frame, components, out, i, mcu_height, &mut px, &mut scratch)?;
    }
    Ok(())
}

pub(super) fn get_marker(reader: &mut Reader, stream: &mut BitStream) -> Result<Marker, String> {
    if let Some(marker) = stream.marker.take() {
        return Ok(marker);
    }
    while !reader.eof() {
        let marker = reader.read_u8_err()?;
        if marker == 255 {
            let mut r = reader.read_u8_err()?;
            while r == 0xFF {
                r = reader.read_u8_err()?;
            }
            if r != 0 {
                return Ok(Marker::from_u8(r));
            }
        }
    }
    Err("no marker before the end of the data".into())
}

impl Decoder<'_> {
    fn check_tables(&self) -> Result<(), String> {
        for component in &self.components {
            if !matches!(
                self.dc_huffman_tables.get(component.dc_huff_table),
                Some(Some(_))
            ) {
                return Err(format!(
                    "no DC Huffman table for component {:?}",
                    component.component_id
                ));
            }
            if !matches!(
                self.ac_huffman_tables.get(component.ac_huff_table),
                Some(Some(_))
            ) {
                return Err(format!(
                    "no AC Huffman table for component {:?}",
                    component.component_id
                ));
            }
        }
        Ok(())
    }

    pub(super) fn coefficient_planes(
        &self,
        mcu_width: usize,
        mcu_height: usize,
    ) -> Result<[Vec<i16>; MAX_COMPONENTS], String> {
        let mut planes: [Vec<i16>; MAX_COMPONENTS] = Default::default();
        for (component, plane) in self.components.iter().zip(&mut planes) {
            *plane = zeroed(
                mcu_width
                    * component.vertical_sample
                    * component.horizontal_sample
                    * mcu_height
                    * 64,
            )?;
        }
        Ok(planes)
    }

    pub(super) fn decode_mcu_ycbcr_baseline(&mut self, pixels: &mut [u8]) -> Result<(), String> {
        self.setup_component_params()?;
        self.check_tables()?;
        let (mut mcu_width, mut mcu_height);
        if self.is_interleaved {
            self.set_upsampling()?;
            mcu_width = self.mcu_x;
            mcu_height = self.mcu_y;
        } else {
            mcu_width = usize::from(self.width).div_ceil(8);
            mcu_height = usize::from(self.height).div_ceil(8);
        }
        if self.input == Space::Luma && self.is_interleaved {
            self.reset_params();
            mcu_width = usize::from(self.width).div_ceil(8);
            mcu_height = usize::from(self.height).div_ceil(8);
        }
        let mut stream = BitStream::new();
        let all_components_in_first_scan = usize::from(self.num_scans) == self.components.len();
        let mut planes = self.coefficient_planes(mcu_width, mcu_height)?;
        'sos: loop {
            for i in 0..mcu_height {
                if stream.overread_by > 0 {
                    break;
                }
                let terminate = if all_components_in_first_scan {
                    self.decode_mcu_width::<false>(mcu_width, i, &mut stream, &mut planes)?
                } else {
                    self.decode_mcu_width::<true>(mcu_width, i, &mut stream, &mut planes)?
                };
                match terminate {
                    McuContinuation::Ok => {}
                    McuContinuation::AnotherSos if all_components_in_first_scan => break 'sos,
                    McuContinuation::AnotherSos => continue 'sos,
                    McuContinuation::InterScanMarker(marker) => {
                        if self.advance_to_next_sos(marker, &mut stream)? {
                            continue 'sos;
                        }
                        break;
                    }
                    McuContinuation::Terminate => break 'sos,
                }
            }
            break;
        }
        self.finish(&planes, pixels)?;
        if !stream.seen_eoi {
            let _ = get_marker(&mut self.stream, &mut stream);
        }
        Ok(())
    }

    pub(super) fn finish(
        &mut self,
        planes: &[Vec<i16>; MAX_COMPONENTS],
        pixels: &mut [u8],
    ) -> Result<(), String> {
        let height = usize::from(self.height);
        let width = usize::from(self.width);
        let mcu_height = if self.is_interleaved {
            self.mcu_y
        } else {
            height.div_ceil(8)
        };
        if mcu_height == 0 || self.components.is_empty() {
            return Ok(());
        }
        let frame = Frame {
            width,
            padded_width: calculate_padded_width(width, self.sample_ratio),
            v_max: self.v_max,
            is_interleaved: self.is_interleaved,
            input: self.input,
            sample_ratio: self.sample_ratio,
        };
        for comp in &mut self.components {
            comp.raw_coeff = zeroed(comp.width_stride * comp.vertical_sample * 8)?;
        }
        let vertically_sampled = self.is_interleaved
            && self
                .components
                .iter()
                .any(|c| matches!(c.sample_ratio, SampleRatios::HV | SampleRatios::V));
        let v_max = if self.is_interleaved { self.v_max } else { 1 };
        let rows_before = |i: usize| {
            if i >= mcu_height {
                height
            } else if vertically_sampled && i > 0 {
                (8 * v_max * i - v_max).min(height)
            } else {
                (8 * v_max * i).min(height)
            }
        };
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(MAX_THREADS)
            .min(mcu_height / MIN_BAND_ROWS)
            .max(1);
        let per_band = mcu_height.div_ceil(threads);
        let mut bands = Vec::with_capacity(threads);
        let mut rest = pixels;
        for first in (0..mcu_height).step_by(per_band) {
            let end = (first + per_band).min(mcu_height);
            let len = ((rows_before(end) - rows_before(first)) * width * 4).min(rest.len());
            let (head, tail) = std::mem::take(&mut rest).split_at_mut(len);
            rest = tail;
            bands.push(Band {
                first,
                end,
                pixels: head,
            });
        }
        let components = &self.components;
        if bands.len() == 1 {
            let mut own = components.clone();
            return bands.pop().map_or(Ok(()), |band| {
                run_band(&frame, &mut own, planes, mcu_height, band)
            });
        }
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(bands.len());
            for band in bands {
                let mut own = components.clone();
                let frame = &frame;
                let handle = std::thread::Builder::new()
                    .spawn_scoped(scope, move || {
                        run_band(frame, &mut own, planes, mcu_height, band)
                    })
                    .map_err(|e| format!("a decode thread did not start: {e}"))?;
                handles.push(handle);
            }
            for handle in handles {
                handle
                    .join()
                    .map_err(|_| "a decode thread panicked".to_string())??;
            }
            Ok(())
        })
    }

    fn decode_mcu_width<const PROGRESSIVE: bool>(
        &mut self,
        mcu_width: usize,
        i: usize,
        stream: &mut BitStream,
        planes: &mut [Vec<i16>; MAX_COMPONENTS],
    ) -> Result<McuContinuation, String> {
        if self.scan_subsampled {
            self.inner_decode_mcu_width::<PROGRESSIVE, true>(mcu_width, i, stream, planes)
        } else {
            self.inner_decode_mcu_width::<PROGRESSIVE, false>(mcu_width, i, stream, planes)
        }
    }

    fn inner_decode_mcu_width<const PROGRESSIVE: bool, const SAMPLED: bool>(
        &mut self,
        mcu_width: usize,
        i: usize,
        stream: &mut BitStream,
        planes: &mut [Vec<i16>; MAX_COMPONENTS],
    ) -> Result<McuContinuation, String> {
        let z_order = self.z_order;
        let z_scans = &z_order[..usize::from(self.num_scans)];
        let mut scan_du_width = if PROGRESSIVE {
            let comp = &self.components[z_scans[0]];
            (usize::from(self.width) * comp.horizontal_sample).div_ceil(self.h_max * 8)
        } else {
            mcu_width
        };
        if PROGRESSIVE && z_scans.len() > 1 {
            let min_du = z_scans
                .iter()
                .map(|&k| self.components[k].width_stride / 8)
                .min()
                .unwrap_or(0);
            scan_du_width = scan_du_width.min(min_du);
        }
        for j in 0..scan_du_width {
            for &k in z_scans {
                let component = &mut self.components[k];
                let dc_table = self.dc_huffman_tables[component.dc_huff_table % MAX_COMPONENTS]
                    .as_ref()
                    .ok_or("a DC table is missing")?;
                let ac_table = self.ac_huffman_tables[component.ac_huff_table % MAX_COMPONENTS]
                    .as_ref()
                    .ok_or("an AC table is missing")?;
                let blocks_per_row = component.width_stride / 8;
                let (v_step, h_step) = if SAMPLED && !PROGRESSIVE {
                    (component.vertical_sample, component.horizontal_sample)
                } else {
                    (1, 1)
                };
                for v_samp in 0..v_step {
                    for h_samp in 0..h_step {
                        let position = if PROGRESSIVE {
                            64 * (j + i * component.vertical_sample * blocks_per_row)
                        } else {
                            64 * ((j * component.horizontal_sample + h_samp)
                                + (i * component.vertical_sample + v_samp) * blocks_per_row)
                        };
                        let block: &mut [i16; 64] = planes[k]
                            .get_mut(position..position + 64)
                            .and_then(|block| block.try_into().ok())
                            .ok_or("invalid component dimensions")?;
                        block.fill(0);
                        if stream
                            .decode_mcu_block(
                                &mut self.stream,
                                dc_table,
                                ac_table,
                                block,
                                &mut component.dc_pred,
                            )
                            .is_err()
                        {
                            return Ok(McuContinuation::Terminate);
                        }
                    }
                }
            }
            self.todo = self.todo.wrapping_sub(1);
            if self.todo == 0 {
                self.handle_rst_main(stream)?;
                continue;
            }
            if stream.marker.is_some() && stream.bits_left == 0 {
                break;
            }
        }
        self.check_stream_marker_after_mcu_width(stream)
    }

    fn check_stream_marker_after_mcu_width(
        &mut self,
        stream: &mut BitStream,
    ) -> Result<McuContinuation, String> {
        if let Some(m) = stream.marker {
            match m {
                Marker::Eoi => {
                    stream.marker = None;
                    stream.seen_eoi = true;
                }
                Marker::Rst(_) => {
                    if self.todo == 0 {
                        self.handle_rst(stream);
                    }
                }
                Marker::Sos => {
                    self.parse_marker_inner(m)?;
                    stream.marker = None;
                    stream.reset();
                    return Ok(McuContinuation::AnotherSos);
                }
                Marker::Dht | Marker::Dqt | Marker::Dri | Marker::Com | Marker::App(_) => {
                    stream.marker = None;
                    return Ok(McuContinuation::InterScanMarker(m));
                }
                _ => {
                    self.parse_marker_inner(m)?;
                    stream.marker = None;
                    stream.reset();
                    return Ok(McuContinuation::Terminate);
                }
            }
        }
        Ok(McuContinuation::Ok)
    }

    fn advance_to_next_sos(
        &mut self,
        first_marker: Marker,
        stream: &mut BitStream,
    ) -> Result<bool, String> {
        const MAX_INTER_SCAN_MARKERS: usize = 64;
        self.parse_marker_inner(first_marker)?;
        stream.reset();
        for _ in 0..MAX_INTER_SCAN_MARKERS {
            match get_marker(&mut self.stream, stream)? {
                Marker::Sos => {
                    self.parse_marker_inner(Marker::Sos)?;
                    stream.reset();
                    return Ok(true);
                }
                Marker::Eoi => {
                    stream.seen_eoi = true;
                    return Ok(false);
                }
                marker @ (Marker::Dht
                | Marker::Dqt
                | Marker::Dri
                | Marker::Com
                | Marker::App(_)) => {
                    self.parse_marker_inner(marker)?;
                }
                _ => {
                    let length = self.stream.get_u16_be_err()?;
                    if length >= 2 {
                        self.stream.skip(usize::from(length - 2));
                    }
                }
            }
        }
        Err("more than 64 markers between scans".into())
    }

    pub(super) fn handle_rst(&mut self, stream: &mut BitStream) {
        self.todo = self.restart_interval;
        if let Some(Marker::Rst(_)) = stream.marker {
            stream.reset();
            self.components.iter_mut().for_each(|x| x.dc_pred = 0);
        }
    }

    pub(super) fn handle_rst_main(&mut self, stream: &mut BitStream) -> Result<(), String> {
        if self.todo == 0 {
            stream.refill(&mut self.stream)?;
        }
        if self.todo == 0
            && self.restart_interval != 0
            && stream.marker.is_none()
            && !stream.seen_eoi
            && let Ok(marker) = get_marker(&mut self.stream, stream)
        {
            stream.marker = Some(marker);
        }
        if self.todo == 0 {
            self.handle_rst(stream);
        }
        Ok(())
    }
}
