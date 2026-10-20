use std::collections::HashMap;

use pfx_gpu::Gpu;
use wgpu::util::DeviceExt;

use crate::aux::Filter;
use crate::net::{Net, Source};
use crate::tile::{self, Plan};
use crate::wgsl::{self, ConvShape};
use crate::{Images, Job, MACS_PER_SUBMISSION, Model, Setup, Step, norm_scales};

const SLOT: u64 = 256;
const BLOCK_BYTES: u64 = 16;

fn tensor_bytes(blocks: u32, level: u32, plan: &Plan) -> u64 {
    u64::from(blocks)
        * u64::from(plan.tile_h >> level)
        * u64::from(plan.tile_w >> level)
        * BLOCK_BYTES
}

fn arena(net: &Net, plan: &Plan) -> (Vec<u64>, Vec<usize>) {
    let count = net.layers.len();
    let mut sizes = vec![tensor_bytes(net.input_channels.div_ceil(4), 0, plan)];
    let mut last = vec![0usize; count + 1];
    for (index, layer) in net.layers.iter().enumerate() {
        sizes.push(tensor_bytes(layer.out_blocks(), layer.out_level(), plan));
        for source in [Some(layer.src0), layer.src1].into_iter().flatten() {
            let tensor = match source {
                Source::Input => 0,
                Source::Layer(from) => from + 1,
            };
            last[tensor] = last[tensor].max(index + 1);
        }
    }
    last[count] = count + 1;
    let mut holders: Vec<usize> = Vec::new();
    let mut slots: Vec<u64> = Vec::new();
    let mut assigned = Vec::with_capacity(count + 1);
    for (tensor, &size) in sizes.iter().enumerate() {
        let free: Vec<usize> = (0..holders.len())
            .filter(|&slot| last[holders[slot]] < tensor)
            .collect();
        let slot = free
            .iter()
            .copied()
            .filter(|&slot| slots[slot] >= size)
            .min_by_key(|&slot| slots[slot])
            .or_else(|| free.iter().copied().max_by_key(|&slot| slots[slot]))
            .unwrap_or_else(|| {
                holders.push(tensor);
                slots.push(0);
                slots.len() - 1
            });
        holders[slot] = tensor;
        slots[slot] = slots[slot].max(size);
        assigned.push(slot);
    }
    (slots, assigned)
}

fn largest_tensor_per_pixel(net: &Net) -> u64 {
    std::iter::once(u64::from(net.input_channels.div_ceil(4)))
        .chain(
            net.layers
                .iter()
                .map(|layer| u64::from(layer.out_blocks()).div_ceil(1 << (2 * layer.out_level()))),
        )
        .max()
        .unwrap_or(1)
        * BLOCK_BYTES
}

struct Pipelines<'a> {
    gpu: &'a Gpu,
    list: Vec<wgpu::ComputePipeline>,
    built: HashMap<(String, &'static str), usize>,
}

impl Pipelines<'_> {
    fn get(&mut self, label: &str, source: String, entry: &'static str) -> usize {
        if let Some(&index) = self.built.get(&(source.clone(), entry)) {
            return index;
        }
        let module = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
            });
        self.list.push(
            self.gpu
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: None,
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
        );
        self.built.insert((source, entry), self.list.len() - 1);
        self.list.len() - 1
    }
}

struct Pass {
    input: usize,
    output: usize,
    layers: Vec<usize>,
}

pub(crate) fn build(gpu: &Gpu, models: &[Model], width: u32, height: u32) -> Result<Setup, String> {
    let limits = gpu.device.limits();
    let binding = limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size);
    let per_pixel = models
        .iter()
        .map(|model| largest_tensor_per_pixel(&model.net))
        .max()
        .unwrap_or(BLOCK_BYTES);
    let plan = tile::plan(
        height,
        width,
        tile::DEFAULT_MAX_TILE_PIXELS.min(binding / per_pixel),
    );
    let arenas: Vec<(Vec<u64>, Vec<usize>)> = models
        .iter()
        .map(|model| arena(&model.net, &plan))
        .collect();
    let mut sizes: Vec<u64> = Vec::new();
    for (slots, _) in &arenas {
        for (index, &size) in slots.iter().enumerate() {
            match sizes.get_mut(index) {
                Some(have) => *have = (*have).max(size),
                None => sizes.push(size),
            }
        }
    }
    if sizes.iter().any(|&size| size > binding) {
        return Err("the denoiser's tiles do not fit the device's storage buffers".into());
    }
    let stride = width.div_ceil(16) * 16;
    let image_bytes = u64::from(stride) * u64::from(height) * 16;
    if image_bytes > binding {
        return Err("the image does not fit the device's storage buffers".into());
    }
    let image = |label: &str| {
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: image_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    };
    let has = |filter: Filter| models.iter().any(|model| model.filter == filter);
    let images = Images {
        color: image("denoise color"),
        albedo: image("denoise albedo"),
        normal: image("denoise normal"),
        output: image("denoise output"),
        clean_albedo: has(Filter::Albedo).then(|| image("denoise clean albedo")),
        clean_normal: has(Filter::Normal).then(|| image("denoise clean normal")),
    };
    let tensors: Vec<wgpu::Buffer> = sizes
        .iter()
        .map(|&size| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("denoise tensor"),
                size: size.max(16),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        })
        .collect();
    let scale = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("denoise input scale"),
        size: 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let bins = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("denoise exposure bins"),
        size: u64::from(width.div_ceil(16)) * u64::from(height.div_ceil(16)) * 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let max_rows = plan.tile_h.div_ceil(wgsl::BLOCK_H);
    let mut row_bytes = vec![0u8; (u64::from(max_rows) * SLOT) as usize];
    for row in 0..max_rows {
        row_bytes[(u64::from(row) * SLOT) as usize..][..4].copy_from_slice(&row.to_le_bytes());
    }
    let rows = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("denoise rows"),
            contents: &row_bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let (norm, rcp_norm) = norm_scales();
    let flip = |v: [u32; 2]| [v[1] as i32, v[0] as i32];
    let mut jobs = vec![Job {
        image: [width, height],
        stride,
        ..Job::default()
    }];
    jobs.extend(plan.tiles.iter().map(|tile| Job {
        src: flip(tile.src),
        dst: flip(tile.dst),
        size: flip(tile.size),
        out_src: flip(tile.out_src),
        out_dst: flip(tile.out_dst),
        out_size: flip(tile.out_size),
        dims: [plan.tile_w, plan.tile_h],
        image: [width, height],
        stride,
        norm,
        rcp_norm,
        pad: 0,
    }));
    let mut job_bytes = vec![0u8; jobs.len() * SLOT as usize];
    for (index, job) in jobs.iter().enumerate() {
        job_bytes[index * SLOT as usize..][..size_of::<Job>()]
            .copy_from_slice(bytemuck::bytes_of(job));
    }
    let job_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("denoise jobs"),
            contents: &job_bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let job_slot = |index: usize| {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &job_buffer,
            offset: index as u64 * SLOT,
            size: wgpu::BufferSize::new(size_of::<Job>() as u64),
        })
    };
    let row_slot = |row: u32| {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &rows,
            offset: u64::from(row) * SLOT,
            size: wgpu::BufferSize::new(4),
        })
    };
    let process = wgsl::process();
    let mut pipelines = Pipelines {
        gpu,
        list: Vec::new(),
        built: HashMap::new(),
    };
    let exposure = has(Filter::Color).then(|| {
        [
            pipelines.get("denoise exposure bins", process.into(), "exposure_bins"),
            pipelines.get("denoise exposure reduce", process.into(), "exposure_reduce"),
        ]
    });
    let shapes: Vec<Vec<ConvShape>> = models
        .iter()
        .map(|model| {
            model
                .net
                .layers
                .iter()
                .map(|layer| ConvShape {
                    width: plan.tile_w >> layer.level,
                    height: plan.tile_h >> layer.level,
                })
                .collect()
        })
        .collect();
    let passes: Vec<Pass> = models
        .iter()
        .zip(&shapes)
        .map(|(model, shapes)| {
            let [input, output] = model.filter.entries();
            Pass {
                input: pipelines.get("denoise input", process.into(), input),
                output: pipelines.get("denoise output", process.into(), output),
                layers: model
                    .net
                    .layers
                    .iter()
                    .zip(shapes)
                    .map(|(layer, shape)| {
                        pipelines.get(&layer.name, wgsl::conv(layer, shape), "main")
                    })
                    .collect(),
            }
        })
        .collect();
    let pipelines = pipelines.list;
    let bind = |pipeline: usize, entries: &[(u32, wgpu::BindingResource)]| {
        let entries: Vec<wgpu::BindGroupEntry> = entries
            .iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: resource.clone(),
            })
            .collect();
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("denoise"),
            layout: &pipelines[pipeline].get_bind_group_layout(0),
            entries: &entries,
        })
    };
    let mut steps = Vec::new();
    if let Some([bins_pipeline, reduce_pipeline]) = exposure {
        steps.push(Step {
            bindings: bind(
                bins_pipeline,
                &[
                    (0, job_slot(0)),
                    (1, images.color.as_entire_binding()),
                    (8, bins.as_entire_binding()),
                ],
            ),
            pipeline: bins_pipeline,
            groups: [width.div_ceil(16), height.div_ceil(16), 1],
            macs: 0,
        });
        steps.push(Step {
            bindings: bind(
                reduce_pipeline,
                &[
                    (0, job_slot(0)),
                    (8, bins.as_entire_binding()),
                    (9, scale.as_entire_binding()),
                ],
            ),
            pipeline: reduce_pipeline,
            groups: [1, 1, 1],
            macs: 0,
        });
    }
    let albedo = images.clean_albedo.as_ref().unwrap_or(&images.albedo);
    let normal = images.clean_normal.as_ref().unwrap_or(&images.normal);
    let tile_groups = [plan.tile_w.div_ceil(8), plan.tile_h.div_ceil(8), 1];
    for (((model, (_, slots)), shapes), pass) in
        models.iter().zip(&arenas).zip(&shapes).zip(&passes)
    {
        let net = &model.net;
        let tensor = |index: usize| &tensors[slots[index]];
        let source = |source: Source| match source {
            Source::Input => tensor(0),
            Source::Layer(from) => tensor(from + 1),
        };
        let (from, to) = match model.filter {
            Filter::Albedo => (&images.albedo, albedo),
            Filter::Normal => (&images.normal, normal),
            Filter::Color => (&images.color, &images.output),
        };
        for index in 0..plan.tiles.len() {
            let input = match model.filter {
                Filter::Color => vec![
                    (0, job_slot(index + 1)),
                    (1, from.as_entire_binding()),
                    (2, albedo.as_entire_binding()),
                    (3, normal.as_entire_binding()),
                    (4, scale.as_entire_binding()),
                    (5, tensor(0).as_entire_binding()),
                ],
                Filter::Albedo | Filter::Normal => vec![
                    (0, job_slot(index + 1)),
                    (1, from.as_entire_binding()),
                    (5, tensor(0).as_entire_binding()),
                ],
            };
            steps.push(Step {
                bindings: bind(pass.input, &input),
                pipeline: pass.input,
                groups: tile_groups,
                macs: 0,
            });
            for (layer_index, ((layer, shape), &pipeline)) in
                net.layers.iter().zip(shapes).zip(&pass.layers).enumerate()
            {
                let groups = wgsl::groups(layer, shape);
                let row_macs = u64::from(groups[0])
                    * u64::from(wgsl::BLOCK_W * wgsl::BLOCK_H)
                    * layer.macs_per_pixel();
                let band =
                    (MACS_PER_SUBMISSION / row_macs.max(1)).clamp(1, u64::from(groups[1])) as u32;
                let mut row = 0;
                while row < groups[1] {
                    let count = band.min(groups[1] - row);
                    let mut entries = vec![
                        (0, row_slot(row)),
                        (1, source(layer.src0).as_entire_binding()),
                        (3, model.weights[layer_index].as_entire_binding()),
                        (4, model.biases[layer_index].as_entire_binding()),
                        (5, tensor(layer_index + 1).as_entire_binding()),
                    ];
                    if let Some(src1) = layer.src1 {
                        entries.push((2, source(src1).as_entire_binding()));
                    }
                    steps.push(Step {
                        bindings: bind(pipeline, &entries),
                        pipeline,
                        groups: [groups[0], count, groups[2]],
                        macs: row_macs * u64::from(count),
                    });
                    row += count;
                }
            }
            let output = match model.filter {
                Filter::Color => vec![
                    (0, job_slot(index + 1)),
                    (1, from.as_entire_binding()),
                    (4, scale.as_entire_binding()),
                    (6, tensor(net.layers.len()).as_entire_binding()),
                    (7, to.as_entire_binding()),
                ],
                Filter::Albedo | Filter::Normal => vec![
                    (0, job_slot(index + 1)),
                    (6, tensor(net.layers.len()).as_entire_binding()),
                    (7, to.as_entire_binding()),
                ],
            };
            steps.push(Step {
                bindings: bind(pass.output, &output),
                pipeline: pass.output,
                groups: tile_groups,
                macs: 0,
            });
        }
    }
    Ok(Setup {
        width,
        height,
        stride,
        plan,
        images,
        pipelines,
        steps,
    })
}
