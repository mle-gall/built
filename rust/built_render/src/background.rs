//! Draw the GL style's hazed sky/ground ramp before geometry, without depth or IDs.

/// The style uniform's float that asks for the flat background (`ground.a`):
/// row 5, lane 3 of the look data.
pub const FLAT_SLOT: (usize, usize) = (5, 3);

/// Does a camera (column-major view and projection matrices) show a
/// parallel view along a model axis? Its picture then has no horizon: Top,
/// Front, Right and the others draw on the flat background (M32).
pub fn flat_view(view: &[f32; 16], projection: &[f32; 16]) -> bool {
    // A perspective projection divides by depth: its last element is 0.
    if projection[15] == 0. {
        return false;
    }
    // The view's third row is the camera's back direction in the model.
    let back = [view[2], view[6], view[10]];
    back.iter().any(|c| c.abs() > 1. - 1e-4)
}

#[cfg(test)]
mod tests {
    use super::flat_view;
    fn look(back: glam::Vec3, perspective: bool) -> bool {
        let up = if back.z.abs() > 0.9 { glam::Vec3::Y } else { glam::Vec3::Z };
        let view = glam::Mat4::look_at_rh(back * 10., glam::Vec3::ZERO, up);
        let projection = if perspective {
            glam::Mat4::perspective_rh(0.8, 1.5, 0.1, 100.)
        } else {
            glam::Mat4::orthographic_rh(-5., 5., -3., 3., 0.1, 100.)
        };
        flat_view(&view.to_cols_array(), &projection.to_cols_array())
    }
    #[test]
    fn parallel_axis_views_are_flat_and_the_rest_keep_the_horizon() {
        for back in [glam::Vec3::Z, -glam::Vec3::Y, glam::Vec3::X, -glam::Vec3::Z] {
            assert!(look(back, false), "{back:?}");
            assert!(!look(back, true), "{back:?} in perspective");
        }
        assert!(!look(glam::Vec3::new(1., -1., 0.8).normalize(), false));
    }
}
pub struct Background {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
}
impl Background {
    pub fn new(
        d: &wgpu::Device,
        format: wgpu::TextureFormat,
        style: &wgpu::Buffer,
        samples: u32,
    ) -> Self {
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("style background"),
            source: wgpu::ShaderSource::Wgsl(include_str!("background.wgsl").into()),
        });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("style background"),
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });
        let group = crate::bind(d, &pipeline.get_bind_group_layout(0), &[style]);
        Self { pipeline, group }
    }
    pub fn draw(&self, e: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, queries: Option<&wgpu::QuerySet>) {
        let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("style background"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: crate::timing::writes(queries, "background"),
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}
