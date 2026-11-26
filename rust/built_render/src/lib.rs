//! A resident Metal scene: compute compaction precedes native indirect draws.
mod timing;
mod pick_tile;
mod captured_frame;
use captured_frame::CapturedFrame;
mod window_timers;
mod window_job;
mod change_callbacks;
mod window_document;
mod close_documents;
mod component_replacement;
mod component_rows;
mod context_pick;
mod context_placements;
mod document_input;
mod document_chunks;
mod document_records;
mod document_state;
mod document_environment;
mod document_terrain;
mod document_cache;
mod document_grid;
mod document_edits;
mod document_targets;
mod document_hlr;
mod document_placements;
mod document_annotations;
mod document_planes;
mod document_host;
mod document_bindings;
mod document_registration;
mod document_camera;
mod section_display;
mod inspection_edits;
mod context_edits;
mod native_view_actions;
mod plugin_workspace;
mod link_watch;
mod context_tools;
mod image_edits;
mod component_files;
mod dwg_process;
mod edge_commands;
mod face_commands;
mod file_converters;
mod file_import;
mod file_placement;
mod linked_files;
mod model_edit;
mod profile_commands;
mod recovery_commands;
mod model_visibility;
mod model_clipboard;
mod group_edit;
mod tree_grouping;
mod model_panels;
mod document_starter;
mod scene_navigation;
mod window_services;
mod background_autosave;
mod panic_log;
mod overlay;
mod commands;
pub mod model_menu;
pub mod desktop_menu_model;
#[cfg(any(target_os="linux", windows))]
pub mod desktop_menu;
pub mod extension_menu;
mod extension_setup;
mod extension_api;
mod plugin_panels;
mod status_hints;
mod command_metadata;
mod model_views;
mod pane_dividers;
/// The one verb source: kinds, menu rows, inspector verbs, dispatch.
mod verbs;
mod viewport_menu;
mod shell_commands;
mod keyboard_bindings;
mod report_sheets;
mod inspector_host;
mod shell_events;
mod deliveries_host;
mod sheet_runtime;
mod settings_rows;
mod settings_host;
mod library_host;
mod start_host;
mod forms_host;
mod dialogs_host;
mod window_commands;
mod shell_files;
mod shell_host;
mod shell_window;
mod shell;
mod shell_runtime;
pub mod platform_paths;
pub mod ui;
mod host_context;
mod inspector;
mod inspector_geometry;
mod glass;
mod axes;
mod sections;
mod preview;
mod preview_texture;
mod geometry_preview;
mod tool_feedback;
mod transform_marks;
mod group_feedback;
mod tool_instances;
mod lines;
mod references;
mod sun;
mod shadow_fit;
mod textures;
pub mod tool_cursors;
mod render_glue;
mod annotation_canvas;
mod tool_overlay;
mod image_assets;
mod survey_raster;
mod text_outlines;
mod svg_texture;
mod view_export;
mod font_picker_host;
mod scale_editor;
mod material_png;
mod survey_import;
mod material_textures;
// R-look: isolated style and background passes.
mod style;
mod background;
mod antialias;
mod navigation;
mod depth_resolve;
mod depth;
mod backend;
mod edges;
mod faces;
mod scene_upload;
use scene_upload::{SceneUpload, PrototypeArrays};
mod xray;
mod ao;
mod resident_readback;
use numpy::PyReadonlyArray1;
use pyo3::{exceptions::PyValueError, prelude::*};
use std::time::Instant;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Command {
    count: u32,
    instances: u32,
    first: u32,
    base: i32,
    start: u32,
}
struct TimingSlot {
    read: wgpu::Buffer,
    active: Vec<&'static str>,
    pending: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}
struct Scene {
    material_shifts: Vec<i64>,
    material_data: Vec<u8>,
    material_layout_dirty: bool,
    face_ranges: Vec<(u32, u32, bool)>,
    inst: wgpu::Buffer,
    flags: Vec<u32>,
    rows: Vec<u8>,
    part_preview: Vec<usize>,
    tool_geometry: Option<geometry_preview::GeometryPreview>,
    protos: wgpu::Buffer,
    prototype_data: Vec<u8>,
    survivors: wgpu::Buffer,
    template_data: Vec<u8>,
    capacity: usize,
    edge_data: Vec<u8>,
    edge_records: wgpu::Buffer,
    viewpoints: wgpu::Buffer,
    counters: wgpu::Buffer,
    vertex_bytes: usize,
    index_data: Vec<u8>,
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    commands: wgpu::Buffer,
    template: wgpu::Buffer,
    hard_template: wgpu::Buffer,
    /// The index buffer of every edge row (`quad_indices`), as long as the
    /// widest row.
    quad_index: wgpu::Buffer,
    compute: wgpu::BindGroup,
    draw: wgpu::BindGroup,
    edge_draw: wgpu::BindGroup,
    xray_draw: wgpu::BindGroup,
    xray_edge_draw: wgpu::BindGroup,
    profile_draw: wgpu::BindGroup,
    n: u32,
    p: u32,
}
/// The last CPU read of a pick pixel remains valid across hit queries until
/// a render or a view / geometry change. A scissored ID pass alone cannot
/// mark the rest of the ID texture clean.
struct PickRead {
    dirty: bool,
    pixel: Option<(u32, u32, i64)>,
    tiles: pick_tile::Tiles,
}
impl PickRead {
    const DIRTY: Self = Self { dirty: true, pixel: None, tiles: pick_tile::Tiles::EMPTY };
}

/// One more view of the resident scene: its own surface, size-dependent
/// attachments, view-owned passes and uniforms. Everything that belongs to
/// the document (geometry, instances, prototypes, materials, textures, the
/// sun's depth map) stays single in the Renderer; ``bind_target`` swaps one
/// of these into the live fields, so every existing pass keeps working.
struct Target {
    surface: Option<wgpu::Surface<'static>>,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::Texture,
    offscreen: wgpu::Texture,
    ids: wgpu::Texture,
    aa: antialias::Antialias,
    transparency: xray::Xray,
    ao: ao::AmbientOcclusion,
    overlay: overlay::Overlay,
    axes: axes::Axes,
    lines: lines::Lines,
    preview: preview::Preview,
    face_marks: preview::Preview,
    group_feedback: Option<preview::Preview>,
    sections: sections::Sections,
    camera_data: Vec<f32>,
    look_data: [[f32; 4]; 11],
    sun: sun::Settings,
    style: u32,
    edges: bool,
    textures_enabled: bool,
    pick_read: PickRead,
    stable_frames: u32,
    pick_pending: bool,
    capture_next: bool,
    captured: Option<CapturedFrame>,
    /// ``scene_epoch`` when it was last live: a later scene change leaves
    /// its pick IDs stale.
    seen_epoch: u64,
}
static NEXT_RENDERER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[pyclass(unsendable)]
struct Renderer {
    id: u64,
    /// What the document ▾ and the project tabs were last sent (document_tabs.rs).
    document_tabs: Option<Py<render_glue::document_tabs::DocumentTabs>>,
    instance: wgpu::Instance,
    /// Inactive views of the same scene, addressed by a never-recycled id.
    targets: std::collections::HashMap<usize, Target>,
    active: usize,
    next_target: usize,
    /// Bumped by every change that invalidates pick IDs for all views.
    scene_epoch: u64,
    textures: textures::Textures,
    textures_enabled: bool,
    material_transparency: bool,
    texture_groups: Vec<wgpu::BindGroup>,
    ao: ao::AmbientOcclusion,
    samples: antialias::SampleModes,
    antialias: bool,
    pick_read: PickRead,
    camera_data: Vec<f32>,
    stable_frames: u32,
    pick_pending: bool,
    aa: antialias::Antialias,
    transparency: xray::Xray,
    look: style::Style,
    background: background::Background,
    look_groups: Vec<wgpu::BindGroup>,
    capture_next: bool,
    captured: Option<CapturedFrame>,
    overlay: overlay::Overlay,
    axes: axes::Axes,
    sections: sections::Sections,
    preview: preview::Preview,
    lines: lines::Lines,
    face_marks: preview::Preview,
    group_feedback: Option<preview::Preview>,
    sun: sun::Sun,
    references: references::References,
    host_context: preview::Preview,
    ghost_geometry: preview::Preview,
    ghost_objects: preview::Preview,
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::Texture,
    offscreen: wgpu::Texture,
    ids: wgpu::Texture,
    camera: wgpu::Buffer,
    compute: wgpu::ComputePipeline,
    compute_pick: wgpu::ComputePipeline,
    draw: wgpu::RenderPipeline,
    draw_plain: wgpu::RenderPipeline,
    draw_noids: wgpu::RenderPipeline,
    plain_noids: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    hard_edge: wgpu::RenderPipeline,
    xray: wgpu::RenderPipeline,
    xray_edge: wgpu::RenderPipeline,
    profile_top: wgpu::RenderPipeline,
    scene: Option<Scene>,
    multi: bool,
    style: u32,
    edges: bool,
    queries: Option<wgpu::QuerySet>,
    resolve: wgpu::Buffer,
    timings: Vec<TimingSlot>,
    last_gpu: Option<f64>,
    last_counters: std::collections::BTreeMap<String, u64>,
    last_pick_gpu: Option<f64>,
    pass_gpu: std::collections::BTreeMap<String, f64>,
    detailed_timestamps: bool,
    last_cpu: f64,
    last_acquire: f64,
    frame_stages: [f64; 5],
    read: wgpu::Buffer,
    gpu_memory: u64,
    /// egui chrome over the view ``ui_target`` (the UI spike); off by default.
    ui: Option<ui::Ui>,
    ui_target: usize,
    ui_scale: f32,
    #[pyo3(get)]
    adapter: String,
    #[pyo3(get)]
    backend_label: String,
}
fn row_runs(ids: Vec<usize>) -> Vec<(usize, usize)> {
    // The CPU mirror includes flags and overrides for untouched rows. Uploading
    // the enclosing range batches scattered selections into one queue write.
    match (ids.iter().min(), ids.iter().max()) {
        (Some(&first), Some(&last)) => vec![(first, last + 1)],
        _ => Vec::new(),
    }
}

/// The command rows as the GPU draws them. The scene keeps an edge row as
/// `[vertices, 0, first vertex, first instance, hard vertices]`, six
/// vertices a quad. The GPU draws a quad from four vertices through the
/// shared quad indices, so its row is the indexed command `[indices, 0, 0,
/// base vertex, first instance]`. `hard` leaves the soft edges out.
fn gpu_commands(template: &[u8], prototypes: usize, hard: bool) -> Vec<u8> {
    let mut out = template.to_vec();
    for row in out.chunks_exact_mut(20).skip(prototypes) {
        let [count, _, first, instance, hard_count]: [u32; 5] = bytemuck::pod_read_unaligned(row);
        let drawn = [if hard { hard_count } else { count }, 0, 0, first / 6 * 4, instance];
        row.copy_from_slice(bytemuck::bytes_of(&drawn));
    }
    out
}

/// Two triangles a quad over its four corners, first corner first in both
/// (the corner whose flat values a quad of six vertices took).
fn quad_indices(quads: usize) -> Vec<u8> {
    let indices: Vec<u32> = (0..quads as u32).flat_map(|q| [0, 1, 2, 0, 2, 3].map(|k| q * 4 + k)).collect();
    bytemuck::cast_slice(&indices).to_vec()
}

/// The most quads one edge row draws.
fn widest_edge_row(template: &[u8], prototypes: usize) -> usize {
    template.chunks_exact(20).skip(prototypes)
        .map(|row| u32::from_le_bytes(row[..4].try_into().unwrap()) as usize / 6).max().unwrap_or(0)
}

#[cfg(test)]
mod edge_command_tests {
    #[test]
    fn an_edge_row_becomes_an_indexed_command_over_four_vertices_a_quad() {
        let face = [36u32, 0, 12, 7, 5];
        let edge = [60u32, 0, 120, 5, 24];      // ten quads from the twentieth, four of them hard
        let template: Vec<u8> = bytemuck::cast_slice(&[face, edge]).to_vec();
        let rows = |hard| -> Vec<[u32; 5]> { bytemuck::pod_collect_to_vec(&super::gpu_commands(&template, 1, hard)) };
        assert_eq!(rows(false), [face, [60, 0, 0, 80, 5]]);
        assert_eq!(rows(true), [face, [24, 0, 0, 80, 5]]);
        assert_eq!(super::widest_edge_row(&template, 1), 10);
    }

    #[test]
    fn a_quad_is_two_triangles_that_start_at_its_first_corner() {
        let indices: Vec<u32> = bytemuck::pod_collect_to_vec(&super::quad_indices(2));
        assert_eq!(indices, [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
    }
}

fn grown_capacity(current: usize, needed: usize) -> usize {
    current.max(needed.max(1).next_power_of_two())
}

fn merge_instance_flags(old: u32, incoming: u32) -> u32 {
    (old & (6 | 64 | 128)) | (incoming & 9)
}

fn texture(d: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    let display_format = [format.remove_srgb_suffix()];
    d.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: if format.is_srgb() { &display_format } else { &[] },
    })
}
fn model_color_view(target: &wgpu::Texture) -> wgpu::TextureView {
    target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(target.format().remove_srgb_suffix()),
        ..Default::default()
    })
}

fn buffer(d: &wgpu::Device, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytes,
        usage,
    })
}
fn capacity_buffer(d: &wgpu::Device, bytes: &[u8], capacity: usize, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let mut padded = vec![0; capacity];
    padded[..bytes.len()].copy_from_slice(bytes);
    buffer(d, &padded, usage)
}

/// A larger buffer holding ``old``'s first ``used`` bytes, copied on the GPU.
/// Re-uploading the whole resident scene from a zero-padded CPU copy made
/// the first edit after an open wait ~0.4 s on the ride (the open uploads
/// exact-size buffers, so the first append always grows them).
fn grown_buffer(d: &wgpu::Device, q: &wgpu::Queue, old: &wgpu::Buffer, used: usize,
    capacity: usize, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let size = ((capacity.max(used).max(4) + 3) & !3) as u64;
    let grown = d.create_buffer(&wgpu::BufferDescriptor {
        label: None, size, usage: usage | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = d.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_buffer_to_buffer(old, 0, &grown, 0, (used as u64) & !3);
    // Submitted now: writes queued after this land after the copy.
    q.submit([encoder.finish()]);
    grown
}

fn bind_prototypes(d: &wgpu::Device, layout: &wgpu::BindGroupLayout,
    buffers: &[&wgpu::Buffer], count: u32) -> wgpu::BindGroup {
    // arrayLength(proto) defines the edge-command offset in the shader.
    // Bind the logical range even when its allocation has spare capacity.
    let entries: Vec<_> = buffers.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry {
        binding: i as u32,
        resource: if i == 2 {
            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: b, offset: 0, size: std::num::NonZeroU64::new(count as u64 * 32),
            })
        } else { b.as_entire_binding() },
    }).collect();
    d.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout, entries: &entries })
}

fn bind(
    d: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: &[&wgpu::Buffer],
) -> wgpu::BindGroup {
    let entries: Vec<_> = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: b.as_entire_binding(),
        })
        .collect();
    d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &entries,
    })
}
impl Renderer {
    /// The offscreen pass behind ``render_image`` and ``page_frame_render``:
    /// the pixels read back, or (``keep``) the rendered texture itself.
    #[allow(clippy::too_many_arguments)]
    fn render_offscreen(&mut self, width: u32, height: u32, view: Vec<f32>,
                        projection: Vec<f32>, style: &Bound<'_, PyAny>, instances: Option<PyReadonlyArray1<'_, u8>>,
                        far_clip: bool, sharp_shadows: bool, keep: bool) -> PyResult<(Vec<u8>, Option<wgpu::Texture>)> {
        if width == 0 || height == 0 || width > self.device.limits().max_texture_dimension_2d
            || height > self.device.limits().max_texture_dimension_2d {
            return Err(PyValueError::new_err("invalid offscreen dimensions"));
        }
        if view.len() != 16 || projection.len() != 16 {
            return Err(PyValueError::new_err("camera matrices need 16 floats"));
        }
        let temporary_rows = instances.as_ref().map(|v| v.as_slice()).transpose()?;
        if let Some(rows) = temporary_rows {
            if self.scene.as_ref().map(|s| s.rows.len()) != Some(rows.len()) {
                return Err(PyValueError::new_err("capture rows must match resident instances"));
            }
            let s = self.scene.as_ref().unwrap();
            if rows.chunks_exact(80).zip(s.rows.chunks_exact(80)).any(|(a, b)| a[..72] != b[..72]) {
                return Err(PyValueError::new_err("capture rows may only change visibility and tint"));
            }
        }
        let shadow_size = if sharp_shadows {
            sun::EXPORT_SIZE.min(self.device.limits().max_texture_dimension_2d)
        } else {
            sun::LIVE_SIZE
        };
        let format = self.config.format.remove_srgb_suffix();
        let mut aa = antialias::Antialias::new(self.samples.count(), &self.device, format, &self.look.buffer, width, height);
        let mut transparency = xray::Xray::new(&self.device, format, width, height);
        aa.bind_textures(&self.device, &self.textures);
        transparency.bind_textures(&self.device, &self.textures);
        if let Some(s) = &self.scene {
            aa.bind_scene(&self.device, &self.camera, &s.inst, &s.survivors,
                          &self.look.buffer, &s.edge_records, &s.viewpoints);
            transparency.bind_scene(&self.device, &self.camera, &s.inst, &s.survivors, &self.look.buffer);
        }
        let old_size = (self.config.width, self.config.height);
        let old_camera = self.camera_data.clone();
        let old_style = self.look.data;
        let old_pick = std::mem::replace(&mut self.pick_read, PickRead::DIRTY);
        let old_flags = (self.style, self.edges, self.textures_enabled, old_pick,
                         self.stable_frames, self.pick_pending, self.capture_next);
        let old_capture = self.captured.take();
        // Keep the original resources alive until the transaction has completed.
        let old_aa = std::mem::replace(&mut self.aa, aa);
        let old_transparency = std::mem::replace(&mut self.transparency, transparency);
        let old_sun = std::mem::replace(&mut self.sun, sun::Sun::new(self.samples.clone(), &self.device, format, shadow_size));
        let old_ao = std::mem::replace(&mut self.ao, ao::AmbientOcclusion::new(self.samples.clone(), &self.device, format));
        let old_overlay = std::mem::replace(&mut self.overlay, overlay::Overlay::new(&self.device, self.config.format));
        let old_color = std::mem::replace(&mut self.offscreen, texture(&self.device, width, height, self.config.format));
        let old_depth = std::mem::replace(&mut self.depth, texture(&self.device, width, height, wgpu::TextureFormat::Depth32Float));
        let old_ids = std::mem::replace(&mut self.ids, texture(&self.device, width, height, wgpu::TextureFormat::R32Uint));
        self.config.width = width;
        self.config.height = height;
        if let (Some(rows), Some(s)) = (temporary_rows, &self.scene) {
            self.queue.write_buffer(&s.inst, 0, rows);
        }
        // Scaffolding (edit box, guides) never belongs in a sheet capture.
        let lines_enabled = std::mem::replace(&mut self.lines.enabled, false);
        let result = (|| {
            self.set_style(style)?;
            self.set_camera(view, projection, far_clip)?;
            if keep {
                self.capture_next = false;
                self.render(false)?;
                return Ok(Vec::new());
            }
            self.capture_next = true;
            self.render(false)?;
            let mut pixels = self.take_capture().unwrap_or_default();
            if self.look.data[3][3] < 0. {
                // PNG and QImage RGBA8888 expect unassociated color.
                for pixel in pixels.chunks_exact_mut(4) {
                    let alpha = u32::from(pixel[3]);
                    if alpha > 0 && alpha < 255 {
                        for channel in &mut pixel[..3] {
                            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                        }
                    }
                }
            }
            Ok(pixels)
        })();
        if temporary_rows.is_some() {
            if let Some(s) = &self.scene { self.queue.write_buffer(&s.inst, 0, &s.rows); }
        }
        self.config.width = old_size.0;
        self.config.height = old_size.1;
        self.aa = old_aa;
        self.transparency = old_transparency;
        self.ao = old_ao;
        self.sun = old_sun;
        self.overlay = old_overlay;
        self.lines.enabled = lines_enabled;
        let rendered = std::mem::replace(&mut self.offscreen, old_color);
        self.depth = old_depth;
        self.ids = old_ids;
        self.look.data = old_style;
        self.queue.write_buffer(&self.look.buffer, 0, bytemuck::cast_slice(&old_style));
        if !old_camera.is_empty() {
            self.queue.write_buffer(&self.camera, 0, bytemuck::cast_slice(&old_camera));
        }
        self.camera_data = old_camera;
        (self.style, self.edges, self.textures_enabled, self.pick_read,
         self.stable_frames, self.pick_pending, self.capture_next) = old_flags;
        self.captured = old_capture;
        result.map(|pixels| (pixels, keep.then_some(rendered)))
    }
}

#[pymethods]
impl Renderer {
    #[new]
    #[pyo3(signature = (window, width, height, scale, *, display=0, platform=""))]
    fn new(window: usize, width: u32, height: u32, scale: f64, display: usize, platform: &str) -> PyResult<Self> {
        let instance = crate::backend::instance();
        let surface = if window == 0 && platform == "offscreen" {
            None
        } else {
            Some(backend::surface(&instance, window, display, platform)?)
        };
        let a = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: surface.as_ref(),
            ..Default::default()
        }))
        .ok_or_else(|| PyValueError::new_err("no supported GPU backend for this window"))?;
        let optional = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES | wgpu::Features::MULTI_DRAW_INDIRECT | wgpu::Features::TIMESTAMP_QUERY;
        if !a.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE) {
            return Err(PyValueError::new_err("GPU adapter lacks indirect first-instance drawing"));
        }
        let features = (a.features() & optional) | wgpu::Features::INDIRECT_FIRST_INSTANCE;
        let (device, queue) = pollster::block_on(a.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: features,
                required_limits: a.limits(),
                memory_hints: Default::default(),
            },
            None,
        ))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let (format, alpha_mode) = if let Some(surface) = &surface {
            let caps = surface.get_capabilities(&a);
            let format = caps.formats.iter().copied().find(|f| f.is_srgb())
                .ok_or_else(|| PyValueError::new_err("no sRGB surface"))?;
            (format, caps.alpha_modes[0])
        } else {
            (wgpu::TextureFormat::Bgra8UnormSrgb, wgpu::CompositeAlphaMode::Auto)
        };
        let w = ((width as f64 * scale) as u32).max(1);
        let h = ((height as f64 * scale) as u32).max(1);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width: w,
            height: h,
            present_mode: wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![format.remove_srgb_suffix()],
        };
        if let Some(surface) = &surface { surface.configure(&device, &config); }
        // Blend and resolve model colors in display space. UI paints through the sRGB view.
        let format = format.remove_srgb_suffix();
        let camera = buffer(
            &device,
            &[0; 112],
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let cs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cull"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &cs,
            entry_point: Some("cull"),
            compilation_options: Default::default(),
            cache: None,
        });
        let pick_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cursor culling"), bind_group_layouts: &[&compute.get_bind_group_layout(0)], push_constant_ranges: &[],
        });
        let compute_pick = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cursor placement culling"), layout: Some(&pick_layout), module: &cs, entry_point: Some("cull"),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &[("PICK".to_string(), 1.)].into_iter().collect(), ..Default::default() }, cache: None,
        });
        let vs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("draw"),
            source: wgpu::ShaderSource::Wgsl(concat!(include_str!("shading.wgsl"), "\n", include_str!("edges.wgsl")).into()),
        });
        let face_attrs = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4];
        let pipeline = |topology, fragment, compare, write, bias, samples| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState {
                    module: &vs,
                    entry_point: Some(if fragment == "edge_fs" || fragment == "hidden_edge_fs" || fragment == "profile_fs" { "edge_vs" } else { "vs" }),
                    compilation_options: Default::default(),
                    buffers: &{ let layouts = if fragment == "edge_fs" || fragment == "hidden_edge_fs" || fragment == "profile_fs" { vec![] } else { vec![wgpu::VertexBufferLayout {
                        array_stride: 28,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &face_attrs,
                    }] }; layouts },
                },
                fragment: Some(wgpu::FragmentState {
                    module: &vs,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: if fragment == "depth_fs" { wgpu::ColorWrites::empty() } else { wgpu::ColorWrites::ALL },
                        }),
                        (samples == 1).then_some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::R32Uint,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                    ],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: write,
                    depth_compare: depth::compare(compare),
                    stencil: Default::default(),
                    bias: depth::bias(bias),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview: None,
                cache: None,
            })
        };
        // Faces back off by their own slope, as GL's polygon-offset fill and
        // the antialiased pass do: with reverse-Z the edges' constant bias is
        // a few float steps at the face's own depth, far less than the old
        // fixed-point-like step near 1, and alone it lost edge pixels to a
        // face sloping away under the line's half-pixel width.
        let face_bias = wgpu::DepthBiasState { constant: 1, slope_scale: 1., clamp: 0. };
        let draw = pipeline(
            wgpu::PrimitiveTopology::TriangleList,
            "fs",
            wgpu::CompareFunction::Less,
            true,
            face_bias,
            1,
        );
        // Keep the same explicit layouts so resident draw groups can be reused.
        let plain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("untextured faces"),
            bind_group_layouts: &[&draw.get_bind_group_layout(0), &draw.get_bind_group_layout(1), &draw.get_bind_group_layout(2)],
            push_constant_ranges: &[],
        });
        let face_variant = |plain: bool, ids: bool| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("untextured faces without alpha discard"), layout: Some(&plain_layout),
            vertex: wgpu::VertexState { module: &vs, entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout { array_stride: 28, step_mode: wgpu::VertexStepMode::Vertex, attributes: &face_attrs }] },
            fragment: Some(wgpu::FragmentState { module: &vs, entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &[("TEXTURED".to_string(), if plain { 0. } else { 1. })].into_iter().collect(), ..Default::default() },
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL }),
                    ids.then_some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true, depth_compare: depth::compare(wgpu::CompareFunction::Less), stencil: Default::default(), bias: depth::bias(face_bias) }),
            multisample: Default::default(), multiview: None, cache: None,
        });
        let draw_plain = face_variant(true, true);
        let draw_noids = face_variant(false, false);
        let plain_noids = face_variant(true, false);
        let xray = pipeline(
            wgpu::PrimitiveTopology::TriangleList,
            "depth_fs",
            wgpu::CompareFunction::Less,
            true,
            Default::default(),
            1,
        );
        let bias = wgpu::DepthBiasState {
            constant: -2,
            slope_scale: -1.,
            clamp: 0.,
        };
        let edge = pipeline(
            wgpu::PrimitiveTopology::TriangleList,
            "edge_fs",
            wgpu::CompareFunction::LessEqual,
            false,
            bias,
            1,
        );
        let xray_edge = pipeline(
            wgpu::PrimitiveTopology::TriangleList,
            "hidden_edge_fs",
            wgpu::CompareFunction::Greater,
            false,
            bias,
            1,
        );
        let profile_top = pipeline(wgpu::PrimitiveTopology::TriangleList, "profile_fs",
            wgpu::CompareFunction::Always, false, bias, 1);
        // GPU timings are optional. A device that advertises timestamps can
        // still fail to allocate the counter sample buffer (Metal, late in a
        // long test run): without a scope wgpu's default handler panics on
        // that. Then the renderer measures nothing instead.
        let queries = features.contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
            for filter in [wgpu::ErrorFilter::Internal, wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Validation] {
                device.push_error_scope(filter);
            }
            let set = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: None,
                ty: wgpu::QueryType::Timestamp,
                count: timing::COUNT,
            });
            let failed = (0..3).map(|_| pollster::block_on(device.pop_error_scope())).fold(false, |f, e| f | e.is_some());
            (!failed).then_some(set)
        }).flatten();
        let timestamps = queries.is_some();
        let resolve = buffer(
            &device,
            &[0; timing::BYTES],
            wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        );
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hover ID readback"),
            size: u64::from(pick_tile::MAX_BYTES),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let samples = antialias::SampleModes::adapter(&a, format, features);
        let requested = Python::attach(|py| navigation::setting(py,"display/msaa","4"))?;
        samples.select(requested.parse().unwrap_or(4));
        let sun = sun::Sun::new(samples.clone(), &device, format, sun::LIVE_SIZE);
        let look = style::Style::new(&device);
        let preview = preview::Preview::new(samples.clone(), &device, format, &camera, &look.buffer, 0);
        let face_marks = preview::Preview::new(samples.clone(), &device, format, &camera, &look.buffer, 1);
        let host_context = preview::Preview::new(samples.clone(), &device, format, &camera, &look.buffer, 2);
        let ghost_geometry = preview::Preview::new(samples.clone(), &device, format, &camera, &look.buffer, 2);
        let ghost_objects = preview::Preview::new(samples.clone(), &device, format, &camera, &look.buffer, 2);
        let background = background::Background::new(&device, format, &look.buffer, 1);
        let look_groups = [&draw, &xray, &edge, &xray_edge, &profile_top].iter().map(|p|
            bind(&device, &p.get_bind_group_layout(1), &[&look.buffer])).collect();
        let textures = textures::Textures::new(&device, &queue, 1, 1, 1, &[vec![255;4]]);
        let texture_groups = [&draw, &xray].iter().map(|p| textures.bind(&device,p)).collect();
        let hard_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hard edges"), bind_group_layouts: &[&edge.get_bind_group_layout(0), &edge.get_bind_group_layout(1)], push_constant_ranges: &[],
        });
        let hard_edge = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hard edges without profile classification"), layout: Some(&hard_layout),
            vertex: wgpu::VertexState { module: &vs, entry_point: Some("edge_vs"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &[("PROFILES".to_string(), 0.)].into_iter().collect(), ..Default::default() }, buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &vs, entry_point: Some("edge_fs"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: false, depth_compare: depth::compare(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: depth::bias(bias) }),
            multisample: Default::default(), multiview: None, cache: None,
        });
        let mut aa = antialias::Antialias::new(samples.count(), &device, format, &look.buffer, w, h);
        aa.bind_textures(&device, &textures);
        let mut transparency = xray::Xray::new(&device, format, w, h);
        transparency.bind_textures(&device, &textures);
        Ok(Self {
            id: NEXT_RENDERER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            document_tabs: None,
            instance, targets: Default::default(), active: 0, next_target: 1, scene_epoch: 0,
            textures, texture_groups, textures_enabled: true, material_transparency: true,
            ao: ao::AmbientOcclusion::new(samples.clone(), &device, format),
            samples: samples.clone(), antialias: true,
            pick_read: PickRead::DIRTY,
            camera_data: Vec::new(),
            stable_frames: 0,
            pick_pending: false,
            aa,
            look, background, look_groups,
            transparency,
            preview,
            face_marks,
            group_feedback: None,
            host_context,
            ghost_geometry,
            ghost_objects,
            references: references::References::new(samples.clone(), &device, format),
            sun,
            capture_next: false,
            captured: None,
            overlay: overlay::Overlay::new(&device, config.format),
            axes: axes::Axes::new(samples.clone(), &device, format, &camera),
            sections: sections::Sections::new(samples.clone(), &device, format, &camera),
            lines: lines::Lines::new(samples.clone(), &device, format, &camera),
            depth: texture(&device, w, h, wgpu::TextureFormat::Depth32Float),
            offscreen: texture(&device, w, h, config.format),
            ids: texture(&device, w, h, wgpu::TextureFormat::R32Uint),
            timings: (0..4)
                .map(|_| TimingSlot {
                    read: buffer(
                        &device,
                        &[0; timing::BYTES + 32],
                        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    ),
                    pending: None,
                    active: Vec::new(),
                })
                .collect(),
            last_gpu: None,
            last_pick_gpu: None,
            pass_gpu: Default::default(),
            last_counters: Default::default(),
            detailed_timestamps: timestamps,
            last_cpu: 0.,
            last_acquire: 0.,
            frame_stages: [0.; 5],
            surface,
            device,
            queue,
            config,
            camera,
            compute,
            compute_pick,
            draw,
            draw_plain,
            draw_noids,
            plain_noids,
            edge,
            hard_edge,
            xray,
            xray_edge,
            profile_top,
            scene: None,
            style: 0,
            edges: true,
            multi: features.contains(wgpu::Features::MULTI_DRAW_INDIRECT),
            queries,
            resolve,
            read,
            gpu_memory: 0,
            ui: None,
            ui_target: 0,
            ui_scale: scale as f32,
            backend_label: format!("wgpu ({:?})", a.get_info().backend),
            adapter: format!(
                "{:?}; multi_draw={}",
                a.get_info(),
                features.contains(wgpu::Features::MULTI_DRAW_INDIRECT)
            ),
        })
    }

    /// Another view of the resident scene on its own NSView: one more
    /// surface and its attachments; the scene is not packed or sent again.
    #[pyo3(signature = (window, width, height, scale, *, display=0, platform=""))]
    fn add_target(&mut self, window: usize, width: u32, height: u32, scale: f64, display: usize, platform: &str) -> PyResult<usize> {
        let surface = backend::surface(&self.instance, window, display, platform)?;
        self.push_target(Some(surface), width, height, scale)
    }
    /// A camera and pick buffer for a pane composed into the winit window.
    fn add_tile_target(&mut self, width: u32, height: u32, scale: f64) -> PyResult<usize> {
        self.push_target(None, width, height, scale)
    }
    /// Present the pane images under a single copy of the window chrome.
    fn present_model_views(&mut self, panes: Vec<(usize, [f32; 4], bool, String)>, guides: Vec<[f32; 4]>) -> PyResult<(f64, Option<f64>)> {
        self.bind_target(0)?;
        for (id, _, _, _) in &panes {
            if !self.targets.get(id).is_some_and(|t| t.surface.is_none()) {
                return Err(PyValueError::new_err("expected a model pane target"));
            }
        }
        let ui = self.ui.as_mut().ok_or_else(|| PyValueError::new_err("attach the window chrome first"))?;
        ui.model_guides = guides;
        for (id, bounds, active, label) in panes {
            ui.add_model_pane(&self.device, id, &self.targets[&id].offscreen, bounds, active, label);
        }
        self.render_page(true)
    }
    /// Make view ``id`` the live one: later calls (camera, style, overlay,
    /// lines, preview, resize, render, pick) are its own.
    fn bind_target(&mut self, id: usize) -> PyResult<()> {
        if id == self.active {
            return Ok(());
        }
        let mut t = self.targets.remove(&id)
            .ok_or_else(|| PyValueError::new_err("no such view target"))?;
        let seen = t.seen_epoch;
        self.swap_live(&mut t);
        t.seen_epoch = self.scene_epoch;
        self.targets.insert(self.active, t);
        self.active = id;
        if seen != self.scene_epoch {
            self.sun.invalidate();
            self.pick_read = PickRead::DIRTY;
        }
        self.queue.write_buffer(&self.look.buffer, 0, bytemuck::cast_slice(&self.look.data));
        if !self.camera_data.is_empty() {
            self.queue.write_buffer(&self.camera, 0, bytemuck::cast_slice(&self.camera_data));
        }
        // The scene's buffers may have been replaced or grown since.
        self.refresh_msaa();
        self.aa.bind_textures(&self.device, &self.textures);
        self.transparency.bind_textures(&self.device, &self.textures);
        if let Some(s) = &self.scene {
            self.aa.bind_scene(&self.device, &self.camera, &s.inst, &s.survivors, &self.look.buffer, &s.edge_records, &s.viewpoints);
            self.transparency.bind_scene(&self.device, &self.camera, &s.inst, &s.survivors, &self.look.buffer);
        }
        Ok(())
    }
    /// Let view ``id`` go (its widget is going away); the first view stays.
    fn remove_target(&mut self, id: usize) -> PyResult<()> {
        if id == 0 || (id != self.active && !self.targets.contains_key(&id)) {
            return Err(PyValueError::new_err("no such removable view target"));
        }
        if self.active == id {
            self.bind_target(0)?;
        }
        if let Some(ui) = &mut self.ui { ui.release_model_texture(id); }
        self.targets.remove(&id);
        Ok(())
    }
    #[getter]
    fn active_target(&self) -> usize { self.active }
    #[getter]
    fn target_count(&self) -> usize { 1 + self.targets.len() }
    /// Bytes of every view's own attachments (surface not included): depth,
    /// colour and ID targets, multisampled colour and depth, X-ray
    /// accumulation — what one more view costs, apart from the scene.
    #[getter]
    fn target_memory(&self) -> u64 {
        let per_pixel = 4 + 4 + 4 + 16 + 16 + 8 + 2;
        let live = self.config.width as u64 * self.config.height as u64;
        self.targets.values()
            .map(|t| t.config.width as u64 * t.config.height as u64)
            .sum::<u64>().saturating_add(live) * per_pixel
    }
    /// Packed little-endian arrays; reject malformed uploads before reaching wgpu.
    fn set_scene(&mut self, prototypes: PrototypeArrays<'_>, instances: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        let raw = instances.as_slice()?;
        let limit = self.device.limits().max_storage_buffer_binding_size as usize;
        if raw.len() > limit || prototypes.2.as_slice()?.len() > limit {
            return Err(PyValueError::new_err("scene exceeds storage binding limit"));
        }
        let geometry = scene_upload::Geometry::prepare(prototypes.0.as_slice()?, prototypes.1.as_slice()?,
            prototypes.2.as_slice()?, prototypes.3.as_slice()?, raw)?;
        self.set_prepared_scene(&mut SceneUpload(Some(geometry)), instances)
    }

    fn set_prepared_scene(&mut self, upload: &mut SceneUpload, instances: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        let raw = instances.as_slice()?;
        let geometry = upload.0.as_ref().ok_or_else(|| PyValueError::new_err("scene was already installed"))?;
        // Camera rebasing can change translations between preparation and upload.
        // Prototype identity and row order must still match the validated commands.
        if raw.len() != geometry.codes.len() * 80 || raw.chunks_exact(80).zip(&geometry.codes)
            .any(|(row, code)| row[64..68] != code.to_le_bytes()) {
            return Err(PyValueError::new_err("instance rows changed after preparation"));
        }
        let geometry = upload.0.take().unwrap();
        let instances = raw;
        let scene_upload::Geometry { edges: expanded_v, vertices: face_v, indices: face_ix,
            commands: expanded_cmd, shifts: material_shifts, bounds, codes } = geometry;
        let p = bounds.as_slice();
        let n = codes.len();
        let count = p.len() / 32;
        let capacity = grown_capacity(0, n);
        let prototype_capacity = grown_capacity(0, count);
        let limit = self.device.limits().max_storage_buffer_binding_size as usize;
        if capacity * 80 > limit || prototype_capacity * 40 > limit {
            return Err(PyValueError::new_err("scene capacity exceeds storage binding limit"));
        }
        self.sun.invalidate();
        let (v, ix, cmd) = (face_v.as_slice(), face_ix.as_slice(), expanded_cmd.as_slice());
        if expanded_v.len() as u64 > self.device.limits().max_storage_buffer_binding_size as u64 {
            return Err(PyValueError::new_err("edge records exceed storage binding limit"));
        }
        // COPY_SRC: an edit grows these on the GPU (grown_buffer).
        let edge_records = buffer(&self.device, &expanded_v, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let vertex = buffer(&self.device, v, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let index = buffer(&self.device, ix, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let protos = capacity_buffer(&self.device, p, prototype_capacity * 32, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let mut padded = vec![0; capacity * 80];
        for row in padded.chunks_exact_mut(80) {
            row[72] = 1;
        }
        padded[..instances.len()].copy_from_slice(instances);

        let inst = buffer(
            &self.device,
            &padded,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let commands = capacity_buffer(
            &self.device,
            cmd,
            prototype_capacity * 40,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST,
        );
        let template = capacity_buffer(&self.device, &gpu_commands(cmd, count, false), prototype_capacity * 40, wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        let hard_template = capacity_buffer(&self.device, &gpu_commands(cmd, count, true), prototype_capacity * 40, wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        let quad_index = buffer(&self.device, &quad_indices(widest_edge_row(cmd, count).max(1)), wgpu::BufferUsages::INDEX);
        let survivors = buffer(&self.device, &vec![0; capacity * 4], wgpu::BufferUsages::STORAGE);
        let viewpoints = buffer(&self.device, &vec![0; capacity * 80], wgpu::BufferUsages::STORAGE);
        let counters = buffer(&self.device, &[0; 32], wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        let compute = bind_prototypes(
            &self.device,
            &self.compute.get_bind_group_layout(0),
            &[&self.camera, &inst, &protos, &commands, &survivors, &viewpoints, &counters],
            count as u32,
        );
        let draw = bind(
            &self.device,
            &self.draw.get_bind_group_layout(0),
            &[&self.camera, &inst, &survivors],
        );
        let edge_draw = bind(
            &self.device,
            &self.edge.get_bind_group_layout(0),
            &[&self.camera, &inst, &survivors, &edge_records, &viewpoints],
        );
        let xray_draw = bind(
            &self.device,
            &self.xray.get_bind_group_layout(0),
            &[&self.camera, &inst, &survivors],
        );
        let xray_edge_draw = bind(
            &self.device,
            &self.xray_edge.get_bind_group_layout(0),
            &[&self.camera, &inst, &survivors, &edge_records, &viewpoints],
        );
        let profile_draw = bind(&self.device, &self.profile_top.get_bind_group_layout(0), &[&self.camera, &inst, &survivors, &edge_records, &viewpoints]);
        self.transparency.bind_scene(&self.device, &self.camera, &inst, &survivors, &self.look.buffer);
        self.aa.bind_scene(&self.device, &self.camera, &inst, &survivors, &self.look.buffer, &edge_records, &viewpoints);
        self.gpu_memory =
            (v.len() + expanded_v.len() + ix.len() + p.len() + cmd.len() * 3 + instances.len() + n * 20 + 112 + 416)
                as u64 + self.textures.bytes + self.textures.materials.size();
        self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
        self.material_transparency = true; // unclassified raw scenes use conservative OIT
        self.scene = Some(Scene {
            edge_data: expanded_v,
            edge_records,
            viewpoints,
            counters,
            face_ranges: vec![(0, count as u32, true)],
            material_layout_dirty: true,
            material_data: vec![0; count*16],
            material_shifts,
            inst,
            rows: instances.to_vec(),
            part_preview: Vec::new(),
            tool_geometry: None,
            protos,
            prototype_data: bounds,
            survivors,
            template_data: expanded_cmd,
            capacity,
            vertex_bytes: v.len(),
            index_data: face_ix,
            flags: instances
                .chunks_exact(80)
                .map(|r| u32::from_le_bytes(r[72..76].try_into().unwrap()))
                .collect(),
            vertex,
            index,
            commands,
            template,
            hard_template,
            quad_index,
            compute,
            draw,
            edge_draw,
            xray_draw,
            xray_edge_draw,
            profile_draw,
            n: n as u32,
            p: count as u32,
        });
        self.preview.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
        self.group_feedback = None;
        self.end_tool_marks();
        self.face_marks.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
        self.ghost_geometry.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
        for t in self.targets.values_mut() {
            t.group_feedback = None;
            t.preview.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
            t.face_marks.pose_marks(&self.queue,glam::Mat4::IDENTITY.to_cols_array(),0);
            t.face_marks.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
            t.pick_read = PickRead::DIRTY;
        }
        self.ghost_objects.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
        // Queue submission orders the first draw after initialization.
        self.device.poll(wgpu::Maintain::Poll);
        Ok(())
    }
    /// Append compact replacement ranges while preserving placement identities.
    fn replace_prototype(&mut self, id: usize, vertices: PyReadonlyArray1<'_, u8>,
        indices: PyReadonlyArray1<'_, u8>, bounds: PyReadonlyArray1<'_, u8>, faces: u32) -> PyResult<()> {
        self.end_tool_geometry();
        self.sun.invalidate();
        let (v, ix, bounds) = (vertices.as_slice()?, indices.as_slice()?, bounds.as_slice()?);
        let s = self.scene.as_mut().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        if id > s.p as usize || v.len()%28 != 0 || ix.len()%4 != 0 || bounds.len()!=32 || faces as usize > ix.len()/4 {
            return Err(PyValueError::new_err("invalid prototype"));
        }
        if ix.chunks_exact(4).any(|i| u32::from_le_bytes(i.try_into().unwrap()) as usize >= v.len()/28) {
            return Err(PyValueError::new_err("vertex index out of range"));
        }
        let source = [Command { count: faces, instances: 0, first: 0, base: 0, start: 0 },
            Command { count: ix.len() as u32/4-faces, instances: 0, first: faces, base: 0, start: 0 }];
        let (edge, expanded_ix, mut commands) = edges::expand(v, ix, bytemuck::cast_slice(&source), 1).map_err(PyValueError::new_err)?;
        let (face, indices, shifts) = faces::compact(v, &expanded_ix, &mut commands, 1);
        let edge_base = s.edge_data.len()/48;
        let vertex_base = s.vertex_bytes/28;
        let index_base = s.index_data.len()/4;
        if (s.edge_data.len()+edge.len()).next_power_of_two() > self.device.limits().max_storage_buffer_binding_size as usize {
            return Err(PyValueError::new_err("edge capacity exceeds storage binding limit"));
        }
        if id == s.p as usize {
            if s.material_data.len() + 16 > self.device.limits().max_storage_buffer_binding_size as usize {
                return Err(PyValueError::new_err("material headers exceed storage binding limit"));
            }
            s.material_layout_dirty = true;
            let old_p = s.p as usize;
            s.p += 1;
            s.prototype_data.extend_from_slice(bounds);
            s.material_shifts.push(0);
            for row in s.material_data[..old_p*16].chunks_exact_mut(16) {
                let offset = u32::from_le_bytes(row[4..8].try_into().unwrap());
                if offset != 0 { row[4..8].copy_from_slice(&(offset+1).to_le_bytes()); }
            }
            s.material_data.splice(old_p*16..old_p*16, [0;16]);
            s.template_data.splice(old_p*20..old_p*20, [0;20]);
            s.template_data.extend_from_slice(&[0;20]);
        }
        s.face_ranges = face_ranges(&s.material_data, s.p);
        s.material_shifts[id] = vertex_base as i64 + shifts[0];
        s.prototype_data[id*32..(id+1)*32].copy_from_slice(bounds);
        s.vertex_bytes += face.len();
        s.index_data.extend_from_slice(&indices);
        s.edge_data.extend_from_slice(&edge);
        for (target, length, usage, offset, addition) in [
            (&mut s.vertex, s.vertex_bytes, wgpu::BufferUsages::VERTEX, vertex_base*28, face.as_slice()),
            (&mut s.index, s.index_data.len(), wgpu::BufferUsages::INDEX, index_base*4, indices.as_slice()),
            (&mut s.edge_records, s.edge_data.len(), wgpu::BufferUsages::STORAGE, edge_base*48, edge.as_slice())] {
            if length as u64 > target.size() {
                *target = grown_buffer(&self.device, &self.queue, target, offset,
                    grown_capacity(target.size() as usize, length), usage);
            }
            if !addition.is_empty() { self.queue.write_buffer(target, offset as u64, addition); }
        }
        let mut face_cmd: Command = bytemuck::pod_read_unaligned(&commands[..20]);
        face_cmd.base += vertex_base as i32;
        face_cmd.first += index_base as u32;
        let mut edge_cmd: [u32;5] = bytemuck::pod_read_unaligned(&commands[20..]);
        edge_cmd[2] += edge_base as u32*6;
        s.template_data[id*20..(id+1)*20].copy_from_slice(bytemuck::bytes_of(&face_cmd));
        s.template_data[(id+s.p as usize)*20..(id+s.p as usize+1)*20].copy_from_slice(bytemuck::bytes_of(&edge_cmd));
        let mut counts = vec![0u32; s.p as usize];
        for row in s.rows.chunks_exact(80) { counts[u32::from_le_bytes(row[64..68].try_into().unwrap()) as usize] += 1; }
        let mut start = 0u32;
        for (i, count) in counts.into_iter().enumerate() {
            s.template_data[i*20+16..i*20+20].copy_from_slice(&start.to_le_bytes());
            let offset = (i+s.p as usize)*20+12;
            s.template_data[offset..offset+4].copy_from_slice(&start.to_le_bytes());
            start += count;
        }
        let soft = gpu_commands(&s.template_data, s.p as usize, false);
        let hard = gpu_commands(&s.template_data, s.p as usize, true);
        let quads = quad_indices(widest_edge_row(&s.template_data, s.p as usize).max(1));
        if quads.len() as u64 > s.quad_index.size() { s.quad_index = buffer(&self.device, &quads, wgpu::BufferUsages::INDEX); }
        for (target, data, usage) in [
            (&mut s.protos, &s.prototype_data, wgpu::BufferUsages::STORAGE),
            (&mut s.template, &soft, wgpu::BufferUsages::COPY_SRC),
            (&mut s.hard_template, &hard, wgpu::BufferUsages::COPY_SRC),
            (&mut s.commands, &soft, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT)] {
            if data.len() as u64 > target.size() { *target = capacity_buffer(&self.device, data, grown_capacity(target.size() as usize, data.len()), usage | wgpu::BufferUsages::COPY_DST); }
            else { self.queue.write_buffer(target, 0, data); }
        }
        s.compute = bind_prototypes(&self.device, &self.compute.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.protos, &s.commands, &s.survivors, &s.viewpoints, &s.counters], s.p);
        s.edge_draw = bind(&self.device, &self.edge.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
        s.xray_edge_draw = bind(&self.device, &self.xray_edge.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
        s.profile_draw = bind(&self.device, &self.profile_top.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
        self.aa.bind_scene(&self.device, &self.camera, &s.inst, &s.survivors, &self.look.buffer, &s.edge_records, &s.viewpoints);
        self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
        Ok(())
    }

    fn begin_tool_geometry(&mut self, code:usize, vertices:PyReadonlyArray1<'_,u8>,
                           points:Vec<[f64;3]>, origin:[f64;3]) -> PyResult<bool> {
        self.end_tool_geometry();
        let Some(s)=self.scene.as_mut() else {return Ok(false)};
        s.tool_geometry=geometry_preview::GeometryPreview::begin(s,code,vertices.as_slice()?,points,origin);
        Ok(s.tool_geometry.is_some())
    }
    fn pose_tool_geometry(&mut self, matrix:[f64;16]) -> PyResult<()> {
        if matrix.iter().any(|v|!v.is_finite()) {return Err(PyValueError::new_err("invalid preview transform"))}
        if let Some(s)=&self.scene {
            if let Some(p)=&s.tool_geometry {
                p.write(s,&self.queue,Some(glam::DMat4::from_cols_array(&matrix)));
                self.pick_read=PickRead::DIRTY; self.scene_epoch += 1; self.stable_frames=0;
                self.sun.invalidate();
            }
        }
        Ok(())
    }
    fn end_tool_geometry(&mut self) {
        if let Some(s)=self.scene.as_mut() {
            if let Some(p)=s.tool_geometry.take() {
                p.write(s,&self.queue,None);
                self.pick_read=PickRead::DIRTY; self.scene_epoch += 1; self.stable_frames=0;
                self.sun.invalidate();
            }
        }
    }

    /// A transient assembly offset writes only the resident GPU translation.
    /// Releasing the slider or replacing the selection restores canonical rows.
    fn preview_parts(&mut self, offsets: Vec<(usize, [f32; 3])>) -> PyResult<()> {
        let Some(s) = self.scene.as_mut() else { return Ok(()); };
        if offsets.iter().any(|(id, delta)| *id >= s.n as usize || !delta.iter().all(|x| x.is_finite())) {
            return Err(PyValueError::new_err("invalid assembly preview"));
        }
        if offsets.is_empty() && s.part_preview.is_empty() { return Ok(()); }
        for row in s.part_preview.drain(..) {
            let at = row * 80 + 48;
            self.queue.write_buffer(&s.inst, at as u64, &s.rows[at..at+12]);
        }
        for (row, delta) in offsets {
            let at = row * 80 + 48;
            let mut position = [0_f32; 3];
            for (i, bytes) in s.rows[at..at+12].chunks_exact(4).enumerate() {
                position[i] = f32::from_le_bytes(bytes.try_into().unwrap()) + delta[i];
            }
            self.queue.write_buffer(&s.inst, at as u64, bytemuck::cast_slice(&position));
            s.part_preview.push(row);
        }
        self.sun.invalidate();
        self.pick_read = PickRead::DIRTY;
        self.stable_frames = 0;
        Ok(())
    }

    /// Change only matrices: preserve paint, highlights, visibility and pick IDs.
    fn set_instance_world(&mut self, ids: Vec<usize>, matrices: PyReadonlyArray1<'_, f32>) -> PyResult<()> {
        self.sun.invalidate();
        let matrices = matrices.as_slice()?;
        let s = self.scene.as_mut().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        if matrices.len() != ids.len()*16 || ids.iter().any(|&id| id >= s.n as usize)
            || matrices.iter().any(|x| !x.is_finite()) {
            return Err(PyValueError::new_err("invalid instance matrices"));
        }
        for (&id, matrix) in ids.iter().zip(matrices.chunks_exact(16)) {
            let data = bytemuck::cast_slice(matrix);
            s.rows[id*80..id*80+64].copy_from_slice(data);
            self.queue.write_buffer(&s.inst, (id*80) as u64, data);
        }
        if !ids.is_empty() { self.pick_read = PickRead::DIRTY; self.scene_epoch += 1; self.stable_frames = 0; }
        Ok(())
    }

    fn set_axes(&mut self, frame: Vec<f32>, spacing: f32, visible: bool) -> PyResult<()> {
        if frame.len() != 16 || frame.iter().any(|x| !x.is_finite()) || !spacing.is_finite() {
            return Err(PyValueError::new_err("invalid axes frame"));
        }
        self.axes.set(&self.queue, &frame, spacing.max(1e-4), visible);
        Ok(())
    }

    /// Update existing resident rows without replacing geometry or pick IDs.
    fn update_instances(&mut self, ids: Vec<usize>, rows: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        self.sun.invalidate();
        self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
        self.stable_frames = 0;
        let rows = rows.as_slice()?;
        let s = self.scene.as_mut().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        if rows.len() != ids.len() * 80 {
            return Err(PyValueError::new_err("invalid instance rows"));
        }
        for row in rows.chunks_exact(80) {
            if u32::from_le_bytes(row[64..68].try_into().unwrap()) >= s.p {
                return Err(PyValueError::new_err("prototype id out of range"));
            }
        }
        let max_rows = self.device.limits().max_storage_buffer_binding_size as usize / 80;
        if ids.iter().any(|&id| id >= max_rows) {
            return Err(PyValueError::new_err("instance rows exceed storage binding limit"));
        }
        let n = ids.iter().map(|i| i + 1).max().unwrap_or(0).max(s.n as usize);
        if grown_capacity(s.capacity, n) > max_rows {
            return Err(PyValueError::new_err("instance capacity exceeds storage binding limit"));
        }
        let structure = n != s.n as usize || ids.iter().zip(rows.chunks_exact(80)).any(|(&id, row)|
            row[64..68] != s.rows[id*80+64..id*80+68]);
        let old_n = s.n as usize;
        s.rows.resize(n * 80, 0);
        s.flags.resize(n, 1);
        for row in s.rows[old_n*80..].chunks_exact_mut(80) {
            row[72] = 1;
        }
        if n > s.capacity {
            s.capacity = grown_capacity(s.capacity, n);
            let mut padded = vec![0; s.capacity*80];
            for row in padded.chunks_exact_mut(80) {
                row[72] = 1;
            }
            padded[..s.rows.len()].copy_from_slice(&s.rows);
            s.inst = buffer(&self.device, &padded, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            s.survivors = buffer(&self.device, &vec![0; s.capacity*4], wgpu::BufferUsages::STORAGE);
            s.viewpoints = buffer(&self.device, &vec![0; s.capacity*80], wgpu::BufferUsages::STORAGE);
            s.compute = bind_prototypes(&self.device, &self.compute.get_bind_group_layout(0),
                &[&self.camera, &s.inst, &s.protos, &s.commands, &s.survivors, &s.viewpoints, &s.counters], s.p);
            s.draw = bind(&self.device, &self.draw.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors]);
            s.edge_draw = bind(&self.device, &self.edge.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
            s.xray_draw = bind(&self.device, &self.xray.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors]);
            s.xray_edge_draw = bind(&self.device, &self.xray_edge.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
            s.profile_draw = bind(&self.device, &self.profile_top.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
            self.aa.bind_scene(&self.device, &self.camera, &s.inst, &s.survivors, &self.look.buffer, &s.edge_records, &s.viewpoints);
            self.transparency.bind_scene(&self.device, &self.camera, &s.inst, &s.survivors, &self.look.buffer);
        }
        s.n = n as u32;
        for (&id, row) in ids.iter().zip(rows.chunks_exact(80)) {
            let mut value = row.to_vec();
            let flags = u32::from_le_bytes(row[72..76].try_into().unwrap());
            s.flags[id] = merge_instance_flags(s.flags[id], flags);
            value[72..76].copy_from_slice(&s.flags[id].to_le_bytes());
            value[76..80].copy_from_slice(&s.rows[id*80+76..id*80+80]);
            s.rows[id*80..(id+1)*80].copy_from_slice(&value);
        }
        for (first, end) in row_runs(ids) {
            self.queue.write_buffer(&s.inst, first as u64 * 80, &s.rows[first*80..end*80]);
        }
        if structure {
            let mut counts = vec![0u32; s.p as usize];
            for row in s.rows.chunks_exact(80) {
                counts[u32::from_le_bytes(row[64..68].try_into().unwrap()) as usize] += 1;
            }
            let mut starts = vec![0u32; counts.len()];
            for i in 1..counts.len() {
                starts[i] = starts[i-1] + counts[i-1];
            }
            for (i, command) in s.template_data.chunks_exact_mut(20).enumerate() {
                let offset = if i < s.p as usize { 16 } else { 12 };
                command[offset..offset+4].copy_from_slice(&starts[i % starts.len()].to_le_bytes());
            }
            self.queue.write_buffer(&s.template, 0, &gpu_commands(&s.template_data, s.p as usize, false));
            self.queue.write_buffer(&s.hard_template, 0, &gpu_commands(&s.template_data, s.p as usize, true));
        }
        Ok(())
    }

    /// Tombstones retain row identities so stale picks cannot select a neighbour.
    fn remove_instances(&mut self, ids: Vec<usize>) -> PyResult<()> {
        self.sun.invalidate();
        self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
        let s = self.scene.as_mut().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        if ids.iter().any(|&id| id >= s.n as usize) {
            return Err(PyValueError::new_err("instance id out of range"));
        }
        for id in ids {
            s.flags[id] = 1;
            s.rows[id*80+72..id*80+76].copy_from_slice(&1u32.to_le_bytes());
            self.queue.write_buffer(&s.inst, id as u64 * 80 + 72, &1u32.to_le_bytes());
        }
        Ok(())
    }

    /// `hovered_ids`: every row of the placement under the cursor (a
    /// component's nested rows light up together, as one object).
    #[pyo3(signature = (selected_ids, hovered_ids=Vec::new()))]
    fn set_highlight(
        &mut self,
        selected_ids: Vec<usize>,
        hovered_ids: Vec<usize>,
    ) -> PyResult<()> {
        if let Some(s) = &mut self.scene {
            if selected_ids
                .iter()
                .chain(hovered_ids.iter())
                .any(|&i| i >= s.flags.len())
            {
                return Err(PyValueError::new_err("instance id out of range"));
            }
            let selected: std::collections::HashSet<_> = selected_ids.into_iter().collect();
            let hovered: std::collections::HashSet<_> = hovered_ids.into_iter().collect();
            for (i, flag) in s.flags.iter_mut().enumerate() {
                let next = (*flag & !6)
                    | if selected.contains(&i) { 2 } else { 0 }
                    | if hovered.contains(&i) { 4 } else { 0 };
                if next != *flag {
                    *flag = next;
                    s.rows[i*80+72..i*80+76].copy_from_slice(&next.to_le_bytes());
                    self.queue
                        .write_buffer(&s.inst, i as u64 * 80 + 72, &next.to_le_bytes());
                }
            }
        }
        Ok(())
    }
    /// Edit context bits are independent of visibility, selection and tint.
    fn set_rest(&mut self, rows: Vec<usize>, mode: &str) -> PyResult<()> {
        self.sun.invalidate();
        let bits = match mode {
            "fade" => 64, "hide" => 64 | 128, "none" | "normal" => 0,
            _ => return Err(PyValueError::new_err("unknown edit rest mode")),
        };
        if let Some(s) = &mut self.scene {
            if rows.iter().any(|&i| i >= s.flags.len()) {
                return Err(PyValueError::new_err("instance id out of range"));
            }
            let mut rest = vec![false; s.flags.len()];
            for row in rows { rest[row] = true; }
            let mut changed = false;
            for (i, flag) in s.flags.iter_mut().enumerate() {
                let next = (*flag & !(64 | 128)) | if rest[i] { bits } else { 0 };
                changed |= next != *flag;
                *flag = next;
                s.rows[i*80+72..i*80+76].copy_from_slice(&next.to_le_bytes());
            }
            // One queue write per context change, rather than N tiny writes.
            if changed {
                self.queue.write_buffer(&s.inst, 0, &s.rows);
                self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
            }
        }
        Ok(())
    }
    fn set_overrides(&mut self, colors: Vec<u32>) -> PyResult<()> {
        self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
        if let Some(s) = &mut self.scene {
            if colors.len() != s.n as usize {
                return Err(PyValueError::new_err("one override per instance required"));
            }
            for (i, color) in colors.iter().enumerate() {
                if s.rows[i*80+76..i*80+80] == color.to_le_bytes() {
                    continue;
                }
                s.rows[i*80+76..i*80+80].copy_from_slice(&color.to_le_bytes());
                self.queue
                    .write_buffer(&s.inst, i as u64 * 80 + 76, &color.to_le_bytes());
            }
        }
        Ok(())
    }
    // R-look: retain string compatibility for existing diagnostic scripts.
    fn set_style(&mut self, style: &Bound<'_, PyAny>) -> PyResult<()> {
        self.pick_read = PickRead::DIRTY;
        let old_section = self.look.data[7];
        let old_opacity = self.look.data[9][0];
        let (mode, edges) = self.look.update(style, &self.queue)?;
        if old_section != self.look.data[7] || old_opacity != self.look.data[9][0] { self.sun.invalidate(); }
        self.ao.update(style)?;
        self.sun.update(style)?;
        self.style = mode;
        let name = if let Ok(d) = style.cast::<pyo3::types::PyDict>() {
            d.get_item("face_mode")?.map(|v| v.extract::<String>()).transpose()?.unwrap_or_default()
        } else { style.extract::<String>()? };
        self.textures_enabled = name != "shaded";
        if let Some(enabled) = edges { self.edges = enabled; }
        Ok(())
    }
    /// Pack classification avoids OIT and depth resolves for opaque scenes.
    fn set_material_transparency(&mut self, enabled: bool) { self.material_transparency = enabled; }
    fn set_reference_layers(&mut self, order: Vec<String>, updates: Vec<references::Upload<'_>>) -> PyResult<()> {
        self.references.replace(&self.device, &self.queue, &self.camera, order, updates)
    }

    fn max_texture_size(&self) -> u32 { self.device.limits().max_texture_dimension_2d }

    /// World-space tool feedback, cleared on commit/cancel.
    #[pyo3(signature = (triangles, edges, runs=Vec::new()))]
    fn set_preview_mesh(&mut self, py: Python<'_>, triangles: PyReadonlyArray1<'_, u8>, edges: PyReadonlyArray1<'_, u8>,
                        runs: Vec<preview_texture::Run<'_>>) -> PyResult<()> {
        let (triangles, edges) = (triangles.as_slice()?, edges.as_slice()?);
        preview::Preview::validate(triangles, edges).map_err(PyValueError::new_err)?;
        if runs.is_empty() {
            self.preview.textures = None;
        } else {
            let format = self.config.format.remove_srgb_suffix();
            let layout = self.preview.layout();
            self.preview.textures.get_or_insert_with(|| preview_texture::TexturedPreview::new(self.samples.clone(), &self.device, format, &layout, false))
                .upload(py, &self.device, &self.queue, runs)?;
        }
        self.preview.upload_validated(&self.device, triangles, edges);
        Ok(())
    }
    /// Detached clipboard geometry is baked once, then translated with one uniform.
    fn begin_group_feedback(&mut self, pack: &Bound<'_, PyAny>, frame: [f64; 3]) -> PyResult<()> {
        let format = self.config.format.remove_srgb_suffix();
        let mut layer = preview::Preview::new(self.samples.clone(), &self.device, format, &self.camera, &self.look.buffer, 3);
        group_feedback::upload(pack, frame, &mut layer, &self.device, &self.queue, self.samples.clone(), format)?;
        self.group_feedback = Some(layer);
        Ok(())
    }
    fn pose_group_feedback(&self, matrix: [f32; 16]) {
        if let Some(layer) = &self.group_feedback { layer.pose_group(&self.queue, matrix); }
    }
    fn end_group_feedback(&mut self) { self.group_feedback = None; }

    /// Depth-tested world-space lines in a named slot (``lines.rs``):
    /// 40-byte segments ``a, b (3 f32), RGBA8, width, dash, gap (f32
    /// physical pixels)``. An empty array empties the slot. A slot is drawn
    /// before the scene edges (GL's scaffold) unless ``above_edges``.
    #[pyo3(signature = (slot, segments, above_edges=false))]
    fn set_lines(&mut self, slot: &str, segments: PyReadonlyArray1<'_, u8>, above_edges: bool) -> PyResult<()> {
        self.lines.upload(&self.device, &self.queue, slot, segments.as_slice()?, above_edges).map_err(PyValueError::new_err)
    }
    #[pyo3(signature=(band, guides, scale))]
    fn set_tool_wire(&mut self, band: Option<&Bound<'_, PyAny>>, guides: &Bound<'_, PyAny>, scale: f32) -> PyResult<()> {
        let mut bytes = Vec::new();
        if let Some(band) = band {
            let color: Vec<f64> = band.get_item(1)?.extract()?;
            let rgba = std::array::from_fn(|i| (color.get(i).copied().unwrap_or(1.).clamp(0.,1.) * 255.).round() as u8);
            let width = band.get_item(2)?.extract::<f32>()?;
            lines::tool_segments(&mut bytes, &band.get_item(0)?, rgba, [scale*width,0.,0.])?;
        }
        lines::tool_segments(&mut bytes, guides, [70,90,120,255], [scale*1.5,scale*15.,scale*10.5])?;
        self.lines.tool_wire(&self.device, &self.queue, bytes).map_err(PyValueError::new_err)
    }
    fn line_counts(&self) -> Vec<(String, u32)> { self.lines.counts() }
    /// Loose-face marks only; groups use group_selection_edges.
    fn set_face_marks(&mut self, triangles: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        let data = triangles.as_slice()?;
        preview::Preview::validate(data, &[]).map_err(PyValueError::new_err)?;
        self.face_marks.set_mark_faces(&self.device, data);
        Ok(())
    }
    #[pyo3(signature=(selection, hover, keys))]
    fn begin_tool_marks(&mut self, selection:&Bound<'_,PyAny>, hover:Option<&Bound<'_,PyAny>>, keys:std::collections::HashSet<u128>) -> PyResult<()> {
        let data=transform_marks::edge_vertices(selection,hover,keys)?;
        preview::Preview::validate(&[],&data).map_err(PyValueError::new_err)?;
        self.face_marks.set_mark_edges(&self.device,&data);
        self.face_marks.pose_marks(&self.queue,glam::Mat4::IDENTITY.to_cols_array(),0);
        Ok(())
    }
    fn pose_tool_marks(&self, matrix:[f32;16], selected:u32) {
        self.face_marks.pose_marks(&self.queue,matrix,selected);
    }
    fn end_tool_marks(&mut self) {
        self.face_marks.set_mark_edges(&self.device,&[]);
        self.face_marks.pose_marks(&self.queue,glam::Mat4::IDENTITY.to_cols_array(),0);
    }
    /// Editable face IDs resolve to (prototype, local triangle index, count).
    /// Shared prototypes affect every placement: duplicate before a local edit.
    fn set_suppressed_faces(&mut self, ranges: Vec<(usize, u32, u32)>) -> PyResult<()> {
        self.sun.invalidate();
        let s = self.scene.as_ref().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        let data = suppressed_indices(&s.index_data, &s.template_data, s.p as usize, &ranges)
            .map_err(PyValueError::new_err)?;
        self.queue.write_buffer(&s.index, 0, &data);
        self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
        Ok(())
    }
    /// Soft/hidden endpoint pairs in prototype coordinates.
    /// Geometry-only reconstruction cannot distinguish a hidden edge from a seam.
    ///
    /// Only the prototypes named are touched, and the result is kept in the
    /// resident edge records and command template: an edit sends its own
    /// rebaked prototypes (cloning and rewriting every edge cost 0.45 s per
    /// edit on the ride), and ``replace_prototype`` no longer resets the
    /// other prototypes' hard-edge counts. A prototype's exclusions change
    /// only with its geometry, which ``replace_prototype`` gives a fresh
    /// range; sending the same pairs again leaves the records as they are.
    fn set_profile_exclusions(&mut self, pairs: Vec<(usize, Vec<f32>)>) -> PyResult<()> {
        let s = self.scene.as_mut().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        for (prototype, endpoints) in &pairs {
            if *prototype >= s.p as usize || endpoints.len()%6 != 0 || endpoints.iter().any(|v| !v.is_finite()) {
                return Err(PyValueError::new_err("profile exclusions need a valid prototype and finite endpoint pairs"));
            }
        }
        let mut touched = Vec::new();
        for (prototype, endpoints) in pairs {
            let keys: std::collections::HashSet<_> = endpoints.chunks_exact(6).map(|p| edges::key(bytemuck::cast_slice(&p[..3]), bytemuck::cast_slice(&p[3..]))).collect();
            let offset = (s.p as usize+prototype)*20;
            let command: [u32;5] = bytemuck::pod_read_unaligned(&s.template_data[offset..offset+20]);
            let range = (command[2]/6) as usize*48..((command[2]+command[0])/6) as usize*48;
            let mut hard = Vec::new(); let mut soft = Vec::new();
            for record in s.edge_data[range.clone()].chunks_exact(48) {
                let mut edge = record.to_vec();
                let n = glam::Vec3::from_array(bytemuck::pod_read_unaligned::<[f32;3]>(&edge[24..36]));
                let candidate = n.length_squared()>2. || keys.contains(&edges::key(&edge[..12],&edge[12..24]));
                if candidate {
                    if n.length_squared()<=2. { edge[24..36].copy_from_slice(bytemuck::cast_slice(&(n*2.).to_array())); }
                    soft.extend(edge);
                } else { hard.extend(edge); }
            }
            let count = (hard.len()/48*6) as u32;
            hard.extend(soft); s.edge_data[range.clone()].copy_from_slice(&hard);
            s.template_data[offset+16..offset+20].copy_from_slice(&count.to_le_bytes());
            touched.push(range);
        }
        let bytes: usize = touched.iter().map(|r| r.len()).sum();
        if bytes*2 > s.edge_data.len() {
            self.queue.write_buffer(&s.edge_records, 0, &s.edge_data);
        } else {
            for range in touched.into_iter().filter(|r| !r.is_empty()) {
                self.queue.write_buffer(&s.edge_records, range.start as u64, &s.edge_data[range]);
            }
        }
        self.queue.write_buffer(&s.template, 0, &gpu_commands(&s.template_data, s.p as usize, false));
        self.queue.write_buffer(&s.hard_template, 0, &gpu_commands(&s.template_data, s.p as usize, true));
        Ok(())
    }
    /// Per-prototype hidden COMMAND rows and original vertex/index streams.
    /// Refresh after world/visibility edits; object_rows are placement IDs,
    /// deliberately separate from the frozen instance flag ABI.
    #[pyo3(signature = (vertices, indices, hidden_commands, hidden_edge_commands, object_rows=Vec::new()))]
    fn set_hidden_geometry(&mut self, vertices: PyReadonlyArray1<'_, u8>, indices: PyReadonlyArray1<'_, u8>,
        hidden_commands: PyReadonlyArray1<'_, u8>, hidden_edge_commands: PyReadonlyArray1<'_, u8>, object_rows: Vec<usize>) -> PyResult<()> {
        let s = self.scene.as_ref().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        let (vertices,indices,faces,edges)=(vertices.as_slice()?,indices.as_slice()?,hidden_commands.as_slice()?,hidden_edge_commands.as_slice()?);
        // Only hidden-object ghosts need compact resident faces. Read them on
        // demand instead of retaining a second full CPU vertex allocation.
        let resident = resident_readback::read(&self.device, &self.queue, s, &object_rows)
            .map_err(PyValueError::new_err)?;
        let (triangles,lines)=ghost_mesh(s,&resident,vertices,indices,faces,edges,&object_rows,false).map_err(PyValueError::new_err)?;
        let (objects,object_edges)=ghost_mesh(s,&resident,vertices,indices,faces,edges,&object_rows,true).map_err(PyValueError::new_err)?;
        self.ghost_geometry.upload(&self.device,&triangles,&lines).map_err(PyValueError::new_err)?;
        self.ghost_objects.upload(&self.device,&objects,&object_edges).map_err(PyValueError::new_err)?;
        Ok(())
    }
    fn msaa_levels(&self) -> Vec<u32> { self.samples.levels.to_vec() }
    fn set_msaa_samples(&mut self, requested: u32) {
        self.samples.select(requested);
        self.refresh_msaa();
    }
    fn set_antialias(&mut self, enabled: bool) { self.antialias = enabled; }
    #[getter]
    fn needs_shadow_fit(&self) -> bool {
        self.style != 1 && self.style != 4 && self.sun.needs_fit()
    }
    fn settle_shadows(&mut self) {
        if self.needs_shadow_fit() {
            self.sun.settle(&self.device, &self.queue, &self.depth, &self.camera_data);
        }
    }
    fn read_pixels(&mut self) -> PyResult<Vec<u8>> {
        self.capture_next = true;
        self.render(true)?;
        Ok(self.take_capture().unwrap_or_default())
    }
    /// Render with resident geometry and textures into isolated sheet targets.
    /// Surface configuration and interactive attachments are never replaced.
    #[pyo3(signature = (width, height, view, projection, style, instances=None, far_clip=false, sharp_shadows=false))]
    fn render_image(&mut self, width: u32, height: u32, view: Vec<f32>,
                    projection: Vec<f32>, style: &Bound<'_, PyAny>, instances: Option<PyReadonlyArray1<'_, u8>>,
                    far_clip: bool, sharp_shadows: bool) -> PyResult<Vec<u8>> {
        Ok(self.render_offscreen(width, height, view, projection, style, instances, far_clip, sharp_shadows, false)?.0)
    }
    /// A sheet frame's picture rendered straight into a texture of the
    /// Sheets page (no read-back): as ``render_image``, kept under the
    /// frame's ``key`` for the page to draw.
    #[pyo3(signature = (key, width, height, view, projection, style, instances=None, far_clip=false, sharp_shadows=false))]
    #[allow(clippy::too_many_arguments)]
    fn page_frame_render(&mut self, key: u64, width: u32, height: u32, view: Vec<f32>,
                         projection: Vec<f32>, style: &Bound<'_, PyAny>, instances: Option<PyReadonlyArray1<'_, u8>>,
                         far_clip: bool, sharp_shadows: bool) -> PyResult<()> {
        let (_, texture) = self.render_offscreen(width, height, view, projection, style, instances, far_clip, sharp_shadows, true)?;
        if let (Some(t), Some(ui)) = (texture, self.ui.as_mut()) {
            ui.page_native_image(&self.device, key, t);
        }
        Ok(())
    }
    fn set_edges(&mut self, enabled: bool) {
        if self.edges != enabled { self.pick_read = PickRead::DIRTY; }
        self.edges = enabled;
    }
    /// ``far_clip``: keep the projection's far plane (a far clip set by
    /// hand); otherwise a perspective view reaches to infinity, which costs
    /// no precision with reverse-Z (see ``depth``).
    #[pyo3(signature = (view, proj, far_clip=false))]
    fn set_camera(&mut self, view: Vec<f32>, proj: Vec<f32>, far_clip: bool) -> PyResult<()> {
        let v: [f32; 16] = view
            .try_into()
            .map_err(|_| PyValueError::new_err("view needs 16 floats"))?;
        let p: [f32; 16] = proj
            .try_into()
            .map_err(|_| PyValueError::new_err("projection needs 16 floats"))?;
        let projection = depth::projection(&p, far_clip);
        self.ao.camera(projection);
        let (row, lane) = background::FLAT_SLOT;
        let flat = if background::flat_view(&v, &p) { 1f32 } else { 0. };
        if self.look.data[row][lane] != flat {
            self.look.data[row][lane] = flat;
            self.queue.write_buffer(&self.look.buffer, ((row * 4 + lane) * 4) as u64, &flat.to_le_bytes());
        }
        let vp = projection * glam::Mat4::from_cols_array(&v);
        let mut data = vp.to_cols_array().to_vec();
        data.extend([
            self.config.width as f32,
            self.config.height as f32,
            self.style as f32,
            if self.textures_enabled { 0. } else { 1. },
        ]);
        let inverse_view = glam::Mat4::from_cols_array(&v).inverse();
        let eye = inverse_view * if p[15] == 0. { glam::Vec4::W } else { glam::Vec4::Z };
        data.extend(eye.to_array());
        data.extend([-1., -1., 1., 1.]);
        if self.camera_data != data {
            self.sun.camera_changed();
            self.stable_frames = 0;
            self.pick_read = PickRead::DIRTY;
            self.queue.write_buffer(&self.camera, 0, bytemuck::cast_slice(&data));
            self.camera_data = data;
        }
        Ok(())
    }
    /// Update a face's transient material rows. Restore from the current canonical stream.
    fn set_tool_material_range(&mut self, code: usize, first: usize, records: PyReadonlyArray1<'_, u8>, restore: bool) -> PyResult<()> {
        let Some(scene) = &self.scene else { return Ok(()); };
        if code >= scene.p as usize { return Ok(()); }
        let rows = records.as_slice()?;
        let header: [u32; 4] = bytemuck::pod_read_unaligned(&scene.material_data[code*16..code*16+16]);
        if rows.len() % 32 != 0 || first.checked_add(rows.len()/32).is_none_or(|end| end > header[2] as usize) {
            return Err(PyValueError::new_err("invalid texture preview range"));
        }
        let offset = header[1] as usize * 16 + first * 32;
        let canonical = scene.material_data.get(offset..offset+rows.len()).ok_or_else(|| PyValueError::new_err("invalid texture preview header"))?;
        if !rows.is_empty() {
            self.queue.write_buffer(&self.textures.materials, offset as u64, if restore { canonical } else { rows });
            self.pick_read = PickRead::DIRTY;
            self.scene_epoch += 1;
            self.stable_frames = 0;
            self.sun.invalidate();
        }
        Ok(())
    }
    fn update_prototype_materials(&mut self, id: usize, data: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        self.sun.invalidate();
        let data = data.as_slice()?;
        let s = self.scene.as_mut().ok_or_else(|| PyValueError::new_err("no scene"))?;
        if id >= s.p as usize || data.len()%32 != 0 { return Err(PyValueError::new_err("invalid material rows")); }
        let limit = self.device.limits().max_storage_buffer_binding_size as usize;
        if data.len() > limit.saturating_sub(s.material_data.len()) {
            return Err(PyValueError::new_err("material records exceed storage binding limit"));
        }
        let mut header = [0u32;4];
        if !data.is_empty() {
            let face: Command = bytemuck::pod_read_unaligned(&s.template_data[id*20..id*20+20]);
            header = [face.base as u32, (s.material_data.len()/16) as u32, (data.len()/32) as u32, 0];
        }
        s.material_data[id*16..(id+1)*16].copy_from_slice(bytemuck::bytes_of(&header));
        let appended_at = s.material_data.len();
        s.material_data.extend_from_slice(data);
        s.face_ranges = face_ranges(&s.material_data, s.p);
        // Write only this prototype's header and its new records into the
        // resident buffer. Allocating the whole table per call (one call per
        // rebaked prototype) queued 432 full copies of the ride's material
        // table on one 432-part bridge edit: ~30 GB before the GPU let any go.
        let materials = &self.textures.materials;
        if s.material_data.len() as u64 <= materials.size()
            && materials.usage().contains(wgpu::BufferUsages::COPY_DST) {
            if s.material_layout_dirty {
                self.queue.write_buffer(materials, 0, &s.material_data);
                s.material_layout_dirty = false;
                return Ok(());
            }
            self.queue.write_buffer(materials, (id*16) as u64, bytemuck::bytes_of(&header));
            if !data.is_empty() {
                self.queue.write_buffer(materials, appended_at as u64, data);
            }
            return Ok(());
        }
        // Out of room (or the exact-size buffer of an open): grow to the next
        // power of two once, so the following edits write in place.
        s.material_layout_dirty = false;
        let capacity = grown_capacity(materials.size() as usize, s.material_data.len()).min(limit);
        self.gpu_memory = self.gpu_memory.saturating_sub(materials.size());
        self.textures.materials = capacity_buffer(&self.device, &s.material_data, capacity,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        self.gpu_memory += self.textures.materials.size();
        self.texture_groups = [&self.draw, &self.xray].iter().map(|p| self.textures.bind(&self.device,p)).collect();
        self.aa.bind_textures(&self.device, &self.textures);
        self.transparency.bind_textures(&self.device, &self.textures);
        Ok(())
    }
    fn set_materials(&mut self, data: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        self.sun.invalidate();
        self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
        let data = data.as_slice()?;
        let scene = self.scene.as_mut().ok_or_else(|| PyValueError::new_err("no scene"))?;
        if data.len() % 16 != 0 || data.len() < scene.p as usize * 16
            || data.len() as u64 > self.device.limits().max_storage_buffer_binding_size as u64 {
            return Err(PyValueError::new_err("invalid sparse material stream"));
        }
        let mut resident = data.to_vec();
        for (header, shift) in resident.chunks_exact_mut(16).zip(&scene.material_shifts) {
            let base = u32::from_le_bytes(header[..4].try_into().unwrap());
            header[..4].copy_from_slice(&((i64::from(base) + shift) as u32).to_le_bytes());
        }
        scene.face_ranges = face_ranges(&resident, scene.p);
        scene.material_layout_dirty = false;
        scene.material_data = resident.clone();
        let data = resident.as_slice();
        self.gpu_memory = self.gpu_memory.saturating_sub(self.textures.materials.size());
        self.textures.materials = buffer(&self.device, data, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        self.gpu_memory += self.textures.materials.size();
        self.texture_groups = [&self.draw, &self.xray].iter().map(|p| self.textures.bind(&self.device,p)).collect();
        self.aa.bind_textures(&self.device, &self.textures);
        self.transparency.bind_textures(&self.device, &self.textures);
        Ok(())
    }
    #[pyo3(signature = (width, height, layers, levels, recipes=None))]
    fn set_textures(&mut self, width: u32, height: u32, layers: u32, levels: Vec<Vec<u8>>, recipes: Option<&[u8]>) -> PyResult<()> {
        self.sun.invalidate();
        self.pick_read = PickRead::DIRTY; self.scene_epoch += 1;
        let limits = self.device.limits();
        if width == 0 || height == 0 || layers == 0 || width > limits.max_texture_dimension_2d
            || height > limits.max_texture_dimension_2d || layers > limits.max_texture_array_layers
            || levels.len() != (32-width.max(height).leading_zeros()) as usize
            || levels.iter().enumerate().any(|(k,v)| v.len() as u64 != (width>>k).max(1) as u64 * (height>>k).max(1) as u64 * layers as u64 * 4) {
            return Err(PyValueError::new_err("invalid texture array dimensions or mip data"));
        }
        if recipes.is_some_and(|r| !textures::valid_recipes(r, layers) ||
            r.len() as u64 > limits.max_storage_buffer_binding_size as u64) {
            return Err(PyValueError::new_err("invalid PBR recipe table"));
        }
        self.gpu_memory = self.gpu_memory.saturating_sub(self.textures.bytes);
        let mut textures = textures::Textures::new(&self.device, &self.queue, width, height, layers, &levels);
        if let Some(data) = recipes {
            textures.recipes = buffer(&self.device, data, wgpu::BufferUsages::STORAGE);
            textures.bytes += data.len() as u64;
        }
        std::mem::swap(&mut textures.materials, &mut self.textures.materials);
        self.textures = textures;
        self.gpu_memory += self.textures.bytes;
        self.texture_groups = [&self.draw, &self.xray].iter().map(|p| self.textures.bind(&self.device,p)).collect();
        self.aa.bind_textures(&self.device, &self.textures);
        self.transparency.bind_textures(&self.device, &self.textures);
        Ok(())
    }
    #[getter]
    fn texture_memory(&self) -> u64 { self.textures.bytes }
    fn set_section_geometry(&mut self, vertices: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        let bytes = vertices.as_slice()?;
        if bytes.len() % 72 != 0 {
            return Err(PyValueError::new_err("section geometry must contain 24-byte triangle vertices"));
        }
        self.gpu_memory = self.gpu_memory.saturating_sub(self.sections.count as u64 * 24);
        self.sections.upload(&self.device, bytes);
        self.gpu_memory += bytes.len() as u64;
        Ok(())
    }
    fn set_msaa(&mut self, enabled: bool) {
        self.antialias = enabled;
    }
    fn resize(&mut self, w: u32, h: u32, scale: f64) {
        self.pick_read = PickRead::DIRTY;
        self.sun.camera_changed();
        self.ui_scale = scale as f32;
        self.config.width = ((w as f64 * scale) as u32).max(1);
        self.config.height = ((h as f64 * scale) as u32).max(1);
        if let Some(ui) = &mut self.ui { ui.release_model_texture(self.active); }
        if let Some(surface) = &self.surface { surface.configure(&self.device, &self.config); }
        self.transparency.resize(&self.device, self.config.width, self.config.height);
        self.aa.resize(&self.device, self.config.width, self.config.height);
        self.offscreen = texture(&self.device, self.config.width, self.config.height, self.config.format);
        self.depth = texture(
            &self.device,
            self.config.width,
            self.config.height,
            wgpu::TextureFormat::Depth32Float,
        );
        self.ids = texture(
            &self.device,
            self.config.width,
            self.config.height,
            wgpu::TextureFormat::R32Uint,
        );
    }
    #[pyo3(signature = (present=true))]
    fn render(&mut self, present: bool) -> PyResult<(f64, Option<f64>)> {
        self.pick_read.pixel = None;
        let start = Instant::now();
        // Poll completion only; never wait for a timestamp in the render path.
        self.device.poll(wgpu::Maintain::Poll);
        for slot in &mut self.timings {
            let ready = slot.pending.as_ref().and_then(|rx| rx.try_recv().ok());
            if let Some(result) = ready {
                if result.is_ok() {
                    let data = slot.read.slice(..).get_mapped_range();
                    let first = u64::from_le_bytes(data[0..8].try_into().unwrap());
                    let last = u64::from_le_bytes(data[24..32].try_into().unwrap());
                    self.last_gpu = Some(
                        last.saturating_sub(first) as f64
                            * self.queue.get_timestamp_period() as f64
                            / 1e6,
                    );
                    if self.detailed_timestamps {
                        self.pass_gpu = timing::decode(&data, self.queue.get_timestamp_period() as f64);
                        for (name, value) in &mut self.pass_gpu { if !slot.active.contains(&name.as_str()) { *value = 0.; } }
                        let ranges: Vec<_> = slot.active.iter().filter(|n| **n != "frame").filter_map(|name| {
                            let k = timing::index(name) as usize * 8;
                            let a = u64::from_le_bytes(data[k..k+8].try_into().unwrap());
                            let b = u64::from_le_bytes(data[k+8..k+16].try_into().unwrap());
                            (b > a && a > 0).then_some((a,b))
                        }).collect();
                        self.last_gpu = ranges.iter().map(|p| p.0).min().zip(ranges.iter().map(|p| p.1).max()).map(|(a,b)| (b-a) as f64 * self.queue.get_timestamp_period() as f64 / 1e6);
                        self.pass_gpu.insert("frame".into(), self.last_gpu.unwrap_or(0.));
                    }
                    for (k, name) in ["instances_submitted", "culled_frustum", "culled_below_pixel", "triangles_submitted", "edge_quads_input", "edge_quads_surviving"].iter().enumerate() {
                        let at = timing::BYTES + k*4;
                        self.last_counters.insert(name.to_string(), u32::from_le_bytes(data[at..at+4].try_into().unwrap()) as u64);
                    }
                    drop(data);
                    slot.read.unmap();
                }
                slot.pending = None;
            }
        }
        let timing_slot = self.timings.iter().position(|s| s.pending.is_none());
        let s = self
            .scene
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        let polled = Instant::now();
        // Metal can wait for the compositor here. The document adapter can
        // continue populating the private scene during that wait.
        let frame = if let Some(surface) = self.surface.as_ref().filter(|_| present) { Some(Python::attach(|py| py.detach(|| surface.get_current_texture()))
            .map_err(|e| PyValueError::new_err(e.to_string()))?) } else { None };
        let target = frame.as_ref().map(|f| &f.texture).unwrap_or(&self.offscreen);
        let acquired = Instant::now();
        self.last_acquire = acquired.duration_since(polled).as_secs_f64() * 1000.;
        let color = model_color_view(target);
        let depth = self.depth.create_view(&Default::default());
        let ids = self.ids.create_view(&Default::default());
        let mut e = self.device.create_command_encoder(&Default::default());
        let tq = self.queries.as_ref().filter(|_| self.detailed_timestamps);
        self.sun.prepare(&self.device, &self.queue, &mut e, s, &self.textures, &self.look.buffer);
        self.axes.welcome_grid = self.ui.as_ref().is_some_and(ui::Ui::empty_start_shown);
        // The multisampled path draws its own background and resolves over
        // every pixel of this target: a background here was drawn to be
        // overwritten.
        let background_drawn = !(PassPlan::new(self.antialias && self.samples.count() > 1, self.style, self.look.data[6][2] != 0., self.ao.active(self.style)).msaa && !self.pick_pending);
        if background_drawn {
            self.background.draw(&mut e, &color, tq);
        }
        e.clear_buffer(&s.counters, 0, None);
        e.copy_buffer_to_buffer(if self.look.data[6][0] > 0. { &s.template } else { &s.hard_template }, 0, &s.commands, 0, s.p as u64 * 40);
        {
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: self
                    .queries
                    .as_ref()
                    .map(|q| wgpu::ComputePassTimestampWrites {
                        query_set: q,
                        beginning_of_pass_write_index: Some(timing::index("culling")),
                        end_of_pass_write_index: Some(timing::index("culling")+1),
                    }),
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &s.compute, &[]);
            pass.dispatch_workgroups(s.n.div_ceil(64), 1, 1);
        }
        let plan = PassPlan::new(self.antialias && self.samples.count() > 1, self.style, self.look.data[6][2] != 0., self.ao.active(self.style));
        let msaa = plan.msaa && !self.pick_pending;
        self.stable_frames = self.stable_frames.saturating_add(1);
        let color_only = !msaa && plan.color_only(self.style, self.edges);
        let draw_ids = if color_only { needs_settled_ids(self.pick_pending, self.pick_read.dirty, self.stable_frames) }
            else { needs_id_pass(msaa, self.pick_pending, self.pick_read.dirty, self.stable_frames) };
        if !msaa { self.references.draw(&mut e, &color, &depth, None); }
        if plan.xray { self.transparency.draw(&mut e, &color, None, s, self.multi, tq); }

        let ao_active = plan.ao;
        let mut sun_drawn = false;
        for edge_only in [false, true] {
            if edge_only && (!msaa || self.pick_pending) {
                self.axes.draw(&mut e, &color, &depth, None);
            }
            // GL's scaffold (box, guides) sits between faces and edges: the
            // scene edges paint over it. After sun and AO, which shade faces.
            if edge_only {
                if msaa { self.lines.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color), false); }
                else { self.lines.draw(&mut e, &color, &depth, None, false); }
            }
            if edge_only && !(self.edges || self.style == 4 || ao_active || self.axes.visible || self.axes.welcome_grid) { break; }
            if !msaa || self.pick_pending {
            let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &color,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    (draw_ids || ao_active).then_some(wgpu::RenderPassColorAttachment {
                        view: &ids,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if edge_only { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) },
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: if edge_only || self.references.active() { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(depth::CLEAR) },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: tq.map(|q| {
                    let name = if edge_only { "edge_quads" } else { "faces" };
                    wgpu::RenderPassTimestampWrites { query_set: q,
                        beginning_of_pass_write_index: Some(timing::index(name)),
                        end_of_pass_write_index: Some(timing::index(name)+1) }
                }),
                occlusion_query_set: None,
            });
            pass.set_pipeline(if self.style == 1 || msaa {
                &self.xray
            } else if !draw_ids {
                &self.draw_noids
            } else if self.textures.layers == 1 {
                &self.draw_plain
            } else {
                &self.draw
            });
            pass.set_bind_group(
                0,
                if self.style == 1 || msaa {
                    &s.xray_draw
                } else {
                    &s.draw
                },
                &[],
            );
            pass.set_bind_group(1, &self.look_groups[if self.style == 1 || msaa { 1 } else { 0 }], &[]);
            pass.set_bind_group(2, &self.texture_groups[usize::from(self.style == 1 || msaa)], &[]);
            pass.set_vertex_buffer(0, s.vertex.slice(..));
            pass.set_index_buffer(s.index.slice(..), wgpu::IndexFormat::Uint32);
            if !edge_only && self.style != 4 {
                if self.style != 1 && !msaa {
                    for &(first, count, plain) in &s.face_ranges {
                        pass.set_pipeline(match (plain, draw_ids) {
                            (true, true) => &self.draw_plain, (false, true) => &self.draw,
                            (true, false) => &self.plain_noids, (false, false) => &self.draw_noids,
                        });
                        if self.multi { pass.multi_draw_indexed_indirect(&s.commands, first as u64 * 20, count); }
                        else { for k in first..first+count { pass.draw_indexed_indirect(&s.commands, k as u64 * 20); } }
                    }
                } else if self.multi {
                    pass.multi_draw_indexed_indirect(&s.commands, 0, s.p);
                } else {
                    for k in 0..s.p { pass.draw_indexed_indirect(&s.commands, k as u64 * 20); }
                }
            }
            if !msaa && edge_only && (self.edges || self.style == 4) {
                for hidden in [true, false] {
                    if hidden && self.style != 1 && self.look.data[6][2] == 0. { continue; }
                    pass.set_pipeline(if hidden { &self.xray_edge } else if self.look.data[6][0] == 0. { &self.hard_edge } else { &self.edge });
                    pass.set_bind_group(0, if hidden { &s.xray_edge_draw } else { &s.edge_draw }, &[]);
                    pass.set_bind_group(1, &self.look_groups[if hidden { 3 } else { 2 }], &[]);
                    pass.set_index_buffer(s.quad_index.slice(..), wgpu::IndexFormat::Uint32);
                    if self.multi { pass.multi_draw_indexed_indirect(&s.commands, s.p as u64 * 20, s.p); }
                    else { for k in s.p..s.p * 2 { pass.draw_indexed_indirect(&s.commands, k as u64 * 20); } }
                }
                if self.style == 1 && self.look.data[6][0] > 0. {
                    pass.set_pipeline(&self.profile_top);
                    pass.set_bind_group(0, &s.profile_draw, &[]);
                    pass.set_bind_group(1, &self.look_groups[4], &[]);
                    pass.set_index_buffer(s.quad_index.slice(..), wgpu::IndexFormat::Uint32);
                    if self.multi { pass.multi_draw_indexed_indirect(&s.commands, s.p as u64 * 20, s.p); }
                    else { for k in s.p..s.p*2 { pass.draw_indexed_indirect(&s.commands, k as u64*20); } }
                }
            }
            drop(pass);
            }
            if msaa && !self.pick_pending {
                self.aa.draw(&mut e, &color, s, self.style, self.edges, self.look.data[6][0] > 0., self.multi, self.queries.as_ref(), Some(edge_only), self.references.active() || self.sections.count > 0 || self.preview.active() || self.group_feedback.as_ref().is_some_and(preview::Preview::active) || self.face_marks.active() || self.ghost_geometry.active() || self.ghost_objects.active() || self.host_context.active() || self.lines.active_layer(true) || (self.look.data[9][0] > 0. && self.material_transparency), &self.axes, &self.references);
            }
            if !edge_only && self.sun.enabled && self.style != 1 && self.style != 4 {
                if msaa { self.aa.resolve_depth(&self.device, &mut e, &depth, None); }
                let output = if msaa { self.aa.color_view() } else { model_color_view(target) };
                self.sun.draw(&self.device, &self.queue, &mut e, &output, &depth, msaa, &self.camera_data,
                              self.queries.as_ref().filter(|_| self.detailed_timestamps));
                sun_drawn = true;
            }
            if !edge_only && ao_active {
                // Caps must contribute depth before AO reconstructs its normals.
                // Submit once here; the non-AO path keeps the final color pass.
                if self.sections.count > 0 {
                    if msaa {
                        self.sections.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color), tq, &self.camera_data);
                    } else {
                        self.sections.draw(&mut e, &color, &depth, None, tq, &self.camera_data);
                    }
                }
                if msaa { self.aa.resolve_depth(&self.device, &mut e, &depth, tq); }
                let output = if msaa && !self.pick_pending { self.aa.color_view() } else { model_color_view(target) };
                self.ao.draw(&self.device, &self.queue, &mut e, target, &depth, &output, msaa && !self.pick_pending, tq);
            }
        }
        if (!msaa || self.pick_pending) && (draw_ids || ao_active) {
            self.pick_read.dirty = false;
            self.pick_read.tiles = pick_tile::Tiles::EMPTY;
        }
        if self.sections.count > 0 && !ao_active {
            if msaa && !self.pick_pending {
                self.sections.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color), tq, &self.camera_data);
            } else {
                self.sections.draw(&mut e, &color, &depth, None, tq, &self.camera_data);
            }
        }
        if msaa {
            self.preview.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color));
            if let Some(group) = &self.group_feedback { group.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color)); }
            self.face_marks.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color));
        } else {
            self.preview.draw(&mut e, &color, &depth, None);
            if let Some(group) = &self.group_feedback { group.draw(&mut e, &color, &depth, None); }
            self.face_marks.draw(&mut e, &color, &depth, None);
        }
        for (enabled,ghost) in [(self.host_context.active(), &self.host_context), (self.look.data[9][2]>0.,&self.ghost_geometry),(self.look.data[9][3]>0.,&self.ghost_objects)] {
            if enabled {
                if msaa { ghost.draw(&mut e,&self.aa.color_view(),&self.aa.depth_view(),Some(&color)); }
                else { ghost.draw(&mut e,&color,&depth,None); }
            }
        }
        // GL's selected hidden edges come after the scene edges.
        if msaa { self.lines.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color), true); }
        else { self.lines.draw(&mut e, &color, &depth, None, true); }
        // Material OIT reads opaque depth and never writes it. AO has already
        // completed on opaque geometry; glass cannot generate contact darkness.
        if self.look.data[9][0] > 0. && self.material_transparency && self.style != 1 && self.style != 4 {
            if msaa { self.aa.resolve_depth(&self.device, &mut e, &depth, None); }
            self.transparency.draw(&mut e, &color, Some(&depth), s, self.multi, tq);
        }
        self.lines.draw_tool(&mut e, &color);
        let color = target.create_view(&Default::default());
        self.overlay
            .draw(&mut e, &color, self.config.width, self.config.height, tq);
        if present && self.ui_target == self.active {
            if let Some(ui) = self.ui.as_mut() {
                ui.paint(&self.device, &self.queue, &mut e, target, &color, [self.config.width, self.config.height], self.ui_scale);
            }
        }
        if let (Some(q), Some(k)) = (&self.queries, timing_slot) {
            e.resolve_query_set(q, 0..if self.detailed_timestamps { timing::COUNT } else { 4 }, &self.resolve, 0);
            e.copy_buffer_to_buffer(&self.resolve, 0, &self.timings[k].read, 0, timing::BYTES as u64);
            e.copy_buffer_to_buffer(&s.counters, 0, &self.timings[k].read, timing::BYTES as u64, 32);
        }
        let stride = (self.config.width * 4).div_ceil(256) * 256;
        let capture = self.capture_next.then(|| {
            let read = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("surface capture"),
                size: stride as u64 * self.config.height as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            e.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(self.config.height),
                    },
                },
                wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
            );
            read
        });
        let command = e.finish();
        let encoded = Instant::now();
        let queue = &self.queue;
        Python::attach(|py| py.detach(|| queue.submit([command])));
        let submitted = Instant::now();
        if let Some(read) = capture {
            let (tx, rx) = std::sync::mpsc::channel();
            read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            self.device.poll(wgpu::Maintain::Wait);
            rx.recv()
                .map_err(|e| PyValueError::new_err(e.to_string()))?
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            let mapped = read.slice(..).get_mapped_range();
            let mut rgba =
                Vec::with_capacity((self.config.width * self.config.height * 4) as usize);
            for row in mapped.chunks_exact(stride as usize) {
                rgba.extend_from_slice(&row[..self.config.width as usize * 4]);
            }
            if matches!(
                self.config.format,
                wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
            ) {
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
            }
            drop(mapped);
            read.unmap();
            self.captured = Some(CapturedFrame { width: self.config.width, height: self.config.height, rgba });
            self.capture_next = false;
        }
        let before_present = Instant::now();
        if let Some(frame) = frame { frame.present(); }
        self.frame_stages = [polled.duration_since(start), acquired.duration_since(polled),
            encoded.duration_since(acquired), submitted.duration_since(encoded), before_present.elapsed()]
            .map(|d| d.as_secs_f64()*1000.);
        let cpu = start.elapsed().as_secs_f64() * 1000.;
        if let (Some(_), Some(k)) = (&self.queries, timing_slot) {
            let (tx, rx) = std::sync::mpsc::channel();
            self.timings[k]
                .read
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |r| {
                    let _ = tx.send(r);
                });
            self.timings[k].pending = Some(rx);
            let mut active = vec!["culling", "overlay", "frame"];
            if background_drawn { active.push("background"); }
            if self.sections.count > 0 { active.push("section_caps"); }
            if !msaa { active.push("faces"); if !msaa && (self.edges || self.style == 4) { active.push("edge_quads"); } }
            if msaa && !self.pick_pending { active.push("antialias"); if ao_active || self.edges || self.style == 4 { active.push("antialias_edges"); } }
            if self.style == 1 { active.extend(["xray_accumulation", "xray_composite"]); }
            if ao_active && msaa { active.push("ao_inputs"); }
            if sun_drawn && self.detailed_timestamps { active.push("sun"); }
            if ao_active { active.extend(["ao_kernel", "ao_blur_x", "ao_blur_y", "ao_upsample_composite"]); }
            self.timings[k].active = active;
        }
        if msaa && needs_settled_ids(false, self.pick_read.dirty, self.stable_frames) { self.ensure_ids()?; }
        self.last_cpu = cpu;
        Ok((cpu, self.last_gpu))
    }
    /// Poll/readback, acquire, encode, submit, present, in milliseconds.
    fn frame_stage_timings(&self) -> Vec<f64> { self.frame_stages.to_vec() }
    fn request_capture(&mut self) {
        self.capture_next = true;
    }
    fn take_capture(&mut self) -> Option<Vec<u8>> {
        self.captured.take().map(|frame| frame.rgba)
    }
    fn save_capture(&mut self, py: Python<'_>, path: std::path::PathBuf) -> PyResult<bool> {
        let Some(frame) = self.captured.take() else { return Ok(false) };
        py.detach(|| frame.write_png(&path))?;
        Ok(true)
    }
    #[pyo3(signature = (center, radius, cursor, scale))]
    fn set_center_cue(&mut self, center: Option<[f32; 3]>, radius: f32, cursor: [f32; 2], scale: f32) {
        self.overlay.center_cue(&self.queue, &self.camera_data,
            [self.config.width, self.config.height], center, radius, cursor, scale);
    }
    fn set_annotation_layer(&mut self, layer: PyRef<'_, annotation_canvas::AnnotationCanvas>) -> PyResult<()> {
        let (w, h) = layer.size();
        if (w, h) != (self.config.width, self.config.height) {
            return Err(PyValueError::new_err("overlay must match physical surface dimensions"));
        }
        let region = layer.drawn_region();
        self.overlay.upload_changed(&self.device, &self.queue, w, h, region, |r| layer.region_rgba(r));
        Ok(())
    }
    fn set_overlay(
        &mut self,
        width: u32,
        height: u32,
        rgba: PyReadonlyArray1<'_, u8>,
    ) -> PyResult<()> {
        let bytes = rgba.as_slice()?;
        if width != self.config.width
            || height != self.config.height
            || bytes.len() as u64 != width as u64 * height as u64 * 4
        {
            return Err(PyValueError::new_err(
                "overlay must match physical surface RGBA dimensions",
            ));
        }
        self.overlay
            .upload(&self.device, &self.queue, width, height, bytes);
        Ok(())
    }
    // ---- The Sheets page (ui/page.rs) -------------------
    /// The sheet as ``built_core.sheet_page_json`` writes it.
    fn page_set(&mut self, snapshot: String) {
        if let Some(ui) = self.ui.as_mut() { ui.page.set(snapshot); }
    }
    /// Let the page go (leaving the workspace).
    fn page_clear(&mut self) {
        if let Some(ui) = self.ui.as_mut() { ui.page.clear(); }
    }
    /// A vector frame's drawing as a display list (JSON, frame-local mm).
    fn page_frame_ops(&mut self, key: u64, ops: &str) -> PyResult<()> {
        let v: serde_json::Value = serde_json::from_str(ops).map_err(|e| PyValueError::new_err(e.to_string()))?;
        if let Some(ui) = self.ui.as_mut() { ui.page.set_ops(key, v); }
        Ok(())
    }
    /// Frame ``key``'s model annotations as a display list, over its content.
    fn page_frame_annots(&mut self, key: u64, ops: &str) -> PyResult<()> {
        let v: serde_json::Value = serde_json::from_str(ops).map_err(|e| PyValueError::new_err(e.to_string()))?;
        if let Some(ui) = self.ui.as_mut() { ui.page.set_annots(key, v); }
        Ok(())
    }
    /// Open the page's text editor on text item ``key``.
    fn page_edit_text(&mut self, key: u64) {
        if let Some(ui) = self.ui.as_mut() { ui.page_edit_text(key); }
    }
    /// What frame ``key`` offers the drawing tools: points (M,2) page mm
    /// with their model points (M,3), edges (E,4) with theirs (E,6).
    #[pyo3(signature = (key, points, world, edges, edge_world, circles=Vec::new()))]
    fn page_frame_snaps(&mut self, key: u64, points: Vec<f64>, world: Vec<f64>, edges: Vec<f64>, edge_world: Vec<f64>,
                        circles: Vec<f64>) {
        let s = crate::sheet_tools::FrameSnaps {
            points: points.chunks_exact(2).map(|c| [c[0], c[1]]).collect(),
            world: world.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
            edges: edges.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
            edge_world: edge_world.chunks_exact(6).map(|c| [[c[0], c[1], c[2]], [c[3], c[4], c[5]]]).collect(),
            circles: circles.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        };
        if let Some(ui) = self.ui.as_mut() { ui.page.snaps.insert(key, s); }
    }
    /// How far apart a baseline run's rows sit (the document's dimension style).
    fn page_set_run_step(&mut self, mm: f64) {
        if let Some(ui) = self.ui.as_mut() { ui.page.run_step = mm; }
    }
    fn page_frame_forget(&mut self, key: u64) {
        if let Some(ui) = self.ui.as_mut() { ui.page.forget_content(key); }
    }
    fn page_has_content(&self, key: u64) -> bool {
        self.ui.as_ref().is_some_and(|ui| ui.page.has_content(key))
    }
    /// A pointer event the chrome left to the page (``ui_pointer`` said
    /// False). Returns the cursor the page wants ("", "move", "grab",
    /// "grabbing", "crosshair", "zoom", "resize_h", "resize_v",
    /// "resize_nwse", "resize_nesw").
    fn page_pointer(&mut self, kind: &str, x: f32, y: f32, button: u8, modifiers: u8) -> String {
        self.ui.as_mut().map(|ui| ui.page_pointer(kind, x, y, button, modifiers).to_string()).unwrap_or_default()
    }
    /// A key the chrome left to the page (arrows nudge, Escape clears).
    fn page_key(&mut self, name: &str, modifiers: u8) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.page_key(name, modifiers))
    }
    /// A sheet command on the page's selection: ``group``, ``ungroup``,
    /// ``copy_style``, ``paste_style`` (its edits come back as actions).
    fn page_command(&mut self, name: &str) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.page_command(name))
    }
    /// Scroll pans; with ``zoom`` it zooms about (x, y) instead.
    fn page_wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32, zoom: bool) {
        if let Some(ui) = self.ui.as_mut() {
            if zoom { ui.page.zoom_about(egui::pos2(x, y), (dy * 0.0025).exp()); } else { ui.page.pan_by(dx, dy); }
        }
    }
    /// Zoom by ``factor`` about (x, y) (a pinch).
    fn page_zoom_by(&mut self, x: f32, y: f32, factor: f32) {
        if let Some(ui) = self.ui.as_mut() { ui.page.zoom_about(egui::pos2(x, y), factor.max(0.05)); }
    }
    fn page_fit(&mut self, width_only: bool) {
        if let Some(ui) = self.ui.as_mut() { ui.page.request_fit(width_only); }
    }
    fn page_zoom_to(&mut self, percent: f32) {
        if let Some(ui) = self.ui.as_mut() { ui.page.request_zoom(percent); }
    }
    /// The zoom in percent of true paper size.
    fn page_zoom(&self) -> f32 { self.ui.as_ref().map_or(100.0, |ui| ui.page.zoom_percent()) }
    /// Where the pointer is on the paper (mm), or None.
    fn page_pointer_mm(&self) -> Option<(f64, f64)> {
        self.ui.as_ref().and_then(|ui| ui.page.pointer_mm).map(|p| (p[0], p[1]))
    }
    fn page_select(&mut self, keys: Vec<u64>) {
        if let Some(ui) = self.ui.as_mut() { ui.page.set_selection(keys); }
    }
    fn page_selection(&self) -> Vec<u64> { self.ui.as_ref().map(|ui| ui.page.selection.clone()).unwrap_or_default() }
    /// A frame of the Sheets workspace: the desk, the page layer, the chrome
    /// over it. No scene pass runs; the resident scene stays as it is.
    #[pyo3(signature = (present=true))]
    fn render_page(&mut self, present: bool) -> PyResult<(f64, Option<f64>)> {
        // Large sheets use the resident model even when opened from the
        // page workspace. Offscreen page exports still draw the page.
        if present && self.ui_target == self.active && self.scene.is_some()
            && self.ui.as_ref().is_some_and(ui::Ui::large_sheet_shown) {
            return self.render(true);
        }
        let start = Instant::now();
        self.device.poll(wgpu::Maintain::Poll);
        let polled = Instant::now();
        let frame = if let Some(surface) = self.surface.as_ref().filter(|_| present) { Some(surface.get_current_texture()
            .map_err(|e| PyValueError::new_err(e.to_string()))?) } else { None };
        let target = frame.as_ref().map(|f| &f.texture).unwrap_or(&self.offscreen);
        let acquired = Instant::now();
        self.last_acquire = acquired.duration_since(polled).as_secs_f64() * 1000.;
        let color = target.create_view(&Default::default());
        let mut e = self.device.create_command_encoder(&Default::default());
        {
            // The desk under the page (ui/page.rs paints the paper over it).
            let _pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("page desk"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.061, g: 0.072, b: 0.088, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        if present && self.ui_target == self.active {
            if let Some(ui) = self.ui.as_mut() {
                ui.paint(&self.device, &self.queue, &mut e, target, &color, [self.config.width, self.config.height], self.ui_scale);
            }
        }
        let stride = (self.config.width * 4).div_ceil(256) * 256;
        let capture = self.capture_next.then(|| {
            let read = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page capture"),
                size: stride as u64 * self.config.height as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            e.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &read, layout: wgpu::TexelCopyBufferLayout {
                    offset: 0, bytes_per_row: Some(stride), rows_per_image: Some(self.config.height) } },
                wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 },
            );
            read
        });
        let encoded = Instant::now();
        self.queue.submit([e.finish()]);
        let submitted = Instant::now();
        if let Some(read) = capture {
            let (tx, rx) = std::sync::mpsc::channel();
            read.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
            self.device.poll(wgpu::Maintain::Wait);
            rx.recv().map_err(|e| PyValueError::new_err(e.to_string()))?
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            let mapped = read.slice(..).get_mapped_range();
            let mut rgba = Vec::with_capacity((self.config.width * self.config.height * 4) as usize);
            for row in mapped.chunks_exact(stride as usize) {
                rgba.extend_from_slice(&row[..self.config.width as usize * 4]);
            }
            if matches!(self.config.format, wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm) {
                for pixel in rgba.chunks_exact_mut(4) { pixel.swap(0, 2); }
            }
            drop(mapped);
            read.unmap();
            self.captured = Some(CapturedFrame { width: self.config.width, height: self.config.height, rgba });
            self.capture_next = false;
        }
        let before_present = Instant::now();
        if let Some(frame) = frame { frame.present(); }
        self.frame_stages = [polled.duration_since(start), acquired.duration_since(polled),
            encoded.duration_since(acquired), submitted.duration_since(encoded), before_present.elapsed()]
            .map(|d| d.as_secs_f64() * 1000.);
        let cpu = start.elapsed().as_secs_f64() * 1000.;
        self.last_cpu = cpu;
        Ok((cpu, None))
    }
    /// The Sheets workspace on (the page replaces the model) or off.
    fn ui_show_sheets(&mut self, shown: bool) {
        if let Some(ui) = self.ui.as_mut() { ui.show_sheets(shown); }
    }
    fn ui_sheets_shown(&self) -> bool { self.ui.as_ref().is_some_and(|ui| ui.sheets_shown()) }
    /// The sheet tools, as ``ui_set_tools``.
    fn ui_set_sheet_tools(&mut self, groups: Vec<Vec<(String, String, String, usize, usize, Vec<u8>)>>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_sheet_tools(groups); }
    }
    fn ui_set_sheet_active(&mut self, id: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_sheet_active(id); }
    }
    /// The sheets: (name, caption, current).
    fn ui_set_sheet_list(&mut self, rows: Vec<(String, String, bool)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_sheet_list(rows); }
    }
    /// The selected item's fields, as ``ui_set_inspector`` (plus the pick,
    /// check, text and button kinds of ui/sheets.rs).
    fn ui_set_sheet_inspector(&mut self, title: String, sections: Vec<(String, Vec<(String, String)>)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_sheet_inspector(title, sections); }
    }
    /// The export buttons: (id, label, enabled).
    fn ui_set_sheet_exports(&mut self, exports: Vec<(String, String, bool)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_sheet_exports(exports); }
    }
    /// The bottom bar: paper, scale, tool, hint, cursor, zoom.
    fn ui_set_sheet_status(&mut self, paper: String, scale: String, tool: String, hint: String, cursor: String, zoom: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_sheet_status(paper, scale, tool, hint, cursor, zoom); }
    }
    /// A question in the dock (an empty title takes it down); ``field`` is
    /// an editable line's text, or None.
    #[pyo3(signature = (title, body, field, buttons))]
    fn ui_set_sheet_prompt(&mut self, title: String, body: String, field: Option<String>, buttons: Vec<(String, String)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_sheet_prompt(title, body, field, buttons); }
    }
    /// Rename sheet ``i`` in place, in the dock.
    fn ui_rename_sheet(&mut self, i: usize) {
        if let Some(ui) = self.ui.as_mut() { ui.rename_sheet(i); }
    }
    /// UI spike: draw egui chrome over the live view from now on.
    #[pyo3(signature = (font=None))]
    fn ui_enable(&mut self, font: Option<String>) -> PyResult<()> {
        let mut ui = ui::Ui::new(&self.device, self.config.format);
        if let Some(path) = font {
            // A folder of Inter faces, or a single font file.
            let dir = std::path::Path::new(&path);
            if dir.is_dir() {
                let read = |n: &str| std::fs::read(dir.join(n)).ok();
                if let Some(regular) = read("Inter-Regular.ttf") {
                    ui.set_fonts(regular, read("Inter-Medium.ttf"), read("Inter-SemiBold.ttf"));
                }
            } else if let Ok(bytes) = std::fs::read(&path) {
                ui.set_fonts(bytes, None, None);
            }
        }
        self.ui = Some(ui);
        self.ui_target = self.active;
        Ok(())
    }
    #[pyo3(signature = (groups, disabled=None))]
    fn ui_set_tools(&mut self, groups: Vec<Vec<(String, String, String, usize, usize, Vec<u8>)>>, disabled: Option<Vec<String>>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_tools(groups, disabled.as_deref().unwrap_or_default()); }
    }
    fn ui_set_active(&mut self, id: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_active(id); }
    }
    #[pyo3(signature=(status, hint, coordinate="", tooltip=""))]
    fn ui_set_status(&mut self, status: String, hint: String, coordinate: &str, tooltip: &str) {
        if let Some(ui) = self.ui.as_mut() { ui.set_status(status, hint, coordinate, tooltip); }
    }
    /// ``kind``: move, press, release, leave; logical points. True when
    /// the chrome takes the event.
    fn ui_pointer(&mut self, kind: &str, x: f32, y: f32, button: u8, modifiers: u8) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.pointer(kind, x, y, button, modifiers))
    }
    /// The value field on the geometry: caption, live reading or what is
    /// typed, its anchor (points), the tool's start on screen and the
    /// document's revision (a commit is read from them, ui/value_field.rs).
    #[pyo3(signature = (caption, value, typing, x, y, start=None, revision=0))]
    fn ui_set_cursor_value(&mut self, caption: String, value: String, typing: bool, x: Option<f32>, y: Option<f32>,
                           start: Option<(f32, f32)>, revision: u64) {
        if let Some(ui) = self.ui.as_mut() { ui.set_cursor_value(caption, value, typing, x, y, start, revision); }
    }
    fn ui_set_selection(&mut self, summary: String, actions: Vec<(String, String, String, bool, bool)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_selection(summary, actions); }
    }
    fn ui_set_icons(&mut self, icons: Vec<(String, usize, usize, Vec<u8>)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_icons(icons); }
    }
    fn ui_set_cluster(&mut self, cluster: Vec<Vec<ui::Item>>, view_menu: Vec<(String, Vec<ui::Item>)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_cluster(cluster, view_menu); }
    }
    /// The active camera for the axis gizmo: yaw and pitch (radians) and
    /// the view label ("Front · Parallel").
    fn ui_set_view_state(&mut self, yaw: f64, pitch: f64, label: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_view_state(yaw, pitch, label); }
    }
    fn ui_input_state(&self) -> std::collections::HashMap<&'static str, bool> {
        self.ui.as_ref().map(ui::Ui::input_state).unwrap_or_default()
    }
    #[pyo3(signature = (name, mods=0))]
    fn ui_claims_key(&self, name: &str, mods: u8) -> bool { self.ui.as_ref().is_some_and(|ui| ui.claims_key(name, mods)) }
    fn ui_key(&mut self, name: &str, pressed: bool, modifiers: u8) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.key(name, pressed, modifiers))
    }
    fn ui_text(&mut self, text: String) -> bool { self.ui.as_mut().is_some_and(|ui| ui.text(text)) }
    /// IME composition for the chrome's focused field (enabled, preedit, commit, disabled).
    fn ui_ime(&mut self, kind: &str, text: String) -> bool { self.ui.as_mut().is_some_and(|ui| ui.ime(kind, text)) }
    /// The focused field's text cursor (points), for the candidate window.
    fn ui_ime_rect(&self) -> Option<(f32, f32, f32, f32)> { self.ui.as_ref().and_then(|ui| ui.ime_rect()) }
    fn ui_area_rect(&self, name: &str) -> Option<(f32, f32, f32, f32)> { self.ui.as_ref().and_then(|ui| ui.area_rect(name)) }
    fn ui_set_options(&mut self, options: Vec<(String, String, String, u8)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_options(options); }
    }
    fn ui_set_tree_group_rows(&mut self, key: String, rows: Vec<(String, String, String)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_tree_group_rows(key, rows); }
    }
    fn ui_set_tree(&mut self, rows: Vec<(Vec<String>, Vec<i64>)>, unfiled: u32) {
        if let Some(ui) = self.ui.as_mut() { ui.set_tree(rows, unfiled); }
    }
    fn ui_profile_png(&mut self) -> PyResult<Vec<u8>> {
        let frame = self.read_pixels()?;
        let panels = &self.ui.as_ref().ok_or_else(|| PyValueError::new_err("No inspector"))?.panels;
        let rect = panels.terrain.plot.ok_or_else(|| PyValueError::new_err("No profile plot"))?;
        let terrain = &panels.data["terrain"];
        let title = format!("{} · {} · Station and elevation (m)", terrain["name"], terrain["source"]);
        inspector::plot_png(&frame, self.config.width as usize, self.config.height as usize, self.ui_scale, rect, &title)
            .map_err(PyValueError::new_err)
    }
    fn ui_panel_set(&mut self, key: &str, snapshot: &str) -> PyResult<()> {
        let value = serde_json::from_str(snapshot).map_err(|e| PyValueError::new_err(e.to_string()))?;
        if key == "selection" { self.preview_parts(Vec::new())?; }
        if let Some(ui) = self.ui.as_mut() { ui.panels.set(key, value); }
        Ok(())
    }
    fn ui_project_hidden(&self) -> bool {
        self.ui.as_ref().is_some_and(|ui| ui.project_hidden())
    }
    fn ui_toggle_review(&mut self) {
        if let Some(ui) = self.ui.as_mut() { ui.toggle_review(); }
    }
    fn ui_show_inspector(&mut self) {
        if let Some(ui) = self.ui.as_mut() { ui.show_inspector(); }
    }
    fn ui_mark_tag_selection(&mut self, scene: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Some(ui) = &mut self.ui else { return Ok(false) };
        let selection = scene.getattr("selection")?;
        let parents = scene.getattr("folder_parents")?;
        let mut tags = std::collections::HashSet::new();
        for item in selection.try_iter()? {
            let item = item?;
            if !item.hasattr("mesh")? { return Ok(false); }
            let tag: Option<String> = item.getattr("layer")?.extract()?;
            tags.insert(tag.unwrap_or_else(|| "Layer 0".into()));
        }
        let mut paths = Vec::new();
        for tag in tags {
            let layer = scene.call_method1("layer_by_key", (&tag,))?;
            let mut path = vec![format!("preview-tag:{tag}")];
            if !layer.is_none() {
                let mut next: Option<String> = layer.getattr("folder")?.extract()?;
                let mut seen = std::collections::HashSet::new();
                while let Some(folder) = next {
                    if !seen.insert(folder.clone()) { break; }
                    path.push(format!("preview-folder:{folder}"));
                    next = parents.call_method1("get", (&folder,))?.extract()?;
                }
            }
            paths.push(path);
        }
        Ok(ui.mark_tag_paths(paths, selection.len()?))
    }
    #[pyo3(signature=(filename, tree, counts, picked=0))]
    fn ui_set_catalog_info(&mut self, filename: String, tree: &Bound<'_,PyAny>, counts: &Bound<'_,PyAny>, picked: usize) -> PyResult<()> {
        if let Some(ui)=self.ui.as_mut() { ui.set_catalog_info(filename,tree,counts)?; ui.set_browser_picks(picked); }
        Ok(())
    }
    #[pyo3(signature = (owner, lines, patterns=None, scene=None))]
    fn ui_set_style_catalog(&mut self, owner: usize, lines: Vec<String>, patterns: Option<&Bound<'_, PyAny>>, scene: Option<&Bound<'_,PyAny>>) -> PyResult<()> {
        if let Some(ui) = self.ui.as_mut() {
            let patterns = if let Some(dict) = patterns.filter(|d| !d.is_empty().unwrap_or(true)) {
                dict.py().import("json")?.call_method1("dumps",(dict,))?.extract()?
            } else { String::new() };
            ui.style_catalog(owner, lines, patterns, scene)?;
        }
        Ok(())
    }
    fn ui_open_line_type(&mut self,name:&str,tags:Vec<String>)->bool{self.ui.as_mut().is_some_and(|ui|ui.open_line_type(name,tags))}
    fn ui_hatch_preference(&mut self,py:Python<'_>,verb:&str,value:&str)->PyResult<()>{
        if let Some(ui)=self.ui.as_mut(){ui.hatch_preference(py,verb,value)?;}
        Ok(())
    }
    fn ui_tick_tag_counts(&mut self, history: &Bound<'_, PyAny>) -> PyResult<bool> {
        match self.ui.as_mut() {
            Some(ui) => ui.tick_tag_counts(history),
            None => Ok(false),
        }
    }
    fn ui_drawing_applied(&mut self, verb:&str, key:&str) { if let Some(ui)=self.ui.as_mut(){ui.drawing_edit_applied(verb,key);} }
    #[pyo3(signature=(name, origin, scene=None, mount=None))]
    fn ui_set_host_context(&mut self, name: String, origin: [f64; 3], scene: Option<&Bound<'_, PyAny>>, mount: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let (edges,tags) = match (scene,mount) {
            (Some(scene),Some(mount)) => host_context::snapshot(scene,mount,origin)?,
            _ => (Vec::new(),Vec::new()),
        };
        self.host_context.upload(&self.device,&[],&edges).map_err(PyValueError::new_err)?;
        if let Some(ui) = self.ui.as_mut() { ui.set_host_context(name,tags); }
        self.stable_frames = 0;
        Ok(())
    }
    fn ui_set_edit_path(&mut self, path: Vec<(String, String)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_edit_path(path); }
    }
    fn ui_set_identity(&mut self, rows: Vec<(String, String, String, Option<bool>)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_identity(rows); }
    }
    fn ui_set_inspector(&mut self, title: String, sections: Vec<(String, Vec<(String, String)>)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_inspector(title, sections); }
    }
    fn ui_start_filing(&mut self, what: String) {
        if let Some(ui) = self.ui.as_mut() { ui.start_filing(what); }
    }
    fn ui_wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.wheel(x, y, dx, dy))
    }
    fn ui_set_delivery(&mut self, active: String, unfiled: u32, problems: u32) {
        if let Some(ui) = self.ui.as_mut() { ui.set_delivery(active, unfiled, problems); }
    }
    fn ui_set_tabs(&mut self,py:Python<'_>,tabs:Vec<Vec<ui::Item>>)->PyResult<()>{
        if let Some(ui)=self.ui.as_mut(){ui.scene_photo_mode(py)?;ui.set_tabs(tabs);}
        Ok(())
    }
    fn ui_finish_scene_photo(&mut self,key:&str,pixels:Option<scene_photo::Pixels>){
        if let Some(ui)=self.ui.as_mut(){ui.finish_scene_photo(key,pixels);}
    }
    /// The window's documents as this renderer's chrome was last sent them
    /// (``DocumentTabs.sync``, document_tabs.rs): one per renderer, so the
    /// comparison is always with what it shows.
    fn document_tabs(&mut self, py: Python<'_>) -> PyResult<Py<render_glue::document_tabs::DocumentTabs>> {
        if self.document_tabs.is_none() {
            self.document_tabs = Some(Py::new(py, render_glue::document_tabs::DocumentTabs::default())?);
        }
        Ok(self.document_tabs.as_ref().unwrap().clone_ref(py))
    }
    fn ui_set_projects(&mut self, left: f32, docs: Vec<(String, bool, bool)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_projects(left, docs); }
    }
    /// The free model area in device coordinates (`Ui::free_ndc`): what
    /// Zoom Extents, Zoom Selection and Review frame into.
    fn ui_free_ndc(&self) -> Option<[f64; 4]> { self.ui.as_ref().and_then(|ui| ui.free_ndc()) }
    fn ui_top_empty(&self, x: f32, y: f32) -> bool { self.ui.as_ref().is_some_and(|ui| ui.top_empty(x, y)) }
    /// The project tabs (views/wgpu_tabs.py, ui/tabs.rs): (id, name, front,
    /// unsaved, link source, tip, picture key), the projects in other
    /// windows (id, name, window), and the tab menu's two switches.
    #[pyo3(signature = (tabs, elsewhere=Vec::new(), can_move=false, pictures=true))]
    fn ui_set_project_tabs(&mut self, tabs: Vec<(String, String, bool, bool, bool, String, String)>,
                           elsewhere: Vec<(String, String, String)>, can_move: bool, pictures: bool) {
        if let Some(ui) = self.ui.as_mut() { ui.set_project_tabs(&self.device, &self.queue, tabs, elsewhere, can_move, pictures); }
    }
    fn ui_cluster_buttons(&self) -> Vec<(String, (f32, f32))> {
        self.ui.as_ref().map_or_else(Vec::new, |ui| ui.cluster_buttons.clone())
    }
    fn ui_project_tab_rects(&self) -> Vec<Option<(f32, f32, f32, f32)>> {
        self.ui.as_ref().map_or_else(Vec::new, |ui| ui.project_tab_rects())
    }
    /// The right-click menu at (x, y) (viewport_menu.rs): header, then
    /// rows of (family label, [(id, label, key, enabled, checked)]).
    fn ui_set_context_menu(&mut self, x: f32, y: f32, title: String, subtitle: String,
                           rows: Vec<(String, Vec<(String, String, String, bool, i8)>)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_context_menu(x, y, title, subtitle, rows); }
    }
    fn ui_close_context_menu(&mut self) {
        if let Some(ui) = self.ui.as_mut() { ui.close_context_menu(); }
    }
    fn ui_context_menu_open(&self) -> bool { self.ui.as_ref().is_some_and(|ui| ui.context_menu_open()) }
    fn ui_context_menu_rows(&self) -> Vec<String> {
        self.ui.as_ref().map_or_else(Vec::new, |ui| ui.context_menu_rows())
    }
    fn ui_over_chrome(&self, x: f32, y: f32) -> bool { self.ui.as_ref().is_some_and(|ui| ui.over_chrome_at(x, y)) }
    fn ui_set_units(&mut self, units: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_units(units); }
    }
    fn ui_set_kinds(&mut self, kinds: Vec<String>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_kinds(kinds); }
    }
    fn ui_set_prompt(&mut self, title: String, body: String, buttons: Vec<(String, String)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_prompt(title, body, buttons); }
    }
    /// The app sheet (an empty title takes it down): tone, and buttons as
    /// (id, label, key hint, role: 0 plain, 1 default, 2 discard, 3 cancel).
    fn ui_set_sheet(&mut self, title: String, body: String, tone: String, buttons: Vec<(String, String, String, u8)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_sheet(title, body, tone, buttons); }
    }
    fn ui_extension_menu(&mut self, rows: Vec<extension_menu::Row>) {
        if let Some(ui) = self.ui.as_mut() { ui.extension_menu(rows); }
    }
    fn ui_plugin_panel(&mut self, raw: &str) -> PyResult<()> {
        if let Some(ui) = self.ui.as_mut() {
            ui.plugin_panel(raw).map_err(pyo3::exceptions::PyValueError::new_err)?;
        }
        Ok(())
    }
    fn ui_plugin_open(&mut self, instance: &str) {
        if let Some(ui) = self.ui.as_mut() { ui.plugin_open(instance); }
    }
    fn ui_plugin_visible(&self, instance: &str) -> bool {
        self.ui.as_ref().is_some_and(|ui| ui.plugin_visible(instance))
    }
    fn ui_plugin_document(&mut self, document: String, live: Vec<String>) {
        if let Some(ui) = self.ui.as_mut() { ui.plugin_document(document, &live); }
    }
    /// Work in progress: (id, what, phase, fraction or -1, cancellable).
    fn ui_set_jobs(&mut self, jobs: Vec<ui::Job>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_jobs(jobs); }
    }
    fn ui_open_activity(&mut self) {
        if let Some(ui) = self.ui.as_mut() { ui.open_activity(); }
    }
    #[pyo3(signature=(history, banner, secondary=None))]
    fn ui_set_activity(&mut self, history: Vec<ui::ActivityRow>, banner: String, secondary: Option<std::collections::HashMap<String,String>>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_activity(history, banner, secondary.unwrap_or_default()); }
    }
    fn ui_rename_browser_row(&mut self, id: &str) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.rename_browser_row(id))
    }
    fn ui_reveal_row(&mut self, key: String) {
        if let Some(ui) = self.ui.as_mut() { ui.reveal_row(key); }
    }
    fn ui_set_proposal_available(&mut self, available: bool) {
        if let Some(ui) = self.ui.as_mut() { ui.set_proposal_available(available); }
    }
    fn ui_set_tree_sort(&mut self, sort: usize) {
        if let Some(ui) = self.ui.as_mut() { ui.set_tree_sort(sort); }
    }
    #[pyo3(signature=(node, placements))]
    fn ui_set_export_focus(&mut self, node: Option<String>, placements: Vec<Vec<String>>) {
        if let Some(ui)=self.ui.as_mut() {ui.set_export_focus(node,placements);}
    }
    fn ui_step_issue(&mut self, step: i32) { if let Some(ui) = self.ui.as_mut() { ui.step_issue(step); } }
    fn ui_set_lens(&mut self, lens: u8) {
        if let Some(ui) = self.ui.as_mut() { ui.set_lens(lens); }
    }
    fn ui_set_dock_width(&mut self, width: f32) {
        if let Some(ui) = self.ui.as_mut() { ui.set_dock_width(width); }
    }
    fn ui_cursor_shape(&self) -> String {
        self.ui.as_ref().map_or(String::new(), |ui| ui.cursor_shape().to_owned())
    }
    fn ui_set_mode(&mut self, mode: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_mode(mode); }
    }
    fn ui_set_issues(&mut self, on: bool) {
        if let Some(ui) = self.ui.as_mut() { ui.set_issues(on); }
    }
    fn ui_set_palette(&mut self, items: Vec<(String, String, String, String, bool)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_palette(items); }
    }
    /// A modal form (rust/built_render/src/forms_host.rs); an empty `seq` closes it.
    #[pyo3(signature = (seq, title="".to_string(), note="".to_string(), width=420.0, fields=Vec::new(), buttons=Vec::new()))]
    fn ui_set_dialog(&mut self, seq: String, title: String, note: String, width: f32,
                     fields: Vec<ui::dialog::Field>, buttons: Vec<ui::dialog::Button>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_dialog(seq, title, note, width, fields, buttons); }
    }
    fn ui_set_soften_preview(&mut self, lines: Vec<([f32; 2], [f32; 2])>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_soften_preview(lines); }
    }
    #[pyo3(signature=(seq, marks, matrix, width, height, origin=(0.0, 0.0)))]
    fn ui_set_join_marks(&mut self, seq: &str, marks: PyRef<'_, edge_commands::JoinMarks>, matrix: [f64; 16], width: f64, height: f64, origin: (f64, f64)) {
        if let Some(ui) = self.ui.as_mut() { ui.set_join_marks(seq, marks.project(matrix, width, height, origin)); }
    }
    fn ui_dialog_open(&self) -> bool { self.ui.as_ref().is_some_and(|ui| ui.dialog_open()) }
    fn ui_font_picker(&mut self, draft: &str) -> PyResult<()> {
        let choice = serde_json::from_str(draft).map_err(|e| PyValueError::new_err(e.to_string()))?;
        if let Some(ui) = &mut self.ui { ui.font_picker = Some(ui::kit::FontPicker::new(choice)); }
        Ok(())
    }
    fn ui_set_form(&mut self, title: String, fields: Vec<ui::FormField>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_form(title, fields); }
    }
    /// The large sheet (sheet_runtime.rs): the whole sheet as JSON
    /// (ui/large_sheet.rs `Spec`); "" closes it.
    fn ui_set_large_sheet(&mut self, spec: &str) -> PyResult<()> {
        match self.ui.as_mut() {
            Some(ui) => ui.set_large_sheet(spec).map_err(pyo3::exceptions::PyValueError::new_err),
            None => Ok(()),
        }
    }
    fn ui_large_sheet_shown(&self) -> bool { self.ui.as_ref().is_some_and(|ui| ui.large_sheet_shown()) }
    /// (sheet id, focused item, search text) as the chrome holds them.
    fn ui_large_sheet_state(&self) -> (String, String, String) {
        self.ui.as_ref().map(|ui| ui.large_sheet_state()).unwrap_or_default()
    }
    /// The sheet's rectangle as last drawn (points), for captures.
    fn ui_large_sheet_rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.ui.as_ref().and_then(|ui| ui.large_sheet_rect())
    }
    #[pyo3(signature = (text=String::new(), scope=String::new()))]
    fn ui_open_commands(&mut self, text: String, scope: String) {
        if let Some(ui) = self.ui.as_mut() { ui.start_commands(text, scope); }
    }
    fn ui_organise_scenes(&mut self) {
        if let Some(ui) = self.ui.as_mut() { ui.organise_scenes(); }
    }
    fn ui_set_command_details(&mut self, details: Vec<(String, bool, bool)>) {
        if let Some(ui) = self.ui.as_mut() { ui.command_details(details); }
    }
    fn ui_set_command_results(&mut self, results: Vec<(String, String, String, bool)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_command_results(results); }
    }
    fn ui_take_actions(&mut self) -> Vec<String> {
        self.ui.as_mut().map(|ui| ui.take_actions()).unwrap_or_default()
    }
    fn ui_repaint_after_ms(&self) -> Option<f64> { self.ui.as_ref().and_then(|ui| ui.repaint_after_ms()) }
    fn ui_frame_ms(&self) -> f64 { self.ui.as_ref().map_or(0., |ui| ui.frame_ms) }
    fn last_timings(&self) -> (f64, Option<f64>, f64, std::collections::BTreeMap<String, Option<f64>>, std::collections::BTreeMap<String, u64>) {
        let mut counters = self.last_counters.clone();
        counters.insert("texture_layers".into(), self.textures.layers as u64);
        counters.insert("instances_input".into(), self.scene.as_ref().map_or(0, |s| s.n as u64));
        // The resident geometry all prototypes share: 28 bytes a vertex, 4 an index.
        counters.insert("vertices_resident".into(), self.scene.as_ref().map_or(0, |s| s.vertex_bytes as u64 / 28));
        counters.insert("indices_resident".into(), self.scene.as_ref().map_or(0, |s| s.index_data.len() as u64 / 4));
        counters.insert("prototypes".into(), self.scene.as_ref().map_or(0, |s| s.p as u64));
        counters.insert("depth_prepass".into(), 0);
        let mut passes: std::collections::BTreeMap<_, _> = self.pass_gpu.iter().map(|(n,v)| (n.clone(), Some(*v))).collect();
        passes.insert("id".into(), self.last_pick_gpu);
        for name in ["profile_top", "profile_classification", "present"] {
            passes.insert(name.into(), None);
        }
        passes.insert("depth_prepass".into(), Some(0.));
        (self.last_cpu, self.last_gpu, self.last_acquire, passes, counters)
    }
    fn ensure_ids(&mut self) -> PyResult<()> { self.ensure_ids_region(None) }
    fn ensure_ids_region(&mut self, cursor: Option<(u32, u32)>) -> PyResult<()> {
        if !self.pick_read.dirty { return Ok(()); }
        self.pick_read.pixel = None;
        // Wireframe IDs include edge-only geometry; retain its existing semantics.
        if self.style == 4 {
            self.pick_pending = true;
            let result = self.render(false);
            self.pick_pending = false;
            result?;
        } else {
            let s = self.scene.as_ref().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&s.hard_template, 0, &s.commands, 0, s.p as u64 * 40);
            let cursor_group = cursor.filter(|_| self.camera_data.len() == 28).map(|(x,y)| {
                let mut camera = self.camera_data.clone();
                let margin = self.look.data[2][3].max(self.look.data[6][1]).max(1.);
                camera[24..28].copy_from_slice(&pick_bounds(x, y, self.config.width, self.config.height, margin));
                let camera = buffer(&self.device, bytemuck::cast_slice(&camera), wgpu::BufferUsages::UNIFORM);
                bind_prototypes(&self.device, &self.compute.get_bind_group_layout(0),
                    &[&camera, &s.inst, &s.protos, &s.commands, &s.survivors, &s.viewpoints, &s.counters], s.p)
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(if cursor_group.is_some() { &self.compute_pick } else { &self.compute });
                pass.set_bind_group(0, cursor_group.as_ref().unwrap_or(&s.compute), &[]);
                pass.dispatch_workgroups(s.n.div_ceil(64), 1, 1);
            }
            let color = model_color_view(&self.offscreen);
            let ids = self.ids.create_view(&Default::default());
            let depth = self.depth.create_view(&Default::default());
            {
                // Depth-only color writes retain texture alpha rejection, but no
                // background, edges, AO, multisampling, overlay or presentation.
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pick IDs"),
                    color_attachments: &[
                        Some(wgpu::RenderPassColorAttachment { view: &color, resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } }),
                        Some(wgpu::RenderPassColorAttachment { view: &ids, resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } }),
                    ],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth, stencil_ops: None,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(depth::CLEAR), store: wgpu::StoreOp::Store }),
                    }),
                    timestamp_writes: timing::writes(self.queries.as_ref(), "id"), occlusion_query_set: None,
                });
                if let Some((x, y)) = cursor { pass.set_scissor_rect(x, y, 1, 1); }
                pass.set_pipeline(&self.xray);
                pass.set_bind_group(0, &s.xray_draw, &[]);
                pass.set_bind_group(1, &self.look_groups[1], &[]);
                pass.set_bind_group(2, &self.texture_groups[1], &[]);
                pass.set_vertex_buffer(0, s.vertex.slice(..));
                pass.set_index_buffer(s.index.slice(..), wgpu::IndexFormat::Uint32);
                if self.multi { pass.multi_draw_indexed_indirect(&s.commands, 0, s.p); }
                else { for k in 0..s.p { pass.draw_indexed_indirect(&s.commands, k as u64 * 20); } }
                // Loose line prototypes have no faces to identify. Keep those
                // pickable without replaying the edges of shaded prototypes.
                if self.edges {
                    pass.set_pipeline(&self.edge);
                    pass.set_bind_group(0, &s.edge_draw, &[]);
                    pass.set_bind_group(1, &self.look_groups[2], &[]);
                    pass.set_index_buffer(s.quad_index.slice(..), wgpu::IndexFormat::Uint32);
                    for k in 0..s.p as usize {
                        if s.template_data[k*20..k*20+4] == [0; 4] {
                            pass.draw_indexed_indirect(&s.commands, (s.p as usize+k) as u64 * 20);
                        }
                    }
                }
            }
            self.queue.submit(Some(encoder.finish()));
            self.pick_read.dirty = !ids_complete(cursor);
        }
        Ok(())
    }
    /// Visible IDs in a physical-pixel, half-open rectangle; explicit picking may wait.
    fn pick_rect(&mut self, x0: u32, y0: u32, x1: u32, y1: u32) -> PyResult<Vec<u32>> {
        let x = x0.min(x1).min(self.config.width);
        let y = y0.min(y1).min(self.config.height);
        let w = x0.max(x1).min(self.config.width) - x;
        let h = y0.max(y1).min(self.config.height) - y;
        if w == 0 || h == 0 {
            return Ok(vec![]);
        }
        self.ensure_ids()?;
        let stride = (w * 4).div_ceil(256) * 256;
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rectangle pick"),
            size: stride as u64 * h as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut e = self.device.create_command_encoder(&Default::default());
        e.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ids,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([e.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| PyValueError::new_err(e.to_string()))?
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let bytes = read.slice(..).get_mapped_range();
        let mut ids = std::collections::BTreeSet::new();
        for row in bytes.chunks_exact(stride as usize) {
            for b in row[..w as usize * 4].chunks_exact(4) {
                let id = u32::from_le_bytes(b.try_into().unwrap());
                if id != 0 {
                    ids.insert(id - 1);
                }
            }
        }
        drop(bytes);
        read.unmap();
        Ok(ids.into_iter().collect())
    }
    /// ``set_profile_exclusions`` touches only the prototypes it is given
    /// and keeps the result (older modules rebuilt every prototype from the
    /// full list, so callers had to send all of it).
    #[getter]
    fn incremental_profile_exclusions(&self) -> bool { true }

    #[getter]
    fn gpu_memory(&self) -> u64 {
        self.scene.as_ref().map_or(0, |s| {
            [&s.vertex, &s.index, &s.edge_records, &s.inst, &s.protos,
             &s.survivors, &s.viewpoints, &s.commands, &s.template, &s.hard_template, &s.quad_index]
                .iter().map(|b| b.size()).sum::<u64>() + 112 + 416
        }) + self.textures.bytes + self.textures.materials.size()
    }

    #[getter]
    fn pick_cache_valid(&self) -> bool { !self.pick_read.dirty }

    fn pick(&mut self, x: u32, y: u32) -> PyResult<i64> {
        if x >= self.config.width || y >= self.config.height {
            return Ok(-1);
        }
        if let Some((px,py,row)) = self.pick_read.pixel {
            if (px,py) == (x,y) { return Ok(row); }
        }
        if let Some(row) = self.pick_read.tiles.get(x, y) { return Ok(row); }
        let measure_id = self.pick_read.dirty && self.style != 4 && self.queries.is_some();
        self.ensure_ids_region(Some((x, y)))?;
        // A scissored pass only produced the requested pixel. A settled ID
        // texture can supply adjacent hits without another GPU synchronization.
        let plan = PassPlan::new(self.antialias && self.samples.count() > 1, self.style, self.look.data[6][2] != 0., self.ao.active(self.style));
        let tile = !self.pick_read.dirty && !measure_id && (plan.msaa || plan.color_only(self.style, self.edges));
        let region = if tile {
            pick_tile::Region::around(x, y, self.config.width, self.config.height)
        } else {
            pick_tile::Region { x, y, width: 1, height: 1 }
        };
        let mut e = self.device.create_command_encoder(&Default::default());
        e.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ids,
                mip_level: 0,
                origin: wgpu::Origin3d { x: region.x, y: region.y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(region.stride()),
                    rows_per_image: Some(region.height),
                },
            },
            wgpu::Extent3d {
                width: region.width,
                height: region.height,
                depth_or_array_layers: 1,
            },
        );
        if measure_id {
            let first = timing::index("id");
            e.resolve_query_set(self.queries.as_ref().unwrap(), first..first+2, &self.resolve, 0);
            e.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 8, 16);
        }
        self.queue.submit([e.finish()]);
        let bytes = if tile { u64::from(region.bytes()) } else if measure_id { 24 } else { 4 };
        let b = self.readback(bytes)?;
        self.last_pick_gpu = measure_id.then(|| {
            let first = u64::from_le_bytes(b[8..16].try_into().unwrap());
            let last = u64::from_le_bytes(b[16..24].try_into().unwrap());
            last.saturating_sub(first) as f64 * self.queue.get_timestamp_period() as f64 / 1e6
        });
        let row = region.row(&b, x, y);
        if tile { self.pick_read.tiles.insert(region, b); }
        self.pick_read.pixel = Some((x,y,row));
        Ok(row)
    }
}
impl Renderer {
    fn push_target(&mut self, surface: Option<wgpu::Surface<'static>>, width: u32, height: u32, scale: f64) -> PyResult<usize> {
        let w = ((width as f64 * scale) as u32).max(1);
        let h = ((height as f64 * scale) as u32).max(1);
        let mut config = self.config.clone();
        config.width = w;
        config.height = h;
        if let Some(surface) = &surface { surface.configure(&self.device, &config); }
        let format = config.format.remove_srgb_suffix();
        let d = &self.device;
        let mut aa = antialias::Antialias::new(self.samples.count(), d, format, &self.look.buffer, w, h);
        aa.bind_textures(d, &self.textures);
        let mut transparency = xray::Xray::new(d, format, w, h);
        transparency.bind_textures(d, &self.textures);
        if let Some(s) = &self.scene {
            aa.bind_scene(d, &self.camera, &s.inst, &s.survivors, &self.look.buffer, &s.edge_records, &s.viewpoints);
            transparency.bind_scene(d, &self.camera, &s.inst, &s.survivors, &self.look.buffer);
        }
        let target = Target {
            surface, config,
            depth: texture(d, w, h, wgpu::TextureFormat::Depth32Float),
            offscreen: texture(d, w, h, self.config.format),
            ids: texture(d, w, h, wgpu::TextureFormat::R32Uint),
            aa, transparency,
            ao: ao::AmbientOcclusion::new(self.samples.clone(), d, format),
            overlay: overlay::Overlay::new(d, self.config.format),
            axes: axes::Axes::new(self.samples.clone(), d, format, &self.camera),
            lines: lines::Lines::new(self.samples.clone(), d, format, &self.camera),
            preview: preview::Preview::new(self.samples.clone(), d, format, &self.camera, &self.look.buffer, 0),
            group_feedback: None,
            face_marks: preview::Preview::new(self.samples.clone(), d, format, &self.camera, &self.look.buffer, 1),
            sections: sections::Sections::new(self.samples.clone(), d, format, &self.camera),
            camera_data: Vec::new(),
            look_data: self.look.data,
            sun: self.sun.settings(),
            style: self.style,
            edges: self.edges,
            textures_enabled: self.textures_enabled,
            pick_read: PickRead::DIRTY,
            stable_frames: 0,
            pick_pending: false,
            capture_next: false,
            captured: None,
            seen_epoch: self.scene_epoch,
        };
        let id = self.next_target;
        self.targets.insert(id, target);
        self.next_target += 1;
        Ok(id)
    }

    fn refresh_msaa(&mut self) {
        let count = self.samples.count();
        if self.aa.samples == count {
            return;
        }
        let mut aa = antialias::Antialias::new(
            count,
            &self.device,
            self.config.format.remove_srgb_suffix(),
            &self.look.buffer,
            self.config.width,
            self.config.height,
        );
        aa.bind_textures(&self.device, &self.textures);
        if let Some(s) = &self.scene {
            aa.bind_scene(
                &self.device,
                &self.camera,
                &s.inst,
                &s.survivors,
                &self.look.buffer,
                &s.edge_records,
                &s.viewpoints,
            );
        }
        self.aa = aa;
    }
    fn swap_live(&mut self, t: &mut Target) {
        use std::mem::swap;
        swap(&mut self.surface, &mut t.surface);
        swap(&mut self.config, &mut t.config);
        swap(&mut self.depth, &mut t.depth);
        swap(&mut self.offscreen, &mut t.offscreen);
        swap(&mut self.ids, &mut t.ids);
        swap(&mut self.aa, &mut t.aa);
        swap(&mut self.transparency, &mut t.transparency);
        swap(&mut self.ao, &mut t.ao);
        swap(&mut self.overlay, &mut t.overlay);
        swap(&mut self.axes, &mut t.axes);
        swap(&mut self.lines, &mut t.lines);
        swap(&mut self.preview, &mut t.preview);
        swap(&mut self.face_marks, &mut t.face_marks);
        swap(&mut self.group_feedback, &mut t.group_feedback);
        swap(&mut self.sections, &mut t.sections);
        swap(&mut self.camera_data, &mut t.camera_data);
        swap(&mut self.look.data, &mut t.look_data);
        swap(&mut self.style, &mut t.style);
        swap(&mut self.edges, &mut t.edges);
        swap(&mut self.textures_enabled, &mut t.textures_enabled);
        swap(&mut self.pick_read, &mut t.pick_read);
        swap(&mut self.stable_frames, &mut t.stable_frames);
        swap(&mut self.pick_pending, &mut t.pick_pending);
        swap(&mut self.capture_next, &mut t.capture_next);
        swap(&mut self.captured, &mut t.captured);
        let sun = self.sun.settings();
        self.sun.restore(t.sun);
        t.sun = sun;
    }
    fn readback(&self, n: u64) -> PyResult<Vec<u8>> {
        let slice = self.read.slice(..n);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .unwrap()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let bytes = slice.get_mapped_range().to_vec();
        self.read.unmap();
        Ok(bytes)
    }
}
/// Public so the `built` app can register the module statically
/// (`append_to_inittab`) and share one renderer with the embedded Python.
/// Check a large sheet's JSON against the chrome's schema without a
/// renderer (sheet_runtime.rs tests): "" when it parses, else why.
#[pyfunction]
fn large_sheet_check(spec: &str) -> String {
    match serde_json::from_str::<ui::large_sheet::Spec>(spec) {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

mod surfaces;
mod component_catalogue;
mod image_io;
mod extensions;
mod release;
mod units_start;
mod preferences;
mod about;
mod sheet_host;
mod sheet_workspace;
mod sheet_frames;
mod start_view;
mod library_view;
mod resident_rows;
mod scene_photo;
mod shortcut_text;
/// The Sheets workspace's page, native.
pub mod sheet_page;
pub mod sheet_tools;
mod sheet_search;
mod camera_inspector;
mod registry_editor;
mod render_result;
mod font_lengths;
mod svg_options;
mod tag_counts;
/// Sheet items' display lists on the page.
pub mod sheet_display;
/// An image item's picture as the sheet shows it, shared with the writers.
#[path = "../../built_core/src/text_search.rs"]
mod text_search;

#[path = "../../built_core/src/sheets/picture.rs"]
#[allow(dead_code)]
pub mod sheet_picture;
/// The Sheets dock's inspector, from the page snapshot.
pub mod sheet_inspector;

/// Start an orthographic ray on the plane through the camera, aligned with the point.
#[pyfunction]
fn parallel_annotation_ray(eye: [f64; 3], forward: [f64; 3], point: [f64; 3]) -> Option<([f64; 3], f64)> {
    let point = glam::DVec3::from(point);
    let forward = glam::DVec3::from(forward).normalize_or_zero();
    let depth = (point - glam::DVec3::from(eye)).dot(forward);
    // Keep the same millimetre tolerance as the perspective ray on the endpoint's face.
    (depth > 1e-3).then(|| ((point - forward * depth).to_array(), depth - 1e-3))
}

/// Document state comes from the controller. Headless views have no dock to unwind.
#[pyfunction(signature = (busy, selected, editing, renderer=None, array_pending=false, typed=false, constrained=false))]
fn model_escape(busy: bool, selected: bool, editing: bool, mut renderer: Option<PyRefMut<'_, Renderer>>, array_pending: bool, typed: bool, constrained: bool) -> &'static str {
    if typed { return "value"; }
    if constrained { return "constraint"; }
    ui::Ui::model_escape(renderer.as_mut().and_then(|r| r.ui.as_mut()), busy, array_pending, selected, editing)
}

/// A context click on a selected item addresses its selection. A miss addresses the background.
#[pyfunction]
fn context_targets(hit: bool, selection: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
    if !hit { return Ok(Vec::new()); }
    selection.try_iter()?.map(|item| item.map(Bound::unbind)).collect()
}

#[pymodule]
pub fn built_render(m: &Bound<'_, PyModule>) -> PyResult<()> {
    document_registration::register_interaction(m)?;
    document_terrain::register(m)?;
    m.add_class::<tool_instances::InstancePreview>()?;
    window_document::register(m)?;
    close_documents::register(m)?;
    m.add_class::<component_rows::ComponentRows>()?;
    m.add_function(wrap_pyfunction!(context_placements::pick_candidates, m)?)?;
    section_display::register(m)?;
    m.add_function(wrap_pyfunction!(context_placements::context_placements, m)?)?;
    m.add_function(wrap_pyfunction!(context_pick::context_pick, m)?)?;
    m.add_function(wrap_pyfunction!(component_rows::select_component_instances, m)?)?;
    component_replacement::register(m)?;
    inspection_edits::register(m)?;
    context_edits::register(m)?;
    m.add_function(wrap_pyfunction!(ao::ao_settings,m)?)?;
    tree_grouping::navigation::register(m)?;
    m.add("VideoError", m.py().get_type::<shell_window::video::VideoError>())?;
    m.add_function(wrap_pyfunction!(shell_window::video::export_video,m)?)?;
    status_hints::register(m)?;
    command_metadata::register(m)?;
    native_view_actions::register(m)?;
    plugin_workspace::register(m)?;
    m.add_class::<link_watch::LinkWatch>()?;
    context_tools::register(m)?;
    image_edits::register(m)?;
    component_files::register(m)?;
    model_panels::register_qt_host(m)?;
    m.add_class::<dwg_process::DwgConverter>()?;
    edge_commands::register(m)?;
    face_commands::register(m)?;
    file_converters::register(m)?;
    file_import::register(m)?;
    file_placement::register(m)?;
    linked_files::register(m)?;
    model_edit::register(m)?;
    profile_commands::register(m)?;
    recovery_commands::register(m)?;
    model_visibility::register(m)?;
    m.add_class::<model_clipboard::ModelClipboard>()?;
    group_edit::register(m)?;
    document_starter::register(m)?;
    scene_navigation::register(m)?;
    m.add_function(wrap_pyfunction!(background_autosave::background_autosave, m)?)?;
    window_services::register(m)?;
    m.add_function(wrap_pyfunction!(tool_cursors::rail_icon,m)?)?;
    render_glue::register_render_glue(m)?;
    m.add_function(wrap_pyfunction!(tool_cursors::tool_cursor_name, m)?)?;
    m.add_function(wrap_pyfunction!(tool_cursors::tool_cursor_art, m)?)?;
    m.add_function(wrap_pyfunction!(image_assets::texture_has_alpha,m)?)?;
    m.add_function(wrap_pyfunction!(image_assets::texture_cutout,m)?)?;
    m.add_function(wrap_pyfunction!(image_assets::readable_tga,m)?)?;
    m.add_function(wrap_pyfunction!(survey_import::read_survey_mesh,m)?)?;
    m.add_function(wrap_pyfunction!(survey_import::survey_mesh_datum,m)?)?;
    m.add_function(wrap_pyfunction!(survey_import::place_survey_mesh,m)?)?;
    m.add_function(wrap_pyfunction!(survey_import::place_alignment,m)?)?;
    m.add_function(wrap_pyfunction!(survey_import::place_survey_points,m)?)?;
    m.add_function(wrap_pyfunction!(captured_frame::blank_picture,m)?)?;
    m.add_function(wrap_pyfunction!(captured_frame::archive_thumbnail,m)?)?;
    m.add_function(wrap_pyfunction!(captured_frame::archive_picture_png,m)?)?;
    m.add_class::<model_views::ModelViews>()?;
    m.add_class::<model_views::DocumentViews>()?;
    m.add_class::<extension_menu::ExtensionMenus>()?;
    m.add_function(wrap_pyfunction!(extension_setup::setup_extension_menus,m)?)?;
    m.add_function(wrap_pyfunction!(model_views::view_layouts, m)?)?;
    m.add_class::<annotation_canvas::AnnotationCanvas>()?;
    m.add_class::<annotation_canvas::AnnotationFont>()?;
    m.add_class::<annotation_canvas::Color>()?;
    m.add_class::<annotation_canvas::NativePen>()?;
    m.add_function(wrap_pyfunction!(tool_overlay::tool_overlay_tile,m)?)?;
    m.add_function(wrap_pyfunction!(image_assets::reference_texture, m)?)?;
    m.add_function(wrap_pyfunction!(image_assets::fit_reference_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(image_assets::swatch_pixels, m)?)?;
    m.add_function(wrap_pyfunction!(image_assets::svg_preview, m)?)?;
    m.add_class::<survey_raster::SurveyRaster>()?;
    m.add_function(wrap_pyfunction!(image_assets::raster_dimensions, m)?)?;
    m.add_function(wrap_pyfunction!(material_textures::material_texture_array, m)?)?;
    navigation::register(m)?;
    m.add_function(wrap_pyfunction!(preferences::msaa_levels,m)?)?;
    m.add_function(wrap_pyfunction!(preferences::apply_msaa,m)?)?;
    m.add_function(wrap_pyfunction!(commands::view_commands, m)?)?;
    m.add_function(wrap_pyfunction!(commands::run_view_command, m)?)?;
    m.add_function(wrap_pyfunction!(commands::command_chrome, m)?)?;
    m.add_function(wrap_pyfunction!(commands::activate_view_command, m)?)?;
    m.add_function(wrap_pyfunction!(commands::rail_layout, m)?)?;
    verbs::register(m)?;
    m.add_function(wrap_pyfunction!(ui::tokens::chrome_token, m)?)?;
    m.add_function(wrap_pyfunction!(commands::leave_navigation, m)?)?;
    m.add_function(wrap_pyfunction!(commands::view_shortcut, m)?)?;
    m.add("build_commit", env!("BUILT_COMMIT"))?;
    m.add_function(wrap_pyfunction!(inspector_geometry::inspector_totals, m)?)?;
    m.add_function(wrap_pyfunction!(inspector_geometry::inspector_curve, m)?)?;
    m.add_function(wrap_pyfunction!(inspector_geometry::inspector_rename, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tree_move, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tag_controls::tree_edit_tag, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tag_controls::inspector_tag_row, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tag_controls::set_new_tags_hidden, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::row_actions::tree_header_state, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::row_actions::tree_purge_tags, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::row_actions::tree_delete_tag, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tag_visibility::isolate_tags, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tag_visibility::has_tag_actions, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tag_visibility::viewport_tag_menu, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tag_visibility::choose_merge_tag, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::row_actions::tree_tag_action, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::row_actions::tree_delete_rows, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tree_group_rows, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::placement_controls::browser_selection, m)?)?;
    m.add_function(wrap_pyfunction!(tree_grouping::tree_group_action, m)?)?;
    m.add_function(wrap_pyfunction!(model_escape, m)?)?;
    m.add_function(wrap_pyfunction!(context_targets, m)?)?;
    m.add_function(wrap_pyfunction!(parallel_annotation_ray, m)?)?;
    m.add_function(wrap_pyfunction!(preview_texture::preview_materials, m)?)?;
    m.add_function(wrap_pyfunction!(tool_feedback::sync_tool_preview, m)?)?;
    m.add_function(wrap_pyfunction!(transform_marks::prepare_face_marks, m)?)?;
    m.add_class::<extensions::ExtensionBrowser>()?;
    m.add_class::<component_catalogue::ComponentCatalogue>()?;
    m.add_function(wrap_pyfunction!(surfaces::surface_spec, m)?)?;
    m.add_function(wrap_pyfunction!(large_sheet_check, m)?)?;
    m.add_function(wrap_pyfunction!(panic_log::install_panic_log, m)?)?;
    m.add_function(wrap_pyfunction!(panic_log::_panic_for_test, m)?)?;
    m.add_class::<ui::operation_form::OperationForm>()?;
    m.add_function(wrap_pyfunction!(scene_photo::render_scene_photo,m)?)?;
    m.add_function(wrap_pyfunction!(scene_photo::opaque_frame,m)?)?;
    m.add_function(wrap_pyfunction!(scene_photo::save_thumbnail_mode,m)?)?;
    start_view::register(m)?;
    library_view::register(m)?;
    m.add_class::<resident_rows::ResidentRows>()?;
    sheet_workspace::register(m)?;
    sheet_frames::register(m)?;
    m.add_function(wrap_pyfunction!(sheet_host::show_delivery_form, m)?)?;
    m.add_function(wrap_pyfunction!(sheet_host::show_view_export, m)?)?;
    m.add_class::<units_start::UnitsDraft>()?;
    m.add_class::<preferences::PreferencesView>()?;
    m.add_class::<preferences::RestartPreferences>()?;
    m.add_class::<preferences::PreferencePage>()?;
    m.add_function(wrap_pyfunction!(preferences::edit_ai_connection, m)?)?;
    preferences::device_profile::register(m)?;
    preferences::motion::register(m)?;
    m.add_class::<preferences::DevicePreferences>()?;
    m.add_class::<preferences::DeviceReadings>()?;
    m.add_class::<about::AboutReview>()?;
    release::check::register(m)?;
    m.add_class::<release::ReleaseReview>()?;
    m.add_function(wrap_pyfunction!(sheet_search::sheet_command_scope, m)?)?;
    m.add_function(wrap_pyfunction!(text_outlines::text_outlines,m)?)?;
    m.add_function(wrap_pyfunction!(text_outlines::line_text_strokes,m)?)?;
    m.add_function(wrap_pyfunction!(about::notice_text,m)?)?;
    m.add_class::<svg_texture::SvgTexture>()?;
    m.add_class::<svg_texture::UvProjection>()?;
    m.add_class::<window_job::WindowJob>()?;
    m.add_class::<window_timers::WindowTimers>()?;
    m.add_class::<change_callbacks::CallbackSlot>()?;
    m.add_class::<change_callbacks::ChangeCallbacks>()?;
    m.add_class::<window_timers::WindowTimer>()?;
    m.add_class::<camera_inspector::CameraInspector>()?;
    m.add_class::<tree_grouping::paint::TreePaint>()?;
    m.add_class::<registry_editor::RegistryEditor>()?;
    m.add_function(wrap_pyfunction!(render_result::blender_availability, m)?)?;
    m.add_class::<render_result::RenderResult>()?;
    m.add_function(wrap_pyfunction!(font_lengths::font_length, m)?)?;
    shell_window::open::register(m)?;
    font_picker_host::register(m)?;
    m.add_function(wrap_pyfunction!(scale_editor::ask_scale,m)?)?;
    m.add_function(wrap_pyfunction!(release::service::start_check,m)?)?;
    view_export::register(m)?;
    m.add_class::<svg_options::SvgOptions>()?;
    m.add_class::<tag_counts::TagCounts>()?;
    m.add_function(wrap_pyfunction!(commands::command_registry, m)?)?;
    m.add_function(wrap_pyfunction!(commands::command_description, m)?)?;
    m.add_function(wrap_pyfunction!(commands::shortcut_conflicts, m)?)?;
    m.add_function(wrap_pyfunction!(commands::shortcut_domains, m)?)?;
    m.add_function(wrap_pyfunction!(commands::shortcut_reserved, m)?)?;
    m.add_function(wrap_pyfunction!(model_menu::run_model_menu, m)?)?;
    keyboard_bindings::register(m)?;
    m.add_function(wrap_pyfunction!(shortcut_text::shortcut_text, m)?)?;
    m.add_function(wrap_pyfunction!(shortcut_text::shortcut_alternatives, m)?)?;
    m.add_class::<SceneUpload>()?;
    m.add_class::<Renderer>()?;
    // The shell's glue, last: it reads the classes and functions above.
    shell::register_shell(m)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indirect_layout() {
        assert_eq!(std::mem::size_of::<Command>(), 20);
        assert_eq!(std::mem::offset_of!(Command, start), 16);
        assert_eq!(std::mem::offset_of!(Command, instances), 4);
    }
}

/// Reference of the shader's conservative homogeneous eight-corner test.
#[cfg(test)]
fn visible(
    vp: glam::Mat4,
    world: glam::Mat4,
    lo: glam::Vec3,
    hi: glam::Vec3,
    size: glam::Vec2,
) -> bool {
    let m = vp * world;
    let mut rejected = [0; 6];
    let mut low = glam::Vec2::splat(f32::INFINITY);
    let mut high = glam::Vec2::splat(f32::NEG_INFINITY);
    let mut crosses = false;
    for k in 0..8 {
        let p = glam::vec3(
            if k & 1 != 0 { hi.x } else { lo.x },
            if k & 2 != 0 { hi.y } else { lo.y },
            if k & 4 != 0 { hi.z } else { lo.z },
        );
        let q = m * p.extend(1.);
        for (i, out) in [
            q.x < -q.w,
            q.y < -q.w,
            q.z < 0.,
            q.x > q.w,
            q.y > q.w,
            q.z > q.w,
        ]
        .iter()
        .enumerate()
        {
            rejected[i] += *out as u32;
        }
        if q.w <= 0. {
            crosses = true;
        } else {
            let xy = glam::vec2(q.x, q.y) / q.w;
            low = low.min(xy);
            high = high.max(xy);
        }
    }
    !rejected.contains(&8) && (crosses || ((high - low) * size * 0.5).max_element() >= 1.)
}
#[cfg(test)]
mod reference_tests {
    use super::*;
    use glam::*;
    #[test]
    fn clip_planes_and_pixel_size() {
        let lo = vec3(-0.1, -0.1, 0.2);
        let hi = vec3(0.1, 0.1, 0.4);
        assert!(visible(
            Mat4::IDENTITY,
            Mat4::IDENTITY,
            lo,
            hi,
            Vec2::splat(1000.)
        ));
        for shift in [
            vec3(2., 0., 0.),
            vec3(-2., 0., 0.),
            vec3(0., 2., 0.),
            vec3(0., -2., 0.),
            vec3(0., 0., 2.),
            vec3(0., 0., -2.),
        ] {
            assert!(!visible(
                Mat4::IDENTITY,
                Mat4::from_translation(shift),
                lo,
                hi,
                Vec2::splat(1000.)
            ));
        }
        assert!(!visible(
            Mat4::IDENTITY,
            Mat4::from_scale(Vec3::splat(0.001)),
            lo,
            hi,
            Vec2::splat(1000.)
        ));
    }
    #[test]
    fn mirrored_and_sheared_boxes_remain_visible() {
        let world = Mat4::from_cols(
            vec4(-2., 0., 0., 0.),
            vec4(0.5, 1., 0., 0.),
            Vec4::Z,
            Vec4::W,
        );
        assert!(visible(
            Mat4::IDENTITY,
            world,
            vec3(-0.1, -0.1, 0.),
            vec3(0.1, 0.1, 0.5),
            Vec2::splat(1000.)
        ));
    }
    #[test]
    fn compact_into_disjoint_capacity_ranges() {
        let codes = [1, 0, 1, 2, 0];
        let keep = [true, false, true, true, true];
        let starts = [0, 2, 4];
        let mut counts = [0; 3];
        let mut out = [u32::MAX; 5];
        for (i, &code) in codes.iter().enumerate() {
            if keep[i] {
                out[starts[code] + counts[code]] = i as u32;
                counts[code] += 1;
            }
        }
        assert_eq!(counts, [1, 2, 1]);
        assert_eq!(out, [4, u32::MAX, 0, 2, 3]);
        // Reset between frames; no stale counts may survive the template copy.
        counts.fill(0);
        assert_eq!(counts, [0, 0, 0]);
    }
}

// The single-sample path supplies color and IDs together. MSAA only needs
// the extra pass for an explicit pick or once after the camera settles.
fn needs_id_pass(msaa: bool, pick_pending: bool, dirty: bool, stable_frames: u32) -> bool {
    !msaa || pick_pending || (dirty && stable_frames >= 2)
}

#[cfg(test)]
mod id_schedule_tests {
    #[test]
    fn multisample_color_does_not_replay_geometry_for_idle_picking() {
        assert!(!super::needs_id_pass(true, false, true, 1));
        assert!(super::needs_id_pass(true, true, true, 0));
        assert!(super::needs_id_pass(false, false, true, 0));
        assert!(super::needs_id_pass(false, true, true, 0));
        assert!(super::needs_id_pass(true, false, true, 2));
        assert!(!super::needs_id_pass(true, false, false, 3));
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;

    #[test]
    fn growth_doubles_and_never_shrinks() {
        assert_eq!(grown_capacity(8, 9), 16);
        assert_eq!(grown_capacity(16, 4), 16);
        assert_eq!(grown_capacity(8, 33), 64);
        assert_eq!(grown_capacity(0, 0), 1);
    }

    #[test]
    fn scattered_updates_use_one_enclosing_write() {
        assert_eq!(row_runs(vec![4, 1, 2, 2, 8]), vec![(1, 9)]);
        assert!(row_runs(vec![]).is_empty());
        assert_eq!(row_runs((0..1000).rev().collect()), vec![(0, 1000)]);
    }

    #[test]
    fn row_updates_preserve_selection_and_hover() {
        assert_eq!(merge_instance_flags(6, 1), 7);
        assert_eq!(merge_instance_flags(7, 0), 6);
        assert_eq!(merge_instance_flags(0, 6), 0);
        assert_eq!(merge_instance_flags(6, 9), 15);
        assert_eq!(merge_instance_flags(15, 8), 14);
        assert_eq!(merge_instance_flags(64 | 128 | 6, 9), 64 | 128 | 15);
    }
}

// Sparse rows contain a back color followed by front/back texture indices.
// Only the fragment texture branch is specialized: vertex paint lookup stays.
fn face_ranges(data: &[u8], prototypes: u32) -> Vec<(u32, u32, bool)> {
    let word = |at: usize| data.get(at..at+4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let mut runs: Vec<(u32, u32, bool)> = Vec::new();
    for p in 0..prototypes {
        let header = p as usize * 16;
        let plain = match (word(header+4), word(header+8)) {
            (Some(start), Some(count)) => (0..count as usize).all(|i| {
                let row = (start as usize+i*2+1)*16;
                word(row+4) == Some(0) && word(row+8) == Some(0)
            }),
            _ => false,
        };
        if let Some(last) = runs.last_mut().filter(|r| r.2 == plain) { last.1 += 1; }
        else { runs.push((p, 1, plain)); }
    }
    runs
}
#[cfg(test)]
mod plain_face_tests {
    #[test]
    fn sparse_texture_indices_split_only_textured_prototypes() {
        let mut words = vec![0u32; 24];
        words[5] = 3; words[6] = 1; // prototype 1 has one sparse vertex
        words[17] = 2; // its front texture
        assert_eq!(super::face_ranges(bytemuck::cast_slice(&words), 3), vec![(0,1,true),(1,1,false),(2,1,true)]);
        words[17] = 0; words[18] = 2; // back texture also needs alpha rejection
        assert_eq!(super::face_ranges(bytemuck::cast_slice(&words), 3)[1], (1,1,false));
        words[18] = 0;
        assert_eq!(super::face_ranges(bytemuck::cast_slice(&words), 3), vec![(0,3,true)]);
        assert_eq!(super::face_ranges(&[], 1), vec![(0,1,false)]);
    }
}

struct PassPlan { msaa: bool, ao: bool, xray: bool }
impl PassPlan {
    fn color_only(&self, style: u32, edges: bool) -> bool {
        !edges && style != 4 && !self.ao && !self.xray
    }
    fn new(antialias: bool, mode: u32, back_edges: bool, ao: bool) -> Self {
        Self { msaa: antialias && mode != 1 && !back_edges,
            ao: ao && mode != 1 && mode != 4, xray: mode == 1 }
    }
}
#[cfg(test)]
mod pass_plan_tests {
    #[test]
    fn disabled_features_encode_no_optional_passes() {
        let p = super::PassPlan::new(false, 0, false, false);
        assert!(!p.msaa && !p.ao && !p.xray);
        let p = super::PassPlan::new(true, 1, false, true);
        assert!(p.xray && !p.msaa && !p.ao);
        let p = super::PassPlan::new(true, 4, true, true);
        assert!(!p.msaa && !p.ao && !p.xray);
        let p = super::PassPlan::new(true, 0, false, true);
        assert!(p.msaa && p.ao && !p.xray);
    }
}

fn needs_settled_ids(pick: bool, dirty: bool, stable: u32) -> bool { pick || (dirty && stable >= 2) }
#[cfg(test)]
mod color_only_schedule_tests {
    #[test]
    fn settled_color_does_not_clear_or_write_an_id_target() {
        assert!(!super::needs_settled_ids(false, false, 4));
        assert!(!super::needs_settled_ids(false, true, 1));
        assert!(super::needs_settled_ids(false, true, 2));
        assert!(super::needs_settled_ids(true, true, 0));
    }
}

#[cfg(test)]
mod cursor_schedule_tests {
    #[test]
    fn cursor_refine_does_not_validate_the_full_frame() {
        assert!(!super::ids_complete(Some((10, 20))));
        assert!(super::ids_complete(None));
        assert!(!super::needs_settled_ids(false, true, 1));
        assert!(super::needs_settled_ids(false, true, 2));
        assert!(super::needs_settled_ids(true, true, 0));
    }
}
fn ids_complete(cursor: Option<(u32, u32)>) -> bool { cursor.is_none() }

fn pick_bounds(x: u32, y: u32, width: u32, height: u32, margin: f32) -> [f32; 4] {
    [2.*(x as f32-margin)/width as f32-1., 1.-2.*(y as f32+1.+margin)/height as f32,
     2.*(x as f32+1.+margin)/width as f32-1., 1.-2.*(y as f32-margin)/height as f32]
}
#[cfg(test)]
mod pick_bounds_tests {
    #[test]
    fn cursor_bounds_include_pixel_and_wide_loose_edges() {
        let b = super::pick_bounds(49, 49, 100, 100, 2.);
        for p in [[-0.02,0.02], [0.,0.], [-0.05,0.05]] {
            assert!(p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]);
        }
        assert!(b[0] < 0. && b[1] < 0. && b[2] > 0. && b[3] > 0.);
    }
}

fn suppressed_indices(source: &[u8], commands: &[u8], prototypes: usize, ranges: &[(usize,u32,u32)]) -> Result<Vec<u8>, &'static str> {
    let mut result = source.to_vec();
    for &(prototype, first, count) in ranges {
        if prototype >= prototypes { return Err("suppression prototype out of range"); }
        let c: Command = bytemuck::pod_read_unaligned(&commands[prototype*20..prototype*20+20]);
        if u64::from(first)+u64::from(count) > u64::from(c.count)/3 { return Err("suppression triangle range out of bounds"); }
        for triangle in first..first+count {
            let at = (c.first as usize+triangle as usize*3)*4;
            let index = result[at..at+4].to_vec();
            result[at+4..at+8].copy_from_slice(&index); result[at+8..at+12].copy_from_slice(&index);
        }
    }
    Ok(result)
}
#[cfg(test)] mod suppression_tests {
    #[test] fn suppress_and_restore_only_requested_triangles() {
        let source = bytemuck::cast_slice(&[0u32,1,2,3,4,5]);
        let cmd = bytemuck::bytes_of(&Command { count:6,instances:0,first:0,base:0,start:0 }).to_vec();
        let result = super::suppressed_indices(source,&cmd,1,&[(0,1,1)]).unwrap();
        assert_eq!(bytemuck::cast_slice::<u8,u32>(&result), &[0,1,2,3,3,3]);
        assert_eq!(super::suppressed_indices(source,&cmd,1,&[]).unwrap(),source);
        assert!(super::suppressed_indices(source,&cmd,1,&[(0,1,2)]).is_err());
    }
    use super::Command;
}

fn ghost_mesh(s:&Scene, resident:&resident_readback::Readback, vertices:&[u8], indices:&[u8], faces:&[u8], edges:&[u8], objects:&[usize], object_pass:bool) -> Result<(Vec<u8>,Vec<u8>), &'static str> {
    if vertices.len()%28!=0 || indices.len()%4!=0 || faces.len()!=s.p as usize*20 || edges.len()!=faces.len() || objects.iter().any(|&i|i>=s.n as usize) {return Err("invalid hidden geometry layout");}
    let objects:std::collections::HashSet<_>=objects.iter().copied().collect();
    let mut triangles=Vec::new();let mut lines=Vec::new();
    for (row_id,row) in s.rows.chunks_exact(80).enumerate() {
        let flags=u32::from_le_bytes(row[72..76].try_into().unwrap());
        if (flags&128)!=0 || objects.contains(&row_id)!=object_pass || (!object_pass && flags&1!=0) {continue;}
        let prototype=u32::from_le_bytes(row[64..68].try_into().unwrap()) as usize;
        let world=glam::Mat4::from_cols_array(&bytemuck::pod_read_unaligned::<[f32;16]>(&row[..64]));
        let normal=glam::Mat3::from_mat4(world).inverse().transpose();
        let emit=|v:Option<&[u8]>,ix:&[u8],commands:&[u8],out:&mut Vec<u8>,arity:u32| -> Result<(), &'static str> {
            let c:Command=bytemuck::pod_read_unaligned(&commands[prototype*20..prototype*20+20]);
            if c.count%arity!=0 || (u64::from(c.first)+u64::from(c.count))*4>ix.len() as u64 {return Err("invalid hidden command range");}
            for i in ix[c.first as usize*4..(c.first+c.count) as usize*4].chunks_exact(4) {
                let index=i64::from(u32::from_le_bytes(i.try_into().unwrap()))+i64::from(c.base);
                let index = usize::try_from(index).map_err(|_| "negative hidden vertex")?;
                let source = match v {
                    Some(bytes) => bytes.get(index*28..index*28+28),
                    None => resident.vertex(index),
                }.ok_or("hidden vertex out of range")?;
                let pos=world.transform_point3(glam::Vec3::from_array(bytemuck::pod_read_unaligned(&source[..12])));
                let n=(normal*glam::Vec3::from_array(bytemuck::pod_read_unaligned(&source[12..24]))).normalize_or_zero();
                out.extend_from_slice(bytemuck::cast_slice(&pos.to_array()));out.extend_from_slice(bytemuck::cast_slice(&n.to_array()));out.extend_from_slice(&source[24..28]);
            } Ok(())
        };
        emit(Some(vertices),indices,faces,&mut triangles,3)?;emit(Some(vertices),indices,edges,&mut lines,2)?;
        if object_pass {
            emit(None,&s.index_data,&s.template_data,&mut triangles,3)?;
            let c:[u32;5]=bytemuck::pod_read_unaligned(&s.template_data[(s.p as usize+prototype)*20..(s.p as usize+prototype+1)*20]);
            for edge in s.edge_data[(c[2]/6) as usize*48..((c[2]+c[0])/6) as usize*48].chunks_exact(48) {
                for endpoint in [&edge[..12],&edge[12..24]] {
                    let point=world.transform_point3(glam::Vec3::from_array(bytemuck::pod_read_unaligned(endpoint)));
                    lines.extend_from_slice(bytemuck::cast_slice(&point.to_array()));
                    lines.extend_from_slice(bytemuck::cast_slice(&[0f32,0.,1.]));
                    lines.extend_from_slice(&[33,43,59,255]);
                }
            }
        }
    }
    Ok((triangles,lines))
}

#[cfg(test)]
mod design_system_tests;
