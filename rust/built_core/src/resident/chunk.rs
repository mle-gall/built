// SPDX-License-Identifier: GPL-3.0-or-later
//! The drawing chunk of a lazy mesh, baked from its arrays
//! (`views.array_bake._bake`): the very chunk the object bake makes from
//! the built mesh, field for field, with no vertex, edge or face object
//! made, so a model that is only looked at stays deferred.
//!
//! The face geometry comes from the kernel (`face_geometry`), the topology
//! from the mesh's plan. Everything here is one pass over those arrays; the
//! document is asked once per attribute row (its paint) and once per
//! distinct textured normal (its mapping). The arithmetic is the object
//! bake's, operation for operation: `tests/test_array_bake.py` holds the
//! two to each other.
use numpy::{PyArray1, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyTuple},
    IntoPyObjectExt,
};

type Obj<'py> = Bound<'py, PyAny>;
type V3 = [f64; 3];

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn numpy(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("numpy")
}
/// `(n, 3)` of float64.
fn vectors(array: &Obj<'_>) -> PyResult<Vec<V3>> {
    let np = numpy(array.py())?;
    let array: PyReadonlyArray2<'_, f64> = np.call_method1("ascontiguousarray", (array, np.getattr("float64")?))?.extract()?;
    Ok(array.as_array().outer_iter().map(|row| [row[0], row[1], row[2]]).collect())
}
/// Any integer array, flattened.
fn integers(array: &Obj<'_>) -> PyResult<Vec<i64>> {
    let np = numpy(array.py())?;
    let array: PyReadonlyArray1<'_, i64> = np.call_method1("ascontiguousarray", (array, np.getattr("int64")?))?.call_method1("reshape", (-1,))?.extract()?;
    Ok(array.as_array().iter().copied().collect())
}
fn flags(array: &Obj<'_>) -> PyResult<Vec<bool>> {
    let np = numpy(array.py())?;
    let array: PyReadonlyArray1<'_, bool> = np.call_method1("ascontiguousarray", (array, np.getattr("bool_")?))?.extract()?;
    Ok(array.as_array().iter().copied().collect())
}
/// `(n, width)` of float64 from its rows.
fn rows_of<'py>(py: Python<'py>, values: Vec<f64>, width: usize) -> PyResult<Obj<'py>> {
    let count = values.len() / width;
    Ok(PyArray1::from_vec(py, values).reshape([count, width])?.into_any())
}
fn triples<'py>(py: Python<'py>, values: &[V3]) -> PyResult<Obj<'py>> {
    rows_of(py, values.iter().flatten().copied().collect(), 3)
}

/// `np.minimum`, `np.maximum`: a NaN on either side is the answer.
fn least(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}
fn most(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
/// The viewport's dot product, in its order of operations.
fn dot(a: V3, b: V3) -> f64 {
    (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]
}
/// The viewport's binary64 normal and light calculation.
fn shade(vector: V3, light: V3) -> f64 {
    let length = dot(vector, vector).sqrt();
    let inverse = if length > 0. { 1. / length } else { 0. };
    let unit = [vector[0] * inverse, vector[1] * inverse, vector[2] * inverse];
    let lit = dot(unit, light).abs();
    if length < 1e-9 {
        0.84375
    } else {
        ((0.62 + 0.38 * lit) * 32.).round_ties_even() / 32.
    }
}
/// `vp._shaded_color`.
fn shaded(base: V3, shade: f64) -> V3 {
    base.map(|channel| (least(1.0, channel * shade) * 64.).round_ties_even() / 64.)
}
fn put(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[derive(Clone, Copy, PartialEq)]
enum Back {
    None,
    Colour,
    ClearColour,
    Texture,
    ClearTexture,
}

/// What one attribute row (and the container's paint) decides of a face.
struct Row<'py> {
    hidden: bool,
    textured: bool,
    opacity: f64,
    /// `round(opacity, 3)`.
    opacity3: f64,
    cull: i64,
    default_back: bool,
    base: Option<V3>,
    texture: Obj<'py>,
    path: usize,
    back: Back,
    back_colour: V3,
    back_texture: Obj<'py>,
    back_path: usize,
    /// `round(back opacity, 3)`.
    back_opacity3: f64,
}

/// The bucket a back side's triangles go to.
#[derive(Clone, Copy, PartialEq)]
enum BackName {
    Colour,
    ClearColour(f64),
    Texture(usize, usize),
    ClearTexture(usize, usize, f64),
}
/// The bucket a textured front's triangles go to.
#[derive(Clone, Copy, PartialEq)]
enum FrontName {
    Opaque(usize, usize),
    Clear(f64, i64, usize, usize),
}

/// The code of `name` among `names`, added when new: the order a dict filled
/// name by name keeps.
fn code_of<T: PartialEq + Copy>(names: &mut Vec<T>, name: T) -> usize {
    match names.iter().position(|known| *known == name) {
        Some(code) => code,
        None => {
            names.push(name);
            names.len() - 1
        }
    }
}
/// The distinct codes of a per-face array in the order their first face
/// comes.
fn ordered(codes: &[i64]) -> Vec<i64> {
    let mut seen = Vec::new();
    for code in codes {
        if *code >= 0 && !seen.contains(code) {
            seen.push(*code);
        }
    }
    seen
}
fn first3(colour: &Obj<'_>) -> PyResult<V3> {
    let mut out = [0.; 3];
    let mut channels = colour.try_iter()?;
    for slot in &mut out {
        *slot = channels.next().ok_or_else(|| PyValueError::new_err("a colour has three channels"))??.extract()?;
    }
    Ok(out)
}

/// World to UV maps of textured faces: one per distinct (attribute row,
/// plain normal), asked of the document.
struct Mapping {
    /// Per face, its map (`usize::MAX`: none).
    of_face: Vec<usize>,
    /// `(gu, cu, gv, cv)` per map.
    maps: Vec<(V3, f64, V3, f64)>,
}

impl Mapping {
    fn new<'py>(py: Python<'py>, faces: &[usize], row_of: &[usize], normal: &[V3], texture: impl Fn(usize) -> Obj<'py>) -> PyResult<Self> {
        let key = |face: usize| -> [u64; 4] {
            let n = normal[face];
            [row_of[face] as u64, n[0].to_bits(), n[1].to_bits(), n[2].to_bits()]
        };
        let mut distinct: Vec<[u64; 4]> = faces.iter().map(|face| key(*face)).collect();
        distinct.sort_unstable();
        distinct.dedup();
        let axes = py.import("core.texture")?.getattr("face_uv_axes")?;
        let vector = py.import("built_core")?.getattr("Vec3")?;
        let mut maps = Vec::with_capacity(distinct.len());
        for entry in &distinct {
            let n = vector.call1((f64::from_bits(entry[1]), f64::from_bits(entry[2]), f64::from_bits(entry[3])))?;
            let map = axes.call1((texture(entry[0] as usize), n))?;
            let along = |at: usize| -> PyResult<V3> { map.get_item(at)?.call_method0("toTuple")?.extract() };
            maps.push((along(0)?, map.get_item(1)?.extract()?, along(2)?, map.get_item(3)?.extract()?));
        }
        let mut of_face = vec![usize::MAX; normal.len()];
        for face in faces {
            of_face[*face] = distinct.binary_search(&key(*face)).unwrap_or(usize::MAX);
        }
        Ok(Self { of_face, maps })
    }
    /// `u, v` of point `p` of `face`, zero without a map.
    fn uv(&self, face: usize, p: V3) -> (f32, f32) {
        match self.maps.get(self.of_face[face]) {
            Some((gu, cu, gv, cv)) => ((dot(p, *gu) + cu) as f32, (dot(p, *gv) + cv) as f32),
            None => (0., 0.),
        }
    }
}

/// The distinct `(row, shade)` pairs of `faces`, sorted, and each face's
/// place among them.
fn shade_pairs(faces: &[usize], row_of: &[usize], shade_ix: &[usize]) -> (Vec<(usize, usize)>, Vec<usize>) {
    let mut distinct: Vec<(usize, usize)> = faces.iter().map(|face| (row_of[*face], shade_ix[*face])).collect();
    distinct.sort_unstable();
    distinct.dedup();
    let at = faces.iter().map(|face| distinct.binary_search(&(row_of[*face], shade_ix[*face])).unwrap_or(0)).collect();
    (distinct, at)
}

/// `bake`, all but the entry's number and the scene's version.
#[pyfunction]
pub fn bake_deferred_chunk<'py>(py: Python<'py>, vp: &Obj<'py>, mesh: &Obj<'py>, paint: &Obj<'py>, msig: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let np = numpy(py)?;
    let module = py.import("views.array_bake")?;
    let materials = py.import("built_core.materials")?;
    let builtins = py.import("builtins")?;
    let round3 = |value: f64| -> PyResult<f64> { builtins.call_method1("round", (value, 3))?.extract() };
    let bytes = |data: &[u8]| PyBytes::new(py, data);

    let plan = module.getattr("lazy_mesh")?.call_method1("plan_of", (mesh,))?;
    let source = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
    let geometry = module.call_method1("_geometry", (mesh, &plan))?;
    let part = |at: usize| geometry.get_item(at);
    let (normal, length, centroid, unit) = (vectors(&part(0)?)?, part(1)?, vectors(&part(2)?)?, vectors(&part(5)?)?);
    let length: Vec<f64> = np.call_method1("ascontiguousarray", (length, np.getattr("float64")?))?.extract::<PyReadonlyArray1<'_, f64>>()?.as_array().iter().copied().collect();
    let (tris_a, face_a, tris_b, face_b) = (integers(&part(3)?)?, integers(&part(4)?)?, integers(&part(6)?)?, integers(&part(7)?)?);
    let table = module.call_method1("_attrs_table", (&source,))?;
    let face_attr_array = np.call_method("asarray", (source.get_item("face_attr")?,), Some(&{
        let kw = PyDict::new(py);
        kw.set_item("dtype", np.getattr("int64")?)?;
        kw
    }))?;
    let face_attr = integers(&face_attr_array)?;
    let nf: usize = plan.getattr("nfaces")?.extract()?;
    let positions = plan.getattr("pos")?;
    let pos = vectors(&positions)?;
    let light: V3 = vp.getattr("_LIGHT")?.call_method0("toTuple")?.extract()?;

    // Everything but the normal is decided by a face's attrs, i.e. by its
    // table entry (k; -1: no attrs): one row per entry.
    let mut keys: Vec<i64> = face_attr.iter().copied().filter(|key| *key >= 0).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.insert(0, -1);
    let paths = PyDict::new(py);
    let path_id = |texture: &Obj<'py>| -> PyResult<usize> {
        let path = texture.get_item("path")?;
        if let Some(known) = paths.get_item(&path)? {
            return known.extract();
        }
        let id = paths.len();
        paths.set_item(path, id)?;
        Ok(id)
    };
    let mut rows = Vec::with_capacity(keys.len());
    for key in &keys {
        let own = if *key >= 0 { table.get_item(*key)? } else { PyDict::new(py).into_any() };
        let attrs = materials.call_method1("effective_attrs", (&own, paint))?;
        let texture = attrs.call_method1("get", ("texture",))?;
        let back = attrs.call_method1("get", ("back",))?;
        let textured = !texture.is_none() && texture.call_method1("get", ("path",))?.is_truthy()?;
        let opacity: f64 = builtins.call_method1("float", (attrs.call_method1("get", ("opacity", 1.0))?,))?.extract()?;
        let colour = attrs.call_method1("get", ("color",))?;
        let base = if colour.is_none() { vp.getattr("DEFAULT_FACE_COLOR")? } else { colour };
        let mut row = Row {
            hidden: own.call_method1("get", ("hidden",))?.is_truthy()?,
            textured,
            opacity,
            opacity3: round3(opacity)?,
            cull: 0,
            default_back: materials.call_method1("back_is_default", (&attrs,))?.is_truthy()?,
            base: first3(&base).ok(),
            path: if textured { path_id(&texture)? } else { 0 },
            texture,
            back: Back::None,
            back_colour: [0.; 3],
            back_texture: none(py),
            back_path: 0,
            back_opacity3: 0.,
        };
        if back.is_instance_of::<PyDict>() {
            let back_opacity: f64 = builtins.call_method1("float", (back.call_method1("get", ("opacity", 1.0))?,))?.extract()?;
            let clear = back_opacity < 0.999;
            let back_texture = back.call_method1("get", ("texture",))?;
            let back_colour = back.call_method1("get", ("color",))?;
            row.back_opacity3 = round3(back_opacity)?;
            if !back_texture.is_none() && back_texture.call_method1("get", ("path",))?.is_truthy()? {
                row.back = if clear { Back::ClearTexture } else { Back::Texture };
                row.back_path = path_id(&back_texture)?;
                row.back_texture = back_texture;
            } else if !back_colour.is_none() {
                row.back = if clear { Back::ClearColour } else { Back::Colour };
                row.back_colour = first3(&back_colour)?;
            }
        }
        row.cull = matches!(row.back, Back::ClearColour | Back::ClearTexture) as i64;
        rows.push(row);
    }
    let path_at = |id: usize| -> PyResult<Obj<'py>> {
        for (path, known) in paths.iter() {
            if known.extract::<usize>()? == id {
                return Ok(path);
            }
        }
        Ok(none(py))
    };
    // np.searchsorted(keys, face_attr): every attr is one of the keys.
    let row_of: Vec<usize> = face_attr.iter().map(|key| keys.partition_point(|known| known < key)).collect();
    if row_of.len() != nf || normal.len() != nf || unit.len() != nf {
        return Err(PyValueError::new_err("the plan and the face arrays differ"));
    }
    let drawn_face: Vec<bool> = row_of.iter().map(|row| !rows[*row].hidden).collect();
    let drawn: Vec<i64> = (0..nf as i64).filter(|face| drawn_face[*face as usize]).collect();
    let mut draw_index = vec![-1i64; nf];
    for (at, face) in drawn.iter().enumerate() {
        draw_index[*face as usize] = at as i64;
    }
    let faces = module.getattr("DeferredFaces")?.call1((mesh, PyArray1::from_vec(py, drawn.clone())))?;

    // The two triangle streams: by the plain normal (colour faces, pick)
    // and by the unit normal (textures, back sides).
    let corner = |tris: &[i64], triangle: usize, at: usize| pos[tris[triangle * 3 + at] as usize];
    let (count_a, count_b) = (face_a.len(), face_b.len());
    let mut has_tris = vec![false; nf];
    for face in &face_a {
        has_tris[*face as usize] = true;
    }

    // The shade of every face, by both normals.
    let shade_a: Vec<f64> = normal.iter().map(|n| shade(*n, light)).collect();
    let shade_b: Vec<f64> = unit.iter().map(|n| shade(*n, light)).collect();
    let mut shades = shade_b.clone();
    // As np.unique: ascending, every NaN one value at the end.
    shades.sort_by(|a, b| match (a.is_nan(), b.is_nan()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        _ => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
    });
    shades.dedup_by(|a, b| a == b || (a.is_nan() && b.is_nan()));
    let shade_ix: Vec<usize> = shade_b
        .iter()
        .map(|value| if value.is_nan() { shades.len() - 1 } else { shades.partition_point(|known| known < value) })
        .collect();
    let shade_values = PyList::new(py, &shades)?;
    let texture_key = |path: usize, shade: usize| -> PyResult<Obj<'py>> { (path_at(path)?, shade_values.get_item(shade)?).into_bound_py_any(py) };

    // ---- back sides (two-side paint) -----------------------------------
    let mut back_code = vec![-1i64; nf];
    let mut back_rgb = vec![[0.; 3]; nf];
    let mut back_names: Vec<BackName> = Vec::new();
    let coloured: Vec<usize> = (0..nf).filter(|face| drawn_face[*face] && matches!(rows[row_of[*face]].back, Back::Colour | Back::ClearColour)).collect();
    if !coloured.is_empty() {
        // Colour backs: the bucket is the key's; the shade the unit normal's.
        let mut code_of_row = vec![-1i64; rows.len()];
        for (at, row) in rows.iter().enumerate() {
            match row.back {
                Back::Colour => code_of_row[at] = code_of(&mut back_names, BackName::Colour) as i64,
                Back::ClearColour => code_of_row[at] = code_of(&mut back_names, BackName::ClearColour(row.back_opacity3)) as i64,
                _ => {}
            }
        }
        for face in &coloured {
            back_rgb[*face] = shaded(rows[row_of[*face]].back_colour, shade_b[*face]);
            back_code[*face] = code_of_row[row_of[*face]];
        }
    }
    // Textured backs: the bucket is (texture, shade of the unit normal).
    let pictured: Vec<usize> = (0..nf).filter(|face| drawn_face[*face] && matches!(rows[row_of[*face]].back, Back::Texture | Back::ClearTexture)).collect();
    let mut back_maps = None;
    if !pictured.is_empty() {
        let (distinct, at) = shade_pairs(&pictured, &row_of, &shade_ix);
        let codes: Vec<i64> = distinct
            .iter()
            .map(|(row, shade)| {
                let row = &rows[*row];
                let name = if row.back == Back::Texture { BackName::Texture(row.back_path, *shade) } else { BackName::ClearTexture(row.back_path, *shade, row.back_opacity3) };
                code_of(&mut back_names, name) as i64
            })
            .collect();
        for (face, at) in pictured.iter().zip(at) {
            back_code[*face] = codes[at];
        }
        back_maps = Some(Mapping::new(py, &pictured, &row_of, &normal, |row| rows[row].back_texture.clone())?);
    }
    // The buckets in the order their first face fills them.
    let (back_vcol, back_tex, back_tcol, back_ttex) = (PyList::empty(py), PyDict::new(py), PyDict::new(py), PyDict::new(py));
    let extend = |buckets: &Bound<'py, PyDict>, key: Obj<'py>, data: &[u8]| -> PyResult<()> { buckets.as_any().call_method1("setdefault", (key, PyList::empty(py)))?.call_method1("append", (bytes(data),)).map(drop) };
    for code in ordered(&back_code) {
        let name = back_names[code as usize];
        let textured = matches!(name, BackName::Texture(..) | BackName::ClearTexture(..));
        let mut data = Vec::new();
        for triangle in 0..count_b {
            let face = face_b[triangle] as usize;
            if back_code[face] != code {
                continue;
            }
            for at in 0..3 {
                let p = corner(&tris_b, triangle, at);
                p.iter().for_each(|value| put(&mut data, *value as f32));
                if textured {
                    let (u, v) = back_maps.as_ref().map_or((0., 0.), |maps| maps.uv(face, p));
                    put(&mut data, u);
                    put(&mut data, v);
                } else {
                    back_rgb[face].iter().for_each(|value| put(&mut data, *value as f32));
                }
            }
        }
        match name {
            BackName::Colour => back_vcol.append(bytes(&data))?,
            BackName::ClearColour(opacity) => extend(&back_tcol, opacity.into_bound_py_any(py)?, &data)?,
            BackName::Texture(path, shade) => extend(&back_tex, texture_key(path, shade)?, &data)?,
            BackName::ClearTexture(path, shade, opacity) => extend(&back_ttex, (texture_key(path, shade)?, opacity).into_bound_py_any(py)?, &data)?,
        }
    }

    // ---- fronts ---------------------------------------------------------
    let textured_face: Vec<bool> = (0..nf).map(|face| rows[row_of[face]].textured && drawn_face[face]).collect();
    let plain: Vec<bool> = (0..nf).map(|face| !rows[row_of[face]].textured && drawn_face[face]).collect();
    let (by_texture, by_clear_texture) = (PyDict::new(py), PyDict::new(py));
    let pictured: Vec<usize> = (0..nf).filter(|face| textured_face[*face]).collect();
    if !pictured.is_empty() {
        let mut front_names: Vec<FrontName> = Vec::new();
        let mut front_code = vec![-1i64; nf];
        let (distinct, at) = shade_pairs(&pictured, &row_of, &shade_ix);
        let codes: Vec<i64> = distinct
            .iter()
            .map(|(row, shade)| {
                let row = &rows[*row];
                let name = if row.opacity >= 0.999 { FrontName::Opaque(row.path, *shade) } else { FrontName::Clear(row.opacity3, row.cull, row.path, *shade) };
                code_of(&mut front_names, name) as i64
            })
            .collect();
        for (face, at) in pictured.iter().zip(at) {
            front_code[*face] = codes[at];
        }
        let maps = Mapping::new(py, &pictured, &row_of, &normal, |row| rows[row].texture.clone())?;
        for code in ordered(&front_code) {
            let mut data = Vec::new();
            for triangle in 0..count_b {
                let face = face_b[triangle] as usize;
                if front_code[face] != code {
                    continue;
                }
                for at in 0..3 {
                    let p = corner(&tris_b, triangle, at);
                    p.iter().for_each(|value| put(&mut data, *value as f32));
                    let (u, v) = maps.uv(face, p);
                    put(&mut data, u);
                    put(&mut data, v);
                }
            }
            match front_names[code as usize] {
                FrontName::Opaque(path, shade) => by_texture.set_item(texture_key(path, shade)?, bytes(&data))?,
                FrontName::Clear(opacity, cull, path, shade) => {
                    by_clear_texture.as_any().call_method1("setdefault", ((opacity, cull), PyDict::new(py)))?.set_item(texture_key(path, shade)?, bytes(&data))?;
                }
            }
        }
    }

    // Colour faces: their shade by the plain normal. RGB only: an imported
    // colour may carry its alpha as a fourth value (opacity is its own
    // attr), which the object bake ignores.
    let base_of = |face: usize| -> PyResult<V3> { rows[row_of[face]].base.ok_or_else(|| PyValueError::new_err("a face colour has three channels")) };
    let mut front_rgb = vec![[0.; 3]; nf];
    for face in 0..nf {
        if plain[face] {
            front_rgb[face] = shaded(base_of(face)?, shade_a[face]);
        }
    }
    // 0 vcol, 1 fvcol, 2+ tcol
    let mut colour_code = vec![-1i64; nf];
    let mut clear_names: Vec<(f64, i64)> = Vec::new();
    for face in 0..nf {
        if !plain[face] {
            continue;
        }
        let row = &rows[row_of[face]];
        colour_code[face] = if row.opacity < 0.999 {
            2 + code_of(&mut clear_names, (row.opacity3, row.cull)) as i64
        } else {
            row.cull
        };
    }
    // Per triangle, per corner: position then colour.
    let mut colours: Vec<[f32; 3]> = Vec::with_capacity(count_a * 3);
    for triangle in 0..count_a {
        let rgb = front_rgb[face_a[triangle] as usize].map(|value| value as f32);
        colours.extend([rgb; 3]);
    }
    // Native soft fans, retaining array geometry and shared prototypes.
    let mut smooth_vcol = Vec::new();
    let smooth = plan.getattr("smooth")?;
    let fan_starts = integers(&plan.getattr("ef_start")?)?;
    let shared: Vec<i64> = fan_starts.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let soft_fans = count_a != 0 && flags(&smooth)?.iter().zip(&shared).any(|(soft, faces)| *soft && *faces > 1);
    if soft_fans {
        let native = module.getattr("_native")?;
        let int64 = np.getattr("int64")?;
        let kernel = native.getattr("native")?.getattr("smooth_corner_normals")?;
        let result = kernel.call1(PyTuple::new(
            py,
            [
                np.call_method1("ascontiguousarray", (&positions, np.getattr("float64")?))?,
                part(0)?,
                part(5)?,
                plan.getattr("corners")?,
                plan.getattr("ring_sizes")?,
                plan.getattr("ring_counts")?,
                plan.getattr("edges")?,
                smooth.clone(),
                plan.getattr("ef_start")?,
                plan.getattr("ef_face")?,
                part(3)?.call_method1("astype", (&int64,))?,
                part(4)?.call_method1("astype", (&int64,))?,
                native.call_method0("qt_hypot")?,
            ],
        )?)?;
        let normals = vectors(&result.get_item(0)?.call_method1("reshape", (-1, 3))?)?;
        let lit = flags(&result.get_item(1)?)?;
        for triangle in 0..count_a {
            let face = face_a[triangle] as usize;
            for at in 0..3 {
                let corner = triangle * 3 + at;
                if lit[corner] && plain[face] {
                    let shade = 0.62 + 0.38 * dot(normals[corner], light).abs();
                    colours[corner] = base_of(face)?.map(|channel| least(1.0, channel * shade) as f32);
                }
            }
            if colour_code[face] == 0 {
                for at in 0..3 {
                    normals[triangle * 3 + at].iter().for_each(|value| put(&mut smooth_vcol, *value as f32));
                }
            }
        }
    }
    let stream = |code: i64| -> Vec<u8> {
        let mut data = Vec::new();
        for triangle in 0..count_a {
            if colour_code[face_a[triangle] as usize] != code {
                continue;
            }
            for at in 0..3 {
                corner(&tris_a, triangle, at).iter().for_each(|value| put(&mut data, *value as f32));
                colours[triangle * 3 + at].iter().for_each(|value| put(&mut data, *value));
            }
        }
        data
    };
    let clear_colours = PyDict::new(py);
    let mut clear_codes = colour_code.clone();
    clear_codes.iter_mut().for_each(|code| {
        if *code < 2 {
            *code = -1;
        }
    });
    for code in ordered(&clear_codes) {
        clear_colours.set_item(clear_names[code as usize - 2], bytes(&stream(code)))?;
    }

    // Pick triangles, areas, the default-back tint.
    let areas: Vec<f64> = drawn.iter().map(|face| 0.5 * length[*face as usize]).collect();
    let default_back: Vec<bool> = (0..nf).map(|face| drawn_face[face] && has_tris[face] && rows[row_of[face]].default_back).collect();
    let (mut v0, mut e1, mut e2, mut owner, mut tint) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut low, mut high): (Option<V3>, Option<V3>) = (None, None);
    let mut widen = |point: V3| {
        low = Some(low.map_or(point, |low| [least(low[0], point[0]), least(low[1], point[1]), least(low[2], point[2])]));
        high = Some(high.map_or(point, |high| [most(high[0], point[0]), most(high[1], point[1]), most(high[2], point[2])]));
    };
    let subtract = |a: V3, b: V3| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let add = |a: V3, b: V3| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
    for triangle in 0..count_a {
        let face = face_a[triangle] as usize;
        if !drawn_face[face] {
            continue;
        }
        let t = [corner(&tris_a, triangle, 0), corner(&tris_a, triangle, 1), corner(&tris_a, triangle, 2)];
        let (first, second) = (subtract(t[1], t[0]), subtract(t[2], t[0]));
        v0.push(t[0]);
        e1.push(first);
        e2.push(second);
        owner.push(draw_index[face]);
        if default_back[face] {
            t.iter().flatten().for_each(|value| put(&mut tint, *value as f32));
        }
    }
    // As np.concatenate((v0, v0 + e1, v0 + e2)).min/max: the corners as
    // the pick reads them back.
    for at in 0..v0.len() {
        widen(v0[at]);
    }
    for at in 0..v0.len() {
        widen(add(v0[at], e1[at]));
    }
    for at in 0..v0.len() {
        widen(add(v0[at], e2[at]));
    }

    // Edges: hard ones drawn, soft ones kept for the silhouettes with the
    // normal and centroid of their (one or two) faces.
    let edges = integers(&plan.getattr("edges")?)?;
    let (hidden_edge, soft_edge) = (flags(&plan.getattr("hidden")?)?, flags(&plan.getattr("soft")?)?);
    let fan_faces = integers(&plan.getattr("ef_face")?)?;
    let edge_count = hidden_edge.len();
    let end = |edge: usize, at: usize| pos[edges[edge * 2 + at] as usize].map(|value| value as f32);
    let hard: Vec<usize> = (0..edge_count).filter(|edge| !hidden_edge[*edge] && !soft_edge[*edge]).collect();
    let softs: Vec<usize> = (0..edge_count).filter(|edge| !hidden_edge[*edge] && soft_edge[*edge] && (shared[*edge] == 1 || shared[*edge] == 2)).collect();
    let mut edge_data = Vec::with_capacity(hard.len() * 24);
    for edge in &hard {
        for at in 0..2 {
            let point = end(*edge, at);
            point.iter().for_each(|value| put(&mut edge_data, *value));
            widen(point.map(f64::from));
        }
    }
    // The layer each drawn edge carries of its own (None: its object's), in
    // the order of `edges` and of `soft_pts`: what a sheet inks a line
    // with, read here so the hidden-line pass need not walk the mesh.
    let layers = plan.getattr("layer")?;
    let layers_of = |edges: &[usize]| -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        for edge in edges {
            out.append(layers.get_item(*edge)?)?;
        }
        Ok(out)
    };
    let mut centre = centroid;
    if centre.iter().any(|c| c[0].is_nan()) {
        // Under 3 corners: Face.centroid.
        let (sizes, counts, corners) = (integers(&plan.getattr("ring_sizes")?)?, integers(&plan.getattr("ring_counts")?)?, integers(&plan.getattr("corners")?)?);
        let mut ring_start = vec![0usize];
        sizes.iter().for_each(|size| ring_start.push(ring_start[ring_start.len() - 1] + *size as usize));
        let mut first_ring = 0usize;
        for face in 0..nf {
            if centre[face][0].is_nan() {
                let ring = &corners[ring_start[first_ring]..ring_start[first_ring + 1]];
                let mut sum = [0f64; 3];
                for index in ring {
                    let point = pos[*index as usize].map(|value| value as f32 as f64);
                    sum = add(sum, point);
                }
                centre[face] = if ring.is_empty() { [0.; 3] } else { sum.map(|total| (total / ring.len() as f64) as f32 as f64) };
            }
            first_ring += counts[face] as usize;
        }
    }
    let chunk = PyDict::new(py);
    let empty = || np.call_method1("zeros", ((0,),));
    let (soft_points, soft_n0, soft_c0, soft_n1, soft_c1, soft_single) = if softs.is_empty() {
        (none(py), empty()?, empty()?, empty()?, empty()?, np.call_method1("zeros", (0, np.getattr("bool_")?))?)
    } else {
        let mut points = Vec::with_capacity(softs.len() * 6);
        let (mut n0, mut c0, mut n1, mut c1, mut single) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for edge in &softs {
            for at in 0..2 {
                points.extend(end(*edge, at));
            }
            let start = fan_starts[*edge] as usize;
            let two = shared[*edge] == 2;
            let first = fan_faces[start] as usize;
            let second = if two { fan_faces[(start + 1).min(fan_faces.len() - 1)] as usize } else { first };
            n0.push(normal[first]);
            c0.push(centre[first]);
            n1.push(normal[second]);
            c1.push(centre[second]);
            single.push(!two);
        }
        let count = softs.len();
        (
            PyArray1::from_vec(py, points).reshape([count, 6])?.into_any(),
            triples(py, &n0)?,
            triples(py, &c0)?,
            triples(py, &n1)?,
            triples(py, &c1)?,
            PyArray1::from_vec(py, single).into_any(),
        )
    };
    let bbox = match (low, high) {
        (Some(low), Some(high)) => ((low[0], low[1], low[2]), (high[0], high[1], high[2])).into_bound_py_any(py)?,
        _ => none(py),
    };
    let fingerprint = module.getattr("LazyFingerprint")?.call1(((mesh, &plan, &table, &face_attr_array),))?;
    let nv = pos.len();
    let samples = PyList::empty(py);
    if nv != 0 {
        let mut picked: Vec<usize> = (0..32).map(|k| k * (nv - 1) / 31).collect();
        picked.sort_unstable();
        picked.dedup();
        for index in picked {
            samples.append((index, (pos[index][0], pos[index][1], pos[index][2])))?;
        }
    }
    // float(np.cumsum(x + y * 1.000003 + z * 1.000007)[-1]): added in order.
    let coordsum = pos.iter().fold(0f64, |sum, p| sum + ((p[0] + p[1] * 1.000003) + p[2] * 1.000007));
    let clear_textures = PyDict::new(py);
    for (name, entries) in by_clear_texture.iter() {
        let (opacity, cull) = (name.get_item(0)?, name.get_item(1)?);
        for (path, data) in entries.cast::<PyDict>()?.iter() {
            clear_textures.set_item((path, &opacity, &cull), data)?;
        }
    }
    let joined = |buckets: &Bound<'py, PyDict>| -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        for (key, parts) in buckets.iter() {
            out.set_item(key, bytes(b"").call_method1("join", (parts,))?)?;
        }
        Ok(out)
    };
    let picked = !v0.is_empty();
    chunk.set_item("fp", fingerprint)?;
    chunk.set_item("vkey", py.None())?;
    chunk.set_item("rev", 0)?;
    chunk.set_item("uid", py.None())?;
    chunk.set_item("msig", msig)?;
    chunk.set_item("serial", py.None())?;
    chunk.set_item("flags_gen", mesh.getattr_opt("_flags_gen")?.unwrap_or(0.into_bound_py_any(py)?))?;
    chunk.set_item("nv", nv)?;
    chunk.set_item("ne", edge_count)?;
    chunk.set_item("nf", nf)?;
    chunk.set_item("samples", samples)?;
    chunk.set_item("coordsum", coordsum)?;
    chunk.set_item("bbox", bbox)?;
    chunk.set_item("obb", py.None())?;
    chunk.set_item("edges", bytes(&edge_data))?;
    chunk.set_item("smooth_vcol", bytes(&smooth_vcol))?;
    chunk.set_item("vcol", bytes(&stream(0)))?;
    chunk.set_item("by_texture", by_texture)?;
    chunk.set_item("tcol", clear_colours)?;
    chunk.set_item("ttex", clear_textures)?;
    chunk.set_item("back_vcol", bytes(b"").call_method1("join", (back_vcol,))?)?;
    chunk.set_item("back_tex", joined(&back_tex)?)?;
    chunk.set_item("back_tcol", joined(&back_tcol)?)?;
    chunk.set_item("back_ttex", joined(&back_ttex)?)?;
    chunk.set_item("fvcol", bytes(&stream(1)))?;
    chunk.set_item("dback", bytes(&tint))?;
    chunk.set_item("faces", faces)?;
    chunk.set_item("areas", PyArray1::from_vec(py, areas))?;
    if picked {
        chunk.set_item("v0", triples(py, &v0)?)?;
        chunk.set_item("e1", triples(py, &e1)?)?;
        chunk.set_item("e2", triples(py, &e2)?)?;
        chunk.set_item("tri_ent", PyArray1::from_vec(py, owner))?;
    } else {
        for name in ["v0", "e1", "e2", "tri_ent"] {
            chunk.set_item(name, py.None())?;
        }
    }
    chunk.set_item("soft_pts", soft_points)?;
    chunk.set_item("soft_n0", soft_n0)?;
    chunk.set_item("soft_c0", soft_c0)?;
    chunk.set_item("soft_n1", soft_n1)?;
    chunk.set_item("soft_c1", soft_c1)?;
    chunk.set_item("soft_single", soft_single)?;
    chunk.set_item("edge_layer", layers_of(&hard)?)?;
    chunk.set_item("soft_layer", layers_of(&softs)?)?;
    chunk.set_item("mesh", mesh)?;
    Ok(chunk)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(bake_deferred_chunk, m)?)
}
