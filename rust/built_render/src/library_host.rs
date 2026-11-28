// SPDX-License-Identifier: GPL-3.0-or-later
//! The Library on the window: the submodule `built_render.wgpu_library`.
//!
//! One place for every kind of reusable resource. Each type is a provider
//! (Materials, Components, Profiles to come) listing item dicts
//! (`key`, `name`, `source`, `category`, `base`, `uses`, `hex`, `thumb`,
//! `verbs`) and running a verb on one. `library_view.rs` decides what the
//! sheet lists, how it sorts, filters and groups, each tile's words and a
//! material's details; this module keeps the browsing state (on the view,
//! saved in the preferences), reads the model's materials and components
//! and who uses them, runs the verbs through the protocol, and feeds the
//! Paint strip and the material inspector. Model edits go through
//! `built_core.protocol.commands`.
mod bulk;
pub(crate) mod destinations;
mod packs;
mod presets;
use presets::PresetsProvider;
pub(crate) use presets::capture_preset;
use std::path::PathBuf;
use crate::library_collections as collections;
use crate::settings_rows::{func, none, obj, opt, s, Res};
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList, PySet, PyTuple},
};

pub const SEP: &str = "\x1f";
const RECENT_MAX: usize = 40;
const THUMB_PX: u32 = 96;
const MINE_KEY: &str = "library/mine";
const STRIP_MODEL: usize = 10;
const STRIP_RECENT: usize = 5;
/// PBR rows of the editor: (channel, label, map channel).
const PBR_ROWS: [(&str, &str, &str); 5] = [
    ("metallic", "Metallic", "metallic_map"),
    ("roughness", "Roughness", "roughness_map"),
    ("normal_scale", "Normal", "normal_map"),
    ("ao_strength", "Occlusion", "ao_map"),
    ("emission_strength", "Emission", "emission_map"),
];

/// This submodule, through its attributes (tests replace `_pick_image`,
/// `_bundled_by_key`, `open_library`).
fn lib(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("built_render")?.getattr("wgpu_library")
}

fn ui(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("views.wgpu_ui")
}

fn render(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_render")
}

fn json(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("json")
}

fn dumps(py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<String> {
    let kw = [("default", py.import("builtins")?.getattr("str")?)].into_py_dict(py)?;
    json(py)?.getattr("dumps")?.call((v,), Some(&kw))?.extract()
}

fn loads<'py>(py: Python<'py>, text: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    json(py)?.call_method1("loads", (text,))
}

fn get<'py>(d: &Bound<'py, PyAny>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    d.call_method1("get", (key,))
}

fn key_kind(key: &str) -> &str {
    key.split_once(':').map_or(key, |(k, _)| k)
}

fn key_rest(key: &str) -> &str {
    key.split_once(':').map_or("", |(_, r)| r)
}

fn paint_tool(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("tools.paint")?.getattr("PaintTool")
}

/// `#rrggbb` of an rgb in 0..1, "" for none.
#[pyfunction]
fn _hex(rgb: &Bound<'_, PyAny>) -> PyResult<String> {
    if rgb.is_none() || !rgb.is_truthy()? {
        return Ok(String::new());
    }
    let mut out = String::from("#");
    for (i, c) in rgb.try_iter()?.enumerate() {
        if i == 3 {
            break;
        }
        let v: f64 = rgb.py().import("builtins")?.getattr("float")?.call1((c?,))?.extract()?;
        out.push_str(&format!("{:02x}", (v.clamp(0.0, 1.0) * 255.0) as i64));
    }
    Ok(out)
}

fn parse_hex(py: Python<'_>, text: &str) -> PyResult<(f64, f64, f64)> {
    let t = text.trim().trim_start_matches('#');
    if t.chars().count() != 6 {
        return Err(pyo3::exceptions::PyValueError::new_err("A colour is six hex digits, like #a0703c"));
    }
    let v: i64 = py.import("builtins")?.getattr("int")?.call1((t, 16))?.extract()?;
    Ok(((v >> 16 & 255) as f64 / 255.0, (v >> 8 & 255) as f64 / 255.0, (v & 255) as f64 / 255.0))
}

fn store(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("built_core.settings")?.call_method0("settings")
}

/// A JSON value kept in the preferences, else `default`.
fn settings_json<'py>(py: Python<'py>, key: &str, default: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let raw = store(py)?.call_method1("value", (key,))?;
    if raw.is_instance_of::<pyo3::types::PyString>() && raw.is_truthy()? {
        if let Ok(v) = loads(py, &raw) {
            return Ok(v);
        }
    }
    Ok(default)
}

fn store_json(py: Python<'_>, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
    if let Ok(text) = json(py)?.call_method1("dumps", (value,)) {
        store(py)?.call_method1("setValue", (key, text))?;
    }
    Ok(())
}

fn mine(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    settings_json(py, MINE_KEY, PyList::empty(py).into_any())
}

/// What a provider reads: the window, the view and the open scene.
#[pyclass(module = "built_render.wgpu_library", dict, get_all, set_all)]
pub struct Ctx {
    win: Py<PyAny>,
    view: Py<PyAny>,
    vp: Py<PyAny>,
    scene: Py<PyAny>,
}

#[pymethods]
impl Ctx {
    #[new]
    fn new(win: &Bound<'_, PyAny>, view: Py<PyAny>) -> PyResult<Self> {
        let vp = win.getattr("viewport")?;
        Ok(Ctx { win: win.clone().unbind(), view, scene: vp.getattr("scene")?.unbind(), vp: vp.unbind() })
    }

    #[pyo3(signature = (command, **args))]
    fn core<'py>(&self, py: Python<'py>, command: &str, args: Option<&Bound<'py, PyDict>>) -> PyResult<Bound<'py, PyAny>> {
        crate::document_registration::view::calls::core_command(self.vp.bind(py), command, args)
    }

    fn flash(&self, py: Python<'_>, text: &Bound<'_, PyAny>) -> PyResult<()> {
        self.vp.bind(py).call_method1("flash_status", (text,))?;
        Ok(())
    }
}

fn ctx_of<'py>(py: Python<'py>, win: &Bound<'py, PyAny>, view: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    Ok(Bound::new(py, Ctx::new(win, view.clone().unbind())?)?.into_any())
}

fn core<'py>(ctx: &Bound<'py, PyAny>, command: &str, kw: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyAny>> {
    ctx.getattr("core")?.call((command,), Some(kw))
}

fn flash(ctx: &Bound<'_, PyAny>, text: &str) -> PyResult<()> {
    ctx.call_method1("flash", (text,))?;
    Ok(())
}

fn notify(vp: &Bound<'_, PyAny>) -> PyResult<()> {
    match opt(vp, "notify_scene_changed") {
        Some(f) => f.call0()?,
        None => vp.call_method0("update")?,
    };
    Ok(())
}

// ---- usage ----------------------------------------------------------------------------

fn cached<'py>(scene: &Bound<'py, PyAny>, attr: &str) -> PyResult<(Bound<'py, PyTuple>, Option<Bound<'py, PyAny>>)> {
    let py = scene.py();
    let sig = PyTuple::new(py, [obj(py, scene.as_ptr() as usize)?.into_bound(py), scene.getattr("content_version")?])?;
    let hit = scene.getattr(attr).ok().filter(|c| !c.is_none()).filter(|c| c.get_item(0).and_then(|s| s.eq(&sig)).unwrap_or(false));
    Ok((sig, hit.map(|c| c.get_item(1)).transpose()?))
}

fn mesh_names<'py>(py: Python<'py>, mesh: &Bound<'py, PyAny>, with_back: bool) -> PyResult<Bound<'py, PyAny>> {
    let names = py.import("built_core")?.call_method1("store_material_names", (mesh,))?;
    if !names.is_none() {
        return Ok(names);
    }
    let set = PySet::empty(py)?;
    for f in mesh.getattr("faces")?.try_iter()? {
        let attrs = f?.getattr("attrs")?;
        set.add(get(&attrs, "mat")?)?;
        if with_back {
            let back = get(&attrs, "back")?;
            if back.is_instance_of::<PyDict>() {
                set.add(get(&back, "mat")?)?;
            }
        }
    }
    Ok(set.into_any())
}

/// Objects wearing each registry material, counted by the Rust survey
/// (`built_core.material_survey`): paint by mesh and placement, kept
/// across edits that leave a mesh alone.
#[pyfunction]
#[pyo3(signature = (scene, history=None))]
fn material_uses<'py>(scene: &Bound<'py, PyAny>, history: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    survey(scene, history)?.get_item(0)
}

/// `(uses, unnamed colours {key: (colour, opacity, n)}, worn rows)`.
fn survey<'py>(scene: &Bound<'py, PyAny>, history: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    let history = history.unwrap_or_else(|| py.None().into_bound(py));
    py.import("built_core")?.call_method1("material_survey", (scene, history))
}

fn material_revision<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    scene.py().import("built_core")?.call_method1("library_material_revision", (scene,))
}

/// Return the completed revision's survey, or None while its worker runs.
fn sheet_usage<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    let version = material_revision(scene)?;
    if scene.getattr_opt("_library_usage_failure")?.is_some_and(|v| v.eq(&version).unwrap_or(false)) {
        return Ok(py.None().into_bound(py));
    }
    let job = scene.getattr_opt("_library_usage_job")?;
    if let Some(job) = job {
        let future = job.get_item(1)?;
        if job.get_item(0)?.eq(&version)? {
            return if future.call_method0("done")?.is_truthy()? {
                match future.call_method0("result") {
                    Ok(result) => Ok(result),
                    Err(error) => {
                        error.print(py);
                        scene.setattr("_library_usage_failure", version)?;
                        Ok(py.None().into_bound(py))
                    }
                }
            } else { Ok(py.None().into_bound(py)) };
        }
        future.call_method0("cancel")?;
    }
    let module = lib(py)?;
    let executor = match module.getattr_opt("_usage_executor")? {
        Some(pool) => pool,
        None => {
            let pool = py.import("concurrent.futures")?.getattr("ThreadPoolExecutor")?.call1((1,))?;
            module.setattr("_usage_executor", &pool)?;
            pool
        }
    };
    let future = executor.call_method1("submit", (py.import("built_core")?.getattr("library_usage")?, scene, version.get_item(0)?))?;
    scene.setattr("_library_usage_job", (version, future))?;
    Ok(py.None().into_bound(py))
}

fn provider_survey<'py>(ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = ctx.py();
    match ctx.getattr_opt("usage")? {
        Some(v) if v.is_none() => Ok((PyDict::new(py), PyDict::new(py), PyList::empty(py)).into_pyobject(py)?.into_any()),
        Some(v) => Ok(v),
        None => survey(&ctx.getattr("scene")?, ctx_history(ctx)),
    }
}

/// The open document's history, when the context has a viewport.
fn ctx_history<'py>(ctx: &Bound<'py, PyAny>) -> Option<Bound<'py, PyAny>> {
    opt(ctx, "vp").and_then(|vp| opt(&vp, "history"))
}

/// `{linked file: Counter(material → objects wearing it)}`, per content
/// version.
#[pyfunction]
fn linked_material_uses<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    let (sig, hit) = cached(scene, "_lib_linked")?;
    if let Some(hit) = hit {
        return Ok(hit);
    }
    let read = || -> PyResult<Bound<'py, PyDict>> {
        let links = py.import("built_core.links")?;
        let purge = py.import("built_core")?;
        let out = PyDict::new(py);
        for path in links.call_method1("sources", (scene,))?.try_iter()? {
            let path = path?;
            let counts = py.import("collections")?.getattr("Counter")?.call0()?;
            for g in links.call_method1("linked_groups", (scene, &path))?.try_iter()? {
                for pg in purge.call_method1("purge_iter_groups", (PyTuple::new(py, [g?])?,))?.try_iter()? {
                    let Some(mesh) = opt(&pg?, "mesh") else { continue };
                    for n in mesh_names(py, &mesh, false)?.try_iter()? {
                        let n = n?;
                        if n.is_truthy()? {
                            let c: i64 = counts.call_method1("get", (&n, 0))?.extract()?;
                            counts.set_item(&n, c + 1)?;
                        }
                    }
                }
            }
            if counts.is_truthy()? {
                out.set_item(path, counts)?;
            }
        }
        Ok(out)
    };
    // No link data: nothing read-only to list.
    let out = read().unwrap_or_else(|_| PyDict::new(py));
    scene.setattr("_lib_linked", (sig, &out))?;
    Ok(out.into_any())
}

/// Select what wears `name` in the open context: its loose faces and each
/// placement with a face or a part painted with it.
#[pyfunction]
fn select_material_uses(ctx: &Bound<'_, PyAny>, name: &Bound<'_, PyAny>) -> PyResult<usize> {
    let py = ctx.py();
    let scene = ctx.getattr("scene")?;
    let purge = py.import("built_core")?;
    let mut picked = Vec::new();
    for f in scene.getattr("mesh")?.getattr("faces")?.try_iter()? {
        let f = f?;
        let attrs = f.getattr("attrs")?;
        let back = get(&attrs, "back")?;
        let back_mat = if back.is_truthy()? { get(&back, "mat")? } else { py.None().into_bound(py) };
        if get(&attrs, "mat")?.eq(name)? || back_mat.eq(name)? {
            picked.push(f);
        }
    }
    let memo = PyDict::new(py);
    for g in scene.getattr("groups")?.try_iter()? {
        let g = g?;
        for pg in purge.call_method1("purge_iter_groups", (PyTuple::new(py, [&g])?,))?.try_iter()? {
            let pg = pg?;
            let paint = pg.getattr("material").ok().filter(|p| p.is_truthy().unwrap_or(false));
            let painted = match paint {
                Some(p) => get(&p, "mat")?.eq(name)?,
                None => false,
            };
            let mesh = pg.getattr("mesh")?;
            let key = mesh.as_ptr() as usize;
            if !memo.contains(key)? {
                memo.set_item(key, mesh_names(py, &mesh, false)?)?;
            }
            if painted || memo.get_item(key)?.expect("set").contains(name)? {
                picked.push(g.clone());
                break;
            }
        }
    }
    let mut kept = Vec::new();
    for e in picked {
        if !e.hasattr("uid")? || scene.call_method1("entity_selectable", (&e,))?.is_truthy()? {
            kept.push(e);
        }
    }
    let n = kept.len();
    scene.call_method1("select", (kept,))?;
    notify(&ctx.getattr("vp")?)?;
    Ok(n)
}

#[pyfunction]
fn unnamed_colours_cached<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    let (sig, hit) = cached(scene, "_lib_unnamed")?;
    if let Some(hit) = hit {
        return Ok(hit);
    }
    let found = py.import("built_core.material_commands")?.call_method1("unnamed_colours", (scene,))?;
    scene.setattr("_lib_unnamed", (sig, &found))?;
    Ok(found)
}

// ---- providers ------------------------------------------------------------------------

fn item<'py>(py: Python<'py>, fields: &[(&str, Bound<'py, PyAny>)]) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in fields {
        d.set_item(*k, v)?;
    }
    Ok(d)
}

fn verbs<'py>(py: Python<'py>, v: &[&str]) -> PyResult<Bound<'py, PyAny>> {
    Ok(PyTuple::new(py, v)?.into_any())
}

fn library_type<'py>(py: Python<'py>, id: &str) -> PyResult<Bound<'py, PyAny>> {
    render(py)?.call_method1("library_type", (id,))
}

/// One resource type. `ready` False shows the tab as coming.
#[pyclass(module = "built_render.wgpu_library", name = "Provider", subclass, dict)]
pub struct LibraryProvider;

#[pymethods]
impl LibraryProvider {
    #[new]
    fn new() -> Self {
        LibraryProvider
    }

    #[classattr]
    fn id() -> &'static str {
        ""
    }

    #[classattr]
    fn label() -> &'static str {
        ""
    }

    #[classattr]
    fn ready() -> bool {
        true
    }

    #[getter]
    fn empty<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        library_type(slf.py(), &slf.getattr("id")?.extract::<String>()?)?.get_item(3)
    }

    #[getter]
    fn facets<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        library_type(slf.py(), &slf.getattr("id")?.extract::<String>()?)?.get_item(1)
    }

    fn items<'py>(&self, py: Python<'py>, _ctx: &Bound<'py, PyAny>) -> Bound<'py, PyList> {
        PyList::empty(py)
    }

    fn verbs<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        render(slf.py())?.call_method1("library_verbs", (slf.getattr("id")?,))
    }

    fn type_actions<'py>(&self, py: Python<'py>, _ctx: &Bound<'py, PyAny>) -> Bound<'py, PyList> {
        PyList::empty(py)
    }

    #[pyo3(signature = (ctx, verb, item, arg=""))]
    fn run(&self, ctx: &Bound<'_, PyAny>, verb: &str, item: &Bound<'_, PyAny>, arg: &str) {
        let _ = (ctx, verb, item, arg);
    }

    fn type_action(&self, _ctx: &Bound<'_, PyAny>, _action: &str) {}

    fn inspector(&self, _ctx: &Bound<'_, PyAny>, _item: &Bound<'_, PyAny>) -> Option<Py<PyAny>> {
        None
    }

    fn edit(&self, _ctx: &Bound<'_, PyAny>, _item: &Bound<'_, PyAny>, _field: &str, _text: &Bound<'_, PyAny>) {}
}

/// Every paint is a Material: a colour or an image as its base, the same
/// PBR values and opacity on top. The model's registry, its unnamed colours
/// (older files), My library, then the bundled images by category and the
/// bundled colours (RAL) as one more category.
#[pyclass(module = "built_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct MaterialsProvider;

fn rename_paint(py: Python<'_>, scene: &Bound<'_, PyAny>, old: &str, new: &str) -> PyResult<()> {
    let tool = paint_tool(py)?;
    if let Some(mat) = opt(&tool, "current_material") {
        if mat.getattr("name")?.eq(old)? {
            let kw = [("name", new)].into_py_dict(py)?;
            py.import("built_core")?.getattr("pick_library_paint")?.call((scene, tool), Some(&kw))?;
        }
    }
    Ok(())
}

fn ask(view: &Bound<'_, PyAny>, pending: (&str, &str, String), title: &str, body: &str, yes: &str) -> PyResult<()> {
    let py = view.py();
    let win = if view.hasattr("window")? { view.call_method0("window")? } else { py.None().into_bound(py) };
    let (w, v, (what, type_id, key)) = (win.clone().unbind(), view.clone().unbind(), (pending.0.to_owned(), pending.1.to_owned(), pending.2));
    let done = func(py, move |py, a| {
        if a.get_item(0)?.eq("yes")? {
            let pending = PyTuple::new(py, [s(py, &what), s(py, &type_id), s(py, &key)])?;
            lib(py)?.call_method1("confirmed", (w.bind(py), v.bind(py), pending))?;
            lib(py)?.call_method1("refresh", (w.bind(py),))?;
        }
        Ok(none(py))
    })?;
    let kw = [("default", "yes"), ("cancel", "cancel"), ("tone", "warning")].into_py_dict(py)?;
    py.import("built_render.chrome_dialogs")?.getattr("ask_later")?.call((win, title, body, vec![("cancel", "Cancel"), ("yes", yes)], done), Some(&kw))?;
    Ok(())
}

fn state<'py>(view: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    lib(view.py())?.call_method1("_state", (view,))
}

fn bump_mine(view: &Bound<'_, PyAny>) -> PyResult<()> {
    let st = state(view)?;
    let rev: i64 = st.get_item("mine_rev")?.extract()?;
    st.set_item("mine_rev", rev + 1)
}

#[pymethods]
impl MaterialsProvider {
    #[new]
    fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(MaterialsProvider)
    }

    #[classattr]
    fn id() -> &'static str {
        "materials"
    }

    #[classattr]
    fn label() -> &'static str {
        "Materials"
    }

    fn _model_items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let scene = ctx.getattr("scene")?;
        let got = provider_survey(ctx)?;
        let (uses, colours, worn) = (got.get_item(0)?, got.get_item(1)?, got.get_item(2)?);
        // Linked-only swatches become editable only after their source is known.
        let pending = ctx.getattr_opt("usage")?.is_some_and(|u| u.is_none());
        let actions = if pending { verbs(py, &["apply"])? } else {
            verbs(py, &["apply", "edit", "select", "-", "duplicate", "rename", "keep", "-", "delete"])?
        };
        let out = PyList::empty(py);
        if let Some(mats) = opt(&scene, "materials").filter(|m| m.is_truthy().unwrap_or(false)) {
            for pair in mats.call_method0("items")?.try_iter()? {
                let pair = pair?;
                let (name, mat) = (pair.get_item(0)?, pair.get_item(1)?);
                let tex = opt(&mat, "texture").filter(|t| t.is_truthy().unwrap_or(false)).map(|t| get(&t, "path")).transpose()?.unwrap_or_else(|| py.None().into_bound(py));
                let raw: String = name.extract()?;
                let original = tex.extract::<String>().ok().map(|p| crate::library_view::file_stem(&p));
                let label = if original.as_deref().and_then(|p| std::path::Path::new(p).file_stem())
                    .and_then(|stem| stem.to_str()) == Some(raw.as_str()) {
                    let words = raw.replace(['_', '-'], " ");
                    let mut chars = words.chars();
                    chars.next().map(|c| c.to_uppercase().to_string() + chars.as_str()).unwrap_or_default()
                } else { raw };
                out.append(item(py, &[
                    ("label", s(py, &label)),
                    ("key", s(py, &format!("m:{}", name.str()?))),
                    ("name", name.clone()),
                    ("source", s(py, "model")),
                    ("category", s(py, "")),
                    ("base", s(py, if tex.is_truthy()? { "image" } else { "colour" })),
                    ("uses", uses.call_method1("get", (&name, 0))?),
                    ("hex", s(py, &_hex(&mat.getattr("color")?)?)),
                    ("thumb", tex),
                    ("verbs", actions.clone()),
                ])?)?;
            }
        }
        for row in worn.try_iter()? {
            out.append(row?)?;
        }
        for pair in colours.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let (key, entry) = (pair.get_item(0)?, pair.get_item(1)?);
            let key: String = key.extract()?;
            out.append(item(py, &[
                ("key", s(py, &format!("uc:{key}"))),
                ("name", s(py, key.split('@').next().unwrap_or(""))),
                ("source", s(py, "model")),
                ("category", s(py, "Unnamed colours")),
                ("base", s(py, "colour")),
                ("uses", entry.get_item(2)?),
                ("hex", s(py, &_hex(&entry.get_item(0)?)?)),
                ("thumb", py.None().into_bound(py)),
                ("verbs", verbs(py, &["apply", "select", "-", "name"])?),
            ])?)?;
        }
        Ok(out)
    }

    fn _mine_items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        let uses = provider_survey(ctx)?.get_item(0)?;
        for (i, e) in mine(py)?.try_iter()?.enumerate() {
            let e = e?;
            if !e.is_instance_of::<PyDict>() {
                continue;
            }
            let t = get(&e, "type")?;
            if !(t.eq("materials")? || t.eq("colours")?) {
                continue;
            }
            let texture = get(&e, "texture")?;
            let tex = if texture.is_truthy()? { get(&texture, "path")? } else { py.None().into_bound(py) };
            out.append(item(py, &[
                ("key", s(py, &format!("my:{i}"))),
                ("name", e.call_method1("get", ("name", ""))?),
                ("source", s(py, "mine")),
                ("category", e.call_method1("get", ("category", ""))?),
                ("base", s(py, if tex.is_truthy()? { "image" } else { "colour" })),
                ("uses", uses.call_method1("get", (get(&e, "name")?, 0))?),
                ("hex", s(py, &_hex(&get(&e, "color")?)?)),
                ("thumb", tex),
                ("verbs", verbs(py, &["apply", "-", "unkeep"])?),
            ])?)?;
        }
        Ok(out)
    }

    fn _bundled<'py>(&self, py: Python<'py>, _ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, catalogue(py)?.call_method0("values")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)
    }

    /// The linked files' materials, read-only, under each file: registry
    /// materials only linked parts wear move out of «In this model».
    fn _linked_items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>, model: &Bound<'py, PyAny>) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
        let scene = ctx.getattr("scene")?;
        let per_file = match ctx.getattr_opt("usage")? {
            Some(v) if v.is_none() => return Ok((model.clone(), PyList::empty(py).into_any())),
            Some(v) => v.get_item(3)?,
            None => linked_material_uses(&scene)?,
        };
        if !per_file.is_truthy()? {
            return Ok((model.clone(), PyList::empty(py).into_any()));
        }
        let files = PyList::empty(py);
        for pair in per_file.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let counts = py.import("builtins")?.getattr("list")?.call1((pair.get_item(1)?.call_method0("items")?,))?;
            files.append((pair.get_item(0)?, counts))?;
        }
        let uses = py.import("builtins")?.getattr("dict")?.call1((provider_survey(ctx)?.get_item(0)?,))?;
        let got = render(py)?.call_method1("library_linked", (dumps(py, model)?, dumps(py, files.as_any())?, dumps(py, &uses)?))?;
        Ok((loads(py, &got.get_item(0)?)?, loads(py, &got.get_item(1)?)?))
    }

    fn items<'py>(slf: &Bound<'py, Self>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let py = slf.py();
        let bundled = slf.getattr("_bundled_cache").ok().filter(|b| !b.is_none());
        let bundled = match bundled {
            Some(b) => b,
            None => {
                let b = slf.call_method1("_bundled", (ctx,))?;
                slf.setattr("_bundled_cache", &b)?;
                b
            }
        };
        let model = slf.call_method1("_model_items", (ctx,))?;
        let pair = slf.call_method1("_linked_items", (ctx, model))?;
        let out = PyList::empty(py);
        let scene = ctx.getattr_opt("scene")?;
        let saved = destinations::saved_ids(py, scene.as_ref(), "materials")?;
        for part in [pair.get_item(0)?, pair.get_item(1)?, slf.call_method1("_mine_items", (ctx,))?] {
            for e in part.try_iter()? {
                let e = e?;
                // A kept material copied into My library is listed once, from there.
                if get(&e, "source")?.eq("mine")? && saved.contains(&crate::library_store::legacy_material_id(&get(&e, "name")?.str()?.to_string())) {
                    continue;
                }
                out.append(e)?;
            }
        }
        for e in destinations::py_items(py, scene.as_ref(), &["materials"])? {
            out.append(e)?;
        }
        for e in bundled.try_iter()? {
            out.append(py.import("builtins")?.getattr("dict")?.call1((e?,))?)?;
        }
        Ok(out)
    }

    fn type_actions<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let usage = sheet_usage(&ctx.getattr("scene")?)?;
        let acts = PyList::empty(py);
        if usage.is_none() {
            acts.append(("purge", "Purge unused materials…"))?;
        } else {
            let k = usage.get_item(1)?.len()?;
            if k != 0 {
                acts.append(("name", format!("Name these: {k} unnamed colour{}", if k == 1 { "" } else { "s" })))?;
            }
            let n: usize = usage.get_item(4)?.extract()?;
            if n > 0 {
                acts.append(("purge", format!("Purge unused materials ({n})")))?;
            }
        }
        Ok(acts)
    }

    fn type_action(&self, py: Python<'_>, ctx: &Bound<'_, PyAny>, action: &str) -> PyResult<()> {
        if action == "purge" {
            let kw = [("layers", false), ("materials", true)].into_py_dict(py)?;
            let out = core(ctx, "purge_unused", &kw)?;
            let ok = !out.is_none() && out.is_truthy()? && get(&out, "ok")?.is_truthy()?;
            flash(ctx, if ok { "Unused materials purged" } else { "Every material is in use" })?;
        } else if action == "name" {
            let out = core(ctx, "name_unnamed_colours", &PyDict::new(py))?;
            let n: i64 = if out.is_truthy()? { out.call_method1("get", ("named", 0))?.extract()? } else { 0 };
            if n != 0 {
                flash(ctx, &format!("{n} colour{} named: each is a material now", if n != 1 { "s" } else { "" }))?;
            } else {
                flash(ctx, "Every colour already has a name")?;
            }
        }
        Ok(())
    }

    fn _mine_entry<'py>(&self, py: Python<'py>, item: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let key: String = item.get_item("key")?.extract()?;
        let i: i64 = py.import("builtins")?.getattr("int")?.call1((key_rest(&key),))?.extract()?;
        mine(py)?.get_item(i)
    }

    /// The registry name or the recipe Paint takes for `item`.
    fn paint_recipe<'py>(slf: &Bound<'py, Self>, ctx: &Bound<'py, PyAny>, item: &Bound<'py, PyAny>) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
        let py = slf.py();
        let key: String = item.get_item("key")?.extract()?;
        let empty = s(py, "");
        match key_kind(&key) {
            "uc" => Ok((empty, get(&unnamed_colours_cached(&ctx.getattr("scene")?)?, &key[3..])?)),
            "my" => Ok((empty, slf.call_method1("_mine_entry", (item,))?)),
            "m" | "lk" => Ok((item.get_item("name")?, py.None().into_bound(py))),
            _ => {
                let recipe = get(item, "recipe")?;
                if recipe.is_truthy()? {
                    return Ok((empty, recipe));
                }
                let entry = catalogue(py)?.call_method1("get", (&key, PyDict::new(py)))?;
                Ok((empty, get(&entry, "recipe")?))
            }
        }
    }

    #[pyo3(signature = (ctx, verb, item, arg=""))]
    fn run(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, verb: &str, item: &Bound<'_, PyAny>, arg: &str) -> PyResult<()> {
        let py = slf.py();
        let key: String = item.get_item("key")?.extract()?;
        let name = item.get_item("name")?;
        let name_s: String = name.str()?.to_string();
        let kind = key_kind(&key).to_owned();
        let scene = ctx.getattr("scene")?;
        let view = ctx.getattr("view")?;
        match (verb, kind.as_str()) {
            ("apply", "uc" | "my" | "worn") => {
                let recipe = slf.call_method1("paint_recipe", (ctx, item))?.get_item(1)?;
                if recipe.is_truthy()? {
                    lib(py)?.getattr("_pick_paint")?.call((ctx.getattr("win")?,), Some(&[("recipe", recipe)].into_py_dict(py)?))?;
                }
            }
            ("apply", _) => {
                let target = if kind == "lk" { format!("m:{name_s}") } else { key.clone() };
                ui(py)?.call_method1("_paint_with", (ctx.getattr("win")?, target))?;
            }
            ("name", _) => {
                slf.call_method1("type_action", (ctx, "name"))?;
            }
            (_, "uc") => {
                let entry = get(&unnamed_colours_cached(&scene)?, &key[3..])?;
                if entry.is_none() {
                    return Ok(());
                }
                if verb == "select" {
                    let loose: Vec<usize> = scene.getattr("mesh")?.getattr("faces")?.try_iter()?.map(|f| f.map(|f| f.as_ptr() as usize)).collect::<PyResult<_>>()?;
                    let mut picks = Vec::new();
                    for pair in entry.get_item("faces")?.try_iter()? {
                        let pair = pair?;
                        let (f, side) = (pair.get_item(0)?, pair.get_item(1)?);
                        if side.eq("front")? && loose.contains(&(f.as_ptr() as usize)) {
                            picks.push(f);
                        }
                    }
                    let groups = scene.getattr("groups")?;
                    for g in entry.get_item("groups")?.try_iter()? {
                        let g = g?;
                        if groups.contains(&g)? {
                            picks.push(g);
                        }
                    }
                    scene.call_method1("select", (picks,))?;
                    notify(&ctx.getattr("vp")?)?;
                }
            }
            ("edit", "m") => {
                lib(py)?.call_method1("set_mode", (&view, ctx.getattr("win")?, ""))?;
                view.setattr("_ui_mat_focus", &key)?;
                let renderer = view.getattr("renderer")?;
                if renderer.hasattr("ui_show_inspector")? {
                    renderer.call_method0("ui_show_inspector")?;
                }
            }
            ("select", "m" | "lk" | "worn") => {
                let n = select_material_uses(ctx, &name)?;
                if n != 0 {
                    flash(ctx, &format!("{n} selected wearing {name_s}"))?;
                } else {
                    flash(ctx, &format!("Nothing in the open context wears {name_s}"))?;
                }
            }
            ("duplicate", "m") => {
                let out = core(ctx, "duplicate_material", &[("name", &name)].into_py_dict(py)?)?;
                if out.is_truthy()? && get(&out, "name")?.is_truthy()? {
                    let k = format!("m:{}", get(&out, "name")?.str()?);
                    view.setattr("_ui_mat_focus", &k)?;
                    state(&view)?.set_item("cursor", &k)?;
                }
            }
            ("rename", "m") => {
                let new = arg.trim();
                if !new.is_empty() && new != name_s {
                    let kw = PyDict::new(py);
                    kw.set_item("name", &name)?;
                    kw.set_item("new_name", new)?;
                    core(ctx, "rename_material", &kw)?;
                    rename_paint(py, &scene, &name_s, new)?;
                    if view.getattr("__dict__")?.call_method1("get", ("_ui_mat_focus",))?.eq(&key)? {
                        view.setattr("_ui_mat_focus", format!("m:{new}"))?;
                    }
                }
            }
            ("delete", "m") => {
                let n: i64 = item.call_method1("get", ("uses", 0))?.extract()?;
                let body = if n != 0 { format!("{n} object{} wearing it go back to the default material.", if n != 1 { "s" } else { "" }) } else { "Nothing wears it.".into() };
                ask(&view, ("delete", "materials", key.clone()), &format!("Delete “{name_s}”?"), &body, "Delete")?;
            }
            ("keep", "m") => {
                // Save to Library…: the material's dictionary, its image among its files.
                let mat = scene.getattr("materials")?.call_method1("get", (&name,))?;
                if !mat.is_none() {
                    let mut data: serde_json::Value = serde_json::from_str(&dumps(py, &mat.call_method0("to_dict")?)?)
                        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
                    let mut files = Vec::new();
                    if let Some(path) = data["texture"]["path"].as_str().map(PathBuf::from).filter(|p| p.is_file()) {
                        let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        data["texture"]["path"] = serde_json::json!(file);
                        files.push((file, path));
                    }
                    let id = crate::library_store::legacy_material_id(&name_s);
                    destinations::save_to_library(&ctx.getattr("win")?, "materials", &name_s, &data.to_string(), Some(id), 1, "", None, None, Some(files),
                                                  Some(vec!["model".into()]), None)?;
                    bump_mine(&view)?;
                }
            }
            ("unkeep", "my") => {
                let kept = mine(py)?;
                let i: i64 = py.import("builtins")?.getattr("int")?.call1((key_rest(&key),))?.extract()?;
                if i >= 0 && (i as usize) < kept.len()? {
                    kept.call_method1("pop", (i,))?;
                    store_json(py, MINE_KEY, &kept)?;
                    bump_mine(&view)?;
                }
            }
            ("map" | "unmap", "m") => {
                let Some((scalar, label, channel)) = PBR_ROWS.iter().find(|(_, _, m)| *m == arg) else { return Ok(()) };
                if verb == "unmap" {
                    let kw = PyDict::new(py);
                    kw.set_item("name", &name)?;
                    kw.set_item(*channel, py.None())?;
                    core(ctx, "set_material_pbr", &kw)?;
                    return Ok(());
                }
                let path = lib(py)?.call_method1("_pick_image", (ctx.getattr("win")?, format!("{label} map")))?;
                if path.is_truthy()? {
                    let kw = PyDict::new(py);
                    kw.set_item("name", &name)?;
                    kw.set_item(*channel, [("path", &path)].into_py_dict(py)?)?;
                    let mat = scene.getattr("materials")?.call_method1("get", (&name,))?;
                    if !mat.is_none() {
                        let pbr = mat.getattr("pbr")?;
                        if !(pbr.is_truthy()? && pbr.contains(*scalar)?) {
                            kw.set_item(*scalar, 1.0)?; // a map with no strength would show nothing
                        }
                        if *channel == "emission_map" && !(pbr.is_truthy()? && pbr.contains("emission")?) {
                            kw.set_item("emission", [1.0, 1.0, 1.0])?; // the map's own colours
                        }
                    }
                    core(ctx, "set_material_pbr", &kw)?;
                }
            }
            ("image", "m") => {
                let path = lib(py)?.call_method1("_pick_image", (ctx.getattr("win")?, "Base image"))?;
                if path.is_truthy()? {
                    let mat = scene.getattr("materials")?.call_method1("get", (&name,))?;
                    let tex = PyDict::new(py);
                    if !mat.is_none() {
                        if let Some(t) = opt(&mat, "texture").filter(|t| t.is_truthy().unwrap_or(false)) {
                            tex.update(t.cast::<pyo3::types::PyMapping>()?)?;
                        }
                    }
                    tex.set_item("path", &path)?;
                    tex.call_method1("pop", ("embed", py.None()))?;
                    let kw = PyDict::new(py);
                    kw.set_item("name", &name)?;
                    kw.set_item("base", "image")?;
                    kw.set_item("texture", tex)?;
                    core(ctx, "set_material_base", &kw)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    #[pyo3(signature = (ctx, what, key, text=""))]
    fn confirmed(&self, py: Python<'_>, ctx: &Bound<'_, PyAny>, what: &str, key: &str, text: &str) -> PyResult<()> {
        let _ = text;
        if what == "delete" {
            core(ctx, "delete_material", &[("name", key_rest(key))].into_py_dict(py)?)?;
            ctx.getattr("view")?.setattr("_ui_mat_focus", py.None())?;
        }
        Ok(())
    }

    fn inspector<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>, item: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let key: String = item.get_item("key")?.extract()?;
        let scene = ctx.getattr("scene")?;
        if let Some(rest) = key.strip_prefix("uc:") {
            let _ = rest;
            let n = item.call_method1("get", ("uses", 0))?;
            let rows = vec![("Colour".into_pyobject(py)?.into_any(), item.get_item("name")?), ("Uses".into_pyobject(py)?.into_any(), n.str()?.into_any()), ("Name".into_pyobject(py)?.into_any(), s(py, "none (an older file)"))];
            let k = unnamed_colours_cached(&scene)?.len()?;
            let acts = vec![(format!("lib:name:{key}"), format!("Name these ({k})")), (format!("lib:select:{key}"), "Select uses".into()), (format!("lib:apply:{key}"), "Paint with it".into())];
            let sections = PyList::empty(py);
            sections.append(("Colour without a material", rows))?;
            sections.append(("Actions", acts))?;
            return Ok(Some(PyTuple::new(py, [s(py, "Unnamed colour"), sections.into_any()])?.into_any()));
        }
        if !key.starts_with("m:") {
            return Ok(None);
        }
        let mat = scene.getattr("materials")?.call_method1("get", (item.get_item("name")?,))?;
        if mat.is_none() {
            return Ok(None);
        }
        let ui = ui(py)?;
        let painting = ui.call_method1("_active_tool_id", (ctx.getattr("win")?,))?.eq("paint")? && crate::document_registration::view::calls::current_material_name(py)?.eq(mat.getattr("name")?)?;
        let focused = ctx.getattr("view")?.getattr("__dict__")?.call_method1("get", ("_ui_mat_focus",))?.eq(&key)?;
        Ok(Some(py.import("built_core")?.call_method1("material_inspector", (mat, item.call_method1("get", ("uses", 0))?, painting, focused))?))
    }

    fn edit(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, item: &Bound<'_, PyAny>, field: &str, text: Option<&str>) -> PyResult<()> {
        let py = slf.py();
        let name = item.get_item("name")?;
        let text = text.unwrap_or("").trim().to_owned();
        let scene = ctx.getattr("scene")?;
        let mat = || scene.getattr("materials")?.call_method1("get", (&name,));
        let textured = |m: &Bound<'_, PyAny>| -> PyResult<bool> {
            Ok(!m.is_none() && opt(m, "texture").map(|t| get(&t, "path").and_then(|p| p.is_truthy())).transpose()?.unwrap_or(false))
        };
        let builtins = py.import("builtins")?;
        let with_name = |pairs: Vec<(&str, Bound<'_, PyAny>)>| -> PyResult<Bound<'_, PyDict>> {
            let kw = PyDict::new(py);
            kw.set_item("name", &name)?;
            for (k, v) in pairs {
                kw.set_item(k, v)?;
            }
            Ok(kw)
        };
        if field == "name" {
            slf.call_method1("run", (ctx, "rename", item, &text))?;
        } else if field == "base" {
            if text.to_lowercase().starts_with("image") {
                if textured(&mat()?)? {
                    return Ok(());
                }
                let copy = PyDict::new(py);
                copy.update(item.cast::<pyo3::types::PyMapping>()?)?;
                copy.set_item("key", format!("m:{}", name.str()?))?;
                slf.call_method1("run", (ctx, "image", copy))?;
            } else {
                core(ctx, "set_material_base", &with_name(vec![("base", s(py, "color"))])?)?;
            }
        } else if field == "color" || field == "finish" || field.starts_with("brush_") {
            if field == "color" && textured(&mat()?)? {
                let (r, g, b) = parse_hex(py, &text)?;
                core(ctx, "edit_material", &with_name(vec![("color", PyList::new(py, [r, g, b])?.into_any())])?)?;
                return Ok(());
            }
            let new = py.import("built_core")?.call_method1("edit_swatch", (ctx.getattr("vp")?.getattr("history")?, paint_tool(py)?, &name, field, &text))?;
            let view = ctx.getattr("view")?;
            if !view.is_none() && view.getattr("__dict__")?.call_method1("get", ("_ui_mat_focus",))?.eq(format!("m:{}", name.str()?))? {
                view.setattr("_ui_mat_focus", format!("m:{}", new.str()?))?;
            }
            ctx.getattr("vp")?.call_method0("refresh_after_edit")?;
        } else if field == "tile" {
            let cleaned = text.replace(',', ".");
            let parts: Vec<&str> = cleaned.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).filter(|p| !p.is_empty()).collect();
            if parts.is_empty() {
                return Err(pyo3::exceptions::PyValueError::new_err("Type the tile size, like 1.2 × 0.6"));
            }
            let w: f64 = builtins.getattr("float")?.call1((parts[0],))?.extract()?;
            let h: f64 = if parts.len() > 1 { builtins.getattr("float")?.call1((parts[1],))?.extract()? } else { w };
            if w <= 0.0 || h <= 0.0 {
                return Err(pyo3::exceptions::PyValueError::new_err("A tile is larger than zero"));
            }
            core(ctx, "edit_material", &with_name(vec![("tile", PyList::new(py, [w, h])?.into_any())])?)?;
        } else if field == "emission" {
            let (r, g, b) = parse_hex(py, &text)?;
            let kw = with_name(vec![("emission", PyList::new(py, [r, g, b])?.into_any())])?;
            let strength = mat()?.getattr("pbr")?;
            if r + g + b > 0.0 && !(strength.is_truthy()? && strength.call_method1("get", ("emission_strength", 0.0))?.extract::<f64>()? > 0.0) {
                kw.set_item("emission_strength", 1.0)?; // a colour with no strength would show nothing
            }
            core(ctx, "set_material_pbr", &kw)?;
        } else if field == "pbr" || field.starts_with("percent_") {
            let values = py.import("built_core")?.call_method1("material_surface_input", (field, &text))?;
            let kw = with_name(vec![])?;
            kw.update(values.cast::<pyo3::types::PyMapping>()?)?;
            core(ctx, "set_material_pbr", &kw)?;
        } else if field == "opacity" {
            let v: f64 = builtins.getattr("float")?.call1((text.trim_end_matches('%'),))?.extract()?;
            if !(0.0..=100.0).contains(&v) {
                return Err(pyo3::exceptions::PyValueError::new_err("Opacity is between 0 and 100 %"));
            }
            core(ctx, "edit_material", &with_name(vec![("opacity", obj(py, v / 100.0)?.into_bound(py))])?)?;
        } else if PBR_ROWS.iter().any(|(c, _, _)| *c == field) {
            let value = if text.is_empty() { py.None().into_bound(py) } else { builtins.getattr("float")?.call1((text.replace(',', "."),))? };
            core(ctx, "set_material_pbr", &with_name(vec![(field, value)])?)?;
        }
        Ok(())
    }
}

/// Definitions in the model (copies counted, kept-in-file ones too), then
/// the bundled starters and 2D people.
#[pyclass(module = "built_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct ComponentsProvider;

/// Only mesh identities are needed to decide whether a row can replace any selected copy.
fn selected_definitions(scene: &Bound<'_, PyAny>) -> PyResult<std::collections::HashSet<usize>> {
    let mut meshes = std::collections::HashSet::new();
    for group in scene.getattr("selection")?.try_iter()? {
        let group = group?;
        if let Some(is_component) = group.getattr_opt("is_component")? {
            if is_component.call0()?.is_truthy()? {
                meshes.insert(group.getattr("mesh")?.as_ptr() as usize);
            }
        }
    }
    Ok(meshes)
}

const COMPONENT_COLLECTIONS: &str = "library_component_collections";

fn move_to_collection(st:&Bound<'_,PyAny>,ctx:&Bound<'_,PyAny>,picks:&Bound<'_,PySet>)->PyResult<()> {
    let py=ctx.py();
    let scene=ctx.getattr("scene")?;
    let version=scene.getattr("content_version")?.unbind();
    let model=provider(py,"components")?.call_method1("_model",(ctx,))?;
    let mut keys=Vec::<String>::new();
    for key in picks.iter() {
        let key:String=key.extract()?;
        if let Some(i)=key.strip_prefix("cm:").and_then(|s|s.parse::<usize>().ok()) {
            let row=model.get_item(i)?;
            let kept=row.get_item(3)?;
            for group in row.get_item(2)?.try_iter()? {keys.push(group?.getattr("uid")?.str()?.to_string());}
            if !kept.is_none(){keys.push(kept.getattr("uid")?.str()?.to_string());}
        } else {keys.push(key);}
    }
    let win=ctx.getattr("win")?;
    let vp=ctx.getattr("vp")?.unbind();
    let scene=scene.unbind();
    let done=func(py,move |py,a|{
        let name=a.get_item(0)?.extract::<Option<String>>()?.unwrap_or_default();
        let scene=scene.bind(py);
        if !name.trim().is_empty() && vp.bind(py).getattr("scene")?.is(scene) && scene.getattr("content_version")?.eq(version.bind(py))? {
            let entries=scene.getattr("plugin_data")?.call_method1("get",(COMPONENT_COLLECTIONS,PyDict::new(py)))?.call_method0("copy")?;
            for key in &keys {entries.set_item(key,name.trim())?;}
            let cmd=py.import("built_core.history")?.call_method1("SetPluginDataCommand",(COMPONENT_COLLECTIONS,entries))?;
            vp.bind(py).getattr("history")?.call_method1("execute",(cmd,))?;
            vp.bind(py).call_method0("notify_scene_changed")?;
        }
        Ok(none(py))
    })?;
    py.import("built_render.chrome_forms")?.getattr("ask_text")?.call((win,"Move to collection","Collection in this project"),Some(&[("done",done)].into_py_dict(py)?))?;
    st.set_item("bulk_mode","")?;
    Ok(())
}

#[pymethods]
impl ComponentsProvider {
    #[new]
    fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(ComponentsProvider)
    }

    #[classattr]
    fn id() -> &'static str {
        "components"
    }

    #[classattr]
    fn label() -> &'static str {
        "Components"
    }

    fn _model<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        ui(py)?.call_method1("_components_in_model", (ctx.getattr("view")?, ctx.getattr("win")?))
    }

    fn items<'py>(slf: &Bound<'py, Self>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let py = slf.py();
        let out = PyList::empty(py);
        let scene = ctx.getattr("scene")?;
        let selected = selected_definitions(&scene)?;
        let engine = py.import("built_core")?;
        let registry = py.import("core.bim_registry")?.call_method1("get", (&scene,))?;
        let structure = py.import("built_core.structure")?;
        let nodes = structure.call_method1("_nodes", (&registry,))?;
        let mut node_metadata = std::collections::HashMap::<String,(String,Option<String>)>::new();
        for (i, row) in slf.call_method1("_model", (ctx,))?.try_iter()?.enumerate() {
            let row = row?;
            let (name, groups, kept) = (row.get_item(0)?, row.get_item(2)?, row.get_item(3)?);
            let gs: Vec<Bound<'_, PyAny>> = groups.try_iter()?.collect::<PyResult<_>>()?;
            let mut paths = std::collections::BTreeSet::new();
            let mut classes = std::collections::BTreeSet::new();
            let mut locations = std::collections::BTreeMap::new();
            for group in gs.iter().chain((!kept.is_none()).then_some(&kept)) {
                let record=group.getattr("ifc")?;
                let own_class=if record.is_truthy()? {
                    let value=if record.call_method1("get",("type",))?.is_truthy()? {
                        py.import("core.record")?.call_method1("apply_type",(&record,&registry))?
                    }else{record};
                    value.call_method1("get",("class",))?.extract::<Option<String>>()?
                }else{None};
                if let Some(c)=&own_class {classes.insert(c.clone());}
                if group.is(&kept) {continue;}
                let ancestor = engine.call_method1("link_linked_ancestor",(group,))?;
                let path:String = if ancestor.is_none() {String::new()} else {
                    ancestor.getattr("ext")?.get_item("link")?.get_item("path")?.extract()?
                };
                let node = structure.call_method1("node_of",(group,))?.extract::<Option<String>>()?.unwrap_or_default();
                if !node_metadata.contains_key(&node) {
                    let metadata = if !node.is_empty() && nodes.contains(&node)? {
                        let name=nodes.get_item(&node)?.get_item("name")?.extract()?;
                        let cls=structure.call_method1("resolve",(&registry,&node))?.call_method1("get",("class",))?.extract()?;
                        (name,cls)
                    } else {("Unfiled".to_owned(),None)};
                    node_metadata.insert(node.clone(),metadata);
                }
                let (name,cls)=&node_metadata[&node];
                if own_class.is_none() {if let Some(cls)=cls {classes.insert(cls.clone());}}
                *locations.entry(format!("{} › {name}",if path.is_empty() {"This model"}else{&path})).or_insert(0usize)+=1;
                paths.insert(path);
            }
            let linked = !paths.is_empty() && !paths.contains("");
            let files:Vec<_>=paths.into_iter().filter(|s|!s.is_empty()).collect();
            let file = if linked {files.first().cloned()} else {None};
            let explicit_class = kept.getattr("ifc").ok().and_then(|v|v.call_method1("get",("class",)).ok()).and_then(|v|v.extract::<String>().ok()).unwrap_or_default();
            let mut v = if gs.is_empty() {
                vec!["apply", "replace", "rename", "-", "unkeepfile"]
            } else {
                vec!["apply", "edit", "select", "replace", "-", "rename", if kept.is_none() { "keepfile" } else { "unkeepfile" }, "save", "-", "delete"]
            };
            if linked { v.retain(|id|matches!(*id,"apply"|"select")); }
            let definition = gs.first().unwrap_or(&kept);
            let mesh = definition.getattr("mesh")?;
            if !selected.iter().any(|&id| id != mesh.as_ptr() as usize) {
                v.retain(|verb| *verb != "replace");
            }
            out.append(item(py, &[
                ("key", s(py, &format!("cm:{i}"))),
                ("file",obj(py,file)?.into_bound(py)),
                ("files",obj(py,files)?.into_bound(py)),
                ("explicit_class",s(py,&explicit_class)),
                ("locations",obj(py,locations)?.into_bound(py)),
                ("classes",obj(py,classes.into_iter().collect::<Vec<_>>())?.into_bound(py)),
                ("definition_id", obj(py, mesh.as_ptr() as usize)?.into_bound(py)),
                ("name", if name.is_truthy()? { name } else { s(py, "Component") }),
                ("source", s(py, if linked { "linked" } else { "model" })),
                ("category", s(py, if gs.is_empty() { "Kept, not placed" } else { "" })),
                ("uses", obj(py, gs.len())?.into_bound(py)),
                ("hex", s(py, "")),
                ("thumb", s(py, &format!("component:{}:{}:{i}", scene.as_ptr() as usize, scene.getattr("content_version")?))),
                ("verbs", verbs(py, &v)?),
            ])?)?;
        }
        let res = crate::document_registration::view::calls::components_dir(py)?;
        for pair in crate::document_registration::view::calls::starter_components(py)?.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let (key, e) = (pair.get_item(0)?, pair.get_item(1)?);
            let people = key.str()?.to_string().starts_with("pp:");
            let k = e.get_item("key")?.str()?.to_string();
            let thumb = res.div(if people { format!("{k}.png") } else { format!("thumbs/{k}.png") })?;
            let thumb = if thumb.call_method0("exists")?.is_truthy()? { thumb.str()?.into_any() } else { py.None().into_bound(py) };
            out.append(item(py, &[
                ("key", key),
                ("name", e.get_item("name")?),
                ("source", s(py, "bundled")),
                ("category", s(py, if people { "People (2D)" } else { "Starters (3D)" })),
                ("uses", obj(py, 0)?.into_bound(py)),
                ("hex", s(py, "")),
                ("thumb", thumb),
                ("verbs", verbs(py, &["apply"])?),
            ])?)?;
        }
        for row in destinations::py_items(py, Some(&scene), &["components"])? {
            out.append(row)?;
        }
        for manifest in collections::manifests()? {
            for original in manifest["items"].as_array().into_iter().flatten() {
                let mut entry=original.clone();
                entry["collection"]=manifest["name"].clone();
                let row=loads(py,&s(py,&entry.to_string()))?;
                out.append(row)?;
            }
        }
        let assigned=scene.getattr("plugin_data")?.call_method1("get",(COMPONENT_COLLECTIONS,PyDict::new(py)))?;
        if assigned.is_truthy()? {
            let rows=slf.call_method1("_model",(ctx,))?;
            for item in out.iter() {
                let key:String=item.get_item("key")?.extract()?;
                let mut label=assigned.call_method1("get",(&key,))?;
                if let Some(i)=key.strip_prefix("cm:").and_then(|s|s.parse::<usize>().ok()) {
                    let row=rows.get_item(i)?;
                    let kept=row.get_item(3)?;
                    for group in row.get_item(2)?.try_iter()?.chain((!kept.is_none()).then_some(Ok(kept))) {
                        label=assigned.call_method1("get",(group?.getattr("uid")?.str()?,))?;
                        if !label.is_none() {break;}
                    }
                }
                if !label.is_none(){item.set_item("collection",label)?;}
            }
        }
        Ok(out)
    }

    fn _row<'py>(slf: &Bound<'py, Self>, ctx: &Bound<'py, PyAny>, item: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let key: String = item.get_item("key")?.extract()?;
        let Ok(i) = key_rest(&key).parse::<usize>() else { return Ok(None) };
        let model = slf.call_method1("_model", (ctx,))?;
        Ok(if i < model.len()? { Some(model.get_item(i)?) } else { None })
    }

    fn type_actions<'py>(slf: &Bound<'py, Self>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let py = slf.py();
        let model=slf.call_method1("_model",(ctx,))?;
        let st=state(&ctx.getattr("view")?)?;
        let cached=st.call_method1("get",("kept_count",))?;
        let n:usize=if !cached.is_none() && cached.get_item(0)?.eq(model.as_ptr() as usize)? && cached.get_item(2)?.eq(ctx.getattr("scene")?.getattr("content_version")?)? {
            cached.get_item(1)?.extract()?
        }else{
            let mut n=0;
            for row in model.try_iter()? {
                let row=row?;
                if !row.get_item(3)?.is_none() && !row.get_item(2)?.is_truthy()? {n+=1;}
            }
            st.set_item("kept_count",(model.as_ptr() as usize,n,ctx.getattr("scene")?.getattr("content_version")?))?;
            n
        };
        let actions = PyList::empty(py);
        actions.append(("import", crate::model_edit::translated(py, "Components from File…", None)?))?;
        if n > 0 {
            actions.append(("purge", format!("Stop keeping unplaced components ({n})")))?;
        }
        Ok(actions)
    }

    fn type_action(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, action: &str) -> PyResult<()> {
        let py = slf.py();
        if action == "import" {
            return crate::component_files::choose_kept_components(&ctx.getattr("win")?);
        }
        if action != "purge" {
            return Ok(());
        }
        let history = py.import("built_core.history")?;
        let mut idle = Vec::new();
        for row in slf.call_method1("_model", (ctx,))?.try_iter()? {
            let row = row?;
            let kept = row.get_item(3)?;
            if !kept.is_none() && !row.get_item(2)?.is_truthy()? {
                idle.push(history.getattr("KeepComponentCommand")?.call1((kept, false))?);
            }
        }
        let n = idle.len();
        if n != 0 {
            let vp = ctx.getattr("vp")?;
            vp.getattr("history")?.call_method1("execute", (history.getattr("CompoundCommand")?.call1((idle,))?,))?;
            vp.call_method0("notify_scene_changed")?;
            flash(ctx, &format!("{n} unplaced components dropped"))?;
        } else {
            flash(ctx, "Every kept component is placed")?;
        }
        Ok(())
    }

    #[pyo3(signature = (ctx, verb, item, arg=""))]
    fn run(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, verb: &str, item: &Bound<'_, PyAny>, arg: &str) -> PyResult<()> {
        let py = slf.py();
        let ui = ui(py)?;
        let key: String = item.get_item("key")?.extract()?;
        let win = ctx.getattr("win")?;
        if key.starts_with("family:") {
            return FamiliesProvider.run(py,ctx,verb,item,arg);
        }
        if !key.starts_with("cm:") {
            if verb == "apply" {
                ui.call_method1("_library_pick", (&win, ctx.getattr("view")?, &key))?;
            }
            return Ok(());
        }
        let Some(row) = slf.call_method1("_row", (ctx, item))?.extract::<Option<Bound<'_, PyAny>>>()? else { return Ok(()) };
        let (name, groups, kept) = (row.get_item(0)?, row.get_item(2)?, row.get_item(3)?);
        let gs: Vec<Bound<'_, PyAny>> = groups.try_iter()?.collect::<PyResult<_>>()?;
        let definition = if !kept.is_none() { kept.clone() } else if let Some(g) = gs.first() { g.clone() } else { return Ok(()) };
        let scene = ctx.getattr("scene")?;
        match verb {
            "apply" => {
                let kc = py.import("built_core.kept_components")?;
                let template = if !kept.is_none() { kept.clone() } else { kc.call_method1("make_template", (&definition,))? };
                win.call_method1("_start_place", (kc.call_method1("place_copy", (template,))?,))?;
            }
            "edit" if !gs.is_empty() => {
                let top = scene.getattr("groups")?;
                let Some(g) = gs.iter().find(|g| top.contains(*g).unwrap_or(false)) else {
                    return flash(ctx, "Its copies are nested: open their group first");
                };
                ui.call_method1("_select", (&win, vec![g]))?;
                ctx.getattr("vp")?.call_method1("begin_group_edit", (g,))?;
            }
            "unique" => {
                let chosen=gs.iter().find(|g|g.getattr("uid").is_ok_and(|v|v.str().is_ok_and(|s|s.to_string()==arg)));
                if let Some(g)=chosen {
                    if py.import("built_core")?.call_method1("link_linked_ancestor",(g,))?.is_none() && !scene.call_method1("entity_locked",(g,))?.is_truthy()? {
                        crate::context_edits::unique_rows(&win,&PyList::new(py,[g])?,false)?;
                    }
                }
            }
            "select" => {
                render(py)?.call_method1("select_component_instances", (&win, &groups))?;
            }
            "replace" => {
                let group_cls = py.import("built_core.group")?.getattr("Group")?;
                let mesh = definition.getattr("mesh")?;
                let mut swap = false;
                for g in scene.getattr("selection")?.try_iter()? {
                    let g = g?;
                    if g.is_instance(&group_cls)? && g.call_method0("is_component")?.is_truthy()? && !g.getattr("mesh")?.is(&mesh) {
                        swap = true;
                    }
                }
                if swap {
                    win.call_method1("replace_selection_with", (&definition,))?;
                } else {
                    flash(ctx, "Select copies of another component first")?;
                }
            }
            "rename" if !arg.trim().is_empty() => {
                let kw = PyDict::new(py);
                kw.set_item("placement", definition.getattr("uid")?)?;
                kw.set_item("name", arg.trim())?;
                core(ctx, "rename_component", &kw)?;
            }
            "keepfile" | "unkeepfile" => {
                win.call_method1("keep_component", (&definition, verb == "keepfile"))?;
            }
            "save" => {
                // Its definition in a .built of its own, carried by the saved item.
                let uid = definition.getattr("uid")?.str()?.to_string();
                let id = crate::library_store::stable_id(&format!("component:{uid}"));
                let folder = std::env::temp_dir().join(format!("built-component-{}", crate::library_store::new_id()));
                let file = destinations::component_file(py, &definition, &folder)?;
                let label = if name.is_truthy()? { name.str()?.to_string() } else { "Component".to_owned() };
                destinations::save_to_library(&win, "components", &label, r#"{"file":"component.built"}"#, Some(id), 1, "", None, None,
                                              Some(vec![("component.built".into(), file)]), Some(vec!["model".into()]), None)?;
            }
            "delete" => {
                let n = gs.len();
                ask(&ctx.getattr("view")?, ("delete", "components", key.clone()), &format!("Delete every copy of “{}”?", name.str()?),
                    &format!("{n} cop{} and their geometry go (Undo brings them back).", if n != 1 { "ies" } else { "y" }), "Delete")?;
            }
            _ => {}
        }
        Ok(())
    }

    #[pyo3(signature = (ctx, what, key, text=""))]
    fn confirmed(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, what: &str, key: &str, text: &str) -> PyResult<()> {
        let py = slf.py();
        let _ = text;
        let it = [("key", key)].into_py_dict(py)?;
        let row: Option<Bound<'_, PyAny>> = slf.call_method1("_row", (ctx, it))?.extract()?;
        if let (Some(row), "delete") = (row, what) {
            let uids: Vec<Bound<'_, PyAny>> = row.get_item(2)?.try_iter()?.map(|g| g?.getattr("uid")).collect::<PyResult<_>>()?;
            match core(ctx, "delete_placements", &[("placements", uids)].into_py_dict(py)?) {
                Ok(_) => {}
                Err(e) if e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) => flash(ctx, "Some copies are nested in other groups: delete those from the tree")?,
                Err(e) => return Err(e),
            }
            ctx.getattr("vp")?.call_method0("update")?;
        }
        Ok(())
    }

    fn inspector<'py>(slf: &Bound<'py, Self>, ctx: &Bound<'py, PyAny>, item: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let py = slf.py();
        let key: String = item.get_item("key")?.extract()?;
        if !key.starts_with("cm:") {
            return Ok(None);
        }
        let Some(row) = slf.call_method1("_row", (ctx, item))?.extract::<Option<Bound<'_, PyAny>>>()? else { return Ok(None) };
        let (name, faces, groups, kept) = (row.get_item(0)?, row.get_item(1)?, row.get_item(2)?, row.get_item(3)?);
        let faces: i64 = faces.extract()?;
        let grouped = faces.to_string().as_bytes().rchunks(3).rev().map(|c| std::str::from_utf8(c).unwrap_or("")).collect::<Vec<_>>().join(" ");
        let rows = vec![("Copies".to_owned(), groups.len()?.to_string()), ("Faces".to_owned(), grouped), ("Keep when unused".to_owned(), if kept.is_none() { "no" } else { "yes" }.to_owned())];
        let acts = vec![(format!("lib:apply:{key}"), "Place a copy".to_owned()), (format!("lib:select:{key}"), "Select copies".to_owned()), (format!("lib:edit:{key}"), "Open for editing".to_owned())];
        let sections = PyList::empty(py);
        sections.append(("Component in this model", rows))?;
        sections.append(("Actions", acts))?;
        let title = if name.is_truthy()? { name } else { s(py, "Component") };
        Ok(Some(PyTuple::new(py, [title, sections.into_any()])?.into_any()))
    }
}

#[pyclass(module = "built_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct FamiliesProvider;

#[pyclass]
struct FamilyCollections(Vec<serde_json::Value>);

#[pymethods]
impl FamiliesProvider {
    #[new]
    fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(FamiliesProvider)
    }

    #[classattr]
    fn id() -> &'static str { "families" }

    #[classattr]
    fn label() -> &'static str { "Families" }

    #[pyo3(signature = (ctx, verb, item, _arg=""))]
    fn run(&self, py: Python<'_>, ctx: &Bound<'_, PyAny>, verb: &str, item: &Bound<'_, PyAny>, _arg: &str) -> PyResult<()> {
        if verb != "apply" { return Ok(()); }
        let file: std::path::PathBuf = item.get_item("file")?.extract()?;
        let scene = ctx.getattr("scene")?;
        let cache = scene.getattr("__dict__")?.call_method1("setdefault", ("_library_family_templates", PyDict::new(py)))?;
        let index: usize = item.get_item("index")?.extract()?;
        let key = (&file, index);
        let mut template = cache.call_method1("get", (key,))?;
        let category: String = item.get_item("family_category")?.extract()?;
        let class = match category.to_lowercase().as_str() {
            "door" | "doors" => Some("IfcDoor"), "window" | "windows" => Some("IfcWindow"), _ => None,
        };
        if template.is_none() {
            let payload = json(py)?.call_method1("loads", (std::fs::read_to_string(&file)?,))?;
            payload.set_item("types", PyList::new(py, [payload.get_item("types")?.get_item(index)?])?)?;
            template = py.import("built_core")?.call_method1("rfa_library_definitions", (payload,))?.get_item(0)?;
            template.getattr("ext")?.set_item("library_family_type", item.get_item("key")?)?;
            if let Some(class) = class {
                template.getattr("ifc")?.set_item("class", class)?;
                template.getattr("ext")?.set_item("library_host", "wall")?;
                template.getattr("ext")?.set_item("junction", if class == "IfcDoor" { "30e27bad196ea165" } else { "5c9f9bb202d14a8c" })?;
            }
            cache.set_item(key, &template)?;
        }
        let group = py.import("built_core.kept_components")?.call_method1("place_copy", (template,))?;
        ctx.getattr("win")?.call_method1("_start_place", (group, class.is_some()))?;
        Ok(())
    }

    fn items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let mut manifests = collections::manifests()?;
        let mut rows = Vec::new();
        for manifest in &mut manifests {
            if let serde_json::Value::Array(items) = manifest["items"].take() {
                rows.extend(items);
            }
        }
        ctx.getattr("view")?.setattr("_library_collections", Bound::new(py, FamilyCollections(manifests))?)?;
        json(py)?.call_method1("loads", (serde_json::Value::Array(rows).to_string(),))
    }
}

/// Display styles: Built's own, then the saved ones wherever they are.
#[pyclass(module = "built_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct StylesProvider;

#[pymethods]
impl StylesProvider {
    #[new]
    fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(StylesProvider)
    }

    #[classattr]
    fn id() -> &'static str {
        "styles"
    }

    #[classattr]
    fn label() -> &'static str {
        "Styles"
    }

    fn items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        for style in py.import("built_core.style")?.getattr("BUILTIN_STYLES")?.try_iter()? {
            let style = style?;
            let name: String = style.getattr("name")?.extract()?;
            out.append(item(py, &[
                ("key", s(py, &format!("style:{name}"))),
                ("name", s(py, &name)),
                ("label", s(py, &crate::settings_rows::tr(py, &name)?)),
                ("source", s(py, "bundled")),
                ("category", s(py, "")),
                ("hex", s(py, &_hex(&style.getattr("background")?)?)),
                ("verbs", verbs(py, &["apply"])?),
            ])?)?;
        }
        let scene = ctx.getattr_opt("scene")?;
        for row in destinations::py_items(py, scene.as_ref(), &["styles"])? {
            out.append(row)?;
        }
        Ok(out)
    }

    #[pyo3(signature = (ctx, verb, item, _arg=""))]
    fn run(&self, py: Python<'_>, ctx: &Bound<'_, PyAny>, verb: &str, item: &Bound<'_, PyAny>, _arg: &str) -> PyResult<()> {
        if verb == "apply" {
            let style = py.import("built_core.style")?.call_method1("style_by_name", (item.get_item("name")?,))?;
            if !style.is_none() {
                ctx.getattr("win")?.call_method1("set_active_style", (style,))?;
            }
        }
        Ok(())
    }
}

/// Section outlines (steel shapes, extrusion and sweep profiles) to
/// draw, extrude and run along a path.
#[pyclass(module = "built_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct ProfilesProvider;

#[pymethods]
impl ProfilesProvider {
    #[new]
    fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(ProfilesProvider)
    }

    #[classattr]
    fn id() -> &'static str {
        "profiles"
    }

    #[classattr]
    fn label() -> &'static str {
        "Profiles"
    }

    /// Shared with the sketch workspace's Save to Library action.
    #[staticmethod]
    fn saved_directory()->PyResult<PathBuf>{
        let profile=std::env::var_os("BUILT_SETTINGS_DIR").map(PathBuf::from);
        crate::platform_paths::local_data_directory(profile).map(|d|d.join("Built").join("profiles"))
            .ok_or_else(||pyo3::exceptions::PyRuntimeError::new_err("Library directory unavailable"))
    }

    fn items<'p>(&self,py:Python<'p>,ctx:&Bound<'p,PyAny>)->PyResult<Bound<'p,PyAny>> {
        let catalogue:serde_json::Value=serde_json::from_str(include_str!("../../../resources/profiles/starter.builtprofile")).expect("bundled profiles");
        let mut items:Vec<_>=catalogue["table"]["sizes"].as_array().unwrap().iter().map(|size|{
            let mut details=vec![("Family".to_owned(),size["family"].as_str().unwrap().to_owned(),"")];
            for (key,v) in size["params"].as_object().unwrap(){
                if v.as_f64()!=Some(0.){details.push((key.to_owned(),format!("{v} mm"),""));}
            }
            for (key,title) in [("standard","Standard"),("source","Source")]{
                if let Some(v)=size[key].as_str().filter(|s|!s.is_empty()){details.push((title.into(),v.into(),""));}
            }
            serde_json::json!({"key":size["key"],"name":size["label"],"category":size["family"],
                "thumb":format!("profile:{}",size["key"].as_str().unwrap()),"defaults":details,
                "aliases":size.get("aliases").cloned().unwrap_or_else(||serde_json::json!([])),
                "source":"bundled","dimensions":size["params"],"verbs":["apply","size"]})
        }).collect();
        let scene=ctx.getattr_opt("scene")?.filter(|s|!s.is_none() && s.hasattr("_root").unwrap_or(false));
        let saved:std::collections::HashSet<String>=destinations::saved_ids(py,scene.as_ref(),"profiles")?.into_iter().chain(destinations::saved_ids(py,scene.as_ref(),"sections")?).collect();
        if let Ok(files)=std::fs::read_dir(Self::saved_directory()?){
            let mut files:Vec<_>=files.flatten().map(|e|e.path()).filter(|p|p.extension().is_some_and(|s|s=="builtprofile")).collect();
            files.sort();
            for file in files{
                let Some(data)=std::fs::read_to_string(&file).ok().and_then(|text|serde_json::from_str::<serde_json::Value>(&text).ok()) else{continue};
                if data["format"]!="built.profile" || data["v"]!=1 || data["use"]!="profile" || !data["sketch"]["curves"].is_array(){continue;}
                let name=data["name"].as_str().unwrap_or("Drawn profile");
                let mut sketch=data["sketch"].clone();
                sketch["name"]=serde_json::json!(name);
                if saved.contains(&crate::library_store::legacy_profile_id(&file)) {continue;}
                items.push(serde_json::json!({"key":format!("local:{}",file.file_name().unwrap_or_default().to_string_lossy()),"name":name,"category":"My profiles","source":"local","sketch":sketch,"verbs":["apply","edit"]}));
            }
        }
        if let Some(scene)=ctx.getattr_opt("scene")?.filter(|s|!s.is_none() && s.hasattr("_root").unwrap_or(false)){
            let rows=py.import("built_core.command_protocol")?.call_method1("live_rows",(scene,))?;
            let rows:Vec<serde_json::Value>=serde_json::from_str(&json(py)?.call_method1("dumps",(rows,))?.extract::<String>()?).unwrap_or_default();
            // A member's frozen copy of a released sketch is its own, not a choice.
            for row in rows.iter().filter(|r|r["kind"]=="sketch" && r["use"]!="section" && r["state"]=="ok" && r["frozen"]!=true){
                items.push(serde_json::json!({"key":format!("sketch:{}",row["uid"].as_str().unwrap_or_default()),"name":row["name"].as_str().unwrap_or("Drawn profile"),"category":"Sketches","source":"model","sketch":row,"verbs":["apply","edit"]}));
            }
        }
        let mut catalog = presets::listed(ctx)?;
        for entry in destinations::entries(py,scene.as_ref(), &["presets"])? {
            catalog.push((entry.entry.data, entry.dest.key));
        }
        for (preset, source) in catalog {
            if preset["source"]["profile_size"] == true {
                let profile = &preset["values"]["profile"];
                items.push(serde_json::json!({"key":format!("size:{source}:{}",preset["uid"]),
                    "name":preset["name"],"category":profile["family"],"source":source,
                    "aliases":profile["catalogue"].get("aliases").cloned().unwrap_or_else(||serde_json::json!([])),
                    "profile_preset":preset,"thumb":presets::profile_thumb(profile),"verbs":["apply"]}));
            }
        }
        // Saved profiles; saved sections are Library › Sections (SectionsProvider).
        for row in destinations::items(py,scene.as_ref(),&["profiles"])? {
            items.push(row);
        }
        json(py)?.call_method1("loads",(serde_json::to_string(&items).unwrap(),))
    }
    /// The header's New profile… and Import drawing… (sketch_workspace.rs).
    fn type_action(&self, ctx: &Bound<'_, PyAny>, action: &str) -> PyResult<()> {
        if action == "pack" { return packs::ask_import(ctx); }
        sketch_entry(ctx, action, false)
    }
    #[pyo3(signature=(ctx,verb,item,_arg=""))]
    fn run(&self,ctx:&Bound<'_,PyAny>,verb:&str,item:&Bound<'_,PyAny>,_arg:&str)->PyResult<()> {
        if verb == "size" {
            return profile_dimensions(ctx, item);
        }
        if verb=="apply" {
            let win=ctx.getattr("win")?;
            win.call_method1("_activate_tool",("sweep",))?;
            let machine=win.getattr("_tools")?.get_item("sweep")?.getattr("_machine")?;
            if item.contains("profile_preset")? { machine.call_method1("set",("preset",item.get_item("profile_preset")?))?; }
            else if item.contains("sketch")? {machine.call_method1("set",("sketch",item.get_item("sketch")?))?;}
            else {machine.call_method1("set",("profile",item.get_item("key")?))?;}
        }
        if verb=="edit" {
            // Drawn profiles open in the Sketch workspace (sketch_workspace.rs).
            let key:String=item.get_item("key")?.extract()?;
            let win=ctx.getattr("win")?;
            cs(ctx.py())?.call_method1("close",(&win,))?;
            if key.starts_with("saved:") {
                let (Some((dest,kind,id)),scene)=(destinations::parse_key(&key),ctx.getattr("scene")?) else {return Ok(())};
                if let Some(l)=destinations::entries(ctx.py(),Some(&scene),&[kind.as_str()])?.into_iter().find(|l|l.dest.key==dest && l.entry.id==id) {
                    let arg=serde_json::json!({"id":l.entry.id,"rev":l.entry.rev,"data":l.entry.data}).to_string();
                    crate::sketch_workspace::enter(&win,"library",&arg)?;
                }
            } else if let Some(file)=key.strip_prefix("local:") {
                let path=Self::saved_directory()?.join(file);
                crate::sketch_workspace::enter(&win,"open",&path.to_string_lossy())?;
            } else if let Some(uid)=key.strip_prefix("sketch:") {
                crate::sketch_workspace::enter(&win,"edit",uid)?;
            }
        }
        Ok(())
    }
}

/// Arm Sweep with a user-sized family without changing its Library defaults.
fn profile_dimensions(ctx: &Bound<'_, PyAny>, item: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = ctx.py();
    let forms = py.import("built_render.chrome_forms")?;
    let fields = PyList::empty(py);
    let params = item.get_item("dimensions")?;
    let circular = matches!(item.get_item("category")?.extract::<String>()?.as_str(), "round" | "chs");
    let mut keys = Vec::new();
    for key in ["width", "height", "web", "flange", "radius"] {
        if !params.contains(key)? || (circular && key == "height") { continue; }
        let value = params.get_item(key)?.extract::<f64>()?;
        if matches!(key, "web" | "flange") && value == 0. { continue; }
        let label = if circular && key == "width" { "Diameter" } else { key };
        fields.append(forms.getattr("Field")?.call1((key, "edit", format!("{label} (mm)"), value.to_string()))?)?;
        keys.push(key);
    }
    let library = item.get_item("key")?.extract::<String>()?;
    let win = ctx.getattr("win")?;
    let window = win.clone().unbind();
    let done = func(py, move |py, args| {
        if args.get_item(0)?.eq("ok")? {
            let values = args.get_item(1)?;
            let dims = PyDict::new(py);
            for key in &keys {
                let text = values.get_item(key)?.str()?.to_string();
                let Some(num) = text.trim().parse::<f64>().ok().filter(|n| n.is_finite()) else {
                    window.bind(py).call_method1("notify", ("warning", "Invalid profile dimensions"))?;
                    return Ok(none(py));
                };
                dims.set_item(key, num)?;
                if circular && *key == "width" {
                    dims.set_item("height", num)?;
                }
            }
            let profile = PyDict::new(py);
            profile.set_item("library", &library)?;
            profile.set_item("params", dims)?;
            let win = window.bind(py);
            win.call_method1("_activate_tool", ("sweep",))?;
            let machine = win.getattr("_tools")?.get_item("sweep")?.getattr("_machine")?;
            if machine.call_method1("set", ("dimensions", profile))?.is_truthy()? {
                cs(py)?.call_method1("close", (win,))?;
            } else {
                win.call_method1("notify", ("warning", "Invalid profile dimensions"))?;
            }
        }
        Ok(none(py))
    })?;
    let kw = PyDict::new(py);
    kw.set_item("on_done", done)?;
    let form = forms.getattr("Form")?.call(("Profile dimensions", fields), Some(&kw))?;
    forms.call_method1("show", (win, form))?;
    Ok(())
}

/// New and Import from the Profiles and Sections tabs: the sheet goes and
/// the Sketch workspace comes (Import asks for the file first).
fn sketch_entry(ctx: &Bound<'_, PyAny>, action: &str, section: bool) -> PyResult<()> {
    let py = ctx.py();
    let win = ctx.getattr("win")?;
    match action {
        "new" => {
            cs(py)?.call_method1("close", (&win,))?;
            crate::sketch_workspace::enter(&win, "new", if section { "section" } else { "" })?;
        }
        "import" => {
            cs(py)?.call_method1("close", (&win,))?;
            render(py)?.getattr("shell_files")?.call_method1("command", (&win, "import:sketch"))?;
        }
        _ => {}
    }
    Ok(())
}

/// Drawn cutter sections (a sketch with `use: section`): the Library's
/// own files and this model's sketch rows. Cut uses them; here they are
/// drawn, imported and edited.
#[pyclass(module = "built_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct SectionsProvider;

#[pymethods]
impl SectionsProvider {
    #[new]
    fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(SectionsProvider)
    }

    #[classattr]
    fn id() -> &'static str {
        "sections"
    }

    #[classattr]
    fn label() -> &'static str {
        "Sections"
    }

    fn items<'p>(&self, py: Python<'p>, ctx: &Bound<'p, PyAny>) -> PyResult<Bound<'p, PyAny>> {
        let scene = ctx.getattr_opt("scene")?.filter(|s| !s.is_none() && s.hasattr("_root").unwrap_or(false));
        // Saved sections, in every destination (docs/library.md).
        let mut items = destinations::items(py, scene.as_ref(), &["sections"])?;
        for row in &mut items {
            if let Some(sketch) = row.get("sketch").filter(|s| s["curves"].is_array()).cloned() {
                let mut details: Vec<serde_json::Value> = row["defaults"].as_array().cloned().unwrap_or_default();
                details.extend(section_rows(&sketch).into_iter().map(|r| serde_json::json!(r)));
                row["defaults"] = serde_json::json!(details);
            }
        }
        // Older loose files not yet in the store.
        let saved = destinations::saved_ids(py, scene.as_ref(), "sections")?;
        if let Ok(files) = std::fs::read_dir(ProfilesProvider::saved_directory()?) {
            let mut files: Vec<_> = files.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|s| s == "builtprofile")).collect();
            files.sort();
            for file in files {
                let Some(data) = std::fs::read_to_string(&file).ok().and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()) else { continue };
                if data["format"] != "built.profile" || data["v"] != 1 || data["use"] != "section" || !data["sketch"]["curves"].is_array() {
                    continue;
                }
                if saved.contains(&crate::library_store::legacy_profile_id(&file)) {
                    continue;
                }
                let name = data["name"].as_str().unwrap_or("Drawn section");
                items.push(serde_json::json!({"key": format!("local:{}", file.file_name().unwrap_or_default().to_string_lossy()), "name": name,
                    "category": "", "source": "mine", "defaults": section_rows(&data["sketch"]), "verbs": ["edit"]}));
            }
        }
        // This model's section sketches (a member's frozen copy included:
        // a section is edited here, never picked for a sweep).
        if let Some(scene) = scene {
            let rows = py.import("built_core.command_protocol")?.call_method1("live_rows", (scene,))?;
            let rows: Vec<serde_json::Value> = serde_json::from_str(&json(py)?.call_method1("dumps", (rows,))?.extract::<String>()?).unwrap_or_default();
            for row in rows.iter().filter(|r| r["kind"] == "sketch" && r["use"] == "section") {
                items.push(serde_json::json!({"key": format!("sketch:{}", row["uid"].as_str().unwrap_or_default()), "name": row["name"].as_str().unwrap_or("Drawn section"),
                    "category": "Sketches", "source": "model", "defaults": section_rows(row), "verbs": ["edit"]}));
            }
        }
        json(py)?.call_method1("loads", (serde_json::Value::Array(items).to_string(),))
    }

    /// The header's New section… (sketch_workspace.rs).
    fn type_action(&self, ctx: &Bound<'_, PyAny>, action: &str) -> PyResult<()> {
        sketch_entry(ctx, action, true)
    }

    #[pyo3(signature=(ctx, verb, item, _arg=""))]
    fn run(&self, ctx: &Bound<'_, PyAny>, verb: &str, item: &Bound<'_, PyAny>, _arg: &str) -> PyResult<()> {
        // Enter or a double-click edits: a section has no place in the model
        // of its own (Cut places it).
        if !matches!(verb, "apply" | "edit") {
            return Ok(());
        }
        let py = ctx.py();
        let key: String = item.get_item("key")?.extract()?;
        let win = ctx.getattr("win")?;
        cs(py)?.call_method1("close", (&win,))?;
        if key.starts_with("saved:") {
            let (Some((dest, kind, id)), scene) = (destinations::parse_key(&key), ctx.getattr("scene")?) else { return Ok(()) };
            if let Some(l) = destinations::entries(py, Some(&scene), &[kind.as_str()])?.into_iter().find(|l| l.dest.key == dest && l.entry.id == id) {
                let arg = serde_json::json!({"id": l.entry.id, "rev": l.entry.rev, "data": l.entry.data}).to_string();
                crate::sketch_workspace::enter(&win, "library", &arg)?;
            }
        } else if let Some(file) = key.strip_prefix("local:") {
            let path = ProfilesProvider::saved_directory()?.join(file);
            crate::sketch_workspace::enter(&win, "open", &path.to_string_lossy())?;
        } else if let Some(uid) = key.strip_prefix("sketch:") {
            crate::sketch_workspace::enter(&win, "edit", uid)?;
        }
        Ok(())
    }
}

/// A section's facts for the detail pane: extent and size.
fn section_rows(sketch: &serde_json::Value) -> Vec<(String, String, String)> {
    let mut rows = Vec::new();
    if let Ok(s) = built_draft2d::Sketch::from_json(sketch) {
        if let Some((lo, hi)) = s.bounds() {
            let size = hi - lo;
            rows.push(("Size".to_owned(), format!("{} × {} mm", trim_mm(size.x), trim_mm(size.y)), String::new()));
        }
        if let Some(e) = &s.extrude {
            rows.push(("Extent".to_owned(), serde_json::to_value(e).ok().and_then(|v| v["extent"].as_str().map(str::to_owned)).unwrap_or_default().replace('_', " "), String::new()));
        }
    }
    rows
}

fn trim_mm(v: f64) -> String {
    let t = format!("{v:.1}");
    t.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The bundled catalogue by key, built once per resources folder and
/// language.
#[pyfunction]
fn _bundled_by_key<'py>(py: Python<'py>, resources: &Bound<'py, PyAny>, language: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let memo = lib(py)?.getattr("_bundled_memo")?;
    let key = (resources.str()?, language.str()?);
    if let Some(hit) = memo.cast::<PyDict>()?.get_item(&key)? {
        return Ok(hit);
    }
    let items = loads(py, &py.import("built_core")?.call_method1("bundled_materials", (resources, language))?)?;
    let out = PyDict::new(py);
    for it in items.try_iter()? {
        let it = it?;
        out.set_item(it.get_item("key")?, it)?;
    }
    memo.set_item(key, &out)?;
    Ok(out.into_any())
}

fn catalogue(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    lib(py)?.call_method0("_catalogue")
}

#[pyfunction]
fn _catalogue(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    let resources = py.import("built_core.paths")?.call_method0("app_root")?.div("resources")?;
    let language = py.import("built_core.i18n")?.call_method0("current_language")?;
    lib(py)?.call_method1("_bundled_by_key", (resources, language))
}

#[pyfunction]
#[pyo3(signature = (win, *, name="", recipe=None))]
fn _pick_paint(py: Python<'_>, win: &Bound<'_, PyAny>, name: &str, recipe: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    kw.set_item("recipe", recipe)?;
    if py.import("built_core")?.getattr("pick_library_paint")?.call((win.getattr("viewport")?.getattr("scene")?, paint_tool(py)?), Some(&kw))?.is_truthy()? {
        win.call_method1("_activate_tool", ("paint",))?;
    }
    Ok(())
}

#[pyfunction]
fn _pick_image<'py>(py: Python<'py>, win: &Bound<'py, PyAny>, title: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    let dialogs = py.import("built_render.shell_files")?.getattr("ShellFileDialog")?;
    let path = dialogs.call_method1("getOpenFileName", (win, title, "", "Images (*.png *.jpg *.jpeg *.webp *.bmp *.tif *.tiff)"))?.get_item(0)?;
    Ok(if path.is_truthy()? { Some(path) } else { None })
}

/// The provider of `type_id` (Materials when unknown).
#[pyfunction]
fn provider<'py>(py: Python<'py>, type_id: &str) -> PyResult<Bound<'py, PyAny>> {
    let all = lib(py)?.getattr("PROVIDERS")?;
    for p in all.try_iter()? {
        let p = p?;
        if p.getattr("id")?.eq(type_id)? {
            return Ok(p);
        }
    }
    all.get_item(0)
}

// ---- state -----------------------------------------------------------------------------

fn sorts(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    lib(py)?.getattr("SORTS")
}

/// The browsing state on the view (type, sort, facet, scope, folds, recent
/// picks, cursor), seeded from the preferences.
#[pyfunction]
fn _state<'py>(view: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = view.py();
    if let Some(st) = view.getattr("__dict__")?.cast::<PyDict>()?.get_item("_ui_lib")? {
        if !st.is_none() {
            return Ok(st);
        }
    }
    let saved = settings_json(py, "library/state", PyDict::new(py).into_any())?;
    let g = |k: &str, d: Bound<'py, PyAny>| saved.call_method1("get", (k, d));
    let builtins = py.import("builtins")?;
    let st = PyDict::new(py);
    st.set_item("type", g("type", s(py, "materials"))?)?;
    st.set_item("view", builtins.getattr("int")?.call1((g("view", obj(py, 0)?.into_bound(py))?,))?)?;
    st.set_item("sort", g("sort", s(py, "name"))?)?;
    st.set_item("filter", "")?;
    st.set_item("fold", PySet::new(py, g("fold", PyList::empty(py).into_any())?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?)?;
    st.set_item("open", PySet::new(py, g("open", PyList::empty(py).into_any())?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?)?;
    st.set_item("recent", settings_json(py, "library/recent", PyList::empty(py).into_any())?)?;
    st.set_item("mine_rev", 0)?;
    st.set_item("cursor", py.None())?;
    st.set_item("sig", py.None())?;
    st.set_item("items", PyDict::new(py))?;
    st.set_item("scope", g("scope", s(py, "s:model"))?)?;
    let sort_ids = builtins.getattr("dict")?.call1((sorts(py)?,))?;
    if !sort_ids.contains(st.get_item("sort")?.expect("set"))? {
        st.set_item("sort", "name")?;
    }
    st.set_item("facet", g("facet", s(py, "all"))?)?;
    let ty = st.get_item("type")?.expect("set");
    let mut known = false;
    for p in lib(py)?.getattr("PROVIDERS")?.try_iter()? {
        let p = p?;
        known |= p.getattr("ready")?.is_truthy()? && p.getattr("id")?.eq(&ty)?;
    }
    if !known {
        st.set_item("type", "materials")?; // "colours" was a type of its own before 2026-11-22
    }
    view.setattr("_ui_lib", &st)?;
    Ok(st.into_any())
}

#[pyfunction]
fn _save_state(st: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = st.py();
    let builtins_sorted = py.import("builtins")?.getattr("sorted")?;
    let sorted = |v: Bound<'_, PyAny>| -> PyResult<Py<PyAny>> { Ok(builtins_sorted.call1((v,))?.unbind()) };
    let d = PyDict::new(py);
    d.set_item("type", st.get_item("type")?)?;
    d.set_item("view", st.get_item("view")?)?;
    d.set_item("sort", st.get_item("sort")?)?;
    d.set_item("facet", st.call_method1("get", ("facet", "all"))?)?;
    d.set_item("scope", st.call_method1("get", ("scope", "s:model"))?)?;
    d.set_item("fold", sorted(st.get_item("fold")?)?)?;
    d.set_item("open", sorted(st.call_method1("get", ("open", PyTuple::empty(py)))?)?)?;
    store_json(py, "library/state", d.as_any())
}

/// The browsing state as `library_view.rs` reads it.
fn browse(st: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = st.py();
    let d = PyDict::new(py);
    d.set_item("sort", st.call_method1("get", ("sort", "name"))?)?;
    d.set_item("facet", st.call_method1("get", ("facet", "all"))?)?;
    d.set_item("group", st.call_method1("get",("component_group","none"))?)?;
    d.set_item("filter", st.call_method1("get", ("filter", ""))?)?;
    let scope = st.call_method1("get", ("scope",))?;
    d.set_item("scope", if scope.is_truthy()? { scope } else { s(py, "s:model") })?;
    d.set_item("fold", py.import("builtins")?.getattr("sorted")?.call1((st.call_method1("get", ("fold", PyTuple::empty(py)))?,))?)?;
    d.set_item("recent", py.import("builtins")?.getattr("list")?.call1((st.call_method1("get", ("recent", PyTuple::empty(py)))?,))?)?;
    d.set_item("cursor", st.call_method1("get", ("cursor",))?)?;
    json(py)?.call_method1("dumps", (d,))?.extract()
}

fn by_key<'py>(items: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(items.py());
    for e in items.try_iter()? {
        let e = e?;
        d.set_item(e.get_item("key")?, e)?;
    }
    Ok(d)
}

/// The items the facet and the search keep.
#[pyfunction]
fn filtered<'py>(items: &Bound<'py, PyAny>, st: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let py = items.py();
    let index = by_key(items)?;
    let keys = render(py)?.call_method1("library_filtered", (dumps(py, items)?, browse(st)?))?;
    PyList::new(py, keys.try_iter()?.map(|k| index.get_item(k?).map(|v| v.expect("listed"))).collect::<PyResult<Vec<_>>>()?)
}

/// The side list: (id, label, count, depth, kind, fold) per row.
#[pyfunction]
fn side_rows<'py>(items: &Bound<'py, PyAny>, st: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = items.py();
    render(py)?.call_method1("library_side", (dumps(py, items)?, browse(st)?))
}

/// The content: [(caption, sub, items)].
#[pyfunction]
fn groups_for<'py>(items: &Bound<'py, PyAny>, st: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let py = items.py();
    let index = by_key(items)?;
    let out = PyList::empty(py);
    for g in render(py)?.call_method1("library_groups", (dumps(py, items)?, browse(st)?))?.try_iter()? {
        let g = g?;
        let picked = PyList::empty(py);
        for k in g.get_item(2)?.try_iter()? {
            picked.append(index.get_item(k?)?.expect("listed"))?;
        }
        out.append((g.get_item(0)?, g.get_item(1)?, picked))?;
    }
    Ok(out)
}

/// A texture or thumbnail as a chrome icon, sharp on a Retina tile.
fn thumb(view: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = view.py();
    if !path.is_truthy()? {
        return Ok(String::new());
    }
    let key = format!("lib:{}", path.str()?);
    let sent = view.getattr("__dict__")?.call_method1("setdefault", ("_ui_tex_icons", PySet::empty(py)?))?;
    if !sent.contains(&key)? {
        sent.call_method1("add", (&key,))?;
        let px = THUMB_PX * 2;
        let name=path.str()?;
        let rgba=if let Some(profile)=name.to_str()?.strip_prefix("profile:"){
            py.import("built_core.command_protocol")?.call_method1("sweep_thumbnail",(profile,px))?
        }else if name.to_str()?.starts_with("junction:"){
            py.import("built_core.command_protocol")?.call_method1("junction_thumbnail",(name.to_str()?,px))?
        }else{render(py)?.call_method1("swatch_pixels", (path, px, true))?};
        if !rgba.is_none() {
            view.getattr("renderer")?.call_method1("ui_set_icons", (vec![(&key, px, px, rgba)],))?;
        }
    }
    Ok(key)
}

#[pyfunction]
fn item_of<'py>(view: &Bound<'py, PyAny>, key: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    _state(view)?.get_item("items")?.call_method1("get", (key,))
}

// ---- the sheet --------------------------------------------------------------------------

fn component_blocks<'py>(py: Python<'py>, item: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let item = serde_json::from_str(&dumps(py, item)?).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    let input = crate::library_view::Input { kind: "components".into(), ..Default::default() };
    let blocks = crate::library_view::detail(&input, &item, &mut Vec::new());
    loads(py, &s(py, &serde_json::Value::Array(blocks).to_string()))
}

#[pyclass]
struct ComponentItems(std::sync::Arc<[crate::library_view::Item]>);

fn component_parts<'py>(st: &Bound<'py, PyAny>, items: &Bound<'py, PyAny>, browse: &Bound<'py, PyDict>)
    -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)>
{
    let py = st.py();
    // _items replaces this tuple when document data or the available actions change.
    let generation = st.get_item("component_selection_rows")?;
    let snapshot = st.call_method1("get", ("native_component_items",))?;
    let catalogue = if !snapshot.is_none() && snapshot.get_item(0)?.is(&generation) {
        snapshot.get_item(1)?
    } else {
        let entries = serde_json::from_str(&dumps(py, items)?)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let catalogue = Bound::new(py, ComponentItems(entries))?.into_any();
        st.set_item("native_component_items", (&generation, &catalogue))?;
        catalogue
    };
    let filters = browse.copy()?;
    filters.del_item("cursor")?;
    let cached = st.call_method1("get", ("native_components",))?;
    if !cached.is_none() && cached.get_item(0)?.is(&catalogue) && cached.get_item(1)?.eq(&filters)? {
        let parts = cached.get_item(2)?.cast_into::<PyDict>()?.copy()?;
        let rows = cached.get_item(3)?;
        if let Some(cursor) = st.get_item("cursor")?.extract::<Option<String>>()? {
            if rows.cast::<LibraryRows>()?.borrow().groups.iter().flat_map(|group| group.iter()).any(|it| it.id == cursor) {
                parts.set_item("cursor", &cursor)?;
                parts.set_item("detail", component_blocks(py, &st.get_item("items")?.get_item(&cursor)?)?)?;
            }
        }
        return Ok((parts.into_any(), rows));
    }
    let input = crate::library_view::Input {
        kind: "components".into(), items: catalogue.cast::<ComponentItems>()?.borrow().0.clone(),
        st: serde_json::from_str(&dumps(py, browse.as_any())?).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?,
        ..Default::default()
    };
    let mut groups = Vec::new();
    let parts = crate::library_view::sheet_rows(&input, Some(&mut groups));
    let parts = loads(py, &s(py, &parts.to_string()))?;
    let rows = Bound::new(py, LibraryRows { groups })?.into_any();
    st.set_item("native_components", (&catalogue, &filters, &parts, &rows))?;
    Ok((parts, rows))
}

/// Native row storage shared between the view cache and the live renderer.
#[pyclass]
pub(crate) struct LibraryRows {
    pub groups: crate::library_view::RowGroups,
}

/// The Library on the large sheet: types as tabs, scopes and categories on
/// the side (linked files' materials read-only), 96-pt tiles, the focused
/// item's details and model usage on the right. With a paint target it is
/// the tree's "Choose material" sheet.
#[pyclass(module = "built_render.wgpu_library", extends = crate::sheet_runtime::Provider, dict, get_all, set_all)]
pub struct LibrarySheet {
    view: Py<PyAny>,
    paint_target: Py<PyAny>,
    catalogue: Py<PyAny>,
}

fn cs(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    render(py)?.getattr("chrome_sheet")
}

pub(crate) fn start_collection(win: &Bound<'_, PyAny>, folder: std::path::PathBuf, name: Option<String>) -> PyResult<()> {
    let py = win.py();
    collections::import_collection(folder, name)?;
    let job = py.import("built_render.chrome_dialogs")?.call_method1("Job", (win, "Import collection"))?;
    win.setattr("_family_import_job", job)?;
    lib(py)?.call_method1("open_library", (win, "families", py.None(), "s:all"))?;
    Ok(())
}

pub(crate) fn collection_form(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let forms = py.import("built_render.chrome_forms")?;
    let field = forms.getattr("Field")?;
    let folder = field.call(("folder", "path", "Collection folder", ""), Some(&[("pick", "folder")].into_py_dict(py)?))?;
    let name = field.call1(("name", "edit", "Collection name", ""))?;
    let change = func(py, |py, args| {
        let form = args.get_item(0)?;
        if args.get_item(1)?.eq("folder")? && !form.call_method1("value", ("name",))?.is_truthy()? {
            let path: std::path::PathBuf = args.get_item(2)?.extract()?;
            form.call_method1("field", ("name",))?.setattr("value", path.file_name().unwrap_or_default().to_string_lossy().as_ref())?;
        }
        Ok(none(py))
    })?;
    let validate = func(py, |py, args| {
        let folder: std::path::PathBuf = args.get_item(0)?.call_method1("value", ("folder",))?.extract()?;
        Ok(s(py, if collections::active() { "A collection is already importing." } else if !folder.is_dir() { "Choose a collection folder." } else { "" }).unbind())
    })?;
    let w = win.clone().unbind();
    let done = func(py, move |py, args| {
        if args.get_item(0)?.eq("ok")? {
            let values = args.get_item(1)?;
            start_collection(w.bind(py), values.get_item("folder")?.extract()?, values.get_item("name")?.extract()?)?;
        }
        Ok(none(py))
    })?;
    let kw = PyDict::new(py);
    kw.set_item("buttons", [("cancel","Cancel"),("ok","Import")])?;
    kw.set_item("note", "Import family types from .rfa files and their type catalogs.")?;
    kw.set_item("on_change", change)?;
    kw.set_item("on_done", done)?;
    kw.set_item("validate", validate)?;
    let form = forms.getattr("Form")?.call(("Import collection", PyList::new(py, [folder,name])?), Some(&kw))?;
    form.setattr("placement", "drawer")?;
    forms.call_method1("show", (win, form))?;
    Ok(())
}

fn cs_verb<'py>(py: Python<'py>, id: &str, label: &str, kw: &[(&str, Bound<'py, PyAny>)]) -> PyResult<Bound<'py, PyAny>> {
    let d = PyDict::new(py);
    for (k, v) in kw {
        d.set_item(*k, v)?;
    }
    cs(py)?.getattr("verb")?.call((id, label), Some(&d))
}

#[pymethods]
impl LibrarySheet {
    #[new]
    #[pyo3(signature = (view, paint_target=None))]
    fn new(py: Python<'_>, view: Py<PyAny>, paint_target: Option<Py<PyAny>>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(crate::sheet_runtime::Provider).add_subclass(LibrarySheet {
            view,
            paint_target: paint_target.unwrap_or_else(|| py.None()),
            catalogue: py.None(),
        })
    }

    #[getter]
    fn id(&self, py: Python<'_>) -> &'static str {
        if self.paint_target.is_none(py) { "library" } else { "tree-material" }
    }

    fn signature<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
        let scene = ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?;
        let st = _state(self.view.bind(py))?;
        let recent: Vec<Bound<'_, PyAny>> = st.get_item("recent")?.try_iter()?.take(12).collect::<PyResult<_>>()?;
        let material_tab = st.get_item("type")?.eq("materials")?;
        let usage_ready = material_tab && !sheet_usage(&scene)?.is_none();
        if material_tab && !usage_ready && !scene.getattr_opt("_library_usage_failure")?
            .is_some_and(|v| material_revision(&scene).and_then(|r| v.eq(r)).unwrap_or(false)) {
            if let Some(host) = ctx.getattr("win")?.getattr_opt("_host")? {
                host.call_method1("wake_after", (32,))?;
            }
        }
        PyTuple::new(py, [
            obj(py, scene.as_ptr() as usize)?.into_bound(py),
            scene.getattr("content_version")?,
            crate::document_registration::view::calls::current_material_name(py)?,
            st.get_item("mine_rev")?,
            obj(py, collections::REVISION.load(std::sync::atomic::Ordering::Relaxed))?.into_bound(py),
            if st.get_item("type")?.eq("components")? { scene.getattr("version")? } else { py.None().into_bound(py) },
            if material_tab { material_revision(&scene)? } else { py.None().into_bound(py) },
            obj(py, usage_ready)?.into_bound(py),
            PyTuple::new(py, recent)?.into_any(),
        ])
    }

    fn _items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyDict>)> {
        let view = self.view.bind(py);
        let st = _state(view)?;
        let lctx = ctx_of(py, &ctx.getattr("win")?, view)?;
        let scene = lctx.getattr("scene")?;
        let tab: String = st.get_item("type")?.extract()?;
        let components = tab == "components";
        let usage = if tab == "materials" { sheet_usage(&scene)? } else { py.None().into_bound(py) };
        lctx.setattr("usage", &usage)?;
        let cache_key = format!("provider_{tab}");

        let revision_key = (scene.as_ptr() as usize, scene.getattr("content_version")?, st.get_item("mine_rev")?,
            collections::REVISION.load(std::sync::atomic::Ordering::Relaxed)).into_pyobject(py)?;
        let generation = (&revision_key, !usage.is_none(), if tab == "materials" { material_revision(&scene)? } else { py.None().into_bound(py) });
        let revision = scene.getattr("version")?;
        // A selection edit only changes which component rows offer Replace.
        if components {
            let cache = st.call_method1("get", ("component_selection_rows",))?;
            if !cache.is_none() && cache.get_item(0)?.eq(&generation)? {
                if cache.get_item(1)?.eq(&revision)? {
                    st.set_item("items", cache.get_item(4)?)?;
                    return Ok((cache.get_item(2)?, cache.get_item(3)?.cast_into::<PyDict>()?));
                }
                let items = cache.get_item(2)?;
                let selected = selected_definitions(&scene)?;
                for item in items.try_iter()? {
                    let item = item?;
                    let Some(mesh) = item.cast::<PyDict>()?.get_item("definition_id")? else { continue };
                    let id: usize = mesh.extract()?;
                    let verbs = item.get_item("verbs")?;
                    let has = verbs.contains("replace")?;
                    let allowed = !item.get_item("source")?.eq("linked")? && selected.iter().any(|&m| m != id);
                    if has != allowed {
                        let mut actions: Vec<String> = verbs.extract()?;
                        actions.retain(|v|v != "replace");
                        if allowed { actions.insert(1, "replace".into()); }
                        item.set_item("verbs", PyTuple::new(py, actions)?)?;
                    }
                }
                let counts = cache.get_item(3)?.cast_into::<PyDict>()?;
                st.set_item("component_selection_rows", (&generation, &revision, &items, &counts, cache.get_item(4)?))?;
                st.set_item("items", cache.get_item(4)?)?;
                return Ok((items,counts));
            }
        }
        let cached = st.call_method1("get", (&cache_key,))?;
        if !components && !cached.is_none() && cached.get_item(0)?.eq(&generation)? {
            st.set_item("items", cached.get_item(3)?)?;
            return Ok((cached.get_item(1)?, cached.get_item(2)?.cast_into::<PyDict>()?));
        }
        let active = provider(py, &tab)?.call_method1("items", (&lctx,))?;
        let mut n = 0;
        for entry in active.try_iter()? {
            let src = entry?.get_item("source")?;
            if tab == "families" || src.eq("model")? || src.eq("linked")? { n += 1; }
        }
        let badges = st.call_method1("get", ("tab_counts",))?;
        let counts = if !badges.is_none() && badges.get_item(0)?.eq(&revision_key)? {
            badges.get_item(1)?.cast_into::<PyDict>()?
        } else {
            let counts = PyDict::new(py);
            st.set_item("tab_counts", (&revision_key, &counts))?;
            counts
        };
        counts.set_item(&tab, n)?;
        let indexed = by_key(&active)?;
        if !components { st.set_item(cache_key, (&generation, &active, &counts, &indexed))?; }
        if components { st.set_item("component_selection_rows", (generation, revision, &active, &counts, &indexed))?; }
        st.set_item("items", indexed)?;
        Ok((active, counts))
    }

    fn build<'py>(slf: &Bound<'py, Self>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let me = slf.borrow();
        let view = me.view.bind(py).clone();
        let st = _state(&view)?;
        let prov = provider(py, &st.get_item("type")?.str()?.to_string())?;
        let prov_id: String = prov.getattr("id")?.extract()?;
        let (mut items, counts) = me._items(py, ctx)?;
        let target = me.paint_target.bind(py).clone();
        drop(me);
        let has_target = !target.is_none();
        let sid: String = slf.getattr("id")?.extract()?;
        let sheet = cs(py)?.getattr("Sheet")?.call((sid, if has_target { "Choose material" } else { "Library" }), Some(&[("width", 1240)].into_py_dict(py)?))?;
        let ty = st.get_item("type")?;
        if !has_target {
            for p in lib(py)?.getattr("PROVIDERS")?.try_iter()? {
                let p = p?;
                let ready = p.getattr("ready")?.is_truthy()?;
                if !ready { continue; }
                let count = if p.getattr("id")?.eq("materials")? { s(py, "") } else { counts.call_method1("get", (p.getattr("id")?, ""))? };
                let kw = PyDict::new(py);
                kw.set_item("on", p.getattr("id")?.eq(&ty)?)?;
                kw.set_item("enabled", ready)?;
                sheet.call_method("tab", (p.getattr("id")?, p.getattr("label")?, count), Some(&kw))?;
            }
        }
        let scene = ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?;
        let catalogue_available = prov_id == "components" && py.import("built_core")?
            .getattr("component_library")?.call_method0("available")?.is_truthy()?;
        if !catalogue_available && st.get_item("scope")?.eq("s:online")? {
            st.set_item("scope", "s:model")?;
        }
        let browse_d = json(py)?.call_method1("loads", (browse(&st)?,))?;
        if prov_id == "components" && !st.get_item("scope")?.eq("s:online")? {
            browse_d.set_item("scope", "s:all")?;
            if let Some(errors) = scene.getattr_opt("link_errors")?.filter(|e| !e.is_none()) {
                browse_d.set_item("link_errors", errors)?;
            }
        }
        if has_target {
            items = PyList::new(py, items.try_iter()?.map(|e| e?.cast_into::<PyDict>()?.copy()).collect::<PyResult<Vec<_>>>()?)?.into_any();
            target.call_method1("prepare", (&items, &browse_d))?;
            st.set_item("items", by_key(&items)?)?;
        }
        let current = if has_target { target.getattr("current")? } else { crate::document_registration::view::calls::current_material_name(py)? };
        let materials = prov_id == "materials";
        let spec = PyDict::new(py);
        spec.set_item("type", &prov_id)?;
        spec.set_item("items", &items)?;
        spec.set_item("st", &browse_d)?;
        spec.set_item("current", if materials { current } else { py.None().into_bound(py) })?;
        let usage = if materials { sheet_usage(&scene)? } else { py.None().into_bound(py) };
        spec.set_item("usage_pending", materials && usage.is_none())?;
        spec.set_item("model_uses", if usage.is_none() { PyDict::new(py).into_any() } else { usage.get_item(0)? })?;
        spec.set_item("unnamed", if usage.is_none() { 0 } else { usage.get_item(1)?.len()? })?;
        let native = view.getattr("renderer")?.is_instance_of::<crate::Renderer>();
        let native_components = native && prov_id == "components" && !has_target && !st.get_item("scope")?.eq("s:online")?;
        let mut native_rows = None;
        let out = if native_components {
            let (parts, rows) = component_parts(&st, &items, browse_d.cast::<PyDict>()?)?;
            native_rows = Some(rows);
            parts
        } else if native && !has_target {
            let key = format!("{}_presentation", prov_id);
            let cached = st.call_method1("get", (&key,))?;
            let selector = (&browse_d, spec.get_item("current")?, spec.get_item("usage_pending")?);
            if !cached.is_none() && cached.get_item(0)?.is(&items) && cached.get_item(1)?.eq(&selector)? {
                native_rows = Some(cached.get_item(3)?);
                cached.get_item(2)?
            } else {
                let input = serde_json::from_str(&dumps(py, spec.as_any())?)
                    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
                let mut groups = Vec::new();
                let parts = crate::library_view::sheet_rows(&input, Some(&mut groups));
                let parts = loads(py, &s(py, &parts.to_string()))?;
                let rows = Bound::new(py, LibraryRows { groups })?.into_any();
                st.set_item(key, (&items, selector, &parts, &rows))?;
                native_rows = Some(rows);
                parts
            }
        } else {
            loads(py, &render(py)?.call_method1("library_sheet", (dumps(py, spec.as_any())?,))?)?
        };
        if !native || matches!(prov_id.as_str(),"profiles"|"presets"|"sections") {
            if prov_id != "families" {
                for path in out.get_item("thumbs")?.try_iter()? { thumb(&view, &path?)?; }
            } else if let Some(key) = out.get_item("cursor")?.extract::<Option<String>>()? {
                let focused = st.get_item("items")?.get_item(key)?;
                thumb(&view, &focused.get_item("thumb")?)?;
            }
        }
        if let Some(rows) = &native_rows { sheet.setattr("_library_rows", rows)?; }
        st.set_item("scope", out.get_item("scope")?)?;
        let searching = out.get_item("searching")?.is_truthy()?;
        let online = out.get_item("online")?.is_truthy()?;
        sheet.call_method1("search", (out.get_item("search")?,))?;
        sheet.getattr("menus")?.call_method1("append", (out.get_item("sort")?,))?;
        if prov_id == "materials" || prov_id == "presets" {
            sheet.getattr("menus")?.call_method1("append", (out.get_item("show")?,))?;
        }
        let faceted=prov_id=="components" && !online;
        if faceted {
            st.set_item("component_facets", out.get_item("side")?)?;
            let group:String=st.call_method1("get",("component_group","none"))?.extract()?;
            let options=[("none","None"),("class","IFC class"),("source","Source")];
            let label=options.iter().find(|(id,_)|*id==group).map_or("None",|p|p.1);
            sheet.call_method1("menu",("group",format!("Group: {label}"),options.to_vec(),group))?;
        }
        sheet.setattr("side", out.get_item("side")?.call_method0("copy")?)?;
        let manifests = if prov_id == "families" { view.getattr_opt("_library_collections")? } else { None };
        let manifests = manifests.as_ref().map(|v| v.extract::<PyRef<FamilyCollections>>()).transpose()?;
        let collections = manifests.as_ref().map_or(&[][..], |m| m.0.as_slice());
        if prov_id=="presets" {
            sheet.call_method1("verb",("typeact:import","Import pack…"))?;
        }else if prov_id=="profiles" {
            sheet.call_method1("verb",("typeact:pack","Import pack…"))?;
        }else if prov_id=="sections" {
            sheet.call_method1("verb",("typeact:new","New section…"))?;
        }else if !has_target {
            sheet.call_method1("verb", ("import-collection", "Import collection…"))?;
        }
        if prov_id == "families" {
            for collection in collections {
                let scope = format!("s:{}", collection["source"].as_str().unwrap_or(""));
                sheet.call_method("side_item", (&scope, collection["name"].as_str().unwrap_or("")), Some(&[("on", st.get_item("scope")?.eq(&scope)?)].into_py_dict(py)?))?;
            }
        }
        if catalogue_available {
            let label = py.import("built_core.i18n")?.call_method1("tr", ("Component library",))?;
            sheet.call_method("side_item", ("s:online", label), Some(&[("on", online)].into_py_dict(py)?))?;
        }
        if online {
            if slf.getattr("catalogue")?.is_none() {
                slf.setattr("catalogue", crate::component_catalogue::session::CatalogueSession::create(ctx)?)?;
            }
            if !searching {
                return slf.getattr("catalogue")?.call_method1("build", (&sheet, ""));
            }
        } else {
            slf.call_method0("close_catalogue")?;
        }
        let content_spec = out.get_item("content")?;
        if let Some(c) = collections.iter().find(|c| st.get_item("scope").is_ok_and(|v|
            v.eq(format!("s:{}", c["source"].as_str().unwrap_or(""))).unwrap_or(false))) {
            if !searching { content_spec.set_item("title", c["name"].as_str().unwrap_or(""))?; }
        }
        let content = sheet.call_method("content", (), Some(content_spec.cast::<PyDict>()?))?;
        if faceted {
            bulk::controls(&st,&content,&sheet,&out,&ctx_of(py,&ctx.getattr("win")?,&view)?)?;
            let mut buttons=Vec::new();
            for token in out.get_item("tokens")?.extract::<Vec<String>>()? {
                buttons.push(cs_verb(py,&format!("toggle:{token}"),&format!("{token} ×"),&[])?);
            }
            if !buttons.is_empty() {
                buttons.push(cs_verb(py,"reset-filter","Reset",&[])?);
                content.call_method1("buttons",(buttons,))?;
            }
        }
        if prov_id == "families" {
            let status = collections::status();
            if !status.is_empty() { content.call_method1("caption", (status,))?; }
        }
        for (index, g) in out.get_item("groups")?.try_iter()?.enumerate() {
            let g = g?;
            if g.get_item("caption")?.is_truthy()? {
                content.call_method("caption", (g.get_item("caption")?,), Some(&[("right", g.get_item("right")?)].into_py_dict(py)?))?;
            }
            let items = g.get_item("tiles")?;
            if has_target {
                content.call_method("tiles", (items,), Some(&[("size", 96)].into_py_dict(py)?))?;
            } else {
                let kwargs = [("thumbs", true)].into_py_dict(py)?;
                if native_rows.is_some() { kwargs.set_item("id", format!("library-group:{index}"))?; }
                content.call_method("rows", (items,), Some(&kwargs))?;
            }
        }
        for collection in collections {
            let scope = format!("s:{}", collection["source"].as_str().unwrap_or(""));
            if st.get_item("scope")?.eq("s:all")? || st.get_item("scope")?.eq(scope)? {
                for report in collection["reports"].as_array().into_iter().flatten() {
                    content.call_method1("text", (report.as_str().unwrap_or(""),))?;
                }
            }
        }
        if !has_target && out.get_item("type_actions")?.is_truthy()? {
            let lctx = ctx_of(py, &ctx.getattr("win")?, &view)?;
            let mut acts = Vec::new();
            for a in prov.call_method1("type_actions", (lctx,))?.try_iter()? {
                let a = a?;
                acts.push(cs_verb(py, &format!("typeact:{}", a.get_item(0)?.str()?), &a.get_item(1)?.str()?.to_string(), &[("quiet", true.into_pyobject(py)?.to_owned().into_any())])?);
            }
            if !acts.is_empty() {
                content.call_method("buttons", (acts,), Some(&[("align", "left")].into_py_dict(py)?))?;
            }
        }
        // A search that hides the cursor moves it to the first row shown;
        // the sheet's focus follows so Enter uses the row on screen.
        let previous = st.call_method1("get", ("cursor",))?;
        let moved = previous.is_truthy()? && !previous.eq(out.get_item("cursor")?)?;
        st.set_item("cursor", out.get_item("cursor")?)?;
        let detail = sheet.call_method("detail", (), Some(&[("width", (if faceted {260i32} else {340}).into_pyobject(py)?.into_any()), ("empty", s(py, "Pick an item to see it here."))].into_py_dict(py)?))?;
        let cursor = st.call_method1("get", ("cursor",))?;
        if has_target {
            detail.call_method1("fields", (&target, "tree-paint"))?;
            detail.call_method1("buttons", (vec![cs_verb(py, "custompaint", "Apply colour", &[])?, cs_verb(py, "activepaint", "Use active paint", &[])?, cs_verb(py, "clearpaint", "Remove paint", &[])?],))?;
            if cursor.is_truthy()? {
                let apply = cs_verb(py, "apply", "Apply material", &[("primary", true.into_pyobject(py)?.to_owned().into_any())])?;
                detail.call_method("buttons", (vec![apply],), Some(&[("align", "fill")].into_py_dict(py)?))?;
            }
        } else {
            detail.setattr("blocks", out.get_item("detail")?.call_method0("copy")?)?;
        }
        let e = if cursor.is_truthy()? { st.get_item("items")?.call_method1("get", (&cursor,))? } else { py.None().into_bound(py) };
        if faceted && st.get_item("checked")?.len()?>0 {
            detail.setattr("blocks",PyList::empty(py))?;
            bulk::detail(&st,&detail)?;
        } else if !e.is_none() && prov_id == "components" {
            slf.call_method1("_component_detail", (&detail, &prov, ctx, &e))?;
        }
        let hint = if prov_id == "profiles" && !st.get_item("items")?.call_method0("values")?.try_iter()?.any(|r| r.is_ok_and(|r| r.contains("profile_preset").unwrap_or(false))) { "More profiles and details come as packs: Import pack…" } else if matches!(prov_id.as_str(), "components" | "families") { "Click or drag to place · … for actions · Esc back to the model" } else { "Click to use · … for actions · Esc back to the model" };
        let links = PyList::empty(py);
        if prov_id == "profiles" {
            for (id, label) in [("typeact:new", "New profile…"), ("typeact:import", "Import drawing…")] {
                links.append((id, crate::settings_rows::tr(py, label)?))?;
            }
        }
        sheet.call_method1("footer", (links, crate::settings_rows::tr(py, hint)?))?;
        if cursor.is_truthy()? {
            let force = st.call_method1("pop", ("force_focus", false))?.is_truthy()? || moved;
            sheet.call_method("focus", (&cursor,), Some(&[("force", force)].into_py_dict(py)?))?;
        }
        let catalogue = slf.getattr("catalogue")?;
        if catalogue.is_none() {
            Ok(sheet)
        } else {
            let result = catalogue.call_method1("build", (&sheet, st.get_item("filter")?))?;
            if let Some(rows) = native_rows { result.setattr("_library_rows", rows)?; }
            Ok(result)
        }
    }

    fn focus_component(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, sheet: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
        let py = slf.py();
        let view = slf.getattr("view")?;
        let st = _state(&view)?;
        let Some(item) = st.get_item("items")?.cast::<PyDict>()?.get_item(key)? else { return Ok(()) };
        let catalogue = slf.getattr("catalogue")?;
        if !catalogue.is_none() { catalogue.call_method1("act", ("focus", vec![key]))?; }
        st.set_item("cursor", key)?;
        let detail = sheet.call_method("detail", (), Some(&[("width", 260)].into_py_dict(py)?))?;
        if st.get_item("checked")?.len()?>0 {
            bulk::detail(&st,&detail)?;
        } else {
            detail.setattr("blocks", component_blocks(py, &item)?)?;
            slf.call_method1("_component_detail", (&detail, provider(py, "components")?, ctx, &item))?;
        }
        sheet.call_method1("focus", (key,))?;
        let pane = detail.call_method0("wire")?;
        if sheet.is_instance_of::<crate::sheet_runtime::Spec>() {
            sheet.getattr("spec")?.set_item("detail", &pane)?;
        }
        let pane = serde_json::from_str(&dumps(py, &pane)?)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        if let Some(ui) = &mut view.getattr("renderer")?.cast::<crate::Renderer>()?.borrow_mut().ui {
            ui.update_library_detail(pane);
        }
        Ok(())
    }

    fn _component_detail(slf: &Bound<'_, Self>, d: &Bound<'_, PyAny>, prov: &Bound<'_, PyAny>, ctx: &Bound<'_, PyAny>, e: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let lctx = ctx_of(py, &ctx.getattr("win")?, slf.getattr("view")?.as_any())?;
        let got = prov.call_method1("inspector", (&lctx, e))?;
        if !got.is_none() {
            for section in got.get_item(1)?.try_iter()? {
                let section = section?;
                if section.get_item(0)?.eq("Actions")? {
                    continue;
                }
                d.call_method1("kv", (section.get_item(1)?,))?;
            }
        }
        let key: String = e.get_item("key")?.extract()?;
        let model = key.starts_with("cm:");
        if model {
            let row = prov.call_method1("_row", (&lctx, e))?;
            if row.is_truthy()? {
                let group = if !row.get_item(3)?.is_none() { row.get_item(3)? } else { row.get_item(2)?.try_iter()?.next().transpose()?.unwrap_or_else(|| py.None().into_bound(py)) };
                let ext = group.getattr("ext").ok().filter(|x| x.is_truthy().unwrap_or(false));
                if let Some(record) = ext.map(|x| get(&x, "catalogue")).transpose()?.filter(|r| r.is_truthy().unwrap_or(false)) {
                    let i18n = py.import("built_core.i18n")?;
                    let line = render(py)?.getattr("ComponentCatalogue")?.call_method1("credit_line", (json(py)?.call_method1("dumps", (record,))?, i18n.call_method0("current_language")?, i18n.getattr("tr")?))?;
                    d.call_method1("text", (line,))?;
                }
            }
        }
        let primary = cs_verb(py, "apply", "Place a copy", &[("key", s(py, "↩")), ("primary", true.into_pyobject(py)?.to_owned().into_any())])?;
        d.call_method("buttons", (vec![primary],), Some(&[("align", "fill")].into_py_dict(py)?))?;
        if model {
            if let Some(locations)=e.cast::<PyDict>()?.get_item("locations")? {
                d.call_method1("caption",("Copies by location",))?;
                d.call_method1("kv",(locations.call_method0("items")?,))?;
            }
        }
        Ok(())
    }

    fn _rename(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, e: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let view = slf.getattr("view")?;
        let prov = provider(py, &_state(&view)?.get_item("type")?.str()?.to_string())?;
        let materials = prov.getattr("id")?.eq("materials")?;
        let lctx = ctx_of(py, &ctx.getattr("win")?, &view)?;
        let title = if materials { "Rename material" } else { "Rename component" };
        let (prov_r, lctx_r, e_r, view_r, win_r) = (prov.clone().unbind(), lctx.clone().unbind(), e.clone().unbind(), view.clone().unbind(), ctx.getattr("win")?.unbind());
        let done = func(py, move |py, a| {
            let text = a.get_item(0)?;
            let text = if text.is_none() { String::new() } else { text.str()?.to_string() };
            let t = text.trim();
            let e = e_r.bind(py);
            if !t.is_empty() && !e.get_item("name")?.eq(t)? {
                if let Err(err) = prov_r.bind(py).call_method1("run", (lctx_r.bind(py), "rename", e, t)) {
                    if err.is_instance_of::<pyo3::exceptions::PyKeyError>(py) || err.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                        let msg = err.value(py).str()?.to_string();
                        flash(lctx_r.bind(py), msg.trim_matches(|c| c == '\'' || c == '"'))?;
                    } else {
                        return Err(err);
                    }
                }
                if materials {
                    let st = _state(view_r.bind(py))?;
                    st.set_item("cursor", format!("m:{t}"))?;
                    st.set_item("force_focus", true)?;
                }
            }
            lib(py)?.call_method1("refresh", (win_r.bind(py),))?;
            Ok(none(py))
        })?;
        let kw = PyDict::new(py);
        kw.set_item("text", e.get_item("name")?)?;
        kw.set_item("done", done)?;
        py.import("built_render.chrome_forms")?.getattr("ask_text")?.call((ctx.getattr("win")?, title, "Name:"), Some(&kw))?;
        Ok(())
    }

    fn act(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, verb: &str, args: Vec<String>) -> PyResult<()> {
        let py = slf.py();
        let view = slf.getattr("view")?;
        let win = ctx.getattr("win")?;
        let st = _state(&view)?;
        let arg = args.first().cloned().unwrap_or_default();
        let lctx = ctx_of(py, &win, &view)?;
        if st.get_item("type")?.eq("components")? && (verb.starts_with("bulk-") || verb=="verb" && arg.starts_with("bulk-")) {
            return bulk::act(&st,&lctx,if verb=="verb"{&arg}else{verb},&arg);
        }
        if (verb=="verb" && arg=="unique-choose") || (verb=="menu" && args.get(1).is_some_and(|v| v=="unique-choose")) {
            let cursor=if verb=="menu" {s(py,&arg)} else {st.get_item("cursor")?};
            let entry=st.get_item("items")?.get_item(&cursor)?;
            let p=provider(py,"components")?;
            let row=p.call_method1("_row",(&lctx,&entry))?;
            let mut choices=Vec::new();
            for group in row.get_item(2)?.try_iter()? {
                let group=group?;
                if py.import("built_core")?.call_method1("link_linked_ancestor",(&group,))?.is_none() && !lctx.getattr("scene")?.call_method1("entity_locked",(&group,))?.is_truthy()? {
                    let uid=group.getattr("uid")?.str()?.to_string();
                    choices.push(format!("{} · {uid}",group.getattr("name")?));
                }
            }
            let stamp=lctx.getattr("scene")?.getattr("content_version")?.unbind();
            let (p,entry,lctx)=(p.unbind(),entry.unbind(),lctx.unbind());
            let done=func(py,move |py,a| {
                let answer=a.get_item(0)?;
                if !answer.is_none() && lctx.bind(py).getattr("scene")?.getattr("content_version")?.eq(stamp.bind(py))? {
                    if let Some((_,uid))=answer.extract::<String>()?.rsplit_once(" · ") {
                        p.bind(py).call_method1("run",(lctx.bind(py),"unique",entry.bind(py),uid))?;
                    }
                }
                Ok(none(py))
            })?;
            py.import("built_render.chrome_forms")?.getattr("ask_choice")?.call((&win,"Make unique","Choose one copy",choices),Some(&[("done",done)].into_py_dict(py)?))?;
            return Ok(());
        }
        if verb == "family-previews" {
            for key in &args {
                if let Some(entry) = st.get_item("items")?.cast::<PyDict>()?.get_item(key)? {
                    thumb(&view, &entry.get_item("thumb")?)?;
                }
            }
            return Ok(());
        }
        if verb == "component-previews" {
            return crate::model_panels::component_preview::upload(&lctx, &args);
        }
        let target = slf.getattr("paint_target")?;
        if !target.is_none() {
            if verb == "tab" {
                return Ok(());
            }
            if verb == "verb" && matches!(arg.as_str(), "custompaint" | "activepaint" | "clearpaint") {
                target.call_method1("apply", (win.getattr("viewport")?, &arg))?;
                cs(py)?.call_method1("close", (&win,))?;
                return Ok(());
            }
        }
        let catalogue = slf.getattr("catalogue")?;
        if !catalogue.is_none() {
            if verb == "tab" || (verb == "side" && arg != "s:online" && !arg.starts_with("online-cat:")) {
                slf.call_method0("close_catalogue")?;
            } else if verb == "side" {
                st.set_item("scope", "s:online")?;
                catalogue.call_method1("act", (verb, args.clone()))?;
                _save_state(&st)?;
                return Ok(());
            } else if catalogue.call_method1("act", (verb, args.clone()))?.is_truthy()? {
                return Ok(());
            }
        }
        if st.get_item("type")?.eq("components")? {
            let filter: String = st.get_item("filter")?.extract()?;
            let next_query = if let Some(path) = arg.strip_prefix("include-file:").filter(|_| verb == "side") {
                let mut files = Vec::new();
                for facet in st.get_item("component_facets")?.try_iter()? {
                    if let Some(file) = facet?.cast::<PyDict>()?.get_item("path")? {
                        files.push(file.extract()?);
                    }
                }
                Some(crate::library_view::facets::include_file(&filter, path, &files))
            } else {
                arg.strip_prefix("toggle:").filter(|_| matches!(verb, "side" | "verb"))
                    .map(|token| crate::library_view::facets::toggle(&filter, token))
            };
            if let Some(query) = next_query {
                st.set_item("scope","s:all")?;
                st.set_item("filter",&query)?;
                cs(py)?.getattr("open")?.call((&win,slf),Some(&[("search",query)].into_py_dict(py)?))?;
                return Ok(());
            }
            if verb=="verb" && arg=="reset-filter" {
                st.set_item("filter","")?;
                cs(py)?.getattr("open")?.call((&win,slf),Some(&[("search","")].into_py_dict(py)?))?;
                return Ok(());
            }

        }
        match verb {
            "search" => st.set_item("filter", &arg)?,
            "tab" => {
                let mut ok = false;
                for p in lib(py)?.getattr("PROVIDERS")?.try_iter()? {
                    let p = p?;
                    ok |= p.getattr("id")?.eq(&arg)? && p.getattr("ready")?.is_truthy()?;
                }
                if ok {
                    st.set_item("type", &arg)?;
                    st.set_item("scope", if matches!(arg.as_str(), "styles") {"s:bundled"} else if matches!(arg.as_str(), "families" | "sections" | "presets" | "profiles") { "s:all" } else { "s:model" })?;
                    st.set_item("cursor", py.None())?;
                    _save_state(&st)?;
                }
            }
            "side" => {
                st.set_item("scope", &arg)?;
                st.set_item("cursor", py.None())?;
                _save_state(&st)?;
            }
            "fold" => {
                st.get_item("fold")?.call_method1("symmetric_difference_update", (PySet::new(py, [&arg])?,))?;
                _save_state(&st)?;
            }
            "headmenu" if args.len() > 1 => {
                let opt_ = &args[1];
                if arg=="group" && matches!(opt_.as_str(),"none"|"class"|"source") { st.set_item("component_group",opt_)?; } else if let Some(facet) = opt_.strip_prefix("facet:") {
                    st.set_item("facet", crate::library_view::facet_value(facet).unwrap_or("all"))?;
                } else if py.import("builtins")?.getattr("dict")?.call1((sorts(py)?,))?.contains(opt_)? {
                    st.set_item("sort", opt_)?;
                }
                _save_state(&st)?;
            }
            "focus" => st.set_item("cursor", &arg)?,
            "open" => {
                slf.call_method1("run", (ctx, "apply", &arg))?;
            }
            "menu" if args.len() > 1 => {
                slf.call_method1("run", (ctx, &args[1], &arg))?;
            }
            "verb" => {
                if arg == "import-collection" {
                    collection_form(&win)?;
                } else if let Some(action) = arg.strip_prefix("typeact:") {
                    provider(py, &st.get_item("type")?.str()?.to_string())?.call_method1("type_action", (&lctx, action))?;
                } else {
                    let cursor = st.call_method1("get", ("cursor",))?;
                    if cursor.is_truthy()? {
                        slf.call_method1("run", (ctx, &arg, cursor))?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn run(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, verb: &str, key: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let view = slf.getattr("view")?;
        let win = ctx.getattr("win")?;
        let st = _state(&view)?;
        let e = st.get_item("items")?.call_method1("get", (key,))?;
        if e.is_none() {
            return Ok(());
        }
        let prov = provider(py, &st.get_item("type")?.str()?.to_string())?;
        let lctx = ctx_of(py, &win, &view)?;
        let target = slf.getattr("paint_target")?;
        if !target.is_none() {
            if verb == "apply" {
                let pr = prov.call_method1("paint_recipe", (&lctx, &e))?;
                let (name, recipe) = (pr.get_item(0)?, pr.get_item(1)?);
                let what = if name.is_truthy()? { s(py, &format!("m:{}", name.str()?)) } else { key.clone() };
                target.call_method1("apply", (win.getattr("viewport")?, what, recipe))?;
                cs(py)?.call_method1("close", (&win,))?;
            }
            return Ok(());
        }
        match verb {
            "rename" => return slf.call_method1("_rename", (ctx, &e)).map(|_| ()),
            "opensource" => {
                let path = get(&e, "file")?;
                if path.is_truthy()? && win.hasattr("open_path")? {
                    win.call_method1("open_path", (path,))?;
                }
                return Ok(());
            }
            "apply" => {
                lib(py)?.call_method1("_remember", (&view, key))?;
                st.set_item("cursor", key)?;
            }
            _ => {}
        }
        if destinations::run(&lctx, verb, &e)? {
            if verb == "apply" {
                cs(py)?.call_method1("close", (&win,))?;
            }
            return Ok(());
        }
        prov.call_method1("run", (&lctx, verb, &e))?;
        if verb == "duplicate" {
            st.set_item("force_focus", true)?;
        }
        if verb == "apply" || (verb == "edit" && prov.getattr("id")?.eq("materials")?) {
            // Painting, placing and the material inspector use the model workspace.
            cs(py)?.call_method1("close", (&win,))?;
        }
        Ok(())
    }

    fn close_catalogue(slf: &Bound<'_, Self>) -> PyResult<()> {
        let catalogue = slf.getattr("catalogue")?;
        if !catalogue.is_none() {
            catalogue.call_method0("close")?;
            slf.setattr("catalogue", slf.py().None())?;
        }
        Ok(())
    }

    fn closed(slf: &Bound<'_, Self>, _ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        let st=_state(&slf.getattr("view")?)?;
        st.set_item("bulk_stamp",slf.py().None())?;
        st.set_item("replacement_origin",slf.py().None())?;
        slf.call_method0("close_catalogue")?;
        Ok(())
    }
}

#[pyfunction]
fn refresh(win: &Bound<'_, PyAny>) -> PyResult<()> {
    crate::sheet_runtime::refresh(win)
}

#[pyfunction]
fn _sheet_provider<'py>(view: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = view.py();
    if let Some(p) = view.getattr("__dict__")?.cast::<PyDict>()?.get_item("_ui_lib_sheet")? {
        if !p.is_none() {
            return Ok(p);
        }
    }
    let p = lib(py)?.getattr("LibrarySheet")?.call1((view,))?;
    view.setattr("_ui_lib_sheet", &p)?;
    Ok(p)
}

/// The Paint strip: the model's materials and the recent ones, the
/// current one's name, Browse all… (the sheet).
#[pyfunction]
fn before_frame(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    if let Some(job) = win.getattr_opt("_family_import_job")?.filter(|j| !j.is_none()) {
        let status = collections::snapshot();
        if status.active {
            if !job.getattr("phase")?.eq(&status.detail)? {
                job.call_method1("update", (py.None(), &status.detail))?;
            }
            view.call_method0("update")?;
        } else {
            win.setattr("_family_import_job", py.None())?;
            let kw = PyDict::new(py);
            if let Some(n) = status.imported {
                kw.set_item("result_title", format!("Imported {n} {}", if n == 1 { "type" } else { "types" }))?;
            } else {
                kw.set_item("failed", true)?;
            }
            kw.set_item("detail", status.detail)?;
            job.call_method("end", (), Some(&kw))?;
        }
    }
    // The modal sheet covers the Paint strip. Its hidden swatches need no refresh.
    let paint_visible = !crate::sheet_runtime::shown(win)?
        && ui(py)?.call_method1("_active_tool_id", (win,))?.eq("paint")?;
    let items = if paint_visible { palette_items(view, win)? } else { PyList::empty(py) };
    if !opt(view, "_ui_palette").is_some_and(|p| p.eq(&items).unwrap_or(false)) {
        view.setattr("_ui_palette", &items)?;
        view.getattr("renderer")?.call_method1("ui_set_palette", (&items,))?;
    }
    Ok(())
}

// ---- opening ---------------------------------------------------------------------------

fn active_tool(win: &Bound<'_, PyAny>) -> PyResult<String> {
    let Some(actions) = opt(win, "_tool_actions") else { return Ok(String::new()) };
    for pair in actions.call_method0("items")?.try_iter()? {
        let pair = pair?;
        if pair.get_item(1)?.call_method0("isChecked")?.is_truthy()? {
            return pair.get_item(0)?.extract();
        }
    }
    Ok(String::new())
}

/// Which type opens: a component being placed opens Components, Paint opens
/// Materials.
#[pyfunction]
fn _land(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    let st = _state(view)?;
    let vp = win.getattr("viewport")?;
    let tool = opt(&vp, "active_tool").or_else(|| opt(&vp, "tools").and_then(|t| opt(&t, "active")));
    if tool.is_some_and(|t| t.get_type().name().map(|n| n.to_string() == "PlaceGroupTool").unwrap_or(false)) {
        st.set_item("type", "components")?;
    } else if active_tool(win)? == "paint" {
        st.set_item("type", "materials")?;
    }
    Ok(())
}

/// Window › Library › <type>, ⌘K, the rail, the Paint strip's Browse all…:
/// the Library sheet on that type. Requires an active document view.
#[pyfunction]
#[pyo3(signature = (win, type_id=None, search=None, scope=None, *, paint_target=None))]
fn open_library(py: Python<'_>, win: &Bound<'_, PyAny>, type_id: Option<String>, search: Option<String>, scope: Option<String>, paint_target: Option<Bound<'_, PyAny>>) -> PyResult<bool> {
    let Some(view) = opt(win, "_doc").and_then(|d| opt(&d, "stand_in")) else { return Ok(false) };
    let st = _state(&view)?;
    _land(&view, win)?;
    if let Some(t) = type_id.filter(|t| !t.is_empty()) {
        if matches!(t.as_str(), "families" | "sections" | "profiles" | "presets") && !st.get_item("type")?.eq(&t)? { st.set_item("scope", "s:all")?; }
        st.set_item("type", t)?;
    }
    if let Some(sc) = scope.filter(|s| !s.is_empty()) {
        st.set_item("scope", sc)?;
    }
    st.set_item("filter", search.clone().unwrap_or_default())?;
    let sheet = match paint_target {
        Some(t) => lib(py)?.getattr("LibrarySheet")?.call1((&view, t))?,
        None => _sheet_provider(&view)?,
    };
    cs(py)?.getattr("open")?.call((win, sheet), Some(&[("search", search)].into_py_dict(py)?))?.is_truthy()
}

/// Route Project commands to their lens or the Library sheet.
#[pyfunction]
fn set_mode(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>, mode: &str) -> PyResult<()> {
    let py = view.py();
    if mode == "library" {
        lib(py)?.call_method1("open_library", (win,))?;
        return Ok(());
    }
    let mut mode = mode;
    if mode == "problems" || mode == "deliveries" {
        // The Export columns: a preset of the same tree, at the same width.
        view.setattr("_ui_lens", 2)?;
        view.setattr("_ui_columns", py.None())?;
        mode = "";
    }
    view.setattr("_ui_mode", mode)?;
    for attr in ["_ui_mat_focus", "_ui_rules_cache", "_ui_tree_sig", "_ui_insp_sig"] {
        view.setattr(attr, py.None())?;
    }
    view.call_method0("update")?;
    Ok(())
}

/// Window › Library › Materials / Components: menu entries (so ⌘K finds
/// them) and the rail's Components button.
#[pyfunction]
fn install(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    if opt(win, "_lib_installed").is_some_and(|v| v.is_truthy().unwrap_or(false)) {
        return Ok(());
    }
    win.setattr("_lib_installed", true)?;
    let opener = |t: &str| -> Res {
        let (w, t) = (win.clone().unbind(), t.to_owned());
        func(py, move |py, _a| Ok(lib(py)?.call_method1("open_library", (w.bind(py), &t))?.unbind()))
    };
    let actions = PyDict::new(py);
    for p in lib(py)?.getattr("PROVIDERS")?.try_iter()? {
        let p = p?;
        if p.getattr("ready")?.is_truthy()? {
            let id: String = p.getattr("id")?.extract()?;
            let a = crate::shell_commands::make(win, &p.getattr("label")?, Some(opener(&id)?), None, false, false)?;
            actions.set_item(&id, a)?;
        }
    }
    win.setattr("_lib_actions", actions)?;
    let comp = crate::shell_commands::make(win, &s(py, "Components"), Some(opener("components")?), None, false, false)?;
    win.setattr("_act_library_components", comp)?;
    Ok(())
}

// ---- the palette strip (Paint's fast path) ----------------------------------------------

/// `(hex, image path, name)` of a bundled or kept material, from its key.
fn recent_look<'py>(py: Python<'py>, key: &str) -> PyResult<(String, Bound<'py, PyAny>, String)> {
    let none = || (String::new(), py.None().into_bound(py), String::new());
    let look = || -> PyResult<Option<(String, Bound<'py, PyAny>, String)>> {
        match key_kind(key) {
            "ral" | "lib" => {
                let it = catalogue(py)?.call_method1("get", (key,))?;
                if it.is_truthy()? {
                    return Ok(Some((it.get_item("hex")?.str()?.to_string(), it.get_item("thumb")?, it.get_item("name")?.str()?.to_string())));
                }
            }
            "my" => {
                let i: i64 = py.import("builtins")?.getattr("int")?.call1((key_rest(key),))?.extract()?;
                let e = mine(py)?.get_item(i)?;
                let texture = get(&e, "texture")?;
                let tex = if texture.is_truthy()? { get(&texture, "path")? } else { py.None().into_bound(py) };
                return Ok(Some((_hex(&get(&e, "color")?)?, tex, e.call_method1("get", ("name", ""))?.str()?.to_string())));
            }
            _ => {}
        }
        Ok(None)
    };
    match look() {
        Ok(Some(v)) => Ok(v),
        Ok(None) => Ok(none()),
        Err(e) if [e.is_instance_of::<pyo3::exceptions::PyOSError>(py), e.is_instance_of::<pyo3::exceptions::PyValueError>(py),
                   e.is_instance_of::<pyo3::exceptions::PyIndexError>(py), e.is_instance_of::<pyo3::exceptions::PyKeyError>(py)].iter().any(|b| *b) => Ok(none()),
        Err(e) => Err(e),
    }
}

/// `(id, name, hex, icon, active)` per swatch; `("-", …)` a rule; `("",
/// name…)` the current material's name.
#[pyfunction]
fn palette_items<'py>(view: &Bound<'py, PyAny>, win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let py = view.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let cur = crate::document_registration::view::calls::current_material_name(py)?;
    let is_cur = |name: &Bound<'_, PyAny>| name.eq(&cur).unwrap_or(false);
    let out = PyList::empty(py);
    let mats: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> = match opt(&scene, "materials").filter(|m| m.is_truthy().unwrap_or(false)) {
        Some(m) => m.call_method0("items")?.try_iter()?.map(|p| { let p = p?; Ok((p.get_item(0)?, p.get_item(1)?)) }).collect::<PyResult<_>>()?,
        None => Vec::new(),
    };
    let usage = sheet_usage(&scene)?;
    let uses = if usage.is_none() { PyDict::new(py).into_any() } else { usage.get_item(0)? };
    let names: Vec<Bound<'_, PyAny>> = mats.iter().map(|(n, _)| n.clone()).collect();
    let counts: Vec<i64> = mats.iter().map(|(n, _)| uses.call_method1("get", (n, 0)).and_then(|v| v.extract())).collect::<PyResult<_>>()?;
    let order: Vec<usize> = render(py)?.call_method1("library_by_use", (names, counts))?.extract()?;
    // The model's, most used first (what you paint with most is under the hand).
    let ordered: Vec<&(Bound<'_, PyAny>, Bound<'_, PyAny>)> = order.iter().filter_map(|i| mats.get(*i)).collect();
    for (name, mat) in ordered.iter().take(STRIP_MODEL) {
        let tex = opt(mat, "texture").filter(|t| t.is_truthy().unwrap_or(false));
        let icon = match tex.map(|t| get(&t, "path")).transpose()?.filter(|p| p.is_truthy().unwrap_or(false)) {
            Some(p) => thumb(view, &p)?,
            None => String::new(),
        };
        out.append((format!("m:{}", name.str()?), name, crate::document_registration::view::calls::hex(&mat.getattr("color")?)?, icon, is_cur(name)))?;
    }
    let st = _state(view)?;
    let in_model = |n: &str| mats.iter().any(|(m, _)| m.eq(n).unwrap_or(false));
    let mut rec: Vec<(String, String, String, String, bool)> = Vec::new();
    for key in st.get_item("recent")?.try_iter()? {
        if rec.len() >= STRIP_RECENT {
            break;
        }
        let key: String = key?.extract()?;
        if !matches!(key_kind(&key), "lib" | "ral" | "my") {
            continue;
        }
        let (hexc, icon, name) = recent_look(py, &key)?;
        if !name.is_empty() && !in_model(&name) && rec.iter().all(|r| r.1 != name) {
            let icon = if icon.is_truthy()? { thumb(view, &icon)? } else { String::new() };
            let active = cur.eq(&name)?;
            rec.push((key, name, hexc, icon, active));
        }
    }
    let out_names: Vec<String> = out.iter().map(|r| r.get_item(1).and_then(|n| n.str().map(|s| s.to_string()))).collect::<PyResult<_>>()?;
    for r in py.import("built_core")?.call_method1("recent_swatches", (paint_tool(py)?,))?.try_iter()? {
        if rec.len() >= STRIP_RECENT {
            break;
        }
        let r = r?;
        let (key, name, colour, path) = (r.get_item(0)?.str()?.to_string(), r.get_item(1)?.str()?.to_string(), r.get_item(2)?, r.get_item(3)?);
        if out_names.iter().all(|n| *n != name) && rec.iter().all(|x| x.1 != name) {
            let icon = if path.is_truthy()? { thumb(view, &path)? } else { String::new() };
            let active = cur.eq(&name)?;
            rec.push((key, name, colour.str()?.to_string(), icon, active));
        }
    }
    if !rec.is_empty() {
        out.append(("-", "", "", "", false))?;
        for r in rec {
            out.append(r)?;
        }
    }
    if cur.is_truthy()? {
        out.append(("-", "", "", "", false))?;
        out.append(("", &cur, "", "", false))?;
    }
    Ok(out)
}

/// A swatch of the strip: paint with it (a recent bundled one too).
#[pyfunction]
fn paint_from_strip(win: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let py = win.py();
    let view = opt(win, "_doc").and_then(|d| opt(&d, "stand_in"));
    if let Some(stamp) = key.strip_prefix("stamp:") {
        py.import("built_core")?.call_method1("pick_recent", (win.getattr("viewport")?.getattr("scene")?, paint_tool(py)?, loads(py, &s(py, stamp))?))?;
        win.call_method1("_activate_tool", ("paint",))?;
        return Ok(());
    }
    if let Some(v) = &view {
        lib(py)?.call_method1("_remember", (v, key))?;
    }
    if let (Some(rest), Some(v)) = (key.strip_prefix("my:"), &view) {
        let Ok(i) = rest.parse::<usize>() else { return Ok(()) };
        let all = mine(py)?;
        if i >= all.len()? {
            return Ok(());
        }
        let entry = all.get_item(i)?;
        let it = item(py, &[("key", s(py, key)), ("name", entry.call_method1("get", ("name", ""))?)])?;
        provider(py, "materials")?.call_method1("run", (ctx_of(py, win, v)?, "apply", it))?;
        return Ok(());
    }
    ui(py)?.call_method1("_paint_with", (win, key))?;
    Ok(())
}

// ---- actions ------------------------------------------------------------------

/// A Library question answered yes.
#[pyfunction]
#[pyo3(signature = (win, view, pending, text=""))]
fn confirmed(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, pending: &Bound<'_, PyAny>, text: &str) -> PyResult<()> {
    let py = win.py();
    let (what, type_id, key) = (pending.get_item(0)?, pending.get_item(1)?, pending.get_item(2)?);
    if what.eq("libdelete")? {
        destinations::confirmed(win, view, &key.str()?.to_string())?;
        _state(view)?.set_item("sig", py.None())?;
        return Ok(());
    }
    let p = provider(py, &type_id.str()?.to_string())?;
    if p.hasattr("confirmed")? {
        p.call_method1("confirmed", (ctx_of(py, win, view)?, what, key, text))?;
    }
    _state(view)?.set_item("sig", py.None())?;
    Ok(())
}

#[pyfunction]
fn _remember(view: &Bound<'_, PyAny>, key: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let st = _state(view)?;
    let recent = PyList::new(py, [key])?;
    for k in st.get_item("recent")?.try_iter()? {
        let k = k?;
        if !k.eq(key)? && recent.len() < RECENT_MAX {
            recent.append(k)?;
        }
    }
    st.set_item("recent", &recent)?;
    store_json(py, "library/recent", recent.as_any())
}

/// `verb:key` from an inspector chip (the material being edited in the
/// dock's inspector).
#[pyfunction]
fn action(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, arg: &str) -> PyResult<()> {
    let py = win.py();
    if view.is_none() {
        return Ok(());
    }
    if arg == "done" {
        view.setattr("_ui_mat_focus", py.None())?;
        view.setattr("_ui_insp_sig", py.None())?;
        view.call_method0("update")?;
        return Ok(());
    }
    let arg = if !arg.contains(SEP) && arg.contains(':') {
        let (verb, rest) = arg.split_once(':').expect("has a colon");
        format!("{verb}{SEP}{rest}")
    } else {
        arg.to_owned()
    };
    let (verb, rest) = arg.split_once(SEP).unwrap_or((&arg, ""));
    let (mut key, mut extra) = rest.split_once(SEP).map(|(a, b)| (a.to_owned(), b.to_owned())).unwrap_or((rest.to_owned(), String::new()));
    if (verb == "map" || verb == "unmap") && extra.is_empty() {
        let (k, e) = key.rsplit_once(':').map(|(a, b)| (a.to_owned(), b.to_owned())).unwrap_or((String::new(), key.clone()));
        key = k;
        extra = e;
    }
    let ctx = ctx_of(py, win, view)?;
    // Whether the view repaints after (Python returned early without).
    let result = (|| -> PyResult<bool> {
        if verb == "browse" {
            lib(py)?.call_method1("open_library", (win, if key.is_empty() { None } else { Some(&key) }))?;
            return Ok(false);
        }
        let it = _item_anywhere(view, win, &key)?;
        if it.is_none() {
            return Ok(false);
        }
        let p = provider(py, if key.starts_with("cm:") || key.starts_with("sc:") || key.starts_with("pp:") { "components" } else { "materials" })?;
        if verb == "apply" {
            lib(py)?.call_method1("_remember", (view, &key))?;
        }
        p.call_method1("run", (&ctx, verb, it, &extra))?;
        view.setattr("_ui_insp_sig", py.None())?;
        Ok(true)
    })();
    let repaint = match result {
        Ok(r) => r,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) || e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {
            let msg = e.value(py).str()?.to_string();
            flash(&ctx, msg.trim_matches(|c| c == '\'' || c == '"'))?;
            true
        }
        Err(e) => return Err(e),
    };
    if repaint {
        view.call_method0("update")?;
    }
    Ok(())
}

/// The item for `key`, from the last sheet build or the model.
#[pyfunction]
fn _item_anywhere<'py>(view: &Bound<'py, PyAny>, win: &Bound<'py, PyAny>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    let py = view.py();
    let it = item_of(view, &s(py, key))?;
    if !it.is_none() {
        return Ok(it);
    }
    let Some(name) = key.strip_prefix("m:") else { return Ok(py.None().into_bound(py)) };
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let mat = scene.getattr("materials")?.call_method1("get", (name,))?;
    if mat.is_none() {
        return Ok(py.None().into_bound(py));
    }
    let tex = opt(&mat, "texture").filter(|t| t.is_truthy().unwrap_or(false)).map(|t| get(&t, "path")).transpose()?.unwrap_or_else(|| py.None().into_bound(py));
    Ok(item(py, &[
        ("key", s(py, key)),
        ("name", s(py, name)),
        ("source", s(py, "model")),
        ("category", s(py, "")),
        ("uses", material_uses(&scene, None)?.call_method1("get", (name, 0))?),
        ("hex", s(py, &_hex(&mat.getattr("color")?)?)),
        ("thumb", tex),
        ("verbs", PyTuple::empty(py).into_any()),
    ])?
    .into_any())
}

// ---- inspector ----------------------------------------------------------------------------

/// The material the dock's inspector edits: one picked for editing, else
/// Paint's current one while Paint is active (`brush:` for an unnamed one).
#[pyfunction]
fn inspector_key(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    let py = view.py();
    let mats = win.getattr("viewport")?.getattr("scene")?.getattr("materials")?;
    let focus = view.getattr("__dict__")?.call_method1("get", ("_ui_mat_focus",))?;
    if focus.is_truthy()? {
        let f: String = focus.str()?.to_string();
        if let Some(name) = f.strip_prefix("m:") {
            if mats.contains(name)? {
                return Ok(Some(f));
            }
        }
    }
    if ui(py)?.call_method1("_active_tool_id", (win,))?.eq("paint")? {
        let cur = crate::document_registration::view::calls::current_material_name(py)?;
        if cur.is_truthy()? && mats.contains(&cur)? {
            return Ok(Some(format!("m:{}", cur.str()?)));
        }
        if !paint_tool(py)?.getattr("current_is_default")?.is_truthy()? {
            return Ok(Some("brush:".into()));
        }
    }
    Ok(None)
}

/// `(title, sections)` for the material the inspector edits, or None.
#[pyfunction]
fn inspector<'py>(view: &Bound<'py, PyAny>, win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = view.py();
    let Some(key) = inspector_key(view, win)? else { return Ok(py.None().into_bound(py)) };
    if key == "brush:" {
        return py.import("built_core")?.call_method1("brush_inspector", (paint_tool(py)?,));
    }
    let it = _item_anywhere(view, win, &key)?;
    if it.is_none() {
        return Ok(it);
    }
    provider(py, "materials")?.call_method1("inspector", (ctx_of(py, win, view)?, it))
}

/// An inspector field of a material: `<field>:<material name>`.
#[pyfunction]
fn edit_field(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, arg: &str, text: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let (field, name) = arg.split_once(':').unwrap_or((arg, ""));
    let key = format!("m:{name}");
    let mut it = _item_anywhere(view, win, &key)?;
    if it.is_none() {
        it = item(py, &[("key", s(py, &key)), ("name", s(py, name))])?.into_any();
    }
    provider(py, "materials")?.call_method1("edit", (ctx_of(py, win, view)?, it, field, text))?;
    py.import("built_core")?.call_method1("refresh_library_paint", (win.getattr("viewport")?.getattr("scene")?, paint_tool(py)?))?;
    lib(py)?.call_method1("refresh", (win,))?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "wgpu_library")?;
    sub.add("SEP", SEP)?;
    let ty = m.getattr("library_type")?.call1(("",))?;
    sub.add("SORTS", ty.get_item(0)?)?;
    let verbs_map = py.import("builtins")?.getattr("dict")?.call1((m.getattr("library_verbs")?.call1(("",))?,))?;
    verbs_map.set_item("-", "")?;
    sub.add("VERBS", verbs_map)?;
    sub.add("RECENT_MAX", RECENT_MAX)?;
    sub.add("ROW_CAP", 600)?; // compatibility with plugins using the former display limit
    sub.add("THUMB_PX", THUMB_PX)?;
    sub.add("STRIP_MODEL", STRIP_MODEL)?;
    sub.add("STRIP_RECENT", STRIP_RECENT)?;
    sub.add("PBR_ROWS", PBR_ROWS.to_vec())?;
    sub.add("_bundled_memo", PyDict::new(py))?;
    sub.add_class::<Ctx>()?;
    sub.add_class::<LibraryProvider>()?;
    sub.add_class::<MaterialsProvider>()?;
    sub.add_class::<ComponentsProvider>()?;
    sub.add_function(wrap_pyfunction!(collections::import_collection, &sub)?)?;
    sub.add_class::<FamiliesProvider>()?;
    sub.add_class::<ProfilesProvider>()?;
    sub.add_class::<SectionsProvider>()?;
    sub.add_class::<PresetsProvider>()?;
    sub.add_class::<StylesProvider>()?;
    sub.add_class::<LibrarySheet>()?;
    sub.add("PROVIDERS", vec![
        Bound::new(py, MaterialsProvider::new())?.into_any(),
        Bound::new(py, ComponentsProvider::new())?.into_any(),
        Bound::new(py, FamiliesProvider::new())?.into_any(),
        Bound::new(py, ProfilesProvider::new())?.into_any(),
        Bound::new(py, SectionsProvider::new())?.into_any(),
        Bound::new(py, PresetsProvider::new())?.into_any(),
        Bound::new(py, StylesProvider::new())?.into_any(),
    ])?;
    for f in [
        wrap_pyfunction!(_hex, &sub)?,
        wrap_pyfunction!(material_uses, &sub)?,
        wrap_pyfunction!(linked_material_uses, &sub)?,
        wrap_pyfunction!(select_material_uses, &sub)?,
        wrap_pyfunction!(unnamed_colours_cached, &sub)?,
        wrap_pyfunction!(_bundled_by_key, &sub)?,
        wrap_pyfunction!(_catalogue, &sub)?,
        wrap_pyfunction!(_pick_paint, &sub)?,
        wrap_pyfunction!(_pick_image, &sub)?,
        wrap_pyfunction!(provider, &sub)?,
        wrap_pyfunction!(_state, &sub)?,
        wrap_pyfunction!(_save_state, &sub)?,
        wrap_pyfunction!(filtered, &sub)?,
        wrap_pyfunction!(side_rows, &sub)?,
        wrap_pyfunction!(groups_for, &sub)?,
        wrap_pyfunction!(item_of, &sub)?,
        wrap_pyfunction!(refresh, &sub)?,
        wrap_pyfunction!(_sheet_provider, &sub)?,
        wrap_pyfunction!(before_frame, &sub)?,
        wrap_pyfunction!(_land, &sub)?,
        wrap_pyfunction!(open_library, &sub)?,
        wrap_pyfunction!(set_mode, &sub)?,
        wrap_pyfunction!(install, &sub)?,
        wrap_pyfunction!(palette_items, &sub)?,
        wrap_pyfunction!(paint_from_strip, &sub)?,
        wrap_pyfunction!(confirmed, &sub)?,
        wrap_pyfunction!(_remember, &sub)?,
        wrap_pyfunction!(action, &sub)?,
        wrap_pyfunction!(_item_anywhere, &sub)?,
        wrap_pyfunction!(inspector_key, &sub)?,
        wrap_pyfunction!(inspector, &sub)?,
        wrap_pyfunction!(edit_field, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    sub.add("item", sub.getattr("item_of")?)?;
    destinations::register(&sub)?;
    packs::register(&sub)?;
    m.add_submodule(&sub)?;
    py.import("sys")?.getattr("modules")?.set_item("built_render.wgpu_library", &sub)?;
    Ok(())
}
