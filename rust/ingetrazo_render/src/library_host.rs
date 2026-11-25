// SPDX-License-Identifier: GPL-3.0-or-later
//! The Library on the window: the submodule `ingetrazo_render.wgpu_library`.
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
//! `core.protocol.commands`.
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
const PBR_ROWS: [(&str, &str, &str); 4] = [
    ("metallic", "Metallic", "metallic_map"),
    ("roughness", "Roughness", "roughness_map"),
    ("normal_scale", "Normal", "normal_map"),
    ("ao_strength", "Occlusion", "ao_map"),
];

/// This submodule, through its attributes (tests replace `_pick_image`,
/// `_bundled_by_key`, `open_library`).
fn lib(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("ingetrazo_render")?.getattr("wgpu_library")
}

fn ui(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("views.wgpu_ui")
}

fn render(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_render")
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
    py.import("ingetrazo_core.settings")?.call_method0("settings")
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
#[pyclass(module = "ingetrazo_render.wgpu_library", dict, get_all, set_all)]
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
    let names = py.import("ingetrazo_core")?.call_method1("store_material_names", (mesh,))?;
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
/// (`ingetrazo_core.material_survey`): paint by mesh and placement, kept
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
    py.import("ingetrazo_core")?.call_method1("material_survey", (scene, history))
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
        let links = py.import("core.links")?;
        let purge = py.import("ingetrazo_core")?;
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
    let purge = py.import("ingetrazo_core")?;
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
    let found = py.import("core.material_commands")?.call_method1("unnamed_colours", (scene,))?;
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
#[pyclass(module = "ingetrazo_render.wgpu_library", name = "Provider", subclass, dict)]
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
#[pyclass(module = "ingetrazo_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct MaterialsProvider;

fn rename_paint(py: Python<'_>, scene: &Bound<'_, PyAny>, old: &str, new: &str) -> PyResult<()> {
    let tool = paint_tool(py)?;
    if let Some(mat) = opt(&tool, "current_material") {
        if mat.getattr("name")?.eq(old)? {
            let kw = [("name", new)].into_py_dict(py)?;
            py.import("ingetrazo_core")?.getattr("pick_library_paint")?.call((scene, tool), Some(&kw))?;
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
    py.import("ingetrazo_render.chrome_dialogs")?.getattr("ask_later")?.call((win, title, body, vec![("cancel", "Cancel"), ("yes", yes)], done), Some(&kw))?;
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
        let got = survey(&scene, ctx_history(ctx))?;
        let (uses, colours, worn) = (got.get_item(0)?, got.get_item(1)?, got.get_item(2)?);
        let out = PyList::empty(py);
        if let Some(mats) = opt(&scene, "materials").filter(|m| m.is_truthy().unwrap_or(false)) {
            for pair in mats.call_method0("items")?.try_iter()? {
                let pair = pair?;
                let (name, mat) = (pair.get_item(0)?, pair.get_item(1)?);
                let tex = opt(&mat, "texture").filter(|t| t.is_truthy().unwrap_or(false)).map(|t| get(&t, "path")).transpose()?.unwrap_or_else(|| py.None().into_bound(py));
                out.append(item(py, &[
                    ("key", s(py, &format!("m:{}", name.str()?))),
                    ("name", name.clone()),
                    ("source", s(py, "model")),
                    ("category", s(py, "")),
                    ("base", s(py, if tex.is_truthy()? { "image" } else { "colour" })),
                    ("uses", uses.call_method1("get", (&name, 0))?),
                    ("hex", s(py, &_hex(&mat.getattr("color")?)?)),
                    ("thumb", tex),
                    ("verbs", verbs(py, &["apply", "edit", "select", "-", "duplicate", "rename", "keep", "-", "delete"])?),
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
        let uses = material_uses(&ctx.getattr("scene")?, None)?;
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
        let per_file = linked_material_uses(&scene)?;
        if !per_file.is_truthy()? {
            return Ok((model.clone(), PyList::empty(py).into_any()));
        }
        let files = PyList::empty(py);
        for pair in per_file.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let counts = py.import("builtins")?.getattr("list")?.call1((pair.get_item(1)?.call_method0("items")?,))?;
            files.append((pair.get_item(0)?, counts))?;
        }
        let uses = py.import("builtins")?.getattr("dict")?.call1((material_uses(&scene, None)?,))?;
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
        for part in [pair.get_item(0)?, pair.get_item(1)?, slf.call_method1("_mine_items", (ctx,))?] {
            for e in part.try_iter()? {
                out.append(e?)?;
            }
        }
        for e in bundled.try_iter()? {
            out.append(py.import("builtins")?.getattr("dict")?.call1((e?,))?)?;
        }
        Ok(out)
    }

    fn type_actions<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let scene = ctx.getattr("scene")?;
        let n = py.import("ingetrazo_core")?.call_method1("purge_unused_materials", (&scene,))?.len()?;
        let acts = PyList::empty(py);
        let k = survey(&scene, ctx_history(ctx))?.get_item(1)?.len()?;
        if k != 0 {
            acts.append(("name", format!("Name these: {k} unnamed colour{}", if k != 1 { "s" } else { "" })))?;
        }
        acts.append(("purge", if n != 0 { format!("Purge unused materials ({n})") } else { "Purge unused materials (none)".into() }))?;
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
                let mat = scene.getattr("materials")?.call_method1("get", (&name,))?;
                if !mat.is_none() {
                    let kept = PyList::empty(py);
                    for e in mine(py)?.try_iter()? {
                        let e = e?;
                        let t = get(&e, "type")?;
                        if !((t.eq("materials")? || t.eq("colours")?) && get(&e, "name")?.eq(&name)?) {
                            kept.append(e)?;
                        }
                    }
                    let entry = PyDict::new(py);
                    entry.set_item("type", "materials")?;
                    entry.update(mat.call_method0("to_dict")?.cast::<pyo3::types::PyMapping>()?)?;
                    kept.append(entry)?;
                    store_json(py, MINE_KEY, kept.as_any())?;
                    bump_mine(&view)?;
                    flash(ctx, &format!("{name_s} is in My library, for every project"))?;
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
        Ok(Some(py.import("ingetrazo_core")?.call_method1("material_inspector", (mat, item.call_method1("get", ("uses", 0))?, painting, focused))?))
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
            let new = py.import("ingetrazo_core")?.call_method1("edit_swatch", (ctx.getattr("vp")?.getattr("history")?, paint_tool(py)?, &name, field, &text))?;
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
        } else if field == "pbr" || field.starts_with("percent_") {
            let values = py.import("ingetrazo_core")?.call_method1("material_surface_input", (field, &text))?;
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
#[pyclass(module = "ingetrazo_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct ComponentsProvider;

fn is_linked(g: &Bound<'_, PyAny>) -> bool {
    g.py().import("core.links").and_then(|l| l.call_method1("is_linked", (g,))).and_then(|v| v.is_truthy()).unwrap_or(false)
}

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
        for (i, row) in slf.call_method1("_model", (ctx,))?.try_iter()?.enumerate() {
            let row = row?;
            let (name, groups, kept) = (row.get_item(0)?, row.get_item(2)?, row.get_item(3)?);
            let gs: Vec<Bound<'_, PyAny>> = groups.try_iter()?.collect::<PyResult<_>>()?;
            let linked = !gs.is_empty() && gs.iter().all(is_linked);
            let mut v = if gs.is_empty() {
                vec!["apply", "replace", "rename", "-", "unkeepfile"]
            } else {
                vec!["apply", "edit", "select", "replace", "-", "rename", if kept.is_none() { "keepfile" } else { "unkeepfile" }, "-", "delete"]
            };
            let definition = gs.first().unwrap_or(&kept);
            let mesh = definition.getattr("mesh")?;
            if !selected.iter().any(|&id| id != mesh.as_ptr() as usize) {
                v.retain(|verb| *verb != "replace");
            }
            out.append(item(py, &[
                ("key", s(py, &format!("cm:{i}"))),
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
        let mut n = 0;
        for row in slf.call_method1("_model", (ctx,))?.try_iter()? {
            let row = row?;
            if !row.get_item(3)?.is_none() && !row.get_item(2)?.is_truthy()? {
                n += 1;
            }
        }
        let actions = PyList::empty(py);
        actions.append(("import", crate::model_edit::translated(py, "Components from File…", None)?))?;
        actions.append(("purge", if n != 0 { format!("Stop keeping unplaced components ({n})") } else { "Stop keeping unplaced components (none)".into() }))?;
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
        let history = py.import("ingetrazo_core.history")?;
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
                let kc = py.import("ingetrazo_core.kept_components")?;
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
            "select" => {
                render(py)?.call_method1("select_component_instances", (&win, &groups))?;
            }
            "replace" => {
                let group_cls = py.import("core.group")?.getattr("Group")?;
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
        let rows = vec![("Copies".to_owned(), groups.len()?.to_string()), ("Faces".to_owned(), grouped), ("Kept in file".to_owned(), if kept.is_none() { "no" } else { "yes" }.to_owned())];
        let acts = vec![(format!("lib:apply:{key}"), "Place a copy".to_owned()), (format!("lib:select:{key}"), "Select copies".to_owned()), (format!("lib:edit:{key}"), "Open for editing".to_owned())];
        let sections = PyList::empty(py);
        sections.append(("Component in this model", rows))?;
        sections.append(("Actions", acts))?;
        let title = if name.is_truthy()? { name } else { s(py, "Component") };
        Ok(Some(PyTuple::new(py, [title, sections.into_any()])?.into_any()))
    }
}

/// Coming: section outlines (steel shapes, extrusion and sweep profiles) to
/// draw, extrude and run along a path.
#[pyclass(module = "ingetrazo_render.wgpu_library", extends = LibraryProvider, dict)]
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

    #[classattr]
    fn ready() -> bool {
        false
    }
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
    let items = loads(py, &py.import("ingetrazo_core")?.call_method1("bundled_materials", (resources, language))?)?;
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
    let resources = py.import("ingetrazo_core.paths")?.call_method0("app_root")?.div("resources")?;
    let language = py.import("ingetrazo_core.i18n")?.call_method0("current_language")?;
    lib(py)?.call_method1("_bundled_by_key", (resources, language))
}

#[pyfunction]
#[pyo3(signature = (win, *, name="", recipe=None))]
fn _pick_paint(py: Python<'_>, win: &Bound<'_, PyAny>, name: &str, recipe: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    kw.set_item("recipe", recipe)?;
    if py.import("ingetrazo_core")?.getattr("pick_library_paint")?.call((win.getattr("viewport")?.getattr("scene")?, paint_tool(py)?), Some(&kw))?.is_truthy()? {
        win.call_method1("_activate_tool", ("paint",))?;
    }
    Ok(())
}

#[pyfunction]
fn _pick_image<'py>(py: Python<'py>, win: &Bound<'py, PyAny>, title: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    let dialogs = py.import("ingetrazo_render.shell_files")?.getattr("ShellFileDialog")?;
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
    let mut known = ty.eq("registry")?;
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
        let rgba = render(py)?.call_method1("swatch_pixels", (path, px, true))?;
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

/// The Library on the large sheet: types as tabs, scopes and categories on
/// the side (linked files' materials read-only), 96-pt tiles, the focused
/// item's details and model usage on the right. With a paint target it is
/// the tree's "Choose material" sheet.
#[pyclass(module = "ingetrazo_render.wgpu_library", extends = crate::sheet_runtime::Provider, dict, get_all, set_all)]
pub struct LibrarySheet {
    view: Py<PyAny>,
    paint_target: Py<PyAny>,
    catalogue: Py<PyAny>,
    registry: Py<PyAny>,
}

fn cs(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    render(py)?.getattr("chrome_sheet")
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
            registry: py.None(),
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
        PyTuple::new(py, [
            obj(py, scene.as_ptr() as usize)?.into_bound(py),
            scene.getattr("content_version")?,
            crate::document_registration::view::calls::current_material_name(py)?,
            st.get_item("mine_rev")?,
            if st.get_item("type")?.eq("components")? { scene.getattr("version")? } else { py.None().into_bound(py) },
            PyTuple::new(py, recent)?.into_any(),
        ])
    }

    fn _items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyDict>)> {
        let view = self.view.bind(py);
        let st = _state(view)?;
        let lctx = ctx_of(py, &ctx.getattr("win")?, view)?;
        let scene = lctx.getattr("scene")?;
        let components = st.get_item("type")?.eq("components")?;
        let generation = (scene.as_ptr() as usize, scene.getattr("content_version")?, st.get_item("mine_rev")?);
        let revision = scene.getattr("version")?;
        // A selection edit only changes which component rows offer Replace.
        if components {
            let cache = st.call_method1("get", ("component_selection_rows",))?;
            if !cache.is_none() && cache.get_item(0)?.eq(&generation)? && !cache.get_item(1)?.eq(&revision)? {
                let items = cache.get_item(2)?;
                let selected = selected_definitions(&scene)?;
                for item in items.try_iter()? {
                    let item = item?;
                    let Some(mesh) = item.cast::<PyDict>()?.get_item("definition_id")? else { continue };
                    let id: usize = mesh.extract()?;
                    let verbs = item.get_item("verbs")?;
                    let has = verbs.contains("replace")?;
                    let allowed = selected.iter().any(|&m| m != id);
                    if has != allowed {
                        let mut actions: Vec<String> = verbs.extract()?;
                        actions.retain(|v|v != "replace");
                        if allowed { actions.insert(1, "replace".into()); }
                        item.set_item("verbs", PyTuple::new(py, actions)?)?;
                    }
                }
                let counts = cache.get_item(3)?.cast_into::<PyDict>()?;
                st.set_item("component_selection_rows", (&generation, &revision, &items, &counts))?;
                st.set_item("items", by_key(&items)?)?;
                return Ok((items,counts));
            }
        }
        let counts = PyDict::new(py);
        let mut active = PyList::empty(py).into_any();
        for p in lib(py)?.getattr("PROVIDERS")?.try_iter()? {
            let p = p?;
            if !p.getattr("ready")?.is_truthy()? {
                continue;
            }
            let items = p.call_method1("items", (&lctx,))?;
            let mut n = 0;
            for e in items.try_iter()? {
                let src = e?.get_item("source")?;
                if src.eq("model")? || src.eq("linked")? {
                    n += 1;
                }
            }
            counts.set_item(p.getattr("id")?, n)?;
            if p.getattr("id")?.eq(st.get_item("type")?)? {
                active = items;
            }
        }
        if components { st.set_item("component_selection_rows", (generation, revision, &active, &counts))?; }
        st.set_item("items", by_key(&active)?)?;
        Ok((active, counts))
    }

    fn build<'py>(slf: &Bound<'py, Self>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let me = slf.borrow();
        let view = me.view.bind(py).clone();
        let st = _state(&view)?;
        let prov = provider(py, &st.get_item("type")?.str()?.to_string())?;
        let prov_id: String = prov.getattr("id")?.extract()?;
        let (items, counts) = me._items(py, ctx)?;
        let target = me.paint_target.bind(py).clone();
        drop(me);
        let has_target = !target.is_none();
        let sid: String = slf.getattr("id")?.extract()?;
        let sheet = cs(py)?.getattr("Sheet")?.call((sid, if has_target { "Choose material" } else { "" }), Some(&[("width", 1240)].into_py_dict(py)?))?;
        let ty = st.get_item("type")?;
        if !has_target {
            for p in lib(py)?.getattr("PROVIDERS")?.try_iter()? {
                let p = p?;
                let ready = p.getattr("ready")?.is_truthy()?;
                let count = if ready { counts.call_method1("get", (p.getattr("id")?, ""))? } else { s(py, "soon") };
                let kw = PyDict::new(py);
                kw.set_item("on", p.getattr("id")?.eq(&ty)?)?;
                kw.set_item("enabled", ready)?;
                sheet.call_method("tab", (p.getattr("id")?, p.getattr("label")?, count), Some(&kw))?;
            }
            sheet.call_method("tab", ("registry", "Project registry"), Some(&[("on", ty.eq("registry")?)].into_py_dict(py)?))?;
        }
        if ty.eq("registry")? {
            if slf.getattr("registry")?.is_none() {
                slf.setattr("registry", render(py)?.getattr("RegistryEditor")?.call1((ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?,))?)?;
            }
            slf.getattr("registry")?.call_method1("build", (&sheet,))?;
            return Ok(sheet);
        }
        let scene = ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?;
        let browse_d = json(py)?.call_method1("loads", (browse(&st)?,))?;
        if has_target {
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
        let history = ctx.getattr("win")?.getattr("viewport")?.getattr("history")?;
        let got = if materials { Some(survey(&scene, Some(history))?) } else { None };
        spec.set_item("model_uses", match &got { Some(g) => py.import("builtins")?.getattr("dict")?.call1((g.get_item(0)?,))?, None => PyDict::new(py).into_any() })?;
        spec.set_item("unnamed", match &got { Some(g) => g.get_item(1)?.len()?, None => 0 })?;
        let out = loads(py, &render(py)?.call_method1("library_sheet", (dumps(py, spec.as_any())?,))?)?;
        for path in out.get_item("thumbs")?.try_iter()? {
            thumb(&view, &path?)?;
        }
        st.set_item("scope", out.get_item("scope")?)?;
        let searching = out.get_item("searching")?.is_truthy()?;
        let online = out.get_item("online")?.is_truthy()?;
        sheet.call_method1("search", (out.get_item("search")?,))?;
        sheet.getattr("menus")?.call_method1("append", (out.get_item("sort")?,))?;
        sheet.setattr("side", out.get_item("side")?)?;
        if prov_id == "components" {
            let label = py.import("ingetrazo_core.i18n")?.call_method1("tr", ("Component library",))?;
            sheet.call_method("side_item", ("s:online", label), Some(&[("on", online)].into_py_dict(py)?))?;
        }
        if online || (prov_id == "components" && searching) {
            if slf.getattr("catalogue")?.is_none() {
                slf.setattr("catalogue", crate::component_catalogue::session::CatalogueSession::create(ctx)?)?;
            }
            if !searching {
                return slf.getattr("catalogue")?.call_method1("build", (&sheet, ""));
            }
        } else {
            slf.call_method0("close_catalogue")?;
        }
        let content = sheet.call_method("content", (), Some(out.get_item("content")?.cast::<PyDict>()?))?;
        for g in out.get_item("groups")?.try_iter()? {
            let g = g?;
            if g.get_item("caption")?.is_truthy()? {
                content.call_method("caption", (g.get_item("caption")?,), Some(&[("right", g.get_item("right")?)].into_py_dict(py)?))?;
            }
            content.call_method("tiles", (g.get_item("tiles")?,), Some(&[("size", 96)].into_py_dict(py)?))?;
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
        st.set_item("cursor", out.get_item("cursor")?)?;
        let detail = sheet.call_method("detail", (), Some(&[("width", 340i32.into_pyobject(py)?.into_any()), ("empty", s(py, "Pick an item to see it here."))].into_py_dict(py)?))?;
        let cursor = st.call_method1("get", ("cursor",))?;
        if has_target {
            detail.call_method1("fields", (&target, "tree-paint"))?;
            detail.call_method1("buttons", (vec![cs_verb(py, "custompaint", "Apply colour", &[])?, cs_verb(py, "activepaint", "Use active paint", &[])?, cs_verb(py, "clearpaint", "Remove paint", &[])?],))?;
            if cursor.is_truthy()? {
                let apply = cs_verb(py, "apply", "Apply material", &[("primary", true.into_pyobject(py)?.to_owned().into_any())])?;
                detail.call_method("buttons", (vec![apply],), Some(&[("align", "fill")].into_py_dict(py)?))?;
            }
        } else {
            detail.setattr("blocks", out.get_item("detail")?)?;
        }
        let e = if cursor.is_truthy()? { st.get_item("items")?.call_method1("get", (&cursor,))? } else { py.None().into_bound(py) };
        if !e.is_none() && prov_id == "components" {
            slf.call_method1("_component_detail", (&detail, &prov, ctx, &e))?;
        }
        sheet.call_method1("footer", (PyTuple::empty(py), "Click to see it · double-click or ↩ uses it · esc back to the model"))?;
        if cursor.is_truthy()? {
            let force = st.call_method1("pop", ("force_focus", false))?.is_truthy()?;
            sheet.call_method("focus", (&cursor,), Some(&[("force", force)].into_py_dict(py)?))?;
        }
        let catalogue = slf.getattr("catalogue")?;
        if catalogue.is_none() {
            Ok(sheet)
        } else {
            catalogue.call_method1("build", (&sheet, st.get_item("filter")?))
        }
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
                    let i18n = py.import("ingetrazo_core.i18n")?;
                    let line = render(py)?.getattr("ComponentCatalogue")?.call_method1("credit_line", (json(py)?.call_method1("dumps", (record,))?, i18n.call_method0("current_language")?, i18n.getattr("tr")?))?;
                    d.call_method1("text", (line,))?;
                }
            }
        }
        let mut vs = vec![cs_verb(py, "apply", "Place a copy", &[("key", s(py, "↩")), ("primary", true.into_pyobject(py)?.to_owned().into_any())])?];
        if model {
            for (id,label) in [("select","Select copies"),("replace","Replace selection with this"),("edit","Open for editing")] {
                if e.get_item("verbs")?.contains(id)? {vs.push(cs_verb(py,id,label,&[])?);}
            }
        }
        d.call_method("buttons", (vs,), Some(&[("align", "fill")].into_py_dict(py)?))?;
        if model {
            let labels = lib(py)?.getattr("VERBS")?;
            let quiet = true.into_pyobject(py)?.to_owned().into_any();
            let mut more = Vec::new();
            for v in ["rename", "keepfile", "unkeepfile", "delete"] {
                if e.get_item("verbs")?.contains(v)? {
                    let label = if v == "rename" { "Rename…".to_owned() } else { labels.get_item(v)?.str()?.to_string() };
                    more.push(cs_verb(py, v, &label, &[("quiet", quiet.clone())])?);
                }
            }
            d.call_method("buttons", (more,), Some(&[("align", "left")].into_py_dict(py)?))?;
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
        py.import("ingetrazo_render.chrome_forms")?.getattr("ask_text")?.call((ctx.getattr("win")?, title, "Name:"), Some(&kw))?;
        Ok(())
    }

    fn act(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, verb: &str, args: Vec<String>) -> PyResult<()> {
        let py = slf.py();
        let view = slf.getattr("view")?;
        let win = ctx.getattr("win")?;
        let st = _state(&view)?;
        let arg = args.first().cloned().unwrap_or_default();
        let lctx = ctx_of(py, &win, &view)?;
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
        let registry = slf.getattr("registry")?;
        if verb == "verb" && !registry.is_none() && st.get_item("type")?.eq("registry")? {
            let mut path = s(py, "");
            if arg == "registry_csv" {
                path = py.import("ingetrazo_render.shell_files")?.getattr("ShellFileDialog")?.call_method1("getOpenFileName", (&win, "Codes CSV", "", "CSV (*.csv)"))?.get_item(0)?;
            }
            if registry.call_method1("act", (&win, &arg, path))?.is_truthy()? && arg == "registry_apply" {
                cs(py)?.call_method1("close", (&win,))?;
            }
            return Ok(());
        }
        match verb {
            "search" => st.set_item("filter", &arg)?,
            "tab" => {
                let mut ok = arg == "registry";
                for p in lib(py)?.getattr("PROVIDERS")?.try_iter()? {
                    let p = p?;
                    ok |= p.getattr("id")?.eq(&arg)? && p.getattr("ready")?.is_truthy()?;
                }
                if ok {
                    st.set_item("type", &arg)?;
                    st.set_item("scope", "s:model")?;
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
                if let Some(facet) = opt_.strip_prefix("facet:") {
                    st.set_item("facet", if matches!(facet, "all" | "colour" | "image") { facet } else { "all" })?;
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
                if let Some(action) = arg.strip_prefix("typeact:") {
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
        slf.setattr("registry", slf.py().None())?;
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
    let items = if ui(py)?.call_method1("_active_tool_id", (win,))?.eq("paint")? { palette_items(view, win)? } else { PyList::empty(py) };
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
        view.setattr("_ui_lens", 2)?;
        ui(py)?.call_method1("apply_dock_width", (view,))?;
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
    let uses = material_uses(&scene, Some(win.getattr("viewport")?.getattr("history")?))?;
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
    for r in py.import("ingetrazo_core")?.call_method1("recent_swatches", (paint_tool(py)?,))?.try_iter()? {
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
        py.import("ingetrazo_core")?.call_method1("pick_recent", (win.getattr("viewport")?.getattr("scene")?, paint_tool(py)?, loads(py, &s(py, stamp))?))?;
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
        return py.import("ingetrazo_core")?.call_method1("brush_inspector", (paint_tool(py)?,));
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
    py.import("ingetrazo_core")?.call_method1("refresh_library_paint", (win.getattr("viewport")?.getattr("scene")?, paint_tool(py)?))?;
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
    sub.add("ROW_CAP", 600)?;
    sub.add("THUMB_PX", THUMB_PX)?;
    sub.add("STRIP_MODEL", STRIP_MODEL)?;
    sub.add("STRIP_RECENT", STRIP_RECENT)?;
    sub.add("PBR_ROWS", PBR_ROWS.to_vec())?;
    sub.add("_bundled_memo", PyDict::new(py))?;
    sub.add_class::<Ctx>()?;
    sub.add_class::<LibraryProvider>()?;
    sub.add_class::<MaterialsProvider>()?;
    sub.add_class::<ComponentsProvider>()?;
    sub.add_class::<ProfilesProvider>()?;
    sub.add_class::<LibrarySheet>()?;
    sub.add("PROVIDERS", vec![
        Bound::new(py, MaterialsProvider::new())?.into_any(),
        Bound::new(py, ComponentsProvider::new())?.into_any(),
        Bound::new(py, ProfilesProvider::new())?.into_any(),
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
    m.add_submodule(&sub)?;
    py.import("sys")?.getattr("modules")?.set_item("ingetrazo_render.wgpu_library", &sub)?;
    Ok(())
}
