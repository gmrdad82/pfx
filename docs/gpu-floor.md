# The live renderer's GPU floor

The least GPU the live renderer (`crates/live`) runs on, and how it says no to one below it. An app's live scene needs this GPU; its CLI and TUI need none.

The floor lives in code as `pfx_gpu::live_floor()` (`crates/gpu/src/floor.rs`). This page explains each number and the pass behind it. When a pass needs more, change the floor in the same commit and update this page.

## The refusal

The live renderer checks the device before it creates anything, and refuses with a typed reason instead of a wgpu validation panic later.

- `Renderer::new` and `Renderer::new_with_output_format` return `Result<Renderer, RendererError>`.
  - `RendererError::Refused(GpuRefusal)` means the GPU is below the floor.
  - `RendererError::Setup(String)` covers every other failure (an empty viewport, an unsupported output format).
  - `String: From<RendererError>`, so callers that use `?` into `String` keep compiling.
- `Gpu::check_floor()` runs the same check on any opened `Gpu`, including one an app built from its own device.
- `Gpu::headless_checked()` and `Gpu::window_checked(window)` check the adapter before asking for a device. They are the only openers that can report `NoAdapter`.
- `Gpu::headless()` and `Gpu::window()` keep their `String` errors and refuse nothing. Bake, pfx and the tests use them.

A `GpuRefusal` has two fields:
- `shortfall: GpuShortfall` is the first thing missing, in this order: feature, downlevel capability, limit, format.
- `adapter: Option<wgpu::AdapterInfo>` holds the adapter's name, driver, driver info and backend. It is `None` only when there is no adapter at all. The refusal's text names the backend through `backend_name`: Vulkan, DX12 or Metal.

`GpuShortfall` has these variants:

| Variant | Meaning |
|---|---|
| `NoAdapter` | No Vulkan, DX12 or Metal adapter at all. |
| `Device(String)` | The driver refused a device although the adapter met the floor. The text is the driver's, for logs. |
| `Surface(String)` | The window can't be presented to. This comes only from `window_checked`. |
| `Feature(&'static str)` | A wgpu feature or downlevel capability is missing, named as wgpu spells it, e.g. `CUBE_ARRAY_TEXTURES`. |
| `Limit { name, needed, has }` | A limit is below the floor, e.g. `max_sampled_textures_per_shader_stage`, needed 18, has 16. |
| `Format { format, needs }` | A texture format lacks a usage the renderer needs; `needs` names what it is for. |

`Display` on both types is for logs only. An app turns the variant into its own words.

Timestamp queries are never part of the floor. Without `TIMESTAMP_QUERY` the renderer runs with profiling off: `Gpu::profiling()` is false and every pass timing is empty.

## Features

| Feature | Floor | Used by |
|---|---|---|
| `TIMESTAMP_QUERY` | optional | `GpuProfiler` pass timings across every pass; `pace` frame timing. Absent means profiling off. |
| `TIMESTAMP_QUERY_INSIDE_ENCODERS` | optional | `GpuProfiler` marks between passes. |
| `FLOAT32_FILTERABLE` | optional | Nothing in the live renderer: every R32Float and Rgba32Float binding is `filterable: false` and read with `textureLoad`. Glass declares its fluid and ripple inputs filterable; callers pass R16Float or Rgba16Float there, so an R32Float input would need this feature. The tracer requires it (below). |

The floor requires no wgpu feature. No pass uses push constants, binding arrays, multisampling, indirect draws, occlusion queries, polygon modes, conservative raster, unclipped depth, subgroups or read-write storage textures.

## Downlevel capabilities

These are WebGPU core, but wgpu reports them per adapter. Every desktop Vulkan driver has them. The check names a missing one as `GpuShortfall::Feature`.

| Capability | Pass that needs it |
|---|---|
| `COMPUTE_SHADERS` | Post chain, depth of field, TAA resolve, reflections, linear depth, exposure, room haze field |
| `VERTEX_STORAGE` | Scene instances, deformers and skins in every mesh pass, shadow casters, creatures, particles, canopy cards, glass, impostors, deformed and lit text |
| `FRAGMENT_STORAGE` | Opaque shading (materials, probes, map materials), glass, impostors, lit text |
| `CUBE_ARRAY_TEXTURES` | Opaque shading and lit text: local reflection probes (`texture_cube_array`, lighting binding 14) |
| `COMPARISON_SAMPLERS` | Sun shadow cascades and local light faces in opaque shading, particles, volumes, lit text |
| `DEPTH_TEXTURE_AND_BUFFER_COPIES` | Shadow atlas and light faces copied to their static backups |
| `NON_POWER_OF_TWO_MIPMAPPED_TEXTURES` | Scene colour mips and the reflection pyramid at viewport size |
| `FULL_DRAW_INDEX_UINT32` | Uint32 index buffers for meshes past 16M vertices |
| `BUFFER_BINDINGS_NOT_16_BYTE_ALIGNED` | Probe records of 60 bytes (the fallback probe binds 60 bytes) |
| `WEBGPU_TEXTURE_FORMAT_SUPPORT` | The format guarantees below |
| `SHADER_F16_IN_F32` | Opaque shading and lit text: probe records unpack with `unpack2x16float` |

Anisotropic filtering (`anisotropy_clamp` 8 on material maps, content and text) is not required. wgpu quietly clamps it to 1 where the adapter lacks it, so it costs sharpness, not correctness.

## Limits

The floor is `wgpu::Limits::default()` (the WebGPU guaranteed limits) with two raises. `Gpu`'s openers request both raises whenever the adapter offers them.

### Raised above the default

| Limit | Default | Floor | Pass that needs it |
|---|---|---|---|
| `max_sampled_textures_per_shader_stage` | 16 | **18** | Tree cards ("live card depth", "live card shading") and lit text ("lit surface text", "lit deformed text"). Each binds, in the fragment stage, 3 shadow textures (sun cascades, transmission, light faces), the 14 of group 3 (`map_base`, `map_normal`, `map_roughness`, `map_metal`, `canopy_cookie`, `sky_cube`, `local_probes`, `ssr_history`, `content0` to `content3`, `contact_field`, `ssr_guide`) and 1 of its own (the card cookie or the glyph atlas). The main frame layout (prepass and opaque) sits at 17. |
| `max_texture_dimension_2d` | 8192 | **16384** | Text and icon atlases grow by powers of two to 16384 rows before `pfx_text` refuses with `AtlasTooLarge`. Sky HDRs and contact fields are app-sized and not checked against the device either. The viewport itself is 3840×2160 at 4K. |

The count passed 16 with the contact field (9f10b83) and reached 18 with the SSR guide at binding 26. wgpu 27 doesn't notice: it checks each bind group against the per-stage limits on its own, taking the largest group (14 here), while WebGPU and Vulkan count the sum over the pipeline layout. `Gpu`'s openers used to ask for 16, and RADV reports a driver limit of 8,388,606, so the tree test passes there. On a driver that reports 16 these pipelines would break the Vulkan limit without any wgpu error. The openers now ask for `min(adapter, 18)`, and the check refuses an adapter below 18.

### How the floor stays true

`crates/live/tests/binding_floor.rs` runs in the gate without a GPU:
- It sums each multi-group pipeline layout's bindings per stage, over the same entry functions the renderer builds its groups from (`frame::lighting_entries()`, `canopy::cookie_entries()`, `glass::compose_entries()` and the rest), using `pfx_gpu::stage_bindings` and `layout_shortfall`.
- It fails when any layout needs more than `live_floor()` in any per-stage limit or in bind groups.
- It fails when `SAMPLED_TEXTURES_PER_STAGE` isn't exactly the largest sum, so the floor moves with the layouts in both directions.
- It scans `crates/live/src` for every pipeline layout with more than one bind group and fails when one is missing from the test's list.

A new texture in group 3 therefore turns the gate red until the floor rises with it. Single-group layouts need no list: wgpu checks those against the device's limits, which the openers set from the floor.

### At the default, with the value in use

| Limit | Default (floor) | Most in use | Pass |
|---|---|---|---|
| `max_storage_textures_per_shader_stage` | 4 | **4** | TAA resolve: colour Rgba16Float, depth R32Float, id R32Uint and normal Rgba8Snorm history, all write-only |
| `max_bind_groups` | 4 | **4** | Opaque, depth/velocity, tree cards and lit text: scene, shadow, deformers or cookie or text, lighting |
| `max_compute_invocations_per_workgroup` | 256 | **256** | Exposure reduction (256×1); depth of field tiles (16×16) |
| `max_compute_workgroup_size_x` | 256 | 256 | Exposure reduction |
| `max_compute_workgroup_size_y` | 256 | 16 | Depth of field tiles |
| `max_compute_workgroup_size_z` | 64 | 4 | Room haze field (4×4×4) |
| `max_compute_workgroup_storage_size` | 16384 | 1024 | Exposure reduction (`array<f32, 256>`) |
| `max_compute_workgroups_per_dimension` | 65535 | 480 | Post chain at 3840 wide (8×8 groups) |
| `max_storage_buffers_per_shader_stage` | 8 | 6 | Shadow casters, vertex: instances, sway, skins, the deformers and their plan, light tints. Opaque shading, fragment: 5 (instances, materials, probe anchors, map materials, light tints) |
| `max_uniform_buffers_per_shader_stage` | 12 | **11** | Tree cards and lit text, fragment: 7 in group 3, 2 in the shadow group, 1 scene, 1 their own. One more uniform in group 3 and this limit needs raising too; the test above catches it. |
| `max_samplers_per_shader_stage` | 16 | 7 | Tree cards and lit text, fragment (5 filtering, 2 comparison) |
| `max_dynamic_uniform_buffers_per_pipeline_layout` | 8 | 1 | Shadow cascades (`CASTER_BYTES` 80 per caster slot) |
| `max_dynamic_storage_buffers_per_pipeline_layout` | 4 | 0 | none |
| `max_bindings_per_bind_group` | 1000 | 26 entries, highest index 25 | Lighting group |
| `max_color_attachments` | 8 | 2 | Prepass, opaque, effects volume, glass |
| `max_color_attachment_bytes_per_sample` | 32 | 16 | Opaque (2 × Rgba16Float); shadow transmission (Rgba32Uint) |
| `max_inter_stage_shader_components` | 60 | 32 + `front_facing` | Mesh vertex → opaque fragment (11 locations) |
| `max_vertex_buffers` | 8 | 2 | Sun shadow depth (positions + instances) |
| `max_vertex_attributes` | 16 | 6 | Text instances |
| `max_vertex_buffer_array_stride` | 2048 | 96 | Text instances |
| `max_texture_dimension_3d` | 2048 | 256 | Room haze field (256³ Rgba8Unorm, clamped in code) |
| `max_texture_array_layers` | 256 | 27, plus app-sized | The transmission map (group 1 binding 4): 3 cascades, then the light faces glass casts into as tiles, 24 layers at worst where a face's tile fills a layer (one extra layer for two lights at the default sizes); local light faces (4 lights × 6). Glass's light tiles add no binding. Material map arrays check the device limit; impostor radiance takes one layer per anchor. |
| `max_uniform_buffer_binding_size` | 64 KiB | 1808 B | Local lights |
| `max_storage_buffer_binding_size` | 128 MiB | grows with the scene | See buffer sizes |
| `max_buffer_size` | 256 MiB | 63–66 MiB at 4K | TAA colour readback, geometry readback (8-byte normals) |
| `min_uniform_buffer_offset_alignment` | 256 | 256 | Shadow cascades and light slots: offsets use the device's alignment |

### Buffer sizes

The openers ask for the adapter's own `max_buffer_size` and `max_storage_buffer_binding_size`. The floor keeps the defaults (256 MiB, 128 MiB) because no fixed buffer comes near them. These buffers grow with the scene and have no cap of their own:

| Buffer | Bytes per item | Passes 128 MiB at |
|---|---|---|
| live instances | 304 per instance, capacity rounded up to a power of two | 262,145 instances |
| live materials / map materials | 240 / 192 per material | about 524k / 700k materials |
| live sway | 32 per swaying vertex | 4.2M vertices |
| live skins | 32 per skinned vertex, then 128 per joint for each posed instance (current and previous palette) | 4.2M skinned vertices |
| probe anchors | 60 (v1) or 80 (v2) per probe | 2.2M / 1.7M probes |
| particles, creatures, impostor instances, text instances | 64, 80, 32, 96 | 2.1M, 1.7M, 4.2M, 1.4M |
| post depth | 4 per pixel | 33.5M pixels (above 8K) |
| TAA readback, geometry readback | 8 per pixel, rows padded to 256 | 256 MiB `max_buffer_size` at 8192² |

A desk scene is orders of magnitude below these.

## Formats

All of these are in WebGPU's guaranteed set. The check reads them from the device (the guaranteed set, or the adapter's own when `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` is on) so a non-conforming adapter is refused, not crashed into.

| Format | Usages | Flags | Used for |
|---|---|---|---|
| Rgba16Float | render, sampled, storage, copy | filterable, blendable, storage write | Scene colour and mips, TAA colour history, reflections, post chain and depth of field, particles and text (premultiplied blend), effects |
| R32Float | render, sampled, storage | storage write | Linear depth, TAA depth history, effects low-res depth target |
| R32Uint | render, sampled, storage | storage write | Object ids, TAA id history, glass ids |
| Rgba8Snorm | sampled, storage | storage write | TAA normal history |
| Rgba8Unorm | sampled, storage | filterable, storage write | Room haze field (3D storage), normal/roughness/metal maps, tree alpha atlas |
| Rgba32Uint | render, sampled | — | Shadow transmission map |
| Rg16Float | render, sampled | — | Velocity |
| R16Float | render, sampled | filterable | Contact field |
| R8Unorm | render, sampled | filterable, blendable | Reactive mask, canopy cookie (blended), text atlas |
| Rgba8UnormSrgb | render, sampled | filterable | Base maps, content slots, the sRGB output |
| Rgba32Float | sampled | — | Impostor geometry (unfiltered) |
| Depth32Float | render, sampled, copy | — | Scene depth, shadow atlas, light faces, glass depth |

Bgra8UnormSrgb is a render target only when the caller asks for it as the output format; the window surface chooses it.

## Per pass

Per-stage counts are summed over the pipeline layout's bind groups, the way WebGPU and Vulkan count them. wgpu 27 checks each group on its own.

| Pass | Where | Groups | Per stage | Targets | Compute |
|---|---|---|---|---|---|
| Depth and velocity prepass | frame.rs | 4 | V: 1 uniform, 5 storage. F: 10 uniform, 5 storage, 17 textures, 6 samplers. C: 2 storage (deformers) | Rg16Float + R32Uint, Depth32Float | — |
| Opaque PBR (± local lights) | frame.rs | 4 | as prepass | 2 × Rgba16Float, Depth32Float | — |
| Shadow casters, transmissive casters | frame.rs, lights.rs | 3 | V: 2 uniform (1 dynamic), 6 storage | depth only; Rgba32Uint | — |
| Sun shadow depth, creature shadow depth | shadow.rs | 1–2 | V: up to 2 uniform (1 dynamic), 2 storage | Depth32Float | — |
| Tree cards (depth, shading) | frame.rs, canopy.rs | 4 | V: 2 uniform, 3 storage. F: 11 uniform, 5 storage, **18 textures**, 7 samplers | as prepass / opaque | — |
| Canopy cookie | canopy.rs | 1 | V: 1 uniform, 1 storage. F: 1 texture, 1 sampler | R8Unorm, blended | — |
| Sky | sky.rs | 1 | F: 1 uniform, 1 texture, 1 sampler | Rgba16Float | — |
| Background motion, scene colour mips, output blit | renderer.rs | 1 | F: ≤ 2 textures | Rg16Float; Rgba16Float; sRGB output | — |
| Linear depth | renderer.rs | 1 | C: 1 texture, 1 storage texture (R32Float) | — | 8×8 |
| Exposure | renderer.rs | 1 | C: 1 texture, 1 storage buffer | — | 256, 1 KiB shared |
| TAA resolve | taa.rs | 1 | C: 1 uniform, 10 textures, **4 storage textures** | — | 8×8 |
| Reflections (depth pyramid, trace) | reflect.rs | 1 | C: up to 7 textures, 1 storage texture | — | 8×8 |
| Content mips | maps.rs | 1 | F: 1 texture, 1 sampler | Rgba8UnormSrgb or Rgba16Float | — |
| Particles, creatures, volume, upsample, haze blur | effects.rs | 1 | V: ≤ 2 storage. F: ≤ 3 textures, 1 sampler | Rgba16Float; volume adds R32Float | — |
| Room haze field | effects.rs | 1 | C: 1 uniform, 1 storage texture (3D Rgba8Unorm) | — | 4×4×4 |
| Glass front, back, compose | glass.rs | 1–3 | V: 2 textures. F: up to 9 textures, 1 sampler | Rgba16Float + R32Uint or R8Unorm, Depth32Float | — |
| Impostors, impostor shadows | impostor.rs | 1 | F: 2 textures (one D2Array), 1 sampler | Rgba16Float, Depth32Float | — |
| Text: surface, overlay, deformed | text.rs | 1–3 | V: ≤ 2 storage. F: 1 texture, 1 sampler | Rgba16Float, blended | — |
| Lit text | text.rs | 4 | V: 2 uniform, 5 storage. F: 11 uniform, 5 storage, **18 textures**, 7 samplers | Rgba16Float, blended | — |
| Post chain (every style) | post/gpu.rs | 1 | C: 1 uniform, ≤ 2 textures, 1 sampler, 1 storage buffer, 1 storage texture | — | 8×8 |
| Post depth conversion | post/gpu.rs | 1 | C: 1 depth texture, 1 storage buffer | — | 8×8 |
| Depth of field (tiles, dilate, gather) | post/dof.rs | 1 | C: ≤ 3 textures, 1 storage texture | — | 16×16, 8×8 |
| Flat pass: colour, ids, lit colour and ids, layer clear and composite | flat/pass.rs | 1–2 | V: 1 uniform. F: 1 uniform, ≤ 4 textures (icon fields, environment, gradient stops, sprite atlas; the group layer in its own pass), 3 samplers | Rgba16Float premultiplied blend; R32Uint | — |
| Flat text, flat finish | flat/pass.rs | 1 | V: 1 dynamic uniform. F: 1 texture, ≤ 1 sampler | Rgba16Float or the output format | — |

The flat pass (`docs/flat.md`) needs less than this floor: every flat pipeline fits `wgpu::Limits::default()` with no optional feature and no downlevel flag beyond core WebGPU, so a phone GPU can draw a flat scene even where the 3D passes would be refused. It has no storage buffers or textures, and its instances are a vertex buffer: 15 attributes at a 240-byte stride, and 60 inter-stage components (the default limit is 60: the per-corner radii took the last four). `flat::tests::every_flat_layout_fits_webgpu_defaults` checks its layouts in the gate. `flat::tests::the_flat_pipelines_run_on_a_device_at_webgpu_defaults` opens a device at exactly `Limits::default()` and draws through every flat pipeline. Its two-group layer composite is listed in `binding_floor.rs` with the rest.

### Outside the live renderer

An app that simulates drives the physics crate's GPU passes itself. They are not part of `Renderer` and not in the floor, but they fit it:
- SPH fluid: 7 storage buffers per compute stage (default 8), 256-wide workgroups, 1 KiB shared memory, fixed buffers up to 8 MiB at 262,144 particles.
- Fluid fields, ripple and caustics: 1768×992 Rgba16Float storage targets, 8×8 workgroups.

## The tracer's floor

The offline path tracer (`crates/trace`) binds more than the live floor allows, so it has a floor of its own: `pfx_gpu::trace_floor()`, next to `live_floor()` in `crates/gpu/src/floor.rs`. It is `wgpu::Limits::default()` with one raise, one required feature, one downlevel capability and two format needs. It holds what every trace needs, whatever its size; what grows with the image or the scene is the build's own check, below.

### The refusal

- `Trace::new` and `Trace::new_detailed` (and `Staged::trace`) check `trace_floor()` on the `Gpu` before they build anything, and return `Result<Trace, TraceError>`.
  - `TraceError::Refused(GpuRefusal)` carries the same `GpuShortfall` and adapter as the live renderer's refusal.
  - `TraceError::Setup(String)` covers every other failure (no materials, an invalid camera, lens or detail).
  - `String: From<TraceError>`, so callers that use `?` into `String` keep compiling, as with `RendererError`.
- After the floor, the build checks this trace's own sizes against the device and refuses with a `GpuShortfall::Limit` before allocating: the image side against `max_texture_dimension_2d`, and the accumulation buffer (48 bytes per pixel, `TRACE_BYTES_PER_PIXEL`) and every scene buffer against `max_storage_buffer_binding_size` and `max_buffer_size`. A device at the default 128 MiB storage range traces images up to 2,796,202 pixels (1920×1080 fits, at 99.5 MB) and refuses a 3840×2160 still by name (`max_storage_buffer_binding_size`, 398,131,200 needed), not crashed into.
- `gpu.check(&trace_floor())` runs the check on any opened `Gpu`.
- `Gpu`'s openers request what both floors need wherever the adapter offers it: the larger of the two floors' storage buffers (at least 16), sampled and storage textures, and the adapter's own buffer sizes. `FLOAT32_FILTERABLE` is requested whenever the adapter has it, as before.

### What it holds

| Need | Default | Floor | Why |
|---|---|---|---|
| `max_storage_buffers_per_shader_stage` | 8 | **15** | `main` binds 10 storage buffers in group 0 (triangles, nodes, shapes, materials, UVs, content texels, accumulation, content info, instance surfaces, content layers) and 2 in group 1 (the sky CDF's rows and columns): 12 unlit. A scene with emitters or local lights adds bindings 14–16 (emitters, emitter CDF, lights): 15 lit. |
| `max_storage_buffer_binding_size`, `max_buffer_size` | 128 MiB, 256 MiB | default | The accumulation buffer is 48 bytes a pixel (398,131,200 B at 3840×2160) and the scene buffers grow with the scene. Both are checked per build against the device, not set by the floor, so a small trace runs on a device at the defaults. `Gpu`'s openers ask for the adapter's own sizes. |
| `max_storage_textures_per_shader_stage` | 4 | 3 in use | Colour, albedo and normal, Rgba32Float, write-only |
| `max_uniform_buffers_per_shader_stage` | 12 | 3 in use | The frame, the environment and the sky CDF |
| `max_sampled_textures_per_shader_stage`, `max_samplers_per_shader_stage` | 16 | 1 each | The environment and its linear sampler |
| `max_bind_groups` | 4 | 2 | The scene, the environment |
| `FLOAT32_FILTERABLE` | — | required | The environment is an Rgba32Float texture sampled with a linear sampler; wgpu derives the tracer's layout from the shader and marks a texture read through a sampler as filterable. |
| `COMPUTE_SHADERS` | — | required | The whole tracer is one compute pass, 8×8 workgroups |
| Rgba32Float: storage, copy source | — | storage write | The traced colour, albedo and normal |
| Rgba32Float: sampled, copy destination | — | filterable | The environment |

The bake's probe batch shares `trace.wgsl`'s scene functions behind its own entry point; this floor and its test cover the tracer's `main` only.

Lavapipe's 128 MiB storage buffer range meets this floor, so lavapipe traces images up to about 2.8 megapixels and refuses larger ones by name.

### How it stays true

`crates/trace/tests/binding_floor.rs` runs in the gate without a GPU. The tracer's pipeline layout is derived by wgpu from the shader, so the test does the same: it parses the lit and unlit shaders (`pfx_trace::gpu::shader(lit)`), validates them with naga, and keeps the globals the compute entry `main` actually uses, the way wgpu builds its automatic layout. Then:
- each layout, summed per stage with `stage_bindings`, fits `trace_floor()` (`layout_shortfall` is `None`);
- the unlit layout binds 12 storage buffers and the lit one 15, and `TRACE_STORAGE_BUFFERS_PER_STAGE` is exactly the larger, so the floor moves with the shader in both directions;
- a floor one storage buffer short, or with two storage textures, fails each layout by name;
- the live floor fails the lit layout (15 needed, 8 offered), which is why the tracer has its own;
- every storage texture `main` writes is Rgba32Float and write-only, a sampled float texture is read through a sampler, and the floor holds both format needs and `FLOAT32_FILTERABLE`;
- the floor's buffer limits are the defaults, because image-sized buffers are the build's check.

`crates/gpu/src/floor.rs` checks the floor itself (an adapter at it passes, each missing limit, feature, capability and format is named), and `crates/gpu/src/lib.rs` checks that the openers' limits meet it wherever the adapter offers it. On the GPU, `this_machine_meets_the_live_floor` also checks `trace_floor()`, `a_device_below_the_trace_floor_is_refused` opens a device at the default limits and gets `TraceError::Refused` naming `max_storage_buffers_per_shader_stage`, and `default_buffer_limits_trace_a_small_image_and_refuse_a_4k_one` opens one at the floor (default buffer limits, 15 storage buffers), traces a 64×64 image and gets `max_storage_buffer_binding_size` for a 3840×2160 one.

## Memory

Not a limit wgpu checks, but a floor all the same. These figures are worked out from the formats and sizes above, not measured. At the default qualities a 4K frame holds:
- the sun shadow atlas and its static backup: 2 × 4096² × 3 Depth32Float = 384 MiB;
- shadow transmission: 60 MiB;
- local light faces: up to 192 MiB;
- the viewport targets, TAA histories, reflections, glass and post scratch: about 147 bytes per pixel, about 1.2 GiB at 3840×2160.

That is about 1.8 GiB before meshes, maps, the sky and probes. The renderer has no memory budget and the floor does not check VRAM: on a GPU with less, an allocation fails inside wgpu instead of being refused.

## Drivers

| Driver | GPU | State |
|---|---|---|
| RADV (Mesa 26.2.2) | AMD Radeon RX 9060 XT (RDNA 4) | Verified 2026-10-03 on the merged branch: the floor check, the `gpu` and `live` library GPU tests and the tree example |
| lavapipe (Mesa 26.2.2, LLVM 22.1.8) | software Vulkan on the CPU | Verified 2026-10-03 for correctness, below; four 4K time budgets fail, as expected on the CPU |
| Intel ANV | — | **Unverified**: never run |
| NVIDIA (proprietary or NVK) | — | **Unverified**: never run |
| Metal | — | **Unverified**: the openers allow it, nothing has run on it |
| DX12 | — | **Unverified** on a device. Resource binding tiers 2 and 3 meet the numeric floors; see below. Tier 1 is not an adapter wgpu 27 exposes |

Lavapipe proves correctness on a second driver, not speed.

### Running on lavapipe

Point the Vulkan loader at lavapipe alone; the openers then fall back to it as the only adapter:

```
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json <test binary> --ignored --test-threads=1
```

Lavapipe reports `llvmpipe (LLVM 22.1.8, 256 bits)`, meets the floor and has timestamps, so profiling stays on. Its limits sit well above the floor where it matters: 16384 for 2D textures, 1,015,808 sampled textures per stage, a 128 MiB storage buffer range, 32 KiB of shared memory.

| Run | RADV | lavapipe |
|---|---|---|
| `pfx-gpu` GPU tests, including the floor check | 9 passed | 9 passed |
| tree example GPU test (cards at 18 textures) | 1 passed | 1 passed |
| `pfx-live` GPU tests (`--skip gpu_bench_4k`) | 73 passed | 69 passed, 4 failed on time budgets only (1085 s) |

The four lavapipe failures are GPU-time budgets meant for real hardware, missed on the CPU:

| Test | Budget | lavapipe |
|---|---|---|
| `glass::tests::twenty_glass_slabs_at_4k` | median 1.0 ms | 24.0 ms |
| `impostor::tests::rounded_box_and_chrome_sphere_at_4k` | 0.5 ms | 12.4 ms; its image error check passed first |
| `text::tests::two_thousand_glyphs_and_fifty_icons_at_4k` | 0.3 ms | 5.1 ms |
| `text::tests::two_thousand_lit_glyphs_at_4k_stay_under_budget` | 0.2 ms | 4.2 ms |

No test failed on an image, a readback or a validation error.

## Windows (DX12)

On Windows the openers take a Vulkan adapter first and use DX12 only when no Vulkan adapter is found. Elsewhere they request Vulkan and Metal and take the first discrete adapter, as before. A refusal names which of those the adapter is.

wgpu 27.0.1 (wgpu-hal 27.0.4, `dx12/adapter.rs`) does not expose a resource binding tier 1 adapter: the backend returns none, because it needs bindless samplers. Tiers 2 and 3 are the adapters a floor can see. Every one of those reports `DownlevelFlags::all()` (shader model 5, so the live floor's capabilities pass, including `SHADER_F16_IN_F32`) and `FLOAT32_FILTERABLE`. Formats stay a check on the device. The numbers below are the limits that file reports.

| Limit | Flat pass (`Limits::default()`) | Live floor | Tracer | Tier 2 | Tier 3 | Tier 1, not exposed |
|---|---|---|---|---|---|---|
| Sampled textures per stage | 16 | **18** | 1 | 1,000,000, or half of that when the device supports ray tracing | 1,048,576, or half with ray tracing | 128, or 64 with ray tracing |
| Storage buffers per stage | 8 | 8 | **15** | 16 | 262,144 | 2 at feature level 11_0, 16 above it |
| Storage textures per stage | 4 | 4 | 3 in use | 16 | 262,144 | same as storage buffers |
| 2D texture side | 8192 | **16384** | the build's own check | 16384 | 16384 | 16384 |
| Samplers per stage | 16 | 7 in use | 1 | 2048 | 2048 | 16 |

Tier 2 passes the flat pass, the live floor and the tracer. The tracer's 15 storage buffers sit one under tier 2's 16, and the live floor's 18 sampled textures sit under tier 2's heap even with ray tracing (500,000). Tier 3 passes all three with the storage count at 262,144.

Tier 1 passes nothing through wgpu, because the adapter is not exposed. The unreached numbers at feature level 11_0 miss `Limits::default()` (2 storage buffers against 8, 2 storage textures against 4), so they miss the flat pass, the live floor and the tracer. Above feature level 11_0 those unreached numbers would meet all three floors, and wgpu still returns no adapter.

The flat pass needs only `wgpu::Limits::default()`. It asks for none of the live floor's two raises and none of the tracer's storage-buffer raise.
