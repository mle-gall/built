//! Frosted glass for the chrome (UI spike). Each frame the finished model
//! image is copied and blurred (dual Kawase: down four halvings, up three
//! doublings, half resolution at the end); every panel then draws that
//! blur through its rounded shape inside egui's own pass, by a paint
//! callback, before egui draws the panel's content on top.

use std::sync::atomic::{AtomicU32, Ordering};

const SLOTS: u32 = 64;
const SLOT: u64 = 256;
static NEXT: AtomicU32 = AtomicU32::new(0);

/// How one surface looks through the glass (the design tokens' surfaces).
#[derive(Clone, Copy)]
pub struct Look {
    pub tint: [f32; 4],
    pub saturation: f32,
    /// Lit and quiet rim strength.
    pub rim: [f32; 2],
    /// Rim width in physical pixels (stays fine at 2×).
    pub rim_width: f32,
    /// The narrow highlight along the top edge.
    pub glow: f32,
    /// Brightest the backdrop may show (linear), soft shoulder.
    pub peak: f32,
    pub inner_shadow: f32,
    pub vignette: f32,
}

struct Level {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

pub struct Glass {
    format: wgpu::TextureFormat,
    size: [u32; 2],
    copy: Option<wgpu::Texture>,
    levels: Vec<Level>,
    ups: Vec<Level>,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    steps: Vec<(wgpu::Buffer, [u32; 2])>,
    blur_scale: f32,
    preview: Option<(egui::TextureId, wgpu::BindGroup)>,
    passes: Vec<(wgpu::BindGroup, usize, bool)>,
}

/// What the callbacks need, kept in egui's callback resources.
pub struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    bind: Option<wgpu::BindGroup>,
}

fn blur_target(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2]) -> Level {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glass level"),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    Level { view: tex.create_view(&Default::default()), texture: tex, size }
}

fn layout(device: &wgpu::Device, dynamic: bool) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("glass"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: dynamic,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn pipeline(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, module: &wgpu::ShaderModule,
            fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None, bind_group_layouts: &[layout], push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fs),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module, entry_point: Some(fs), compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

fn step_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler,
              src: &wgpu::TextureView, src_size: [u32; 2], blur_scale: f32, crop: [f32; 2]) -> (wgpu::BindGroup, wgpu::Buffer) {
    let texel = [0.5 * blur_scale / src_size[0] as f32, 0.5 * blur_scale / src_size[1] as f32, crop[0], crop[1]];
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("glass step"), size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: true,
    });
    buf.slice(..).get_mapped_range_mut().copy_from_slice(bytemuck::cast_slice(&texel));
    buf.unmap();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    (group, buf)
}

impl Glass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> (Self, Resources) {
        let blur = device.create_shader_module(wgpu::include_wgsl!("blur.wgsl"));
        let glass = device.create_shader_module(wgpu::include_wgsl!("glass.wgsl"));
        let blur_layout = layout(device, false);
        let pane_layout = layout(device, true);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let resources = Resources {
            pipeline: pipeline(device, &pane_layout, &glass, "fs", format, Some(premultiplied)),
            layout: pane_layout,
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("glass panes"),
                size: SLOT * SLOTS as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            bind: None,
        };
        let glass = Self {
            format,
            size: [0, 0],
            copy: None,
            levels: Vec::new(),
            ups: Vec::new(),
            down: pipeline(device, &blur_layout, &blur, "down", format, None),
            up: pipeline(device, &blur_layout, &blur, "up", format, None),
            blur_layout,
            sampler,
            steps: Vec::new(),
            passes: Vec::new(),
            blur_scale: 1.0,
            preview: None,
        };
        (glass, resources)
    }

    fn resize(&mut self, device: &wgpu::Device, size: [u32; 2], res: &mut Resources) {
        self.size = size;
        let copy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glass copy"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let copy_view = copy.create_view(&Default::default());
        self.copy = Some(copy);
        let half = |s: [u32; 2]| [(s[0] / 2).max(1), (s[1] / 2).max(1)];
        let mut s = size;
        self.levels = (0..5).map(|_| { s = half(s); blur_target(device, self.format, s) }).collect();
        // Up: 1/16, 1/8, 1/4, 1/2.
        self.ups = (0..4).rev().map(|i| blur_target(device, self.format, self.levels[i].size)).collect();
        self.passes.clear();
        self.steps.clear();
        // Down: copy → L0 → L1 → L2 → L3; up: L3 → U0(1/8) → U1(1/4) → U2(1/2).
        let mut plan: Vec<(&wgpu::TextureView, [u32; 2], usize, bool)> = vec![(&copy_view, size, 0, false)];
        for i in 1..5 {
            plan.push((&self.levels[i - 1].view, self.levels[i - 1].size, i, false));
        }
        plan.push((&self.levels[4].view, self.levels[4].size, 0, true));
        for i in 1..4 {
            plan.push((&self.ups[i - 1].view, self.ups[i - 1].size, i, true));
        }
        let mut passes = Vec::new();
        let mut steps = Vec::new();
        for (src, src_size, target, up) in plan {
            let (group, buf) = step_group(device, &self.blur_layout, &self.sampler, src, src_size, self.blur_scale, [1.0; 2]);
            passes.push((group, target, up));
            steps.push((buf, src_size));
        }
        self.passes = passes;
        self.steps = steps;
        res.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass panes"),
            layout: &res.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &res.uniforms, offset: 0, size: wgpu::BufferSize::new(80),
                    }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.ups[3].view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        }));
    }

    /// Reuse the model-only downsample already computed for the window chrome.
    /// A tab switch copies at most one small texture without drawing or reading pixels back.
    pub fn tab_picture(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<wgpu::Texture> {
        let level = self.levels.iter().find(|level| level.size[0].max(level.size[1]) <= 528)?;
        let source = &level.texture;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("background document tab"),
            size: source.size(),
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[self.format.remove_srgb_suffix()],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(source.as_image_copy(), texture.as_image_copy(), source.size());
        queue.submit([encoder.finish()]);
        Some(texture)
    }

    pub fn showing_preview(&self) -> bool { self.preview.is_some() }

    /// Blur the model or the centre-cropped recent preview with the same passes.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, e: &mut wgpu::CommandEncoder, frame: &wgpu::Texture,
                   res: &mut Resources, blur_scale: f32, preview: Option<(egui::TextureId, wgpu::Texture)>) {
        NEXT.store(0, Ordering::Relaxed);
        let size = [frame.width(), frame.height()];
        if size != self.size || self.copy.is_none() {
            self.blur_scale = blur_scale;
            self.preview = None;
            self.resize(device, size, res);
        } else if self.blur_scale != blur_scale {
            self.blur_scale = blur_scale;
            self.preview = None;
            for (uniform, [w, h]) in &self.steps {
                let step = [0.5 * blur_scale / *w as f32, 0.5 * blur_scale / *h as f32];
                queue.write_buffer(uniform, 0, bytemuck::cast_slice(&step));
            }
        }
        let Some(copy) = &self.copy else { return };
        if let Some((id, texture)) = preview {
            if self.preview.as_ref().map(|(key, _)| *key) != Some(id) {
                let aspect = size[0] as f32 / size[1] as f32;
                let source = texture.width() as f32 / texture.height() as f32;
                let crop = [(aspect / source).min(1.0), (source / aspect).min(1.0)];
                let view = texture.create_view(&Default::default());
                let (group, _) = step_group(device, &self.blur_layout, &self.sampler, &view,
                    [texture.width(), texture.height()], blur_scale, crop);
                self.preview = Some((id, group));
            }
        } else {
            self.preview = None;
        }
        if self.preview.is_none() {
            e.copy_texture_to_texture(frame.as_image_copy(), copy.as_image_copy(),
                wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 });
        }
        for (i, (group, target, up)) in self.passes.iter().enumerate() {
            let group = if i == 0 { self.preview.as_ref().map_or(group, |(_, g)| g) } else { group };
            let view = if *up { &self.ups[*target].view } else { &self.levels[*target].view };
            let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(if *up { &self.up } else { &self.down });
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// One panel's glass, as an egui paint callback over ``rect``.
pub struct Pane {
    slot: u32,
    rect: egui::Rect,
    radius: f32,
    look: Look,
}

pub fn pane(rect: egui::Rect, radius: f32, look: Look) -> egui::Shape {
    let slot = NEXT.fetch_add(1, Ordering::Relaxed) % SLOTS;
    // The callback's own rect is grown a little so the outer line fits.
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(
        rect.expand(2.0), Pane { slot, rect, radius, look }))
}

impl egui_wgpu::CallbackTrait for Pane {
    fn prepare(&self, _device: &wgpu::Device, queue: &wgpu::Queue, screen: &egui_wgpu::ScreenDescriptor,
               _e: &mut wgpu::CommandEncoder, resources: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get::<Resources>() else { return Vec::new() };
        let ppp = screen.pixels_per_point;
        let r = self.rect;
        let l = self.look;
        let data: [f32; 20] = [
            r.min.x * ppp, r.min.y * ppp, r.max.x * ppp, r.max.y * ppp,
            self.radius * ppp, screen.size_in_pixels[0] as f32, screen.size_in_pixels[1] as f32, l.saturation,
            l.tint[0], l.tint[1], l.tint[2], l.tint[3],
            l.rim[0], l.rim[1], l.rim_width, l.glow,
            l.peak, ppp, l.inner_shadow, l.vignette,
        ];
        queue.write_buffer(&res.uniforms, self.slot as u64 * SLOT, bytemuck::cast_slice(&data));
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>,
             resources: &egui_wgpu::CallbackResources) {
        let Some(res) = resources.get::<Resources>() else { return };
        let Some(bind) = &res.bind else { return };
        pass.set_pipeline(&res.pipeline);
        pass.set_bind_group(0, bind, &[(self.slot as u64 * SLOT) as u32]);
        pass.draw(0..3, 0..1);
    }
}
