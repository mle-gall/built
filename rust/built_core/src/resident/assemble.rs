// SPDX-License-Identifier: GPL-3.0-or-later
//! From the prototypes and placements a pack gathered to the resident
//! buffers: the scene bake, the indirect command tables, the instance rows
//! in the view frame, the world box and the frame origin.
use super::packed_scene::{dtype, COMMAND_BYTES, INSTANCE_BYTES};
use numpy::{
    PyArray1, PyArray2, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2, PyUntypedArrayMethods,
};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

type Obj<'py> = Bound<'py, PyAny>;

/// Beyond this distance from the frame origin f32 coordinates lose the
/// precision the model needs (`built_core.coordinate_frame.REBASE_DISTANCE`).
pub(crate) const REBASE_DISTANCE: f64 = 512.;

/// `built_core.coordinate_frame.choose_origin`: keep the frame while the centre
/// stays within reach of it, else move it to the centre's whole metres.
pub(crate) fn choose_origin(center: [f64; 3], previous: [f64; 3]) -> [f64; 3] {
    let reach = (0..3).map(|k| (center[k] - previous[k]).abs()).fold(0., f64::max);
    if !center.iter().all(|v| v.is_finite()) || reach <= REBASE_DISTANCE {
        previous
    } else {
        center.map(f64::round_ties_even)
    }
}

/// `row · linear`, the terms added in index order (NumPy's `ni,nij->nj`).
pub(crate) fn through(row: [f64; 3], linear: &[[f64; 3]; 3]) -> [f64; 3] {
    std::array::from_fn(|j| (0..3).fold(0., |sum, i| sum + row[i] * linear[i][j]))
}

/// The concatenated source arrays of the prototypes, as the bake takes them.
#[derive(Default)]
struct Sources {
    pos: Vec<f32>,
    corners: Vec<i64>,
    sizes: Vec<i64>,
    counts: Vec<i64>,
    edges: Vec<i64>,
    flags: Vec<u8>,
    palette: Vec<u8>,
    hidden: Vec<bool>,
    attributes: Vec<i64>,
    maps: Vec<f32>,
    paints: Vec<u32>,
    /// Running ends of pos, corners, sizes, counts, edges per prototype.
    offsets: Vec<i64>,
    /// `(code, endpoints)` of the prototypes with soft or hidden edges.
    exclusions: Vec<(usize, Vec<f32>)>,
    /// Per corner, an exact surface normal or NaN; used if any is given.
    corner_normals: Vec<f32>,
    exact: bool,
}

impl Sources {
    fn gather(meshes: &Bound<'_, PyList>, with_maps: bool) -> PyResult<Self> {
        let mut all = Self { offsets: vec![0; 5], ..Self::default() };
        for (code, mesh) in meshes.iter().enumerate() {
            let field = |at: usize| mesh.get_item(at);
            let pos: PyReadonlyArray2<'_, f32> = field(0)?.extract()?;
            let corners: PyReadonlyArray1<'_, i64> = field(1)?.extract()?;
            let sizes: PyReadonlyArray1<'_, i64> = field(2)?.extract()?;
            let counts: PyReadonlyArray1<'_, i64> = field(3)?.extract()?;
            let edges: PyReadonlyArray2<'_, i64> = field(4)?.extract()?;
            let flags: PyReadonlyArray1<'_, u8> = field(5)?.extract()?;
            let palette: PyReadonlyArray2<'_, u8> = field(6)?.extract()?;
            let hidden: PyReadonlyArray1<'_, bool> = field(7)?.extract()?;
            let attributes: PyReadonlyArray1<'_, i64> = field(8)?.extract()?;
            let paints: PyReadonlyArray2<'_, u32> = field(10)?.extract()?;
            // Exact corner normals of curved supports (optional 12th field).
            let corners_here = corners.as_array().len();
            match (mesh.len()? > 11).then(|| field(11)).transpose()?.filter(|n| !n.is_none()) {
                Some(n) => {
                    let n: PyReadonlyArray2<'_, f32> = n.extract()?;
                    if n.shape() != [corners_here, 3] {
                        return Err(PyValueError::new_err("invalid prototype corner normals"));
                    }
                    all.exact = true;
                    all.corner_normals.extend(n.as_array().iter().copied());
                }
                None => all.corner_normals.extend(std::iter::repeat(f32::NAN).take(3 * corners_here)),
            }
            if pos.shape()[1] != 3 || edges.shape()[1] != 2 || palette.shape()[1] != 4 || paints.shape()[1] != 3 {
                return Err(PyValueError::new_err("invalid prototype source arrays"));
            }
            let (pos, edges, flags) = (pos.as_array(), edges.as_array(), flags.as_array());
            if flags.iter().any(|f| f & 3 != 0) {
                // The endpoints of the edges a profile must not draw.
                let mut points = Vec::new();
                for (edge, _) in edges.rows().into_iter().zip(flags.iter()).filter(|(_, f)| **f & 3 != 0) {
                    for end in edge {
                        let row = usize::try_from(*end).ok().filter(|row| *row < pos.nrows());
                        let row = row.ok_or_else(|| PyValueError::new_err("edge index out of range"))?;
                        points.extend(pos.row(row).iter().copied());
                    }
                }
                all.exclusions.push((code, points));
            }
            let palette_base = (all.palette.len() / 4) as i64;
            all.pos.extend(pos.iter().copied());
            all.corners.extend(corners.as_array().iter().copied());
            all.sizes.extend(sizes.as_array().iter().copied());
            all.counts.extend(counts.as_array().iter().copied());
            all.edges.extend(edges.iter().copied());
            all.flags.extend(flags.iter().copied());
            all.palette.extend(palette.as_array().iter().copied());
            all.hidden.extend(hidden.as_array().iter().copied());
            all.attributes.extend(attributes.as_array().iter().map(|a| a + palette_base));
            all.paints.extend(paints.as_array().iter().copied());
            if with_maps {
                let maps: PyReadonlyArray2<'_, f32> = field(9)?.extract()?;
                if maps.shape()[1] != 16 {
                    return Err(PyValueError::new_err("invalid prototype material maps"));
                }
                all.maps.extend(maps.as_array().iter().copied());
            }
            all.offsets.extend([
                (all.pos.len() / 3) as i64,
                all.corners.len() as i64,
                all.sizes.len() as i64,
                all.counts.len() as i64,
                (all.edges.len() / 2) as i64,
            ]);
        }
        Ok(all)
    }
}

fn rows_of<'py, T: numpy::Element>(py: Python<'py>, data: Vec<T>, width: usize) -> PyResult<Bound<'py, PyArray2<T>>> {
    let rows = data.len() / width;
    PyArray1::from_vec(py, data).reshape([rows, width])
}

/// A structured view of `bytes` (`VERTEX`, `INSTANCE` or `COMMAND`).
fn records<'py>(py: Python<'py>, bytes: Vec<u8>, layout: &str) -> PyResult<Obj<'py>> {
    PyArray1::from_vec(py, bytes).call_method1("view", (dtype(py, layout)?,))
}

fn command(table: &mut [u8], code: usize, count: u32, first: u32, base: u32, start: u32) {
    let row = &mut table[code * COMMAND_BYTES..(code + 1) * COMMAND_BYTES];
    row[0..4].copy_from_slice(&count.to_le_bytes());
    row[8..12].copy_from_slice(&first.to_le_bytes());
    row[12..16].copy_from_slice(&(base as i32).to_le_bytes());
    row[16..20].copy_from_slice(&start.to_le_bytes());
}

/// The resident buffers of a pack.
///
/// `meshes` are the prototypes' source arrays (`source_arrays`), in code
/// order; `codes`, `worlds`, `flags` and `paints` describe the placement
/// rows; `coordinates` is `(origin, box, wide)` per prototype. Returns the
/// fields of the `PackedScene` this side computes.
#[allow(clippy::too_many_arguments)]
pub(super) fn assemble<'py>(
    py: Python<'py>,
    meshes: &Bound<'py, PyList>,
    codes: Vec<usize>,
    mut worlds: Vec<[[f64; 4]; 4]>,
    flags: Vec<u32>,
    paints: Vec<u32>,
    tints: Vec<Option<Vec<f64>>>,
    coordinates: &Bound<'py, PyList>,
    billboard_codes: Vec<usize>,
    with_maps: bool,
    previous_origin: &Obj<'py>,
    wide: bool,
    hypot: &str,
) -> PyResult<Bound<'py, PyDict>> {
    let np = py.import("numpy")?;
    let prototype_count = meshes.len();
    let rows = codes.len();
    if flags.len() != rows || paints.len() != rows || tints.len() != rows || worlds.len() != rows
        || coordinates.len() != prototype_count || codes.iter().any(|code| *code >= prototype_count)
    {
        return Err(PyValueError::new_err("placement rows do not match their prototypes"));
    }
    let sources = Sources::gather(meshes, with_maps)?;
    let faces = sources.counts.len();
    let exclusions = PyList::empty(py);
    let floats = py.import("array")?.getattr("array")?;
    for (code, points) in &sources.exclusions {
        // Sequence semantics for incremental edits without boxing every float.
        let endpoints = floats.call1(("f",))?;
        endpoints.call_method1("frombytes", (pyo3::types::PyBytes::new(py, bytemuck_bytes(points)),))?;
        exclusions.append((*code, endpoints))?;
    }
    let maps = if with_maps {
        rows_of(py, sources.maps, 16)?.into_any()
    } else {
        // No texture and no sprite: one shared zero mapping, not 16 floats a face.
        np.call_method1("broadcast_to", (np.call_method1("zeros", (16, np.getattr("float32")?))?, (faces, 16)))?
    };
    let (raw, indices, bounds, table, materials) = crate::scene_bake::bake_scene(
        py,
        rows_of(py, sources.pos, 3)?.readonly(),
        PyArray1::from_vec(py, sources.corners).readonly(),
        PyArray1::from_vec(py, sources.sizes).readonly(),
        PyArray1::from_vec(py, sources.counts).readonly(),
        rows_of(py, sources.palette, 4)?.readonly(),
        PyArray1::from_vec(py, sources.hidden).readonly(),
        rows_of(py, sources.edges, 2)?.readonly(),
        PyArray1::from_vec(py, sources.flags).readonly(),
        rows_of(py, sources.offsets, 5)?.readonly(),
        PyArray1::from_vec(py, sources.attributes).readonly(),
        hypot,
        maps.extract()?,
        rows_of(py, sources.paints, 3)?.readonly(),
        true,
        if sources.exact { Some(rows_of(py, sources.corner_normals, 3)?.readonly()) } else { None },
    )?;

    // The prototypes' frame anchors and boxes.
    let mut origins = Vec::with_capacity(prototype_count);
    let mut boxes = Vec::with_capacity(prototype_count);
    for entry in coordinates.iter() {
        let origin: [f64; 3] = entry.get_item(0)?.extract()?;
        let bounds: PyReadonlyArray2<'_, f64> = entry.get_item(1)?.extract()?;
        let bounds = bounds.as_array();
        if bounds.shape() != [2, 3] {
            return Err(PyValueError::new_err("a prototype box is two corners"));
        }
        origins.push(origin);
        boxes.push([[bounds[[0, 0]], bounds[[0, 1]], bounds[[0, 2]]], [bounds[[1, 0]], bounds[[1, 1]], bounds[[1, 2]]]]);
    }

    // The rows: world matrices in the model frame, then in the view frame.
    let mut origin: [f64; 3] = previous_origin.extract()?;
    let mut moved = false;
    let mut world_bounds = [[0.; 3]; 2];
    let mut seen = false;
    let mut far = false;
    for (row, world) in worlds.iter_mut().enumerate() {
        let code = codes[row];
        let linear: [[f64; 3]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| world[r][c]));
        let anchor = through(origins[code], &linear);
        for axis in 0..3 {
            world[3][axis] += anchor[axis];
        }
        let (low, high) = (boxes[code][0], boxes[code][1]);
        let centre = through(std::array::from_fn(|k| (low[k] + high[k]) / 2.), &linear);
        let spread: [[f64; 3]; 3] = linear.map(|r| r.map(f64::abs));
        let radius = through(std::array::from_fn(|k| (high[k] - low[k]) * 0.5), &spread);
        far |= (0..3).any(|axis| world[3][axis].abs() > REBASE_DISTANCE);
        if flags[row] & 1 == 0 {
            for axis in 0..3 {
                let at = centre[axis] + world[3][axis];
                let (near, beyond) = (at - radius[axis], at + radius[axis]);
                if !seen {
                    world_bounds[0][axis] = near;
                    world_bounds[1][axis] = beyond;
                } else {
                    world_bounds[0][axis] = world_bounds[0][axis].min(near);
                    world_bounds[1][axis] = world_bounds[1][axis].max(beyond);
                }
            }
            seen = true;
        }
    }
    if seen {
        let centre = std::array::from_fn(|k| (world_bounds[0][k] + world_bounds[1][k]) / 2.);
        let chosen = choose_origin(centre, origin);
        moved = chosen != origin;
        origin = chosen;
        for corner in &mut world_bounds {
            for axis in 0..3 {
                corner[axis] -= origin[axis];
            }
        }
    }
    let rebased = rows > 0
        && (wide || origin.iter().any(|v| *v != 0.) || far || origins.iter().any(|o| o.iter().any(|v| *v != 0.)));
    let translations = rebased.then(|| worlds.iter().flat_map(|w| [w[3][0], w[3][1], w[3][2]]).collect::<Vec<f64>>());
    let mut instances = vec![0u8; rows * INSTANCE_BYTES];
    let mut members = vec![0u32; prototype_count];
    for (row, world) in worlds.iter().enumerate() {
        let out = &mut instances[row * INSTANCE_BYTES..(row + 1) * INSTANCE_BYTES];
        for r in 0..4 {
            for c in 0..4 {
                let value = if r == 3 && c < 3 { world[r][c] - origin[c] } else { world[r][c] };
                out[(r * 4 + c) * 4..(r * 4 + c) * 4 + 4].copy_from_slice(&(value as f32).to_le_bytes());
            }
        }
        let tint = match &tints[row] {
            Some(rgb) if rgb.len() >= 3 => {
                let byte = |v: f64| (v * 255.).clamp(0., 255.) as u8;
                u32::from_le_bytes([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), 255])
            }
            _ => 0,
        };
        for (at, word) in [codes[row] as u32, paints[row], flags[row], tint].into_iter().enumerate() {
            out[64 + at * 4..68 + at * 4].copy_from_slice(&word.to_le_bytes());
        }
        members[codes[row]] += 1;
    }

    // The indirect draws: faces, edges, and the hidden ones after them.
    let table = table.readonly();
    let table = table.as_slice()?;
    let mut tables: [Vec<u8>; 4] = std::array::from_fn(|_| vec![0u8; prototype_count * COMMAND_BYTES]);
    let mut start = 0u32;
    for code in 0..prototype_count {
        let entry = &table[code * 5..code * 5 + 5];
        let (base, faces, edges, hidden_faces, hidden_edges) = (entry[0], entry[1], entry[2], entry[3], entry[4]);
        let visible_edges = if billboard_codes.contains(&code) { 0 } else { edges };
        command(&mut tables[0], code, faces, base, base, start);
        command(&mut tables[1], code, visible_edges, base + faces, base, start);
        command(&mut tables[2], code, hidden_faces, base + faces + edges, base, start);
        command(&mut tables[3], code, hidden_edges, base + faces + edges + hidden_faces, base, start);
        start += members[code];
    }
    let [commands, edge_commands, hidden_commands, hidden_edge_commands] = tables;

    let out = PyDict::new(py);
    out.set_item("vertices", raw.call_method1("view", (dtype(py, "VERTEX")?,))?)?;
    out.set_item("indices", indices)?;
    out.set_item("bounds", bounds.call_method1("reshape", (-1, 2, 4))?)?;
    out.set_item("commands", records(py, commands, "COMMAND")?)?;
    out.set_item("edge_commands", records(py, edge_commands, "COMMAND")?)?;
    out.set_item("hidden_commands", records(py, hidden_commands, "COMMAND")?)?;
    out.set_item("hidden_edge_commands", records(py, hidden_edge_commands, "COMMAND")?)?;
    out.set_item("instances", records(py, instances, "INSTANCE")?)?;
    out.set_item("materials", materials)?;
    out.set_item("profile_exclusions", exclusions.to_tuple())?;
    out.set_item("world_bounds", rows_of(py, world_bounds.concat(), 3)?)?;
    if moved {
        out.set_item("origin", PyTuple::new(py, origin)?)?;
    } else {
        out.set_item("origin", previous_origin)?;
    }
    out.set_item("prototype_origins", PyTuple::new(py, origins.iter().map(|o| PyTuple::new(py, o)).collect::<PyResult<Vec<_>>>()?)?)?;
    match translations {
        Some(translations) => out.set_item("model_translations", rows_of(py, translations, 3)?)?,
        None => out.set_item("model_translations", py.None())?,
    }
    Ok(out)
}

fn bytemuck_bytes(values: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and any byte pattern is a valid u8.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_stays_until_the_centre_is_out_of_reach() {
        assert_eq!(choose_origin([100., -511.9, 0.], [0.; 3]), [0.; 3]);
        assert_eq!(choose_origin([1000.4, 2.5, -3.5], [0.; 3]), [1000., 2., -4.]);
        assert_eq!(choose_origin([f64::NAN, 9e9, 0.], [7., 8., 9.]), [7., 8., 9.]);
    }

    #[test]
    fn a_row_goes_through_the_linear_part_in_index_order() {
        let linear = [[1., 2., 3.], [4., 5., 6.], [7., 8., 9.]];
        assert_eq!(through([1., 10., 100.], &linear), [741., 852., 963.]);
    }
}
