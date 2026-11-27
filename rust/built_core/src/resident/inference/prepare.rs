// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture prototype geometry under the document lease. The BVH build consumes
//! array snapshots after releasing the document.
use super::lease::SceneLease;
use numpy::{ndarray::Array2, IntoPyArray, PyReadonlyArray2, PyReadonlyArray3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySlice, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
fn optional<'p>(v: Option<Obj<'p>>) -> Option<Obj<'p>> {
    v.filter(|v| !v.is_none())
}
fn slice(py: Python<'_>, a: usize, n: usize) -> Bound<'_, PySlice> {
    PySlice::new(py, a as isize, (a + n) as isize, 1)
}
fn store_source<'p>(
    packed: &Obj<'p>,
    mesh: &Obj<'p>,
    store: &Obj<'p>,
    code: usize,
) -> PyResult<(Obj<'p>, [f64; 3])> {
    let py = packed.py();
    let saved = match packed.getattr_opt("_snap_sources")? {
        Some(cache) => optional(Some(cache.call_method1("get", (mesh.as_ptr() as usize,))?)),
        None => None,
    };
    let (snap, frame) = match saved {
        Some(pair) => (pair.get_item(0)?, pair.get_item(1)?),
        None => (
            store.call_method0("snapshot")?,
            store.getattr("frame_origin")?,
        ),
    };
    let native = snap
        .cast::<crate::store::py::PyStoreSnapshot>()
        .ok()
        .and_then(|s| s.get().inner.state.exact.native.clone());
    let arrays = if let Some(patch) = native {
        super::exact::remember(packed, code, patch)?;
        snap.call_method0("display_arrays")?
    } else {
        super::exact::forget(packed, code)?;
        // Positions and edges only: the faces are the bake's, not read here.
        let kw = PyDict::new(py);
        kw.set_item("incidence", false)?;
        kw.set_item("faces", false)?;
        snap.call_method("arrays", (), Some(&kw))?
    };
    let source = PyDict::new(py);
    for (name, col) in [
        ("pos", "pos"),
        ("edges", "edges"),
        ("flags", "flags"),
        ("curves", "curve"),
        ("layers", "layer"),
    ] {
        source.set_item(name, arrays.get_item(col)?)?;
    }
    let meta = PyDict::new(py);
    meta.set_item("edge_layers", snap.call_method0("layer_names")?)?;
    source.set_item("meta", meta)?;
    Ok((source.into_any(), frame.extract()?))
}
fn source_columns<'p>(
    source: &Obj<'p>,
    np: &Obj<'p>,
    scene: &Obj<'p>,
    grouped: bool,
    reveal: bool,
) -> PyResult<[Obj<'p>; 4]> {
    let py = np.py();
    let pos = np.call_method1("ascontiguousarray", (source.get_item("pos")?, "float64"))?;
    let edges = np.call_method1("ascontiguousarray", (source.get_item("edges")?, "int64"))?;
    let n = edges.len()?;
    let mut keep = np.call_method1("ones", (n, "bool"))?;
    if let (Some(layers), Some(visible), Some(meta)) = (
        optional(Some(source.call_method1("get", ("layers",))?)),
        optional(scene.getattr_opt("entity_visible")?),
        optional(Some(source.call_method1("get", ("meta",))?)),
    ) {
        let names = meta.call_method1("get", ("edge_layers",))?;
        if names.is_truthy()? {
            let ns = py.import("types")?.getattr("SimpleNamespace")?;
            let mut allowed = Vec::new();
            for name in names.try_iter()? {
                let kw = PyDict::new(py);
                kw.set_item("layer", name?)?;
                allowed.push(visible.call1((ns.call((), Some(&kw))?,))?.is_truthy()?);
            }
            allowed.push(true);
            let ids = np.call_method1("maximum", (layers, -1))?;
            keep = allowed.into_pyarray(py).get_item(ids)?;
        }
    }
    let flags = source.get_item("flags")?;
    if !reveal && !flags.is_none() {
        let bits = if grouped { 3 } else { 2 };
        let mask = np.call_method1("bitwise_and", (flags, bits))?;
        keep = np.call_method1("logical_and", (keep, np.call_method1("equal", (mask, 0))?))?;
    }
    let curves = source.call_method1("get", ("curves",))?;
    let curves = if curves.is_none() {
        np.call_method1("full", (n, -1, "int64"))?
    } else {
        np.call_method1("ascontiguousarray", (curves, "int64"))?
    };
    Ok([pos, edges, keep, curves])
}
fn line_columns<'p>(
    np: &Obj<'p>,
    packed_lines: &Obj<'p>,
    mesh: Option<&Obj<'p>>,
    scene: &Obj<'p>,
    grouped: bool,
    reveal: bool,
) -> PyResult<[Obj<'p>; 4]> {
    let py = np.py();
    let mut ids = Vec::new();
    let lines = if let Some(mesh) = mesh {
        let lines = PyList::empty(py);
        let visible = optional(scene.getattr_opt("edge_snappable")?);
        for edge in mesh.getattr("edges")?.try_iter()? {
            let e = edge?;
            if grouped && !reveal && e.getattr("soft")?.is_truthy()? {
                continue;
            }
            let shown = match &visible {
                Some(f) => f.call1((&e,))?.is_truthy()?,
                None => reveal || !e.getattr("hidden")?.is_truthy()?,
            };
            if !shown {
                continue;
            }
            for end in ["v0", "v1"] {
                lines.append(e.getattr(end)?.call_method0("position_tuple")?)?;
            }
            ids.push(e.getattr("curve")?.extract::<Option<i64>>()?.unwrap_or(-1));
        }
        lines.into_any()
    } else {
        packed_lines.clone()
    };
    let pos = np
        .call_method1("ascontiguousarray", (lines, "float64"))?
        .call_method1("reshape", (-1, 3))?;
    let edges = np
        .call_method1("arange", (pos.len()?,))?
        .call_method1("reshape", (-1, 2))?;
    let n = edges.len()?;
    let keep = np.call_method1("ones", (n, "bool"))?;
    if mesh.is_none() {
        ids.resize(n, -1);
    }
    Ok([pos, edges, keep, ids.into_pyarray(py).into_any()])
}
fn prototype_arrays<'p>(
    packed: &Obj<'p>,
    np: &Obj<'p>,
    code: usize,
    scene: &Obj<'p>,
    row: Option<usize>,
    cache: &Bound<'p, PyDict>,
) -> PyResult<Obj<'p>> {
    let py = packed.py();
    let cmd = packed.getattr("commands")?.get_item(code)?;
    let start = cmd.get_item("base")?.extract()?;
    let count = cmd.get_item("count")?.extract()?;
    let vertices = packed.getattr("vertices")?.get_item("pos")?;
    let triangles = np.call_method1(
        "ascontiguousarray",
        (vertices.get_item(slice(py, start, count))?,),
    )?;
    let group = match (optional(packed.getattr_opt("groups")?), row) {
        (Some(g), Some(i)) => optional(Some(g.get_item(i)?)),
        _ => None,
    };
    let grouped = group.is_some();
    let mesh = match group {
        Some(g) => Some(g.getattr("mesh")?),
        None => optional(scene.getattr_opt("loose_mesh")?),
    };
    let key = (mesh.as_ref().map(|m| m.as_ptr() as usize), grouped);
    let columns = if let Some(hit) = cache.get_item(key)? {
        hit.extract::<[Obj<'p>; 4]>()?
    } else {
        let mut frame = [0.; 3];
        let mut source = None;
        if let Some(m) = &mesh {
            let d = m.getattr("__dict__")?;
            source = optional(Some(d.call_method1("get", ("_deferred_source",))?));
            if source.is_none() {
                if let Some(st) = optional(Some(d.call_method1("get", ("_store",))?)) {
                    let (s, f) = store_source(packed, m, &st, code)?;
                    source = Some(s);
                    frame = f;
                }
            }
        }
        let reveal = scene
            .getattr_opt("show_hidden_geometry")?
            .map(|v| v.is_truthy())
            .transpose()?
            .unwrap_or(false);
        let mut cols = match source {
            Some(s) => source_columns(&s, np, scene, grouped, reveal)?,
            None => {
                let n = packed
                    .getattr("edge_commands")?
                    .get_item(code)?
                    .get_item("count")?
                    .extract()?;
                line_columns(
                    np,
                    &vertices.get_item(slice(py, start + count, n))?,
                    mesh.as_ref(),
                    scene,
                    grouped,
                    reveal,
                )?
            }
        };
        if mesh.is_some() {
            let origins = packed.getattr("prototype_origins")?;
            if code < origins.len()? {
                let origin = origins.get_item(code)?.extract::<[f64; 3]>()?;
                if frame != origin {
                    let delta: [f64; 3] = std::array::from_fn(|i| frame[i] - origin[i]);
                    cols[0] = cols[0].add(delta)?;
                }
            }
            cache.set_item(key, PyTuple::new(py, &cols)?)?;
        }
        cols
    };
    let [pos, edges, keep, curves] = columns;
    Ok((triangles, pos, edges, keep, curves)
        .into_pyobject(py)?
        .into_any())
}
struct Placements<'p> {
    ids: Obj<'p>,
    mats: Obj<'p>,
    center: Obj<'p>,
    extent: Obj<'p>,
}
impl<'p> Placements<'p> {
    fn read(packed: &Obj<'p>) -> PyResult<Self> {
        let py = packed.py();
        let instances = packed.getattr("instances")?;
        let info = instances.get_item("info")?;
        let ids = info
            .get_item((PySlice::full(py), 0))?
            .call_method1("astype", ("int64",))?;
        let mats = instances
            .get_item("world")?
            .call_method1("astype", ("float64",))?
            .call_method1("reshape", (-1, 16))?;
        let bb = packed
            .getattr("bounds")?
            .get_item(&ids)?
            .call_method1("astype", ("float64",))?;
        let boxes = bb.extract::<PyReadonlyArray3<f64>>()?;
        let boxes = boxes.as_array();
        let mat = mats.extract::<PyReadonlyArray2<f64>>()?;
        let mat = mat.as_array();
        if boxes.dim().0 != mat.nrows() || boxes.dim().1 != 2 || boxes.dim().2 < 3 {
            return Err(PyValueError::new_err(
                "expected two prototype bounds per placement",
            ));
        }
        let mut c = Array2::<f64>::zeros((mat.nrows(), 3));
        let mut e = c.clone();
        for i in 0..mat.nrows() {
            for j in 0..3 {
                for k in 0..3 {
                    let v = mat[[i, k * 4 + j]];
                    let lo = boxes[[i, 0, k]];
                    let hi = boxes[[i, 1, k]];
                    c[[i, j]] += v * ((hi + lo) * 0.5);
                    e[[i, j]] += v.abs() * ((hi - lo) * 0.5);
                }
                c[[i, j]] += mat[[i, 12 + j]];
            }
        }
        Ok(Self {
            ids,
            mats,
            center: c.into_pyarray(py).into_any(),
            extent: e.into_pyarray(py).into_any(),
        })
    }
    fn build(&self, native: &Obj<'p>, op: &str, geometry: &Obj<'p>) -> PyResult<Obj<'p>> {
        native.call_method1(
            op,
            (
                geometry,
                &self.ids,
                &self.mats,
                self.center.sub(&self.extent)?,
                self.center.add(&self.extent)?,
            ),
        )
    }
    fn index(
        &self,
        base: &Obj<'p>,
        protos: &Obj<'p>,
        packed: &Obj<'p>,
        scene: &Obj<'p>,
    ) -> PyResult<Obj<'p>> {
        let result = packed
            .py()
            .get_type::<super::index::ResidentInference>()
            .call1((
                base,
                protos,
                packed.py().get_type::<PyList>().call1((protos,))?,
                packed,
                scene,
            ))?;
        result.setattr("world_box_cache", (&self.center, &self.extent))?;
        Ok(result)
    }
}
#[pyfunction]
fn resident_rebase<'p>(index: &Obj<'p>, packed: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = packed.py();
    let p = Placements::read(packed)?;
    if let Some(exact) = index.getattr("packed")?.getattr_opt("_exact_prototypes")? {
        packed.setattr("_exact_prototypes", exact.call_method0("copy")?)?;
    }
    let protos = index.getattr("protos")?;
    let base = p.build(
        py.import("built_core")?.as_any(),
        "InstanceSet",
        &protos,
    )?;
    p.index(&base, &protos, packed, &index.getattr("scene")?)
}
/// Past this many triangles a rebaked prototype's snap trees are carried
/// over from its previous ones (`patch_snap_proto`) rather than rebuilt.
const CARRY_PAST: usize = 20_000;

/// For an edited store, take the triangle stream and hard segments straight
/// from their owners. Intermediate dense vertex, edge-index, curve and mask
/// arrays would all be discarded as soon as the replacement BVH is ready.
fn patch_store<'p>(
    packed: &Obj<'p>,
    code: usize,
    scene: &Obj<'p>,
    row: isize,
    old: &crate::InstanceProto,
) -> PyResult<Option<Obj<'p>>> {
    use crate::store::{keys::Kind, py::PyMeshStore};
    let py = packed.py();
    if packed.hasattr("_snap_sources")? || row < 0 {
        return Ok(None);
    }
    let Some(groups) = optional(packed.getattr_opt("groups")?) else {
        return Ok(None);
    };
    let group = groups.get_item(row)?;
    let mesh = if group.is_none() {
        scene.getattr_opt("loose_mesh")?
    } else {
        group.getattr_opt("mesh")?
    };
    let Some(mesh) = mesh else { return Ok(None) };
    let own = mesh.getattr("__dict__")?;
    if !own.call_method1("get", ("_deferred_source",))?.is_none() {
        return Ok(None);
    }
    let store = own.call_method1("get", ("_store",))?;
    if !store.is_exact_instance_of::<PyMeshStore>() {
        return Ok(None);
    }
    let borrowed = store.cast::<PyMeshStore>()?.borrow();
    let st = &borrowed.inner.state;
    if st.exact.native.is_some() || !st.layers.is_empty() {
        return Ok(None);
    }
    let reveal = scene
        .getattr_opt("show_hidden_geometry")?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(false);
    let mask = if reveal {
        0
    } else if group.is_none() {
        2
    } else {
        3
    };
    let origins = packed.getattr("prototype_origins")?;
    let origin = if code < origins.len()? {
        origins.get_item(code)?.extract::<[f64; 3]>()?
    } else {
        [0.; 3]
    };
    let cmd = packed.getattr("commands")?.get_item(code)?;
    let start = cmd.get_item("base")?.extract()?;
    let count: usize = cmd.get_item("count")?.extract()?;
    if count % 3 != 0 {
        return Ok(None);
    }
    let position: PyReadonlyArray2<f32> = packed
        .getattr("vertices")?
        .get_item("pos")?
        .get_item(slice(py, start, count))?
        .extract()?;
    let positions = position.as_array();
    let triangle = |t: usize| {
        let a = std::array::from_fn(|j| positions[[3 * t, j]] as f64);
        let b = std::array::from_fn(|j| positions[[3 * t + 1, j]] as f64 - a[j]);
        let c = std::array::from_fn(|j| positions[[3 * t + 2, j]] as f64 - a[j]);
        [a, b, c]
    };
    let slots = borrowed.inner.live_slots(Kind::Edge);
    let segment = |e: u32| {
        st.ends.get(e as usize).map(|i| {
            let p = *st.vpos.get(i as usize);
            if st.frame.origin == origin {
                p
            } else {
                std::array::from_fn(|j| p[j] + (st.frame.origin[j] - origin[j]))
            }
        })
    };
    let patch = packed
        .getattr_opt("_snap_edit")?
        .and_then(|v| v.cast_into::<crate::resident::plain_edit::SnapPatch>().ok());
    let patch = patch.as_ref().map(|p| p.get()).filter(|p| {
        p.code == code
            && p.revision == borrowed.inner.revision
            && p.lineage == borrowed.inner.lineage
            && p.anchor == origin
            && p.faces == count / 3
            && p.edges == slots.len()
            && st.ecurve.iter().all(|&v| v < 0)
    });
    let source = patch.map(|p| p.after);
    let inner = if let Some(p) = patch.filter(|p| old.source.is_some() && old.source == p.before) {
        old.inner.patch_spans(p, triangle, |i| segment(slots[i]))
    } else {
        let (mut a, mut b, mut c) = (
            Vec::with_capacity(count / 3),
            Vec::with_capacity(count / 3),
            Vec::with_capacity(count / 3),
        );
        let mut edges = Vec::with_capacity(slots.len());
        for e in slots {
            if st.eflags.get(e as usize) & mask != 0 {
                continue;
            }
            if *st.ecurve.get(e as usize) >= 0 {
                return Ok(None);
            }
            edges.push(segment(e));
        }
        for t in 0..count / 3 {
            let [x, y, z] = triangle(t);
            a.push(x);
            b.push(y);
            c.push(z);
        }
        py.detach(|| old.inner.patched(a, b, c, edges))
    };
    super::exact::forget(packed, code)?;
    let proto = Bound::new(
        py,
        crate::InstanceProto {
            source,
            inner: std::sync::Arc::new(inner),
            curves: Vec::new(),
        },
    )?;
    Ok(Some((&proto, &proto).into_pyobject(py)?.into_any()))
}

#[pyfunction]
#[pyo3(signature = (packed, code, scene, first_row, previous=None))]
pub(super) fn resident_prototype<'p>(
    packed: &Obj<'p>,
    code: usize,
    scene: &Obj<'p>,
    first_row: isize,
    previous: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = packed.py();
    if let Some(old) = previous.and_then(|p| p.cast::<crate::InstanceProto>().ok()) {
        let old = old.borrow();
        if old.inner.n_tris() >= CARRY_PAST {
            if let Some(patched) = patch_store(packed, code, scene, first_row, &old)? { return Ok(patched); }
        }
    }
    let np = py.import("numpy")?;
    let array = prototype_arrays(
        packed,
        np.as_any(),
        code,
        scene,
        usize::try_from(first_row).ok(),
        &PyDict::new(py),
    )?;
    let core = py.import("built_core")?;
    if let Some(old) = previous.filter(|p| p.is_instance(&core.getattr("InstanceProto").unwrap()).unwrap_or(false)) {
        if old.len()? >= CARRY_PAST {
            let proto = core.call_method1("patch_snap_proto", (old, array))?;
            return Ok((&proto, &proto).into_pyobject(py)?.into_any());
        }
    }
    let built = core.call_method1(
        "build_snap_index",
        (
            PyList::new(py, [array])?,
            np.call_method1("empty", (0, "int64"))?,
            np.call_method1("empty", ((0, 16),))?,
            np.call_method1("empty", ((0, 3),))?,
            np.call_method1("empty", ((0, 3),))?,
        ),
    )?;
    let proto = built.get_item(0)?.get_item(0)?;
    Ok((&proto, &proto).into_pyobject(py)?.into_any())
}
#[pyfunction(signature=(packed,scene=None))]
fn resident_prepare<'p>(packed: &Obj<'p>, scene: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let py = packed.py();
    let np = py.import("numpy")?;
    let p = Placements::read(packed)?;
    let mut first = vec![None; packed.getattr("commands")?.len()?];
    {
        let ids = p.ids.extract::<numpy::PyReadonlyArray1<i64>>()?;
        for (row, &code) in ids.as_array().iter().enumerate() {
            first
                .get_mut(code as usize)
                .ok_or_else(|| PyValueError::new_err("invalid prototype code"))?
                .get_or_insert(row);
        }
    }
    let none = py.None().into_bound(py);
    let scene = scene.unwrap_or(&none);
    let lease = scene.is_instance_of::<SceneLease>();
    let lock = if lease {
        Some(scene.getattr("_reading")?)
    } else {
        None
    };
    if let Some(l) = &lock {
        l.call_method0("acquire")?;
    }
    let capture = || -> PyResult<Option<(Obj<'p>, Obj<'p>)>> {
        let document = if lease {
            scene.getattr("scene")?
        } else {
            scene.clone()
        };
        let canceled =
            || -> PyResult<bool> { Ok(lease && scene.getattr("released")?.is_truthy()?) };
        let arrays = PyList::empty(py);
        let cache = PyDict::new(py);
        for (code, row) in first.into_iter().enumerate() {
            if canceled()? {
                return Ok(None);
            }
            arrays.append(prototype_arrays(
                packed,
                np.as_any(),
                code,
                &document,
                row,
                &cache,
            )?)?;
        }
        if canceled()? {
            return Ok(None);
        }
        let weak = if document.is_none() {
            document
        } else {
            py.import("weakref")?.call_method1("ref", (document,))?
        };
        Ok(Some((arrays.into_any(), weak)))
    };
    let result = capture();
    if let Some(l) = lock {
        let clear = scene.setattr("scene", py.None());
        l.call_method0("release")?;
        clear?;
    }
    let Some((arrays, weak)) = result? else {
        return Ok(py.None().into_bound(py));
    };
    let built = p.build(
        py.import("built_core")?.as_any(),
        "build_snap_index",
        &arrays,
    )?;
    p.index(&built.get_item(1)?, &built.get_item(0)?, packed, &weak)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(resident_prepare, m)?)?;
    m.add_function(wrap_pyfunction!(resident_rebase, m)?)?;
    Ok(())
}
