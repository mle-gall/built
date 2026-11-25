//! Read back a sloped silhouette so samples cover different fractions of each edge pixel.
use super::{Antialias, SampleModes};

#[test]
fn fallback_uses_highest_available_level() {
    for (levels, expected) in [(vec![1], 1), (vec![1, 2], 2)] {
        let samples = SampleModes::new(levels);
        samples.select(4);
        assert_eq!(samples.count(), expected);
    }
}

#[test]
fn resolved_edge_coverage() {
    let instance = crate::backend::instance();
    let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let features = adapter.features() & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        },
        None,
    ))
    .unwrap();
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let modes = SampleModes::adapter(&adapter, format, features);
    eprintln!(
        "{} ({:?}) supports {:?}",
        adapter.get_info().name,
        adapter.get_info().backend,
        modes.levels
    );
    let style = crate::style::Style::new(&device);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("slanted edge"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@vertex fn vs(@builtin(vertex_index) i:u32) -> @builtin(position) vec4f {
    var points=array<vec2f,3>(vec2f(-0.91,-0.83), vec2f(0.87,-0.64), vec2f(-0.55,0.97));
    return vec4f(points[i],0.,1.);
}
@fragment fn fs() -> @location(0) vec4f {return vec4f(1.);}
"#
            .into(),
        ),
    });
    for &count in modes.levels.iter() {
        modes.select(count);
        let aa = Antialias::new(modes.count(), &device, format, &style.buffer, 64, 64);
        let output = crate::texture(&device, 64, 64, format);
        let target = output.create_view(&Default::default());
        let multi = aa.color_view();
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count,
                ..Default::default()
            },
            primitive: Default::default(),
            depth_stencil: None,
            multiview: None,
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if count > 1 { &multi } else { &target },
                    resolve_target: (count > 1).then_some(&target),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            size: 64 * 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            label: None,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    bytes_per_row: Some(256),
                    ..Default::default()
                },
            },
            output.size(),
        );
        queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let data = read.slice(..).get_mapped_range();
        let shades: std::collections::BTreeSet<_> = data.chunks_exact(4).map(|p| p[0]).collect();
        assert_eq!(
            shades.len(),
            count as usize + 1,
            "{count} samples: {shades:?}"
        );
    }
}
