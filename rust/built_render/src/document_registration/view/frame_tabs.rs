// SPDX-License-Identifier: GPL-3.0-or-later
//! Project tabs in the chrome's top bar (`views.wgpu_tabs`; shell design
//! option B): what the strip shows of the window's documents, its verbs,
//! and the link-source tabs (render-glue lane; from `views/wgpu_tabs.py`).
//!
//! The strip itself is `ui/tabs.rs`; the per-frame record of what was sent
//! is `render_glue/document_tabs.rs`.
use super::*;
use pyo3::exceptions::{PyException, PyOSError, PyValueError};
use pyo3::types::PyCFunction;

fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    optional(o, name)
}
fn id(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}
fn tabs_module<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_tabs")
}
fn path_of<'py>(py: Python<'py>, value: impl IntoPyObject<'py>) -> Reply<'py> {
    py.import("pathlib")?.getattr("Path")?.call1((value,))
}
/// `value`, or `fallback` where Python raised an `Exception`.
fn or_else<'py>(py: Python<'py>, value: Reply<'py>, fallback: impl FnOnce() -> Reply<'py>) -> Reply<'py> {
    match value {
        Err(error) if error.is_instance_of::<PyException>(py) => fallback(),
        other => other,
    }
}
/// The app's other windows (the native app), when there is such a module.
fn windows<'py>(py: Python<'py>) -> Reply<'py> {
    py.import("built_render")?.getattr("shell_windows")
}
/// Whether `doc` is one of `docs` (`in`: by identity, then equality).
fn among(docs: &Obj<'_>, doc: &Obj<'_>) -> PyResult<bool> {
    docs.contains(doc)
}

pub(super) fn docs<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let documents = attr(&a.get_item(0)?, "documents")?;
    if documents.is_none() { Ok(PyList::empty(py).into_any()) } else { documents.call0() }
}

pub(super) fn dirty<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let check = attr(&a.get_item(0)?, "_doc_is_dirty")?;
    if check.is_none() {
        return false.into_bound_py_any(py);
    }
    let asked = check.call1((a.get_item(1)?,)).and_then(|dirty| dirty.is_truthy()).and_then(|dirty| dirty.into_bound_py_any(py));
    or_else(py, asked, || false.into_bound_py_any(py))
}

/// `(host document, placements)` when `doc`'s file is linked into another
/// project open in this window, else `(None, 0)`. The host the source was
/// opened from wins ("Open source").
pub(super) fn link_host<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, doc, given) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let nothing = || (py.None(), 0).into_bound_py_any(py);
    let path = doc.getattr("path")?;
    if path.is_none() {
        return nothing();
    }
    let links = py.import("built_core.links")?;
    let docs = if given.is_none() { direct(py, docs, (&win,))? } else { given };
    let preferred = attr(&doc, "link_host")?;
    let mut order = Vec::new();
    if among(&docs, &preferred)? {
        order.push(preferred.clone());
    }
    for other in docs.try_iter()? {
        let other = other?;
        if !other.is(&preferred) {
            order.push(other);
        }
    }
    for host in order {
        let viewport = host.getattr("viewport")?;
        if host.is(&doc) || viewport.is_none() {
            continue;
        }
        let count = viewport.getattr("scene").and_then(|scene| links.call_method1("linked_groups", (scene, &path))).and_then(|groups| groups.len());
        let count = match count {
            Ok(count) => count,
            Err(error) if error.is_instance_of::<PyException>(py) => 0,
            Err(error) => return Err(error),
        };
        if count != 0 {
            return (host, count).into_bound_py_any(py);
        }
    }
    nothing()
}

pub(super) fn saved_text<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let time = py.import("time")?;
    let modified = match path_of(py, a.get_item(0)?)?.call_method0("stat") {
        Ok(stat) => stat.getattr("st_mtime")?,
        Err(error) if error.is_instance_of::<PyOSError>(py) => return "".into_bound_py_any(py),
        Err(error) => return Err(error),
    };
    let age: f64 = time.call_method0("time")?.sub(&modified)?.extract()?;
    let format = if age < 20. * 3600. { "saved %H:%M" } else { "saved %d %b" };
    time.call_method1("strftime", (format, time.call_method1("localtime", (modified,))?))
}

pub(super) fn tip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, doc, host, count) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let mut lines = Vec::new();
    let path = doc.getattr("path")?;
    if path.is_none() {
        lines.push("Not saved yet".to_owned());
    } else {
        let home: String = py.import("pathlib")?.getattr("Path")?.call_method0("home")?.str()?.extract()?;
        let place: String = path.str()?.extract()?;
        let place = match place.strip_prefix(&home) {
            Some(rest) => format!("~{rest}"),
            None => place,
        };
        let saved: String = direct(py, saved_text, (&path,))?.extract()?;
        lines.push(if saved.is_empty() { place } else { format!("{place} · {saved}") });
    }
    if !host.is_none() {
        lines.push(format!("linked in {} · {} placement{}", host.call_method0("display_name")?.str()?, count.str()?, if count.eq(1)? { "" } else { "s" }));
    }
    if direct(py, dirty, (&win, &doc))?.is_truthy()? {
        lines.push("Unsaved changes".to_owned());
    }
    lines.join("\n").into_bound_py_any(py)
}

fn pictures_on<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let stored = py.import("built_core.settings")?.call_method0("settings")?.call_method1("value", ("tabs/hover_picture", "1"))?;
    (stored.str()?.to_str()? != "0").into_bound_py_any(py)
}

/// The strip's tabs: (id, name, front, unsaved, link source, tip, optional
/// external picture key), in the window's order. `docs`: the window's
/// documents, when the caller already read them.
fn tabs<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, given) = (a.get_item(0)?, a.get_item(1)?);
    let docs = if given.is_none() { direct(py, docs, (&win,))? } else { py.get_type::<PyList>().call1((given,))? };
    let front = attr(&win, "_doc")?;
    let out = PyList::empty(py);
    for doc in docs.try_iter()? {
        let doc = doc?;
        let [host, count] = unpack::<2>(&direct(py, link_host, (&win, &doc, &docs))?)?;
        out.append((
            doc.getattr("tab_id")?,
            doc.call_method0("display_name")?,
            doc.is(&front),
            direct(py, dirty, (&win, &doc))?,
            !host.is_none(),
            direct(py, tip, (&win, &doc, &host, count))?,
            "",
        ))?;
    }
    Ok(out.into_any())
}

/// The projects in the app's other windows (the native app): (id, name,
/// window).
fn elsewhere<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    match or_else(py, windows(py), || Ok(none(py)))? {
        module if module.is_none() => Ok(PyList::empty(py).into_any()),
        module => module.call_method1("elsewhere", (a.get_item(0)?,)),
    }
}

fn can_move<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    match or_else(py, windows(py), || Ok(none(py)))? {
        module if module.is_none() => false.into_bound_py_any(py),
        module => module.call_method1("can_move", (a.get_item(0)?,)),
    }
}

/// The renderer's record of what its document ▾ and tab strip show
/// (`built_render.DocumentTabs`): `.sync(renderer, win, left)` sends
/// what changed, once a frame. A stand-in renderer without one (tests)
/// leaves the record on the view.
pub(super) fn document_tabs<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let record = py.import("built_render")?.getattr("DocumentTabs")?;
    let own = attr(&attr(&view, "renderer")?, "document_tabs")?;
    let state = if own.is_callable() { own.call0()? } else { none(py) };
    if state.is_instance(&record)? {
        return Ok(state);
    }
    let kept = view.getattr("__dict__")?.call_method1("get", ("_ui_document_tabs",))?;
    if !kept.is_none() {
        return Ok(kept);
    }
    let state = record.call0()?;
    view.setattr("_ui_document_tabs", &state)?;
    Ok(state)
}

pub(super) fn close<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, doc) = (a.get_item(0)?, a.get_item(1)?);
    let own = attr(&win, "close_document")?;
    let closed = if own.is_none() { win.call_method1("run_command", ("close_tab", doc))? } else { own.call1((doc,))? };
    closed.is_truthy()?.into_bound_py_any(py)
}

pub(super) fn activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let (win, doc) = (a.get_item(0)?, a.get_item(1)?);
    if !doc.is(&attr(&win, "_doc")?) {
        win.call_method1("activate_document", (doc,))?;
    }
    Ok(none(a.py()))
}

/// `ptab:<arg>` from the strip.
fn action<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, view) = (a.get_item(0)?, a.get_item(1)?);
    let arg: String = a.get_item(2)?.extract()?;
    let module = tabs_module(py)?;
    let docs = direct(py, docs, (&win,))?;
    let (verb, rest) = arg.split_once(':').unwrap_or((&arg, ""));
    let int = py.get_type::<pyo3::types::PyInt>();
    let doc_at = |text: &str| -> PyResult<Option<Obj<'py>>> {
        let index: i64 = match int.call1((text,)) {
            Ok(index) => index.extract()?,
            Err(error) if error.is_instance_of::<PyValueError>(py) => return Ok(None),
            Err(error) => return Err(error),
        };
        if 0 <= index && (index as usize) < docs.len()? { Ok(Some(docs.get_item(index)?)) } else { Ok(None) }
    };
    match verb {
        "activate" => {
            if let Some(doc) = doc_at(rest)? {
                direct(py, activate, (&win, doc))?;
            }
        }
        "close" => {
            if let Some(doc) = doc_at(rest)? {
                direct(py, close, (&win, doc))?;
            }
        }
        "new" => {
            win.call_method0("_on_new_tab")?;
        }
        "goto" => match or_else(py, windows(py), || Ok(none(py)))? {
            module if module.is_none() => return Ok(none(py)),
            module => {
                module.call_method1("goto", (&win, rest))?;
            }
        },
        "menu" => {
            let (what, index) = rest.split_once(':').unwrap_or((rest, ""));
            if doc_at(index)?.is_some() {
                py.import("built_render")?.call_method1("project_tab_menu", (&win, what, int.call1((index,))?))?;
            }
        }
        _ => {}
    }
    if !view.is_none() {
        module.call_method1("document_tabs", (&view,))?.call_method0("stale")?;
        view.call_method0("update")?;
    }
    Ok(none(py))
}

fn source_context<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let win = a.get_item(0)?;
    let nothing = || (py.None(), py.None()).into_bound_py_any(py);
    let doc = attr(&win, "_doc")?;
    let host = attr(&doc, "link_host")?;
    if host.is_none() || !among(&direct(py, docs, (&win,))?, &host)? {
        return nothing();
    }
    let scene = host.getattr("viewport")?.getattr("scene")?;
    let address = attr(&doc, "link_mount")?;
    let mount = if address.is_truthy()? { scene.call_method1("resolve_occurrence", (address,))? } else { none(py) };
    if mount.is_none() {
        return nothing();
    }
    let record = py.import("built_core.links")?.call_method1("link_of", (&mount,))?;
    let linked = if record.is_truthy()? { record.call_method1("get", ("path", ""))? } else { "".into_bound_py_any(py)? };
    if path_of(py, linked)?.ne(doc.getattr("path")?)? {
        return nothing();
    }
    (host, mount).into_bound_py_any(py)
}

fn sync_host_context<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let module = tabs_module(py)?;
    let doc = attr(&win, "_doc")?;
    let host = attr(&doc, "link_host")?;
    let scene = if !host.is_none() && among(&direct(py, docs, (&win,))?, &host)? { host.getattr("viewport")?.getattr("scene")? } else { none(py) };
    let origin = match attr(&view, "packed")?.getattr_opt("origin")? {
        Some(origin) => py.get_type::<PyTuple>().call1((origin,))?,
        None => (0., 0., 0.).into_bound_py_any(py)?,
    };
    let sig = if scene.is_none() {
        PyTuple::empty(py).into_any()
    } else {
        (id(&scene), scene.getattr("content_version")?, host.getattr("viewport")?.getattr("history")?.getattr("revision")?, &origin).into_bound_py_any(py)?
    };
    if view.getattr("__dict__")?.call_method1("get", ("_ui_host_sig",))?.eq(&sig)? {
        return Ok(none(py));
    }
    let [host, mount] = unpack::<2>(&module.call_method1("source_context", (&win,))?)?;
    let name = if host.is_truthy()? { host.call_method0("display_name")? } else { "".into_bound_py_any(py)? };
    let shown = if mount.is_truthy()? { scene } else { none(py) };
    view.getattr("renderer")?.call_method1("ui_set_host_context", (name, origin, shown, mount))?;
    view.setattr("_ui_host_sig", sig)?;
    Ok(none(py))
}

/// "Open source": the linked file in a tab of its own, right of the
/// project it is linked into (a ⛓ tab), or its tab brought to the front.
pub(super) fn open_link_source<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, path, mount) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let host = attr(&win, "_doc")?;
    let file = path_of(py, &path)?;
    let opened = win.call_method1("open_path", (&file,))?.is_truthy()?;
    if opened {
        // The open keeps this until the document is there: it holds the
        // window and the host weakly, and the placement by its address.
        let weak = py.import("weakref")?.getattr("ref")?;
        let (held_win, held_path) = (weak.call1((&win,))?.unbind(), path.clone().unbind());
        let held_host = if host.is_none() { None } else { Some(weak.call1((&host,))?.unbind()) };
        let address = if mount.is_truthy()? { Some(host.getattr("viewport")?.getattr("scene")?.call_method1("occurrence_path", (&mount,))?.unbind()) } else { None };
        let bind = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
            let py = args.py();
            let doc = args.get_item(0)?;
            let win = held_win.bind(py).call0()?;
            let host = match &held_host {
                Some(host) => host.bind(py).call0()?,
                None => py.None().into_bound(py),
            };
            if win.is_none() || host.is_none() || doc.is(&host) || !among(&direct(py, docs, (&win,))?, &host)? {
                return Ok(());
            }
            let scene = host.getattr("viewport")?.getattr("scene")?;
            let mount = match &address {
                Some(address) => address.bind(py).clone(),
                None => match py.import("built_core.links")?.call_method1("linked_groups", (&scene, held_path.bind(py)))?.try_iter()?.next() {
                    Some(source) => scene.call_method1("occurrence_path", (source?,))?,
                    None => py.None().into_bound(py),
                },
            };
            doc.setattr("link_host", &host)?;
            doc.setattr("link_mount", mount)?;
            let view = optional(&win, "view")?;
            if !view.is_none() {
                view.setattr("_ui_host_sig", py.None())?;
            }
            Ok(())
        })?;
        py.import("built_render")?.getattr("document_open")?.call_method1("after_open", (&win, file, bind))?;
    }
    opened.into_bound_py_any(py)
}

pub(super) fn install_tabs(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add(
        "__doc__",
        "Project tabs in the chrome's top bar (shell design option B).\n\nThe tab strip sits beside the window buttons, as in Figma. This module sends\nthe tabs, and ``rust/built_render/src/ui/tabs.rs`` draws them and reports\nclicks. Document switching, closing and creation use the window commands.\n\n- A tab is one open project. ``•`` means unsaved. The tab in front has the\n  accent line.\n- A link source is a ⛓ tab: an open project whose file another open\n  project links. It opens right of its host (\"Open source\" in the tree, the\n  inspector or the right-click menu), and its tip names the host and\n  counts the placements.\n- A background tab can show its last model frame. The renderer owns the\n  tab textures and reads the hover preference with this strip's payload.\n- Scenes are never tabs: they keep the ‹ Scene ▾ › control.\n",
    )?;
    module.add("time", py.import("time")?)?;
    modules::import_names(module, "pathlib", &[("Path", "Path")])?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("pictures_on", Mode::Function, pictures_on, &[], "", ""),
            ("tabs", Mode::Function, tabs, &["win", "docs=None"], "", ""),
            ("_elsewhere", Mode::Function, elsewhere, &["win"], "", ""),
            ("_can_move", Mode::Function, can_move, &["win"], "", ""),
            ("document_tabs", Mode::Function, document_tabs, &["view"], "", ""),
            ("action", Mode::Function, action, &["win", "view", "arg"], "", ""),
            ("source_context", Mode::Function, source_context, &["win"], "", ""),
            ("sync_host_context", Mode::Function, sync_host_context, &["view", "win"], "", ""),
            ("open_link_source", Mode::Function, open_link_source, &["win", "path", "mount=None"], "", ""),
        ],
    )
}
