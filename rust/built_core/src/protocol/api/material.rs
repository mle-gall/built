use super::*;
fn colour(rgb: &Obj<'_>) -> PyResult<Vec<f64>> {
    let float = rgb.py().import("builtins")?.getattr("float")?;
    rgb.try_iter()?
        .take(3)
        .map(|v| float.call1((v?,))?.extract())
        .collect()
}
#[pyfunction]
fn colour_hex(rgb: &Obj<'_>) -> PyResult<String> {
    let mut out = String::from("#");
    for c in colour(rgb)? {
        out.push_str(&format!(
            "{:02X}",
            (1_f64.min(c).max(0.) * 255.).round_ties_even() as u8
        ));
    }
    if out.len() != 7 {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "expected three colour channels",
        ));
    }
    Ok(out)
}
#[pyfunction]
fn colour_name(rgb: &Obj<'_>) -> PyResult<String> {
    Ok(format!("Colour {}", colour_hex(rgb)?))
}
fn bare<'p>(t: Option<&Obj<'p>>, py: Python<'p>) -> PyResult<Obj<'p>> {
    match t {
        Some(t) if t.is_truthy()? => {
            let d = py.get_type::<PyDict>().call1((t,))?;
            d.call_method1("pop", ("uvw", py.None()))?;
            Ok(d)
        }
        _ => Ok(none(py)),
    }
}
#[pyfunction(signature=(materials,color=None,texture=None,opacity=None))]
fn find_material<'p>(
    materials: &Obj<'p>,
    color: Option<&Obj<'p>>,
    texture: Option<&Obj<'p>>,
    opacity: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = materials.py();
    let tex = bare(texture, py)?;
    for pair in otherwise(materials.clone(), dict(py))?
        .call_method0("items")?
        .try_iter()?
    {
        let pair = pair?;
        let m = pair.get_item(1)?;
        if !m.getattr("pbr")?.is_none() || !m.getattr("opacity")?.eq(opacity)? {
            continue;
        }
        let mt = m.getattr("texture")?;
        let same = if !tex.is_none() {
            mt.is_truthy()? && bare(Some(&mt), py)?.eq(&tex)?
        } else if !mt.is_truthy()? {
            let a = m.getattr("color")?;
            match color {
                Some(b) if !a.is_none() => colour(&a)?
                    .iter()
                    .zip(colour(b)?)
                    .all(|(x, y)| (x - y).abs() < 0.5 / 255.),
                _ => false,
            }
        } else {
            false
        };
        if same {
            return pair.get_item(0);
        }
    }
    Ok(none(py))
}
#[pyfunction]
fn fresh_name(materials: &Obj<'_>, base: &str) -> PyResult<String> {
    let mut n = 2;
    let mut name = base.to_owned();
    while materials.is_truthy()? && materials.contains(&name)? {
        name = format!("{base} ({n})");
        n += 1
    }
    Ok(name)
}
#[pyfunction(signature=(scene,color=None,texture=None,opacity=None))]
fn material_for_paint<'p>(
    scene: &Obj<'p>,
    color: Option<&Obj<'p>>,
    texture: Option<&Obj<'p>>,
    opacity: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let mats = scene.getattr_opt("materials")?.unwrap_or(dict(py));
    let found = find_material(&mats, color, texture, opacity)?;
    if !found.is_none() {
        return mats.get_item(found);
    }
    let tex = bare(texture, py)?;
    let kw = PyDict::new(py);
    kw.set_item("opacity", opacity)?;
    let base = if !tex.is_none() {
        let stem = py
            .import("pathlib")?
            .call_method1(
                "Path",
                (tex.call_method1("get", ("path", "Image"))?.str()?,),
            )?
            .getattr("stem")?
            .call_method0("strip")?;
        kw.set_item("texture", tex)?;
        if stem.is_truthy()? {
            stem.extract()?
        } else {
            "Image".into()
        }
    } else {
        let rgb = match color {
            Some(c) if c.is_truthy()? => colour(c)?,
            _ => vec![0.8; 3],
        };
        let rgb = PyTuple::new(py, rgb)?;
        kw.set_item("color", &rgb)?;
        colour_name(rgb.as_any())?
    };
    py.import("built_core.materials")?
        .call_method("Material", (fresh_name(&mats, &base)?,), Some(&kw))
}
fn anonymous<'p>(
    out: &Obj<'p>,
    paint: &Obj<'p>,
    item: &Obj<'p>,
    face: Option<&str>,
) -> PyResult<()> {
    if !paint.is_instance_of::<PyDict>() {
        return Ok(());
    }
    let c = get(paint, "color")?;
    if c.is_none() || get(paint, "mat")?.is_truthy()? || get(paint, "texture")?.is_truthy()? {
        return Ok(());
    }
    let py = out.py();
    let opacity = get(paint, "opacity")?;
    let mut key = colour_hex(&c)?;
    if !opacity.is_none() {
        let val = py.import("builtins")?.call_method1("float", (&opacity,))?;
        key.push('@');
        key.push_str(
            &val.call_method1("__format__", (".2f",))?
                .extract::<String>()?,
        );
    }
    let row = if out.contains(&key)? {
        out.get_item(key)?
    } else {
        let d = dict(py);
        d.set_item("color", PyTuple::new(py, colour(&c)?)?)?;
        d.set_item("opacity", opacity)?;
        d.set_item("faces", list(py))?;
        d.set_item("groups", list(py))?;
        out.set_item(key, &d)?;
        d
    };
    if let Some(side) = face {
        row.get_item("faces")?
            .call_method1("append", ((item, side),))?;
    } else {
        row.get_item("groups")?.call_method1("append", (item,))?;
    }
    Ok(())
}
#[pyfunction]
fn unnamed_colours<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let out = dict(py);
    let core = py.import("built_core")?;
    for mesh in core
        .call_method1("purge_iter_meshes", (scene,))?
        .try_iter()?
    {
        for f in mesh?.getattr("faces")?.try_iter()? {
            let f = f?;
            let a = f.getattr("attrs")?;
            anonymous(&out, &a, &f, Some("front"))?;
            anonymous(&out, &get(&a, "back")?, &f, Some("back"))?
        }
    }
    for top in core
        .call_method1("purge_document_groups", (scene,))?
        .try_iter()?
    {
        for pair in py
            .import("core.group")?
            .call_method1("iter_placements", (top?,))?
            .try_iter()?
        {
            let g = pair?.get_item(0)?;
            anonymous(
                &out,
                &g.getattr_opt("material")?.unwrap_or(none(py)),
                &g,
                None,
            )?;
        }
    }
    Ok(out)
}
fn touch(scene: &Obj<'_>) -> PyResult<()> {
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}
#[pyfunction]
fn add_init(this: &Obj<'_>, material: &Obj<'_>) -> PyResult<()> {
    this.setattr("material", material)
}
#[pyfunction]
fn add_do(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let m = this.getattr("material")?;
    let n = m.getattr("name")?;
    let mats = scene.getattr("materials")?;
    if mats.contains(&n)? {
        return Err(PyValueError::new_err(format!(
            "there is already a material called {}",
            n.repr()?
        )));
    }
    mats.set_item(n, m)?;
    touch(scene)
}
#[pyfunction]
fn add_undo(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    scene.getattr("materials")?.call_method1(
        "pop",
        (
            this.getattr("material")?.getattr("name")?,
            scene.py().None(),
        ),
    )?;
    touch(scene)
}
#[pyfunction]
fn name_init(this: &Obj<'_>) -> PyResult<()> {
    this.setattr("_plan", this.py().None())?;
    this.setattr("_added", list(this.py()))
}
#[pyfunction]
fn survey(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let materials = py.get_type::<PyDict>().call1((otherwise(
        scene.getattr_opt("materials")?.unwrap_or(none(py)),
        dict(py),
    )?,))?;
    let plan = PyList::empty(py);
    for e in unnamed_colours(scene)?.call_method0("values")?.try_iter()? {
        let e = e?;
        let rgb = e.get_item("color")?;
        let op = e.get_item("opacity")?;
        let mut found = find_material(&materials, Some(&rgb), None, Some(&op))?;
        let mut mat = none(py);
        if found.is_none() {
            found = fresh_name(&materials, &colour_name(&rgb)?)?
                .into_pyobject(py)?
                .into_any();
            let kw = PyDict::new(py);
            kw.set_item("color", rgb)?;
            kw.set_item("opacity", op)?;
            mat = py
                .import("built_core.materials")?
                .call_method("Material", (&found,), Some(&kw))?;
            materials.set_item(&found, &mat)?
        }
        plan.append((found, mat, e.get_item("faces")?, e.get_item("groups")?))?
    }
    this.setattr("_plan", plan)
}
#[pyfunction]
fn geom<'p>(_this: &Obj<'p>, scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    py.get_type::<PyList>().call1((py
        .import("built_core")?
        .call_method1("purge_iter_meshes", (scene,))?,))
}
#[pyfunction]
fn count(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<usize> {
    if this.getattr("_plan")?.is_none() {
        survey(this, scene)?
    }
    this.getattr("_plan")?.len()
}
fn stamp(d: &Obj<'_>, name: &Obj<'_>, undo: bool) -> PyResult<()> {
    if undo {
        d.call_method1("pop", ("mat", d.py().None()))?;
    } else {
        d.set_item("mat", name)?
    }
    Ok(())
}
fn apply(this: &Obj<'_>, scene: &Obj<'_>, undo: bool) -> PyResult<()> {
    let py = scene.py();
    let mats = scene.getattr("materials")?;
    if !undo {
        count(this, scene)?;
        this.setattr("_added", list(py))?
    }
    let added = this.getattr("_added")?;
    for row in otherwise(this.getattr("_plan")?, list(py))?.try_iter()? {
        let row = row?;
        let name = row.get_item(0)?;
        let mat = row.get_item(1)?;
        if !undo && !mat.is_none() && !mats.contains(&name)? {
            mats.set_item(&name, mat)?;
            added.call_method1("append", (&name,))?;
        }
        for pair in row.get_item(2)?.try_iter()? {
            let pair = pair?;
            let attrs = pair.get_item(0)?.getattr("attrs")?;
            if pair.get_item(1)?.eq("front")? {
                stamp(&attrs, &name, undo)?
            } else {
                let back = get(&attrs, "back")?;
                if back.is_instance_of::<PyDict>() {
                    let back = py.get_type::<PyDict>().call1((back,))?;
                    stamp(&back, &name, undo)?;
                    attrs.set_item("back", back)?
                }
            }
        }
        for g in row.get_item(3)?.try_iter()? {
            let g = g?;
            let paint = g.getattr("material")?;
            if paint.is_instance_of::<PyDict>() {
                let d = py.get_type::<PyDict>().call1((paint,))?;
                stamp(&d, &name, undo)?;
                g.setattr("material", d)?
            }
        }
    }
    if undo {
        for n in added.try_iter()? {
            mats.call_method1("pop", (n?, py.None()))?;
        }
        this.setattr("_added", list(py))?
    }
    touch(scene)
}
#[pyfunction]
fn name_do(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    apply(this, scene, false)
}
#[pyfunction]
fn name_undo(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    apply(this, scene, true)
}
pub(super) fn install(m: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    let h = py.import("built_core.history")?;
    m.add("DeleteMaterialCommand", h.getattr("DeleteMaterialCommand")?)?;
    let ns = PyDict::new(py);
    ns.set_item("doc_undo", true)?;
    meth!(ns, m, "__init__", add_init);
    meth!(ns, m, "do", add_do);
    meth!(ns, m, "undo", add_undo);
    class(m, "AddMaterialCommand", vec![h.getattr("Command")?], &ns)?;
    let ns = PyDict::new(py);
    meth!(ns, m, "__init__", name_init);
    meth!(ns, m, "do", name_do);
    meth!(ns, m, "undo", name_undo);
    meth!(ns, m, "count", count);
    meth!(ns, m, "geometry_meshes", geom);
    class(
        m,
        "NameColoursCommand",
        vec![h.getattr("MeshCommand")?],
        &ns,
    )?;
    let vals = ("metallic", "roughness", "normal_scale", "ao_strength").into_pyobject(py)?;
    let maps = ("metallic_map", "roughness_map", "normal_map", "ao_map").into_pyobject(py)?;
    m.add("PBR_KEYS", vals.as_any().add(&maps)?)?;
    m.add("PBR_VALUES", vals)?;
    m.add("PBR_MAPS", maps)?;
    m.add_function(wrap_pyfunction!(colour_hex, m)?)?;
    m.add_function(wrap_pyfunction!(colour_name, m)?)?;
    m.add_function(wrap_pyfunction!(find_material, m)?)?;
    m.add_function(wrap_pyfunction!(fresh_name, m)?)?;
    m.add_function(wrap_pyfunction!(material_for_paint, m)?)?;
    m.add_function(wrap_pyfunction!(unnamed_colours, m)?)
}
