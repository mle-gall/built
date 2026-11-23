//! Composite Qt's straight-alpha RGBA pixels in the same surface as the scene.
pub struct Overlay {
    pipeline: wgpu::RenderPipeline,
    cue: wgpu::Buffer,
    cue_data: [f32; 4],
    image: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
}
impl Overlay {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
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
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        Self {
            pipeline,
            cue: crate::buffer(device, &[0; 16], wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            cue_data: [0.; 4],
            image: None,
        }
    }
    /// Project the acquired circle in physical pixels, testing its reach in logical pixels.
    pub fn center_cue(&mut self, queue: &wgpu::Queue, camera: &[f32], size: [u32; 2],
                      center: Option<[f32; 3]>, radius: f32, cursor: [f32; 2], scale: f32) {
        use glam::{Mat4, Vec2, Vec3};
        let cue = center.and_then(|center| {
            let vp = Mat4::from_cols_array(camera.get(..16)?.try_into().ok()?);
            let size = Vec2::new(size[0] as f32, size[1] as f32);
            let pixel = |p: Vec3| {
                let clip = vp * p.extend(1.);
                (clip.w > 0.).then(|| (clip.truncate().truncate() / clip.w * Vec2::new(1., -1.) + 1.) * 0.5 * size)
            };
            let center = Vec3::from_array(center);
            let pc = pixel(center)?;
            let projected = [Vec3::X, Vec3::Y].into_iter()
                .filter_map(|axis| pixel(center + radius * axis))
                .map(|rim| pc.distance(rim)).fold(0., f32::max);
            let distance = pc.distance(Vec2::from_array(cursor) * scale);
            (projected >= 12. * scale && distance <= (40. * scale).min(projected * 0.5))
                .then_some([pc.x, pc.y, scale, 1.])
        }).unwrap_or([0.; 4]);
        if self.cue_data != cue {
            queue.write_buffer(&self.cue, 0, bytemuck::cast_slice(&cue));
            self.cue_data = cue;
        }
    }

    pub fn upload(&mut self, d: &wgpu::Device, q: &wgpu::Queue, w: u32, h: u32, rgba: &[u8]) {
        self.ensure(d, w, h);
        self.upload_region(q, 0, 0, w, h, rgba);
    }
    /// Give the layer a ``w`` × ``h`` image (kept when it already is that size).
    pub fn ensure(&mut self, d: &wgpu::Device, w: u32, h: u32) {
        if self.image.as_ref().map(|v| (v.2, v.3)) != Some((w, h)) {
            let texture = d.create_texture(&wgpu::TextureDescriptor {
                label: Some("overlay RGBA"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 1, resource: self.cue.as_entire_binding(),
                }, wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            });
            self.image = Some((texture, group, w, h));
        }
    }
    /// Replace the ``w`` × ``h`` rectangle at ``x``, ``y`` of the image.
    /// False when it does not fit the image or the pixels do not match the
    /// rectangle.
    pub fn upload_region(&mut self, q: &wgpu::Queue, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) -> bool {
        let Some((texture, _, iw, ih)) = self.image.as_ref() else { return false };
        if w == 0 || h == 0 || x + w > *iw || y + h > *ih || rgba.len() as u64 != w as u64 * h as u64 * 4 {
            return false;
        }
        q.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        true
    }
    pub fn draw(&self, e: &mut wgpu::CommandEncoder, color: &wgpu::TextureView, w: u32, h: u32, queries: Option<&wgpu::QuerySet>) {
        if let Some((_, group, iw, ih)) = &self.image {
            if (*iw, *ih) != (w, h) {
                return;
            }
            let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: crate::timing::writes(queries, "overlay"),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
