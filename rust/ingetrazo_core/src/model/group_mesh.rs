// SPDX-License-Identifier: GPL-3.0-or-later
//! Meshes made from placements (`core/group.py`): a mesh copied through a
//! matrix (`transformed_mesh`), a component's placements merged into one
//! (`world_mesh`), and the texture maps that travel with a face through a
//! matrix or a move (`transformed_attrs`, `translated_attrs`,
//! `placed_face_paint`, `remap_uvws`).
//!
//! A copy is one `MeshStore.from_block` over the stores' arrays: positions
//! mapped per mesh by NumPy's `pts @ rot.T + trans` (kept NumPy's, so the
//! bits are the ones the Python had), rings reversed through a mirror, the
//! edges' soft and flat flags and curves, attribute rows copied with their
//! texture maps re-fitted once per row, the planar projection a textured
//! face without a map shows pinned face by face.
//! `tests/test_transformed_mesh_golden.py`.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use super::module;

type Obj<'py> = Bound<'py, PyAny>;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn get<'py>(d: &Obj<'py>, key: &str) -> PyResult<Obj<'py>> {
    d.call_method1(pyo3::intern!(d.py(), "get"), (key,))
}

fn copy_dict<'py>(d: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(d.py());
    out.call_method1(pyo3::intern!(d.py(), "update"), (d,))?;
    Ok(out)
}

/// `np_affine(m)`: `(rot, trans)` NumPy arrays, `pts @ rot.T + trans` is
/// `m.map(p)` for every row (`data()` is column-major).
#[pyfunction]
#[pyo3(name = "group_np_affine")]
pub fn np_affine<'py>(m: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = m.py();
    let d: Vec<f64> = m.call_method0(pyo3::intern!(py, "data"))?.extract()?;
    let np = module(py, "numpy")?;
    let rot = np.call_method1("array", (vec![vec![d[0], d[4], d[8]], vec![d[1], d[5], d[9]], vec![d[2], d[6], d[10]]], "float64"))?;
    let trans = np.call_method1("array", (vec![d[12], d[13], d[14]], "float64"))?;
    Ok((rot, trans))
}

/// `_remapped_uvw(uvw, minv)`: the 8-number map re-fitted for geometry
/// mapped through the matrix whose inverse is `minv` (each row a
/// homogeneous covector: `row @ inverse`).
#[pyfunction]
#[pyo3(name = "group_remapped_uvw")]
pub fn remapped_uvw<'py>(uvw: &Obj<'py>, minv: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = uvw.py();
    let numbers = uvw.len()? == 8
        && uvw.try_iter()?.all(|x| x.is_ok_and(|x| x.is_exact_instance_of::<pyo3::types::PyFloat>() || x.is_exact_instance_of::<pyo3::types::PyInt>()));
    if minv.is_exact_instance_of::<crate::math::Matrix4>() && numbers {
        let modern = module(py, "sys")?.getattr("version_info")?.ge((3, 12))?;
        return minv.call_method1(pyo3::intern!(py, "remap_uvw"), (uvw, modern));
    }
    let d: Vec<f64> = minv.call_method0(pyo3::intern!(py, "data"))?.extract()?;
    let u: Vec<f64> = uvw.extract()?;
    let mut out = Vec::with_capacity(8);
    for base in [0usize, 4] {
        for col in 0..4 {
            out.push(super::pyfloat::sum((0..4).map(|k| u[base + k] * d[4 * col + k])));
        }
    }
    Ok(PyList::new(py, out)?.into_any())
}

/// The world→UV map a textured face without a fitted `uvw` is showing,
/// recovered at its positions before the transform (`minv` its inverse):
/// the planar projection of the face's normal there. None for a degenerate
/// or stale face.
/// What the planar projections ask once per build: the hypot convention,
/// `core.texture.face_uv_axes` and the vector class.
struct UvCx<'py> {
    hypot: Obj<'py>,
    axes: Obj<'py>,
    vec3: Obj<'py>,
}

impl<'py> UvCx<'py> {
    fn new(py: Python<'py>) -> PyResult<Self> {
        Ok(UvCx {
            hypot: module(py, "core.store_mesh")?.call_method0("_hypot")?,
            axes: module(py, "core.texture")?.getattr("face_uv_axes")?,
            vec3: module(py, "ingetrazo_core")?.getattr("Vec3")?,
        })
    }
}

fn implicit_uvw<'py>(cx: &UvCx<'py>, face: &Obj<'py>, tex: &Obj<'py>, minv: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = face.py();
    let st = face.getattr(pyo3::intern!(py, "_m"))?.getattr(pyo3::intern!(py, "_store"))?;
    let key = face.getattr(pyo3::intern!(py, "key"))?;
    if !st.call_method1(pyo3::intern!(py, "is_live"), (&key,))?.is_truthy()? {
        return Ok(None);
    }
    let c = st.call_method1(pyo3::intern!(py, "pretransform_face_normal"), (&key, minv.call_method0(pyo3::intern!(py, "data"))?, &cx.hypot))?;
    if c.is_none() {
        return Ok(None);
    }
    let c: Vec<f64> = c.extract()?;
    let normal = cx.vec3.call1((c[0], c[1], c[2]))?;
    uv_axes(py, &cx.axes, tex, &normal).map(Some)
}

/// `[*gu, cu, *gv, cv]` of `core.texture.face_uv_axes(tex, normal)`.
fn uv_axes<'py>(py: Python<'py>, face_uv_axes: &Obj<'py>, tex: &Obj<'py>, normal: &Obj<'py>) -> PyResult<Obj<'py>> {
    let axes = face_uv_axes.call1((tex, normal))?;
    let (gu, cu, gv, cv) = (axes.get_item(0)?, axes.get_item(1)?, axes.get_item(2)?, axes.get_item(3)?);
    let xyz = |v: &Obj<'py>| -> PyResult<[f64; 3]> {
        Ok([v.call_method0("x")?.extract()?, v.call_method0("y")?.extract()?, v.call_method0("z")?.extract()?])
    };
    let (u, v) = (xyz(&gu)?, xyz(&gv)?);
    let out = vec![u[0], u[1], u[2], cu.extract::<f64>()?, v[0], v[1], v[2], cv.extract::<f64>()?];
    Ok(PyList::new(py, out)?.into_any())
}

fn is_map(uvw: &Obj<'_>) -> PyResult<bool> {
    Ok(uvw.is_truthy()? && uvw.len()? == 8)
}

/// `_remap_uvws(mesh, m, faces=None)`: each face's `uvw` re-fitted so that
/// at the transformed positions it gives the prototype's UVs; a face still
/// on the implicit planar projection gets that projection pinned first.
#[pyfunction]
#[pyo3(name = "group_remap_uvws", signature = (mesh, m, faces=None))]
pub fn remap_uvws<'py>(mesh: &Obj<'py>, m: &Obj<'py>, faces: Option<&Obj<'py>>) -> PyResult<()> {
    let py = mesh.py();
    let (minv, ok): (Obj<'py>, bool) = m.call_method0("inverted")?.extract()?;
    if !ok {
        return Ok(());
    }
    let faces = match faces.filter(|f| !f.is_none()) {
        Some(f) => f.clone(),
        None => mesh.getattr("faces")?,
    };
    let cx = UvCx::new(py)?;
    for f in faces.try_iter()? {
        let f = f?;
        let attrs = f.getattr(pyo3::intern!(py, "attrs"))?;
        let t = if attrs.is_truthy()? { get(&attrs, "texture")? } else { none(py) };
        let mut uvw = if t.is_truthy()? { get(&t, "uvw")? } else { none(py) };
        if !is_map(&uvw)? {
            match if t.is_truthy()? { implicit_uvw(&cx, &f, &t, &minv)? } else { None } {
                Some(u) => uvw = u,
                None => continue,
            }
        }
        let tex = copy_dict(&t)?;
        tex.set_item("uvw", remapped_uvw(&uvw, &minv)?)?;
        attrs.set_item("texture", tex)?;
    }
    Ok(())
}

/// `transformed_attrs(attrs, m)`: a copy of face `attrs` with the
/// texture's `uvw` re-fitted for geometry mapped through `m`.
#[pyfunction]
#[pyo3(name = "group_transformed_attrs")]
pub fn transformed_attrs<'py>(attrs: &Obj<'py>, m: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = attrs.py();
    let out = if attrs.is_truthy()? { copy_dict(attrs)? } else { PyDict::new(py) };
    let t = get(out.as_any(), "texture")?;
    let uvw = if t.is_truthy()? { get(&t, "uvw")? } else { none(py) };
    if !is_map(&uvw)? {
        return Ok(out);
    }
    let (minv, ok): (Obj<'py>, bool) = m.call_method0("inverted")?.extract()?;
    if !ok {
        return Ok(out);
    }
    let tex = copy_dict(&t)?;
    tex.set_item("uvw", remapped_uvw(&uvw, &minv)?)?;
    out.set_item("texture", tex)?;
    Ok(out)
}

/// `translated_attrs(attrs, delta)`: a copy of face `attrs` whose texture
/// map's constant term follows a move by `delta`.
#[pyfunction]
#[pyo3(name = "group_translated_attrs")]
pub fn translated_attrs<'py>(attrs: &Obj<'py>, delta: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = attrs.py();
    let out = if attrs.is_truthy()? { copy_dict(attrs)? } else { PyDict::new(py) };
    let t = get(out.as_any(), "texture")?;
    let uvw = if t.is_truthy()? { get(&t, "uvw")? } else { none(py) };
    if is_map(&uvw)? {
        let u: Vec<Obj<'py>> = uvw.try_iter()?.collect::<PyResult<_>>()?;
        let mut new: Vec<Obj<'py>> = u.clone();
        let (dx, dy, dz) = (delta.call_method0("x")?, delta.call_method0("y")?, delta.call_method0("z")?);
        for base in [0usize, 4] {
            // c - (gx * dx + gy * dy + gz * dz), with Python's operators.
            let s = u[base].mul(&dx)?.add(u[base + 1].mul(&dy)?)?.add(u[base + 2].mul(&dz)?)?;
            new[base + 3] = u[base + 3].sub(s)?;
        }
        let tex = copy_dict(&t)?;
        tex.set_item("uvw", PyList::new(py, new)?)?;
        out.set_item("texture", tex)?;
    }
    Ok(out)
}

/// `placed_face_paint(attrs, face, inverse)`: both sides' effective paint
/// pinned to the prototype before exploding.
#[pyfunction]
#[pyo3(name = "group_placed_face_paint")]
pub fn placed_face_paint<'py>(attrs: &Obj<'py>, face: &Obj<'py>, inverse: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = attrs.py();
    let placed = |side: &Obj<'py>| -> PyResult<Obj<'py>> {
        let mut tex = get(side, "texture")?;
        if !tex.is_truthy()? {
            let pbr = get(side, "pbr")?;
            let maps = pbr.is_truthy()? && pbr.try_iter()?.any(|k| k.is_ok_and(|k| k.call_method1("endswith", ("_map",)).is_ok_and(|v| v.is_truthy().unwrap_or(false))));
            if !maps {
                return Ok(side.clone());
            }
            tex = PyDict::new(py).into_any(); // PBR-only maps share a one-metre planar tile.
        }
        let mut uvw = get(&tex, "uvw")?;
        if !uvw.is_truthy()? {
            uvw = uv_axes(py, &module(py, "core.texture")?.getattr("face_uv_axes")?, &tex, &face.call_method0("normal")?)?;
        }
        let t = copy_dict(&tex)?;
        t.set_item("uvw", remapped_uvw(&uvw, inverse)?)?;
        let out = copy_dict(side)?;
        out.set_item("texture", t)?;
        Ok(out.into_any())
    };
    let mut out = placed(attrs)?;
    let back = get(attrs, "back")?;
    if back.is_instance_of::<PyDict>() {
        let o = copy_dict(&out)?;
        o.set_item("back", placed(&back)?)?;
        out = o.into_any();
    }
    Ok(out)
}

/// A face attribute row copied, its `uvw` re-fitted for the matrix whose
/// inverse is `minv` (None: copied as it is).
fn placed_row<'py>(attrs: &Obj<'py>, minv: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let out = copy_dict(attrs)?;
    let t = get(out.as_any(), "texture")?;
    if let Some(minv) = minv {
        if t.is_instance_of::<PyDict>() {
            let uvw = get(&t, "uvw")?;
            if is_map(&uvw)? {
                let tex = copy_dict(&t)?;
                tex.set_item("uvw", remapped_uvw(&uvw, minv)?)?;
                out.set_item("texture", tex)?;
            }
        }
    }
    Ok(out.into_any())
}

/// An int64 array's values (any shape, in order), read in place.
fn ints(a: &Obj<'_>) -> PyResult<Vec<i64>> {
    use numpy::{PyArrayDyn, PyArrayMethods};
    match a.cast::<PyArrayDyn<i64>>() {
        Ok(arr) => Ok(arr.readonly().as_array().iter().copied().collect()),
        Err(_) => a.call_method0("ravel")?.call_method1("astype", ("int64",))?.call_method0("tolist")?.extract(),
    }
}

fn i64_array<'py>(py: Python<'py>, v: Vec<i64>) -> Obj<'py> {
    numpy::PyArray1::from_vec(py, v).into_any()
}

/// The geometry of every `(mesh or placement, matrix)` of `parts` in one
/// new store-backed mesh (see the module notes). `uv`: `"fitted"`
/// re-fits only the faces that have a map; `"all"` also pins the planar
/// projection a textured face without one shows.
fn store_built<'py>(py: Python<'py>, parts: &[(Obj<'py>, Obj<'py>)], all_maps: bool) -> PyResult<Obj<'py>> {
    let np = module(py, "numpy")?;
    // Arrays stay NumPy's (offsets, masks, concatenation in C); only the
    // mirror's ring reversal, faces' own dicts and the planar maps to pin
    // are read here.
    let (pos_parts, edge_parts, flag_parts, curve_parts) = (PyList::empty(py), PyList::empty(py), PyList::empty(py), PyList::empty(py));
    let (corner_parts, size_parts, count_parts, fa_parts) = (PyList::empty(py), PyList::empty(py), PyList::empty(py), PyList::empty(py));
    let table = PyList::empty(py);
    let mut implicit: Vec<(usize, Obj<'py>)> = Vec::new();
    let (mut v_off, mut c_off, mut f_off) = (0i64, 0i64, 0usize);
    for (g, m) in parts {
        let src = match g.getattr_opt(pyo3::intern!(py, "mesh"))? {
            Some(mesh) if !mesh.is_none() => mesh,
            _ => g.clone(),
        };
        let st = src.getattr(pyo3::intern!(py, "_store"))?;
        let a = st.call_method0(pyo3::intern!(py, "arrays"))?;
        let mut pos = a.get_item(pyo3::intern!(py, "pos"))?;
        let transformed = !m.is_none();
        if transformed {
            let (rot, trans) = np_affine(m)?;
            pos = pos.call_method1(pyo3::intern!(py, "__matmul__"), (rot.getattr(pyo3::intern!(py, "T"))?,))?.call_method1(pyo3::intern!(py, "__add__"), (trans,))?;
        }
        let n_pos = pos.len()? as i64;
        let mut corners = a.get_item(pyo3::intern!(py, "corners"))?;
        let sizes = a.get_item(pyo3::intern!(py, "ring_sizes"))?;
        if transformed && m.call_method0(pyo3::intern!(py, "determinant"))?.extract::<f64>()? < 0.0 {
            let mut c = ints(&corners)?;
            let mut at = 0usize;
            for n in ints(&sizes)? {
                c[at..at + n as usize].reverse();
                at += n as usize;
            }
            corners = i64_array(py, c);
        }
        let curves = a.get_item(pyo3::intern!(py, "curve"))?;
        let live = curves.call_method1(pyo3::intern!(py, "__ge__"), (0,))?;
        let shifted = np.call_method1(pyo3::intern!(py, "where"), (&live, curves.call_method1(pyo3::intern!(py, "__add__"), (c_off,))?, -1))?;
        if live.call_method0(pyo3::intern!(py, "any"))?.is_truthy()? {
            c_off = shifted.call_method0(pyo3::intern!(py, "max"))?.extract::<i64>()? + 1;
        }
        pos_parts.append(pos)?;
        edge_parts.append(a.get_item(pyo3::intern!(py, "edges"))?.call_method1(pyo3::intern!(py, "__add__"), (v_off,))?)?;
        // Soft (1) and flat (4).
        flag_parts.append(np.call_method1(pyo3::intern!(py, "bitwise_and"), (a.get_item(pyo3::intern!(py, "flags"))?, 5u8))?)?;
        curve_parts.append(shifted)?;
        corner_parts.append(corners.call_method1(pyo3::intern!(py, "__add__"), (v_off,))?)?;
        size_parts.append(sizes)?;
        count_parts.append(a.get_item(pyo3::intern!(py, "ring_counts"))?)?;
        // Attribute rows: the source's, re-fitted once per row; a face's
        // own dict a row of its own.
        let d = src.getattr(pyo3::intern!(py, "__dict__"))?;
        let src_table = d.call_method1(pyo3::intern!(py, "get"), ("_attr_table",))?;
        let own = d.call_method1(pyo3::intern!(py, "get"), ("_attr_own",))?;
        let rows = a.get_item(pyo3::intern!(py, "face_attr"))?;
        let n_faces = rows.len()?;
        let inverse = if transformed {
            let (minv, ok): (Obj<'py>, bool) = m.call_method0(pyo3::intern!(py, "inverted"))?.extract()?;
            ok.then_some(minv)
        } else {
            None
        };
        let base = table.len() as i64;
        if src_table.is_truthy()? {
            for row in src_table.try_iter()? {
                table.append(placed_row(&row?, inverse.as_ref())?)?;
            }
        }
        let mut fa = np.call_method1(pyo3::intern!(py, "where"), (rows.call_method1(pyo3::intern!(py, "__ge__"), (0,))?, rows.call_method1(pyo3::intern!(py, "__add__"), (base,))?, -1))?;
        if own.is_truthy()? {
            // The faces with a dict of their own, in face order.
            let keys: Vec<u128> = st.call_method1(pyo3::intern!(py, "keys"), ("face",))?.extract()?;
            let index: std::collections::HashMap<u128, usize> = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
            let mut owned: Vec<(usize, Obj<'py>)> = Vec::new();
            for (k, attrs) in own.cast::<PyDict>()?.iter() {
                if let Some(&i) = k.extract::<u128>().ok().and_then(|k| index.get(&k)) {
                    owned.push((i, attrs));
                }
            }
            owned.sort_by_key(|(i, _)| *i);
            let mut v = ints(&fa)?;
            for (i, attrs) in owned {
                if attrs.is_none() {
                    continue;
                }
                if attrs.is_truthy()? {
                    v[i] = table.len() as i64;
                    table.append(placed_row(&attrs, inverse.as_ref())?)?;
                } else {
                    v[i] = -1;
                }
            }
            fa = i64_array(py, v);
        }
        if all_maps {
            if let Some(minv) = &inverse {
                // The rows that show a planar projection, then their faces.
                let mut pinned: Vec<bool> = Vec::with_capacity(table.len());
                for row in table.iter() {
                    let t = get(&row, "texture")?;
                    pinned.push(t.is_truthy()? && !is_map(&get(&t, "uvw")?)?);
                }
                if pinned.iter().any(|&p| p) {
                    for (i, r) in ints(&fa)?.into_iter().enumerate() {
                        if r >= 0 && pinned[r as usize] {
                            implicit.push((f_off + i, minv.clone()));
                        }
                    }
                }
            }
        }
        f_off += n_faces;
        fa_parts.append(fa)?;
        v_off += n_pos;
    }
    let mesh = module(py, "core.mesh")?.getattr(pyo3::intern!(py, "Mesh"))?.call0()?;
    if v_off == 0 {
        return Ok(mesh);
    }
    let cat = |l: &Bound<'py, PyList>| -> PyResult<Obj<'py>> { np.call_method1(pyo3::intern!(py, "ascontiguousarray"), (np.call_method1(pyo3::intern!(py, "concatenate"), (l,))?,)) };
    let kw = module(py, "core.store_mesh")?.call_method0(pyo3::intern!(py, "_weld_args"))?.cast_into::<PyDict>()?;
    kw.set_item("sequential", false)?;
    let namespace = module(py, "core.mesh")?.getattr(pyo3::intern!(py, "_KEY_NAMESPACES"))?.call_method0(pyo3::intern!(py, "__next__"))?;
    let positions = np.call_method1(pyo3::intern!(py, "ascontiguousarray"), (np.call_method1(pyo3::intern!(py, "concatenate"), (&pos_parts,))?, "float64"))?;
    let args = PyTuple::new(py, [
        namespace,
        positions,
        cat(&edge_parts)?,
        cat(&flag_parts)?,
        cat(&curve_parts)?,
        none(py),
        PyList::empty(py).into_any(),
        cat(&corner_parts)?,
        cat(&size_parts)?,
        cat(&count_parts)?,
        cat(&fa_parts)?,
    ])?;
    let store = module(py, "ingetrazo_core")?.getattr(pyo3::intern!(py, "MeshStore"))?.call_method("from_block", args, Some(&kw))?;
    module(py, "ingetrazo_core")?.getattr(pyo3::intern!(py, "store_views"))?.call_method1(pyo3::intern!(py, "become_store"), (&mesh, store, table))?;
    mesh.call_method0(pyo3::intern!(py, "resplit_curves"))?;
    if !implicit.is_empty() {
        let cx = UvCx::new(py)?;
        let faces = mesh.getattr(pyo3::intern!(py, "faces"))?;
        for (i, minv) in implicit {
            let f = faces.get_item(i)?;
            let attrs = f.getattr(pyo3::intern!(py, "attrs"))?;
            let t = get(&attrs, "texture")?;
            if let Some(uvw) = implicit_uvw(&cx, &f, &t, &minv)? {
                let tex = copy_dict(&t)?;
                tex.set_item("uvw", remapped_uvw(&uvw, &minv)?)?;
                attrs.set_item("texture", tex)?;
            }
        }
    }
    Ok(mesh)
}

/// `transformed_mesh(src, m)`: a deep copy of `src` with every position
/// mapped through `m`, its texture maps re-fitted (see the module notes).
#[pyfunction]
#[pyo3(name = "group_transformed_mesh")]
pub fn transformed_mesh<'py>(src: &Obj<'py>, m: &Obj<'py>) -> PyResult<Obj<'py>> {
    store_built(src.py(), &[(src.clone(), m.clone())], true)
}

/// `world_mesh(group, placements=None)`: a world-space mesh for a group:
/// the mesh itself for a classic group, a transformed copy for an
/// instance, and with nested placements every placement (or those
/// `placements` yields) merged into one mesh.
#[pyfunction]
#[pyo3(name = "group_world_mesh", signature = (group, placements=None))]
pub fn world_mesh<'py>(group: &Obj<'py>, placements: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let py = group.py();
    let m = group.getattr_opt("xform")?.unwrap_or_else(|| none(py));
    let kids = group.getattr_opt("children")?;
    if !kids.as_ref().is_some_and(|k| k.is_truthy().unwrap_or(false)) {
        let mesh = group.getattr("mesh")?;
        return if m.is_none() { Ok(mesh) } else { transformed_mesh(&mesh, &m) };
    }
    let walk = match placements.filter(|p| !p.is_none()) {
        Some(p) => p.clone(),
        None => module(py, "ingetrazo_core")?.call_method1("group_iter_placements", (group,))?,
    };
    let mut parts = Vec::new();
    for pair in walk.try_iter()? {
        let pair = pair?;
        parts.push((pair.get_item(0)?, pair.get_item(1)?));
    }
    store_built(py, &parts, false)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(np_affine, m)?)?;
    m.add_function(wrap_pyfunction!(remapped_uvw, m)?)?;
    m.add_function(wrap_pyfunction!(remap_uvws, m)?)?;
    m.add_function(wrap_pyfunction!(transformed_attrs, m)?)?;
    m.add_function(wrap_pyfunction!(translated_attrs, m)?)?;
    m.add_function(wrap_pyfunction!(placed_face_paint, m)?)?;
    m.add_function(wrap_pyfunction!(transformed_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(world_mesh, m)?)?;
    Ok(())
}
