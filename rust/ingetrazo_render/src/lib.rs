//! A resident Metal scene: compute compaction precedes native indirect draws.
mod timing;
mod panic_log;
mod overlay;
pub mod ui;
mod inspector;
mod glass;
mod axes;
mod sections;
mod preview;
mod lines;
mod sun;
mod textures;
// R-look: isolated style and background passes.
mod style;
mod background;
mod antialias;
mod depth_resolve;
mod depth;
mod edges;
mod faces;
mod xray;
mod ao;
mod resident_readback;
use numpy::PyReadonlyArray1;
use pyo3::{exceptions::PyValueError, prelude::*};
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use std::{ptr::NonNull, time::Instant};
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
    compute: wgpu::BindGroup,
    draw: wgpu::BindGroup,
    edge_draw: wgpu::BindGroup,
    xray_draw: wgpu::BindGroup,
    xray_edge_draw: wgpu::BindGroup,
    profile_draw: wgpu::BindGroup,
    n: u32,
    p: u32,
}
/// One more view of the resident scene: its own surface, size-dependent
/// attachments, view-owned passes and uniforms. Everything that belongs to
/// the document (geometry, instances, prototypes, materials, textures, the
/// sun's depth map) stays single in the Renderer; ``bind_target`` swaps one
/// of these into the live fields, so every existing pass keeps working.
struct Target {
    surface: wgpu::Surface<'static>,
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
    sections: sections::Sections,
    camera_data: Vec<f32>,
    look_data: [[f32; 4]; 11],
    sun: sun::Settings,
    style: u32,
    edges: bool,
    textures_enabled: bool,
    ids_dirty: bool,
    stable_frames: u32,
    pick_pending: bool,
    capture_next: bool,
    captured: Option<Vec<u8>>,
    /// ``scene_epoch`` when it was last live: a later scene change leaves
    /// its pick IDs stale.
    seen_epoch: u64,
}
#[pyclass(unsendable)]
struct Renderer {
    instance: wgpu::Instance,
    /// Views of the same scene; ``None`` at the live one's index.
    targets: Vec<Option<Target>>,
    active: usize,
    alive: Vec<bool>,
    /// Bumped by every change that invalidates pick IDs for all views.
    scene_epoch: u64,
    textures: textures::Textures,
    textures_enabled: bool,
    material_transparency: bool,
    texture_groups: Vec<wgpu::BindGroup>,
    ao: ao::AmbientOcclusion,
    antialias: bool,
    ids_dirty: bool,
    camera_data: Vec<f32>,
    stable_frames: u32,
    pick_pending: bool,
    aa: antialias::Antialias,
    transparency: xray::Xray,
    look: style::Style,
    background: background::Background,
    look_groups: Vec<wgpu::BindGroup>,
    capture_next: bool,
    captured: Option<Vec<u8>>,
    overlay: overlay::Overlay,
    /// The Sheets workspace's page (views/wgpu_sheets.py): drawn by
    /// ``render_page`` instead of the scene.
    page: overlay::Overlay,
    axes: axes::Axes,
    sections: sections::Sections,
    preview: preview::Preview,
    lines: lines::Lines,
    face_marks: preview::Preview,
    sun: sun::Sun,
    ghost_geometry: preview::Preview,
    ghost_objects: preview::Preview,
    surface: wgpu::Surface<'static>,
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
}
fn row_runs(ids: Vec<usize>) -> Vec<(usize, usize)> {
    // The CPU mirror includes flags and overrides for untouched rows. Uploading
    // the enclosing range batches scattered selections into one queue write.
    match (ids.iter().min(), ids.iter().max()) {
        (Some(&first), Some(&last)) => vec![(first, last + 1)],
        _ => Vec::new(),
    }
}

fn grown_capacity(current: usize, needed: usize) -> usize {
    current.max(needed.max(1).next_power_of_two())
}

fn merge_instance_flags(old: u32, incoming: u32) -> u32 {
    (old & (6 | 64 | 128)) | (incoming & 9)
}

fn texture(d: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
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
        view_formats: &[],
    })
}
fn multisampled(d: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some("4x color resolve"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 4,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
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
#[pymethods]
impl Renderer {
    #[new]
    fn new(ns_view_ptr: usize, width: u32, height: u32, scale: f64) -> PyResult<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..Default::default()
        });
        let ptr = NonNull::new(ns_view_ptr as *mut std::ffi::c_void)
            .ok_or_else(|| PyValueError::new_err("null NSView"))?;
        // Qt owns the NSView. Python retains this Renderer only while its widget lives.
        // wgpu's Metal backend installs/reuses a CAMetalLayer on this AppKit view.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::AppKit(AppKitDisplayHandle::new()),
                raw_window_handle: RawWindowHandle::AppKit(AppKitWindowHandle::new(ptr)),
            })
        }
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let a = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .ok_or_else(|| PyValueError::new_err("no Metal adapter"))?;
        let optional = wgpu::Features::MULTI_DRAW_INDIRECT | wgpu::Features::TIMESTAMP_QUERY;
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
        let caps = surface.get_capabilities(&a);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .ok_or_else(|| PyValueError::new_err("no sRGB surface"))?;
        let w = ((width as f64 * scale) as u32).max(1);
        let h = ((height as f64 * scale) as u32).max(1);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width: w,
            height: h,
            present_mode: wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
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
        let queries = features.contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
            device.create_query_set(&wgpu::QuerySetDescriptor {
                label: None,
                ty: wgpu::QueryType::Timestamp,
                count: timing::COUNT,
            })
        });
        let resolve = buffer(
            &device,
            &[0; timing::BYTES],
            wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        );
        let read = buffer(
            &device,
            &[0; 256],
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let sun = sun::Sun::new(&device, format);
        let look = style::Style::new(&device);
        let preview = preview::Preview::new(&device, format, &camera, &look.buffer, 0);
        let face_marks = preview::Preview::new(&device, format, &camera, &look.buffer, 1);
        let ghost_geometry = preview::Preview::new(&device, format, &camera, &look.buffer, 2);
        let ghost_objects = preview::Preview::new(&device, format, &camera, &look.buffer, 2);
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
        let mut aa = antialias::Antialias::new(&device, format, &look.buffer, w, h);
        aa.bind_textures(&device, &textures);
        let mut transparency = xray::Xray::new(&device, format, w, h);
        transparency.bind_textures(&device, &textures);
        Ok(Self {
            instance, targets: vec![None], active: 0, alive: vec![true], scene_epoch: 0,
            textures, texture_groups, textures_enabled: true, material_transparency: true,
            ao: ao::AmbientOcclusion::new(&device, format),
            antialias: true,
            ids_dirty: true,
            camera_data: Vec::new(),
            stable_frames: 0,
            pick_pending: false,
            aa,
            look, background, look_groups,
            transparency,
            preview,
            face_marks,
            ghost_geometry,
            ghost_objects,
            sun,
            capture_next: false,
            captured: None,
            overlay: overlay::Overlay::new(&device, format),
            page: overlay::Overlay::new(&device, format),
            axes: axes::Axes::new(&device, format, &camera),
            sections: sections::Sections::new(&device, format, &camera),
            lines: lines::Lines::new(&device, format, &camera),
            depth: texture(&device, w, h, wgpu::TextureFormat::Depth32Float),
            offscreen: texture(&device, w, h, format),
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
            detailed_timestamps: features.contains(wgpu::Features::TIMESTAMP_QUERY),
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
            adapter: format!(
                "{:?}; multi_draw={}",
                a.get_info(),
                features.contains(wgpu::Features::MULTI_DRAW_INDIRECT)
            ),
        })
    }

    /// Another view of the resident scene on its own NSView: one more
    /// surface and its attachments; the scene is not packed or sent again.
    fn add_target(&mut self, ns_view_ptr: usize, width: u32, height: u32, scale: f64) -> PyResult<usize> {
        let ptr = NonNull::new(ns_view_ptr as *mut std::ffi::c_void)
            .ok_or_else(|| PyValueError::new_err("null NSView"))?;
        let surface = unsafe {
            self.instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::AppKit(AppKitDisplayHandle::new()),
                raw_window_handle: RawWindowHandle::AppKit(AppKitWindowHandle::new(ptr)),
            })
        }
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let w = ((width as f64 * scale) as u32).max(1);
        let h = ((height as f64 * scale) as u32).max(1);
        let mut config = self.config.clone();
        config.width = w;
        config.height = h;
        surface.configure(&self.device, &config);
        let format = config.format;
        let d = &self.device;
        let mut aa = antialias::Antialias::new(d, format, &self.look.buffer, w, h);
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
            offscreen: texture(d, w, h, format),
            ids: texture(d, w, h, wgpu::TextureFormat::R32Uint),
            aa, transparency,
            ao: ao::AmbientOcclusion::new(d, format),
            overlay: overlay::Overlay::new(d, format),
            axes: axes::Axes::new(d, format, &self.camera),
            lines: lines::Lines::new(d, format, &self.camera),
            preview: preview::Preview::new(d, format, &self.camera, &self.look.buffer, 0),
            face_marks: preview::Preview::new(d, format, &self.camera, &self.look.buffer, 1),
            sections: sections::Sections::new(d, format, &self.camera),
            camera_data: Vec::new(),
            look_data: self.look.data,
            sun: self.sun.settings(),
            style: self.style,
            edges: self.edges,
            textures_enabled: self.textures_enabled,
            ids_dirty: true,
            stable_frames: 0,
            pick_pending: false,
            capture_next: false,
            captured: None,
            seen_epoch: self.scene_epoch,
        };
        self.targets.push(Some(target));
        self.alive.push(true);
        Ok(self.targets.len() - 1)
    }
    /// Make view ``id`` the live one: later calls (camera, style, overlay,
    /// lines, preview, resize, render, pick) are its own.
    fn bind_target(&mut self, id: usize) -> PyResult<()> {
        if id == self.active {
            return Ok(());
        }
        let mut t = self.targets.get_mut(id).and_then(Option::take)
            .ok_or_else(|| PyValueError::new_err("no such view target"))?;
        let seen = t.seen_epoch;
        self.swap_live(&mut t);
        t.seen_epoch = self.scene_epoch;
        self.targets[self.active] = Some(t);
        self.active = id;
        if seen != self.scene_epoch {
            self.ids_dirty = true;
        }
        self.queue.write_buffer(&self.look.buffer, 0, bytemuck::cast_slice(&self.look.data));
        if !self.camera_data.is_empty() {
            self.queue.write_buffer(&self.camera, 0, bytemuck::cast_slice(&self.camera_data));
        }
        // The scene's buffers may have been replaced or grown since.
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
        if id == 0 || !self.alive.get(id).copied().unwrap_or(false) {
            return Err(PyValueError::new_err("no such removable view target"));
        }
        if self.active == id {
            self.bind_target(0)?;
        }
        self.targets[id] = None;
        self.alive[id] = false;
        Ok(())
    }
    #[getter]
    fn active_target(&self) -> usize { self.active }
    #[getter]
    fn target_count(&self) -> usize { self.alive.iter().filter(|a| **a).count() }
    /// Bytes of every view's own attachments (surface not included): depth,
    /// colour and ID targets, 4× multisampled colour and depth, X-ray
    /// accumulation — what one more view costs, apart from the scene.
    #[getter]
    fn target_memory(&self) -> u64 {
        let per_pixel = 4 + 4 + 4 + 16 + 16 + 8 + 2;
        let live = self.config.width as u64 * self.config.height as u64;
        self.targets.iter().flatten()
            .map(|t| t.config.width as u64 * t.config.height as u64)
            .sum::<u64>().saturating_add(live) * per_pixel
    }
    /// Packed little-endian arrays; reject malformed uploads before reaching wgpu.
    fn set_scene(
        &mut self,
        prototypes: (
            PyReadonlyArray1<'_, u8>,
            PyReadonlyArray1<'_, u8>,
            PyReadonlyArray1<'_, u8>,
            PyReadonlyArray1<'_, u8>,
        ),
        instances: PyReadonlyArray1<'_, u8>,
    ) -> PyResult<()> {
        self.sun.invalidate();
        let (v, ix, p, cmd) = (&prototypes.0, &prototypes.1, &prototypes.2, &prototypes.3);
        let (v, ix, p, cmd) = (
            v.as_slice()?,
            ix.as_slice()?,
            p.as_slice()?,
            cmd.as_slice()?,
        );
        let instances = instances.as_slice()?;
        if v.len() % 28 != 0
            || ix.len() % 4 != 0
            || p.is_empty()
            || p.len() % 32 != 0
            || cmd.len() != p.len() / 32 * 40
            || instances.is_empty()
            || instances.len() % 80 != 0
        {
            return Err(PyValueError::new_err("invalid scene array layout"));
        }
        let n = instances.len() / 80;
        let count = p.len() / 32;
        let capacity = grown_capacity(0, n);
        let prototype_capacity = grown_capacity(0, count);
        let limit = self.device.limits().max_storage_buffer_binding_size as usize;
        if capacity * 80 > limit || prototype_capacity * 40 > limit {
            return Err(PyValueError::new_err("scene capacity exceeds storage binding limit"));
        }
        let mut counts = vec![0u32; count];
        for row in instances.chunks_exact(80) {
            let id = u32::from_le_bytes(row[64..68].try_into().unwrap()) as usize;
            if id >= count {
                return Err(PyValueError::new_err("prototype id out of range"));
            }
            counts[id] += 1;
        }
        let mut start = 0;
        for (j, (row, capacity)) in cmd.chunks_exact(20).zip(counts.iter().cycle()).enumerate() {
            if j == count {
                start = 0;
            }
            let c: Command = bytemuck::pod_read_unaligned(row);
            if c.instances != 0
                || c.start != start
                || (c.first as u64 + c.count as u64) * 4 > ix.len() as u64
            {
                return Err(PyValueError::new_err("invalid command range"));
            }
            for index in ix[c.first as usize * 4..(c.first + c.count) as usize * 4].chunks_exact(4)
            {
                let j = u32::from_le_bytes(index.try_into().unwrap()) as i64 + c.base as i64;
                if j < 0 || j >= v.len() as i64 / 28 {
                    return Err(PyValueError::new_err("vertex index out of range"));
                }
            }
            start += capacity;
        }
        for bytes in [p, instances] {
            if bytes.len() as u64 > self.device.limits().max_storage_buffer_binding_size as u64 {
                return Err(PyValueError::new_err("scene exceeds storage binding limit"));
            }
        }
        // Preserve the scene ABI; pack unique edges separately from face vertices.
        let (expanded_v, expanded_ix, mut expanded_cmd) = edges::expand(v, ix, cmd, count)
            .map_err(PyValueError::new_err)?;
        let (face_v, face_ix, material_shifts) = faces::compact(v, &expanded_ix, &mut expanded_cmd, count);
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
        let template = capacity_buffer(&self.device, cmd, prototype_capacity * 40, wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        let mut hard_cmd = cmd.to_vec();
        for row in hard_cmd.chunks_exact_mut(20).skip(count) {
            let hard_count = row[16..20].to_vec();
            row[..4].copy_from_slice(&hard_count);
        }
        let hard_template = capacity_buffer(&self.device, &hard_cmd, prototype_capacity * 40, wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
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
        self.ids_dirty = true; self.scene_epoch += 1;
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
            protos,
            prototype_data: p.to_vec(),
            survivors,
            template_data: cmd.to_vec(),
            capacity,
            vertex_bytes: v.len(),
            index_data: ix.to_vec(),
            flags: instances
                .chunks_exact(80)
                .map(|r| u32::from_le_bytes(r[72..76].try_into().unwrap()))
                .collect(),
            vertex,
            index,
            commands,
            template,
            hard_template,
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
        self.face_marks.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
        self.ghost_geometry.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
        for t in self.targets.iter_mut().flatten() {
            t.preview.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
            t.face_marks.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
            t.ids_dirty = true;
        }
        self.ghost_objects.upload(&self.device,&[],&[]).map_err(PyValueError::new_err)?;
        self.device.poll(wgpu::Maintain::Wait);
        Ok(())
    }
    /// Append compact replacement ranges while preserving placement identities.
    fn replace_prototype(&mut self, id: usize, vertices: PyReadonlyArray1<'_, u8>,
        indices: PyReadonlyArray1<'_, u8>, bounds: PyReadonlyArray1<'_, u8>, faces: u32) -> PyResult<()> {
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
        let mut hard = s.template_data.clone();
        for row in hard.chunks_exact_mut(20).skip(s.p as usize) { let count = row[16..20].to_vec(); row[..4].copy_from_slice(&count); }
        for (target, data, usage) in [
            (&mut s.protos, &s.prototype_data, wgpu::BufferUsages::STORAGE),
            (&mut s.template, &s.template_data, wgpu::BufferUsages::COPY_SRC),
            (&mut s.hard_template, &hard, wgpu::BufferUsages::COPY_SRC),
            (&mut s.commands, &s.template_data, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT)] {
            if data.len() as u64 > target.size() { *target = capacity_buffer(&self.device, data, grown_capacity(target.size() as usize, data.len()), usage | wgpu::BufferUsages::COPY_DST); }
            else { self.queue.write_buffer(target, 0, data); }
        }
        s.compute = bind_prototypes(&self.device, &self.compute.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.protos, &s.commands, &s.survivors, &s.viewpoints, &s.counters], s.p);
        s.edge_draw = bind(&self.device, &self.edge.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
        s.xray_edge_draw = bind(&self.device, &self.xray_edge.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
        s.profile_draw = bind(&self.device, &self.profile_top.get_bind_group_layout(0), &[&self.camera, &s.inst, &s.survivors, &s.edge_records, &s.viewpoints]);
        self.aa.bind_scene(&self.device, &self.camera, &s.inst, &s.survivors, &self.look.buffer, &s.edge_records, &s.viewpoints);
        self.ids_dirty = true; self.scene_epoch += 1;
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
        if !ids.is_empty() { self.ids_dirty = true; self.scene_epoch += 1; self.stable_frames = 0; }
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
        self.ids_dirty = true; self.scene_epoch += 1;
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
            self.queue.write_buffer(&s.template, 0, &s.template_data);
            let mut hard = s.template_data.clone();
            for row in hard.chunks_exact_mut(20).skip(s.p as usize) { let count = row[16..20].to_vec(); row[..4].copy_from_slice(&count); }
            self.queue.write_buffer(&s.hard_template, 0, &hard);
        }
        Ok(())
    }

    /// Tombstones retain row identities so stale picks cannot select a neighbour.
    fn remove_instances(&mut self, ids: Vec<usize>) -> PyResult<()> {
        self.sun.invalidate();
        self.ids_dirty = true; self.scene_epoch += 1;
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
                self.ids_dirty = true; self.scene_epoch += 1;
            }
        }
        Ok(())
    }
    fn set_overrides(&mut self, colors: Vec<u32>) -> PyResult<()> {
        self.ids_dirty = true; self.scene_epoch += 1;
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
        self.ids_dirty = true;
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
    /// World-space tool feedback, cleared on commit/cancel.
    fn set_preview_mesh(&mut self, triangles: PyReadonlyArray1<'_, u8>, edges: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        self.preview.upload(&self.device, triangles.as_slice()?, edges.as_slice()?).map_err(PyValueError::new_err)
    }
    /// Depth-tested world-space lines in a named slot (``lines.rs``):
    /// 40-byte segments ``a, b (3 f32), RGBA8, width, dash, gap (f32
    /// physical pixels)``. An empty array empties the slot. A slot is drawn
    /// before the scene edges (GL's scaffold) unless ``above_edges``.
    #[pyo3(signature = (slot, segments, above_edges=false))]
    fn set_lines(&mut self, slot: &str, segments: PyReadonlyArray1<'_, u8>, above_edges: bool) -> PyResult<()> {
        self.lines.upload(&self.device, &self.queue, slot, segments.as_slice()?, above_edges).map_err(PyValueError::new_err)
    }
    fn line_counts(&self) -> Vec<(String, u32)> { self.lines.counts() }
    /// Loose-face marks only; groups use group_selection_edges.
    fn set_face_marks(&mut self, triangles: PyReadonlyArray1<'_, u8>) -> PyResult<()> {
        self.face_marks.upload(&self.device, triangles.as_slice()?, &[]).map_err(PyValueError::new_err)
    }
    /// Editable face IDs resolve to (prototype, local triangle index, count).
    /// Shared prototypes affect every placement: duplicate before a local edit.
    fn set_suppressed_faces(&mut self, ranges: Vec<(usize, u32, u32)>) -> PyResult<()> {
        self.sun.invalidate();
        let s = self.scene.as_ref().ok_or_else(|| PyValueError::new_err("set_scene first"))?;
        let data = suppressed_indices(&s.index_data, &s.template_data, s.p as usize, &ranges)
            .map_err(PyValueError::new_err)?;
        self.queue.write_buffer(&s.index, 0, &data);
        self.ids_dirty = true; self.scene_epoch += 1;
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
        let mut hard = s.template_data.clone();
        for row in hard.chunks_exact_mut(20).skip(s.p as usize) {
            let count = row[16..20].to_vec(); row[..4].copy_from_slice(&count);
        }
        self.queue.write_buffer(&s.template, 0, &s.template_data);
        self.queue.write_buffer(&s.hard_template, 0, &hard);
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
    fn set_antialias(&mut self, enabled: bool) { self.antialias = enabled; }
    fn read_pixels(&mut self) -> PyResult<Vec<u8>> {
        self.capture_next = true;
        self.render(true)?;
        Ok(self.captured.take().unwrap_or_default())
    }
    /// Render with resident geometry and textures into isolated sheet targets.
    /// Surface configuration and interactive attachments are never replaced.
    #[pyo3(signature = (width, height, view, projection, style, instances=None, far_clip=false))]
    fn render_image(&mut self, width: u32, height: u32, view: Vec<f32>,
                    projection: Vec<f32>, style: &Bound<'_, PyAny>, instances: Option<PyReadonlyArray1<'_, u8>>,
                    far_clip: bool) -> PyResult<Vec<u8>> {
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
        let format = self.config.format;
        let mut aa = antialias::Antialias::new(&self.device, format, &self.look.buffer, width, height);
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
        let old_flags = (self.style, self.edges, self.textures_enabled, self.ids_dirty,
                         self.stable_frames, self.pick_pending, self.capture_next);
        let old_capture = self.captured.take();
        // Keep the original resources alive until the transaction has completed.
        let old_aa = std::mem::replace(&mut self.aa, aa);
        let old_transparency = std::mem::replace(&mut self.transparency, transparency);
        let old_sun = std::mem::replace(&mut self.sun, sun::Sun::new(&self.device, format));
        let old_ao = std::mem::replace(&mut self.ao, ao::AmbientOcclusion::new(&self.device, format));
        let old_overlay = std::mem::replace(&mut self.overlay, overlay::Overlay::new(&self.device, format));
        let old_color = std::mem::replace(&mut self.offscreen, texture(&self.device, width, height, format));
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
            self.capture_next = true;
            self.render(false)?;
            Ok(self.captured.take().unwrap_or_default())
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
        self.offscreen = old_color;
        self.depth = old_depth;
        self.ids = old_ids;
        self.look.data = old_style;
        self.queue.write_buffer(&self.look.buffer, 0, bytemuck::cast_slice(&old_style));
        if !old_camera.is_empty() {
            self.queue.write_buffer(&self.camera, 0, bytemuck::cast_slice(&old_camera));
        }
        self.camera_data = old_camera;
        (self.style, self.edges, self.textures_enabled, self.ids_dirty,
         self.stable_frames, self.pick_pending, self.capture_next) = old_flags;
        self.captured = old_capture;
        result
    }
    fn set_edges(&mut self, enabled: bool) {
        self.ids_dirty |= self.edges != enabled;
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
            self.stable_frames = 0;
            self.ids_dirty = true;
            self.queue.write_buffer(&self.camera, 0, bytemuck::cast_slice(&data));
            self.camera_data = data;
        }
        Ok(())
    }
    /// Kept for the R2a measurement script: anti-aliasing now lives in the
    /// antialias module, so this is the same switch as ``set_antialias``.
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
        self.ids_dirty = true; self.scene_epoch += 1;
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
        self.ids_dirty = true; self.scene_epoch += 1;
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
        self.ids_dirty = true;
        self.ui_scale = scale as f32;
        self.config.width = ((w as f64 * scale) as u32).max(1);
        self.config.height = ((h as f64 * scale) as u32).max(1);
        self.surface.configure(&self.device, &self.config);
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
        let frame = if present { Some(self.surface.get_current_texture()
            .map_err(|e| PyValueError::new_err(e.to_string()))?) } else { None };
        let target = frame.as_ref().map(|f| &f.texture).unwrap_or(&self.offscreen);
        let acquired = Instant::now();
        self.last_acquire = acquired.duration_since(polled).as_secs_f64() * 1000.;
        let color = target.create_view(&Default::default());
        let depth = self.depth.create_view(&Default::default());
        let ids = self.ids.create_view(&Default::default());
        let mut e = self.device.create_command_encoder(&Default::default());
        let tq = self.queries.as_ref().filter(|_| self.detailed_timestamps);
        self.sun.prepare(&self.device, &self.queue, &mut e, s, &self.textures, &self.look.buffer);
        self.background.draw(&mut e, &color, tq);
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
        let plan = PassPlan::new(self.antialias, self.style, self.look.data[6][2] != 0., self.ao.active(self.style));
        let msaa = plan.msaa && !self.pick_pending;
        self.stable_frames = self.stable_frames.saturating_add(1);
        let color_only = !msaa && !self.edges && self.style != 4 && !plan.ao && !plan.xray;
        let draw_ids = if color_only { needs_settled_ids(self.pick_pending, self.ids_dirty, self.stable_frames) }
            else { needs_id_pass(msaa, self.pick_pending, self.ids_dirty, self.stable_frames) };
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
            if edge_only && !(self.edges || self.style == 4 || ao_active || self.axes.visible) { break; }
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
                        load: if edge_only { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(depth::CLEAR) },
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
                    if self.multi {
                        for k in s.p..s.p*2 { pass.draw_indirect(&s.commands, k as u64*20); }
                    } else {
                        for k in s.p..s.p * 2 { pass.draw_indirect(&s.commands, k as u64 * 20); }
                    }
                }
                if self.style == 1 && self.look.data[6][0] > 0. {
                    pass.set_pipeline(&self.profile_top);
                    pass.set_bind_group(0, &s.profile_draw, &[]);
                    pass.set_bind_group(1, &self.look_groups[4], &[]);
                    if self.multi { for k in s.p..s.p*2 { pass.draw_indirect(&s.commands, k as u64*20); } }
                    else { for k in s.p..s.p*2 { pass.draw_indirect(&s.commands, k as u64*20); } }
                }
            }
            drop(pass);
            }
            if msaa && !self.pick_pending {
                self.aa.draw(&mut e, &color, s, self.style, self.edges, self.look.data[6][0] > 0., self.multi, self.queries.as_ref(), Some(edge_only), self.sections.count > 0 || self.preview.active() || self.face_marks.active() || self.ghost_geometry.active() || self.ghost_objects.active() || self.lines.active_layer(true) || (self.look.data[9][0] > 0. && self.material_transparency), &self.axes);
            }
            if !edge_only && self.sun.enabled && self.style != 1 && self.style != 4 {
                if msaa { self.aa.resolve_depth(&self.device, &mut e, &depth, None); }
                let output = if msaa { self.aa.color_view() } else { target.create_view(&Default::default()) };
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
                let output = if msaa && !self.pick_pending { self.aa.color_view() } else { target.create_view(&Default::default()) };
                self.ao.draw(&self.device, &self.queue, &mut e, target, &depth, &output, msaa && !self.pick_pending, tq);
            }
        }
        if (!msaa || self.pick_pending) && (draw_ids || ao_active) { self.ids_dirty = false; }
        if self.sections.count > 0 && !ao_active {
            if msaa && !self.pick_pending {
                self.sections.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color), tq, &self.camera_data);
            } else {
                self.sections.draw(&mut e, &color, &depth, None, tq, &self.camera_data);
            }
        }
        if msaa {
            self.preview.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color));
            self.face_marks.draw(&mut e, &self.aa.color_view(), &self.aa.depth_view(), Some(&color));
        } else {
            self.preview.draw(&mut e, &color, &depth, None);
            self.face_marks.draw(&mut e, &color, &depth, None);
        }
        for (enabled,ghost) in [(self.look.data[9][2]>0.,&self.ghost_geometry),(self.look.data[9][3]>0.,&self.ghost_objects)] {
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
        self.queue.submit([command]);
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
            self.captured = Some(rgba);
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
            let mut active = vec!["background", "culling", "overlay", "frame"];
            if self.sections.count > 0 { active.push("section_caps"); }
            if !msaa { active.push("faces"); if !msaa && (self.edges || self.style == 4) { active.push("edge_quads"); } }
            if msaa && !self.pick_pending { active.push("antialias"); if ao_active || self.edges || self.style == 4 { active.push("antialias_edges"); } }
            if self.style == 1 { active.extend(["xray_accumulation", "xray_composite"]); }
            if ao_active && msaa { active.push("ao_inputs"); }
            if sun_drawn && self.detailed_timestamps { active.push("sun"); }
            if ao_active { active.extend(["ao_kernel", "ao_blur_x", "ao_blur_y", "ao_upsample_composite"]); }
            self.timings[k].active = active;
        }
        if msaa && needs_settled_ids(false, self.ids_dirty, self.stable_frames) { self.ensure_ids()?; }
        self.last_cpu = cpu;
        Ok((cpu, self.last_gpu))
    }
    /// Poll/readback, acquire, encode, submit, present, in milliseconds.
    fn frame_stage_timings(&self) -> Vec<f64> { self.frame_stages.to_vec() }
    fn request_capture(&mut self) {
        self.capture_next = true;
    }
    fn take_capture(&mut self) -> Option<Vec<u8>> {
        self.captured.take()
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
    /// The Sheets page layer: the ``w`` × ``h`` rectangle at ``x``, ``y`` of
    /// a ``width`` × ``height`` image (physical pixels, straight RGBA). The
    /// image is (re)made at that size first; False when the rectangle does
    /// not fit it.
    #[allow(clippy::too_many_arguments)]
    fn page_upload(&mut self, width: u32, height: u32, x: u32, y: u32, w: u32, h: u32,
                   rgba: PyReadonlyArray1<'_, u8>) -> PyResult<bool> {
        let bytes = rgba.as_slice()?;
        if width == 0 || height == 0 {
            return Err(PyValueError::new_err("page must have a size"));
        }
        self.page.ensure(&self.device, width, height);
        Ok(self.page.upload_region(&self.queue, x, y, w, h, bytes))
    }
    /// The page layer's image size, or None.
    fn page_size(&self) -> Option<(u32, u32)> { self.page.size() }
    /// Let the page layer go (leaving the workspace).
    fn page_clear(&mut self) { self.page.clear(); }
    /// A frame of the Sheets workspace: the desk, the page layer, the chrome
    /// over it. No scene pass runs; the resident scene stays as it is.
    #[pyo3(signature = (present=true))]
    fn render_page(&mut self, present: bool) -> PyResult<(f64, Option<f64>)> {
        let start = Instant::now();
        self.device.poll(wgpu::Maintain::Poll);
        let polled = Instant::now();
        let frame = if present { Some(self.surface.get_current_texture()
            .map_err(|e| PyValueError::new_err(e.to_string()))?) } else { None };
        let target = frame.as_ref().map(|f| &f.texture).unwrap_or(&self.offscreen);
        let acquired = Instant::now();
        self.last_acquire = acquired.duration_since(polled).as_secs_f64() * 1000.;
        let color = target.create_view(&Default::default());
        let mut e = self.device.create_command_encoder(&Default::default());
        {
            // The composer's desk (slate, 70 76 84) where the page has no pixels yet.
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
        self.page.draw(&mut e, &color, self.config.width, self.config.height, None);
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
            self.captured = Some(rgba);
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
    fn ui_set_tools(&mut self, groups: Vec<Vec<(String, String, String, usize, usize, Vec<u8>)>>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_tools(groups); }
    }
    fn ui_set_active(&mut self, id: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_active(id); }
    }
    fn ui_set_status(&mut self, status: String, hint: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_status(status, hint); }
    }
    /// ``kind``: move, press, release, leave; logical points. True when
    /// the chrome takes the event.
    fn ui_pointer(&mut self, kind: &str, x: f32, y: f32, button: u8, modifiers: u8) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.pointer(kind, x, y, button, modifiers))
    }
    fn ui_set_cursor_value(&mut self, caption: String, value: String, typing: bool, x: f32, y: f32) {
        if let Some(ui) = self.ui.as_mut() { ui.set_cursor_value(caption, value, typing, x, y); }
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
    fn ui_claims_key(&self, name: &str) -> bool { self.ui.as_ref().is_some_and(|ui| ui.claims_key(name)) }
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
        if let Some(ui) = self.ui.as_mut() { ui.panels.set(key, value); }
        Ok(())
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
    fn ui_set_tabs(&mut self, tabs: Vec<Vec<ui::Item>>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_tabs(tabs); }
    }
    fn ui_set_projects(&mut self, left: f32, docs: Vec<(String, bool, bool)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_projects(left, docs); }
    }
    fn ui_top_empty(&self, x: f32, y: f32) -> bool { self.ui.as_ref().is_some_and(|ui| ui.top_empty(x, y)) }
    /// The project tabs (views/wgpu_tabs.py, ui/tabs.rs): (id, name, front,
    /// unsaved, link source, tip, picture key), the projects in other
    /// windows (id, name, window), and the tab menu's two switches.
    #[pyo3(signature = (tabs, elsewhere=Vec::new(), can_move=false, pictures=true))]
    fn ui_set_project_tabs(&mut self, tabs: Vec<(String, String, bool, bool, bool, String, String)>,
                           elsewhere: Vec<(String, String, String)>, can_move: bool, pictures: bool) {
        if let Some(ui) = self.ui.as_mut() { ui.set_project_tabs(tabs, elsewhere, can_move, pictures); }
    }
    fn ui_project_tab_rects(&self) -> Vec<Option<(f32, f32, f32, f32)>> {
        self.ui.as_ref().map_or_else(Vec::new, |ui| ui.project_tab_rects())
    }
    /// The right-click menu at (x, y) (views/context_menu.py): header, then
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
    fn ui_plugin_panel(&mut self, raw: &str) -> PyResult<()> {
        if let Some(ui) = self.ui.as_mut() {
            ui.plugin_panel(raw).map_err(pyo3::exceptions::PyValueError::new_err)?;
        }
        Ok(())
    }
    fn ui_plugin_open(&mut self, instance: &str) {
        if let Some(ui) = self.ui.as_mut() { ui.plugin_open(instance); }
    }
    fn ui_plugin_document(&mut self, document: String, live: Vec<String>) {
        if let Some(ui) = self.ui.as_mut() { ui.plugin_document(document, &live); }
    }
    /// Work in progress: (id, what, phase, fraction or -1, cancellable).
    fn ui_set_jobs(&mut self, jobs: Vec<ui::Job>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_jobs(jobs); }
    }
    fn ui_set_activity(&mut self, history: Vec<ui::ActivityRow>, banner: String) {
        if let Some(ui) = self.ui.as_mut() { ui.set_activity(history, banner); }
    }
    fn ui_set_colset(&mut self, colset: u8) {
        if let Some(ui) = self.ui.as_mut() { ui.set_colset(colset); }
    }
    #[pyo3(signature = (types, active, view, sort, sorts, verbs, type_actions, empty, filter, rows))]
    #[allow(clippy::too_many_arguments)]
    fn ui_set_library(&mut self, types: Vec<(String, String, u32, bool)>, active: String, view: u8, sort: String,
                      sorts: Vec<(String, String)>, verbs: Vec<(String, String)>, type_actions: Vec<(String, String)>,
                      empty: String, filter: String, rows: Vec<(Vec<String>, Vec<i64>)>) {
        if let Some(ui) = self.ui.as_mut() {
            ui.set_library(types, active, view, sort, sorts, verbs, type_actions, empty, filter, rows);
        }
    }
    fn ui_set_dock_width(&mut self, width: f32) {
        if let Some(ui) = self.ui.as_mut() { ui.set_dock_width(width); }
    }
    fn ui_cursor_shape(&self) -> String {
        self.ui.as_ref().map_or(String::new(), |ui| ui.cursor_shape().to_owned())
    }
    fn ui_library_opened(&mut self) {
        if let Some(ui) = self.ui.as_mut() { ui.library_opened(); }
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
    /// A modal form (views/chrome_forms.py); an empty `seq` closes it.
    #[pyo3(signature = (seq, title="".to_string(), note="".to_string(), width=420.0, fields=Vec::new(), buttons=Vec::new()))]
    fn ui_set_dialog(&mut self, seq: String, title: String, note: String, width: f32,
                     fields: Vec<ui::dialog::Field>, buttons: Vec<ui::dialog::Button>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_dialog(seq, title, note, width, fields, buttons); }
    }
    fn ui_dialog_open(&self) -> bool { self.ui.as_ref().is_some_and(|ui| ui.dialog_open()) }
    fn ui_set_form(&mut self, title: String, fields: Vec<ui::FormField>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_form(title, fields); }
    }
    /// The start page (views/wgpu_start.py): shown or not, and whether a
    /// document waits behind it (Esc returns to it).
    fn ui_show_start(&mut self, shown: bool, can_return: bool) {
        if let Some(ui) = self.ui.as_mut() { ui.show_start(shown, can_return); }
    }
    fn ui_start_shown(&self) -> bool { self.ui.as_ref().is_some_and(|ui| ui.start_shown()) }
    /// The page's rows: files ([path, name, meta, project, thumb], [pinned,
    /// missing]); projects ([path, name, meta], [pinned, missing], files
    /// (path, name, role)); templates (id, name, description); continue
    /// [path, name, place, last in, problems, auto-save, thumb, open] or [];
    /// the engine footer; side sections and bottom links as (id, label).
    #[allow(clippy::too_many_arguments)]
    fn ui_set_start(&mut self, files: Vec<(Vec<String>, Vec<i64>)>,
                    projects: Vec<(Vec<String>, Vec<i64>, Vec<(String, String, String)>)>,
                    templates: Vec<(String, String, String)>, cont: Vec<String>, footer: String,
                    nav: Vec<(String, String)>, links: Vec<(String, String)>) {
        if let Some(ui) = self.ui.as_mut() { ui.set_start(files, projects, templates, cont, footer, nav, links); }
    }
    /// A key while the page is up: the key's name and its text. True: taken.
    fn ui_start_key(&mut self, name: &str, text: &str) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.start_key(name, text))
    }
    /// The large sheet (views/chrome_sheet.py): the whole sheet as JSON
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
    fn ui_open_commands(&mut self) {
        if let Some(ui) = self.ui.as_mut() { ui.open_commands(); }
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
        if !self.ids_dirty { return Ok(()); }
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
            let color = self.offscreen.create_view(&Default::default());
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
                    for k in 0..s.p as usize {
                        if s.template_data[k*20..k*20+4] == [0; 4] {
                            pass.draw_indirect(&s.commands, (s.p as usize+k) as u64 * 20);
                        }
                    }
                }
            }
            self.queue.submit(Some(encoder.finish()));
            self.ids_dirty = !ids_complete(cursor);
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
             &s.survivors, &s.viewpoints, &s.commands, &s.template, &s.hard_template]
                .iter().map(|b| b.size()).sum::<u64>() + 112 + 416
        }) + self.textures.bytes + self.textures.materials.size()
    }

    #[getter]
    fn pick_cache_valid(&self) -> bool { !self.ids_dirty }

    fn pick(&mut self, x: u32, y: u32) -> PyResult<i64> {
        if x >= self.config.width || y >= self.config.height {
            return Ok(-1);
        }
        let measure_id = self.ids_dirty && self.style != 4 && self.queries.is_some();
        self.ensure_ids_region(Some((x, y)))?;
        let mut e = self.device.create_command_encoder(&Default::default());
        e.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ids,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        if measure_id {
            let first = timing::index("id");
            e.resolve_query_set(self.queries.as_ref().unwrap(), first..first+2, &self.resolve, 0);
            e.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 8, 16);
        }
        self.queue.submit([e.finish()]);
        let b = self.readback(if measure_id { 24 } else { 4 })?;
        self.last_pick_gpu = measure_id.then(|| {
            let first = u64::from_le_bytes(b[8..16].try_into().unwrap());
            let last = u64::from_le_bytes(b[16..24].try_into().unwrap());
            last.saturating_sub(first) as f64 * self.queue.get_timestamp_period() as f64 / 1e6
        });
        Ok(u32::from_le_bytes(b[..4].try_into().unwrap()) as i64 - 1)
    }
}
impl Renderer {
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
        swap(&mut self.sections, &mut t.sections);
        swap(&mut self.camera_data, &mut t.camera_data);
        swap(&mut self.look.data, &mut t.look_data);
        swap(&mut self.style, &mut t.style);
        swap(&mut self.edges, &mut t.edges);
        swap(&mut self.textures_enabled, &mut t.textures_enabled);
        swap(&mut self.ids_dirty, &mut t.ids_dirty);
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
/// renderer (views/chrome_sheet.py tests): "" when it parses, else why.
#[pyfunction]
fn large_sheet_check(spec: &str) -> String {
    match serde_json::from_str::<ui::large_sheet::Spec>(spec) {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

#[pymodule]
pub fn ingetrazo_render(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(large_sheet_check, m)?)?;
    m.add_function(wrap_pyfunction!(panic_log::install_panic_log, m)?)?;
    m.add_function(wrap_pyfunction!(panic_log::_panic_for_test, m)?)?;
    m.add_class::<Renderer>()
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
