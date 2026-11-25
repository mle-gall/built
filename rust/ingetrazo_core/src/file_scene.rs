// SPDX-License-Identifier: GPL-3.0-or-later
//! Scene-level IGZ/Built records in archive order. Model classes supply
//! their own record conversion and remain the authority on editing defaults.
use pyo3::{
    exceptions::{PyAttributeError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyString, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
type Dict<'py> = Bound<'py, PyDict>;
fn attr<'py>(obj: &Obj<'py>, key: &str) -> PyResult<Obj<'py>> {
    match obj.getattr(key) {
        Err(e) if e.is_instance_of::<PyAttributeError>(obj.py()) => {
            Ok(obj.py().None().into_bound(obj.py()))
        }
        r => r,
    }
}
fn flag(obj: &Obj<'_>, key: &str, default: bool) -> PyResult<bool> {
    if obj.hasattr(key)? {
        obj.getattr(key)?.is_truthy()
    } else {
        Ok(default)
    }
}
fn list<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.py().get_type::<PyList>().call1((v,))
}
fn rows<'py>(
    seq: &Obj<'py>,
    mut convert: impl FnMut(&Obj<'py>) -> PyResult<Obj<'py>>,
) -> PyResult<Bound<'py, PyList>> {
    let result = PyList::empty(seq.py());
    for obj in seq.try_iter()? {
        result.append(convert(&obj?)?)?;
    }
    Ok(result)
}
fn record<'py>(obj: &Obj<'py>) -> PyResult<Obj<'py>> {
    obj.call_method0("to_dict")
}
fn put(out: &Dict<'_>, key: &str, v: &Obj<'_>) -> PyResult<()> {
    if v.is_truthy()? {
        out.set_item(key, v)?;
    }
    Ok(())
}
fn archive_folders(scene: &Obj<'_>, out: &Dict<'_>, layers: bool) -> PyResult<()> {
    let py = scene.py();
    let module = py.import(if layers {
        "core.layers"
    } else {
        "ingetrazo_core.scene_tree"
    })?;
    let folders = module.call_method1("folders_of", (scene,))?;
    if layers {
        out.set_item(
            "layer_folder_uids",
            scene.getattr("layer_folder_uids")?.call_method0("copy")?,
        )?;
    }
    if folders.is_truthy()? {
        let keys = if layers {
            ("layer_folders", "layer_folder_parents")
        } else {
            ("scene_folders", "scene_folder_parents")
        };
        out.set_item(keys.0, list(&folders)?)?;
        let parents = PyDict::new(py);
        for f in folders.try_iter()? {
            let f = f?;
            let parent = module.call_method1("folder_parent", (scene, &f))?;
            if parent.is_truthy()? {
                parents.set_item(f, parent)?;
            }
        }
        put(out, keys.1, &parents)?;
    }
    if layers {
        let vis = attr(scene, "folder_visibility")?;
        out.set_item(
            "layer_folder_visibility",
            if vis.is_none() {
                PyDict::new(py).into_any()
            } else {
                py.get_type::<PyDict>().call1((vis,))?
            },
        )?;
        let labels = PyDict::new(py);
        if let Ok(items) = attr(scene, "layer_folder_names")?.cast::<PyDict>() {
            for (key, v) in items {
                if folders.contains(&key)?
                    && v.is_instance_of::<PyString>()
                    && v.is_truthy()?
                    && !v.eq(&key)?
                {
                    labels.set_item(key, v)?;
                }
            }
        }
        put(out, "layer_folder_names", &labels)?;
    }
    Ok(())
}
fn archive_styles(scene: &Obj<'_>, out: &Dict<'_>) -> PyResult<()> {
    let py = scene.py();
    let layers = attr(scene, "layers")?;
    if !layers.is_none() {
        let class = py.import("core.layers")?.getattr("Layer")?;
        out.set_item(
            "layers",
            rows(&layers, |ly| {
                if ly.get_type().is(&class) {
                    Ok(crate::file_styles::igz_archive_layer(ly)?.into_any())
                } else {
                    record(ly)
                }
            })?,
        )?;
    }
    put(out, "active_layer", &attr(scene, "active_layer")?)?;
    archive_folders(scene, out, true)?;
    let kinds = attr(scene, "line_types")?;
    if kinds.is_truthy()? {
        let all = PyDict::new(py);
        for (name, v) in kinds.cast::<PyDict>()? {
            let Ok(e) = v.cast::<PyDict>() else { continue };
            let Some(pattern) = e.get_item("pattern")? else {
                continue;
            };
            if !pattern.is_truthy()? {
                continue;
            }
            let r = PyDict::new(py);
            r.set_item(
                "label",
                e.get_item("label")?.unwrap_or_else(|| name.clone()),
            )?;
            r.set_item("pattern", list(&pattern)?)?;
            for k in ["text", "text_size", "text_at", "pieces", "color"] {
                if let Some(value) = e.get_item(k)? {
                    r.set_item(k, if k == "color" { list(&value)? } else { value })?;
                }
            }
            all.set_item(name, r)?;
        }
        out.set_item("line_types", all)?;
    }
    let round = py.import("builtins")?.getattr("round")?;
    for (key, n) in [("linetype_colors", 4), ("bricks", 2)] {
        let v = attr(scene, key)?;
        if v.is_truthy()? {
            let all = PyDict::new(py);
            for (k, cs) in v.cast::<PyDict>()? {
                all.set_item(
                    k,
                    rows(&cs, |c| round.call1((c.call_method0("__float__")?, n)))?,
                )?;
            }
            out.set_item(key, all)?;
        }
    }
    let patterns = attr(scene, "hatch_patterns")?;
    if patterns.is_truthy()? {
        let all = PyDict::new(py);
        for (name, v) in patterns.cast::<PyDict>()? {
            if let Ok(p) = v.cast::<PyDict>() {
                let row = PyDict::new(py);
                row.set_item(
                    "label",
                    p.get_item("label")?.unwrap_or_else(|| name.clone()),
                )?;
                row.set_item(
                    "families",
                    p.get_item("families")?
                        .unwrap_or_else(|| PyList::empty(py).into_any()),
                )?;
                all.set_item(name, row)?;
            }
        }
        out.set_item("hatch_patterns", all)?;
    }
    Ok(())
}

fn archive_settings(scene: &Obj<'_>, out: &Dict<'_>, stream: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let codec = py.import("formats.igz")?;
    let style = attr(scene, "dimension_style")?;
    if style.is_truthy()? {
        out.set_item("dimension_style", style.call_method0("copy")?)?;
    }
    for (source, target) in [("camera_home", "camera"), ("views_home", "views")] {
        if let Ok(v) = attr(scene, source)?.cast::<PyDict>() {
            if codec.call_method1("_non_finite_where", (v,))?.eq("scene")? {
                let copy = v.copy()?;
                if target == "camera" && !flag(stream, "built", false)? {
                    for k in ["target", "up"] {
                        if let Some(point) = copy.get_item(k)? {
                            if (point.is_instance_of::<PyList>()
                                || point.is_instance_of::<PyTuple>())
                                && point.len()? == 3
                            {
                                copy.set_item(
                                    k,
                                    py.import("formats.built")?
                                        .call_method1("compact_values", (point,))?,
                                )?;
                            }
                        }
                    }
                }
                out.set_item(target, copy)?;
            }
        }
    }
    if let Ok(v) = attr(scene, "units")?.cast::<PyDict>() {
        if v.len() != 2
            || !v.call_method1("get", ("length",))?.eq("m")?
            || !v.call_method1("get", ("precision",))?.eq(2)?
        {
            out.set_item("units", v.copy()?)?;
        }
    }
    // The links registry is a root table of its own; the file keeps it in
    // plugin_data, where older builds read it.
    let data = PyDict::new(py);
    let plugin = attr(scene, "plugin_data")?;
    if plugin.is_truthy()? {
        data.call_method1("update", (plugin,))?;
    }
    if let Some(reg) = scene.getattr_opt("link_registry")?.filter(|r| r.is_truthy().unwrap_or(false)) {
        data.set_item("links", reg)?;
    }
    if data.is_truthy()? {
        let json = py.import("json")?;
        let keep = PyDict::new(py);
        for (k, v) in data.iter() {
            match json.call_method1("dumps", (v,)) {
                Ok(js) => keep.set_item(k.str()?, json.call_method1("loads", (js,))?)?,
                Err(e)
                    if e.is_instance_of::<PyTypeError>(py)
                        || e.is_instance_of::<PyValueError>(py) =>
                {
                    continue
                }
                Err(e) => return Err(e),
            }
        }
        if flag(stream, "compatibility", false)? {
            if let Some(p) = keep
                .get_item("project")?
                .and_then(|v| v.cast_into::<PyDict>().ok())
            {
                let path = py
                    .import("ingetrazo_core.project")?
                    .call_method1("path_for", (scene,))?;
                if path.is_truthy()? {
                    p.call_method1("pop", ("rel", py.None()))?;
                    p.call_method1(
                        "update",
                        (crate::model::link_io::link_record(&path, Some(&stream.getattr("destination")?))?,),
                    )?;
                }
            }
        }
        put(out, "plugin_data", &keep)?;
    }
    let scales = attr(scene, "custom_scales")?;
    if scales.is_truthy()? {
        out.set_item(
            "custom_scales",
            rows(&scales, |v| {
                py.get_type::<pyo3::types::PyFloat>().call1((v,))
            })?,
        )?;
    }
    let back = attr(scene, "back_face_color")?;
    if back.is_truthy()? {
        out.set_item("back_face_color", list(&back)?)?;
    }
    let planes = attr(scene, "section_planes")?;
    if planes.is_truthy()? {
        out.set_item("section_planes", rows(&planes, record)?)?;
        let vis = PyDict::new(py);
        for (from, to) in [
            ("show_section_planes", "planes_hidden"),
            ("show_section_cuts", "cuts_hidden"),
        ] {
            if !flag(scene, from, true)? {
                vis.set_item(to, true)?;
            }
        }
        put(out, "section_view", &vis)?;
    }
    let vis = PyDict::new(py);
    for (from, to) in [
        ("show_hidden_objects", "objects"),
        ("show_hidden_geometry", "geometry"),
    ] {
        if flag(scene, from, false)? {
            vis.set_item(to, true)?;
        }
    }
    put(out, "hidden_view", &vis)?;
    if !flag(scene, "show_axes", true)? {
        out.set_item("axes_hidden", true)?;
    }
    for (from, to, module, class) in [
        ("display_style", "display_style", "core.style", "Style"),
        ("shadows", "shadows", "core.sun", "ShadowSettings"),
    ] {
        let value = attr(scene, from)?;
        if !value.is_none() {
            let r = record(&value)?;
            if !r.eq(record(&py.import(module)?.getattr(class)?.call0()?)?)? {
                out.set_item(to, r)?;
            }
        }
        if from == "display_style" {
            let north = attr(scene, "north_deg")?;
            if north.is_truthy()? {
                let deg = py.get_type::<pyo3::types::PyFloat>().call1((north,))?;
                if deg.is_truthy()? {
                    out.set_item(
                        "north_deg",
                        py.import("builtins")?.call_method1("round", (deg, 6))?,
                    )?;
                }
            }
        }
    }
    let animation = attr(scene, "animation")?;
    if !animation.is_none() {
        put(out, "scene_animation", &record(&animation)?)?;
    }
    Ok(())
}

#[pyfunction]
pub fn igz_archive_scene_fields(scene: &Obj<'_>, out: &Dict<'_>, stream: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    archive_styles(scene, out)?;
    let views = attr(scene, "saved_views")?;
    if views.is_truthy()? {
        let class = py.import("ingetrazo_core.saved_views")?.getattr("SavedView")?;
        out.set_item(
            "saved_views",
            rows(&views, |v| {
                if v.get_type().is(&class) && v.hasattr("_file_view")? {
                    v.call_method0("_file_view")
                } else {
                    record(v)
                }
            })?,
        )?;
    }
    archive_folders(scene, out, false)?;
    let comps = attr(scene, "compositions")?;
    if comps.is_truthy()? {
        let class = py.import("core.composition")?.getattr("Composicion")?;
        out.set_item(
            "compositions",
            rows(&comps, |c| {
                if c.get_type().is(&class) {
                    Ok(crate::file_sheet::igz_archive_sheet(py, c)?.into_any())
                } else {
                    record(c)
                }
            })?,
        )?;
    }
    let dims = attr(scene, "dimensions")?;
    if dims.is_truthy()? {
        out.set_item(
            "dimensions",
            crate::file_annotations::igz_archive_annotations(py, &dims, false)?,
        )?;
    }
    let mats = attr(scene, "materials")?;
    if mats.is_truthy()? {
        let class = py.import("core.materials")?.getattr("Material")?;
        out.set_item(
            "materials",
            rows(&mats.call_method0("values")?, |m| {
                if m.get_type().is(&class) {
                    Ok(crate::file_styles::igz_archive_material(m)?.into_any())
                } else {
                    record(m)
                }
            })?,
        )?;
    }
    archive_settings(scene, out, stream)?;
    let datum = attr(scene, "georef")?;
    if !datum.is_none() {
        let r = PyDict::new(py);
        r.set_item("datum", record(&datum)?)?;
        out.set_item("georef", r)?;
    }
    let tile = attr(scene, "tile_layer")?;
    if !tile.is_none() {
        out.set_item("tile_layer", record(&tile)?)?;
    }
    for key in ["geo_paths", "geo_points"] {
        let seq = attr(scene, key)?;
        if seq.is_truthy()? {
            out.set_item(key, rows(&seq, record)?)?;
        }
    }
    let signs = attr(scene, "text_placement_anchors")?;
    if signs.is_truthy()? {
        let kept = PyList::empty(py);
        for anchor in signs.try_iter()? {
            let r = anchor?.call_method1("archive", (scene,))?;
            if !r.is_none() {
                kept.append(r)?;
            }
        }
        put(out, "text_placement_anchors", &kept)?;
    }
    let labels = attr(scene, "text_labels")?;
    if labels.is_truthy()? {
        let class = py.import("core.textlabel")?.getattr("TextLabel")?;
        let mut native = true;
        for label in labels.try_iter()? {
            if !label?.get_type().is(&class) {
                native = false;
                break;
            }
        }
        let records = if native {
            crate::file_annotations::igz_archive_annotations(py, &labels, true)?
        } else {
            rows(&labels, record)?
        };
        out.set_item("text_labels", records)?;
    }
    let guides = attr(scene, "guides")?;
    if guides.is_truthy()? {
        let lines = PyList::empty(py);
        let circles = PyList::empty(py);
        for guide in guides.try_iter()? {
            let g = guide?;
            if g.hasattr("is_finite")? && !g.call_method0("is_finite")?.is_truthy()? {
                continue;
            }
            if flag(&g, "is_circle", false)? {
                circles.append(record(&g)?)?;
            } else {
                lines.append(record(&g)?)?;
            }
        }
        put(out, "guides", &lines)?;
        put(out, "guide_circles", &circles)?;
    }
    let images = attr(scene, "image_planes")?;
    if images.is_truthy()? {
        out.set_item("image_planes", rows(&images, record)?)?;
    }
    Ok(())
}

fn data<'py>(payload: &Dict<'py>, key: &str) -> PyResult<Obj<'py>> {
    Ok(payload
        .get_item(key)?
        .unwrap_or_else(|| payload.py().None().into_bound(payload.py())))
}
fn string_list<'py>(payload: &Dict<'py>, key: &str) -> PyResult<Bound<'py, PyList>> {
    let result = PyList::empty(payload.py());
    if let Ok(values) = data(payload, key)?.cast::<PyList>() {
        for v in values {
            if v.is_instance_of::<PyString>() && v.is_truthy()? {
                result.append(v)?;
            }
        }
    }
    Ok(result)
}
fn strings<'py>(payload: &Dict<'py>, key: &str, require_value: bool) -> PyResult<Dict<'py>> {
    let result = PyDict::new(payload.py());
    if let Ok(values) = data(payload, key)?.cast::<PyDict>() {
        for (k, v) in values {
            if k.is_instance_of::<PyString>()
                && v.is_instance_of::<PyString>()
                && (!require_value || v.is_truthy()?)
            {
                result.set_item(k, v)?;
            }
        }
    }
    Ok(result)
}
#[pyfunction]
pub fn igz_reset_scene(scene: &Obj<'_>, payload: &Dict<'_>, survey: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    for k in [
        "mesh",
        "selection",
        "groups",
        "geo_paths",
        "geo_points",
        "text_labels",
        "dimensions",
    ] {
        scene.getattr(k)?.call_method0("clear")?;
    }
    for k in ["georef", "tile_layer", "active_layer", "back_face_color"] {
        scene.setattr(k, py.None())?;
    }
    scene.setattr("photo_mesh", survey)?;
    for k in [
        "layer_folders",
        "scene_folders",
        "section_planes",
        "custom_scales",
    ] {
        scene.setattr(k, PyList::empty(py))?;
    }
    for k in [
        "folder_parents",
        "hatch_patterns",
        "line_types",
        "linetype_colors",
        "bricks",
        "scene_folder_parents",
    ] {
        scene.setattr(k, PyDict::new(py))?;
    }
    for (k, v) in [
        ("show_section_planes", true),
        ("show_section_cuts", true),
        ("show_hidden_objects", false),
        ("show_hidden_geometry", false),
        ("show_axes", true),
    ] {
        scene.setattr(k, v)?;
    }
    let factory = py
        .import("core.scene")?
        .getattr("Scene")?
        .getattr("__dataclass_fields__")?
        .get_item("dimension_style")?
        .getattr("default_factory")?;
    scene.setattr("dimension_style", factory.call0()?)?;
    let module = py.import("core.layers")?;
    let class = module.getattr("Layer")?;
    let default = module.getattr("DEFAULT_LAYER")?;
    // Reset the default layer before decoding the file. Its construction is
    // observable by scripts that supply an ID allocator for command replay.
    let initial = class.call1((&default,))?;
    let raw = data(payload, "layers")?;
    let layers = PyList::empty(py);
    let mut has_default = false;
    if raw.is_truthy()? {
        for item in raw.try_iter()? {
            let item = item?;
            let layer = crate::file_styles::igz_load_layer(item.cast::<PyDict>()?, &class)?;
            has_default |= layer.getattr("key")?.eq(&default)?;
            layers.append(layer)?;
        }
        if !has_default {
            layers.insert(0, class.call1((default,))?)?;
        }
    } else {
        layers.append(initial)?;
    }
    scene.setattr("layers", layers)?;
    let ids = data(payload, "layer_folder_uids")?;
    scene.setattr(
        "layer_folder_uids",
        if ids.is_truthy()? {
            py.get_type::<PyDict>().call1((ids,))?
        } else {
            PyDict::new(py).into_any()
        },
    )?;
    let active = data(payload, "active_layer")?;
    if active.is_instance_of::<PyString>() {
        let layer = scene.call_method1("layer", (active,))?;
        if !layer.is_none() {
            scene.setattr("active_layer", layer.getattr("key")?)?;
        }
    }
    scene.setattr("layer_folders", string_list(payload, "layer_folders")?)?;
    module.call_method1("folders_of", (scene,))?;
    scene.setattr(
        "folder_parents",
        strings(payload, "layer_folder_parents", false)?,
    )?;
    let visibility = PyDict::new(py);
    if let Ok(raw) = data(payload, "layer_folder_visibility")?.cast::<PyDict>() {
        let folders = scene.getattr("layer_folders")?;
        for (k, v) in raw {
            if folders.contains(&k)? {
                visibility.set_item(k, v.is_truthy()?)?;
            }
        }
    }
    scene.setattr("folder_visibility", visibility)?;
    scene.setattr(
        "layer_folder_names",
        strings(payload, "layer_folder_names", true)?,
    )?;
    for (key, load) in [
        ("line_types", "load_line_types"),
        ("linetype_colors", "load_linetype_colors"),
        ("bricks", "load_bricks"),
    ] {
        let owner = if load == "load_line_types" { "tag_edits" } else { "tag_styles" };
        crate::model::module(py, "ingetrazo_core")?.getattr(owner)?.call_method1(load, (scene, data(payload, key)?))?;
    }
    if let Ok(patterns) = data(payload, "hatch_patterns")?.cast::<PyDict>() {
        let clean = py.import("core.hatches")?.getattr("clean_families")?;
        let dest = scene.getattr("hatch_patterns")?;
        for (k, v) in patterns.iter().take(500) {
            let Ok(name) = k.extract::<String>() else {
                continue;
            };
            let Ok(raw) = v.cast::<PyDict>() else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let families = clean.call1((data(raw, "families")?,))?;
            if families.is_truthy()? {
                let entry = PyDict::new(py);
                let label = data(raw, "label")?
                    .extract::<String>()
                    .map_or_else(|_| name.clone(), |s| s.chars().take(120).collect());
                entry.set_item("label", label)?;
                entry.set_item("families", families)?;
                dest.set_item(name.chars().take(64).collect::<String>(), entry)?;
            }
        }
    }
    let tile = data(payload, "tile_layer")?;
    if tile.is_truthy()? {
        let class = py.import("georef.tiles")?.getattr("TileLayer")?;
        if let Ok(layer) = class.call_method1("from_dict", (tile,)) {
            scene.setattr("tile_layer", layer)?;
        }
    }
    Ok(())
}
#[pyfunction]
pub fn igz_restore_views(scene: &Obj<'_>, payload: &Dict<'_>) -> PyResult<()> {
    let py = scene.py();
    let class = py.import("ingetrazo_core.saved_views")?.getattr("SavedView")?;
    let dest = PyList::empty(py);
    if let Some(seq) = payload.get_item("saved_views")? {
        for v in seq.try_iter()? {
            let v = v?;
            if class.hasattr("_load_file_view")? && v.is_instance_of::<PyDict>() {
                let view = class.call_method1("__new__", (&class,))?;
                if view.call_method1("_load_file_view", (&v,))?.is_truthy()? {
                    dest.append(view)?;
                    continue;
                }
            }
            dest.append(class.call_method1("from_dict", (v,))?)?;
        }
    }
    scene.setattr("saved_views", dest)?;
    scene.setattr("scene_folders", string_list(payload, "scene_folders")?)?;
    scene.setattr(
        "scene_folder_parents",
        strings(payload, "scene_folder_parents", false)?,
    )?;
    scene.setattr("compositions", PyList::empty(py))?;
    if let Some(seq) = payload.get_item("compositions")? {
        for item in seq.try_iter()? {
            let item = item?;
            match item
                .cast::<PyDict>()
                .map_err(PyErr::from)
                .and_then(|v| crate::file_sheet::igz_load_sheet(py, v))
            {
                Ok(sheet) => {
                    scene
                        .getattr("compositions")?
                        .call_method1("append", (sheet,))?;
                }
                Err(e) => {
                    py.import("logging")?
                        .call_method1("getLogger", ("formats.igz",))?
                        .call_method1(
                            "warning",
                            (
                                "sheet %r skipped: %s",
                                item.call_method1("get", ("name", "?"))?,
                                e.value(py),
                            ),
                        )?;
                }
            }
        }
    }
    Ok(())
}
fn restore_records(
    scene: &Obj<'_>,
    payload: &Dict<'_>,
    key: &str,
    module: &str,
    class: &str,
) -> PyResult<()> {
    if let Some(seq) = payload.get_item(key)? {
        let class = scene.py().import(module)?.getattr(class)?;
        let dest = scene.getattr(key)?;
        for v in seq.try_iter()? {
            dest.call_method1("append", (class.call_method1("from_dict", (v?,))?,))?;
        }
    }
    Ok(())
}
#[pyfunction]
pub fn igz_finish_scene(scene: &Obj<'_>, payload: &Dict<'_>) -> PyResult<usize> {
    let py = scene.py();
    crate::file_annotations::igz_load_annotations(py, scene, payload, false)?;
    let style = scene.getattr("dimension_style")?;
    style.set_item("ends", "tick")?;
    if let Ok(raw) = data(payload, "dimension_style")?.cast::<PyDict>() {
        style.call_method1("update", (raw,))?;
    }
    for (key, to) in [
        ("camera", "camera_home"),
        ("views", "views_home"),
        ("plugin_data", "plugin_data"),
    ] {
        let raw = data(payload, key)?;
        let value = if let Ok(d) = raw.cast::<PyDict>() {
            d.copy()?.into_any()
        } else if key == "plugin_data" {
            PyDict::new(py).into_any()
        } else {
            py.None().into_bound(py)
        };
        if key == "plugin_data" {
            // The links registry is the document's root table.
            let reg = value.call_method1("pop", ("links", py.None()))?;
            let reg = if reg.is_instance_of::<PyDict>() { reg } else { PyDict::new(py).into_any() };
            scene.setattr(to, value)?;
            scene.setattr("link_registry", reg)?;
            continue;
        }
        scene.setattr(to, value)?;
    }
    scene.setattr(
        "units",
        py.import("core.units")?
            .call_method1("model_units_of", (payload,))?,
    )?;
    if let Ok(seq) = data(payload, "custom_scales")?.cast::<PyList>() {
        let scales = PyList::empty(py);
        for v in seq {
            if (v.is_instance_of::<pyo3::types::PyFloat>()
                || v.is_instance_of::<pyo3::types::PyInt>())
                && v.gt(0)?
            {
                scales.append(v.extract::<f64>()?)?;
            }
        }
        scene.setattr("custom_scales", scales)?;
    }
    if let Ok(seq) = data(payload, "back_face_color")?.cast::<PyList>() {
        if seq.len() == 3 {
            let vals = rows(seq, |c| py.get_type::<pyo3::types::PyFloat>().call1((c,)))?;
            scene.setattr("back_face_color", vals.to_tuple())?;
        }
    }
    for (key, module, class) in [
        ("display_style", "core.style", "Style"),
        ("shadows", "core.sun", "ShadowSettings"),
    ] {
        let c = py.import(module)?.getattr(class)?;
        let raw = data(payload, key)?;
        scene.setattr(
            key,
            if raw.is_instance_of::<PyDict>() {
                c.call_method1("from_dict", (raw,))?
            } else {
                c.call0()?
            },
        )?;
    }
    scene.setattr(
        "animation",
        py.import("ingetrazo_core")?
            .getattr("SceneAnimation")?
            .call_method1("from_dict", (data(payload, "scene_animation")?,))?,
    )?;
    if let Ok(seq) = data(payload, "section_planes")?.cast::<PyList>() {
        let class = py.import("ingetrazo_core.section")?.getattr("SectionPlane")?;
        let mut active = false;
        let planes = rows(seq, |v| {
            let plane = class.call_method1("from_dict", (v,))?;
            if plane.getattr("active")?.is_truthy()? {
                if active {
                    plane.setattr("active", false)?;
                }
                active = true;
            }
            Ok(plane)
        })?;
        scene.setattr("section_planes", planes)?;
    }
    for (block, key, to, inverted) in [
        ("section_view", "planes_hidden", "show_section_planes", true),
        ("section_view", "cuts_hidden", "show_section_cuts", true),
        ("hidden_view", "objects", "show_hidden_objects", false),
        ("hidden_view", "geometry", "show_hidden_geometry", false),
    ] {
        if let Ok(r) = data(payload, block)?.cast::<PyDict>() {
            scene.setattr(to, data(r, key)?.is_truthy()? ^ inverted)?;
        }
    }
    scene.setattr("show_axes", !data(payload, "axes_hidden")?.is_truthy()?)?;
    let north = py
        .get_type::<pyo3::types::PyFloat>()
        .call1((data(payload, "north_deg")?,))
        .and_then(|n| py.import("core.sun")?.call_method1("wrap_north_deg", (n,)));
    match north {
        Ok(n) => scene.setattr("north_deg", n)?,
        Err(e) if e.is_instance_of::<PyTypeError>(py) || e.is_instance_of::<PyValueError>(py) => {
            scene.setattr("north_deg", 0.)?
        }
        Err(e) => return Err(e),
    }
    if let Ok(r) = data(payload, "georef")?.cast::<PyDict>() {
        let datum = data(r, "datum")?;
        if datum.is_instance_of::<PyDict>() {
            scene.setattr(
                "georef",
                py.import("georef.datum")?
                    .getattr("SceneDatum")?
                    .call_method1("from_dict", (datum,))?,
            )?;
        }
    }
    restore_records(scene, payload, "geo_paths", "georef.geopath", "GeoPath")?;
    restore_records(scene, payload, "geo_points", "georef.points", "GeoPoint")?;
    py.import("core.composition")?
        .call_method1("bind_sheet_anchors", (scene,))?;
    let anchors = scene.getattr("text_placement_anchors")?;
    anchors.call_method0("clear")?;
    if let Some(seq) = payload.get_item("text_placement_anchors")? {
        let class = py
            .import("ingetrazo_core.annotation_anchor")?
            .getattr("TextPlacementAnchor")?;
        for item in seq.try_iter()? {
            let v = class.call_method1("restore", (scene, item?))?;
            if !v.is_none() {
                anchors.call_method1("append", (v,))?;
            }
        }
    }
    crate::file_annotations::igz_load_annotations(py, scene, payload, true)?;
    let guides = scene.getattr("guides")?;
    guides.call_method0("clear")?;
    let mut repairs = 0;
    for (key, circles) in [("guides", false), ("guide_circles", true)] {
        if let Some(seq) = payload.get_item(key)? {
            let factory = py.import("core.guide")?.getattr("guide_from_dict")?;
            let finite = py.import("formats.igz")?.getattr("_finite_points")?;
            for item in seq.try_iter()? {
                let mut raw = item?.cast_into::<PyDict>()?;
                if circles {
                    raw = raw.copy()?;
                    raw.set_item("kind", "circle")?;
                }
                let circle = data(&raw, "kind")?.eq("circle")?;
                let mut ok = true;
                for k in if circle {
                    &["center", "normal"][..]
                } else {
                    &["point", "direction", "origin"]
                } {
                    if let Some(v) = raw.get_item(k)? {
                        if matches!(*k, "direction" | "normal" | "origin") && !v.is_truthy()? {
                            continue;
                        }
                        if !finite.call1((v,))?.is_truthy()? {
                            ok = false;
                            break;
                        }
                    }
                }
                if circle {
                    if let Some(r) = raw.get_item("radius")? {
                        ok &= (r.is_instance_of::<pyo3::types::PyInt>()
                            || r.is_instance_of::<pyo3::types::PyFloat>())
                            && r.extract::<f64>()?.is_finite();
                    }
                }
                if ok {
                    guides.call_method1("append", (factory.call1((raw,))?,))?;
                } else {
                    repairs += 1;
                }
            }
        }
    }
    scene.getattr("image_planes")?.call_method0("clear")?;
    restore_records(
        scene,
        payload,
        "image_planes",
        "ingetrazo_core.image_plane",
        "ImagePlane",
    )?;
    Ok(repairs)
}
