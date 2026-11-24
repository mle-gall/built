// SPDX-License-Identifier: GPL-3.0-or-later
//! Build one definition-local mesh before any occurrence can see its store.
use crate::math::xyz;
use crate::store::{
    build::{Block, Points},
    edit::Change,
    fillet::maximal_holes,
    geometry::{Hypot, V3},
    py::PyMeshStore,
    Store, SOFT,
};
use numpy::{PyReadonlyArray2, PyReadonlyArray3};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::collections::HashMap;

type Point = [f64; 3];
type Segment = [Point; 2];

struct Face {
    rings: Vec<Vec<Point>>,
    attribute: i64,
}

fn read_ring(ring: &Bound<'_, PyAny>, single: &mut bool) -> PyResult<Vec<Point>> {
    if ring.hasattr("_coordinate_span")? {
        let (array, indices) = ring
            .call_method0("_coordinate_span")?
            .extract::<(Bound<PyAny>, Bound<PyAny>)>()?;
        let start: isize = indices.getattr("start")?.extract()?;
        let step: isize = indices.getattr("step")?.extract()?;
        let count = indices.len()?;
        macro_rules! rows {
            ($array:expr) => {{
                let a = $array.as_array();
                if a.shape()[1] != 3 {
                    return Err(PyValueError::new_err("expected three coordinates"));
                }
                let mut points = Vec::with_capacity(count);
                for k in 0..count {
                    let i = step
                        .checked_mul(k as isize)
                        .and_then(|n| start.checked_add(n))
                        .and_then(|n| usize::try_from(n).ok())
                        .filter(|&i| i < a.shape()[0])
                        .ok_or_else(|| PyValueError::new_err("invalid coordinate span"))?;
                    points.push([a[[i, 0]] as f64, a[[i, 1]] as f64, a[[i, 2]] as f64]);
                }
                return Ok(points);
            }};
        }
        if let Ok(a) = array.extract::<PyReadonlyArray2<f32>>() {
            rows!(a);
        }
        *single = false;
        if let Ok(a) = array.extract::<PyReadonlyArray2<f64>>() {
            rows!(a);
        }
    }
    *single = false;
    ring.try_iter()?.map(|p| Ok(xyz(&p?)?.to_array())).collect()
}

fn segments(source: &Bound<'_, PyAny>) -> PyResult<Vec<Segment>> {
    if source.is_none() {
        return Ok(Vec::new());
    }
    if let Some(chunks) = source.getattr_opt("chunks")? {
        let mut out = Vec::with_capacity(source.len()?);
        for chunk in chunks.try_iter()? {
            let chunk = chunk?;
            macro_rules! append {
                ($array:expr) => {{
                    let arr = $array.as_array();
                    if arr.shape()[1..] != [2, 3] {
                        return Err(PyValueError::new_err("expected pairs of three coordinates"));
                    }
                    for pair in arr.outer_iter() {
                        out.push(std::array::from_fn(|i| {
                            std::array::from_fn(|j| pair[[i, j]] as f64)
                        }));
                    }
                }};
            }
            if let Ok(arr) = chunk.extract::<PyReadonlyArray3<f32>>() {
                append!(arr);
            } else {
                let arr = chunk.extract::<PyReadonlyArray3<f64>>()?;
                append!(arr);
            }
        }
        return Ok(out);
    }
    source
        .try_iter()?
        .map(|row| {
            let (a, b) = row?.extract::<(Bound<PyAny>, Bound<PyAny>)>()?;
            Ok([xyz(&a)?.to_array(), xyz(&b)?.to_array()])
        })
        .collect()
}

fn build(
    mut store: Store,
    faces: &[Face],
    single: bool,
    soft: Vec<Segment>,
    loose: Vec<Segment>,
    curves: Vec<(i64, Vec<Segment>)>,
) -> Result<Store, String> {
    let total = faces
        .iter()
        .flat_map(|f| &f.rings)
        .map(Vec::len)
        .sum::<usize>();
    let bulk = total >= 1024 && faces.iter().all(|f| f.rings.iter().all(|r| !r.is_empty()));
    if bulk {
        let values = faces.iter().flat_map(|f| &f.rings).flatten();
        let narrow: Vec<_>;
        let wide: Vec<_>;
        let pos = if single {
            narrow = values.map(|p| p.map(|v| v as f32)).collect();
            Points::Single(&narrow)
        } else {
            wide = values.copied().collect();
            Points::Double(&wide)
        };
        let indices = (0..total as i64).collect::<Vec<_>>();
        let sizes = faces
            .iter()
            .flat_map(|f| &f.rings)
            .map(|r| r.len() as i64)
            .collect::<Vec<_>>();
        let counts = faces
            .iter()
            .map(|f| f.rings.len() as i64)
            .collect::<Vec<_>>();
        let attrs = faces.iter().map(|f| f.attribute).collect::<Vec<_>>();
        let block = Block {
            pos,
            edges: &[],
            flags: None,
            curves: None,
            layers: None,
            layer_names: Vec::new(),
            rings: &indices,
            sizes: &sizes,
            counts: &counts,
            face_attr: Some(&attrs),
            sequential: false,
        };
        store = Store::from_block(store.state.ns, &block, store.weld)?;
    } else {
        for face in faces {
            let mut change = Change::default();
            use crate::store::topo::OpError;
            let slot = match store.add_face(&face.rings, &mut change) {
                Ok(slot) => slot,
                Err(OpError::Full) => return Err("definition exceeds the store capacity".into()),
                Err(_) => {
                    store.finish(change);
                    continue;
                }
            };
            if face.attribute >= 0 {
                store.set_face_attr(slot, (face.attribute + 1) as u32);
            }
            store.finish(change);
        }
    }
    // Bulk welding may leave a later point registered in the same probe
    // neighbourhood. The stored coordinate wins for an exact endpoint.
    let key = |p: Point| p.map(|x| if x == 0. { 0 } else { x.to_bits() });
    let mut exact = HashMap::new();
    if bulk && !soft.is_empty() {
        for i in 0..store.state.vpos.len() {
            exact.insert(key(*store.state.vpos.get(i)), i as u32);
        }
    }
    for [a, b] in soft {
        let vertex = |p| exact.get(&key(p)).copied().or_else(|| store.lookup(p));
        if let (Some(a), Some(b)) = (vertex(a), vertex(b)) {
            if let Some(edge) = (a != b).then(|| store.find_edge(a, b)).flatten() {
                store.set_edge_flags(edge, SOFT);
            }
        }
    }
    for [a, b] in loose {
        let mut change = Change::default();
        match store.add_edge(a, b, &mut change) {
            Ok(_) | Err(crate::store::topo::OpError::Degenerate) => (),
            Err(e) => return Err(format!("edge: {e:?}")),
        }
        store.finish(change);
    }
    for (id, segments) in curves {
        for [a, b] in segments {
            if let (Some(a), Some(b)) = (store.lookup(a), store.lookup(b)) {
                if let Some(edge) = store.find_edge(a, b) {
                    store.set_edge_curve(edge, id);
                }
            }
        }
    }
    Ok(store)
}

#[pyfunction]
pub fn skp_definition<'py>(
    py: Python<'py>,
    mesh: Bound<'py, PyAny>,
    faces: &Bound<'py, PyAny>,
    soft: &Bound<'py, PyAny>,
    loose: &Bound<'py, PyAny>,
    curves: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let initial = {
        let s = store.try_borrow()?;
        if s.inner.revision != 0 || !s.inner.state.vpos.is_empty() {
            return Err(PyValueError::new_err("expected an empty definition"));
        }
        Store::empty(s.inner.state.ns, s.inner.weld)
    };
    let table = mesh.getattr("_attr_table")?;
    let table_index = py.import("core.store_mesh")?.getattr("_table_index")?;
    let index = PyDict::new(py);
    let mut rows = Vec::new();
    let mut single = true;
    for (i, face) in faces.try_iter()?.enumerate() {
        if i % 1024 == 0 && i != 0 {
            py.detach(std::thread::yield_now);
        }
        let (outer, holes, attr) = face?.extract::<(Bound<PyAny>, Bound<PyAny>, Bound<PyAny>)>()?;
        let mut rings = vec![read_ring(&outer, &mut single)?];
        if !holes.is_none() {
            for hole in holes.try_iter()? {
                rings.push(read_ring(&hole?, &mut single)?);
            }
        }
        if rings.len() > 2 {
            if rings[1..].iter().any(|r| r.len() < 3) {
                continue;
            }
            let polygons = rings[1..]
                .iter()
                .map(|r| r.iter().map(|&[x, y, z]| V3(x, y, z)).collect())
                .collect::<Vec<_>>();
            let mask = py.detach(|| maximal_holes(&polygons, Hypot::LibStdCxx));
            let mut keep = std::iter::once(true).chain(mask);
            rings.retain(|_| keep.next().unwrap_or(false));
            single = false;
        }
        let attribute = if attr.is_truthy()? {
            table_index.call1((&table, &index, attr))?.extract()?
        } else {
            -1
        };
        rows.push(Face { rings, attribute });
    }
    let soft = segments(soft)?;
    let loose = segments(loose)?;
    let mut strokes = Vec::new();
    if !curves.is_none() {
        for curve in curves.try_iter()? {
            strokes.push((
                mesh.call_method0("next_curve_id")?.extract()?,
                segments(&curve?)?,
            ));
        }
    }
    let result = py
        .detach(move || build(initial, &rows, single, soft, loose, strokes))
        .map_err(PyValueError::new_err)?;
    store.try_borrow_mut()?.inner = result;
    mesh.call_method0("_bump")?;
    Ok(mesh)
}
