//! Gather annotation records and resolve the model attachments of dimension endpoints.
use super::*;
fn items<'py>(obj: &Obj<'py>, name: &str) -> PyResult<Vec<Obj<'py>>> {
    match opt(obj, name) {
        Some(v) => v.try_iter()?.collect(),
        None => Ok(vec![]),
    }
}
fn lift<'py>(drape: &Obj<'py>, p: &Obj<'py>) -> PyResult<Obj<'py>> {
    if drape.is_none() {
        p.call_method0("toTuple")
    } else {
        drape.call1((p,))?.call_method0("toTuple")
    }
}
#[pyfunction]
fn drape_xy<'py>(drape: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    lift(drape, &core(drape.py())?.call_method1("Vec3", (x, y, 0.))?)
}
#[pyfunction]
#[pyo3(signature=(source,frame,providers=None))]
pub(super) fn annotations<'py>(
    source: &Obj<'py>,
    frame: &Obj<'py>,
    providers: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = source.py();
    let scene = source.getattr("scene")?;
    let view = saved_view(&scene, frame)?;
    if absent(&view) {
        return Ok(PyList::empty(py).into_any());
    }
    let (base, bounds) = camera_inputs(&source.getattr("camera")?, &scene)?;
    let ctx = PyDict::new(py);
    ctx.set_item("view", &view)?;
    ctx.set_item("base", base)?;
    ctx.set_item("bounds", bounds)?;
    let data = PyDict::new(py);
    data.set_item("root", scene.getattr("_root")?)?;
    data.set_item("groups", list(&scene.getattr("groups")?)?)?;
    data.set_item("loose", scene.getattr("loose_mesh")?)?;
    let drape = opt(source, "drape").unwrap_or_else(|| py.None().into_bound(py));
    let paths = PyList::empty(py);
    for path in items(&scene, "geo_paths")? {
        let points = PyList::empty(py);
        for p in items(&path, "points")? {
            points.append(lift(&drape, &p)?)?;
        }
        let profile = PyList::empty(py);
        for p in path.call_method0("profile_points")?.try_iter()? {
            profile.append(p?.call_method0("toTuple")?)?;
        }
        paths.append((points, path.getattr("closed")?, profile))?;
    }
    data.set_item("paths", paths)?;
    data.set_item(
        "drape",
        partial(py, &this(py)?.getattr("drape_xy")?, vec![drape])?,
    )?;
    let planes = PyList::empty(py);
    for sp in items(&scene, "section_planes")? {
        planes.append((
            sp.getattr("point")?.call_method0("toTuple")?,
            sp.getattr("normal")?.call_method0("toTuple")?,
            sp.getattr("symbol")?,
        ))?;
    }
    data.set_item("planes", planes)?;
    if let Some(providers) = providers.filter(|p| p.is_truthy().unwrap_or(false)) {
        data.set_item("providers", providers)?;
        data.set_item("scene", frame_scene(&scene, frame, &view)?)?;
    }
    if flag(frame, "annotations", false)? {
        let style = opt(&scene, "dimension_style").unwrap_or_else(|| PyDict::new(py).into_any());
        data.set_item("ends", style.call_method1("get", ("ends", "arrow"))?)?;
        let units = style.call_method1("get", ("units", "m"))?;
        let dec = py
            .import("builtins")?
            .call_method1("int", (style.call_method1("get", ("decimals", 2))?,))?;
        let show = style
            .call_method1("get", ("show_unit", true))?
            .is_truthy()?;
        let layers = py.import("core.layers")?;
        let fmt = py.import("core.units")?.getattr("format_length")?;
        let hidden = |e: &Obj<'_>| -> PyResult<bool> {
            Ok(scene.call_method1("entity_hidden", (e,))?.is_truthy()?
                || scene.call_method1("similar_hidden", (e,))?.is_truthy()?)
        };
        let dims = PyList::empty(py);
        for d in items(&scene, "dimensions")? {
            let mut row = vec![
                d.getattr("a")?.call_method0("toTuple")?,
                d.getattr("b")?.call_method0("toTuple")?,
            ];
            for p in d.call_method0("line_points")?.try_iter()? {
                row.push(p?.call_method0("toTuple")?);
            }
            row.push(d.call_method1(
                "display_text",
                (fmt.call1((d.call_method0("value")?, &units, &dec, show))?,),
            )?);
            row.push(layers.call_method1("layer_of", (&d,))?);
            row.push(PyBool::new(py, hidden(&d)?).to_owned().into_any());
            dims.append(PyTuple::new(py, row)?)?;
        }
        data.set_item("dims", dims)?;
        let labels = PyList::empty(py);
        for l in items(&scene, "text_labels")? {
            labels.append((
                l.getattr("anchor")?.call_method0("toTuple")?,
                l.call_method0("position")?.call_method0("toTuple")?,
                l.getattr("text")?,
                layers.call_method1("layer_of", (&l,))?,
                hidden(&l)?,
            ))?;
        }
        data.set_item("labels", labels)?;
    }
    core(py)?.call_method1("sheet_frame_annotations", (frame, ctx, data))
}
fn vec3<'py>(p: &Obj<'py>) -> PyResult<Obj<'py>> {
    core(p.py())?.getattr("Vec3")?.call1(as_tuple(p)?)
}
#[pyfunction]
#[pyo3(signature=(scene,item,attr,segment=None))]
fn capture_hold(
    scene: &Obj<'_>,
    item: &Obj<'_>,
    attr: &str,
    segment: Option<&Obj<'_>>,
) -> PyResult<bool> {
    let Some(cur) = opt(item, attr).filter(|v| v.is_truthy().unwrap_or(false)) else {
        return Ok(false);
    };
    let py = item.py();
    let anchors = py.import("core.annotation_anchor")?;
    let point = vec3(&cur)?;
    let store = |hit: &Obj<'_>, owner: &Obj<'_>| -> PyResult<bool> {
        let attachment = anchors
            .getattr("Attachment")?
            .call_method1("capture", (scene, hit, owner, &point))?;
        if attachment.is_none() {
            return Ok(false);
        }
        item.setattr(
            format!("hold_{}", attr.chars().next().unwrap_or_default()),
            (as_tuple(&cur)?, attachment),
        )?;
        Ok(true)
    };
    if let Some(seg) = segment {
        let (hit, owner): (Obj, Obj) = anchors
            .call_method1(
                "edge_by_endpoints",
                (scene, vec3(&seg.get_item(0)?)?, vec3(&seg.get_item(1)?)?),
            )?
            .extract()?;
        if !hit.is_none() && store(&hit, &owner)? {
            return Ok(true);
        }
    }
    let hold = py
        .import("core.dimension")?
        .call_method1("resolve_vertex_anchor", (scene, &point))?;
    if hold.is_none() {
        return Ok(false);
    }
    let chain = hold.getattr("chain")?;
    let owner = if chain.is_truthy()? {
        chain.get_item(-1)?
    } else {
        py.None().into_bound(py)
    };
    store(&hold.getattr("vertex")?, &owner)
}
#[pyfunction]
fn frame_uid<'py>(frame: &Obj<'py>) -> PyResult<Obj<'py>> {
    let uid = frame.getattr("uid")?;
    if uid.is_truthy()? {
        return Ok(uid);
    }
    let uid = frame
        .py()
        .import("uuid")?
        .call_method0("uuid4")?
        .getattr("hex")?;
    frame.setattr("uid", &uid)?;
    Ok(uid)
}
#[pyfunction]
fn anchor(item: &Obj<'_>, frame: &Obj<'_>, scene: &Obj<'_>, points: &Obj<'_>) -> PyResult<()> {
    item.setattr("anchor_uid", frame_uid(frame)?)?;
    for pair in points.try_iter()? {
        let pair = pair?;
        let attr: String = pair.get_item(0)?.extract()?;
        let pt = pair.get_item(1)?;
        item.setattr(&attr, pt.get_item("world")?.extract::<Vec<f64>>()?)?;
        if matches!(attr.as_str(), "a_world" | "b_world") {
            let seg = pt.call_method1("get", ("segment",))?;
            capture_hold(scene, item, &attr, (!seg.is_none()).then_some(&seg))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn follow_hold(scene: &Obj<'_>, item: &Obj<'_>, attr: &str, holds: &Obj<'_>) -> PyResult<bool> {
    let Some(cur) = opt(item, attr).filter(|v| v.is_truthy().unwrap_or(false)) else {
        return Ok(false);
    };
    let py = item.py();
    let slot = format!("hold_{}", attr.chars().next().unwrap_or_default());
    let coord = as_tuple(&cur)?;
    if let Some(saved) = opt(item, &slot) {
        if as_tuple(&saved.get_item(0)?)?.eq(&coord)? {
            let original = saved.get_item(1)?;
            let reference = if original.is_instance_of::<PyDict>() {
                py.import("core.annotation_anchor")?
                    .getattr("Attachment")?
                    .call_method1("restore", (scene, &original, vec3(&cur)?))?
            } else {
                original.clone()
            };
            if !reference.is_none() {
                let p = reference.call_method0("position")?;
                if !p.is_none() {
                    let now = p.call_method0("toTuple")?;
                    let moved = !coord.eq(&now)?;
                    if moved {
                        item.setattr(attr, list(&now)?)?;
                    }
                    if moved || !reference.is(&original) {
                        item.setattr(&slot, (now, reference))?;
                    }
                    return Ok(true);
                }
            }
        }
    }
    let key = (item.getattr("_doc_key")?, attr).into_pyobject(py)?;
    let held = holds.call_method1("get", (&key,))?;
    let current = list(&cur)?;
    if held.is_none() || !held.get_item(0)?.eq(&current)? {
        let hold = py
            .import("core.dimension")?
            .call_method1("resolve_vertex_anchor", (scene, vec3(&cur)?))?;
        let found = !hold.is_none();
        holds.set_item(key, (current, hold))?;
        return Ok(found);
    }
    let hold = held.get_item(1)?;
    if hold.is_none() {
        return Ok(false);
    }
    let p = hold.call_method1("position", (scene,))?;
    if p.is_none() {
        holds.set_item(key, (current, py.None()))?;
        return Ok(false);
    }
    let now = list(&p.call_method0("toTuple")?)?;
    if !now.eq(current)? {
        item.setattr(attr, &now)?;
        holds.set_item(key, (now, hold))?;
    }
    Ok(true)
}
#[pyfunction]
fn anchor_snaps<'py>(
    source: &Obj<'py>,
    frame: &Obj<'py>,
) -> PyResult<Option<(Obj<'py>, Obj<'py>)>> {
    geometry::snaps(source, frame)?
        .map(|s| Ok((s.get_item(1)?, s.get_item(3)?)))
        .transpose()
}
#[pyfunction]
fn anchor_hold(scene: &Obj<'_>, holds: &Obj<'_>, item: &Obj<'_>, attr: &str) -> PyResult<bool> {
    follow_hold(scene, item, attr, holds)
}
#[pyfunction]
fn follow_anchors<'py>(
    source: &Obj<'py>,
    comp: &Obj<'py>,
    memo: &Obj<'py>,
    holds: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = source.py();
    let scene = source.getattr("scene")?;
    let (base, bounds) = camera_inputs(&source.getattr("camera")?, &scene)?;
    let frames = PyList::empty(py);
    for frame in items(comp, "frames")? {
        let view = saved_view(&scene, &frame)?;
        if absent(&view) || !flag(&frame, "uid", false)? {
            continue;
        }
        let ctx = PyDict::new(py);
        ctx.set_item("frame", frame)?;
        ctx.set_item("view", view)?;
        ctx.set_item("base", &base)?;
        ctx.set_item("bounds", bounds)?;
        frames.append(ctx)?;
    }
    let m = this(py)?;
    core(py)?.call_method1(
        "sheet_follow_anchors",
        (
            comp,
            frames,
            partial(py, &m.getattr("anchor_snaps")?, vec![source.clone()])?,
            partial(py, &m.getattr("anchor_hold")?, vec![scene, holds.clone()])?,
            memo,
        ),
    )
}
#[pyclass(dict, module = "ingetrazo_render.sheet_frames")]
struct Ground;
#[pymethods]
impl Ground {
    #[new]
    #[pyo3(signature=(changed,owner=None))]
    fn new(changed: &Obj<'_>, owner: Option<&Obj<'_>>) -> Self {
        let _ = (changed, owner);
        Self
    }
    #[pyo3(signature=(changed,owner=None))]
    fn __init__(s: &Bound<'_, Self>, changed: &Obj<'_>, owner: Option<&Obj<'_>>) -> PyResult<()> {
        s.setattr("_owner", owner)?;
        s.setattr("_changed", changed)?;
        s.setattr("_dem", s.py().None())?;
        s.setattr("_survey", s.py().None())
    }
    fn profiles<'py>(
        s: &Bound<'py, Self>,
        comp: &Obj<'py>,
        scene: &Obj<'py>,
    ) -> PyResult<Obj<'py>> {
        let py = s.py();
        if !comp.getattr("perfiles")?.is_truthy()? {
            return Ok(py.None().into_bound(py));
        }
        let datum = scene
            .getattr("georef")
            .unwrap_or_else(|_| py.None().into_bound(py));
        let mut dem = s.getattr("_dem")?;
        if dem.is_none() || !dem.getattr("datum")?.is(&datum) {
            let kw = PyDict::new(py);
            let owner = s.getattr("_owner")?;
            if !owner.is_none() {
                kw.set_item("parent", owner)?;
            }
            dem = py
                .import("georef.dem")?
                .getattr("DEMSampler")?
                .call((&datum,), Some(&kw))?;
            dem.getattr("changed")?
                .call_method1("connect", (s.getattr("_changed")?,))?;
            s.setattr("_dem", &dem)?;
        }
        let mut sampler = py.None().into_bound(py);
        if !datum.is_none() {
            if let Some(survey) = opt(scene, "photo_mesh") {
                if flag(&survey, "visible", false)? {
                    sampler = s.getattr("_survey")?;
                    if sampler.is_none()
                        || !sampler.getattr("mesh")?.is(&survey)
                        || !sampler.getattr("datum")?.is(&datum)
                    {
                        sampler = py
                            .import("georef.photomesh")?
                            .call_method1("PhotoMeshSampler", (survey, datum))?;
                        s.setattr("_survey", &sampler)?;
                    }
                }
            }
        }
        core(py)?.call_method1("sheet_terrain_profiles", (comp, scene, dem, sampler))
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Ground>()?;
    for f in [
        wrap_pyfunction!(annotations, m)?,
        wrap_pyfunction!(drape_xy, m)?,
        wrap_pyfunction!(capture_hold, m)?,
        wrap_pyfunction!(follow_hold, m)?,
        wrap_pyfunction!(anchor, m)?,
        wrap_pyfunction!(frame_uid, m)?,
        wrap_pyfunction!(anchor_snaps, m)?,
        wrap_pyfunction!(anchor_hold, m)?,
        wrap_pyfunction!(follow_anchors, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
