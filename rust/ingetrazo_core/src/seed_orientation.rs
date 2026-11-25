// SPDX-License-Identifier: GPL-3.0-or-later
//! Propagate the reference face winding across two-face edges only.
use pyo3::{prelude::*, types::PyDict};
use std::collections::VecDeque;
type Obj<'p> = Bound<'p, PyAny>;

fn steps<'p>(f: &Obj<'p>) -> PyResult<Vec<(Obj<'p>, Obj<'p>)>> {
    let mut rings = vec![f.getattr("loop")?];
    if let Some(holes) = f.getattr_opt("hole_loops")?.filter(|v| !v.is_none()) {
        for ring in holes.try_iter()? {
            rings.push(ring?);
        }
    }
    let mut out = vec![];
    for ring in rings {
        let vs = ring.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        out.extend(
            vs.iter()
                .zip(vs.iter().cycle().skip(1))
                .map(|(a, b)| (a.clone(), b.clone())),
        );
    }
    Ok(out)
}

#[pyfunction]
pub(crate) fn edge_between<'p>(a: &Obj<'p>, b: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    for e in a.getattr("edges")?.try_iter()? {
        let edge = e?;
        let v0 = edge.getattr("v0")?;
        let v1 = edge.getattr("v1")?;
        if (v0.eq(a)? && v1.eq(b)?) || (v0.eq(b)? && v1.eq(a)?) {
            return Ok(Some(edge));
        }
    }
    Ok(None)
}

fn search<'p>(seed: &Obj<'p>, need_faces: bool) -> PyResult<(Vec<Obj<'p>>, usize)> {
    let py = seed.py();
    let api = py.import("core.store_mesh")?;
    if seed.is_instance(&api.getattr("FaceView")?)? {
        let mesh = seed.getattr("_m")?;
        if mesh.is_instance(&api.getattr("StoreMesh")?)? {
            let store = mesh.getattr("_store")?;
            let key = seed.getattr("key")?;
            if store.call_method1("is_live", (&key,))?.is_truthy()? {
                let plan = store
                    .call_method0("snapshot")?
                    .call_method1("seed_orientation", (key,))?;
                let mut faces = vec![];
                if need_faces {
                    let face = api.getattr("FaceView")?;
                    for k in plan.get_item(0)?.try_iter()? {
                        faces.push(face.call1((&mesh, k?))?);
                    }
                }
                return Ok((faces, plan.get_item(1)?.extract()?));
            }
        }
    }
    let seen = PyDict::new(py);
    seen.set_item(seed.getattr("key")?, false)?;
    let mut todo = VecDeque::from([(seed.clone(), false)]);
    let mut flipped = Vec::new();
    while let Some((face, turn)) = todo.pop_front() {
        for (mut a, mut b) in steps(&face)? {
            if turn {
                std::mem::swap(&mut a, &mut b);
            }
            let Some(edge) = edge_between(&a, &b)? else {
                continue;
            };
            let faces = edge.getattr("faces")?;
            if faces.len()? != 2 {
                continue;
            }
            for nb in faces.try_iter()? {
                let nb = nb?;
                let key = nb.getattr("key")?;
                if seen.contains(&key)? {
                    continue;
                }
                let mut reverse = false;
                if need_faces {
                    for (x, y) in steps(&nb)? {
                        if x.eq(&a)? && y.eq(&b)? {
                            reverse = true;
                            break;
                        }
                    }
                }
                seen.set_item(key, reverse)?;
                if reverse {
                    flipped.push(nb.clone());
                }
                todo.push_back((nb, reverse));
            }
        }
    }
    Ok((flipped, seen.len()))
}
#[pyfunction]
pub(crate) fn faces_to_flip<'p>(seed: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    Ok(search(seed, true)?.0)
}
#[pyfunction]
pub(crate) fn connected_count(seed: &Obj<'_>) -> PyResult<usize> {
    Ok(search(seed, false)?.1)
}
