//! Unlit reference imagery in model space. Each atlas has independent dimensions.
use numpy::PyReadonlyArray1;
use pyo3::{exceptions::PyValueError, prelude::*};
use std::collections::{HashMap, HashSet};
use wgpu::util::DeviceExt;

pub type Upload<'py> = (
    String,
    PyReadonlyArray1<'py, f32>,
    u32,
    u32,
    PyReadonlyArray1<'py, u8>,
    f32,
    [f32; 4],
    bool,
);
struct Layer {
    vertices: wgpu::Buffer,
    group: wgpu::BindGroup,
    count: u32,
    writes_depth: bool,
}
pub struct References {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipelines: Vec<wgpu::RenderPipeline>,
    layers: HashMap<String, Layer>,
    order: Vec<String>,
}
impl References {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        use wgpu::{BindingType as B, ShaderStages as S};
        let uniform = B::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let entries = [
            (S::VERTEX, uniform),
            (
                S::FRAGMENT,
                B::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            ),
            (S::FRAGMENT, B::Sampler(wgpu::SamplerBindingType::Filtering)),
            (S::FRAGMENT, uniform),
        ];
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reference image"),
            entries: &entries
                .iter()
                .enumerate()
                .map(|(i, (stage, ty))| wgpu::BindGroupLayoutEntry {
                    binding: i as u32,
                    visibility: *stage,
                    ty: *ty,
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reference imagery"),
            source: wgpu::ShaderSource::Wgsl(include_str!("references.wgsl").into()),
        });
        let mut pipelines = Vec::new();
        for count in [1, 4] {
            for writes_depth in [false, true] {
                pipelines.push(
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("reference imagery"),
                        layout: Some(&pl),
                        vertex: wgpu::VertexState {
                            module: &shader,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[wgpu::VertexBufferLayout {
                                array_stride: 20,
                                step_mode: wgpu::VertexStepMode::Vertex,
                                attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2],
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
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: writes_depth,
                            depth_compare: wgpu::CompareFunction::GreaterEqual,
                            stencil: Default::default(),
                            bias: wgpu::DepthBiasState {
                                constant: -4,
                                slope_scale: -4.,
                                clamp: 0.,
                            },
                        }),
                        primitive: Default::default(),
                        multisample: wgpu::MultisampleState {
                            count,
                            ..Default::default()
                        },
                        multiview: None,
                        cache: None,
                    }),
                );
            }
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            sampler,
            pipelines,
            layers: HashMap::new(),
            order: Vec::new(),
        }
    }
    pub fn active(&self) -> bool {
        !self.order.is_empty()
    }

    pub fn replace(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &wgpu::Buffer,
        order: Vec<String>,
        updates: Vec<Upload<'_>>,
    ) -> PyResult<()> {
        let wanted: HashSet<_> = order.iter().collect();
        let mut staged = HashMap::new();
        let limit = device.limits();
        for (key, verts, width, height, image, opacity, fade, writes_depth) in updates {
            let pts = verts.as_slice()?;
            let pixels = image.as_slice()?;
            if !wanted.contains(&key)
                || width == 0
                || height == 0
                || width > limit.max_texture_dimension_2d
                || height > limit.max_texture_dimension_2d
                || pixels.len() != width as usize * height as usize * 4
                || pts.is_empty()
                || pts.len() % 15 != 0
                || pts.len() as u64 * 4 > limit.max_buffer_size
                || pts.iter().any(|x| !x.is_finite())
                || !opacity.is_finite()
                || fade.iter().any(|x| !x.is_finite())
            {
                return Err(PyValueError::new_err(
                    "invalid reference triangles or texture",
                ));
            }
            let tex = device.create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("reference atlas"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                pixels,
            );
            let appearance = crate::buffer(
                device,
                bytemuck::cast_slice(&[fade, [opacity, 0., 0., 0.]]),
                wgpu::BufferUsages::UNIFORM,
            );
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: camera.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &tex.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: appearance.as_entire_binding(),
                    },
                ],
            });
            staged.insert(
                key,
                Layer {
                    vertices: crate::buffer(
                        device,
                        bytemuck::cast_slice(pts),
                        wgpu::BufferUsages::VERTEX,
                    ),
                    count: (pts.len() / 5) as u32,
                    group,
                    writes_depth,
                },
            );
        }
        if wanted.len() != order.len()
            || order
                .iter()
                .any(|k| !staged.contains_key(k) && !self.layers.contains_key(k))
        {
            return Err(PyValueError::new_err(
                "unknown or duplicate reference layer",
            ));
        }
        self.layers.retain(|k, _| wanted.contains(k));
        self.layers.extend(staged);
        self.order = order;
        Ok(())
    }
    pub fn draw(
        &self,
        e: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
    ) {
        if self.order.is_empty() {
            return;
        }
        let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("reference imagery"),
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
        for key in &self.order {
            let layer = &self.layers[key];
            pass.set_pipeline(
                &self.pipelines
                    [2 * usize::from(resolve.is_some()) + usize::from(layer.writes_depth)],
            );
            pass.set_bind_group(0, &layer.group, &[]);
            pass.set_vertex_buffer(0, layer.vertices.slice(..));
            pass.draw(0..layer.count, 0..1);
        }
    }
}
