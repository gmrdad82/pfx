use super::components::{Component, SampleRatios, UpSampler};

fn horizontal(input: &[i16], output: &mut [i16]) {
    output[0] = input[0];
    output[1] = (input[0] * 3 + input[1] + 2) >> 2;
    for (output_window, input_window) in output[2..].chunks_exact_mut(2).zip(input.windows(3)) {
        let sample = 3 * input_window[1] + 2;
        output_window[0] = (sample + input_window[0]) >> 2;
        output_window[1] = (sample + input_window[2]) >> 2;
    }
    let out_len = output.len() - 2;
    let input_len = input.len() - 2;
    let i_last = &input[input_len..];
    output[out_len] = (3 * i_last[1] + i_last[0] + 2) >> 2;
    output[out_len + 1] = i_last[1];
}

fn vertical(input: &[i16], in_near: &[i16], in_far: &[i16], output: &mut [i16]) {
    let (out_top, out_bottom) = output.split_at_mut(input.len());
    for ((sample, near), x) in input.iter().zip(in_near).zip(out_top) {
        *x = ((3 * sample) + 2 + near) >> 2;
    }
    for ((sample, far), x) in input.iter().zip(in_far).zip(out_bottom) {
        *x = ((3 * sample) + 2 + far) >> 2;
    }
}

fn generic(input: &[i16], output: &mut [i16]) {
    let difference = output.len() / input.len();
    if difference > 0 {
        for (sample, chunk) in input.iter().zip(output.chunks_exact_mut(difference)) {
            chunk.fill(*sample);
        }
    }
}

fn run(
    kind: UpSampler,
    input: &[i16],
    in_near: &[i16],
    in_far: &[i16],
    scratch: &mut [i16],
    output: &mut [i16],
) -> Result<(), String> {
    let n = input.len();
    let bad = || Err("the sampled rows do not line up".to_string());
    match kind {
        UpSampler::NoOp => {}
        UpSampler::Horizontal => {
            if n < 3 || output.len() != n * 2 {
                return bad();
            }
            horizontal(input, output);
        }
        UpSampler::Vertical => {
            if output.len() != n * 2 || in_near.len() != n || in_far.len() != n {
                return bad();
            }
            vertical(input, in_near, in_far, output);
        }
        UpSampler::HV => {
            if n < 3
                || output.len() != n * 4
                || in_near.len() != n
                || in_far.len() != n
                || scratch.len() < n * 2
            {
                return bad();
            }
            let scratch = &mut scratch[..n * 2];
            vertical(input, in_near, in_far, scratch);
            let (top, bottom) = output.split_at_mut(n * 2);
            horizontal(&scratch[..n], top);
            horizontal(&scratch[n..], bottom);
        }
        UpSampler::Generic => {
            if n == 0 {
                return bad();
            }
            generic(input, output);
        }
    }
    Ok(())
}

fn rows(data: &[i16], start: usize, end: usize) -> Result<&[i16], String> {
    data.get(start..end)
        .ok_or_else(|| "invalid component dimensions".to_string())
}

pub(super) fn upsample(
    component: &mut Component,
    mcu_height: usize,
    i: usize,
    scratch: &mut [i16],
    has_vertical_sample: bool,
) -> Result<(), String> {
    match component.sample_ratio {
        SampleRatios::V | SampleRatios::HV => {
            let mut dest_start = 0;
            let width_stride = component.width_stride;
            let stride_bytes_written = width_stride * component.sample_ratio.sample();
            if i > 0 {
                let dest = component
                    .first_row_upsample_dest
                    .get_mut(0..stride_bytes_written)
                    .ok_or("invalid component dimensions")?;
                let row_down = rows(&component.raw_coeff, 0, width_stride)?;
                run(
                    component.up_sampler,
                    &component.row,
                    &component.row_up,
                    row_down,
                    scratch,
                    dest,
                )?;
            }
            let stride = width_stride * component.vertical_sample;
            let stop_offset = component.raw_coeff.len() / width_stride;
            if component.raw_coeff.len() != stop_offset * stride {
                return Err("invalid component dimensions".into());
            }
            for (pos, curr_row) in component.raw_coeff.chunks_exact(width_stride).enumerate() {
                let raw = &component.raw_coeff;
                let (row_up, row_down) = if i == 0 && pos == 0 {
                    (
                        rows(raw, pos * stride, (pos + 1) * stride)?,
                        rows(raw, (pos + 1) * stride, (pos + 2) * stride)?,
                    )
                } else if i > 0 && pos == 0 {
                    (
                        &component.row[..],
                        rows(raw, (pos + 1) * stride, (pos + 2) * stride)?,
                    )
                } else if i == mcu_height.saturating_sub(1) && pos == stop_offset - 1 {
                    (
                        rows(raw, (pos - 1) * stride, pos * stride)?,
                        rows(raw, pos * stride, (pos + 1) * stride)?,
                    )
                } else if pos > 0 && pos < stop_offset - 1 {
                    (
                        rows(raw, (pos - 1) * stride, pos * stride)?,
                        rows(raw, (pos + 1) * stride, (pos + 2) * stride)?,
                    )
                } else if pos == stop_offset - 1 {
                    let prev_row = rows(raw, (pos - 1) * stride, pos * stride)?;
                    if component.row_up.len() != prev_row.len()
                        || component.row.len() != curr_row.len()
                    {
                        return Err("invalid component dimensions".into());
                    }
                    component.row_up.copy_from_slice(prev_row);
                    component.row.copy_from_slice(curr_row);
                    continue;
                } else {
                    return Err("invalid component dimensions".into());
                };
                let dest = component
                    .upsample_dest
                    .get_mut(dest_start..dest_start + stride_bytes_written)
                    .ok_or("invalid component dimensions")?;
                dest_start += stride_bytes_written;
                run(
                    component.up_sampler,
                    curr_row,
                    row_up,
                    row_down,
                    scratch,
                    dest,
                )?;
            }
        }
        SampleRatios::H => {
            let need = component.raw_coeff.len() * 2;
            if component.upsample_dest.len() < need {
                let extra = need - component.upsample_dest.len();
                pfx_core::memory::more(&mut component.upsample_dest, extra, super::SCRATCH)?;
            }
            component.upsample_dest.resize(need, 0);
            if has_vertical_sample {
                let length = component.first_row_upsample_dest.len();
                let last = component
                    .upsample_dest
                    .rchunks_exact(length)
                    .next()
                    .ok_or("invalid component dimensions")?;
                component.first_row_upsample_dest.copy_from_slice(last);
            }
            for (single_row, output_stride) in component
                .raw_coeff
                .chunks_exact(component.width_stride)
                .zip(
                    component
                        .upsample_dest
                        .chunks_exact_mut(component.width_stride * 2),
                )
            {
                run(
                    component.up_sampler,
                    single_row,
                    &[],
                    &[],
                    &mut [],
                    output_stride,
                )?;
            }
        }
        SampleRatios::Generic(h, v) => {
            let width_stride = component.width_stride;
            for (single_row, output_stride) in component.raw_coeff.chunks_exact(width_stride).zip(
                component
                    .upsample_dest
                    .chunks_exact_mut(width_stride * h * v),
            ) {
                for row in output_stride.chunks_exact_mut(width_stride * h) {
                    run(component.up_sampler, single_row, &[], &[], &mut [], row)?;
                }
            }
        }
        SampleRatios::None => {}
    }
    Ok(())
}
