// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered document edits used by annotation, section, view and sheet commands.
use crate::math::{xyz, Vec3};
use pyo3::{prelude::*, types::PyDict};
type Obj<'py> = Bound<'py, PyAny>;
type Indexed<'py> = (usize, Obj<'py>);

/// Restore the occurrence filter and tint saved with a camera, or clear an
/// active navigator display when recalling an ordinary scene.
#[pyfunction]
pub(crate) fn recall_scene_display(vp: &Obj<'_>, view: &Obj<'_>) -> PyResult<()> {
    let py = vp.py();
    let scene = vp.getattr("scene")?;
    let key = view.getattr("navigator_view")?;
    let options = PyDict::new(py);
    options.set_item("remember", false)?;
    if !key.is_none() {
        let registry = py
            .import("core.bim_registry")?
            .call_method1("get", (&scene,))?;
        let saved = registry
            .call_method1("get", ("views", PyDict::new(py)))?
            .call_method1("get", (&key,))?;
        if !saved.is_none() {
            options.set_item("key", key)?;
            options.set_item("exact", saved.call_method1("get", ("exact", false))?)?;
            options.set_item("isolate", saved.call_method1("get", ("isolate", false))?)?;
            let expr = saved.get_item("filter")?;
            let color = saved.call_method1("get", ("color_by",))?;
            py.import("core.agent_facets")?.call_method(
                "set_display",
                (vp, expr, color),
                Some(&options),
            )?;
            return Ok(());
        }
    }
    let active = |obj: &Obj<'_>, field| -> PyResult<bool> {
        Ok(obj.hasattr(field)? && !obj.getattr(field)?.is_none())
    };
    if active(&scene, "_navigator_allowed")?
        || active(vp, "active_view_key")?
        || active(vp, "view_tint")?
    {
        py.import("core.agent_facets")?
            .call_method("set_display", (vp,), Some(&options))?;
    }
    Ok(())
}

/// Saved views and their folders are persistent edits. They advance the
/// document version without invalidating every other saved camera's picture.
#[pyfunction(signature = (scene, changed_view=None))]
pub(crate) fn bump_scene_catalog(scene: &Obj<'_>, changed_view: Option<&Obj<'_>>) -> PyResult<()> {
    let version: i64 = scene.getattr("version")?.extract()?;
    let catalog: i64 = scene.getattr("_scene_catalog_ticks")?.extract()?;
    scene.setattr("version", version + 1)?;
    scene.setattr("_scene_catalog_ticks", catalog + 1)?;
    if let Some(view) = changed_view {
        view.setattr("_photo_tick", catalog + 1)?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn add_item(list: &Obj<'_>, item: &Obj<'_>) -> PyResult<()> {
    list.call_method1("append", (item,))?;
    Ok(())
}
#[pyfunction]
pub(crate) fn remove_item(list: &Obj<'_>, item: &Obj<'_>) -> PyResult<()> {
    if list.contains(item)? {
        list.call_method1("remove", (item,))?;
    }
    Ok(())
}

#[pyfunction(signature=(list, items, discard, strict=true))]
pub(crate) fn delete_items<'py>(
    list: &Obj<'py>,
    items: Vec<Obj<'py>>,
    discard: Option<&Obj<'py>>,
    strict: bool,
) -> PyResult<Vec<Indexed<'py>>> {
    let mut before = Vec::new();
    for item in &items {
        if list.contains(item)? {
            before.push((
                list.call_method1("index", (item,))?.extract()?,
                item.clone(),
            ));
        }
    }
    if let Some(discard) = discard {
        for item in items {
            remove_item(list, &item)?;
            discard.call1((item,))?;
        }
    } else if !strict {
        for item in items {
            remove_item(list, &item)?;
        }
    } else {
        // Saved views remove every occurrence recorded in the first pass.
        for (_, item) in &before {
            list.call_method1("remove", (item,))?;
        }
    }
    Ok(before)
}
#[pyfunction]
pub(crate) fn translate_items(items: Vec<Obj<'_>>, field: &str, step: &Obj<'_>) -> PyResult<()> {
    let step = xyz(step)?;
    for item in items {
        item.setattr(field, Vec3::from(xyz(&item.getattr(field)?)? + step))?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn shift_guide(guide: &Obj<'_>, step: &Obj<'_>) -> PyResult<()> {
    let delta = xyz(step)?;
    guide.setattr("point", Vec3::from(xyz(&guide.getattr("point")?)? + delta))?;
    let origin = guide.getattr("origin")?;
    if !origin.is_none() {
        guide.setattr("origin", Vec3::from(xyz(&origin)? + delta))?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn shift_dimension(
    dim: &Obj<'_>,
    line: bool,
    step: &Obj<'_>,
    free_a: bool,
    free_b: bool,
) -> PyResult<()> {
    let mut delta = xyz(step)?;
    if line {
        let ab = xyz(&dim.getattr("b")?)? - xyz(&dim.getattr("a")?)?;
        let length = ab.length();
        if length > 1e-9 {
            let dir = ab / length;
            delta -= dir * delta.dot(dir);
        }
        dim.setattr("offset", Vec3::from(xyz(&dim.getattr("offset")?)? + delta))?;
    } else {
        for (field, free) in [("a", free_a), ("b", free_b)] {
            if free {
                dim.setattr(field, Vec3::from(xyz(&dim.getattr(field)?)? + delta))?;
            }
        }
    }
    Ok(())
}

type GuideState<'py> = (Obj<'py>, Vec3, Option<Vec3>, Option<Vec3>);
#[pyfunction]
pub(crate) fn guide_state<'py>(guide: &Obj<'py>) -> PyResult<GuideState<'py>> {
    let vector = |field| -> PyResult<Option<Vec3>> {
        let v = guide.getattr(field)?;
        if v.is_none() {
            Ok(None)
        } else {
            Ok(Some(Vec3::from(xyz(&v)?)))
        }
    };
    Ok((
        guide.clone(),
        Vec3::from(xyz(&guide.getattr("point")?)?),
        vector("direction")?,
        vector("origin")?,
    ))
}
#[pyfunction]
pub(crate) fn place_guide(state: &Obj<'_>, matrix: Option<&Obj<'_>>) -> PyResult<()> {
    let guide = state.get_item(0)?;
    let map = |v: &Obj<'_>| -> PyResult<Vec3> {
        Ok(Vec3::from(if let Some(m) = matrix {
            xyz(&m.call_method1("map", (v,))?)?
        } else {
            xyz(v)?
        }))
    };
    guide.setattr("point", map(&state.get_item(1)?)?)?;
    let direction = state.get_item(2)?;
    if !direction.is_none() {
        let d = if let Some(m) = matrix {
            xyz(&m.call_method1("mapVector", (&direction,))?)?
        } else {
            xyz(&direction)?
        };
        guide.setattr(
            "direction",
            Vec3::from(if d.length() > 1e-12 {
                d.normalize()
            } else {
                xyz(&direction)?
            }),
        )?;
    }
    let origin = state.get_item(3)?;
    if !origin.is_none() {
        guide.setattr("origin", map(&origin)?)?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn rotate_sections(planes: Vec<Obj<'_>>, matrix: &Obj<'_>) -> PyResult<()> {
    for p in planes {
        p.setattr("point", matrix.call_method1("map", (p.getattr("point")?,))?)?;
        let n = xyz(&matrix.call_method1("mapVector", (p.getattr("normal")?,))?)?;
        if n.length() > 1e-12 {
            p.setattr("normal", Vec3::from(n.normalize()))?;
        }
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn restore_active(scene: &Obj<'_>, active: &Obj<'_>) -> PyResult<()> {
    let member = scene.getattr("section_planes")?.contains(active)?;
    if member {
        scene.call_method1("set_active_section", (active,))?;
    } else {
        scene.call_method1("set_active_section", (scene.py().None(),))?;
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn activate_section<'py>(scene: &Obj<'py>, plane: &Obj<'py>, place: bool) -> PyResult<Obj<'py>> {
    let old = scene.call_method0("active_section")?;
    if place {
        add_item(&scene.getattr("section_planes")?, plane)?;
    }
    scene.call_method1("set_active_section", (plane,))?;
    Ok(old)
}
#[pyfunction]
pub(crate) fn section_mode<'py>(
    scene: &Obj<'py>,
    plane: &Obj<'py>,
    cut: bool,
    frame: bool,
) -> PyResult<(Obj<'py>, bool)> {
    let old = (
        scene.call_method0("active_section")?,
        plane.getattr("hidden")?.is_truthy()?,
    );
    if cut {
        scene.call_method1("set_active_section", (plane,))?;
    } else if plane.getattr("active")?.is_truthy()? {
        scene.call_method1("set_active_section", (scene.py().None(),))?;
    }
    plane.setattr("hidden", !frame)?;
    Ok(old)
}
#[pyfunction]
pub(crate) fn set_section_label<'py>(
    plane: &Obj<'py>,
    name: &Obj<'py>,
    symbol: &Obj<'py>,
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let old = (plane.getattr("name")?, plane.getattr("symbol")?);
    plane.setattr("name", name)?;
    plane.setattr("symbol", symbol)?;
    Ok(old)
}
#[pyfunction]
pub(crate) fn set_section_fill<'py>(plane: &Obj<'py>, fill: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = plane.py();
    let copied = |v: Obj<'py>| -> PyResult<Obj<'py>> {
        if v.is_truthy()? {
            py.get_type::<PyDict>().call1((v,))
        } else {
            Ok(py.None().into_bound(py))
        }
    };
    let old = copied(plane.getattr("fill")?)?;
    plane.setattr("fill", copied(fill.clone())?)?;
    Ok(old)
}
#[pyfunction]
pub(crate) fn make_plan<'py>(
    scene: &Obj<'py>,
    plane: &Obj<'py>,
    view: &Obj<'py>,
    sheet: &Obj<'py>,
    add_plane: bool,
) -> PyResult<Obj<'py>> {
    let old = activate_section(scene, plane, add_plane)?;
    add_item(&scene.getattr("saved_views")?, view)?;
    if !sheet.is_none() {
        add_item(&scene.getattr("compositions")?, sheet)?;
    }
    Ok(old)
}
#[pyfunction]
pub(crate) fn insert_scene(scene: &Obj<'_>, item: &Obj<'_>, at: &Obj<'_>) -> PyResult<()> {
    let views = scene.getattr("saved_views")?;
    let n = views.len()?;
    let index = if at.is_none() || at.gt(n)? {
        n
    } else if at.lt(0)? {
        0
    } else {
        at.extract()?
    };
    views.call_method1("insert", (index, item))?;
    scene
        .py()
        .import("built_core.scene_tree")?
        .call_method1("sync_order", (scene,))?;
    Ok(())
}
#[pyfunction]
pub(crate) fn move_item(
    list: &Obj<'_>,
    item: &Obj<'_>,
    index: &Obj<'_>,
    clamp: bool,
    reject_unchanged: bool,
) -> PyResult<Option<usize>> {
    if reject_unchanged && !list.contains(item)? {
        return Ok(None);
    }
    let old: usize = list.call_method1("index", (item,))?.extract()?;
    if reject_unchanged && index.eq(old)? {
        return Ok(None);
    }
    list.call_method1("remove", (item,))?;
    if clamp {
        let n = list.len()?;
        let at = if index.lt(0)? {
            0
        } else if index.gt(n)? {
            n
        } else {
            index.extract()?
        };
        list.call_method1("insert", (at, item))?;
    } else {
        list.call_method1("insert", (index, item))?;
    }
    Ok(Some(old))
}
#[pyfunction]
pub(crate) fn rename_view<'py>(
    scene: &Obj<'py>,
    view: &Obj<'py>,
    name: &Obj<'py>,
) -> PyResult<Option<(Obj<'py>, Vec<Obj<'py>>)>> {
    let py = scene.py();
    let old = view.getattr("name")?;
    if !name.is_truthy()? {
        return Ok(None);
    }
    let name = name.call_method0("strip")?;
    if !name.is_truthy()? || name.eq(&old)? {
        return Ok(None);
    }
    for v in scene.getattr("saved_views")?.try_iter()? {
        let v = v?;
        if !v.is(view) && v.getattr("name")?.eq(&name)? {
            return Ok(None);
        }
    }
    let frames = py
        .import("core.outliner")?
        .call_method1("frames_showing", (scene, &old))?;
    let new_key = format!("scene:{}", name.str()?);
    let mut touched = Vec::new();
    for pair in frames.try_iter()? {
        let frame = pair?.get_item(1)?;
        frame.setattr("view_key", &new_key)?;
        touched.push(frame);
    }
    view.setattr("name", name)?;
    Ok(Some((old, touched)))
}
#[pyfunction]
pub(crate) fn update_scene<'py>(
    scene: &Obj<'py>,
    view: &Obj<'py>,
    camera: &Obj<'py>,
    style: &Obj<'py>,
    after: &Obj<'py>,
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let capture = || -> PyResult<Obj<'py>> {
        let d = PyDict::new(scene.py());
        for k in view.getattr("_record_fields")?.try_iter()? {
            let k = k?;
            let field: String = k.extract()?;
            d.set_item(k, view.getattr(field.as_str())?)?;
        }
        Ok(d.into_any())
    };
    let before = capture()?;
    if after.is_none() {
        view.call_method1("recapture", (scene, camera, style))?;
        Ok((before, capture()?))
    } else {
        restore_fields(view, after)?;
        Ok((before, after.clone()))
    }
}
#[pyfunction]
pub(crate) fn restore_fields(o: &Obj<'_>, values: &Obj<'_>) -> PyResult<()> {
    for kv in values.call_method0("items")?.try_iter()? {
        let kv = kv?;
        let name: String = kv.get_item(0)?.extract()?;
        o.setattr(name.as_str(), kv.get_item(1)?)?;
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn edit_view_tree<'py>(
    scene: &Obj<'py>,
    change: &Obj<'py>,
    before: &Obj<'py>,
    after: &Obj<'py>,
) -> PyResult<Option<(Obj<'py>, Obj<'py>)>> {
    if !after.is_none() {
        crate::scene_tree::restore(scene, after)?;
        return Ok(Some((before.clone(), after.clone())));
    }
    let initial = crate::scene_tree::state(scene)?;
    if !change.call1((scene,))?.is_truthy()? {
        crate::scene_tree::restore(scene, &initial)?;
        return Ok(None);
    }
    Ok(Some((initial, crate::scene_tree::state(scene)?)))
}
#[pyfunction]
pub(crate) fn set_direction<'py>(guide: &Obj<'py>, direction: &Obj<'py>) -> PyResult<Obj<'py>> {
    let old = guide.getattr("direction")?;
    guide.setattr("direction", Vec3::from(xyz(direction)?))?;
    Ok(old)
}
#[pyfunction]
pub(crate) fn set_text(o: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    o.setattr("text", value)
}
/// Use the sheet codec's item table for subclasses as well as base items.
#[pyfunction]
pub(crate) fn sheet_container<'py>(sheet: &Obj<'py>, item: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let classes = sheet.py().import("core.composition")?;
    for &(field, kind) in crate::file_sheet::ITEMS {
        if item.is_instance(&classes.getattr(kind)?)? {
            return Ok(Some(sheet.getattr(field)?));
        }
    }
    Ok(None)
}

#[pyfunction]
pub(crate) fn append_sheet_item<'py>(
    sheet: &Obj<'py>,
    item: &Obj<'py>,
    container: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    if !container.is_none() {
        container.call_method1("append", (item,))?;
        Ok(None)
    } else {
        let previous = sheet.getattr("cajetin")?;
        sheet.setattr("cajetin", item)?;
        Ok(Some(previous))
    }
}
#[pyfunction]
pub(crate) fn remove_sheet_item(
    sheet: &Obj<'_>,
    item: &Obj<'_>,
    container: &Obj<'_>,
    replacement: &Obj<'_>,
) -> PyResult<()> {
    if !container.is_none() {
        container.call_method1("remove", (item,))?;
    } else {
        sheet.setattr("cajetin", replacement)?;
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn capture_sheet_fields<'py>(
    item: &Obj<'py>,
    changes: &Obj<'py>,
    before: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    if !before.is_none() {
        return item.py().get_type::<PyDict>().call1((before,));
    }
    let saved = PyDict::new(item.py());
    for key in changes.try_iter()? {
        let key = key?;
        let field = key.extract::<String>()?;
        saved.set_item(key, item.getattr(field.as_str())?)?;
    }
    Ok(saved.into_any())
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "presentation")?;
    sub.add_function(wrap_pyfunction!(recall_scene_display, &sub)?)?;
    sub.add_function(wrap_pyfunction!(bump_scene_catalog, &sub)?)?;
    sub.add_function(wrap_pyfunction!(sheet_container, &sub)?)?;
    sub.add_function(wrap_pyfunction!(append_sheet_item, &sub)?)?;
    sub.add_function(wrap_pyfunction!(remove_sheet_item, &sub)?)?;
    sub.add_function(wrap_pyfunction!(capture_sheet_fields, &sub)?)?;
    sub.add_function(wrap_pyfunction!(edit_view_tree, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_direction, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_text, &sub)?)?;
    sub.add_function(wrap_pyfunction!(add_item, &sub)?)?;
    sub.add_function(wrap_pyfunction!(remove_item, &sub)?)?;
    sub.add_function(wrap_pyfunction!(delete_items, &sub)?)?;
    sub.add_function(wrap_pyfunction!(translate_items, &sub)?)?;
    sub.add_function(wrap_pyfunction!(shift_guide, &sub)?)?;
    sub.add_function(wrap_pyfunction!(shift_dimension, &sub)?)?;
    sub.add_function(wrap_pyfunction!(guide_state, &sub)?)?;
    sub.add_function(wrap_pyfunction!(place_guide, &sub)?)?;
    sub.add_function(wrap_pyfunction!(rotate_sections, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_active, &sub)?)?;
    sub.add_function(wrap_pyfunction!(activate_section, &sub)?)?;
    sub.add_function(wrap_pyfunction!(section_mode, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_section_label, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_section_fill, &sub)?)?;
    sub.add_function(wrap_pyfunction!(make_plan, &sub)?)?;
    sub.add_function(wrap_pyfunction!(insert_scene, &sub)?)?;
    sub.add_function(wrap_pyfunction!(move_item, &sub)?)?;
    sub.add_function(wrap_pyfunction!(rename_view, &sub)?)?;
    sub.add_function(wrap_pyfunction!(update_scene, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_fields, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}
