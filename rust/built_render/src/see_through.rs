// SPDX-License-Identifier: GPL-3.0-or-later
//! See-through while carrying: the objects a Move or Rotate carries draw
//! translucent, with their edges, so a grip or a snap behind them shows
//! without orbiting.
//!
//! The carried instance rows carry `SEE_FLAG`. The scene passes still cull
//! them (their edge transforms stay current) but every scene pipeline drops
//! them in its vertex shader, so they leave no colour, depth or pick ID: the
//! pick and the inference see through them too. This pass then draws only
//! those rows, one indirect command each (no second pass over the scene):
//! their faces' depth, their nearest faces at `ui::tokens::SEE_THROUGH`
//! opacity, then their edges.
use crate::{bind, buffer, depth};

/// The instance flag of a carried row (`info.z`; shading.wgsl `SEE`).
pub const SEE_FLAG: u32 = 512;

pub struct SeeThrough {
    depth: wgpu::RenderPipeline,
    faces: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    /// The carried rows, ascending.
    pub ids: Vec<u32>,
    commands: Option<wgpu::Buffer>,
    survivors: Option<wgpu::Buffer>,
    counts: [u32; 2],
}

impl SeeThrough {
    pub fn new(d: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("see-through carried objects"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(include_str!("shading.wgsl"), "\n", include_str!("edges.wgsl"), "\n", include_str!("see_through.wgsl")).into(),
            ),
        });
        let constants: std::collections::HashMap<String, f64> = [
            ("SEE".to_string(), 1.),
            ("SEE_ALPHA".to_string(), f64::from(crate::ui::tokens::SEE_THROUGH)),
        ]
        .into_iter()
        .collect();
        let face_attrs = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4];
        let face_bias = depth::bias(wgpu::DepthBiasState { constant: 1, slope_scale: 1., clamp: 0. });
        let edge_bias = depth::bias(wgpu::DepthBiasState { constant: -2, slope_scale: -1., clamp: 0. });
        let pipeline = |edge: bool, fragment: &str, write: bool, colour: bool, compare, bias| {
            let options = wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() };
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("see-through"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(if edge { "edge_vs" } else { "vs" }),
                    compilation_options: options.clone(),
                    buffers: &if edge {
                        vec![]
                    } else {
                        vec![wgpu::VertexBufferLayout {
                            array_stride: 28,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &face_attrs,
                        }]
                    },
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: options,
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: if colour { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() },
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: write,
                    depth_compare: depth::compare(compare),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        Self {
            depth: pipeline(false, "see_fs", true, false, wgpu::CompareFunction::Less, face_bias),
            faces: pipeline(false, "see_fs", false, true, wgpu::CompareFunction::LessEqual, face_bias),
            edges: pipeline(true, "see_edge_fs", false, true, wgpu::CompareFunction::LessEqual, edge_bias),
            ids: Vec::new(),
            commands: None,
            survivors: None,
            counts: [0; 2],
        }
    }

    pub fn active(&self) -> bool {
        !self.ids.is_empty() && self.commands.is_some()
    }

    /// One face command and (unless the row draws no edges) one edge command
    /// per carried row, from the scene's command rows (`template`: faces
    /// first, then edges, as `gpu_commands` lays them out).
    pub fn prepare(&mut self, d: &wgpu::Device, rows: &[u8], template: &[u8], prototypes: usize) {
        let mut faces: Vec<[u32; 5]> = Vec::new();
        let mut edges: Vec<[u32; 5]> = Vec::new();
        let mut survivors: Vec<u32> = Vec::new();
        for &id in &self.ids {
            let row = &rows[id as usize * 80..(id as usize + 1) * 80];
            let proto = u32::from_le_bytes(row[64..68].try_into().unwrap()) as usize;
            let flags = u32::from_le_bytes(row[72..76].try_into().unwrap());
            if proto >= prototypes || flags & 129 != 0 {
                continue;
            }
            let k = survivors.len() as u32;
            survivors.push(id);
            let face: [u32; 5] = bytemuck::pod_read_unaligned(&template[proto * 20..proto * 20 + 20]);
            faces.push([face[0], 1, face[2], face[3], k]);
            if flags & 8 == 0 {
                let at = (prototypes + proto) * 20;
                let edge: [u32; 5] = bytemuck::pod_read_unaligned(&template[at..at + 20]);
                edges.push([edge[0], 1, edge[2], edge[3], k]);
            }
        }
        if survivors.is_empty() {
            self.commands = None;
            self.survivors = None;
            return;
        }
        self.counts = [faces.len() as u32, edges.len() as u32];
        faces.extend(edges);
        self.commands = Some(buffer(d, bytemuck::cast_slice(&faces), wgpu::BufferUsages::INDIRECT));
        self.survivors = Some(buffer(d, bytemuck::cast_slice(&survivors), wgpu::BufferUsages::STORAGE));
    }

    /// Draw the carried rows over the finished scene (single-sampled colour
    /// and depth; the multisampled path resolves its depth first).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        d: &wgpu::Device,
        e: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        scene: SceneBuffers<'_>,
        textures: &crate::textures::Textures,
    ) {
        let (Some(commands), Some(survivors)) = (&self.commands, &self.survivors) else {
            return;
        };
        let face_group = bind(d, &self.faces.get_bind_group_layout(0), &[scene.camera, scene.inst, survivors]);
        let depth_group = bind(d, &self.depth.get_bind_group_layout(0), &[scene.camera, scene.inst, survivors]);
        let edge_group = bind(d, &self.edges.get_bind_group_layout(0), &[scene.camera, scene.inst, survivors, scene.edge_records, scene.viewpoints]);
        let style = |p: &wgpu::RenderPipeline| bind(d, &p.get_bind_group_layout(1), &[scene.style]);
        let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("see-through"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_vertex_buffer(0, scene.vertex.slice(..));
        pass.set_index_buffer(scene.index.slice(..), wgpu::IndexFormat::Uint32);
        for (pipeline, group) in [(&self.depth, &depth_group), (&self.faces, &face_group)] {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(1, &style(pipeline), &[]);
            pass.set_bind_group(2, &textures.bind(d, pipeline), &[]);
            for k in 0..self.counts[0] {
                pass.draw_indexed_indirect(commands, u64::from(k) * 20);
            }
        }
        if self.counts[1] > 0 {
            pass.set_pipeline(&self.edges);
            pass.set_bind_group(0, &edge_group, &[]);
            pass.set_bind_group(1, &style(&self.edges), &[]);
            pass.set_index_buffer(scene.quad_index.slice(..), wgpu::IndexFormat::Uint32);
            for k in 0..self.counts[1] {
                pass.draw_indexed_indirect(commands, u64::from(self.counts[0] + k) * 20);
            }
        }
    }
}

/// The resident scene's buffers this pass reads.
pub struct SceneBuffers<'a> {
    pub camera: &'a wgpu::Buffer,
    pub style: &'a wgpu::Buffer,
    pub inst: &'a wgpu::Buffer,
    pub edge_records: &'a wgpu::Buffer,
    pub viewpoints: &'a wgpu::Buffer,
    pub vertex: &'a wgpu::Buffer,
    pub index: &'a wgpu::Buffer,
    pub quad_index: &'a wgpu::Buffer,
}

/// Mark `ids` carried (and unmark the rows that no longer are) in a scene's
/// flags and row bytes; the changed rows' spans, for the GPU write.
pub fn mark(flags: &mut [u32], rows: &mut [u8], old: &[u32], new: &[u32]) -> Vec<(usize, usize)> {
    let mut touched: Vec<usize> = old.iter().chain(new.iter()).map(|&i| i as usize).filter(|&i| i < flags.len()).collect();
    touched.sort_unstable();
    touched.dedup();
    let carried: std::collections::HashSet<u32> = new.iter().copied().collect();
    let mut changed = Vec::new();
    for i in touched {
        let next = (flags[i] & !SEE_FLAG) | if carried.contains(&(i as u32)) { SEE_FLAG } else { 0 };
        if next != flags[i] {
            flags[i] = next;
            rows[i * 80 + 72..i * 80 + 76].copy_from_slice(&next.to_le_bytes());
            changed.push(i);
        }
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for i in changed {
        match spans.last_mut() {
            Some((_, end)) if *end == i => *end = i + 1,
            _ => spans.push((i, i + 1)),
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marking_sets_and_clears_the_carried_bit_and_keeps_the_rest() {
        let mut flags = vec![0u32, 2, 8, 64];
        let mut rows = vec![0u8; 4 * 80];
        for (i, f) in flags.iter().enumerate() {
            rows[i * 80 + 72..i * 80 + 76].copy_from_slice(&f.to_le_bytes());
        }
        let spans = mark(&mut flags, &mut rows, &[], &[1, 2]);
        assert_eq!(spans, vec![(1, 3)]);
        assert_eq!(flags, vec![0, 2 | SEE_FLAG, 8 | SEE_FLAG, 64]);
        assert_eq!(u32::from_le_bytes(rows[80 + 72..80 + 76].try_into().unwrap()), 2 | SEE_FLAG);
        // Esc: the carried set empties and every row is as it was.
        let spans = mark(&mut flags, &mut rows, &[1, 2], &[]);
        assert_eq!(spans, vec![(1, 3)]);
        assert_eq!(flags, vec![0, 2, 8, 64]);
    }

    #[test]
    fn every_scene_pipeline_drops_carried_rows_and_this_pass_draws_them() {
        let shading = include_str!("shading.wgsl");
        let edges = include_str!("edges.wgsl");
        let flag = format!("& {SEE_FLAG}u");
        assert!(shading.contains("override SEE: bool = false;"));
        assert!(shading.contains(&flag), "the face vertex shader drops carried rows");
        assert!(edges.contains(&flag), "the edge vertex shader drops carried rows");
        let own = include_str!("see_through.wgsl");
        assert!(own.contains("fn see_fs") && own.contains("fn see_edge_fs"));
    }
}
