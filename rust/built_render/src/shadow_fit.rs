//! Once a view settles, read just four bounds from a subsample of its depth.
use glam::{Mat4, Vec2, Vec3};
use std::sync::mpsc;

pub fn projection(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::ComputePipeline,
    depth: &wgpu::Texture,
    camera: &[f32],
    light: &[f32; 16],
) -> Option<[f32; 16]> {
    let inverse = Mat4::from_cols_array(camera.get(..16)?.try_into().ok()?).inverse();
    let matrix = Mat4::from_cols_array(light);
    let transform = crate::buffer(
        device,
        bytemuck::cast_slice(&(matrix * inverse).to_cols_array()),
        wgpu::BufferUsages::UNIFORM,
    );
    let bounds = crate::buffer(
        device,
        bytemuck::cast_slice(&[i32::MAX, i32::MAX, i32::MIN, i32::MIN]),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("visible shadow bounds"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &depth.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: transform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: bounds.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(depth.width().div_ceil(64), depth.height().div_ceil(64), 1);
    }
    encoder.copy_buffer_to_buffer(&bounds, 0, &read, 0, 16);
    queue.submit([encoder.finish()]);
    let (send, recv) = mpsc::channel();
    read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = send.send(r);
    });
    device.poll(wgpu::Maintain::Wait);
    recv.recv().ok()?.ok()?;
    let bytes = read.slice(..).get_mapped_range();
    let v: &[i32] = bytemuck::cast_slice(&bytes);
    if v[0] > v[2] {
        return None;
    }
    let lo = Vec2::new(v[0] as f32, v[1] as f32) / 1048576.;
    let hi = Vec2::new(v[2] as f32, v[3] as f32) / 1048576.;
    let center = (lo + hi) * 0.5;
    let margin = 0.1 / crate::sun::footprint(light)[0];
    let half = (hi - lo).max_element() * 0.54 + margin;
    if !(0. ..0.9).contains(&half) {
        return None;
    }
    let crop = Mat4::from_scale(Vec3::new(1. / half, 1. / half, 1.))
        * Mat4::from_translation(-center.extend(0.));
    Some((crop * matrix).to_cols_array())
}
