//! Depth-tested world-space scaffold lines: the open group's dashed box,
//! guide lines and a selected group's hidden edges.
//!
//! GL draws these as GL_LINES inside its scene pass, depth-tested against
//! the faces, with a dash measured in window pixels along each line
//! (`u_stipple 4`). The overlay has no depth, so the parts behind faces
//! showed through on Metal; the native preview edges have depth but cannot
//! dash. Here every segment carries its own colour, width and screen-space
//! dash, so the camera never forces a re-upload: only the segments do.
//!
//! Segments live in named slots, so the edit box, the guides and the hidden
//! edges change independently: one slot's upload never resends another's.
//!
//! GL's order decides who wins where a line and a model edge cover the
//! same pixels: it draws the box and the guides (`_draw_scaffold`) after
//! the faces but before the scene edges, which paint over them, and the
//! selected group's hidden edges after the scene edges. A slot is therefore
//! drawn either under the edges (between the face and the edge passes) or
//! above them (after the edges); depth against the faces is the same.
use std::collections::BTreeMap;

/// a (3 f32), b (3 f32), RGBA8 colour, width, dash, gap (f32 pixels).
pub const SEGMENT_BYTES: usize = 40;

struct Slot {
    buffer: wgpu::Buffer,
    count: u32,
    /// Drawn after the scene edges (GL's selected hidden edges) instead of
    /// before them (GL's scaffold: box, guides).
    above_edges: bool,
}

pub struct Lines {
    samples: crate::antialias::SampleModes,
    pipelines: Vec<wgpu::RenderPipeline>,
    group: wgpu::BindGroup,
    slots: BTreeMap<String, Slot>,
    /// Off for sheet captures (`render_image`): scaffolding is never printed.
    pub enabled: bool,
}

/// Reject what would draw garbage before it reaches the GPU.
pub fn validate(data: &[u8]) -> Result<u32, &'static str> {
    if data.len() % SEGMENT_BYTES != 0 {
        return Err("lines need 40-byte segments");
    }
    for segment in data.chunks_exact(SEGMENT_BYTES) {
        let float = |at: usize| f32::from_le_bytes(segment[at..at + 4].try_into().unwrap());
        if [0, 4, 8, 12, 16, 20, 28, 32, 36].iter().any(|&at| !float(at).is_finite()) {
            return Err("nonfinite line segment");
        }
        if float(28) < 0. || float(32) < 0. || float(36) < 0. {
            return Err("line width and dash must not be negative");
        }
    }
    u32::try_from(data.len() / SEGMENT_BYTES).map_err(|_| "too many line segments")
}

impl Lines {
    pub fn new(samples: crate::antialias::SampleModes, d: &wgpu::Device, format: wgpu::TextureFormat, camera: &wgpu::Buffer) -> Self {
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scaffold lines"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lines.wgsl").into()),
        });
        let bindings = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bindings],
            push_constant_ranges: &[],
        });
        let pipelines = samples.levels.iter().copied().map(|samples| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("depth-tested scaffold lines"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: SEGMENT_BYTES as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4, 3 => Float32x3],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                // The scene edges' bias: GL offsets its faces back by
                // glPolygonOffset(1, 1), so a line lying on a face wins
                // LEQUAL; a line behind one loses. No depth writes:
                // scaffolding must not hide anything.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: false,
                    depth_compare: crate::depth::compare(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: crate::depth::bias(wgpu::DepthBiasState { constant: -2, slope_scale: -1., clamp: 0. }),
                }),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                multiview: None,
                cache: None,
            })
        }).collect();
        let group = crate::bind(d, &bindings, &[camera]);
        Self { samples, pipelines, group, slots: BTreeMap::new(), enabled: true }
    }

    /// Replace one slot's segments. The buffer is reused while it is large
    /// enough, so a changing slot (guides while orbiting) writes in place.
    pub fn upload(
        &mut self,
        d: &wgpu::Device,
        q: &wgpu::Queue,
        slot: &str,
        data: &[u8],
        above_edges: bool,
    ) -> Result<(), &'static str> {
        let count = validate(data)?;
        if count == 0 {
            if let Some(s) = self.slots.get_mut(slot) {
                s.count = 0;
            }
            return Ok(());
        }
        match self.slots.get_mut(slot) {
            Some(s) if s.buffer.size() >= data.len() as u64 => {
                q.write_buffer(&s.buffer, 0, data);
                s.count = count;
                s.above_edges = above_edges;
            }
            _ => {
                let buffer = d.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("scaffold lines"),
                    size: (data.len() as u64).next_power_of_two(),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                q.write_buffer(&buffer, 0, data);
                self.slots.insert(slot.to_string(), Slot { buffer, count, above_edges });
            }
        }
        Ok(())
    }

    pub fn counts(&self) -> Vec<(String, u32)> {
        self.slots.iter().map(|(k, s)| (k.clone(), s.count)).collect()
    }

    pub fn active(&self) -> bool {
        self.enabled && self.slots.values().any(|s| s.count > 0)
    }

    /// Whether anything draws in one layer: the MSAA colour must outlive
    /// the edge pass only for the slots drawn after it.
    pub fn active_layer(&self, above_edges: bool) -> bool {
        self.enabled && self.slots.values().any(|s| s.count > 0 && s.above_edges == above_edges)
    }

    pub fn draw(
        &self,
        e: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        above_edges: bool,
    ) {
        if !self.active_layer(above_edges) {
            return;
        }
        let mut p = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scaffold lines"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: resolve,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        p.set_pipeline(&self.pipelines[self.samples.index(resolve.is_some())]);
        p.set_bind_group(0, &self.group, &[]);
        for s in self.slots.values().filter(|s| s.count > 0 && s.above_edges == above_edges) {
            p.set_vertex_buffer(0, s.buffer.slice(..u64::from(s.count) * SEGMENT_BYTES as u64));
            p.draw(0..6, 0..s.count);
        }
    }
}

#[cfg(test)]
mod tests {
    fn segment(a: [f32; 3], b: [f32; 3], style: [f32; 3]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(bytemuck::cast_slice(&a));
        v.extend_from_slice(bytemuck::cast_slice(&b));
        v.extend_from_slice(&[90, 110, 140, 255]);
        v.extend_from_slice(bytemuck::cast_slice(&style));
        v
    }

    #[test]
    fn validates_layout_and_values() {
        assert_eq!(super::validate(&[]), Ok(0));
        assert!(super::validate(&[0; 39]).is_err());
        let ok = segment([0.; 3], [1., 0., 0.], [1., 4., 4.]);
        assert_eq!(super::validate(&ok), Ok(1));
        assert!(super::validate(&segment([f32::NAN, 0., 0.], [1., 0., 0.], [1., 0., 0.])).is_err());
        assert!(super::validate(&segment([0.; 3], [1., 0., 0.], [1., f32::INFINITY, 0.])).is_err());
        assert!(super::validate(&segment([0.; 3], [1., 0., 0.], [-1., 0., 0.])).is_err());
    }

    #[test]
    fn dash_phase_matches_gl_stipple() {
        // basic.frag: mod(distance, 2·dash) >= dash discards; lines.wgsl
        // generalises the gap. Equal dash and gap must agree with GL.
        let dash = 4f32;
        for along in [0., 1.5, 3.99, 4., 6., 7.99, 8., 12.5] {
            let gl = along % (2. * dash) >= dash;
            let period = dash + dash;
            let ours = along - (along / period).floor() * period >= dash;
            assert_eq!(gl, ours, "{along}");
        }
    }

    #[test]
    fn slots_reuse_buffers_and_validate_on_metal_both_samples() {
        let instance = crate::backend::instance();
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            return;
        };
        let (d, q) = pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let camera = crate::buffer(&d, &[0; 112], wgpu::BufferUsages::UNIFORM);
        let mut lines = super::Lines::new(crate::antialias::SampleModes::test(), &d, wgpu::TextureFormat::Bgra8UnormSrgb, &camera);
        assert!(!lines.active());
        let one = segment([0.; 3], [1., 0., 0.], [1., 4., 4.]);
        let three = [one.clone(), one.clone(), one.clone()].concat();
        lines.upload(&d, &q, "box", &three, false).unwrap();
        lines.upload(&d, &q, "guides", &one, false).unwrap();
        assert!(lines.active());
        assert!(lines.active_layer(false) && !lines.active_layer(true));
        let size = lines.slots["box"].buffer.size();
        lines.upload(&d, &q, "box", &one, false).unwrap();
        assert_eq!(lines.slots["box"].buffer.size(), size, "a smaller upload reuses the buffer");
        assert_eq!(lines.counts(), vec![("box".into(), 1), ("guides".into(), 1)]);
        assert!(lines.upload(&d, &q, "box", &one[..39], false).is_err());
        assert_eq!(lines.slots["box"].count, 1, "a rejected upload keeps the old segments");
        lines.upload(&d, &q, "hidden", &one, true).unwrap();
        assert!(lines.active_layer(true));
        lines.upload(&d, &q, "box", &[], false).unwrap();
        lines.upload(&d, &q, "guides", &[], false).unwrap();
        assert!(lines.active() && !lines.active_layer(false), "only the layer above the edges is left");
        lines.upload(&d, &q, "hidden", &[], true).unwrap();
        assert!(!lines.active());
        lines.upload(&d, &q, "box", &one, false).unwrap();
        lines.enabled = false;
        assert!(!lines.active() && !lines.active_layer(false));
        // Both pipelines record against matching targets without validation errors.
        lines.enabled = true;
        let target = |samples, format| d.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: 8, height: 8, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] });
        let resolved = target(1, wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut e = d.create_command_encoder(&Default::default());
        lines.draw(&mut e, &resolved.create_view(&Default::default()),
                   &target(1, wgpu::TextureFormat::Depth32Float).create_view(&Default::default()), None, false);
        lines.draw(&mut e, &target(4, wgpu::TextureFormat::Bgra8UnormSrgb).create_view(&Default::default()),
                   &target(4, wgpu::TextureFormat::Depth32Float).create_view(&Default::default()),
                   Some(&resolved.create_view(&Default::default())), false);
        q.submit([e.finish()]);
        d.poll(wgpu::Maintain::Wait);
    }
}
