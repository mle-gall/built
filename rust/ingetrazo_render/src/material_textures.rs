//! Resident material textures and their complete, budgeted mip chains.
use image::{imageops, RgbaImage};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyTuple},
};
use std::{collections::HashMap, path::PathBuf};

struct Layer {
    pixels: RgbaImage,
    premultiplied: Option<RgbaImage>,
}

impl Layer {
    fn new(mut pixels: RgbaImage) -> Self {
        imageops::flip_vertical_in_place(&mut pixels);
        let premultiplied = pixels.pixels().any(|p| p[3] != 255).then(|| {
            let mut copy = pixels.clone();
            crate::image_assets::premultiply(&mut copy);
            copy
        });
        Self {
            pixels,
            premultiplied,
        }
    }

    fn append(&self, dst: &mut Vec<u8>, width: u32, height: u32) {
        if self.pixels.dimensions() == (width, height) {
            dst.extend_from_slice(self.pixels.as_raw());
            return;
        }
        let src = self.premultiplied.as_ref().unwrap_or(&self.pixels);
        let mut mip = imageops::resize(src, width, height, imageops::FilterType::Triangle);
        if self.premultiplied.is_some() {
            crate::image_assets::unpremultiply(&mut mip);
        }
        dst.extend_from_slice(mip.as_raw());
    }
}

fn levels(w: u32, h: u32) -> impl Iterator<Item = (u32, u32)> {
    std::iter::successors(Some((w, h)), |&(w, h)| {
        (w != 1 || h != 1).then(|| ((w / 2).max(1), (h / 2).max(1)))
    })
}

enum Entry {
    Base(PathBuf),
    Pbr([Option<PathBuf>; 6], bool, [f32; 4]),
}

#[pyfunction]
pub fn material_texture_array<'py>(
    py: Python<'py>,
    entries: &Bound<'py, PyAny>,
    device_limit: u32,
    memory_budget: u64,
) -> PyResult<Bound<'py, PyTuple>> {
    let entries: Vec<Entry> = entries
        .try_iter()?
        .map(|value| {
            let value = value?;
            Ok(if let Some(maps) = value.getattr_opt("images")? {
                Entry::Pbr(
                    maps.extract()?,
                    value.getattr("flip_y")?.extract()?,
                    value.getattr("factors")?.extract()?,
                )
            } else {
                Entry::Base(value.extract()?)
            })
        })
        .collect::<PyResult<_>>()?;
    let pbr = entries.iter().any(|e| matches!(e, Entry::Pbr(..)));
    let mut paths = Vec::new();
    let mut descriptors = vec![[0u32; 12]; if pbr { entries.len() + 1 } else { 0 }];
    let mut ids = HashMap::new();
    for (i, entry) in entries.into_iter().enumerate() {
        if !pbr {
            if let Entry::Base(path) = entry {
                paths.push(path);
            }
            continue;
        }
        let row = &mut descriptors[i + 1];
        let mut register = |channel: usize, path: PathBuf| {
            if !path.as_os_str().is_empty() {
                row[channel] = *ids.entry(path.clone()).or_insert_with(|| {
                    paths.push(path);
                    paths.len() as u32
                });
            }
        };
        match entry {
            Entry::Base(path) => register(0, path),
            Entry::Pbr(maps, flip, factors) => {
                for (channel, path) in maps.into_iter().enumerate() {
                    if let Some(path) = path {
                        register(channel, path);
                    }
                }
                row[6] = 1;
                row[7] = u32::from(flip);
                row[8..].copy_from_slice(&factors.map(f32::to_bits));
            }
        }
    }
    let count = paths.len() + 1;
    let stride = count as u128 * 4;
    if device_limit == 0 || stride > u128::from(memory_budget) {
        return Err(PyValueError::new_err(
            "texture budget cannot hold one texel per layer",
        ));
    }
    let images: Vec<_> = paths
        .iter()
        .map(|p| crate::image_assets::decode(py, p).map(|img| img.map(Layer::new)))
        .collect::<PyResult<_>>()?;
    for row in &mut descriptors {
        for id in &mut row[..6] {
            if *id != 0 && images[*id as usize - 1].is_none() {
                *id = 0;
            }
        }
    }
    let (mut w, mut h) = images.iter().flatten().fold((1, 1), |(w, h), layer| {
        (w.max(layer.pixels.width()), h.max(layer.pixels.height()))
    });
    w = w.min(device_limit);
    h = h.min(device_limit);
    while levels(w, h)
        .map(|(x, y)| u128::from(x) * u128::from(y) * stride)
        .sum::<u128>()
        > u128::from(memory_budget)
    {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let mut mips = vec![];
    for (mw, mh) in levels(w, h) {
        let data = py.detach(|| {
            let bytes = mw as usize * mh as usize * 4;
            let mut data = Vec::with_capacity(count * bytes);
            data.resize(bytes, 255);
            for layer in &images {
                if let Some(layer) = layer {
                    layer.append(&mut data, mw, mh);
                } else {
                    data.resize(data.len() + bytes, 255);
                }
            }
            data
        });
        mips.push(PyBytes::new(py, &data));
    }
    if !pbr {
        return (w, h, count, mips).into_pyobject(py);
    }
    let data: Vec<_> = descriptors
        .into_iter()
        .flatten()
        .flat_map(u32::to_le_bytes)
        .collect();
    (w, h, count, mips, PyBytes::new(py, &data)).into_pyobject(py)
}
