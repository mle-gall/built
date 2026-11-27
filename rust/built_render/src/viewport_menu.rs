// SPDX-License-Identifier: GPL-3.0-or-later
//! The viewport's right-click menu and the inspector's verbs, on the window
//! (docs/context-menu.md). `verbs.rs` decides which rows a selection gets,
//! their labels, keys and checks, the header and what runs each verb. This
//! module reads the document into those [`Facts`], keeps what the menu was
//! opened over, and runs the dispatch the verb table names:
//!
//! - `slot`: the window's Edit-menu slot (`_on_reverse_faces`…);
//! - `tree`: the project tree's own verb on the item row `i:<uid>`;
//! - `core`: a core protocol command on the selected placements;
//! - `glue`: one of the few actions below that need a field, a dialog or
//!   the selection itself.
//!
//! `ui/context_menu.rs` draws the menu and reports `ctx:<id>`. The document,
//! its history and the window slots are still Python; they are reached
//! through their attributes.
use crate::verbs::{self, Facts};
use pyo3::{
    exceptions::PyKeyError,
    prelude::*,
    types::{PyCFunction, PyDict, PyList, PyTuple},
};

// ---- reading Python objects ------------------------------------------------------------

fn opt<'py>(o: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    o.getattr(name).ok().filter(|v| !v.is_none())
}

fn flag(o: &Bound<'_, PyAny>, name: &str) -> bool {
    opt(o, name).is_some_and(|v| v.is_truthy().unwrap_or(false))
}

fn text(o: &Bound<'_, PyAny>, name: &str) -> String {
    opt(o, name).and_then(|v| v.extract::<String>().ok()).unwrap_or_default()
}

fn type_name(o: &Bound<'_, PyAny>) -> String {
    o.get_type().name().map(|n| n.to_string()).unwrap_or_default()
}

fn class<'py>(py: Python<'py>, module: &str, name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import(module)?.getattr(name)
}

fn is_component(g: &Bound<'_, PyAny>) -> bool {
    g.call_method0("is_component").and_then(|v| v.is_truthy()).unwrap_or(false)
}

/// A group's name, else what it is.
fn group_label(g: &Bound<'_, PyAny>) -> String {
    let name = text(g, "name");
    if !name.is_empty() {
        name
    } else if is_component(g) {
        "Component".into()
    } else {
        "Group".into()
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The `path` of a link record's file, by name.
fn record_file(rec: &Bound<'_, PyAny>) -> String {
    rec.call_method1("get", ("path", "")).and_then(|p| p.extract::<String>()).map(|p| file_name(&p)).unwrap_or_default()
}

fn guide(py: Python<'_>, e: &Bound<'_, PyAny>) -> bool {
    py.import("built_core.guide")
        .and_then(|m| m.call_method1("is_guide", (e,)))
        .and_then(|v| v.is_truthy())
        .unwrap_or(false)
}

/// `built_core.units.<fmt>(value)`, "" when the value or the format fails.
fn measure(py: Python<'_>, fmt: &str, value: PyResult<f64>) -> String {
    value
        .and_then(|v| py.import("built_core.units")?.call_method1(fmt, (v,))?.extract::<String>())
        .unwrap_or_default()
}

/// The value the last command runs with, when it has one (Push/Pull's
/// distance, which a double-click repeats).
fn repeat_value(py: Python<'_>, win: &Bound<'_, PyAny>, label: &Bound<'_, PyAny>) -> PyResult<String> {
    let Some(tools) = opt(win, "_tools").filter(|t| t.is_truthy().unwrap_or(false)) else {
        return Ok(String::new());
    };
    for tool in tools.call_method0("values")?.try_iter()? {
        let tool = tool?;
        if !opt(&tool, "name").is_some_and(|n| n.eq(label).unwrap_or(false)) {
            continue;
        }
        let number = tool.get_type().getattr("last_distance").ok().and_then(|v| v.extract::<f64>().ok());
        if let Some(v) = number.filter(|v| *v != 0.0) {
            return Ok(measure(py, "fmt_len", Ok(v.abs())));
        }
    }
    Ok(String::new())
}

/// Whether Hide has anything to hide (`_hideable` is not empty), stopping
/// at the first: the full list was an `is_hidden` per entity of a
/// select-all, on every change of the selection.
fn any_hideable(entities: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = entities.py();
    let mesh = py.import("built_core.mesh")?;
    let types = (mesh.getattr("Edge")?, mesh.getattr("Face")?, py.import("built_core.group")?.getattr("Group")?).into_pyobject(py)?;
    let hidden = py.import("built_core")?.getattr("is_hidden")?;
    for e in entities.try_iter()? {
        let e = e?;
        if e.is_instance(&types)? && !hidden.call1((&e,))?.is_truthy()? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Where loose geometry lives: the groups open for editing, else Model.
fn context_path(py: Python<'_>, scene: &Bound<'_, PyAny>) -> PyResult<String> {
    let open = py.import("built_core.outliner")?.call_method1("open_path", (scene,))?;
    let mut names = Vec::new();
    for g in open.try_iter()? {
        let g = g?;
        names.push(if g.is_none() { "Group".into() } else { group_label(&g) });
    }
    Ok(if names.is_empty() { "Model".into() } else { names.join(" › ") })
}

/// The group's node path in the project tree, or "Not filed".
fn filed_path(py: Python<'_>, scene: &Bound<'_, PyAny>, g: &Bound<'_, PyAny>) -> String {
    let path = || -> PyResult<Option<String>> {
        let structure = py.import("built_core.structure")?;
        let key = structure.call_method1("node_of", (g,))?;
        if !key.is_truthy()? {
            return Ok(None);
        }
        let registry = py.import("core.bim_registry")?.call_method1("get", (scene,))?;
        let parts: Vec<String> = structure.call_method1("path", (registry, key))?.extract()?;
        Ok(Some(parts.join(" › ")))
    };
    // A stale key: no path, not an error.
    path().ok().flatten().unwrap_or_else(|| "Not filed".into())
}

// ---- the facts ---------------------------------------------------------------------------

/// What the verb table needs to know about the selection `sel`: counts and
/// flags read from the document, and the header's names and measures. With
/// `win`, also what the window can do now (Repeat, Hide, Paste…).
/// A selection split by kind: groups, faces, edges and the rest, and the
/// selected edges' keys when the selection keeps them.
pub struct Kinds<'py> {
    groups: Vec<Bound<'py, PyAny>>,
    faces: Vec<Bound<'py, PyAny>>,
    edges: Vec<Bound<'py, PyAny>>,
    others: Vec<Bound<'py, PyAny>>,
    edge_keys: Option<Bound<'py, PyAny>>,
}

/// Past this many entries a selection is read by kind (`values_of`).
const HUGE_SELECTION: usize = 50_000;

impl<'py> Kinds<'py> {
    /// One pass, the commonest kinds first.
    fn of(sel: &[Bound<'py, PyAny>]) -> PyResult<Self> {
        let py = match sel.first() {
            Some(e) => e.py(),
            None => return Ok(Self { groups: vec![], faces: vec![], edges: vec![], others: vec![], edge_keys: None }),
        };
        let group_cls = class(py, "built_core.group", "Group")?;
        let face_cls = class(py, "built_core.mesh", "Face")?;
        let edge_cls = class(py, "built_core.mesh", "Edge")?;
        let mut k = Self { groups: vec![], faces: vec![], edges: vec![], others: vec![], edge_keys: None };
        for e in sel {
            if e.is_instance(&face_cls).unwrap_or(false) {
                k.faces.push(e.clone());
            } else if e.is_instance(&edge_cls).unwrap_or(false) {
                k.edges.push(e.clone());
            } else if e.is_instance(&group_cls).unwrap_or(false) {
                k.groups.push(e.clone());
            } else {
                k.others.push(e.clone());
            }
        }
        Ok(k)
    }

    /// A huge selection by the selection's own lists of each kind (a
    /// select-all of a big model: no handle type-tested), else None.
    fn bulk(selection: &Bound<'py, PyAny>) -> PyResult<Option<(Vec<Bound<'py, PyAny>>, Self)>> {
        if !selection.hasattr("values_of")? || selection.len()? < HUGE_SELECTION {
            return Ok(None);
        }
        let py = selection.py();
        let group_cls = class(py, "built_core.group", "Group")?;
        let face_cls = class(py, "built_core.mesh", "Face")?;
        let edge_cls = class(py, "built_core.mesh", "Edge")?;
        let of = |kind: &str| -> PyResult<Vec<Bound<'py, PyAny>>> { selection.call_method1("values_of", (kind,))?.try_iter()?.collect() };
        let mut k = Self { groups: vec![], faces: of("face")?, edges: of("edge")?, others: vec![], edge_keys: None };
        // Store views of other meshes are faces and edges all the same.
        if k.faces.iter().take(1).chain(k.edges.iter().take(1)).any(|e| !e.is_instance(&face_cls).unwrap_or(false) && !e.is_instance(&edge_cls).unwrap_or(false)) {
            return Ok(None);
        }
        for e in of("placement")? {
            if e.is_instance(&group_cls)? { k.groups.push(e) } else { k.others.push(e) }
        }
        k.others.extend(of("vertex")?);
        k.others.extend(of("other")?);
        k.edge_keys = Some(selection.call_method1("element_keys", ("edge",))?);
        let all = k.groups.iter().chain(&k.faces).chain(&k.edges).chain(&k.others).cloned().collect();
        Ok(Some((all, k)))
    }
}

pub fn read_facts(
    scene: &Bound<'_, PyAny>,
    sel: &[Bound<'_, PyAny>],
    win: Option<&Bound<'_, PyAny>>,
    view: Option<&Bound<'_, PyAny>>,
    locked_group: Option<&Bound<'_, PyAny>>,
    locked_image: Option<&Bound<'_, PyAny>>,
) -> PyResult<Facts> {
    read_facts_of(scene, sel, Kinds::of(sel)?, win, view, locked_group, locked_image)
}

#[allow(clippy::too_many_arguments)]
fn read_facts_of<'py>(
    scene: &Bound<'py, PyAny>,
    sel: &[Bound<'py, PyAny>],
    kinds: Kinds<'py>,
    win: Option<&Bound<'py, PyAny>>,
    view: Option<&Bound<'py, PyAny>>,
    locked_group: Option<&Bound<'py, PyAny>>,
    locked_image: Option<&Bound<'py, PyAny>>,
) -> PyResult<Facts> {
    let py = scene.py();
    let links = py.import("built_core.links")?;
    let group_cls = class(py, "built_core.group", "Group")?;
    let Kinds { groups, faces, edges, others, edge_keys } = kinds;
    let linked = |g: &Bound<'_, PyAny>| links.call_method1("is_linked", (g,)).and_then(|v| v.is_truthy()).unwrap_or(false);
    fn ancestor<'py>(links: &Bound<'py, PyModule>, g: &Bound<'py, PyAny>) -> Option<Bound<'py, PyAny>> {
        let _ = links;
        crate::host_context::core_module(g.py()).ok()?.call_method1("link_linked_ancestor", (g,)).ok().filter(|v| !v.is_none())
    }
    let all = |f: &dyn Fn(&Bound<'_, PyAny>) -> bool| !sel.is_empty() && sel.iter().all(f);
    // Only what is neither a face, an edge nor a group can be of these.
    let all_type = |names: &[&str]| !sel.is_empty() && others.len() == sel.len()
        && others.iter().all(|e| names.contains(&type_name(e).as_str()));
    let linked_part = groups.iter().any(|g| ancestor(&links, g).is_some() && !linked(g));
    let mut f = Facts {
        total: sel.len(),
        groups: groups.len(),
        faces: faces.len(),
        edges: edges.len(),
        painted_group: groups.iter().any(|g| flag(g, "material")),
        components: groups.iter().filter(|g| is_component(g)).count(),
        linked_part,
        all_links: !groups.is_empty() && groups.iter().all(linked),
        all_annotations: all_type(&["Dimension", "TextLabel"]),
        all_guides: all(&|e| guide(py, e)),
        all_sections: all_type(&["SectionPlane"]),
        all_images: all_type(&["ImagePlane"]),
        all_paths: all_type(&["GeoPath"]),
        all_locked: !groups.is_empty() && groups.iter().all(|g| flag(g, "locked")),
        ..Facts::default()
    };
    let Some(win) = win else {
        return Ok(f);
    };
    let vp = win.getattr("viewport")?;
    let one = if sel.len() == 1 { Some(&sel[0]) } else { None };
    let contains = |x: &Bound<'_, PyAny>| sel.iter().any(|e| e.is(x));
    if let Some(actions) = opt(win, "_registry_actions") {
        let action = actions.call_method1("get", ("menu:repeat_last_command",))?;
        if !action.is_none() {
            let keys: Vec<String> = action.call_method0("portable_keys")?.extract()?;
            f.repeat_key = Some(keys.into_iter().next().unwrap_or_default());
        }
    }
    if let Some(last) = opt(win, "_last_command") {
        let label = last.get_item(0)?;
        f.repeat = Some(label.str()?.to_string());
        f.repeat_value = repeat_value(py, win, &label)?;
    }
    if let Some(g) = locked_group.filter(|g| !contains(g)) {
        f.locked_group = Some(group_label(g));
    }
    if let Some(im) = locked_image.filter(|im| !contains(im)) {
        let name = text(im, "name");
        f.locked_image = Some(if name.is_empty() { "image".into() } else { name });
    }
    let geometry = !sel.is_empty() && faces.len() + edges.len() == sel.len();
    let selection = PyList::new(py, sel)?;
    f.taggable = crate::tree_grouping::tag_visibility::has_tag_actions(scene, selection.as_any())?;
    f.hideable = any_hideable(&selection)?;
    f.makes_faces = geometry && win.call_method1("_makes_faces", (&selection,))?.is_truthy()?;
    f.textured_face = faces.len() == 1 && sel.len() == 1 && !win.call_method0("_single_textured_face")?.is_none();
    f.bridgeable = geometry && win.call_method1("_bridgeable", (&selection,))?.is_truthy()?;
    f.context = context_path(py, scene)?;
    let store = opt(&scene.getattr("mesh")?, "_store").filter(|s| s.hasattr("stale_keys").unwrap_or(false) && s.hasattr("curved_edges").unwrap_or(false));
    match store {
        // Many edges: which are the active mesh's, and whether one is a
        // curve's, from the store in two calls.
        Some(store) if edges.len() > 256 => {
            let keys = match edge_keys {
                Some(keys) => keys.cast_into::<PyList>()?,
                None => PyList::new(py, edges.iter().map(|e| e.getattr("key")).collect::<PyResult<Vec<_>>>()?)?,
            };
            // The keys are distinct: count the stale ones, and build the
            // selected set only when some edge is a curve's.
            let stale: std::collections::HashSet<u128> = store.call_method1("stale_keys", ("edge", &keys))?.extract::<Vec<u128>>()?.into_iter().collect();
            f.loose_edges = keys.len() - stale.len();
            let curved: Vec<(i128, u128)> = store.call_method0("curved_edges")?.extract()?;
            f.curve = !curved.is_empty() && {
                let mine: std::collections::HashSet<u128> = keys.extract::<Vec<u128>>()?.into_iter().filter(|k| !stale.contains(k)).collect();
                curved.iter().any(|(_, key)| mine.contains(key))
            };
        }
        _ => {
            let mesh_edges = scene.getattr("mesh")?.getattr("edges")?;
            let loose: Vec<_> = edges.iter().filter(|e| mesh_edges.contains(*e).unwrap_or(false)).collect();
            f.loose_edges = loose.len();
            f.curve = loose.iter().any(|e| opt(e, "curve").is_some());
        }
    }
    if let Some(one) = one.filter(|o| o.is_instance(&group_cls).unwrap_or(false)) {
        let editable = crate::context_tools::text_editable(scene, one)?;
        f.name = text(one, "name");
        f.filed = filed_path(py, scene, one);
        f.text3d = editable.is_some();
        f.edited_letters = editable == Some(false);
        f.billboard = flag(one, "billboard");
        if is_component(one) {
            let mesh = one.getattr("mesh")?;
            let roots = opt(&scene, "groups").filter(|g| g.is_truthy().unwrap_or(false));
            let roots = match roots {
                Some(r) => r,
                None => PyTuple::empty(py).into_any(),
            };
            let mut instances = 0;
            for o in py.import("built_core")?.call_method1("purge_iter_groups", (roots,))?.try_iter()? {
                instances += usize::from(o?.getattr("mesh")?.is(&mesh));
            }
            f.instances = instances;
        }
        let rec = links.call_method1("link_of", (one,))?;
        if rec.is_truthy()? {
            f.file = record_file(&rec);
            f.include_nested = Some(rec.call_method1("get", ("include_nested", true))?.is_truthy()?);
        }
    }
    if f.components > 0 && groups.len() == sel.len() && !f.linked_part && !f.all_links {
        f.replaceable = win.call_method1("_replacement_choices", (&selection,))?.is_truthy()?;
        let kept = py.import("built_core.kept_components")?;
        f.kept = true;
        for group in groups.iter().filter(|g| is_component(g)) {
            if kept.call_method1("kept_template", (scene, group.getattr("mesh")?))?.is_none() {
                f.kept = false;
                break;
            }
        }
    }
    if linked_part {
        let part = &groups[0];
        f.name = text(part, "name");
        let host = ancestor(&links, part);
        f.host = host.is_some();
        if let Some(host) = host {
            let rec = links.call_method1("link_of", (&host,))?;
            f.host_file = if rec.is_truthy()? { record_file(&rec) } else { String::new() };
        }
    }
    if groups.len() > 1 && groups.len() == sel.len() {
        let solids = py.import("built_core.solids")?;
        f.solids = groups.iter().filter(|g| solids.call_method1("is_solid", (*g,)).and_then(|v| v.is_truthy()).unwrap_or(false)).count();
    }
    if faces.len() == 1 {
        f.area = measure(py, "fmt_area", faces[0].call_method0("area").and_then(|v| v.extract()));
    }
    if edges.len() == 1 {
        let e = &edges[0];
        let length = (|| e.getattr("b")?.sub(e.getattr("a")?)?.call_method0("length")?.extract())();
        f.length = measure(py, "fmt_len", length);
    }
    if let Some(one) = one {
        let kind = type_name(one);
        if kind == "Dimension" || kind == "TextLabel" {
            f.dimension = kind == "Dimension";
            let label = || -> PyResult<String> {
                if f.dimension {
                    let t = text(one, "text");
                    if !t.is_empty() {
                        return Ok(t);
                    }
                    return one.call_method0("label")?.extract();
                }
                let t = text(one, "text");
                let first = t.trim().lines().next().unwrap_or("");
                Ok(first.chars().take(40).collect())
            };
            f.text = label().unwrap_or_default();
        }
        if guide(py, one) {
            f.guide_point = opt(one, "direction").is_none() || kind == "GuidePoint";
        }
        if kind == "SectionPlane" || kind == "ImagePlane" {
            f.name = text(one, "name");
            f.section_active = flag(one, "active");
            f.image_locked = flag(one, "locked");
        }
    }
    f.path_polygon = others.iter().any(|e| {
        type_name(e) == "GeoPath" && opt(e, "points").and_then(|p| p.len().ok()).unwrap_or(0) >= 3
    });
    if sel.is_empty() {
        let clip = flag(&vp, "clipboard");
        f.paste = clip
            || match opt(win, "_clipboard_available") {
                Some(available) => available.call0()?.is_truthy()?,
                None => py.import("built_render")?.getattr("shell_files")?.getattr("_clipboard")?.call_method0("available")?.is_truthy()?,
            };
        let own = vp.getattr("__dict__")?.call_method1("get", ("_isolate_base",))?;
        f.isolated = !own.is_none() || flag(&vp, "_previous_views");
        f.guides = opt(scene, "guides").is_some_and(|g| g.len().unwrap_or(0) > 0 || g.try_iter().is_ok_and(|mut i| i.next().is_some()));
    } else if view.is_some() {
        f.reveal_key = crate::shortcut_text::shortcut_text("Ctrl+Shift+J").map(|s| s.1);
    }
    Ok(f)
}

fn items<'py>(sel: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    sel.try_iter()?.collect()
}

fn facts_dict<'py>(py: Python<'py>, f: &Facts) -> PyResult<Bound<'py, PyAny>> {
    let json = serde_json::to_string(f).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    py.import("json")?.call_method1("loads", (json,))
}

/// Facts a caller kept as a dict (the menu's `facts`), or none.
fn facts_from(py: Python<'_>, facts: Option<&Bound<'_, PyAny>>) -> Facts {
    let Some(facts) = facts.filter(|f| !f.is_none()) else {
        return Facts::default();
    };
    if let Ok(menu) = facts.cast::<ViewportMenu>() {
        return menu.borrow().facts.clone();
    }
    py.import("json")
        .and_then(|j| j.call_method1("dumps", (facts,)))
        .and_then(|s| s.extract::<String>())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// The facts of the selection `sel`, as a dict (tests and the inspector).
#[pyfunction]
#[pyo3(signature = (scene, sel, win=None, view=None, *, locked_group=None, locked_image=None))]
fn selection_facts<'py>(
    scene: &Bound<'py, PyAny>,
    sel: &Bound<'py, PyAny>,
    win: Option<Bound<'py, PyAny>>,
    view: Option<Bound<'py, PyAny>>,
    locked_group: Option<Bound<'py, PyAny>>,
    locked_image: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let sel = items(sel)?;
    let f = read_facts(scene, &sel, win.as_ref(), view.as_ref(), locked_group.as_ref(), locked_image.as_ref())?;
    facts_dict(scene.py(), &f)
}

/// The kind of the selection `sel` (docs/context-menu.md).
pub fn kind_of(scene: &Bound<'_, PyAny>, sel: &Bound<'_, PyAny>) -> PyResult<String> {
    let f = match Kinds::bulk(sel)? {
        Some((all, kinds)) => read_facts_of(scene, &all, kinds, None, None, None, None)?,
        None => read_facts(scene, &items(sel)?, None, None, None, None)?,
    };
    Ok(verbs::kind(&f).to_owned())
}

#[pyfunction]
fn selection_kind(scene: &Bound<'_, PyAny>, sel: &Bound<'_, PyAny>) -> PyResult<String> {
    kind_of(scene, sel)
}

/// The verb ids of a linked file's tree row.
#[pyfunction]
#[pyo3(signature = (nested, state="resolved"))]
fn file_verb_ids(nested: bool, state: &str) -> Vec<String> {
    verbs::file_verbs(nested, state).into_iter().map(|(v, _)| v).collect()
}

// ---- the menu ----------------------------------------------------------------------------

/// One entry of a menu row.
#[pyclass(module = "built_render", get_all, from_py_object)]
#[derive(Clone)]
pub struct MenuVerb {
    id: String,
    label: String,
    key: String,
    enabled: bool,
    /// -1: no check; 0 / 1: a check mark, off / on.
    checked: i8,
}

#[pymethods]
impl MenuVerb {
    #[new]
    #[pyo3(signature = (id, label, key=String::new(), enabled=true, checked=-1))]
    fn new(id: String, label: String, key: String, enabled: bool, checked: i8) -> Self {
        Self { id, label, key, enabled, checked }
    }
}

/// A row: plain, or a segmented family ("Intersect: Model | Selection").
#[pyclass(module = "built_render", get_all, from_py_object)]
#[derive(Clone)]
pub struct MenuRow {
    verbs: Vec<MenuVerb>,
    /// A segmented row's family name ("Intersect"); "" for a plain row.
    label: String,
}

#[pymethods]
impl MenuRow {
    /// The row as docs/context-menu.md writes it: "Hide",
    /// "Explode | Make component…", "Intersect: Model | Selection | Context".
    fn text(&self) -> String {
        let verbs = self.verbs.iter().map(|v| v.label.as_str()).collect::<Vec<_>>().join(" | ");
        if self.label.is_empty() { verbs } else { format!("{}: {verbs}", self.label) }
    }
}

/// An open right-click menu: its rows and what it acts on (the selection and
/// the locked things under the pointer).
#[pyclass(module = "built_render")]
pub struct ViewportMenu {
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    title: String,
    #[pyo3(get)]
    subtitle: String,
    #[pyo3(get)]
    rows: Vec<MenuRow>,
    /// Where it was opened (logical points, the view's coordinates).
    #[pyo3(get, set)]
    at: (f64, f64),
    sel: Vec<Py<PyAny>>,
    locked_group: Option<Py<PyAny>>,
    locked_image: Option<Py<PyAny>>,
    facts: Facts,
    /// A tool's own menu (Position Texture) answers its verbs.
    tool: Option<Py<PyAny>>,
}

fn entries(rows: Vec<verbs::Row>) -> Vec<MenuRow> {
    rows.into_iter()
        .map(|(label, verbs)| MenuRow {
            label,
            verbs: verbs
                .into_iter()
                .map(|(id, label, key, enabled, checked)| MenuVerb { id, label, key, enabled, checked })
                .collect(),
        })
        .collect()
}

impl ViewportMenu {
    fn all(&self) -> impl Iterator<Item = &MenuVerb> {
        self.rows.iter().flat_map(|r| r.verbs.iter())
    }
}

#[pymethods]
impl ViewportMenu {
    /// A tool's own menu: `rows` are `[(family, [(id, label, enabled)])]`.
    #[staticmethod]
    #[pyo3(signature = (tool, title, rows, at=(0.0, 0.0)))]
    fn for_tool(tool: Py<PyAny>, title: String, rows: Vec<(String, Vec<(String, String, bool)>)>, at: (f64, f64)) -> Self {
        let rows = rows
            .into_iter()
            .map(|(label, verbs)| MenuRow {
                label,
                verbs: verbs.into_iter().map(|(id, label, enabled)| MenuVerb { id, label, key: String::new(), enabled, checked: -1 }).collect(),
            })
            .collect();
        Self {
            kind: "texture_position".into(),
            title,
            subtitle: String::new(),
            rows,
            at,
            sel: Vec::new(),
            locked_group: None,
            locked_image: None,
            facts: Facts::default(),
            tool: Some(tool),
        }
    }

    #[getter]
    fn sel(&self, py: Python<'_>) -> Vec<Py<PyAny>> {
        self.sel.iter().map(|e| e.clone_ref(py)).collect()
    }

    #[getter]
    fn locked_group(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.locked_group.as_ref().map(|e| e.clone_ref(py))
    }

    #[getter]
    fn locked_image(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.locked_image.as_ref().map(|e| e.clone_ref(py))
    }

    #[getter]
    fn tool(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.tool.as_ref().map(|e| e.clone_ref(py))
    }

    #[getter]
    fn facts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        facts_dict(py, &self.facts)
    }

    fn verbs(&self) -> Vec<MenuVerb> {
        self.all().cloned().collect()
    }

    fn find(&self, vid: &str) -> Option<MenuVerb> {
        self.all().find(|v| v.id == vid).cloned()
    }

    /// Each row as text (tests, docs): "Hide", "Intersect: Model | …".
    fn labels(&self) -> Vec<String> {
        self.rows.iter().map(MenuRow::text).collect()
    }

    fn ids(&self) -> Vec<String> {
        self.all().map(|v| v.id.clone()).collect()
    }
}

/// The menu for the current selection, or `sel` (the right-click has
/// already selected what it hit, see [`context_menu_open_at`]).
pub fn build<'py>(
    win: &Bound<'py, PyAny>,
    view: Option<&Bound<'py, PyAny>>,
    sel: Option<&Bound<'py, PyAny>>,
    locked_image: Option<&Bound<'py, PyAny>>,
    locked_group: Option<&Bound<'py, PyAny>>,
) -> PyResult<ViewportMenu> {
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let sel = match sel.filter(|s| !s.is_none()) {
        Some(s) => items(s)?,
        None => items(&scene.getattr("selection")?)?,
    };
    let f = read_facts(&scene, &sel, Some(win), view, locked_group, locked_image)?;
    let kind = verbs::kind(&f);
    let (title, subtitle) = verbs::header(&f, kind);
    Ok(ViewportMenu {
        kind: kind.to_owned(),
        title,
        subtitle,
        rows: entries(verbs::menu_rows(&f, kind)),
        at: (0.0, 0.0),
        sel: sel.into_iter().map(Bound::unbind).collect(),
        locked_group: locked_group.map(|g| g.clone().unbind()),
        locked_image: locked_image.map(|g| g.clone().unbind()),
        facts: f,
        tool: None,
    })
}

#[pyfunction]
#[pyo3(signature = (win, view=None, sel=None, *, locked_image=None, locked_group=None))]
fn context_menu_build<'py>(
    win: &Bound<'py, PyAny>,
    view: Option<Bound<'py, PyAny>>,
    sel: Option<Bound<'py, PyAny>>,
    locked_image: Option<Bound<'py, PyAny>>,
    locked_group: Option<Bound<'py, PyAny>>,
) -> PyResult<ViewportMenu> {
    build(win, view.as_ref(), sel.as_ref(), locked_image.as_ref(), locked_group.as_ref())
}

// ---- running a verb ------------------------------------------------------------------------

fn none_or<'py>(py: Python<'py>, o: Option<&Bound<'py, PyAny>>) -> Bound<'py, PyAny> {
    o.cloned().unwrap_or_else(|| py.None().into_bound(py))
}

/// The view a window command reaches (`None` when headless).
fn view_or_none<'py>(py: Python<'py>, view: Option<&Bound<'py, PyAny>>) -> Bound<'py, PyAny> {
    none_or(py, view.filter(|v| !v.is_none()))
}

fn sep(py: Python<'_>) -> PyResult<String> {
    py.import("views.wgpu_ui")?.getattr("SEP")?.extract()
}

/// Run the project tree's own verb on the item row of `g`.
fn tree(win: &Bound<'_, PyAny>, view: Option<&Bound<'_, PyAny>>, verb: &str, g: &Bound<'_, PyAny>, extra: Option<&str>) -> PyResult<()> {
    let py = win.py();
    let sep = sep(py)?;
    let mut key = format!("{verb}{sep}i:{}", g.getattr("uid")?.str()?);
    if let Some(extra) = extra {
        key.push_str(&sep);
        key.push_str(extra);
    }
    py.import("views.wgpu_ui")?.call_method1("_tree_action", (win, view_or_none(py, view), key))?;
    Ok(())
}

fn uids<'py>(sel: &[Bound<'py, PyAny>]) -> Vec<Bound<'py, PyAny>> {
    sel.iter().filter_map(|g| opt(g, "uid").filter(|u| u.is_truthy().unwrap_or(false))).collect()
}

fn groups_of<'py>(sel: &[Bound<'py, PyAny>]) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let Some(first) = sel.first() else { return Ok(Vec::new()) };
    let cls = class(first.py(), "built_core.group", "Group")?;
    Ok(sel.iter().filter(|e| e.is_instance(&cls).unwrap_or(false)).cloned().collect())
}

/// The linked file a part or a link placement belongs to.
fn link_host<'py>(sel: &[Bound<'py, PyAny>]) -> PyResult<Option<Bound<'py, PyAny>>> {
    let Some(part) = groups_of(sel)?.into_iter().next() else { return Ok(None) };
    let links = part.py().import("built_core.links")?;
    if links.call_method1("is_linked", (&part,))?.is_truthy()? {
        return Ok(Some(part));
    }
    Ok(Some(crate::host_context::core_module(part.py())?.call_method1("link_linked_ancestor", (&part,))?).filter(|h| !h.is_none()))
}

/// Rename… : the chrome's name prompt, answered into the tree's own rename
/// (`rename` on the row `i:<uid>`, core `rename_placement`).
fn ask_rename(win: &Bound<'_, PyAny>, view: Option<&Bound<'_, PyAny>>, g: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let (w, v, target) = (win.clone().unbind(), view.map(|v| v.clone().unbind()), g.clone().unbind());
    let done = PyCFunction::new_closure(py, None, None, move |args, _kwargs| -> PyResult<()> {
        let py = args.py();
        let answer = args.get_item(0)?;
        let answer = if answer.is_none() { String::new() } else { answer.extract::<String>()? };
        let answer = answer.trim();
        if !answer.is_empty() {
            let view = v.as_ref().map(|v| v.bind(py).clone());
            tree(w.bind(py), view.as_ref(), "rename", target.bind(py), Some(answer))?;
        }
        Ok(())
    })?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("text", text(g, "name"))?;
    kwargs.set_item("done", done)?;
    py.import("built_render.chrome_forms")?.getattr("ask_text")?.call((win, "Rename", "Name:"), Some(&kwargs))?;
    Ok(())
}

/// Edit text… : what a double-click on the text or dimension does (the
/// Select tool's own handler), at the point the menu was opened on.
fn edit_text(win: &Bound<'_, PyAny>, view: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let Some(px) = opt(&vp, "_context_pixel") else { return Ok(()) };
    let tools = view.and_then(|v| opt(v, "tools")).filter(|t| t.is_truthy().unwrap_or(false)).unwrap_or_else(|| vp.clone());
    let input = py.import("built_core")?;
    let point = input.getattr("ScreenPoint")?.call1((px.get_item(0)?.extract::<f64>()?, px.get_item(1)?.extract::<f64>()?))?;
    let no_modifier = input.getattr("Input")?.getattr("NoModifier")?;
    let origin = class(py, "built_core", "Vec3")?.call0()?;
    let ctx = class(py, "tools.base", "ToolContext")?.call1((tools, origin, point, no_modifier, py.None()))?;
    class(py, "tools.rust_tools", "SelectTool")?.call0()?.call_method1("on_double_click", (ctx,))?;
    Ok(())
}

fn glue(
    win: &Bound<'_, PyAny>,
    view: Option<&Bound<'_, PyAny>>,
    sel: &[Bound<'_, PyAny>],
    name: &str,
    arg: &str,
    f: &Facts,
) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let ui = || py.import("views.wgpu_ui");
    let first = || sel.first().ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("nothing selected"));
    match name {
        "grow" => {
            let scene = vp.getattr("scene")?;
            let current = scene.getattr("selection")?;
            let ops = py.import("built_core")?;
            let found = match arg {
                "all_connected" | "bounding_edges" => ops.call_method1(arg, (current,))?,
                "same_material" | "same_layer" | "same_definition" =>
                    ops.call_method1(arg, (&scene, current))?,
                _ => return Err(PyKeyError::new_err(arg.to_owned())),
            };
            ui()?.call_method1("_select", (win, found))?;
            let vars = PyDict::new(py);
            vars.set_item("n", scene.getattr("selection")?.len()?)?;
            let msg = crate::model_edit::translated(py, "{n} entities selected", Some(&vars))?;
            vp.call_method1("flash_status", (msg,))?;
        }
        "selectiontag" => crate::tree_grouping::tag_visibility::viewport_tag_menu(win, PyList::new(py, sel)?.as_any())?,
        "intersect" => {
            let intersect = py.import("built_core.intersect")?;
            let mode = match arg {
                "model" => "WITH_MODEL",
                "selection" => "WITH_SELECTION",
                "context" => "WITH_CONTEXT",
                _ => return Err(PyKeyError::new_err(arg.to_owned())),
            };
            win.call_method1("_on_intersect_faces", (intersect.getattr(mode)?,))?;
        }
        "isolate" => {
            ui()?.call_method1("_isolate", (win, uids(&groups_of(sel)?)))?;
        }
        "fileto" => {
            crate::document_registration::view::calls::file_to(win, groups_of(sel)?.len())?;
        }
        "lock" => crate::context_edits::context_lock(&vp, groups_of(sel)?, !f.all_locked)?,
        "keep" => {
            win.call_method1("keep_selected_components", (!f.kept,))?;
        }
        "contents" => {
            let scene = vp.getattr("scene")?;
            let mut picks = Vec::new();
            for p in sel {
                if let Some(children) = opt(p, "children") {
                    for c in children.try_iter()? {
                        let c = c?;
                        if scene.call_method1("entity_selectable", (&c,))?.is_truthy()? {
                            picks.push(c);
                        }
                    }
                }
            }
            let picks = if picks.is_empty() { sel.to_vec() } else { picks };
            ui()?.call_method1("_select", (win, picks))?;
        }
        "link" => {
            if let Some(target) = link_host(sel)? {
                ui()?.call_method1("_link_action", (win, view_or_none(py, view), target, arg))?;
            }
        }
        "selectlink" => {
            if let Some(host) = link_host(sel)? {
                ui()?.call_method1("_select", (win, vec![host]))?;
            }
        }
        "deleteguide" => {
            let cmd = class(py, "built_core.history", "DeleteGuidesCommand")?.call1((PyList::new(py, sel)?,))?;
            vp.getattr("history")?.call_method1("execute", (cmd,))?;
            vp.call_method0("update")?;
        }
        "showall" => {
            ui()?.call_method1("_show_all", (&vp,))?;
        }
        "secname" => {
            let kwargs = PyDict::new(py);
            kwargs.set_item("automatic", false)?;
            win.getattr("prompt_section_name")?.call((first()?,), Some(&kwargs))?;
        }
        "reveal" => {
            ui()?.call_method1("_tree_action", (win, view_or_none(py, view), format!("revealselection{}", sep(py)?)))?;
        }
        "rename" => ask_rename(win, view, first()?)?,
        "edittext" => edit_text(win, view)?,
        _ => return Err(PyKeyError::new_err(name.to_owned())),
    }
    Ok(())
}

/// Run verb `vid` of a `kind` selection `sel` as the verb table says. False
/// when it names nothing to run.
#[allow(clippy::too_many_arguments)]
pub fn run(
    win: &Bound<'_, PyAny>,
    view: Option<&Bound<'_, PyAny>>,
    kind: &str,
    vid: &str,
    sel: &[Bound<'_, PyAny>],
    locked_group: Option<&Bound<'_, PyAny>>,
    locked_image: Option<&Bound<'_, PyAny>>,
    f: &Facts,
) -> PyResult<bool> {
    let py = win.py();
    let (how, name, arg) = verbs::dispatch(kind, vid);
    match how {
        "slot" => {
            let Some(slot) = opt(win, &name) else { return Ok(false) };
            let face = class(py, "built_core.mesh", "Face")?;
            let args: Option<Vec<Bound<'_, PyAny>>> = match arg.as_str() {
                "" => Some(Vec::new()),
                "sel0" => sel.first().map(|e| vec![e.clone()]),
                "none" => Some(vec![py.None().into_bound(py)]),
                "face0" => sel.iter().find(|e| e.is_instance(&face).unwrap_or(false)).map(|e| vec![e.clone()]),
                "locked_group" => Some(vec![none_or(py, locked_group)]),
                "locked_image" => Some(vec![none_or(py, locked_image)]),
                other => Some(vec![other.into_pyobject(py)?.into_any()]),
            };
            let Some(args) = args else { return Ok(false) };
            slot.call1(PyTuple::new(py, args)?)?;
        }
        "tree" => {
            let Some(first) = sel.first() else { return Ok(false) };
            tree(win, view, &name, first, None)?;
        }
        "core" => {
            let vp = win.getattr("viewport")?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("placements", uids(sel))?;
            crate::document_registration::view::calls::core_command(&vp, &name, Some(&kwargs))?;
            vp.call_method0("update")?;
        }
        "glue" => glue(win, view, sel, &name, &arg, f)?,
        _ => return Ok(false),
    }
    Ok(true)
}

#[pyfunction]
#[pyo3(signature = (win, view, kind, vid, sel, *, locked_group=None, locked_image=None, facts=None))]
#[allow(clippy::too_many_arguments)]
fn context_menu_run(
    win: &Bound<'_, PyAny>,
    view: Option<Bound<'_, PyAny>>,
    kind: &str,
    vid: &str,
    sel: &Bound<'_, PyAny>,
    locked_group: Option<Bound<'_, PyAny>>,
    locked_image: Option<Bound<'_, PyAny>>,
    facts: Option<Bound<'_, PyAny>>,
) -> PyResult<bool> {
    let f = facts_from(win.py(), facts.as_ref());
    let view = view.filter(|v| !v.is_none());
    run(win, view.as_ref(), kind, vid, &items(sel)?, locked_group.as_ref(), locked_image.as_ref(), &f)
}

/// One inspector verb, run as the right-click menu runs it.
#[pyclass(module = "built_render")]
struct SelectionVerb {
    win: Py<PyAny>,
    view: Option<Py<PyAny>>,
    kind: String,
    vid: String,
    /// Shared by every verb of one row: a select-all is 600k handles.
    sel: std::sync::Arc<Vec<Py<PyAny>>>,
    facts: Facts,
}

#[pymethods]
impl SelectionVerb {
    fn __call__(&self, py: Python<'_>) -> PyResult<bool> {
        let view = self.view.as_ref().map(|v| v.bind(py).clone());
        let sel: Vec<_> = self.sel.iter().map(|e| e.bind(py).clone()).collect();
        run(self.win.bind(py), view.as_ref(), &self.kind, &self.vid, &sel, None, None, &self.facts)
    }
}

/// `(id, label, shortcut, run, enabled, in More)` for the inspector's
/// verbs: the very entries the right-click menu offers this selection, run
/// the same way.
#[pyfunction]
fn inspector_verbs<'py>(
    win: &Bound<'py, PyAny>,
    sel: &Bound<'py, PyAny>,
) -> PyResult<Vec<(String, String, String, Py<SelectionVerb>, bool, bool)>> {
    let py = win.py();
    let view = opt(win, "_doc").and_then(|d| opt(&d, "stand_in"));
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let (sel, f) = match Kinds::bulk(sel)? {
        Some((all, kinds)) => {
            let f = read_facts_of(&scene, &all, kinds, Some(win), view.as_ref(), None, None)?;
            // The verbs act on the selection in its own order.
            (items(sel)?, f)
        }
        None => {
            let sel = items(sel)?;
            let f = read_facts(&scene, &sel, Some(win), view.as_ref(), None, None)?;
            (sel, f)
        }
    };
    let kind = verbs::kind(&f);
    let shared = std::sync::Arc::new(sel.into_iter().map(Bound::unbind).collect::<Vec<_>>());
    verbs::inspector(&f)
        .into_iter()
        .map(|(vid, label, key, enabled, more)| {
            let run = Py::new(py, SelectionVerb {
                win: win.clone().unbind(),
                view: view.as_ref().map(|v| v.clone().unbind()),
                kind: kind.to_owned(),
                vid: vid.clone(),
                sel: shared.clone(),
                facts: f.clone(),
            })?;
            Ok((vid, label, key, run, enabled, more))
        })
        .collect()
}

// ---- on the view ------------------------------------------------------------------------

/// Hand the menu to the chrome.
fn send(view: &Bound<'_, PyAny>, menu: &ViewportMenu) -> PyResult<()> {
    let Some(renderer) = opt(view, "renderer").filter(|r| r.hasattr("ui_set_context_menu").unwrap_or(false)) else {
        return Ok(());
    };
    let rows: Vec<(String, Vec<(String, String, String, bool, i8)>)> = menu
        .rows
        .iter()
        .map(|r| (r.label.clone(), r.verbs.iter().map(|v| (v.id.clone(), v.label.clone(), v.key.clone(), v.enabled, v.checked)).collect()))
        .collect();
    renderer.call_method1("ui_set_context_menu", (menu.at.0, menu.at.1, &menu.title, &menu.subtitle, rows))?;
    view.call_method0("update")?;
    Ok(())
}

/// Keep `menu` as the view's open menu and show it at `at`.
fn present<'py>(view: &Bound<'py, PyAny>, menu: ViewportMenu) -> PyResult<Bound<'py, ViewportMenu>> {
    let menu = Bound::new(view.py(), menu)?;
    view.setattr("_ctx_menu", &menu)?;
    send(view, &menu.borrow())?;
    Ok(menu)
}

#[pyfunction]
fn context_menu_send<'py>(view: &Bound<'py, PyAny>, menu: Bound<'py, ViewportMenu>) -> PyResult<()> {
    view.setattr("_ctx_menu", &menu)?;
    send(view, &menu.borrow())
}

/// The right-click at `(x, y)`: select what is under the pointer if it is
/// not selected yet, build the menu for the selection, and hand it to the
/// chrome. Returns the menu.
#[pyfunction]
fn context_menu_open_at<'py>(view: &Bound<'py, PyAny>, x: f64, y: f64) -> PyResult<Option<Bound<'py, ViewportMenu>>> {
    let Some(source) = opt(view, "source_viewport") else { return Ok(None) };
    let win = source.call_method0("window")?;
    source.setattr("_context_pixel", (x, y))?;
    let tools = view.getattr("tools")?;
    let picked = tools.call_method1("_context_pick", (x, y))?;
    // A locked image or group under the pointer, which no click selects:
    // its Unlock lives here (as in the classic menu).
    let under = tools.call_method1("pick_locked_image_plane", (x, y))?;
    let locked_image = (!under.is(&picked) && !under.is_none()).then_some(under);
    let locked_group = Some(tools.call_method1("pick_locked_group", (x, y))?).filter(|g| !g.is_none());
    let sel = if picked.is_none() {
        PyList::empty(view.py())
    } else {
        PyList::new(view.py(), items(&source.getattr("scene")?.getattr("selection")?)?)?
    };
    let mut menu = build(&win, Some(view), Some(sel.as_any()), locked_image.as_ref(), locked_group.as_ref())?;
    menu.at = (x, y);
    present(view, menu).map(Some)
}

/// The window's right-click menu at the last context pixel. Headless, the
/// menu is only built, for the scripted runs and tests that read it.
#[pyfunction]
#[pyo3(signature = (win, locked_image=None, locked_group=None))]
fn context_menu_show<'py>(
    win: &Bound<'py, PyAny>,
    locked_image: Option<Bound<'py, PyAny>>,
    locked_group: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, ViewportMenu>> {
    let view = opt(win, "view");
    let mut menu = build(win, view.as_ref(), None, locked_image.as_ref(), locked_group.as_ref())?;
    if let Some(px) = opt(&win.getattr("viewport")?, "_context_pixel") {
        menu.at = (px.get_item(0)?.extract()?, px.get_item(1)?.extract()?);
    }
    match view {
        Some(view) => present(&view, menu),
        None => Bound::new(win.py(), menu),
    }
}

/// `ctx:<id>` from the chrome: run that verb of the open menu.
#[pyfunction]
fn context_menu_action(win: &Bound<'_, PyAny>, view: Option<Bound<'_, PyAny>>, vid: &str) -> PyResult<bool> {
    let py = win.py();
    let Some(view) = view.filter(|v| !v.is_none()) else { return Ok(false) };
    let menu = opt(&view, "_ctx_menu");
    view.setattr("_ctx_menu", py.None())?;
    let Some(menu) = menu.and_then(|m| m.cast_into::<ViewportMenu>().ok()) else { return Ok(false) };
    let menu = menu.borrow();
    if !menu.find(vid).is_some_and(|v| v.enabled) {
        return Ok(false);
    }
    let tools = view.getattr("tools")?;
    if let Some(tool) = &menu.tool {
        let tool = tool.bind(py);
        return Ok(tool.is(&tools.getattr("tool")?) && tool.call_method1("menu_action", (&tools, vid))?.is_truthy()?);
    }
    let sel: Vec<_> = menu.sel.iter().map(|e| e.bind(py).clone()).collect();
    let locked_group = menu.locked_group.as_ref().map(|g| g.bind(py).clone());
    let locked_image = menu.locked_image.as_ref().map(|g| g.bind(py).clone());
    if !run(win, Some(&view), &menu.kind, vid, &sel, locked_group.as_ref(), locked_image.as_ref(), &menu.facts)? {
        return Ok(false);
    }
    let _ = tools.call_method0("sync");
    win.getattr("viewport")?.call_method0("update")?;
    Ok(true)
}

/// Whether this view's chrome draws the right-click menu (the native shell,
/// or the Qt app on Metal).
#[pyfunction]
fn context_menu_available(view: &Bound<'_, PyAny>) -> bool {
    flag(view, "_ui_attached")
        && opt(view, "renderer").is_some_and(|r| r.hasattr("ui_set_context_menu").unwrap_or(false) && r.hasattr("ui_over_chrome").unwrap_or(false))
}

// ---- scripted runs only (`built --script`, `menu ctx_demo:<kind>`) ------------------------

/// The selection a demo of `kind` shows (tests/fixtures/shell/make_kinds.py
/// makes one of each).
fn demo_pick<'py>(scene: &Bound<'py, PyAny>, kind: &str) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let py = scene.py();
    let links = py.import("built_core.links")?;
    let groups = items(&scene.getattr("groups")?)?;
    let named = |name: &str| groups.iter().find(|g| text(g, "name") == name).cloned();
    let first = |obj: Bound<'py, PyAny>| -> PyResult<Vec<Bound<'py, PyAny>>> { Ok(items(&obj)?.into_iter().take(1).collect()) };
    let mesh = scene.getattr("mesh")?;
    let link = groups.iter().find(|g| links.call_method1("is_linked", (*g,)).and_then(|v| v.is_truthy()).unwrap_or(false)).cloned();
    let picks: Vec<Option<Bound<'py, PyAny>>> = match kind {
        "faces" => first(mesh.getattr("faces")?)?.into_iter().map(Some).collect(),
        "edges" => first(mesh.getattr("edges")?)?.into_iter().map(Some).collect(),
        "geometry" => first(mesh.getattr("faces")?)?.into_iter().chain(first(mesh.getattr("edges")?)?).map(Some).collect(),
        "group" => vec![named("Box A")],
        "component" => vec![named("Door")],
        "groups" => vec![named("Box A"), named("Box B")],
        "mixed" => std::iter::once(named("Box A")).chain(first(mesh.getattr("faces")?)?.into_iter().map(Some)).collect(),
        "link" => vec![link],
        "linked_part" => match link {
            Some(l) => vec![opt(&l, "children").and_then(|c| c.get_item(0).ok())],
            None => Vec::new(),
        },
        "annotation" => first(scene.getattr("text_labels")?)?.into_iter().map(Some).collect(),
        "dimension" => first(scene.getattr("dimensions")?)?.into_iter().map(Some).collect(),
        "guide" => first(scene.getattr("guides")?)?.into_iter().map(Some).collect(),
        _ => Vec::new(),
    };
    Ok(picks.into_iter().flatten().collect())
}

/// The right-click menu of `kind` on screen, opened where that thing is
/// drawn, for captures and visual checks. Never part of a user's path.
#[pyfunction]
fn context_menu_demo(win: &Bound<'_, PyAny>, kind: &str) -> PyResult<bool> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let sel = demo_pick(&scene, kind)?;
    scene.call_method1("select", (PyList::new(py, &sel)?,))?;
    let view = win.getattr("view")?;
    let mut point = None;
    if let Some(first) = sel.first() {
        let bounds = scene.call_method0("selection_bounds")?;
        let (lo, hi) = (bounds.get_item(0)?, bounds.get_item(1)?);
        point = if !lo.is_none() {
            Some(lo.add(hi)?.mul(0.5)?)
        } else if first.hasattr("position")? {
            Some(first.call_method0("position")?)
        } else if first.hasattr("point")? {
            Some(first.getattr("point")?)
        } else {
            None
        };
    }
    let px = match point {
        Some(p) => Some(view.getattr("tools")?.call_method1("_world_to_pixel", (p,))?).filter(|p| !p.is_none()),
        None => None,
    };
    let (x, y) = match px {
        Some(px) => (px.get_item(0)?.extract::<f64>()?, px.get_item(1)?.extract::<f64>()?),
        None => (view.call_method0("width")?.extract::<f64>()? * 0.45, view.call_method0("height")?.extract::<f64>()? * 0.4),
    };
    vp.setattr("_context_pixel", (x, y))?;
    let mut menu = build(win, Some(&view), None, None, None)?;
    menu.at = (x, y);
    present(&view, menu)?;
    Ok(true)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<MenuVerb>()?;
    m.add_class::<MenuRow>()?;
    m.add_class::<ViewportMenu>()?;
    m.add_class::<SelectionVerb>()?;
    for f in [
        wrap_pyfunction!(selection_facts, m)?,
        wrap_pyfunction!(selection_kind, m)?,
        wrap_pyfunction!(file_verb_ids, m)?,
        wrap_pyfunction!(context_menu_build, m)?,
        wrap_pyfunction!(context_menu_run, m)?,
        wrap_pyfunction!(inspector_verbs, m)?,
        wrap_pyfunction!(context_menu_send, m)?,
        wrap_pyfunction!(context_menu_open_at, m)?,
        wrap_pyfunction!(context_menu_show, m)?,
        wrap_pyfunction!(context_menu_action, m)?,
        wrap_pyfunction!(context_menu_available, m)?,
        wrap_pyfunction!(context_menu_demo, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
