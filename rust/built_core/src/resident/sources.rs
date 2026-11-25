// SPDX-License-Identifier: GPL-3.0-or-later
//! A prototype's source arrays for the scene bake, read off its mesh: the
//! topology as the document keeps it (the resident renderer needs no welded
//! plan), the paint each face is drawn with and, for textured faces, the
//! mapping of each side.
//!
//! The material rules themselves (`built_core.materials`: the container's paint
//! under a face's own, the back of a face) are the document's and are asked
//! per attribute row, not per face.
use super::assemble::{choose_origin, REBASE_DISTANCE};
use numpy::{
    ndarray::{Array, Array1, Array2, ArrayView, Dimension},
    IntoPyArray, PyArray1, PyArray2, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2,
};
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyTuple},
    IntoPyObjectExt,
};
use std::collections::HashMap;

type Obj<'py> = Bound<'py, PyAny>;

/// `max(1, min(255, round(opacity * 255)))`: the opacity byte, never zero
/// (zero marks an unpainted face).
fn alpha(attrs: &Obj<'_>) -> PyResult<u8> {
    let opacity: f64 = match attrs.call_method1("get", ("opacity",))? {
        value if value.is_none() => 1.0,
        value => value.call_method0("__float__")?.extract()?,
    };
    Ok((opacity * 255.).round_ties_even().clamp(1., 255.) as u8)
}

/// `np.clip(rgb * 255, 0, 255)` as bytes (the cast truncates).
fn rgb_bytes(color: &Obj<'_>) -> PyResult<[u8; 3]> {
    let mut out = [0u8; 3];
    for (axis, byte) in out.iter_mut().enumerate() {
        let value: f64 = color.get_item(axis)?.extract()?;
        *byte = (value * 255.).clamp(0., 255.) as u8;
    }
    Ok(out)
}

fn present(attrs: &Obj<'_>, key: &str) -> PyResult<bool> {
    attrs.contains(key)
}
fn value<'py>(attrs: &Obj<'py>, key: &str) -> PyResult<Option<Obj<'py>>> {
    Ok(Some(attrs.call_method1("get", (key,))?).filter(|v| !v.is_none()))
}

/// A copy of `view`. Nothing is read off an empty one: the strides NumPy
/// gives an array without elements are not ones ndarray's debug checks
/// accept (a panic in debug builds only).
fn owned<A: Clone, D: Dimension>(view: ArrayView<'_, A, D>) -> Array<A, D> {
    if view.is_empty() {
        return Array::from_shape_vec(view.raw_dim(), Vec::new()).expect("an empty shape holds no elements");
    }
    view.to_owned()
}

/// The positions the normals are taken from, in the precision the document
/// keeps them (f32 for a file's own arrays, f64 for an edited mesh or one
/// moved to another frame): the face mapping is resolved per distinct
/// normal, so the last bit decides which faces share one.
enum Points {
    Single(Array2<f32>),
    Double(Array2<f64>),
}

impl Points {
    fn read(pos: &Obj<'_>) -> PyResult<Self> {
        if let Ok(single) = pos.extract::<PyReadonlyArray2<'_, f32>>() {
            return Ok(Self::Single(owned(single.as_array())));
        }
        let np = pos.py().import("numpy")?;
        let double: PyReadonlyArray2<'_, f64> = np.call_method1("asarray", (pos, np.getattr("float64")?))?.extract()?;
        Ok(Self::Double(owned(double.as_array())))
    }
    fn rows(&self) -> usize {
        match self {
            Self::Single(a) => a.nrows(),
            Self::Double(a) => a.nrows(),
        }
    }
    /// The points as a NumPy array in their own precision.
    fn array<'py>(&self, py: Python<'py>) -> Obj<'py> {
        match self {
            Self::Single(a) => a.clone().into_pyarray(py).into_any(),
            Self::Double(a) => a.clone().into_pyarray(py).into_any(),
        }
    }
    fn single(&self) -> Array2<f32> {
        match self {
            Self::Single(a) => a.clone(),
            Self::Double(a) => a.mapv(|v| v as f32),
        }
    }
    /// `[min, max]` over the points, zeros when there are none.
    fn span(&self) -> [[f64; 3]; 2] {
        let mut span = [[0.; 3]; 2];
        let mut each = |row: usize, point: [f64; 3]| {
            for axis in 0..3 {
                if row == 0 {
                    span[0][axis] = point[axis];
                    span[1][axis] = point[axis];
                } else {
                    span[0][axis] = span[0][axis].min(point[axis]);
                    span[1][axis] = span[1][axis].max(point[axis]);
                }
            }
        };
        match self {
            Self::Single(a) => a.rows().into_iter().enumerate().for_each(|(i, r)| each(i, [r[0] as f64, r[1] as f64, r[2] as f64])),
            Self::Double(a) => a.rows().into_iter().enumerate().for_each(|(i, r)| each(i, [r[0], r[1], r[2]])),
        }
        span
    }
    /// The points moved by `shift`, in double precision.
    fn moved(&self, shift: [f64; 3]) -> Self {
        let mut out = match self {
            Self::Single(a) => a.mapv(f64::from),
            Self::Double(a) => a.clone(),
        };
        for mut row in out.rows_mut() {
            for axis in 0..3 {
                row[axis] += shift[axis];
            }
        }
        Self::Double(out)
    }
    /// The unit normal of each face's outer ring (Newell's sum, in this
    /// precision, then widened), for the faces in `wanted`.
    fn normals(&self, corners: &[i64], sizes: &[i64], counts: &[i64], wanted: &[usize]) -> PyResult<Vec<[f64; 3]>> {
        let mut ring_start = Vec::with_capacity(sizes.len() + 1);
        let mut at = 0usize;
        ring_start.push(0);
        for size in sizes {
            at += *size as usize;
            ring_start.push(at);
        }
        let mut first_ring = Vec::with_capacity(counts.len());
        let mut ring = 0usize;
        for count in counts {
            first_ring.push(ring);
            ring += *count as usize;
        }
        let bad = || PyValueError::new_err("face rings do not match their corners");
        let mut out = Vec::with_capacity(wanted.len());
        for face in wanted {
            let ring = *first_ring.get(*face).ok_or_else(bad)?;
            let (begin, end) = (*ring_start.get(ring).ok_or_else(bad)?, *ring_start.get(ring + 1).ok_or_else(bad)?);
            let ring = corners.get(begin..end).ok_or_else(bad)?;
            macro_rules! newell {
                ($points:expr, $float:ty) => {{
                    let mut sum = [0 as $float; 3];
                    for (k, corner) in ring.iter().enumerate() {
                        let next = ring[(k + 1) % ring.len()];
                        let (a, b) = ($points.row(*corner as usize), $points.row(next as usize));
                        let cross = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                        if k == 0 {
                            sum = cross;
                        } else {
                            for axis in 0..3 {
                                sum[axis] += cross[axis];
                            }
                        }
                    }
                    let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
                    let length = if length > (1e-30 as $float) { length } else { 1e-30 as $float };
                    [(sum[0] / length) as f64, (sum[1] / length) as f64, (sum[2] / length) as f64]
                }};
            }
            if ring.iter().any(|c| *c < 0 || *c as usize >= self.rows()) {
                return Err(bad());
            }
            out.push(match self {
                Self::Single(points) => newell!(points, f32),
                Self::Double(points) => newell!(points, f64),
            });
        }
        Ok(out)
    }
}

fn integers(array: &Obj<'_>) -> PyResult<Vec<i64>> {
    let np = array.py().import("numpy")?;
    let wide: PyReadonlyArray1<'_, i64> = np.call_method1("asarray", (array, np.getattr("int64")?))?.extract()?;
    Ok(wide.as_array().iter().copied().collect())
}

/// Use the GL mapping and side resolution, before triangulation splits
/// corners: `(maps, paints)` per face.
#[allow(clippy::too_many_arguments)]
fn material_columns<'py>(
    py: Python<'py>,
    points: &Points,
    corners: &[i64],
    sizes: &[i64],
    counts: &[i64],
    table: &[Obj<'py>],
    attributes: &[i64],
    texture_ids: &Bound<'py, PyDict>,
    origin: [f64; 3],
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let np = py.import("numpy")?;
    let faces = counts.len();
    let shared = || -> PyResult<Obj<'py>> {
        np.call_method1("broadcast_to", (np.call_method1("zeros", (16, np.getattr("float32")?))?, (faces, 16)))
    };
    // The common unpainted prototype has no back or texture rows to resolve.
    let mut any = false;
    for own in table {
        any |= own.is_truthy()?;
    }
    if !any {
        return Ok((shared()?, PyArray2::<u32>::zeros(py, [faces, 3], false).into_any()));
    }
    let materials = py.import("built_core.materials")?;
    let colors = PyList::empty(py);
    let slots = PyList::empty(py);
    let mut textures: Vec<[Option<Obj<'py>>; 2]> = Vec::with_capacity(table.len());
    for own in table {
        let mut back = own.call_method1("get", ("back",))?;
        if !back.is_instance_of::<PyDict>() {
            back = if materials.call_method1("back_is_default", (own,))?.is_truthy()? {
                PyDict::new(py).into_any()
            } else {
                own.clone()
            };
        }
        let mut color = value(&back, "color")?;
        if color.is_none() && present(&back, "pbr")? {
            color = Some((1., 1., 1.).into_bound_py_any(py)?);
        }
        if color.is_none() && (back.call_method1("get", ("texture",))?.is_truthy()? || present(&back, "opacity")?) {
            color = Some((210. / 255., 210. / 255., 210. / 255.).into_bound_py_any(py)?);
        }
        match color {
            Some(color) => colors.append((color.get_item(0)?, color.get_item(1)?, color.get_item(2)?, alpha(&back)?))?,
            None => colors.append(py.None())?,
        }
        let mut ids = [0u32; 2];
        let mut sides = [None, None];
        for (side, attrs) in [own, &back].into_iter().enumerate() {
            let mut texture = attrs.call_method1("get", ("texture",))?;
            let mut path = if texture.is_truthy()? { texture.call_method1("get", ("path",))? } else { py.None().into_bound(py) };
            if let Some(recipe) = value(attrs, "pbr")? {
                path = super::pack::pbr_entry(py, &path, &recipe)?;
                if !texture.is_truthy()? {
                    let unit = PyDict::new(py);
                    unit.set_item("sw", 1.)?;
                    unit.set_item("sh", 1.)?;
                    texture = unit.into_any();
                }
            }
            if path.is_truthy()? {
                if !texture_ids.contains(&path)? {
                    texture_ids.set_item(&path, texture_ids.len() + 1)?;
                }
                ids[side] = texture_ids.get_item(&path)?.expect("just set").extract()?;
            }
            sides[side] = Some(texture).filter(|t| !t.is_none());
        }
        slots.append(ids)?;
        textures.push(sides);
    }
    let paints = py.import("built_core")?.call_method1(
        "pack_materials",
        (colors, slots, PyArray1::from_slice(py, attributes)),
    )?;
    let textured: Vec<usize> = {
        let paints: PyReadonlyArray2<'_, u32> = paints.extract()?;
        let paints = paints.as_array();
        (0..faces).filter(|face| paints[[*face, 1]] != 0 || paints[[*face, 2]] != 0).collect()
    };
    if textured.is_empty() {
        return Ok((shared()?, paints));
    }
    let normals = points.normals(corners, sizes, counts, &textured)?;
    let vec3 = py.import("built_core")?.getattr("Vec3")?;
    let axes = py.import("built_core.texture")?.getattr("face_uv_axes")?;
    let rebased = origin.iter().any(|v| *v != 0.);
    let mut resolved: HashMap<(i64, [u64; 3]), [f32; 16]> = HashMap::new();
    let mut maps = Array2::<f32>::zeros((faces, 16));
    for (face, normal) in textured.iter().zip(&normals) {
        let attribute = attributes[*face];
        // One mapping per distinct (attribute row, normal); -0 is 0.
        let key = (attribute, normal.map(|v| (v + 0.).to_bits()));
        let row = match resolved.get(&key) {
            Some(row) => *row,
            None => {
                let mut row = [0f32; 16];
                for (side, texture) in textures[attribute as usize].iter().enumerate() {
                    let Some(texture) = texture.as_ref().filter(|t| t.is_truthy().unwrap_or(false)) else { continue };
                    let got = axes.call1((texture, vec3.call1((normal[0], normal[1], normal[2]))?))?;
                    let gu: [f64; 3] = got.get_item(0)?.call_method0("toTuple")?.extract()?;
                    let mut cu: f64 = got.get_item(1)?.extract()?;
                    let gv: [f64; 3] = got.get_item(2)?.call_method0("toTuple")?.extract()?;
                    let mut cv: f64 = got.get_item(3)?.extract()?;
                    if rebased {
                        // Tiled UV phase at prototype zero; retaining integer
                        // tile repetitions here would lose the small local UVs.
                        let dot = |g: [f64; 3]| (0..3).fold(0., |sum, k| sum + g[k] * origin[k]);
                        cu = (cu + dot(gu)).rem_euclid(1.);
                        cv = (cv + dot(gv)).rem_euclid(1.);
                    }
                    let at = side * 8;
                    for k in 0..3 {
                        row[at + k] = gu[k] as f32;
                        row[at + 4 + k] = gv[k] as f32;
                    }
                    row[at + 3] = cu as f32;
                    row[at + 7] = cv as f32;
                }
                resolved.insert(key, row);
                row
            }
        };
        maps.row_mut(*face).iter_mut().zip(row).for_each(|(slot, v)| *slot = v);
    }
    Ok((maps.into_pyarray(py).into_any(), paints))
}

/// The bake's eleven arrays for one prototype (`mesh` drawn under `paint`):
/// positions, ring corners, ring sizes, ring counts, edges, edge flags, the
/// colour of each attribute row, each face's hidden flag and attribute row,
/// each face's mapping and its paint slots.
///
/// With `origins` (the pack's list of prototype frames) the prototype gets a
/// frame of its own, appended there as `(origin, box, wide)`.
#[pyfunction]
#[pyo3(signature = (mesh, paint = None, texture_ids = None, face_overrides = None, billboard = None, origins = None,
    previous_origin = None, focus = None, matrix = None))]
#[allow(clippy::too_many_arguments)]
fn source_arrays<'py>(
    py: Python<'py>,
    mesh: &Obj<'py>,
    paint: Option<Obj<'py>>,
    texture_ids: Option<Bound<'py, PyDict>>,
    face_overrides: Option<Obj<'py>>,
    billboard: Option<Obj<'py>>,
    origins: Option<Bound<'py, PyList>>,
    previous_origin: Option<[f64; 3]>,
    focus: Option<Obj<'py>>,
    matrix: Option<Obj<'py>>,
) -> PyResult<Bound<'py, PyTuple>> {
    let np = py.import("numpy")?;
    let overrides = face_overrides.filter(|o| o.is_truthy().unwrap_or(false));
    if overrides.is_some() {
        // Resolve lazy faces before reading topology; never mutate document attrs.
        py.get_type::<PyList>().call1((mesh.getattr("faces")?,))?;
    }
    let own = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    let source = own.get_item("_deferred_source")?.filter(|s| !s.is_none());
    let stored = mesh.get_type().name()? == "StoreMesh";
    let (pos, corners, sizes, counts, edges, flags, table, attr) = match &source {
        // A file's own arrays: one coordinate per face or edge corner.
        Some(source) => {
            let field = |key: &str| source.get_item(key);
            let edges = field("edges")?;
            let flags = match field("flags")? {
                flags if flags.is_none() => np.call_method1("zeros", (edges.len()?, np.getattr("uint8")?))?,
                flags => flags,
            };
            let table = py.import("views.array_bake")?.call_method1("_attrs_table", (source,))?;
            (field("pos")?, field("rings")?, field("sizes")?, field("counts")?, edges, flags, table, field("face_attr")?)
        }
        // A store-backed mesh: its welded arrays, vertex indices already
        // shared by corners and edges.
        None if stored => {
            let arrays = own.get_item("_store")?.ok_or_else(|| PyTypeError::new_err("a store mesh has a store"))?.call_method0("arrays")?;
            let field = |key: &str| arrays.get_item(key);
            let columns = py.import("built_core")?.getattr("store_views")?.call_method1("attr_columns", (mesh, overrides.as_ref()))?;
            (field("pos")?, field("corners")?, field("ring_sizes")?, field("ring_counts")?, field("edges")?,
             field("flags")?.bitand(7)?, columns.get_item(0)?, columns.get_item(1)?)
        }
        None => return Err(PyTypeError::new_err("a resident prototype is a store mesh or a deferred one")),
    };
    let mut points = Points::read(&pos)?;
    let mut corners = integers(&corners)?;
    let sizes = integers(&sizes)?;
    let counts = integers(&counts)?;
    let edges: PyReadonlyArray2<'_, i64> = np.call_method1("asarray", (edges, np.getattr("int64")?))?.call_method1("reshape", (-1, 2))?.extract()?;
    let mut edges = owned(edges.as_array());
    let flags: PyReadonlyArray1<'_, u8> = np.call_method1("asarray", (flags, np.getattr("uint8")?))?.extract()?;
    let flags = owned(flags.as_array());
    let attr = integers(&attr)?;

    let mut origin = [0.; 3];
    if let Some(origins) = &origins {
        let frame: [f64; 3] = if stored {
            mesh.getattr("_store")?.getattr("frame_origin")?.extract()?
        } else {
            [0.; 3]
        };
        let span = points.span();
        let wide = (0..3).map(|k| span[1][k] - span[0][k]).fold(f64::NEG_INFINITY, f64::max) > 2. * REBASE_DISTANCE;
        let mut center: [f64; 3] = std::array::from_fn(|k| (span[0][k] + span[1][k]) / 2. + frame[k]);
        if let Some(focus) = focus.as_ref().filter(|_| wide) {
            let mut local = np.call_method1("asarray", (focus,))?;
            if let Some(matrix) = &matrix {
                // Singular placements still draw their projected subspace.
                let linear = matrix.get_item((pyo3::types::PySlice::new(py, 0, 3, 1), pyo3::types::PySlice::new(py, 0, 3, 1)))?;
                let translation = matrix.get_item((3, pyo3::types::PySlice::new(py, 0, 3, 1)))?;
                local = local.sub(translation)?.matmul(np.getattr("linalg")?.call_method1("pinv", (linear,))?)?;
            }
            let local: [f64; 3] = local.extract()?;
            center = std::array::from_fn(|k| {
                let (low, high) = (span[0][k] + frame[k], span[1][k] + frame[k]);
                // np.clip: the lower bound first, then the upper.
                local[k].max(low).min(high)
            });
        }
        origin = choose_origin(center, previous_origin.unwrap_or([0.; 3]));
        let shift: [f64; 3] = std::array::from_fn(|k| frame[k] - origin[k]);
        let moved_box: Vec<f64> = span.iter().flat_map(|corner| (0..3).map(|k| corner[k] + shift[k])).collect();
        origins.append((
            PyTuple::new(py, origin)?,
            PyArray1::from_vec(py, moved_box).reshape([2, 3])?,
            wide,
        ))?;
        if frame != origin {
            points = points.moved(shift);
        }
    }
    // IGZ source streams can repeat a coordinate for each face/edge. Soft fan
    // adjacency requires common vertex IDs; weld only prototypes that need it.
    if flags.iter().any(|f| f & 1 != 0) {
        let remap = py.import("built_core")?.call_method1("pack_weld", (points.single().into_pyarray(py),))?;
        let remap = integers(&remap)?;
        let through = |index: i64| -> PyResult<i64> {
            usize::try_from(index).ok().and_then(|i| remap.get(i).copied()).ok_or_else(|| PyValueError::new_err("corner index out of range"))
        };
        for corner in &mut corners {
            *corner = through(*corner)?;
        }
        for end in edges.iter_mut() {
            *end = through(*end)?;
        }
    }
    // The attrs each row is drawn with under the container's paint; row 0
    // is a face without attrs of its own.
    let materials = py.import("built_core.materials")?;
    let paint = paint.unwrap_or_else(|| py.None().into_bound(py));
    let mut rows: Vec<Obj<'py>> = Vec::with_capacity(table.len()? + 1);
    let effective = |own: Obj<'py>| -> PyResult<Obj<'py>> {
        let got = materials.call_method1("effective_attrs", (own, &paint))?;
        Ok(if got.is_truthy()? { got } else { PyDict::new(py).into_any() })
    };
    rows.push(effective(PyDict::new(py).into_any())?);
    for own in table.try_iter()? {
        rows.push(effective(own?)?);
    }
    let mut palette = Array2::<u8>::from_elem((rows.len(), 4), 210);
    let mut hidden_rows = vec![false; rows.len()];
    for (k, own) in rows.iter().enumerate() {
        palette[[k, 3]] = 0;
        hidden_rows[k] = own.call_method1("get", ("hidden",))?.is_truthy()?;
        let color = value(own, "color")?;
        if let Some(color) = &color {
            let rgb = rgb_bytes(color)?;
            for axis in 0..3 {
                palette[[k, axis]] = rgb[axis];
            }
            palette[[k, 3]] = alpha(own)?;
        }
        // Texture-only and opacity-only faces also need a nonzero opacity byte.
        let pbr = present(own, "pbr")?;
        if pbr && color.is_none() {
            for axis in 0..3 {
                palette[[k, axis]] = 255;
            }
        }
        if own.call_method1("get", ("texture",))?.is_truthy()? || present(own, "opacity")? || pbr {
            palette[[k, 3]] = alpha(own)?;
        }
    }
    let attributes: Vec<i64> = attr.iter().map(|a| a + 1).collect();
    if attributes.iter().any(|a| *a < 0 || *a as usize >= rows.len()) {
        return Err(PyValueError::new_err("face attribute out of range"));
    }
    let ids = texture_ids.unwrap_or_else(|| PyDict::new(py));
    let (mut maps, paints) = material_columns(py, &points, &corners, &sizes, &counts, &rows, &attributes, &ids, origin)?;
    let single = points.single().into_pyarray(py);
    let corners = PyArray1::from_vec(py, corners);
    let sizes = PyArray1::from_vec(py, sizes);
    let counts = PyArray1::from_vec(py, counts);
    if billboard.is_some_and(|b| b.is(&true.into_bound_py_any(py).expect("bool"))) {
        // The sprite's image fitted to its faces; GL's cutout uses the same
        // image on either side.
        maps = py.import("views.wgpu_billboards")?.call_method1(
            "fitted_maps",
            (points.array(py), &corners, &sizes, &counts, maps.call_method0("copy")?),
        )?;
        let column = |at: usize| (pyo3::types::PySlice::full(py), at);
        paints.set_item(column(2), paints.get_item(column(1))?)?;
    }
    let hidden: Vec<bool> = attributes.iter().map(|a| hidden_rows[*a as usize]).collect();
    PyTuple::new(
        py,
        [
            single.into_any(),
            corners.into_any(),
            sizes.into_any(),
            counts.into_any(),
            edges.into_pyarray(py).into_any(),
            flags.into_pyarray(py).into_any(),
            palette.into_pyarray(py).into_any(),
            Array1::from(hidden).into_pyarray(py).into_any(),
            PyArray1::from_vec(py, attributes).into_any(),
            maps,
            paints,
        ],
    )
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(source_arrays, m)?)
}
