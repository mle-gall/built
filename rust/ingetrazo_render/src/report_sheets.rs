// SPDX-License-Identifier: GPL-3.0-or-later
//! The About, Extensions and Cut list sheets on the window: providers of the
//! large sheet (`sheet_runtime.rs`) over their Rust view models
//! (`about.rs`, `extensions.rs`, `surfaces.rs`).
//!
//! What stays Python is what each one reports on or acts through: the
//! extension installer and discovery (`ingetrazo_core.extension_manager`,
//! `core.extensions`), the parts of an assembly (`ingetrazo_core.parts`) and the
//! document's units, history and selection.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};

fn opt<'py>(o: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    o.getattr(name).ok().filter(|v| !v.is_none())
}

/// Whether this is the `built` binary (`chrome_sheet.in_shell`, which
/// tests replace).
fn in_shell(py: Python<'_>) -> PyResult<bool> {
    py.import("ingetrazo_render.chrome_sheet")?.call_method0("in_shell")?.is_truthy()
}

fn json_dumps<'py>(py: Python<'py>, v: &Bound<'py, PyAny>) -> PyResult<String> {
    py.import("json")?.call_method1("dumps", (v,))?.extract()
}

fn json_loads<'py>(py: Python<'py>, s: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("json")?.call_method1("loads", (s,))
}

/// A sheet whose spec a Rust view model built, for the sheet host.
fn spec_sheet<'py>(py: Python<'py>, spec: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("ingetrazo_render.chrome_sheet")?.getattr("Spec")?.call1((json_loads(py, spec)?,))
}

fn tr(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("ingetrazo_core.i18n")?.getattr("tr")
}

/// Put `text` on the clipboard (the system pasteboard; false where there
/// is none, as headless).
pub fn copy_text(py: Python<'_>, text: &str) -> PyResult<bool> {
    let item = ("public.utf8-plain-text", PyBytes::new(py, text.as_bytes()));
    py.import("ingetrazo_render.shell_services")?.call_method1("pasteboard_write", (vec![item],))?.is_truthy()
}

// ---- CSV (Python's csv module, QUOTE_MINIMAL) ------------------------------------------

/// One table as CSV: `,` (the excel dialect) or tab (excel-tab), `\r\n`
/// line ends, fields quoted when they hold the delimiter, a quote or a line
/// break, quotes doubled.
pub fn csv(rows: &[Vec<String>], delimiter: char) -> String {
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .map(|cell| {
                if cell.contains(delimiter) || cell.contains('"') || cell.contains('\n') || cell.contains('\r') {
                    format!("\"{}\"", cell.replace('"', "\"\""))
                } else {
                    cell.clone()
                }
            })
            .collect();
        out.push_str(&cells.join(&delimiter.to_string()));
        out.push_str("\r\n");
    }
    out
}

fn cells(table: &Bound<'_, PyAny>) -> PyResult<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    for row in table.try_iter()? {
        let mut out = Vec::new();
        for cell in row?.try_iter()? {
            out.push(cell?.str()?.to_string());
        }
        rows.push(out);
    }
    Ok(rows)
}

// ---- About ----------------------------------------------------------------------------

/// What About says about this copy: version, OS, runtime, window and engine,
/// for `AboutReview`.
#[pyfunction]
#[pyo3(signature = (version, gl_line="", host=None, renderer=None))]
pub fn about_review<'py>(
    py: Python<'py>,
    version: &str,
    gl_line: &str,
    host: Option<Bound<'py, PyAny>>,
    renderer: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let platform = py.import("platform")?;
    let sys = py.import("sys")?;
    let environ = py.import("os")?.getattr("environ")?;
    let call = |name: &str| -> PyResult<String> { platform.call_method0(name)?.extract() };
    let system = call("system")?;
    let mut pretty = String::new();
    if system == "Linux" {
        if let Ok(release) = platform.call_method0("freedesktop_os_release") {
            pretty = release.call_method1("get", ("PRETTY_NAME", ""))?.extract()?;
        }
    }
    let shell = in_shell(py)?;
    // The window's size as Python has it (ints print as 1280, not 1280.0).
    let mut size: Option<Bound<'py, PyAny>> = None;
    if let Some(host) = &host {
        size = Some(PyList::new(py, [host.getattr("width")?, host.getattr("height")?, host.getattr("scale")?])?.into_any());
    }
    let has_env = |k: &str| environ.call_method1("get", (k,)).and_then(|v| v.is_truthy()).unwrap_or(false);
    let frozen = opt(&sys, "frozen").is_some_and(|v| v.is_truthy().unwrap_or(false));
    let d = PyDict::new(py);
    d.set_item("version", version)?;
    d.set_item("flatpak", has_env("FLATPAK_ID"))?;
    d.set_item("appimage", has_env("APPIMAGE"))?;
    d.set_item("packaged", frozen || (shell && sys.hasattr("_MEIPASS")?))?;
    d.set_item("system", &system)?;
    d.set_item("release", call("release")?)?;
    d.set_item("machine", call("machine")?)?;
    d.set_item("pretty", pretty)?;
    let os_version: String = if system == "Darwin" { platform.call_method0("mac_ver")?.get_item(0)?.extract()? } else { call("version")? };
    d.set_item("os_version", os_version)?;
    d.set_item("python", call("python_version")?)?;
    d.set_item("gl", gl_line)?;
    let backend = renderer.as_ref().and_then(|r| opt(r, "backend_label"));
    d.set_item("renderer", match backend {
        Some(b) => b,
        None => "wgpu".into_pyobject(py)?.into_any(),
    })?;
    d.set_item("size", size)?;
    d.set_item("native", host.is_some())?;
    d.set_item("language", py.import("ingetrazo_core.i18n")?.call_method0("current_language")?)?;
    d.set_item("engine", py.import("core._native")?.call_method0("engine")?)?;
    d.set_item("commit", env!("BUILT_COMMIT"))?;
    let review = py.import("ingetrazo_render")?.getattr("AboutReview")?;
    review.call1((json_dumps(py, d.as_any())?,))
}

/// Help ▸ About.
#[pyclass(module = "ingetrazo_render")]
pub struct AboutSheet {
    #[pyo3(get, set)]
    review: Option<Py<PyAny>>,
}

/// After a copy, the button says so for a moment; then the sheet rebuilds.
#[pyclass(module = "ingetrazo_render")]
struct AboutCopied {
    win: Py<PyAny>,
    sheet: Py<AboutSheet>,
}

#[pymethods]
impl AboutCopied {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let sheets = py.import("ingetrazo_render.chrome_sheet")?;
        let win = self.win.bind(py);
        if sheets.call_method1("current", (win,))?.is(self.sheet.bind(py)) {
            sheets.call_method1("refresh", (win,))?;
        }
        Ok(())
    }
}

#[pymethods]
impl AboutSheet {
    #[new]
    fn new() -> Self {
        Self { review: None }
    }

    #[getter]
    fn id(&self) -> &'static str {
        "about"
    }

    fn build<'py>(&mut self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        if self.review.is_none() {
            let version: String = py.import("ingetrazo_core.version")?.getattr("__version__")?.extract()?;
            let host = if in_shell(py)? { opt(&ctx.getattr("win")?, "_host") } else { None };
            let renderer = opt(ctx, "view").and_then(|v| opt(&v, "renderer"));
            self.review = Some(about_review(py, &version, "", host, renderer)?.unbind());
        }
        let spec: String = self.review.as_ref().expect("made above").bind(py).call_method1("sheet", (tr(py)?,))?.extract()?;
        spec_sheet(py, &spec)
    }

    fn signature(&self, py: Python<'_>, _ctx: &Bound<'_, PyAny>) -> PyResult<bool> {
        match &self.review {
            Some(r) => r.bind(py).call_method0("copied")?.is_truthy(),
            None => Ok(false),
        }
    }

    fn act(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, verb: &str, args: Vec<String>) -> PyResult<()> {
        let py = slf.py();
        let Some(review) = slf.borrow().review.as_ref().map(|r| r.clone_ref(py)) else { return Ok(()) };
        let (Some(first), "verb") = (args.first(), verb) else { return Ok(()) };
        let review = review.bind(py);
        let action = review.call_method1("action", (first,))?;
        if action.is_none() {
            return Ok(());
        }
        let (kind, value): (String, String) = action.extract()?;
        if kind == "copy" {
            if !copy_text(py, &value)? {
                return Ok(());
            }
            review.call_method0("acknowledge")?;
            let win = ctx.getattr("win")?;
            let later = Py::new(py, AboutCopied { win: win.clone().unbind(), sheet: slf.clone().unbind() })?;
            py.import("ingetrazo_core.timers")?.call_method1("later", (&win, 1510, later))?;
        } else if kind == "open" {
            py.import("webbrowser")?.call_method1("open", (value,))?;
        }
        Ok(())
    }

    fn closed(&self, _ctx: &Bound<'_, PyAny>) {}
}

// ---- Cut list -------------------------------------------------------------------------

/// The assembly the inspector and the cut list report on: the one selected
/// group with children, else the group open for editing.
pub fn assembly_of<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = scene.py();
    let group_cls = py.import("core.group")?.getattr("Group")?;
    let sel: Vec<Bound<'py, PyAny>> = scene.getattr("selection")?.try_iter()?.collect::<PyResult<_>>()?;
    let group = if sel.len() == 1 && sel[0].is_instance(&group_cls)? { Some(sel[0].clone()) } else { None };
    let has_children = |g: &Bound<'py, PyAny>| opt(g, "children").is_some_and(|c| c.is_truthy().unwrap_or(false));
    if let Some(g) = group.as_ref().filter(|g| has_children(g)) {
        return Ok(Some(g.clone()));
    }
    let editing = opt(scene, "edit_group").filter(has_children);
    Ok(editing.or(group))
}

#[pyfunction]
fn selected_assembly<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    assembly_of(scene)
}

/// The cut list of the selected assembly: its parts, grouped by size and
/// optionally by material; Copy, CSV, and picking a row selects its parts.
#[pyclass(module = "ingetrazo_render")]
pub struct CutListSheet {
    #[pyo3(get, set)]
    match_material: bool,
    #[pyo3(get, set)]
    selected: String,
    built_for: Option<(usize, i64, usize)>,
    #[pyo3(get)]
    assembly: Option<Py<PyAny>>,
    #[pyo3(get)]
    members: Vec<Vec<String>>,
    #[pyo3(get)]
    table: Vec<Vec<String>>,
}

fn identity(o: Option<&Bound<'_, PyAny>>) -> usize {
    o.map_or(0, |o| o.as_ptr() as usize)
}

impl CutListSheet {
    fn sig(&self, ctx: &Bound<'_, PyAny>) -> PyResult<(usize, i64, usize)> {
        let scene = ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?;
        let version: i64 = scene.getattr("content_version")?.extract()?;
        Ok((scene.as_ptr() as usize, version, identity(assembly_of(&scene)?.as_ref())))
    }
}

#[pymethods]
impl CutListSheet {
    #[new]
    fn new() -> Self {
        Self { match_material: true, selected: String::new(), built_for: None, assembly: None, members: Vec::new(), table: Vec::new() }
    }

    #[getter]
    fn id(&self) -> &'static str {
        "cutlist"
    }

    fn signature(&self, ctx: &Bound<'_, PyAny>) -> PyResult<(usize, i64, usize)> {
        self.sig(ctx)
    }

    fn build<'py>(&mut self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let scene = ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?;
        let sig = self.sig(ctx)?;
        if self.built_for != Some(sig) {
            self.selected.clear();
        }
        let assembly = assembly_of(&scene)?;
        self.built_for = Some(sig);
        let fine = py.import("core.units")?.getattr("fmt_len_fine")?;
        let parts = PyList::empty(py);
        let rows = py.import("ingetrazo_core.parts")?.call_method1("part_rows", (assembly.as_ref().map_or_else(|| py.None().into_bound(py), |a| a.clone()),))?;
        for r in rows.try_iter()? {
            let r = r?;
            let part = PyDict::new(py);
            part.set_item("uid", r.get_item("part")?.getattr("uid")?.str()?)?;
            let name = r.get_item("name")?;
            part.set_item("name", if name.is_truthy()? { name } else { "Part".into_pyobject(py)?.into_any() })?;
            part.set_item("material", r.get_item("material")?)?;
            let size = r.get_item("size")?;
            let dims = PyList::empty(py);
            for v in size.try_iter()? {
                dims.append(fine.call1((v?,))?)?;
            }
            part.set_item("size", size)?;
            part.set_item("dimensions", dims)?;
            parts.append(part)?;
        }
        let name = assembly.as_ref().and_then(|a| opt(a, "name")).filter(|n| n.is_truthy().unwrap_or(false));
        let snapshot = PyDict::new(py);
        snapshot.set_item("assembly", match name {
            Some(n) => n,
            None => "Parts".into_pyobject(py)?.into_any(),
        })?;
        snapshot.set_item("revision", scene.getattr("content_version")?)?;
        snapshot.set_item("parts", parts)?;
        snapshot.set_item("selected", &self.selected)?;
        snapshot.set_item("match_material", self.match_material)?;
        let spec: String = py.import("ingetrazo_render")?.call_method1("surface_spec", ("cutlist", json_dumps(py, snapshot.as_any())?))?.extract()?;
        let mut value: serde_json::Value = serde_json::from_str(&spec).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let members = value.as_object_mut().and_then(|o| o.remove("members")).unwrap_or_default();
        self.members = serde_json::from_value(members).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let table = &value["content"]["blocks"][0]["cells"];
        self.table = if table.is_null() {
            Vec::new()
        } else {
            table.as_array().into_iter().flatten().map(|row| {
                row.as_array().into_iter().flatten().map(|c| c.as_str().map(str::to_owned).unwrap_or_else(|| c.to_string())).collect()
            }).collect()
        };
        self.assembly = assembly.map(Bound::unbind);
        spec_sheet(py, &value.to_string())
    }

    fn closed(&self, py: Python<'_>, ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        if ctx.getattr("replaced")?.is_truthy()? || self.selected.is_empty() || self.built_for != Some(self.sig(ctx)?) {
            return Ok(());
        }
        let Some(assembly) = self.assembly.as_ref().map(|a| a.bind(py)) else { return Ok(()) };
        if !crate::host_context::core_module(py)?.call_method1("link_linked_ancestor", (assembly,))?.is_none() {
            return Ok(());
        }
        let Some(uids) = self.selected.parse::<usize>().ok().and_then(|i| self.members.get(i.wrapping_sub(1))) else { return Ok(()) };
        let vp = ctx.getattr("win")?.getattr("viewport")?;
        let scene = vp.getattr("scene")?;
        if !scene.getattr("edit_group")?.is(assembly) {
            vp.call_method1("begin_group_edit", (assembly,))?;
        }
        let scene = vp.getattr("scene")?;
        if scene.getattr("edit_group")?.is(assembly) {
            let mut picks = Vec::new();
            for p in assembly.getattr("children")?.try_iter()? {
                let p = p?;
                if uids.contains(&p.getattr("uid")?.str()?.to_string()) {
                    picks.push(p);
                }
            }
            scene.call_method1("select", (picks,))?;
            scene.call_method0("bump_view")?;
            ctx.getattr("view")?.call_method0("update")?;
        }
        Ok(())
    }

    fn act(&mut self, py: Python<'_>, ctx: &Bound<'_, PyAny>, verb: &str, args: Vec<String>) -> PyResult<()> {
        let first = args.first().map(String::as_str).unwrap_or("");
        if verb == "pick" && !first.is_empty() && first.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(i) = first.parse::<usize>() {
                if (1..=self.members.len()).contains(&i) {
                    self.selected = first.to_owned();
                }
            }
        }
        if verb != "verb" {
            return Ok(());
        }
        match first {
            "group" => {
                self.match_material = !self.match_material;
                self.selected.clear();
            }
            "copy" => {
                copy_text(py, &csv(&self.table, '\t'))?;
            }
            "csv" => {
                let dialogs = py.import("ingetrazo_render.shell_files")?.getattr("ShellFileDialog")?;
                let answer = dialogs.call_method1("getSaveFileName", (ctx.getattr("win")?, "Export cut list", "Cut list.csv", "CSV (*.csv)"))?;
                let path: String = answer.get_item(0)?.extract()?;
                if !path.is_empty() {
                    if let Err(e) = std::fs::write(&path, csv(&self.table, ',')) {
                        ctx.getattr("win")?.getattr("viewport")?.call_method1("flash_status", (e.to_string(),))?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// The tab-separated cut list (tests, the inspector's copy).
#[pyfunction]
fn cut_list_csv(table: &Bound<'_, PyAny>, tab: bool) -> PyResult<String> {
    Ok(csv(&cells(table)?, if tab { '\t' } else { ',' }))
}

// ---- Extensions -----------------------------------------------------------------------

/// The Extension Manager: the installed, browse and update lists of the
/// Rust `ExtensionBrowser`, previews fetched on a host worker pool, and the
/// installer's operations, which stay Python (plugins).
#[pyclass(module = "ingetrazo_render", dict)]
pub struct ExtensionsSheet {
    #[pyo3(get)]
    model: Py<PyAny>,
    #[pyo3(get, set)]
    catalog: Py<PyAny>,
    #[pyo3(get)]
    bundled: Py<PyAny>,
    #[pyo3(get, set)]
    cache: Py<PyAny>,
    #[pyo3(get)]
    pool: Py<PyAny>,
    #[pyo3(get, set)]
    refreshing: Option<Py<PyAny>>,
    #[pyo3(get)]
    requests: Py<PyDict>,
    #[pyo3(get, set)]
    reload: bool,
    timer: Option<Py<PyAny>>,
    #[pyo3(get)]
    entries: Py<PyDict>,
    #[pyo3(get)]
    hands: Py<PyDict>,
}

/// `d[key]`, raising KeyError as Python does.
fn need<'py>(d: &Bound<'py, PyDict>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    d.get_item(key)?.ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(key.to_owned()))
}

fn manager(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.extension_manager")
}

fn now_ms(py: Python<'_>) -> PyResult<u64> {
    let t: f64 = py.import("time")?.call_method0("monotonic")?.extract()?;
    Ok((t * 1000.0) as u64)
}

impl ExtensionsSheet {
    /// What the installer and discovery know now, for the model.
    fn snapshot(&self, py: Python<'_>, win: &Bound<'_, PyAny>) -> PyResult<()> {
        let em = manager(py)?;
        let entries = self.entries.bind(py);
        entries.clear();
        for e in self.bundled.bind(py).try_iter()? {
            let e = e?;
            entries.set_item(e.getattr("id")?, e)?;
        }
        let catalog = self.catalog.bind(py);
        let has_catalog = catalog.is_truthy()?;
        if has_catalog {
            for e in catalog.getattr("extensions")?.try_iter()? {
                let e = e?;
                entries.set_item(e.getattr("id")?, e)?;
            }
        }
        let states = em.call_method0("extension_startup_state")?;
        let hands = self.hands.bind(py);
        hands.clear();
        for p in em.call_method0("hand_installed_plugins")?.try_iter()? {
            let p = p?;
            hands.set_item(em.call_method1("hand_installed_ident", (&p,))?, p)?;
        }
        let vars = py.import("builtins")?.getattr("vars")?;
        let catalog_rows = PyDict::new(py);
        let incompatible = PyList::empty(py);
        let old = PyList::empty(py);
        for (k, e) in entries.iter() {
            catalog_rows.set_item(&k, vars.call1((&e,))?)?;
            if !em.call_method1("compatible_with_current", (&e,))?.is_truthy()? {
                incompatible.append(&k)?;
            }
            if em.call_method1("tested_version_is_old", (&e,))?.is_truthy()? {
                old.append(&k)?;
            }
        }
        let managed = PyDict::new(py);
        for k in states.try_iter()? {
            let k = k?;
            managed.set_item(&k, em.call_method1("installed_info", (&k,))?)?;
        }
        let hand_paths = PyDict::new(py);
        for (k, p) in hands.iter() {
            hand_paths.set_item(k, p.str()?)?;
        }
        let exts = py.import("core.extensions")?;
        let shipped = PyDict::new(py);
        let first_dir = exts.call_method0("plugin_dirs")?.get_item(0)?;
        for row in exts.call_method1("installed_candidates", (first_dir,))?.try_iter()? {
            let row = row?;
            shipped.set_item(row.get_item(0)?, row.get_item(1)?.str()?)?;
        }
        let d = PyDict::new(py);
        d.set_item("catalog", catalog_rows)?;
        d.set_item("states", &states)?;
        d.set_item("managed", managed)?;
        d.set_item("hands", hand_paths)?;
        d.set_item("shipped", shipped)?;
        d.set_item("startup", opt(win, "_extension_startup_state").unwrap_or_else(|| states.clone()))?;
        d.set_item("fetched", if has_catalog { catalog.getattr("fetched_at")? } else { "".into_pyobject(py)?.into_any() })?;
        d.set_item("safe", em.call_method0("safe_mode")?)?;
        d.set_item("disabled", PyList::new(py, em.call_method0("disabled_overrides")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?)?;
        d.set_item("reviewed", if has_catalog { catalog.getattr("reviewed")? } else { PyDict::new(py).into_any() })?;
        let running = match opt(win, "_running_extensions") {
            Some(r) => PyList::new(py, r.try_iter()?.collect::<PyResult<Vec<_>>>()?)?,
            None => PyList::empty(py),
        };
        d.set_item("running", running)?;
        d.set_item("errors", opt(win, "_extension_load_errors").unwrap_or_else(|| PyDict::new(py).into_any()))?;
        d.set_item("incompatible", incompatible)?;
        d.set_item("old", old)?;
        d.set_item("cache", self.cache.bind(py).str()?)?;
        d.set_item("lang", py.import("ingetrazo_core.i18n")?.call_method0("current_language")?)?;
        d.set_item("build", py.import("ingetrazo_core.version")?.getattr("__version__")?)?;
        let text = py.import("json")?.getattr("dumps")?.call((d,), Some(&[("default", py.import("builtins")?.getattr("str")?)].into_py_dict(py)?))?;
        self.model.bind(py).call_method1("load", (text,))?;
        Ok(())
    }
}

use pyo3::types::IntoPyDict;

/// The installer's question for "Replace copies".
#[pyclass(module = "ingetrazo_render")]
struct ConfirmCopies {
    sheet: Py<PyAny>,
    win: Py<PyAny>,
    ident: String,
}

#[pymethods]
impl ConfirmCopies {
    fn __call__(&self, py: Python<'_>, paths: &Bound<'_, PyAny>) -> PyResult<bool> {
        let sheet = self.sheet.bind(py);
        let names: Vec<String> = paths.try_iter()?.map(|p| p?.getattr("name")?.extract()).collect::<PyResult<_>>()?;
        let model = sheet.getattr("model")?;
        let (title, msg): (String, String) = model.call_method1("copies", (&self.ident, names, tr(py)?))?.extract()?;
        sheet.call_method1("confirm", (self.win.bind(py), title, msg))?.is_truthy()
    }
}

#[pymethods]
impl ExtensionsSheet {
    #[new]
    fn new(py: Python<'_>) -> PyResult<Self> {
        let em = manager(py)?;
        let model = py.import("ingetrazo_render")?.getattr("ExtensionBrowser")?.call0()?;
        let pool_kwargs = [("max_workers", 3i32.into_pyobject(py)?.into_any()), ("thread_name_prefix", "catalog".into_pyobject(py)?.into_any())].into_py_dict(py)?;
        let pool = py.import("concurrent.futures")?.getattr("ThreadPoolExecutor")?.call((), Some(&pool_kwargs))?;
        let cache = py.import("ingetrazo_core.autosave")?.call_method0("_app_data_base")?.div("extension-screenshots")?;
        Ok(Self {
            model: model.unbind(),
            catalog: em.call_method0("load_cached_catalog")?.unbind(),
            bundled: em.call_method0("bundled_extensions")?.unbind(),
            cache: cache.unbind(),
            pool: pool.unbind(),
            refreshing: None,
            requests: PyDict::new(py).unbind(),
            reload: true,
            timer: None,
            entries: PyDict::new(py).unbind(),
            hands: PyDict::new(py).unbind(),
        })
    }

    #[getter]
    fn id(&self) -> &'static str {
        "extensions"
    }

    fn closed(&mut self, py: Python<'_>, _ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        let kwargs = [("wait", false), ("cancel_futures", true)].into_py_dict(py)?;
        self.pool.bind(py).call_method("shutdown", (), Some(&kwargs))?;
        if let Some(timer) = self.timer.take() {
            timer.bind(py).call_method0("stop")?;
            timer.bind(py).call_method0("deleteLater")?;
        }
        Ok(())
    }

    fn signature(&mut self, py: Python<'_>, ctx: &Bound<'_, PyAny>) -> PyResult<(u64, bool)> {
        let em = manager(py)?;
        let now = now_ms(py)?;
        let model = self.model.bind(py);
        let requests = self.requests.bind(py);
        for (icon, job) in requests.copy()?.iter() {
            if job.call_method0("done")?.is_truthy()? {
                requests.del_item(&icon)?;
                let result = if job.call_method0("exception")?.is_none() { job.call_method0("result")? } else { py.None().into_bound(py) };
                model.call_method1("preview_done", (icon, now, result))?;
            }
        }
        for job in model.call_method1("jobs", (now,))?.try_iter()? {
            let job = job?;
            let fut = self.pool.bind(py).call_method1("submit", (job.getattr("run")?, em.getattr("download")?))?;
            requests.set_item(job.getattr("icon")?, fut)?;
        }
        if model.call_method0("pending")?.is_truthy()? || self.refreshing.is_some() {
            if let Some(host) = opt(&ctx.getattr("win")?, "_host") {
                host.call_method1("wake_after", (100,))?;
            } else if let Some(view) = opt(ctx, "view") {
                if self.timer.is_none() {
                    let timer = py.import("ingetrazo_core.timers")?.call_method1("timer", (&view, view.getattr("update")?))?;
                    self.timer = Some(timer.unbind());
                }
                let timer = self.timer.as_ref().expect("made above").bind(py);
                if !timer.call_method0("isActive")?.is_truthy()? {
                    timer.call_method1("start", (100,))?;
                }
            }
        }
        let done = match &self.refreshing {
            Some(r) => r.bind(py).call_method0("done")?.is_truthy()?,
            None => false,
        };
        Ok((model.getattr("revision")?.extract()?, done))
    }

    fn build<'py>(&mut self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let t = tr(py)?;
        let model = self.model.bind(py).clone();
        if let Some(refreshing) = self.refreshing.as_ref().map(|r| r.bind(py).clone()) {
            if refreshing.call_method0("done")?.is_truthy()? {
                match refreshing.call_method0("result") {
                    Ok(catalog) => {
                        let cached = catalog.getattr("from_cache")?.is_truthy()?;
                        model.setattr("notice", t.call1((if cached { "Cached catalog" } else { "Catalog refreshed" },))?)?;
                        self.catalog = catalog.unbind();
                    }
                    Err(e) => {
                        let kwargs = [("error", e.value(py).str()?)].into_py_dict(py)?;
                        model.setattr("notice", t.call(("Could not refresh catalog: {error}",), Some(&kwargs))?)?;
                    }
                }
                self.refreshing = None;
                self.reload = true;
            }
        }
        if self.reload {
            self.snapshot(py, &ctx.getattr("win")?)?;
            self.reload = false;
        }
        let spec: String = model.call_method1("sheet", (&t,))?.extract()?;
        let sheet = spec_sheet(py, &spec)?;
        for icon in model.call_method0("icons")?.try_iter()? {
            sheet.getattr("icon")?.call1(icon?.cast_into::<pyo3::types::PyTuple>()?)?;
        }
        Ok(sheet)
    }

    /// A yes/no question on the window's sheet.
    fn confirm(&self, py: Python<'_>, win: &Bound<'_, PyAny>, title: &str, text: &str) -> PyResult<bool> {
        let t = tr(py)?;
        let kwargs = [("cancel", "cancel"), ("default", "cancel")].into_py_dict(py)?;
        let buttons = vec![("cancel".into_pyobject(py)?.into_any(), t.call1(("Cancel",))?), ("ok".into_pyobject(py)?.into_any(), title.into_pyobject(py)?.into_any())];
        let answer = py.import("ingetrazo_render.chrome_dialogs")?.getattr("ask")?.call((win, title, text, buttons), Some(&kwargs))?;
        Ok(answer.extract::<Option<String>>()?.as_deref() == Some("ok"))
    }

    fn act(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, verb: &str, args: Vec<String>) -> PyResult<()> {
        let py = slf.py();
        let em = manager(py)?;
        let t = tr(py)?;
        let model = slf.borrow().model.bind(py).clone();
        let win = ctx.getattr("win")?;
        let action = if verb != "verb" {
            if verb == "tab" {
                slf.borrow_mut().reload = true;
            }
            if !model.call_method1("receive", (verb, args))?.is_truthy()? {
                return Ok(());
            }
            "refresh".to_owned()
        } else {
            args.first().cloned().unwrap_or_default()
        };
        let ask = |title: &Bound<'_, PyAny>, text: &Bound<'_, PyAny>| -> PyResult<bool> {
            slf.as_any().call_method1("confirm", (&win, title, text))?.is_truthy()
        };
        let open_url = |target: Bound<'_, PyAny>| -> PyResult<()> {
            py.import("ingetrazo_core.desktop_open")?.call_method1("open_url", (target,))?;
            Ok(())
        };
        let result = (|| -> PyResult<bool> {
            match action.as_str() {
                "refresh" => {
                    let mut me = slf.borrow_mut();
                    if me.refreshing.is_none() {
                        me.refreshing = Some(me.pool.bind(py).call_method1("submit", (em.getattr("refresh_catalog")?,))?.unbind());
                        model.setattr("notice", t.call1(("Refreshing catalog…",))?)?;
                    }
                    return Ok(false);
                }
                "safe" => {
                    let safe = em.call_method0("safe_mode")?.is_truthy()?;
                    em.call_method1("set_safe_mode", (!safe,))?;
                }
                "reset" => {
                    if !ask(&t.call1(("Reset overrides",))?, &t.call1(("Enable all extensions and leave safe mode on next launch?",))?)? {
                        return Ok(false);
                    }
                    em.call_method0("reset_overrides")?;
                }
                _ => {
                    let plan = model.call_method1("command", (&action, &t))?;
                    if plan.is_none() {
                        return Ok(false);
                    }
                    let (op, ident, questions): (String, String, Vec<(String, String)>) = plan.extract()?;
                    for (title, msg) in &questions {
                        if !ask(&title.into_pyobject(py)?.into_any(), &msg.into_pyobject(py)?.into_any())? {
                            return Ok(false);
                        }
                    }
                    let entries = slf.borrow().entries.bind(py).clone();
                    let hands = slf.borrow().hands.bind(py).clone();
                    match op.as_str() {
                        "install" | "replace" => {
                            let copies = Py::new(py, ConfirmCopies { sheet: slf.clone().into_any().unbind(), win: win.clone().unbind(), ident: ident.clone() })?;
                            let ids = pyo3::types::PySet::new(py, entries.keys().iter())?;
                            let kwargs = PyDict::new(py);
                            kwargs.set_item("confirm_manual_copies", copies)?;
                            kwargs.set_item("catalog_ids", ids)?;
                            let installer = em.getattr(if op == "install" { "install_extension" } else { "replace_hand_installed" })?;
                            installer.call((need(&entries, &ident)?,), Some(&kwargs))?;
                        }
                        "enable" | "disable" => {
                            em.call_method1("set_extension_enabled", (&ident, op == "enable"))?;
                            em.call_method1("set_override", (&ident, false))?;
                        }
                        "override_on" | "override_off" => {
                            em.call_method1("set_override", (&ident, op == "override_off"))?;
                        }
                        "set_aside" => {
                            em.call_method1("remove_hand_installed", (need(&hands, &ident)?,))?;
                        }
                        "uninstall" => {
                            em.call_method1("uninstall_extension", (&ident,))?;
                        }
                        "repository" => {
                            open_url(need(&entries, &ident)?.getattr("repository")?)?;
                            return Ok(false);
                        }
                        "logs" => {
                            open_url(py.import("ingetrazo_core.paths")?.call_method0("user_log_dir")?)?;
                            return Ok(false);
                        }
                        "folder" => {
                            let folder = match hands.get_item(&ident)? {
                                Some(f) => f,
                                None => py.import("core.extensions")?.call_method0("user_plugins_dir")?.div(&ident)?,
                            };
                            let target = if folder.call_method0("is_dir")?.is_truthy()? { folder } else { folder.getattr("parent")? };
                            open_url(target)?;
                            return Ok(false);
                        }
                        _ => {}
                    }
                }
            }
            Ok(true)
        })();
        match result {
            Ok(true) => {
                slf.borrow_mut().reload = true;
                model.setattr("notice", t.call1(("Restart to apply changes",))?)?;
            }
            Ok(false) => {}
            Err(e) => {
                let installer_error = em.getattr("ExtensionManagerError")?;
                let oserror = py.get_type::<pyo3::exceptions::PyOSError>();
                let value = e.value(py);
                if value.is_instance(&installer_error)? || value.is_instance(&oserror)? {
                    model.setattr("notice", value.str()?)?;
                } else {
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}

/// Help ▸ About, Window ▸ Extensions, the inspector's Cut list…: the sheet
/// `sid` on the window.
#[pyfunction]
fn show_report_sheet<'py>(py: Python<'py>, win: &Bound<'py, PyAny>, sid: &str) -> PyResult<Bound<'py, PyAny>> {
    let chrome = py.import("ingetrazo_render.chrome_sheet")?;
    if sid == "extensions" {
        let opened = py.import("ingetrazo_render")?.call_method0("ExtensionsSheet")
            .and_then(|provider| chrome.call_method1("open", (win, provider)));
        return match opened {
            Err(error) if error.is_instance_of::<pyo3::exceptions::PyException>(py) => {
                crate::extension_setup::log_failure(py, "the extension manager could not open", &error);
                let tr = tr(py)?;
                let fields = PyDict::new(py);
                fields.set_item("error", error.to_string())?;
                let detail = tr.call(("Could not load extensions: {error}",), Some(&fields))?;
                let kwargs = PyDict::new(py);
                kwargs.set_item("tone", "warning")?;
                let title = tr.call1(("Extension manager",))?;
                py.import("ingetrazo_render.chrome_dialogs")?.call_method("notify", (win, title, detail), Some(&kwargs))?;
                Ok(py.None().into_bound(py))
            }
            result => result,
        };
    }
    let provider = match sid {
        "about" => Py::new(py, AboutSheet::new())?.into_any(),
        "cutlist" => Py::new(py, CutListSheet::new())?.into_any(),
        _ => return Err(pyo3::exceptions::PyKeyError::new_err(sid.to_owned())),
    };
    chrome.call_method1("open", (win, provider))
}

/// A document-scoped, read-only text result supplied by an extension.
#[pyclass]
struct PluginTextReport {
    #[pyo3(get)]
    title: String,
    text: String,
    copy: String,
    acknowledgement: String,
    copied: bool,
}

#[pymethods]
impl PluginTextReport {
    #[classattr]
    #[pyo3(name = "slot")]
    const SLOT: &'static str = "utility";

    #[classattr]
    #[pyo3(name = "id")]
    const ID: &'static str = "report";

    #[new]
    fn new(title: String, text: String, copy: String, acknowledgement: String) -> Self {
        Self { title, text, copy, acknowledgement, copied: false }
    }

    fn sections<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let label = if self.copied { &self.acknowledgement } else { &self.copy };
        let data = serde_json::json!([{"id":"report", "title":"", "items":[
            {"kind":"log", "id":"text", "lines":self.text.lines().collect::<Vec<_>>()},
            {"kind":"button", "id":"copy", "label":label}
        ]}]);
        json_loads(py, &data.to_string())
    }

    fn event(&mut self, py: Python<'_>, control: &str, event: &str, _value: &Bound<'_, PyAny>) -> PyResult<()> {
        if (control, event) == ("copy", "invoke") {
            self.copied = copy_text(py, &self.text)?;
        }
        Ok(())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PluginTextReport>()?;
    m.add_class::<AboutSheet>()?;
    m.add_class::<CutListSheet>()?;
    m.add_class::<ExtensionsSheet>()?;
    m.add_function(wrap_pyfunction!(about_review, m)?)?;
    m.add_function(wrap_pyfunction!(selected_assembly, m)?)?;
    m.add_function(wrap_pyfunction!(cut_list_csv, m)?)?;
    m.add_function(wrap_pyfunction!(show_report_sheet, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::csv;

    #[test]
    fn csv_quotes_like_pythons_csv_module() {
        let rows = vec![vec!["a".into(), "b,c".into(), "say \"hi\"".into()], vec!["x\ty".into(), "".into(), "l\nm".into()]];
        assert_eq!(csv(&rows, ','), "a,\"b,c\",\"say \"\"hi\"\"\"\r\nx\ty,,\"l\nm\"\r\n");
        assert_eq!(csv(&rows, '\t'), "a\tb,c\t\"say \"\"hi\"\"\"\r\n\"x\ty\"\t\t\"l\nm\"\r\n");
    }
}
