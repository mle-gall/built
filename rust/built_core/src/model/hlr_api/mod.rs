//! Scene collection, section cuts and layer-aware sheet drawings.
use super::{
    hlr_view as view, section_cut as section,
    section_fill::{arr, flat_f64},
};
use crate::sheets::drawing as lines;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
mod collect;
use collect::*;
type Obj<'p> = Bound<'p, PyAny>;
type P = [f64; 3];
fn truth(value: &Obj<'_>) -> PyResult<bool> {
    value.is_truthy()
}

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn list(py: Python<'_>) -> Obj<'_> {
    PyList::empty(py).into_any()
}
fn rows<const N: usize>(v: &Obj<'_>) -> PyResult<Vec<[P; N]>> {
    let f = flat_f64(v)?;
    if f.len() % (3 * N) != 0 {
        return Err(PyValueError::new_err("incomplete geometry row"));
    }
    Ok(f.chunks_exact(3 * N)
        .map(|r| std::array::from_fn(|i| [r[3 * i], r[3 * i + 1], r[3 * i + 2]]))
        .collect())
}
#[pyfunction(signature=(tris,hard,soft,plane,split_cuts=false))]
fn clip_to_section<'p>(
    py: Python<'p>,
    tris: &Obj<'p>,
    hard: &Obj<'p>,
    soft: &Obj<'p>,
    plane: &Obj<'p>,
    #[pyo3(from_py_with=truth)] split_cuts: bool,
) -> PyResult<Bound<'p, PyTuple>> {
    section::section_clip(py, tris, hard, soft, plane, split_cuts)
}
#[pyfunction(signature=(cut_edges,tol=None,members=None))]
fn section_loops<'p>(
    py: Python<'p>,
    cut_edges: &Obj<'p>,
    tol: Option<&Obj<'p>>,
    members: Option<&Bound<'p, PyList>>,
) -> PyResult<Bound<'p, PyList>> {
    if cut_edges.len()? == 0 {
        return Ok(PyList::empty(py));
    }
    let tol = tol
        .map(|t| {
            py.import("builtins")?
                .call_method1("float", (t,))?
                .extract::<f64>()
        })
        .transpose()?;
    section::section_loops(py, cut_edges, tol, members)
}
fn active<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    if scene
        .getattr_opt("show_section_cuts")?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(true)
    {
        if let Some(f) = scene.getattr_opt("active_section")? {
            return f.call0();
        }
    }
    Ok(none(scene.py()))
}
struct Geometry<'p> {
    tris: Obj<'p>,
    hard: Obj<'p>,
    soft: Obj<'p>,
    normals: Obj<'p>,
    tags: Option<[Obj<'p>; 3]>,
}
impl<'p> Geometry<'p> {
    fn read(scene: &Obj<'p>, g: Option<&Obj<'p>>, layers: bool) -> PyResult<Self> {
        let py = scene.py();
        let packed = match g {
            Some(g) if !layers || g.len()? == 7 => Some(g),
            _ => None,
        };
        let data = match packed {
            Some(g) => g.clone(),
            None => collect_geometry(scene, layers)?.into_any(),
        };
        // Viewport arrays put triangle tags first. The scene walk returns them last.
        let tags = if !layers {
            None
        } else if packed.is_some() {
            Some([data.get_item(4)?, data.get_item(5)?, data.get_item(6)?])
        } else {
            Some([data.get_item(5)?, data.get_item(3)?, data.get_item(4)?])
        };
        if packed.is_some() && data.len()? != if layers { 7 } else { 4 } {
            return Err(PyValueError::new_err(
                "expected four geometry arrays, or seven with layer tags",
            ));
        }
        Ok(Self {
            tris: data.get_item(0)?,
            hard: data.get_item(1)?,
            soft: data.get_item(2)?,
            normals: if packed.is_some() {
                data.get_item(3)?
            } else {
                none(py)
            },
            tags,
        })
    }
    fn clip(&mut self, sp: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>)> {
        let py = sp.py();
        if !self.normals.is_none() {
            if self.soft.len()? != self.normals.len()? {
                return Err(PyValueError::new_err("one normal pair per soft edge"));
            }
            let t =
                view::hlr_geometry_lists(py, &self.tris, &self.hard, &self.soft, &self.normals)?;
            self.tris = t.get_item(0)?;
            self.hard = t.get_item(1)?;
            self.soft = t.get_item(2)?;
            self.normals = none(py);
        }
        let Some(tags) = &mut self.tags else {
            let t = clip_to_section(py, &self.tris, &self.hard, &self.soft, sp, true)?;
            self.tris = t.get_item(0)?;
            self.hard = t.get_item(1)?;
            self.soft = t.get_item(2)?;
            return Ok((t.get_item(3)?, list(py)));
        };
        let cuts = PyList::empty(py);
        let cut_tags = PyList::empty(py);
        for (i, items) in [&mut self.tris, &mut self.hard, &mut self.soft]
            .into_iter()
            .enumerate()
        {
            let out = PyList::empty(py);
            let kept_tags = PyList::empty(py);
            for (tag, group) in grouped(items, &tags[i])?.iter() {
                let mut input = std::array::from_fn::<_, 3, _>(|_| list(py));
                input[i] = group;
                let parts = clip_to_section(py, &input[0], &input[1], &input[2], sp, true)?;
                for row in parts.get_item(i)?.try_iter()? {
                    out.append(row?)?;
                    kept_tags.append(&tag)?
                }
                if i == 0 {
                    for row in parts.get_item(3)?.try_iter()? {
                        cuts.append(row?)?;
                        cut_tags.append(&tag)?
                    }
                }
            }
            *items = out.into_any();
            tags[i] = kept_tags.into_any();
        }
        Ok((cuts.into_any(), cut_tags.into_any()))
    }
}
fn grouped<'p>(values: &Obj<'p>, tags: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let groups = PyDict::new(values.py());
    for (v, t) in values.try_iter()?.zip(tags.try_iter()?) {
        let t = t?;
        let group = match groups.get_item(&t)? {
            Some(g) => g,
            None => {
                let g = list(values.py());
                groups.set_item(&t, &g)?;
                g
            }
        };
        group.call_method1("append", (v?,))?;
    }
    Ok(groups)
}
struct Rings<'p> {
    drawn: Obj<'p>,
    layers: Obj<'p>,
    depths: Obj<'p>,
    cap: Option<lines::Cap>,
}
#[allow(clippy::too_many_arguments)]
fn rings<'p>(
    sp: &Obj<'p>,
    cuts: &Obj<'p>,
    cut_tags: &Obj<'p>,
    whole: &Obj<'p>,
    whole_tags: &Obj<'p>,
    layered: bool,
    depth_layers: Option<&Obj<'p>>,
    basis: [P; 4],
    caps: bool,
) -> PyResult<Rings<'p>> {
    let py = sp.py();
    let drawn = PyList::empty(py);
    let tags = PyList::empty(py);
    let depths = PyList::empty(py);
    let mut p = Vec::new();
    let mut q = Vec::new();
    let [e, r, u, f] = basis;
    let mut cap = None;
    if cuts.len()? > 0 {
        let groups = if layered {
            let closed = view::hlr_closing_tags(py, cuts, cut_tags)?;
            grouped(cuts, closed.as_any())?
        } else {
            let d = PyDict::new(py);
            d.set_item(py.None(), cuts)?;
            d
        };
        let wanted = match depth_layers {
            Some(w) if layered && w.is_truthy()? => {
                Some(py.import("builtins")?.call_method1("set", (w,))?)
            }
            _ => None,
        };
        let mut triangles = None;
        for (tag, edges) in groups.iter() {
            let edge = rows::<2>(&edges)?;
            let (nodes, loops) = section::chain_loops(&edge, None);
            for (idx, _) in loops {
                let world: Vec<P> = idx.iter().map(|&i| nodes[i]).collect();
                let xy: Vec<[f64; 2]> = world
                    .iter()
                    .map(|v| {
                        let a = view::to_cam(v, &e, &r, &u, &f);
                        [a[0], a[1]]
                    })
                    .collect();
                drawn.append(arr(
                    py,
                    xy.iter().flatten().copied().collect(),
                    &[xy.len(), 2],
                )?)?;
                if caps {
                    p.extend(&xy);
                    q.extend(xy.iter().cycle().skip(1).take(xy.len()))
                }
                if layered {
                    tags.append(&tag)?
                }
                if let Some(w) = &wanted {
                    let depth = if w.contains(&tag)? {
                        let groups = match &triangles {
                            Some(g) => g,
                            None => triangles.insert(grouped(whole, whole_tags)?),
                        };
                        let tris = match groups.get_item(&tag)? {
                            Some(t) => rows::<3>(&t)?,
                            None => vec![],
                        };
                        view::solid_depth(
                            &world,
                            &tris,
                            crate::math::xyz(&sp.getattr("normal")?)?.to_array(),
                        )
                    } else {
                        None
                    };
                    depths.append(depth)?;
                }
            }
        }
        if caps && !p.is_empty() {
            let eye = e.into_pyobject(py)?;
            let right = r.into_pyobject(py)?;
            let up = u.into_pyobject(py)?;
            let fwd = f.into_pyobject(py)?;
            if let Some(abg) = view::hlr_cap_depth_affine(
                py,
                sp,
                eye.as_any(),
                right.as_any(),
                up.as_any(),
                fwd.as_any(),
            )? {
                cap = Some(lines::Cap {
                    p,
                    q,
                    abg: abg.extract()?,
                })
            }
        }
    }
    Ok(Rings {
        drawn: drawn.into_any(),
        layers: if layered { tags.into_any() } else { none(py) },
        depths: depths.into_any(),
        cap,
    })
}
#[pyfunction(signature=(scene,camera,geometry=None,profiles=true,fills=true,caps=true,hidden=false,layers=false,depth_layers=None))]
fn hlr_drawing<'p>(
    scene: &Obj<'p>,
    camera: &Obj<'p>,
    geometry: Option<&Obj<'p>>,
    #[pyo3(from_py_with=truth)] profiles: bool,
    #[pyo3(from_py_with=truth)] fills: bool,
    #[pyo3(from_py_with=truth)] caps: bool,
    #[pyo3(from_py_with=truth)] hidden: bool,
    #[pyo3(from_py_with=truth)] layers: bool,
    depth_layers: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let g = Geometry::read(scene, geometry, layers)?;
    draw(
        g,
        camera,
        &active(scene)?,
        profiles,
        fills,
        caps,
        hidden,
        depth_layers,
    )
}
fn draw<'p>(
    mut g: Geometry<'p>,
    camera: &Obj<'p>,
    sp: &Obj<'p>,
    profiles: bool,
    fills: bool,
    caps: bool,
    hidden: bool,
    depth_layers: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = camera.py();
    let whole = g.tris.clone();
    let whole_tags = g
        .tags
        .as_ref()
        .map(|t| t[0].clone())
        .unwrap_or_else(|| none(py));
    let (cuts, ctags) = if sp.is_none() {
        (list(py), list(py))
    } else {
        g.clip(sp)?
    };
    let basis = view::hlr_camera_basis(py, camera)?;
    let (e, r, u, f): (P, P, P, P) = basis.extract()?;
    let forward = basis.get_item(3)?;
    // The classification kernel indexes hard edges, soft silhouettes and cut chords in that order.
    let mut edges = rows::<2>(&g.hard)?;
    let nh = edges.len();
    let tags = PyList::empty(py);
    if let Some(ts) = &g.tags {
        tags.call_method1("extend", (&ts[1],))?;
    }
    let mut push = |row, idx| -> PyResult<()> {
        edges.push(row);
        if let Some(ts) = &g.tags {
            tags.append(ts[2].get_item(idx)?)?
        }
        Ok(())
    };
    let np = py.import("numpy")?;
    if !g.normals.is_none() && g.soft.len()? > 0 {
        // Use the original NumPy matrix products. The sign at a grazing angle
        // determines whether the line is present at all.
        let normals = np.call_method1("asarray", (&g.normals,))?;
        let all = py.import("builtins")?.call_method1("slice", (py.None(),))?;
        let a: Vec<f64> = normals
            .get_item((&all, 0, &all))?
            .matmul(&forward)?
            .extract()?;
        let b: Vec<f64> = normals
            .get_item((&all, 1, &all))?
            .matmul(&forward)?
            .extract()?;
        let soft = rows::<2>(&g.soft)?;
        if a.len() != soft.len() || b.len() != soft.len() {
            return Err(PyValueError::new_err("one normal pair per soft edge"));
        }
        for (i, row) in soft.into_iter().enumerate() {
            if b[i].is_nan() || (a[i] < 0.) != (b[i] < 0.) {
                push(row, i)?
            }
        }
    } else if g.normals.is_none() {
        for (i, row) in g.soft.try_iter()?.enumerate() {
            let row = row?;
            let nb = row.get_item(3)?;
            if !nb.is_none() {
                let facing = |v: Obj<'p>| -> PyResult<bool> {
                    Ok(np.call_method1("dot", (v, &forward))?.extract::<f64>()? < 0.)
                };
                if facing(row.get_item(2)?)? == facing(nb)? {
                    continue;
                }
            }
            push(
                [row.get_item(0)?.extract()?, row.get_item(1)?.extract()?],
                i,
            )?;
        }
    }
    let ns = edges.len() - nh;
    edges.extend(rows::<2>(&cuts)?);
    let class = py.import("core.hlr")?.getattr("HlrDrawing")?;
    if edges.is_empty() {
        return class.call1((
            arr::<f64>(py, vec![], &[0, 4])?,
            arr::<f64>(py, vec![], &[0, 2, 3])?,
            arr::<i8>(py, vec![], &[0])?,
            list(py),
        ));
    }
    let layered = g.tags.is_some();
    let ring = rings(
        sp,
        &cuts,
        &ctags,
        &whole,
        &whole_tags,
        layered,
        depth_layers,
        [e, r, u, f],
        caps,
    )?;
    let ids = if layered {
        tags.call_method1("extend", (&ctags,))?;
        if tags.len() != edges.len() {
            return Err(PyValueError::new_err("one tag per edge"));
        }
        let table = PyDict::new(py);
        let mut ids = Vec::new();
        for t in tags.iter() {
            let id = table
                .call_method1("setdefault", (t, table.len()))?
                .extract()?;
            ids.push(id)
        }
        Some(ids)
    } else {
        None
    };
    let cam = |p: &P| view::to_cam(p, &e, &r, &u, &f);
    let a: Vec<_> = edges.iter().map(|v| cam(&v[0])).collect();
    let b: Vec<_> = edges.iter().map(|v| cam(&v[1])).collect();
    let tris = rows::<3>(&g.tris)?
        .into_iter()
        .map(|t| t.map(|p| cam(&p)))
        .collect();
    let cap = ring.cap;
    let d = py.detach(|| {
        lines::drawing(&lines::Input {
            a: &a,
            b: &b,
            world: &edges,
            n_hard: nh,
            n_sil: ns,
            tris,
            profiles,
            hidden,
            cap,
            tags: ids.as_deref(),
        })
    });
    let n = d.segs.len();
    let layers = if layered {
        let out = PyList::empty(py);
        for i in d.rows {
            out.append(tags.get_item(i as usize)?)?
        }
        np.call_method1("asarray", (out, "object"))?
    } else {
        none(py)
    };
    class.call1((
        arr(py, d.segs.into_iter().flatten().collect(), &[n, 4])?,
        arr(
            py,
            d.world.into_iter().flatten().flatten().collect(),
            &[n, 2, 3],
        )?,
        arr(py, d.kinds, &[n])?,
        if fills { ring.drawn } else { list(py) },
        layers,
        if layered && !fills {
            list(py)
        } else {
            ring.layers
        },
        if fills
            && depth_layers
                .map(|v| v.is_truthy())
                .transpose()?
                .unwrap_or(false)
        {
            ring.depths
        } else {
            none(py)
        },
    ))
}
#[pyfunction(signature=(scene,camera,return_world=false,geometry=None))]
fn hlr_view<'p>(
    scene: &Obj<'p>,
    camera: &Obj<'p>,
    #[pyo3(from_py_with=truth)] return_world: bool,
    geometry: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let d = hlr_drawing(
        scene, camera, geometry, false, false, true, false, false, None,
    )?;
    let segs = d.getattr("segs")?;
    if return_world {
        Ok(PyTuple::new(scene.py(), [segs, d.getattr("world")?])?.into_any())
    } else {
        Ok(segs)
    }
}
#[pyfunction(signature=(scene,camera,geometry=None))]
fn hlr_perspective<'p>(
    scene: &Obj<'p>,
    camera: &Obj<'p>,
    geometry: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let mut g = Geometry::read(scene, geometry, false)?;
    let sp = active(scene)?;
    let cut = if sp.is_none() {
        list(py)
    } else {
        g.clip(&sp)?.0
    };
    let Some(parts) =
        view::hlr_perspective_parts(py, &g.tris, &g.hard, &cut, &g.soft, &g.normals, camera)?
    else {
        return arr::<f64>(py, vec![], &[0, 4]);
    };
    let top = py.import("core.camera")?.call_method0("OrbitCamera")?;
    top.setattr("perspective", false)?;
    top.setattr(
        "target",
        camera.getattr("target")?.get_type().call1((0., 0., 0.))?,
    )?;
    let pitch = std::f64::consts::FRAC_PI_2;
    top.setattr("yaw", -pitch)?;
    top.setattr("pitch", pitch)?;
    top.setattr("distance", parts.get_item(2)?.extract::<f64>()? + 1.)?;
    let empty = arr::<f64>(py, vec![], &[0, 2, 3])?;
    let g = Geometry {
        tris: parts.get_item(0)?,
        hard: parts.get_item(1)?,
        soft: empty.clone(),
        normals: empty,
        tags: None,
    };
    draw(g, &top, &none(py), false, false, false, false, None)?.getattr("segs")
}
#[pyfunction(signature=(record,segs,world,kinds,loops,layers=None,loop_layers=None,loop_depths=None))]
fn init_record(
    record: &Obj<'_>,
    segs: &Obj<'_>,
    world: &Obj<'_>,
    kinds: &Obj<'_>,
    loops: &Obj<'_>,
    layers: Option<&Obj<'_>>,
    loop_layers: Option<&Obj<'_>>,
    loop_depths: Option<&Obj<'_>>,
) -> PyResult<()> {
    for (name, value) in [
        ("segs", Some(segs)),
        ("world", Some(world)),
        ("kinds", Some(kinds)),
        ("loops", Some(loops)),
        ("layers", layers),
        ("loop_layers", loop_layers),
        ("loop_depths", loop_depths),
        ("styles", None),
        ("labels", None),
    ] {
        record.setattr(name, value)?
    }
    Ok(())
}
#[pyfunction]
fn len_record(record: &Obj<'_>) -> PyResult<usize> {
    record.getattr("segs")?.len()
}
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let ns = PyDict::new(py);
    ns.set_item("__module__", "core.hlr")?;
    ns.set_item("__doc__", "Visible segments, their world endpoints and line classes, with section loops and optional layer metadata.")?;
    ns.set_item(
        "__slots__",
        (
            "segs",
            "world",
            "kinds",
            "loops",
            "layers",
            "styles",
            "labels",
            "loop_layers",
            "loop_depths",
        ),
    )?;
    let method = py.import("functools")?.getattr("partialmethod")?;
    ns.set_item(
        "__init__",
        method.call1((wrap_pyfunction!(init_record, m)?,))?,
    )?;
    ns.set_item(
        "__len__",
        method.call1((wrap_pyfunction!(len_record, m)?,))?,
    )?;
    let class = py
        .import("builtins")?
        .call_method1("type", ("HlrDrawing", PyTuple::empty(py), ns))?;
    m.add("HlrDrawing", class)?;
    for (name, value) in [
        ("KIND_EDGE", lines::KIND_EDGE),
        ("KIND_PROFILE", lines::KIND_PROFILE),
        ("KIND_CUT", lines::KIND_CUT),
        ("KIND_HIDDEN", lines::KIND_HIDDEN),
    ] {
        m.add(name, value)?
    }
    m.add_function(wrap_pyfunction!(collect_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(collect_circles, m)?)?;
    m.add_function(wrap_pyfunction!(clip_to_section, m)?)?;
    m.add_function(wrap_pyfunction!(section_loops, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_drawing, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_view, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_perspective, m)?)?;
    Ok(())
}
