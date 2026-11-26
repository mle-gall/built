// SPDX-License-Identifier: GPL-3.0-or-later
//! The chrome's right column (`views.wgpu_ui`): the project tree's model
//! and rows, the export problems, the material strip, the components and
//! the takeoff.
use super::chrome_frame::{attr, id, own, render, truthy, ui};
use super::*;
use pyo3::exceptions::{PyException, PyIndexError, PyValueError};

fn builtins<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("builtins")
}
fn tuple_of<'py>(py: Python<'py>, values: impl IntoPyObject<'py>) -> Reply<'py> {
    py.get_type::<PyTuple>().call1((values,))
}
fn sorted<'py>(py: Python<'py>, values: impl IntoPyObject<'py>) -> Reply<'py> {
    builtins(py)?.call_method1("sorted", (values,))
}
fn slice<'py>(py: Python<'py>, from: isize, to: isize) -> Bound<'py, pyo3::types::PySlice> {
    pyo3::types::PySlice::new(py, from, to, 1)
}
/// `text.partition(":")` as `(head, rest)`.
fn parts(key: &Obj<'_>) -> PyResult<(String, String)> {
    let key: String = key.extract()?;
    Ok(match key.split_once(':') {
        Some((head, rest)) => (head.to_owned(), rest.to_owned()),
        None => (key, String::new()),
    })
}
/// `getattr(win, "_outliner_kinds", ())`.
fn outliner_kinds<'py>(win: &Obj<'py>) -> Reply<'py> {
    Ok(win.getattr_opt("_outliner_kinds")?.unwrap_or_else(|| PyTuple::empty(win.py()).into_any()))
}
fn current_path(win: &Obj<'_>) -> PyResult<String> {
    match attr(win, "_current_path")? {
        path if path.is_truthy()? => path.str()?.extract(),
        _ => Ok(String::new()),
    }
}
/// Run `body` under the wait cursor, the viewport's own put back after.
pub(super) fn waiting<'py, T>(viewport: &Obj<'py>, body: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    let py = viewport.py();
    let previous = viewport.call_method0("cursor")?;
    viewport.call_method1("setCursor", (py.import("built_core")?.getattr("Input")?.getattr("WaitCursor")?,))?;
    let result = body();
    viewport.call_method1("setCursor", (previous,))?;
    result
}

pub(super) fn kind_of<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let class = a.get_item(0)?;
    let kinds = ui(py)?.getattr("KINDS")?;
    if kinds.contains(&class)? {
        return kinds.get_item(&class);
    }
    if class.is_truthy()? {
        class.str()?.call_method1("removeprefix", ("Ifc",))
    } else {
        "Drawing only".into_bound_py_any(py)
    }
}

/// The tree's nodes and memberships (core/link_structure.py), kept until
/// the model changes: a click that only changes the selection rebuilds the
/// rows from it without walking every placement again.
fn tree_model<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, scene, version, readers) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let cached = own(&view, "_ui_tree_model")?;
    let key = (id(&scene), &version).into_bound_py_any(py)?;
    // A tree prepared on the open worker has only the document/version key.
    // Adopt it at that version and start following this pane's history feed.
    let epoch = if readers.is_none() { none(py) } else { readers.call_method1("filing_epoch", (scene.getattr("content_version")?,))? };
    let keyed = |model: &Obj<'py>| view.setattr("_ui_tree_model", ((id(&scene), &version, &epoch), model));
    if let Some(cached) = &cached {
        let stamp = cached.get_item(0)?;
        if stamp.get_item(slice(py, 0, 2))?.eq(&key)? {
            let model = cached.get_item(1)?;
            if stamp.len()? == 2 {
                keyed(&model)?;
            }
            return Ok(model);
        }
    }
    let mut kept = None;
    if let Some(cached) = cached.filter(|_| !epoch.is_none()) {
        let stamp = cached.get_item(0)?;
        let model = cached.get_item(1)?;
        if stamp.get_item(0)?.eq(id(&scene))?
            && stamp.get_item(slice(py, 2, isize::MAX))?.eq((&epoch,))?
            && py.import("built_core")?.call_method1("refresh_unfiled", (&scene, model.get_item("tree")?))?.is_truthy()?
        {
            kept = Some(model);
        }
    }
    let model = match kept {
        Some(model) => model,
        None => ui(py)?.call_method1("prepare_tree_model", (&scene,))?,
    };
    keyed(&model)?;
    Ok(model)
}

fn prepare_tree_model<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let scene = a.get_item(0)?;
    let collections = py.import("collections")?;
    let counter = collections.getattr("Counter")?;
    let structure = py.import("built_core.structure")?;
    let registry = py.import("core.bim_registry")?.call_method1("get", (&scene,))?;
    let tree = py.import("built_core.link_structure")?.getattr("Tree")?.call1((&scene, &registry))?;
    let nodes = tree.getattr("nodes")?;
    let recipes = PyDict::new(py);
    for entry in nodes.call_method0("items")?.try_iter()? {
        let [key, node] = unpack::<2>(&entry?)?;
        recipes.set_item(key, node.get_item("recipe")?)?;
    }
    let (counts, problems, to_fix) = (counter.call0()?, counter.call0()?, PyList::empty(py));
    let node_layers = collections.getattr("defaultdict")?.call1((&counter,))?;
    let add = |tally: &Obj<'py>, key: &Obj<'py>, more: Obj<'py>| tally.set_item(key, tally.get_item(key)?.add(more)?);
    let unfiled = PySet::empty(py)?;
    for group in tree.getattr("unfiled")?.try_iter()? {
        unfiled.add(id(&group?))?;
    }
    let one = 1.into_bound_py_any(py)?;
    for entry in tree.getattr("direct")?.call_method0("items")?.try_iter()? {
        let [key, groups] = unpack::<2>(&entry?)?;
        let host = nodes.call_method1("get", (&key, PyDict::new(py)))?.call_method1("get", ("kind",))?.eq("host")?;
        for group in groups.try_iter()? {
            let group = group?;
            add(&counts, &key, one.clone())?;
            let layer = attr(&group, "layer")?;
            if layer.is_truthy()? {
                add(&node_layers.get_item(&key)?, &layer, one.clone())?;
            }
            if host {
                let record = match group.getattr("ifc")? {
                    record if record.is_truthy()? => record,
                    _ => PyDict::new(py).into_any(),
                };
                let unset = !recipes.as_any().get_item(&key)?.call_method1("get", ("class",))?.is_none()
                    && !structure.call_method1("_has_status", (record,))?.is_truthy()?;
                let bad = unset as i32 + unfiled.contains(id(&group))? as i32;
                add(&problems, &key, bad.into_bound_py_any(py)?)?;
                if bad != 0 {
                    to_fix.append(group)?;
                }
            }
        }
    }
    let order = tree.call_method0("order")?;
    for entry in builtins(py)?.call_method1("reversed", (&order,))?.try_iter()? {
        let key = entry?.get_item(0)?;
        let parent = nodes.get_item(&key)?.get_item("parent")?;
        add(&counts, &parent, counts.get_item(&key)?)?;
        add(&problems, &parent, problems.get_item(&key)?)?;
    }
    for entry in tree.getattr("links")?.call_method0("items")?.try_iter()? {
        let [key, copies] = unpack::<2>(&entry?)?;
        if !counts.get_item(&key)?.is_truthy()? {
            counts.set_item(&key, py.import("built_core")?.call_method1("source_direct_count", (copies,))?)?;
        }
    }
    Ok(fields!(py;
        "tree" => tree, "registry" => registry, "recipes" => recipes, "counts" => counts, "problems" => problems,
        "to_fix" => to_fix, "node_layers" => node_layers, "order" => order,
    )
    .into_any())
}

fn open_line_type<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, name, tags) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let view = render(py)?.getattr("chrome_dialogs")?.call_method1("view_of", (&win,))?;
    if !truthy(&view, "_ui_attached")? {
        win.getattr("viewport")?.call_method1("flash_status", ("Line type editing requires the native Drawing lens.",))?;
        return false.into_bound_py_any(py);
    }
    // The line editor opens in the inspector, whatever the tree's columns.
    view.setattr("_ui_mode", "")?;
    view.setattr("_ui_tree_sig", py.None())?;
    let module = ui(py)?;
    module.call_method1("_tree", (&view, &win))?;
    let opened = view.getattr("renderer")?.call_method1("ui_open_line_type", (name, py.get_type::<PyList>().call1((tags,))?))?;
    view.call_method0("update")?;
    Ok(opened)
}

fn sync_project_rows<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let renderer = view.getattr("renderer")?;
    if renderer.call_method0("ui_project_hidden")?.is(&true.into_bound_py_any(py)?) {
        return Ok(none(py));
    }
    let module = ui(py)?;
    module.call_method1("_tree", (&view, &win))?;
    // The dock counts the entities of the tags in focus, under every preset.
    if view.getattr("renderer")?.call_method1("ui_tick_tag_counts", (win.getattr("viewport")?.getattr("history")?,))?.is_truthy()? {
        view.call_method0("update")?;
    }
    module.call_method1("_inspector", (&view, &win))?;
    Ok(none(py))
}

/// Whether the dock shows the export checks: its Readiness column or the
/// Fix checklist of the document inspector. The dock says so (`readiness`);
/// until it has, the Export preset with its own columns asks for them.
pub(super) fn wants_readiness(view: &Obj<'_>) -> PyResult<bool> {
    if let Some(asked) = own(view, "_ui_readiness")?.filter(|asked| !asked.is_none()) {
        return asked.is_truthy();
    }
    let export = own(view, "_ui_lens")?.is_some_and(|lens| lens.eq(2).unwrap_or(false));
    Ok(export && !own(view, "_ui_columns")?.is_some_and(|mask| !mask.is_none()))
}

/// Bind the membership index, revision and disclosure state to the native
/// sidebar providers. The rows are the same under every column preset.
fn tree<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let discover = a.get_item(2)?.is_truthy()?;
    let viewport = win.getattr("viewport")?;
    let scene = attr(&viewport, "scene")?;
    if scene.is_none() {
        return Ok(none(py));
    }
    let module = ui(py)?;
    let core = py.import("built_core")?;
    let state = view.getattr("__dict__")?;
    let mut exp = state.call_method1("setdefault", ("_ui_exp", PySet::empty(py)?))?;
    let pages = state.call_method1("setdefault", ("_ui_pages", PyDict::new(py)))?;
    let fixes = truthy(&view, "_ui_fix_rows")?;
    let flt = if discover || fixes { "".into_bound_py_any(py)? } else { state.call_method1("get", ("_ui_filter", ""))? };
    let lens = if discover { 0.into_bound_py_any(py)? } else { state.call_method1("get", ("_ui_lens", 0))? };
    let catalog = match own(&view, "_ui_sheet_catalog")? {
        Some(catalog) => catalog,
        None => {
            let catalog = core.getattr("ProjectSheets")?.call0()?;
            view.setattr("_ui_sheet_catalog", &catalog)?;
            catalog
        }
    };
    let sheet_revision = catalog.call_method1("refresh", (&scene,))?;
    view.setattr("_ui_sheet_revision", &sheet_revision)?;
    // The export checks include watertightness: they must follow mesh edits.
    let records = core.getattr("deliveries")?.call_method1("records", (&scene,))?;
    let geometric = !discover && (records.is_truthy()? || wants_readiness(&view)?);
    let content = scene.getattr("content_version")?;
    let version = if geometric { content.clone() } else { direct(py, tree_version, (&win, &scene))? };
    let kinds = outliner_kinds(&win)?;
    let mut kind_ids = Vec::new();
    for kind in kinds.try_iter()? {
        kind_ids.push(id(&kind?));
    }
    let filtering = flt.is_truthy()?;
    let sig = PyTuple::new(
        py,
        [
            id(&scene).into_bound_py_any(py)?,
            id(&scene.getattr("edit_group")?).into_bound_py_any(py)?,
            scene.getattr("mesh")?.getattr("revision")?,
            version.clone(),
            tuple_of(py, sorted(py, &exp)?)?,
            tuple_of(py, sorted(py, pages.call_method0("items")?)?)?,
            flt.clone(),
            if filtering { content.clone() } else { none(py) },
            direct(py, super::chrome_frame::sel_key, (scene.getattr("selection")?,))?,
            geometric.into_bound_py_any(py)?,
            view.getattr_opt("_ui_layer_rev")?.unwrap_or(0.into_bound_py_any(py)?),
            scene.getattr("show_section_planes")?,
            sheet_revision,
            PyTuple::new(py, kind_ids)?.into_any(),
        ],
    )?;
    if !discover {
        let old = attr(&view, "_ui_tree_sig")?;
        if sig.as_any().eq(&old)? { return Ok(none(py)); }
        // Selection is slot 8. All structural inputs must still match.
        if !old.is_none() && !filtering && !geometric && scene.getattr("edit_group")?.is_none()
            && sig.get_slice(0, 8).eq(old.get_item(slice(py, 0, 8))?)?
            && sig.get_slice(9, sig.len()).eq(old.get_item(slice(py, 9, isize::MAX))?)?
            && attr(&scene, "_browser_picks")?.is_none()
            && state.call_method1("get", ("_ui_mode", ""))?.eq("")?
            && view.getattr("renderer")?.hasattr("ui_mark_tag_selection")?
            && view.getattr("renderer")?.call_method1("ui_mark_tag_selection", (&scene,))?.is(&true.into_bound_py_any(py)?)
        {
            view.setattr("_ui_tree_sig", &sig)?;
            return Ok(none(py));
        }
    }
    let first = !view.hasattr("_ui_tree")?;
    if !discover {
        view.setattr("_ui_tree_sig", &sig)?;
    }
    // The selected ids only once the signature (the selection's revision) moved.
    let sel_ids = if discover {
        let ids = PySet::empty(py)?;
        for entity in scene.getattr("selection")?.try_iter()? {
            ids.add(id(&entity?))?;
        }
        ids.into_any()
    } else {
        render(py)?.call_method1("browser_selection", (&scene,))?
    };
    let mode = if discover { "".into_bound_py_any(py)? } else { state.call_method1("get", ("_ui_mode", ""))? };
    let structure = py.import("built_core.structure")?;
    let layers = py.import("built_core.layers")?;
    let history = attr(&viewport, "history")?;
    let readers = attr(&attr(&history, "feed")?, "readers")?;
    let model = module.call_method1("_tree_model", (&view, &scene, &version, if kinds.is_truthy()? { none(py) } else { readers }))?;
    let part = |name: &str| model.get_item(name);
    let (tree, registry, recipes) = (part("tree")?, part("registry")?, part("recipes")?);
    let (counts, to_fix, node_layers, order) = (part("counts")?, part("to_fix")?, part("node_layers")?, part("order")?);
    if discover || fixes {
        exp = core.call_method1("reveal_branches", (&scene, &tree))?;
    }
    let (nodes, children, direct, unfiled) = (tree.getattr("nodes")?, tree.getattr("children")?, tree.getattr("direct")?, tree.getattr("unfiled")?);
    let node_of = tree.getattr("member")?;
    let active = registry.call_method1("get", ("structure", PyDict::new(py)))?.call_method1("get", ("active",))?;
    let label = |key: &Obj<'py>| -> PyResult<String> { Ok(format!("n:{}", key.str()?.extract::<String>()?)) };
    if first {
        // The host's own top nodes open; files stay folded (each is a root).
        for entry in order.try_iter()? {
            let key = entry?.get_item(0)?;
            let node = nodes.get_item(&key)?;
            if !node.get_item("parent")?.is_truthy()? && node.get_item("kind")?.eq("host")? {
                exp.call_method1("add", (label(&key)?,))?;
            }
        }
        let mut key = active.clone();
        while nodes.contains(&key)? && nodes.get_item(&key)?.get_item("parent")?.is_truthy()? {
            key = nodes.get_item(&key)?.get_item("parent")?;
            exp.call_method1("add", (label(&key)?,))?;
        }
    }
    let rows = core.getattr("filing_rows")?.call1(PyTuple::new(
        py,
        [
            scene.clone(),
            model.clone(),
            exp.clone(),
            pages,
            flt.clone(),
            (discover || fixes).into_bound_py_any(py)?,
            module.getattr("ROW_CAP")?,
            // Every row carries its tag's drawing cells: a preset only shows them.
            1.into_bound_py_any(py)?,
            sel_ids.clone(),
            active.clone(),
            module.getattr("KINDS")?,
            module.getattr("TIPS")?,
            module.getattr("FILE_STATES")?,
            attr(&win, "_outliner_kinds")?,
        ],
    )?)?;
    let path = current_path(&win)?;
    let [rows, path_bar] = unpack::<2>(
        &py.import("plugins._browser")?.call_method1("project_sidebar", (&scene, &tree, rows, &flt, &exp, &path, false, &sel_ids))?,
    )?;
    rows.call_method1("extend", (catalog.call_method1("rows", (&flt, &exp))?,))?;
    let in_project = match win.getattr_opt("__dict__")? {
        Some(own) => own.contains("_project")?,
        None => false,
    };
    let project = || render(py)?.getattr("shell_project");
    if in_project {
        rows.call_method1("extend", (project()?.call_method1("tree_rows", (&win, &flt, &exp))?,))?;
    }
    if discover {
        return core.call_method1("reveal_selection", (rows,));
    }
    let renderer = view.getattr("renderer")?;
    if geometric {
        let chosen = attr(&view, "_delivery_key")?;
        let mut delivery = records.call_method1("get", (&chosen,))?;
        if !delivery.is_truthy()? {
            delivery = match records.call_method0("values")?.try_iter()?.next() {
                Some(first) => first?,
                None => PyDict::new(py).into_any(),
            };
        }
        let profile = if delivery.is_truthy()? {
            delivery.call_method1("get", ("options", PyDict::new(py)))?.call_method1("get", ("profile", ""))?
        } else {
            none(py)
        };
        let cache_key = (id(&scene), &content, &profile).into_bound_py_any(py)?;
        let mut cache = attr(&view, "_ui_rules_cache")?;
        if !cache.is_truthy()? || cache.get_item(0)?.ne(&cache_key)? {
            let rules = module.call_method1("_problem_rules", (&scene, &registry, structure.call_method1("_nodes", (&registry,))?, &recipes, &unfiled, &profile))?;
            if in_project {
                rules.call_method1("extend", (project()?.call_method1("problems", (&win,))?,))?;
            }
            cache = (cache_key, rules).into_bound_py_any(py)?;
            view.setattr("_ui_rules_cache", &cache)?;
        }
        let key = if chosen.is_truthy()? { chosen } else { "".into_bound_py_any(py)? };
        // The checks annotate the rows (the Readiness column); they never change them.
        let readiness = core.call_method1("export_readiness", (&scene, &tree, &records, key, cache.get_item(1)?))?;
        renderer.call_method1("ui_panel_set", ("readiness", readiness))?;
    }
    let rows = core.call_method1("mark_filing_selection", (rows,))?;
    let layer_lists = PyDict::new(py);
    for entry in node_layers.call_method0("items")?.try_iter()? {
        let [key, tally] = unpack::<2>(&entry?)?;
        layer_lists.set_item(key, py.get_type::<PyList>().call1((tally,))?)?;
    }
    view.setattr(
        "_ui_tree",
        fields!(py;
            "direct" => &direct, "children" => &children, "unfiled" => &unfiled, "recipes" => &recipes, "to_fix" => &to_fix,
            "active" => &active, "node_of" => &node_of, "nodes" => &nodes, "tree" => &tree, "counts" => &counts,
            "node_layers" => layer_lists,
        ),
    )?;
    renderer.call_method1("ui_set_tree", (rows, unfiled.len()?))?;
    let file_name = py.import("pathlib")?.getattr("Path")?.call1((&path,))?.getattr("name")?;
    renderer.call_method1("ui_set_catalog_info", (file_name, &tree, &counts, sel_ids.len()?))?;
    renderer.call_method1("ui_set_edit_path", (path_bar,))?;
    if renderer.hasattr("ui_set_proposal_available")? {
        let none_yet = tree.getattr("_host_roots")?.lt(0)?;
        renderer.call_method1("ui_set_proposal_available", (!mode.is_truthy()? && none_yet,))?;
    }
    if !own(&view, "_ui_sort_loaded")?.is_some_and(|loaded| loaded.is_truthy().unwrap_or(false)) && renderer.hasattr("ui_set_tree_sort")? {
        let stored = py.import("built_core.settings")?.call_method0("settings")?.call_method(
            "value",
            ("ui/project_tree_sort", 0),
            Some(&fields!(py; "type" => py.get_type::<pyo3::types::PyInt>())),
        )?;
        renderer.call_method1("ui_set_tree_sort", (builtins(py)?.call_method1("max", (0, stored))?,))?;
        view.setattr("_ui_sort_loaded", true)?;
    }
    let line_types = py.get_type::<PyList>().call1((layers.getattr("LINETYPES")?,))?;
    let custom = layers.getattr("CUSTOM_LINETYPE")?;
    for key in scene.getattr("line_types")?.try_iter()? {
        line_types.call_method1("append", (custom.add(key?)?,))?;
    }
    // A tag's drawing style is in its inspector under every preset.
    renderer.call_method1("ui_set_style_catalog", (id(&scene), line_types, scene.getattr("hatch_patterns")?, scene.clone()))?;
    renderer.call_method1("ui_set_columns", (&lens, own(&view, "_ui_columns")?.unwrap_or_else(|| none(py))))?;
    renderer.call_method1("ui_set_mode", (if mode.eq("library")? { "Library" } else { "" },))?;
    renderer.call_method1("ui_set_issues", (own(&view, "_ui_issues")?.is_some_and(|issues| issues.is_truthy().unwrap_or(false)),))?;
    if !truthy(&view, "_ui_kinds_sent")? {
        let kinds = py.get_type::<PySet>().call1((module.getattr("KINDS")?.call_method0("values")?,))?;
        let kinds = kinds.sub(PySet::new(py, ["Level", "Building", "Site"])?)?;
        renderer.call_method1("ui_set_kinds", (sorted(py, kinds)?,))?;
        view.setattr("_ui_kinds_sent", true)?;
    }
    let path = if structure.call_method1("_nodes", (&registry,))?.contains(&active)? {
        " › ".into_bound_py_any(py)?.call_method1("join", (structure.call_method1("path", (&registry, &active))?,))?
    } else {
        "".into_bound_py_any(py)?
    };
    renderer.call_method1("ui_set_delivery", (path, unfiled.len()?, to_fix.len()?))?;
    Ok(none(py))
}

pub(super) fn tree_version<'py>(a: &Args<'py>) -> Reply<'py> {
    let (win, scene) = (a.get_item(0)?, a.get_item(1)?);
    let feed = attr(&attr(&win.getattr("viewport")?, "history")?, "feed")?;
    let content = scene.getattr("content_version")?;
    if feed.is_none() || outliner_kinds(&win)?.is_truthy()? {
        return Ok(content);
    }
    feed.getattr("readers")?.call_method1("browser_version", (content,))
}

fn problem_rules<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (scene, registry, nodes, recipes, unfiled, profile) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?, a.get_item(4)?, a.get_item(5)?);
    let structure = py.import("built_core.structure")?;
    let ids_module = py.import("core.structure_ids")?;
    // (severity, rule id, label, [groups])
    let rules = PyList::empty(py);
    let list = |values: &Obj<'py>| py.get_type::<PyList>().call1((values,));
    rules.append(("Blocks export", "unfiled", "Not filed", list(&unfiled)?))?;
    let requirements = (|| -> Reply<'py> {
        let profiles = registry.call_method1("get", ("export_profiles", PyDict::new(py)))?;
        let checked = if profile.is_none() {
            registry.clone()
        } else {
            let checked = py.get_type::<PyDict>().call1((&registry,))?;
            checked.set_item("export_profiles", fields!(py; &profile => profiles.call_method1("get", (&profile, PyDict::new(py)))?))?;
            checked
        };
        ids_module.call_method1("load_requirements", (&scene, checked))
    })();
    let requirements = match requirements {
        Ok(requirements) => requirements,
        Err(error) if error.is_instance_of::<PyException>(py) => PyList::empty(py).into_any(),
        Err(error) => return Err(error),
    };
    let checking = requirements.is_truthy()?;
    let ids = PyDict::new(py);
    let (missing, hidden, unclassified) = (PyList::empty(py), PyList::empty(py), PyList::empty(py));
    for entry in structure.call_method1("_memberships", (&scene, &registry))?.try_iter()? {
        let [group, source, _root, key] = unpack::<4>(&entry?)?;
        if !nodes.contains(&key)? || source.is_truthy()? {
            continue;
        }
        let record = match group.getattr("ifc")? {
            record if record.is_truthy()? => record,
            _ => PyDict::new(py).into_any(),
        };
        if !recipes.get_item(&key)?.call_method1("get", ("class",))?.is_none() {
            if !record.call_method1("get", ("class",))?.is_truthy()? {
                unclassified.append(&group)?;
            }
            if !structure.call_method1("_has_status", (&record,))?.is_truthy()? {
                missing.append(&group)?;
            }
            if truthy(&group, "hidden")? {
                hidden.append(&group)?;
            }
        }
        if checking && record.is_truthy()? {
            for hit in ids_module.call_method1("requirement_hits", (&group, &registry, &requirements))?.try_iter()? {
                let hit = hit?;
                let mut name = "IDS".into_bound_py_any(py)?;
                for field in ["specification", "file"] {
                    let given = hit.call_method1("get", (field,))?;
                    if given.is_truthy()? {
                        name = given;
                        break;
                    }
                }
                ids.call_method1("setdefault", (name, PyList::empty(py)))?.call_method1("append", (&group,))?;
            }
        }
    }
    rules.append(("Needs attention", "class", "Missing IFC class", unclassified))?;
    for entry in sorted(py, ids.items())?.try_iter()? {
        let [spec, groups] = unpack::<2>(&entry?)?;
        let spec = spec.str()?.extract::<String>()?;
        rules.append(("Blocks export", format!("ids:{spec}"), format!("IDS: {spec}"), groups))?;
    }
    let links = py.import("built_core.links")?;
    let link_structure = py.import("built_core.link_structure")?;
    let (stale, states) = (PyList::empty(py), PyDict::new(py));
    for group in links.call_method1("linked_groups", (&scene,))?.try_iter()? {
        let group = group?;
        let path = links.call_method1("link_of", (&group,))?.get_item("path")?;
        if !states.contains(&path)? {
            states.set_item(&path, link_structure.call_method1("file_state", (&scene, &path))?)?;
        }
        let state = states.as_any().get_item(&path)?;
        if state.eq("missing")? || state.eq("modified")? {
            stale.append(group)?;
        }
    }
    rules.append(("Needs attention", "links", "Linked file changed or missing", stale))?;
    let [leaky, checked, total] = unpack::<3>(&py.import("built_core")?.call_method1("solid_problem_targets", (&scene,))?)?;
    rules.append(("Needs attention", "solid", "Solid geometry problems", leaky))?;
    if checked.lt(&total)? {
        let label = format!("Watertight: checked {} of {} meshes", checked.str()?, total.str()?);
        rules.append(("Info", "solidcut", label, PyList::empty(py)))?;
    }
    rules.append(("Needs attention", "status", "Status missing", missing))?;
    rules.append(("Info", "hidden", "Hidden items in exported nodes", hidden))?;
    if !checking {
        rules.append(("Info", "noids", "No IDS checklist in the selected profile", PyList::empty(py)))?;
    }
    Ok(rules.into_any())
}

pub(super) fn hex<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let rgb = a.get_item(0)?;
    if !rgb.is_truthy()? {
        return "".into_bound_py_any(py);
    }
    let mut bytes = Vec::new();
    for channel in rgb.get_item(slice(py, 0, 3))?.try_iter()? {
        let level: f64 = py.get_type::<pyo3::types::PyFloat>().call1((channel?,))?.extract()?;
        bytes.push((level.clamp(0.0, 1.0) * 255.) as i64);
    }
    "#%02x%02x%02x".into_bound_py_any(py)?.rem(PyTuple::new(py, bytes)?)
}

pub(super) fn current_material_name<'py>(a: &Args<'py>) -> Reply<'py> {
    let material = attr(&a.py().import("tools.paint")?.getattr("PaintTool")?, "current_material")?;
    attr(&material, "name")
}

/// The checked tool of the window ("paint", "select", …) or "".
fn active_tool_id<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    if let Some(actions) = a.get_item(0)?.getattr_opt("_tool_actions")? {
        for entry in actions.call_method0("items")?.try_iter()? {
            let [key, action] = unpack::<2>(&entry?)?;
            if action.call_method0("isChecked")?.is_truthy()? {
                return Ok(key);
            }
        }
    }
    "".into_bound_py_any(py)
}

pub(super) fn components_dir<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py().import("built_core.paths")?.call_method0("app_root")?.div("resources")?.div("components")
}

/// The bundled set: `sc:<key>` 3D starters, `pp:<key>` 2D people.
pub(super) fn starter_components<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let json = py.import("json")?;
    let folder = direct(py, components_dir, ())?;
    let out = PyDict::new(py);
    for (file, prefix) in [("components.json", "sc:"), ("people.json", "pp:")] {
        let path = folder.div(file)?;
        if path.call_method0("exists")?.is_truthy()? {
            let text = path.call_method("read_text", (), Some(&fields!(py; "encoding" => "utf-8")))?;
            for entry in json.call_method1("loads", (text,))?.try_iter()? {
                let entry = entry?;
                out.set_item(prefix.into_bound_py_any(py)?.add(entry.get_item("key")?)?, entry)?;
            }
        }
    }
    Ok(out.into_any())
}

/// Placed and retained component definitions for this document revision.
pub(super) fn components_in_model<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let sig = (id(&scene), scene.getattr("content_version")?).into_bound_py_any(py)?;
    let cache = attr(&view, "_ui_comp_cache")?;
    if !cache.is_none() && cache.get_item(0)?.eq(&sig)? {
        return cache.get_item(1);
    }
    let rows = render(py)?.getattr("ComponentRows")?.call1((&scene,))?.call_method0("finish")?;
    view.setattr("_ui_comp_cache", (sig, &rows))?;
    Ok(rows)
}

/// The selected occurrence's Library definition and its placed copies.
pub(super) fn component_definition<'py>(view: &Obj<'py>, win: &Obj<'py>, group: &Obj<'py>) -> PyResult<Option<(usize, Obj<'py>)>> {
    let rows = direct(view.py(), components_in_model, (view, win))?;
    let mesh = group.getattr("mesh")?;
    for (index, row) in rows.try_iter()?.enumerate() {
        let row = row?;
        let copies = row.get_item(2)?;
        if copies.len()? > 0 && copies.get_item(0)?.getattr("mesh")?.is(&mesh) {
            return Ok(Some((index, row)));
        }
    }
    Ok(None)
}

/// A Library row: paint with a material, place a component.
fn library_pick<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, view, key) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let module = ui(py)?;
    let (kind, _) = parts(&key)?;
    let starter = || direct(py, starter_components, ())?.call_method1("get", (&key,));
    match kind.as_str() {
        "sc" => {
            let entry = starter()?;
            if entry.is_truthy()? {
                let name = py.import("built_core.i18n")?.call_method1("tr", (entry.get_item("name")?,))?;
                win.call_method1("_on_insert_component", (entry.get_item("key")?, name))?;
            }
        }
        "pp" => {
            let entry = starter()?;
            if entry.is_truthy()? {
                let file = format!("{}.png", entry.get_item("key")?.str()?.extract::<String>()?);
                win.call_method1("_on_insert_person_2d", (file, entry.get_item("height")?, entry.get_item("name")?))?;
            }
        }
        "cm" => {
            direct(py, component_verb, (&win, &view, "cplace", &key))?;
        }
        _ => {
            module.call_method1("_paint_with", (&win, &key))?;
        }
    }
    Ok(none(py))
}

pub(super) fn component_verb<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, view, verb, key) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let components = match attr(&view, "_ui_comp_cache")? {
        cache if cache.is_truthy()? => cache.get_item(1)?,
        _ => PyList::empty(py).into_any(),
    };
    let (_, index) = parts(&key)?;
    let row = py.get_type::<pyo3::types::PyInt>().call1((index,)).and_then(|index| components.get_item(index));
    let row = match row {
        Ok(row) => row,
        Err(error) if error.is_instance_of::<PyValueError>(py) || error.is_instance_of::<PyIndexError>(py) => return Ok(none(py)),
        Err(error) => return Err(error),
    };
    let [_name, _faces, groups, kept] = unpack::<4>(&row)?;
    let definition = if !kept.is_none() {
        kept.clone()
    } else if groups.is_truthy()? {
        groups.get_item(0)?
    } else {
        return Ok(none(py));
    };
    if definition.is_none() {
        return Ok(none(py));
    }
    let viewport = || win.getattr("viewport");
    let verb: String = verb.extract()?;
    match verb.as_str() {
        "cplace" => {
            let components = py.import("built_core.kept_components")?;
            let template = if kept.is_none() { components.call_method1("make_template", (&definition,))? } else { kept };
            win.call_method1("_start_place", (components.call_method1("place_copy", (template,))?,))?;
        }
        "cselect" => {
            render(py)?.call_method1("select_component_instances", (&win, groups))?;
        }
        "creplace" => {
            let group = py.import("built_core.group")?.getattr("Group")?;
            let mesh = definition.getattr("mesh")?;
            let mut swap = false;
            for entity in viewport()?.getattr("scene")?.getattr("selection")?.try_iter()? {
                let entity = entity?;
                // Every entry is asked, as the list the menu once built did.
                swap |= entity.is_instance(&group)? && entity.call_method0("is_component")?.is_truthy()? && !entity.getattr("mesh")?.is(&mesh);
            }
            if swap {
                win.call_method1("replace_selection_with", (&definition,))?;
            } else {
                viewport()?.call_method1("flash_status", ("Select copies of another component first",))?;
            }
        }
        "ckeep" => {
            win.call_method1("keep_component", (&definition, kept.is_none()))?;
        }
        "crename" => {
            win.call_method1("rename_component", (&definition,))?;
        }
        _ => {}
    }
    Ok(none(py))
}

/// Apply a material from the strip or the Library: Paint becomes active.
fn paint_with<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, key) = (a.get_item(0)?, a.get_item(1)?);
    let library = render(py)?.getattr("wgpu_library")?;
    let (kind, rest) = parts(&key)?;
    if kind == "m" {
        library.call_method("_pick_paint", (&win,), Some(&fields!(py; "name" => rest)))?;
    } else {
        let item = library.call_method0("_catalogue")?.call_method1("get", (&key,))?;
        if item.is_truthy()? {
            library.call_method("_pick_paint", (&win,), Some(&fields!(py; "recipe" => item.get_item("recipe")?)))?;
        }
    }
    Ok(none(py))
}

fn measure<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let auto = a.get_item(2)?.is_truthy()?;
    let viewport = win.getattr("viewport")?;
    let scene = viewport.getattr("scene")?;
    let history = || viewport.getattr("history");
    let takeoff = match own(&scene, "_inspector_takeoff")? {
        Some(takeoff) => takeoff,
        None if auto => return Ok(none(py)),
        None => {
            let takeoff = py.import("built_core")?.getattr("InspectorTakeoff")?.call0()?;
            scene.setattr("_inspector_takeoff", &takeoff)?;
            takeoff
        }
    };
    if auto {
        takeoff.call_method1("refresh", (history()?,))?;
        return Ok(none(py));
    }
    let started = std::time::Instant::now();
    waiting(&viewport, || takeoff.call_method1("refresh", (history()?,)))?;
    let spent = started.elapsed().as_secs_f64();
    viewport.call_method1("flash_status", (format!("Measured {} items in {spent:.1} s", takeoff.getattr("count")?.str()?),))?;
    view.setattr("_ui_insp_sig", py.None())?;
    Ok(none(py))
}

pub(super) fn qty_rows<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let units = ui(py)?.getattr("UNIT_TEXT")?;
    let format = builtins(py)?.getattr("format")?;
    let rows = PyList::empty(py);
    for entry in sorted(py, a.get_item(0)?.call_method0("items")?)?.try_iter()? {
        let [unit, value] = unpack::<2>(&entry?)?;
        let shown = format.call1((value, if unit.eq("und")? { ".0f" } else { ".3f" }))?;
        rows.append((units.call_method1("get", (&unit, &unit))?, shown))?;
    }
    Ok(rows.into_any())
}

pub(super) fn export_csv<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let win = a.get_item(0)?;
    let dialog = render(py)?.getattr("shell_files")?.getattr("ShellFileDialog")?;
    let path = dialog.call_method1("getSaveFileName", (&win, "Export quantities CSV", "metrado.csv", "CSV (*.csv);;All files (*)"))?.get_item(0)?;
    if !path.is_truthy()? {
        return Ok(none(py));
    }
    let viewport = win.getattr("viewport")?;
    waiting(&viewport, || {
        let file = builtins(py)?.call_method("open", (&path, "w"), Some(&fields!(py; "encoding" => "utf-8")))?;
        let written = py.import("core.bim")?.call_method1("quantities_csv", (viewport.getattr("scene")?,)).and_then(|text| file.call_method1("write", (text,)));
        file.call_method0("close")?;
        written
    })?;
    viewport.call_method1("flash_status", (format!("Quantities exported to {}", path.str()?),))?;
    Ok(none(py))
}

/// The tag (layer) a node's recipe names: by key, else by label, else by
/// the last part of a folder path.
pub(super) fn layer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (scene, tag) = (a.get_item(0)?, a.get_item(1)?);
    if !tag.is_truthy()? {
        return Ok(none(py));
    }
    let found = scene.call_method1("layer", (&tag,))?;
    let name: String = tag.str()?.extract()?;
    match name.rsplit_once('/') {
        Some((_, last)) if found.is_none() => scene.call_method1("layer", (last,)),
        _ => Ok(found),
    }
}

pub(super) fn tree_groups<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let out = PyList::empty(py);
    let data = attr(&a.get_item(0)?, "_ui_tree")?;
    if !data.is_truthy()? {
        return Ok(out.into_any());
    }
    let (direct, children) = (data.get_item("direct")?, data.get_item("children")?);
    let mut stack = vec![a.get_item(1)?];
    while let Some(key) = stack.pop() {
        out.call_method1("extend", (direct.call_method1("get", (&key, PyTuple::empty(py)))?,))?;
        for child in children.call_method1("get", (&key, PyTuple::empty(py)))?.try_iter()? {
            stack.push(child?);
        }
    }
    Ok(out.into_any())
}

pub(super) fn group_by_uid<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, uid) = (a.get_item(0)?, a.get_item(1)?);
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let text: String = uid.extract()?;
    if let Some(address) = text.strip_prefix('@').filter(|address| address.starts_with('[')) {
        return scene.call_method1("resolve_occurrence", (py.import("json")?.call_method1("loads", (address,))?,));
    }
    for entry in py.import("built_core.structure")?.call_method1("_placements", (&scene,))?.try_iter()? {
        let group = entry?.get_item(0)?;
        if group.getattr("uid")?.eq(&uid)? {
            return Ok(group);
        }
    }
    Ok(none(py))
}

pub(super) fn select<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let viewport = a.get_item(0)?.getattr("viewport")?;
    let scene = viewport.getattr("scene")?;
    let chosen = PyList::empty(py);
    for group in a.get_item(1)?.try_iter()? {
        let group = group?;
        if scene.call_method1("entity_selectable", (&group,))?.is_truthy()? {
            chosen.append(group)?;
        }
    }
    scene.call_method1("select", (chosen.try_iter()?,))?;
    let update = viewport.getattr("update")?;
    viewport.getattr_opt("notify_scene_changed")?.unwrap_or(update).call0()?;
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_tree_model", Mode::Function, tree_model, &["view", "scene", "cv", "readers"], "", ""),
    ("prepare_tree_model", Mode::Function, prepare_tree_model, &["scene"], "", ""),
    ("open_line_type", Mode::Function, open_line_type, &["win", "name", "tags"], "", ""),
    ("_sync_project_rows", Mode::Function, sync_project_rows, &["view", "win"], "", ""),
    ("_tree", Mode::Function, tree, &["view", "win", "*", "discover=False"], "", ""),
    ("_problem_rules", Mode::Function, problem_rules, &["scene", "registry", "nodes", "recipes", "unfiled", "profile=None"], "", ""),
    ("_active_tool_id", Mode::Function, active_tool_id, &["win"], "", ""),
    ("_components_in_model", Mode::Function, components_in_model, &["view", "win"], "", ""),
    ("_library_pick", Mode::Function, library_pick, &["win", "view", "key"], "", ""),
    ("_paint_with", Mode::Function, paint_with, &["win", "key"], "", ""),
    ("_measure", Mode::Function, measure, &["view", "win", "auto=False"], "", ""),
    ("_select", Mode::Function, select, &["win", "groups"], "", ""),
];
