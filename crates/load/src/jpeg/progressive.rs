use super::bitstream::BitStream;
use super::headers::parse_sos;
use super::mcu::get_marker;
use super::{Decoder, MAX_COMPONENTS, MAX_SCANS, Marker, Space};

impl Decoder<'_> {
    pub(super) fn decode_mcu_ycbcr_progressive(&mut self, pixels: &mut [u8]) -> Result<(), String> {
        self.setup_component_params()?;
        let mut seen_scans = 1;
        if self.input == Space::Luma && self.is_interleaved {
            self.reset_params();
        }
        if self.is_interleaved {
            self.set_upsampling()?;
        }
        let (mcu_width, mcu_height) = if self.is_interleaved {
            (self.mcu_x, self.mcu_y)
        } else {
            (
                usize::from(self.width).div_ceil(8),
                usize::from(self.height).div_ceil(8),
            )
        };
        let mut block = self.coefficient_planes(mcu_width, mcu_height)?;
        let mut stream = BitStream::new_progressive(self.succ_low, self.spec_start, self.spec_end);
        if self
            .parse_entropy_coded_data(&mut stream, &mut block)
            .is_err()
        {
            return self.finish(&block, pixels);
        }
        let mut marker = stream
            .marker
            .take()
            .ok_or("a marker is missing where one was expected")?;
        'eoi: while marker != Marker::Eoi {
            match marker {
                Marker::Sos => {
                    parse_sos(self)?;
                    stream.update_progressive_params(self.succ_low, self.spec_start, self.spec_end);
                    if self
                        .parse_entropy_coded_data(&mut stream, &mut block)
                        .is_err()
                    {
                        break 'eoi;
                    }
                    match get_marker(&mut self.stream, &mut stream) {
                        Ok(next) => {
                            marker = next;
                            seen_scans += 1;
                            if seen_scans > MAX_SCANS {
                                return Err(format!("more than {MAX_SCANS} scans"));
                            }
                            stream.reset();
                            continue 'eoi;
                        }
                        Err(_) => break 'eoi,
                    }
                }
                Marker::Rst(_) => self.handle_rst(&mut stream),
                _ => self.parse_marker_inner(marker)?,
            }
            match get_marker(&mut self.stream, &mut stream) {
                Ok(next) => marker = next,
                Err(_) => break 'eoi,
            }
        }
        self.finish(&block, pixels)
    }

    fn reset_prog_params(&mut self, stream: &mut BitStream) {
        stream.reset();
        self.components.iter_mut().for_each(|x| x.dc_pred = 0);
        self.todo = if self.restart_interval != 0 {
            self.restart_interval
        } else {
            usize::MAX
        };
    }

    fn parse_entropy_coded_data(
        &mut self,
        stream: &mut BitStream,
        buffer: &mut [Vec<i16>; MAX_COMPONENTS],
    ) -> Result<(), String> {
        self.reset_prog_params(stream);
        if usize::from(self.num_scans) > self.components.len() {
            return Err(format!(
                "{} components in a scan of a {}-component image",
                self.num_scans,
                self.components.len()
            ));
        }
        if self.num_scans == 1 {
            if self.spec_end != 0 && self.spec_start == 0 {
                return Err("a scan merges DC and AC".into());
            }
            let k = self.z_order[0];
            if k >= self.components.len() {
                return Err(format!("component {k} is missing"));
            }
            let component = &self.components[k];
            let mcu_width =
                (usize::from(self.width) * component.horizontal_sample).div_ceil(self.h_max * 8);
            let mcu_height =
                (usize::from(self.height) * component.vertical_sample).div_ceil(self.v_max * 8);
            for i in 0..mcu_height {
                for j in 0..mcu_width {
                    if self.spec_start != 0 && self.succ_high == 0 && stream.eob_run > 0 {
                        stream.eob_run -= 1;
                    } else {
                        let start = 64 * (j + i * (self.components[k].width_stride / 8));
                        let data: &mut [i16; 64] = buffer[k]
                            .get_mut(start..start + 64)
                            .and_then(|slice| slice.try_into().ok())
                            .ok_or("the coefficients are too short")?;
                        if self.spec_start == 0 {
                            let pos = self.components[k].dc_huff_table & (MAX_COMPONENTS - 1);
                            let dc_table = self.dc_huffman_tables[pos]
                                .as_ref()
                                .ok_or("a DC Huffman table is missing")?;
                            let dc_pred = &mut self.components[k].dc_pred;
                            if self.succ_high == 0 {
                                stream.decode_prog_dc_first(
                                    &mut self.stream,
                                    dc_table,
                                    &mut data[0],
                                    dc_pred,
                                )?;
                            } else {
                                stream.decode_prog_dc_refine(&mut self.stream, &mut data[0])?;
                            }
                        } else {
                            let pos = self.components[k].ac_huff_table;
                            let ac_table = self
                                .ac_huffman_tables
                                .get(pos)
                                .and_then(Option::as_ref)
                                .ok_or_else(|| format!("AC Huffman table {pos} is missing"))?;
                            if self.succ_high == 0 {
                                stream.decode_mcu_ac_first(&mut self.stream, ac_table, data)?;
                            } else {
                                stream.decode_mcu_ac_refine(&mut self.stream, ac_table, data)?;
                            }
                        }
                    }
                    self.todo = self.todo.wrapping_sub(1);
                    self.handle_rst_main(stream)?;
                }
            }
        } else {
            if self.spec_end != 0 {
                return Err("a scan merges DC and AC".into());
            }
            for k in 0..usize::from(self.num_scans) {
                let n = self.z_order[k];
                if n >= self.components.len() {
                    return Err(format!("component {n} is missing"));
                }
                let table = self.components[n].dc_huff_table;
                if !matches!(self.dc_huffman_tables.get(table), Some(Some(_))) {
                    return Err(format!("DC Huffman table {table} is missing"));
                }
            }
            for i in 0..self.mcu_y {
                for j in 0..self.mcu_x {
                    for k in 0..usize::from(self.num_scans) {
                        let n = self.z_order[k];
                        let component = &mut self.components[n];
                        let huff_table = self
                            .dc_huffman_tables
                            .get(component.dc_huff_table)
                            .and_then(Option::as_ref)
                            .ok_or("a DC Huffman table is missing")?;
                        for v_samp in 0..component.vertical_sample {
                            for h_samp in 0..component.horizontal_sample {
                                let x2 = j * component.horizontal_sample + h_samp;
                                let y2 = i * component.vertical_sample + v_samp;
                                let position = 64 * (x2 + y2 * component.width_stride / 8);
                                let data = buffer[n]
                                    .get_mut(position)
                                    .ok_or("the coefficients are too short")?;
                                if self.succ_high == 0 {
                                    stream.decode_prog_dc_first(
                                        &mut self.stream,
                                        huff_table,
                                        data,
                                        &mut component.dc_pred,
                                    )?;
                                } else {
                                    stream.decode_prog_dc_refine(&mut self.stream, data)?;
                                }
                            }
                        }
                    }
                    self.todo = self.todo.wrapping_sub(1);
                    self.handle_rst_main(stream)?;
                }
            }
        }
        Ok(())
    }
}
