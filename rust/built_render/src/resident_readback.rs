// SPDX-License-Identifier: GPL-3.0-or-later
//! Rare hidden-object requests can recover vertices from their GPU owner.
use super::{Command, Scene};
use std::collections::BTreeSet;
use std::ops::Range;

#[derive(Default)]
pub struct Readback {
    // Source vertex range and byte offset in the compact staging buffer.
    ranges: Vec<(Range<usize>, usize)>,
    bytes: Vec<u8>,
}

impl Readback {
    pub fn vertex(&self, index: usize) -> Option<&[u8]> {
        let i = self.ranges.partition_point(|(range, _)| range.end <= index);
        let (range, offset) = self.ranges.get(i)?;
        let start = offset + index.checked_sub(range.start)? * 28;
        self.bytes.get(start..start + 28)
    }
}

pub(super) fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &Scene,
    objects: &[usize],
) -> Result<Readback, String> {
    let mut prototypes = BTreeSet::new();
    for &id in objects {
        if id >= scene.n as usize {
            return Err("invalid hidden object row".into());
        }
        let row = &scene.rows[id * 80..(id + 1) * 80];
        let flags = u32::from_le_bytes(row[72..76].try_into().unwrap());
        if flags & 128 == 0 {
            prototypes.insert(u32::from_le_bytes(row[64..68].try_into().unwrap()) as usize);
        }
    }
    let mut indices = Vec::new();
    for prototype in prototypes {
        let cmd: Command = bytemuck::pod_read_unaligned(
            &scene.template_data[prototype * 20..(prototype + 1) * 20],
        );
        for i in scene.index_data
            [cmd.first as usize * 4..(cmd.first as usize + cmd.count as usize) * 4]
            .chunks_exact(4)
        {
            indices.push(
                (i64::from(u32::from_le_bytes(i.try_into().unwrap())) + i64::from(cmd.base))
                    as usize,
            );
        }
    }
    indices.sort_unstable();
    indices.dedup();
    let bytes = indices.len() * 28;
    let mut result = Readback::default();
    if bytes == 0 {
        return Ok(result);
    }
    for (offset, index) in indices.into_iter().enumerate() {
        if index >= scene.vertex_bytes / 28 {
            return Err("invalid resident vertex".into());
        }
        if let Some((range, _)) = result.ranges.last_mut() {
            if range.end == index {
                range.end += 1;
                continue;
            }
        }
        result.ranges.push((index..index + 1, offset * 28));
    }
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hidden object readback"),
        size: bytes as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    for (range, offset) in &result.ranges {
        encoder.copy_buffer_to_buffer(
            &scene.vertex,
            (range.start * 28) as u64,
            &staging,
            *offset as u64,
            (range.len() * 28) as u64,
        );
    }
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = send.send(r);
    });
    device.poll(wgpu::Maintain::Wait);
    recv.recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    result.bytes = staging.slice(..).get_mapped_range().to_vec();
    staging.unmap();
    Ok(result)
}
