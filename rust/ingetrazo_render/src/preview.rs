//! Tool-owned triangles/edges and loose-face marks; empty by default.
use wgpu::util::DeviceExt;
pub struct Preview {
    samples: crate::antialias::SampleModes,
    pipelines: Vec<wgpu::RenderPipeline>,
    group: wgpu::BindGroup,
    mark_pose: wgpu::Buffer,
    triangles: Option<wgpu::Buffer>,
    edges: Option<wgpu::Buffer>,
    counts: [u32; 2],
    pub textures: Option<crate::preview_texture::TexturedPreview>,
}
impl Preview {
    pub fn new(samples: crate::antialias::SampleModes,
        d: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera: &wgpu::Buffer,
        style: &wgpu::Buffer,
        kind: u32,
    ) -> Self {
        let marks = kind == 1;
        let ghost = kind == 2;
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tool previews"),
            source: wgpu::ShaderSource::Wgsl(include_str!("preview.wgsl").into()),
        });
        let bindings = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[0, 1, 2].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bindings],
            push_constant_ranges: &[],
        });
        let pipelines = samples.levels.iter().copied().flat_map(|samples| [false,true].map(|edge| {
            let constants = [("MARK".to_string(), if marks {1.} else {0.}), ("EDGE".to_string(), if edge {1.} else {0.}), ("GHOST".to_string(), if ghost {1.} else {0.})].into_iter().collect();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some("depth-tested tool preview"), layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                    buffers: &[wgpu::VertexBufferLayout { array_stride: 28, step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4] }] },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })] }),
                primitive: wgpu::PrimitiveState { topology: if edge { wgpu::PrimitiveTopology::LineList } else { wgpu::PrimitiveTopology::TriangleList }, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: !marks && !ghost && !edge,
                    depth_compare: crate::depth::compare(wgpu::CompareFunction::LessEqual), stencil: Default::default(),
                    bias: crate::depth::bias(wgpu::DepthBiasState { constant: if marks || edge {-2} else {0}, slope_scale: if marks || edge {-1.} else {0.}, clamp: 0. }) }),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() }, multiview: None, cache: None })
        })).collect();
        let mark_pose = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selection mark pose"), contents: &[0u8; 80],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let group = crate::bind(d, &bindings, &[camera, style, &mark_pose]);
        Self { samples,
            pipelines,
            group,
            mark_pose,
            triangles: None,
            edges: None,
            counts: [0; 2],
            textures: None,
        }
    }
    pub fn validate(triangles: &[u8], edges: &[u8]) -> Result<(), &'static str> {
        if triangles.len() % 84 != 0 || edges.len() % 56 != 0 {
            return Err("preview requires 28-byte triangle vertices and edge pairs");
        }
        for vertex in triangles.chunks_exact(28).chain(edges.chunks_exact(28)) {
            if vertex[..24]
                .chunks_exact(4)
                .any(|v| !f32::from_le_bytes(v.try_into().unwrap()).is_finite())
            {
                return Err("nonfinite preview vertex");
            }
        }
        Ok(())
    }
    pub fn upload(
        &mut self,
        d: &wgpu::Device,
        triangles: &[u8],
        edges: &[u8],
    ) -> Result<(), &'static str> {
        Self::validate(triangles, edges)?;
        self.textures = None;
        self.upload_validated(d, triangles, edges);
        Ok(())
    }

    pub fn upload_validated(&mut self, d: &wgpu::Device, triangles: &[u8], edges: &[u8]) {
        self.set_mark_faces(d, triangles);
        self.set_mark_edges(d, edges);
    }
    fn buffer(d: &wgpu::Device, data: &[u8]) -> Option<wgpu::Buffer> {
        (!data.is_empty()).then(|| d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("transient mesh"), contents: data, usage: wgpu::BufferUsages::VERTEX,
        }))
    }
    pub fn set_mark_faces(&mut self, d: &wgpu::Device, data: &[u8]) {
        self.triangles = Self::buffer(d, data);
        self.counts[0] = (data.len() / 28) as u32;
    }
    pub fn set_mark_edges(&mut self, d: &wgpu::Device, data: &[u8]) {
        self.edges = Self::buffer(d, data);
        self.counts[1] = (data.len() / 28) as u32;
    }
    pub fn pose_marks(&self, queue: &wgpu::Queue, matrix: [f32; 16], selected: u32) {
        let mut data = [0u8; 80];
        data[..64].copy_from_slice(bytemuck::bytes_of(&matrix));
        data[64..68].copy_from_slice(&selected.to_le_bytes());
        queue.write_buffer(&self.mark_pose, 0, &data);
    }
    pub fn layout(&self) -> wgpu::BindGroupLayout { self.pipelines[0].get_bind_group_layout(0) }
    pub fn active(&self) -> bool {
        self.counts != [0; 2] || self.textures.as_ref().is_some_and(|t| t.active())
    }
    pub fn draw(
        &self,
        e: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
    ) {
        if !self.active() {
            return;
        }
        let mut p = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tool preview"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        p.set_bind_group(0, &self.group, &[]);
        if let Some(textures) = &self.textures { textures.draw(&mut p, resolve.is_some()); }
        for (k, buffer) in [&self.triangles, &self.edges].into_iter().enumerate() {
            if let Some(buffer) = buffer {
                p.set_pipeline(&self.pipelines[self.samples.index(resolve.is_some()) * 2 + k]);
                p.set_vertex_buffer(0, buffer.slice(..));
                p.draw(0..self.counts[k], 0..1);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn preview_and_marks_validate_on_metal_both_samples() {
        let instance = crate::backend::instance();
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            return;
        };
        let (d, _) = pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let camera = crate::buffer(&d, &[0; 112], wgpu::BufferUsages::UNIFORM);
        let style = crate::style::Style::new(&d);
        for kind in [0, 1, 2] {
            let mut preview = super::Preview::new(crate::antialias::SampleModes::test(),
                &d,
                wgpu::TextureFormat::Bgra8UnormSrgb,
                &camera,
                &style.buffer,
                kind,
            );
            assert!(preview.upload(&d, &[0; 83], &[]).is_err());
            preview.upload(&d, &[0; 84], &[0; 56]).unwrap();
            assert_eq!(preview.counts, [3, 2]);
            assert_eq!(preview.pipelines.len(), 4);
            preview.upload(&d, &[], &[]).unwrap();
            assert_eq!(preview.counts, [0, 0]);
        }
    }
}
