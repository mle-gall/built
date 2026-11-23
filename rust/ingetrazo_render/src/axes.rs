//! GL-compatible drawing axes: procedural one-physical-pixel lines, read-only depth.
use wgpu::util::DeviceExt;
pub struct Axes {
    pipelines: Vec<wgpu::RenderPipeline>,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    data: Vec<f32>,
    pub visible: bool,
    pub welcome_grid: bool,
}
impl Axes {
    pub fn new(d: &wgpu::Device, format: wgpu::TextureFormat, camera: &wgpu::Buffer) -> Self {
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("drawing axes"), source: wgpu::ShaderSource::Wgsl(include_str!("axes.wgsl").into()),
        });
        let bind_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None, entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry { binding,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None,
            bind_group_layouts: &[&bind_layout], push_constant_ranges: &[] });
        let pipelines = [1, 4].into_iter().map(|count| d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("drawing axes"), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false, depth_compare: crate::depth::compare(wgpu::CompareFunction::Less),
                stencil: Default::default(), bias: crate::depth::bias(wgpu::DepthBiasState { constant: 2, slope_scale: 1., clamp: 0. }) }),
            multisample: wgpu::MultisampleState { count, ..Default::default() }, multiview: None, cache: None,
        })).collect::<Vec<_>>();
        let uniform = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("axes frame"),
            contents: &[0; 80], usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera.as_entire_binding() },
                       wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() }] });
        Self { pipelines, group, uniform, data: Vec::new(), visible: false, welcome_grid: false }
    }
    pub fn set(&mut self, queue: &wgpu::Queue, frame: &[f32], spacing: f32, visible: bool) {
        self.visible = visible;
        let mut data = frame.to_vec();
        let grid_ink = egui::Rgba::from(crate::ui::tokens::OFF);
        data.extend([spacing, grid_ink.r(), grid_ink.g(), grid_ink.b()]);
        if self.data != data {
            queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&data));
            self.data = data;
        }
    }
    pub fn encode<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, msaa: bool) {
        if !self.visible && !self.welcome_grid { return; }
        pass.set_pipeline(&self.pipelines[usize::from(msaa)]);
        pass.set_bind_group(0, &self.group, &[]);
        if self.welcome_grid {
            // A quad for each of 33 lines in each direction on the drawing plane.
            pass.draw(0..6, 3..69);
        }
        if self.visible {
            pass.draw(0..3126, 0..3);
        }
    }
    pub fn draw(&self, e: &mut wgpu::CommandEncoder, color: &wgpu::TextureView,
                depth: &wgpu::TextureView, resolve: Option<&wgpu::TextureView>) {
        if !self.visible && !self.welcome_grid { return; }
        let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("drawing axes"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color, resolve_target: resolve,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None, occlusion_query_set: None,
        });
        self.encode(&mut pass, resolve.is_some());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn axes_validate_on_metal_at_both_sample_counts() {
        let instance = crate::backend::instance();
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else { return; };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let camera = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 112,
            usage: wgpu::BufferUsages::UNIFORM, mapped_at_creation: false });
        let mut axes = super::Axes::new(&device, wgpu::TextureFormat::Bgra8UnormSrgb, &camera);
        axes.set(&queue, &glam::Mat4::IDENTITY.to_cols_array(), 0.16, true);
        assert!(axes.visible);
        axes.set(&queue, &glam::Mat4::IDENTITY.to_cols_array(), 0.16, false);
        assert!(!axes.visible);
        assert_eq!(axes.data.len(), 20);
    }
}
