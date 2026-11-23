//! Texture runs for a moving tool preview. Keep an image resident across drag updates.
use numpy::PyReadonlyArray1;
use pyo3::{exceptions::PyValueError, prelude::*};
use std::collections::{HashMap, HashSet};
use wgpu::util::DeviceExt;

pub type Run<'py> = (String, f32, f32, PyReadonlyArray1<'py, f32>);

/// Split tool faces by material, reusing the document's UV mapper for positioned textures.
#[pyfunction]
pub fn preview_materials<'py>(
    py: Python<'py>,
    faces: &Bound<'py, PyAny>,
    controller: &Bound<'py, PyAny>,
    tool_opacity: f32,
) -> PyResult<(
    Bound<'py, pyo3::types::PyList>,
    Bound<'py, pyo3::types::PyList>,
)> {
    use pyo3::types::{PyDict, PyList};
    let plain = PyList::empty(py);
    let textured = PyList::empty(py);
    let mut by_opacity = Vec::<(f32, Bound<'py, PyDict>)>::new();
    for item in faces.try_iter()? {
        let face = item?;
        let attrs = face
            .getattr("attrs")
            .ok()
            .and_then(|a| a.cast_into::<PyDict>().ok());
        let Some(attrs) = attrs else {
            plain.append(face)?;
            continue;
        };
        let texture = attrs
            .get_item("texture")?
            .and_then(|t| t.cast_into::<PyDict>().ok());
        let Some(texture) = texture else {
            plain.append(face)?;
            continue;
        };
        if !texture
            .get_item("path")?
            .is_some_and(|path| path.is_truthy().unwrap_or(false))
        {
            plain.append(face)?;
            continue;
        }
        let opacity = tool_opacity
            * attrs
                .get_item("opacity")?
                .map(|o| o.extract::<f32>())
                .transpose()?
                .unwrap_or(1.);
        let runs = if let Some((_, runs)) = by_opacity.iter().find(|(a, _)| *a == opacity) {
            runs.clone()
        } else {
            let runs = PyDict::new(py);
            by_opacity.push((opacity, runs.clone()));
            runs
        };
        controller.call_method1("_append_textured_face", (runs, face, texture))?;
    }
    for (opacity, runs) in by_opacity {
        for (key, points) in runs.iter() {
            let (path, shade): (String, f32) = key.extract()?;
            textured.append((path, shade, opacity, points))?;
        }
    }
    Ok((plain, textured))
}

pub struct TexturedPreview {
    pipelines: [wgpu::RenderPipeline; 2],
    sampler: wgpu::Sampler,
    images: HashMap<String, wgpu::BindGroup>,
    runs: Vec<(String, wgpu::Buffer, u32)>,
}

impl TexturedPreview {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        scene: &wgpu::BindGroupLayout,
    ) -> Self {
        let images = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tool preview image"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[scene, &images],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("textured tool preview"),
            source: wgpu::ShaderSource::Wgsl(include_str!("preview.wgsl").into()),
        });
        let pipelines = [1, 4].map(|count| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("textured preview"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("texture_vs"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 28,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x4],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("texture_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: crate::depth::compare(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count,
                    ..Default::default()
                },
                multiview: None,
                cache: None,
            })
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipelines,
            sampler,
            images: HashMap::new(),
            runs: vec![],
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        runs: Vec<Run<'_>>,
        load: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let mut next = Vec::new();
        let mut new_images = HashMap::new();
        for (path, shade, opacity, points) in runs {
            let points = points.as_slice()?;
            if points.len() % 15 != 0
                || points.iter().any(|p| !p.is_finite())
                || !shade.is_finite()
                || !opacity.is_finite()
            {
                return Err(PyValueError::new_err("invalid textured preview vertices"));
            }
            if points.is_empty() {
                continue;
            }
            if !self.images.contains_key(&path) && !new_images.contains_key(&path) {
                let limit = device.limits().max_texture_dimension_2d;
                let result = load.call1((&path, limit))?;
                let (w, h, bytes): (u32, u32, PyReadonlyArray1<'_, u8>) = result.extract()?;
                let bytes = bytes.as_slice()?;
                if w == 0
                    || h == 0
                    || w > limit
                    || h > limit
                    || bytes.len() as u64 != u64::from(w) * u64::from(h) * 4
                {
                    return Err(PyValueError::new_err("invalid preview image dimensions"));
                }
                let texture = device.create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some("paste material"),
                        size: wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    bytes,
                );
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.pipelines[0].get_bind_group_layout(1),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &texture.create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                new_images.insert(path.clone(), group);
            }
            let vertices: Vec<_> = points
                .chunks_exact(5)
                .map(|p| [p[0], p[1], p[2], p[3], p[4], shade, opacity])
                .collect();
            next.push((
                path,
                crate::buffer(
                    device,
                    bytemuck::cast_slice(&vertices),
                    wgpu::BufferUsages::VERTEX,
                ),
                vertices.len() as u32,
            ));
        }
        let paths: HashSet<_> = next.iter().map(|(path, _, _)| path).collect();
        self.images.retain(|p, _| paths.contains(p));
        self.images.extend(new_images);
        self.runs = next;
        Ok(())
    }

    pub fn active(&self) -> bool {
        !self.runs.is_empty()
    }
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, msaa: bool) {
        pass.set_pipeline(&self.pipelines[usize::from(msaa)]);
        for (path, vertices, count) in &self.runs {
            pass.set_bind_group(1, &self.images[path], &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..*count, 0..1);
        }
    }
}
