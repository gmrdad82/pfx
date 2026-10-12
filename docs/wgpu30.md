# Moving to wgpu 30

pfx moved from wgpu 27.0.1 to wgpu 30.0.1, naga 30 and egui 0.36.2. An app that pins pfx and also uses wgpu or egui directly moves its own pins to the same versions and makes the changes below; an app that only calls pfx's crates sees the pfx changes at the end. The counts are the sites pfx itself had to change.

## Hardware ray queries

- `pfx_gpu` asks for `wgpu::Features::EXPERIMENTAL_RAY_QUERY` whenever the adapter offers it (Vulkan with `VK_KHR_ray_query`, DX12 with raytracing tier 1.1) and opens the device with `wgpu::ExperimentalFeatures::enabled()` in that case only. wgpu 30's Metal backend offers no ray queries, so Apple GPUs keep the software path.
- `pfx-trace` traces with ray queries over a hardware TLAS when the device has the feature, and with its own BVH otherwise; `Trace::hardware_rays()` says which. `PFX_TRACE_RT=0` forces the software path. Both paths draw the same random numbers, so the same seed gives the same picture within float differences at triangle edges.
- Analytic shapes (rounded boxes, cones, ellipsoids, capsules, smooth unions) are tested by the shader's exact loop on both paths. An AABB BLAS for them is behind `PFX_TRACE_RT_BOXES=1` and is not yet exact on RADV, which leaves some box candidates unreported. naga 30.0.1's `rayQueryGenerateIntersection` validity check reads the candidate's t instead of the committed one, so the trace never generates intersections; it keeps the nearest shape hit itself.
- The ray-query trace shader takes the driver several seconds to compile the first time (about 8 s on RADV); the driver's shader cache keeps it after that.

## wgpu 27 -> 30

- `PipelineLayoutDescriptor.bind_group_layouts` is `&[Option<&BindGroupLayout>]`: wrap every entry in `Some(..)` (an array of layouts: `&layouts.map(Some)`). 67 sites (live, physics, post).
- `PipelineLayoutDescriptor.push_constant_ranges` is gone, replaced by `immediate_size: u32`: `push_constant_ranges: &[]` -> `immediate_size: 0`. 67 sites (live, physics, post); no crate uses push constants.
- `RenderPipelineDescriptor.multiview` -> `multiview_mask: Option<NonZeroU32>` (`multiview: None` -> `multiview_mask: None`). 33 sites.
- `RenderPassDescriptor` gained `multiview_mask: Option<NonZeroU32>`: add `multiview_mask: None`. 82 sites.
- `Buffer(Slice)::get_mapped_range()` / `get_mapped_range_mut()` return `Result<_, MapRangeError>`: `.expect("a mapped range")`. 49 sites.
- `BufferViewMut` is write-only (no Deref/Index to `[u8]`): `view[a..b].copy_from_slice(x)` -> `view.slice(a..b).copy_from_slice(x)`; whole buffer `view.copy_from_slice(x)`; `.fill(0)` -> `.slice(..).fill(0)`; `bytemuck::cast_slice_mut(&mut view)` -> build the bytes first, then copy. Seen in physics (fluid.rs fill x2) and the sites the scripted pass and the first fixes covered.
- `DepthStencilState.depth_write_enabled` is `Option<bool>` and `depth_compare` is `Option<CompareFunction>`: wrap in `Some(..)`. 14 literals (live, post).
- `VertexState.buffers` is `&[Option<VertexBufferLayout>]`: wrap each layout in `Some(..)`; helpers taking `&[VertexBufferLayout]` take `&[Option<VertexBufferLayout>]`. 14 literals, 3 helper signatures (live).
- `SamplerDescriptor.mipmap_filter` is the new `wgpu::MipmapFilterMode` (`Nearest`/`Linear`), not `FilterMode`. 13 sites (live); a returned descriptor's `.mipmap_filter` compares against `MipmapFilterMode` too.
- `Instance::new` takes `InstanceDescriptor` by value and `InstanceDescriptor` has no `Default`: `wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() })`. 8 sites (gpu, live, physics, trace).
- `Instance::enumerate_adapters` is async: `pollster::block_on(instance.enumerate_adapters(backends))` or `.await`. 3 sites (gpu, live).
- `RequestAdapterOptions` gained `apply_limit_buckets: bool`: set `false` in literals that do not end in `..Default::default()`. 5 sites (gpu, live, physics).
- `SurfaceConfiguration` gained `color_space: wgpu::SurfaceColorSpace` (`Auto`). 1 site (gpu).
- `AdapterInfo` literals: `..wgpu::AdapterInfo::new(device_type, backend)` (the struct is non-exhaustive). gpu tests.
- `Limits.max_storage_buffer_binding_size` and `max_uniform_buffer_binding_size` are `u64` (clippy flags `u64::from(..)` around them, remove it). 4 sites (denoise, post, trace).
- `Limits.max_inter_stage_shader_components` is gone; `max_inter_stage_shader_variables` counts vec4 locations, so `n * 4 <= components` becomes `n <= variables`. 1 site (live test).
- `Surface::get_current_texture()` returns `wgpu::CurrentSurfaceTexture` (not `Result<SurfaceTexture, SurfaceError>`): `Success(frame) | Suboptimal(frame)` is the frame, `Lost | Outdated` reconfigure, `Timeout | Occluded` skip the frame, `Validation` is the error case. `wgpu::SurfaceError` is no longer matched here. 2 sites (editor-shell, game).
- `SurfaceTexture::present()` moved: `queue.present(frame)`. 2 sites (editor-shell, game).
- `Device::push_error_scope(filter)` returns an `ErrorScopeGuard`; `device.pop_error_scope()` is gone: `let scope = device.push_error_scope(..); ... pollster::block_on(scope.pop())`. 1 site (live test).

## egui / egui-wgpu / epaint 0.33 -> 0.36

- `Context::run` is gone; use `Context::run_ui(input, |ui: &mut Ui| ..)`. The root `Ui` covers the viewport with no margin or background; reach the context with `ui.ctx()`. 5 sites (live hud, editor-shell window and offscreen, tests).
- `SidePanel` and `TopBottomPanel` are replaced by `egui::Panel::{left,right,top,bottom}(id)`; `Panel::show(ui, ..)` and `CentralPanel::show(ui, ..)` take a `&mut Ui`, not a `&Context`. `exact_height` -> `exact_size`, `default_height`/`default_width` -> `default_size`. `Panel` is resizable by default on every side (the old top/bottom panels were not): add `.resizable(false)` to keep a fixed bar. `Area::show(ctx, ..)` is unchanged. About 25 sites (editor, editor-viewport, tests).
- pfx-editor-shell `App::ui` takes `&mut Ui` instead of `&Context` (so panels can show inside it), and `drive(ctx, size, script, |ui| ..)`; `Editor::ui` and `Session::ui` follow. Apps that implement `App` change `fn ui(&mut self, ctx: &Context)` to `fn ui(&mut self, ui: &mut Ui)` and call `ui.ctx()` for the context.
- `RawInput.modifiers` is gone: send `Event::ModifiersChanged(modifiers)` as the first event of the frame. 1 site (editor-shell offscreen).
- `Event::MouseWheel` gained `phase: TouchPhase` (`TouchPhase::Move` when unknown). 2 literals (editor-shell offscreen, editor input test).
- `InputState::raw_scroll_delta` is gone (only the smoothed `smooth_scroll_delta` remains): the unsmoothed per-frame sum is rebuilt from `input.events` (`pfx_editor_viewport::scrolled`, with `Context::options(|o| o.input_options)` for `line_scroll_speed` and the scroll modifiers). 2 sites (editor, editor-viewport).
- `Context::wants_keyboard_input` -> `egui_wants_keyboard_input`. 1 site (editor).
- `Context::style()` -> `global_style()` (`style_of(theme)` unchanged). 4 sites (editor-style, editor-shell test).
- `TexturesDelta.set` maps each `TextureId` to a `SmallVec<[ImageDelta; 1]>`: loop the deltas and call `update_texture` for each. 2 sites (live hud, editor-shell paint).
- `Id::new(source)` needs `Hash + Debug`: functions taking `impl Hash` for a salt take `impl Hash + Debug`. 2 sites (editor-toml Source::new, editor-viewport Viewport::open).
- `TextEdit::frame(bool)` takes a `Frame`: `.frame(false)` -> `.frame(egui::Frame::NONE)`. 1 site (editor-toml).
- `TextEditOutput.response` is an `AtomLayoutResponse` (derefs to `Response`, does not clone as one): use `output.response.response.clone()` to move a `Response`. 1 site (editor-toml).
- `TextEditState` cursor `CCursor.index` is a `CharIndex(usize)` newtype: read `.0`. 1 site (editor-toml panel).
- `LayoutSection.byte_range` is `Range<ByteIndex>` (`epaint::text::ByteIndex(usize)`): wrap with `ByteIndex(..)`. 7 sites (editor-toml job.rs and its tests).
- `Row.ends_with_newline` moved to `PlacedRow.ends_with_newline`: set it where the `PlacedRow` is built, and a cached `Arc<Row>` is no longer mutated. 3 sites (editor-toml job.rs).
- `ui.data_mut(|d| d.insert_temp(..))` and `d.remove::<T>(..)` now return values of different types, so a `match` using one in each arm needs both closures to end in `;`. 1 site (editor-style number widget).
- egui-wgpu `Renderer::update_texture` is per `ImageDelta` (see TexturesDelta). `RendererOptions`/`ScreenDescriptor` literals needed no change.

## pfx's own API

- `pfx_editor_shell::App::ui` takes `&mut egui::Ui` (listed with the egui changes above).
- `pfx_trace::gpu::SceneBuffers` has an `rt` field (the ray-query geometry) and `pfx_trace::gpu::shader_rt(lit)` returns the ray-query variant of the trace shader.
