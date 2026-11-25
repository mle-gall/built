// SPDX-License-Identifier: GPL-3.0-or-later
//! `built_core.links`, served from here (`core/links.py` is gone): linked files,
//! another .igz placed in this one as a live reference.
//!
//! A linked file is a component whose definition is not the host's: it is
//! the source document, loaded from disk. The source stays a document of
//! its own. Open it, edit it, save it, and every host that links it shows
//! the change (the window watches the files). Inside the host the linked
//! part moves, turns and scales like any component, but it cannot be
//! opened for editing there: entering it offers to open the source.
//!
//! **What is stored where.**
//! - Each placed copy carries `Group.ext["link"] = {"path": …}`, the
//!   source as an absolute path, and `"rel"`, the same path relative to
//!   the host when it was last saved. `"rel"` is tried first, so a project
//!   folder can move as a whole.
//! - The host's links registry (`scene.link_registry`, a root table of the
//!   document; the file stores it as `plugin_data["links"]`) keeps, per
//!   source, the tag overrides and whether the source is *packed*.
//! - Unpacked (the default), the host file does not hold the source's
//!   geometry. The copies are saved as empty placements and filled in again
//!   from the source when the host is opened (`link_resolve`).
//! - Packed (Blender's «Pack external data»), the geometry is saved in the
//!   host like any component's, and the source is no longer read.
//!   Unpacking links it again.
//!
//! **Tags.** The source's tags come into the host under their own names
//! prefixed by the file's («Site / Wood»), so they never mix with the
//! host's tags of the same name. They sit in a folder named after the file
//! inside «Linked files», where the source's own folders are rebuilt. Each
//! tag shows as the source has it, unless it was switched in the host.
//! That switch is an *override*: the host keeps it and never writes it to
//! the source, and the source's later changes to that tag no longer reach
//! it.
//!
//! The logic is `links.rs` (the queries, the registry, the tags) and
//! `link_io.rs` (read, make, reload, resolve). This module serves the names
//! plugins use, the constants, `LinkError` and the undoable registry step.

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyDict;

use super::module;
use super::py_class::{class, method};

type Obj<'py> = Bound<'py, PyAny>;

const MODULE: &str = "built_core.links";

static LINK_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static SET_REGISTRY: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// `json.loads(json.dumps(v))`: the registry copied as
/// `SetPluginDataCommand` copies.
fn json_copy<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let json = module(py, "json")?;
    json.call_method1(intern!(py, "loads"), (json.call_method1(intern!(py, "dumps"), (v,))?,))
}

#[pyfunction]
fn reg_init<'py>(me: &Obj<'py>, value: &Obj<'py>) -> PyResult<()> {
    me.setattr(intern!(me.py(), "value"), json_copy(value)?)?;
    me.setattr(intern!(me.py(), "_before"), me.py().None())
}

fn bump(scene: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    scene.setattr(intern!(py, "version"), scene.getattr(intern!(py, "version"))?.add(1)?)
}

#[pyfunction]
fn reg_do<'py>(me: &Obj<'py>, scene: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    me.setattr(intern!(py, "_before"), json_copy(&scene.getattr(intern!(py, "link_registry"))?)?)?;
    scene.setattr(intern!(py, "link_registry"), json_copy(&me.getattr(intern!(py, "value"))?)?)?;
    bump(scene)
}

#[pyfunction]
fn reg_undo<'py>(me: &Obj<'py>, scene: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    scene.setattr(intern!(py, "link_registry"), json_copy(&me.getattr(intern!(py, "_before"))?)?)?;
    bump(scene)
}

/// An undoable step that replaces the links registry with `value` (a copy,
/// through JSON both ways, as `SetPluginDataCommand` copies). The registry
/// is a document value: the step's transaction undoes it.
#[pyfunction]
fn set_registry_command<'py>(value: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = value.py();
    let cls = SET_REGISTRY.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        let m = module(py, MODULE)?.cast_into::<PyModule>()?;
        let ns = PyDict::new(py);
        ns.set_item("doc_undo", true)?;
        method(&ns, "__init__", wrap_pyfunction!(reg_init, &m)?)?;
        method(&ns, "do", wrap_pyfunction!(reg_do, &m)?)?;
        method(&ns, "undo", wrap_pyfunction!(reg_undo, &m)?)?;
        let command = py.import("built_core.history")?.getattr("Command")?;
        Ok(class(py, MODULE, "SetLinkRegistryCommand", &[command], &ns)?.unbind())
    })?;
    cls.bind(py).call1((value,))
}

/// Fill the module `built_core.links`.
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__doc__", "Linked files: another .igz placed in this one as a live reference (rust/built_core/src/model/links_api.rs).")?;
    let rs = module(py, "built_core")?;
    // The plugin surface (docs/model-port.md).
    for (name, native) in [
        ("link_of", "link_of"),
        ("fill_link", "link_fill"),
        ("is_linked", "link_is_linked"),
        ("linked_groups", "link_linked_groups"),
        ("sources", "link_sources"),
        ("link_state", "link_state"),
        ("make_link", "link_make"),
        ("reload", "link_reload"),
    ] {
        m.add(name, rs.getattr(native)?)?;
    }
    m.add("LINK_KEY", "link")?; // Group.ext key on each placed copy
    m.add("REGISTRY_KEY", "links")?; // the file's plugin_data key for the registry
    m.add("LINKED_FOLDER", "Linked files")?; // the tag folder all links' folders sit in
    m.add("SEP", " / ")?; // between the file's name and a tag's
    let err = LINK_ERROR.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        let ns = PyDict::new(py);
        ns.set_item("__doc__", "A source that cannot be linked: missing, unreadable, or a cycle.")?;
        let base = py.import("builtins")?.getattr("Exception")?;
        Ok(class(py, MODULE, "LinkError", &[base], &ns)?.unbind())
    })?;
    m.add("LinkError", err.bind(py))?;
    m.add_function(wrap_pyfunction!(set_registry_command, m)?)?;
    Ok(())
}
