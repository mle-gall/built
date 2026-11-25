// SPDX-License-Identifier: GPL-3.0-or-later
//! The dock's verbs (`views.wgpu_ui._tree_action`): one action string from
//! the project tree, the inspector's chips or a prompt, dispatched to the
//! core protocol and the window.
use super::chrome_frame::{attr, id, own, render, truthy, ui};
use super::*;
use pyo3::exceptions::{PyKeyError, PyOSError, PyValueError};
use pyo3::types::PyCFunction;

const SEP: char = '\x1f';

/// `text.partition(sep)` as `(head, tail)`.
fn partition(text: &str, sep: char) -> (&str, &str) {
    text.split_once(sep).unwrap_or((text, ""))
}
fn empty<'py>(py: Python<'py>) -> Bound<'py, PyList> {
    PyList::empty(py)
}
fn list<'py>(values: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    Ok(values.py().get_type::<PyList>().call1((values,))?.cast_into()?)
}
fn loads<'py>(py: Python<'py>, text: &str) -> Reply<'py> {
    py.import("json")?.call_method1("loads", (text,))
}
fn is_one(value: &str, names: &[&str]) -> bool {
    names.contains(&value)
}
fn flash(viewport: &Obj<'_>, message: impl AsRef<str>) -> PyResult<()> {
    viewport.call_method1("flash_status", (message.as_ref(),)).map(drop)
}
fn changed(viewport: &Obj<'_>) -> PyResult<()> {
    viewport.call_method0("notify_scene_changed").map(drop)
}
fn core<'py>(viewport: &Obj<'py>, command: &str, args: Bound<'py, PyDict>) -> Reply<'py> {
    core_command(viewport, command.into_bound_py_any(viewport.py())?.as_any(), Some(&args))
}
/// The file a dialog gave, `None` when it was cancelled.
fn chosen<'py>(py: Python<'py>, dialog: &str, args: impl pyo3::call::PyCallArgs<'py>) -> PyResult<Option<Obj<'py>>> {
    let path = render(py)?.getattr("shell_files")?.getattr("ShellFileDialog")?.call_method1(dialog, args)?.get_item(0)?;
    Ok(if path.is_truthy()? { Some(path) } else { None })
}
/// `str(exc)` flashed when `result` failed with one of the errors a file
/// verb reports instead of raising.
fn told<'py>(viewport: &Obj<'py>, result: PyResult<()>, value_error: bool) -> PyResult<()> {
    let py = viewport.py();
    match result {
        Err(error) if error.is_instance_of::<PyOSError>(py) || (value_error && error.is_instance_of::<PyValueError>(py)) => {
            flash(viewport, error.value(py).str()?.to_str()?)
        }
        other => other,
    }
}

/// One verb from the dock: `verb SEP rowkey [SEP extra]`. Row keys:
/// n:<node> (n: alone is Unfiled), i:/c:<uid> items, d:<node>:<mesh>
/// copies of a definition, g:<uid> filed elsewhere.
fn tree_action<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, view) = (a.get_item(0)?, a.get_item(1)?);
    let action: String = a.get_item(2)?.extract()?;
    if !action.contains(SEP) {
        chip(&win, &view, &action)?;
        return Ok(none(py));
    }
    let viewport = win.getattr("viewport")?;
    match Row::new(&win, &view, &viewport, &action)?.run() {
        Err(error) if error.is_instance_of::<PyKeyError>(py) || error.is_instance_of::<PyValueError>(py) => {
            flash(&viewport, error.value(py).str()?.to_str()?)?;
        }
        other => other?,
    }
    Ok(none(py))
}

/// The inspector's chips and the delivery counts: "verb:arg".
fn chip<'py>(win: &Obj<'py>, view: &Obj<'py>, action: &str) -> PyResult<()> {
    let py = win.py();
    let module = ui(py)?;
    let (verb, key) = partition(action, ':');
    let viewport = win.getattr("viewport")?;
    let scene = viewport.getattr("scene")?;
    let group_type = py.import("core.group")?.getattr("Group")?;
    let groups = empty(py);
    for entity in list(&scene.getattr("selection")?)?.iter() {
        if entity.is_instance(&group_type)? {
            groups.append(entity)?;
        }
    }
    let again = |action: String| module.call_method1("_tree_action", (win, view, action)).map(drop);
    let tail = partition(key, ':').1;
    if verb == "unfiled" {
        let unfiled = match attr(view, "_ui_tree")? {
            data if data.is_truthy()? => data.call_method1("get", ("unfiled", empty(py)))?,
            _ => empty(py).into_any(),
        };
        module.call_method1("_select", (win, unfiled))?;
        return Ok(());
    }
    if verb != "insp" {
        return Ok(());
    }
    if let Some(delivery) = key.strip_prefix("delivery:") {
        module.call_method1("_run", (win, PyList::new(py, [format!("deliver:{delivery}")])?))?;
    } else if key.starts_with("opensheetrow:") {
        again(format!("open{SEP}{tail}"))?;
    } else if key.starts_with("sectionview:") {
        py.import("views.view_commands")?.call_method1("run", (win, tail))?;
        view.setattr("_ui_tree_sig", py.None())?;
        view.setattr("_ui_insp_sig", py.None())?;
    } else if key.starts_with("section:") {
        let fields: Vec<&str> = key.split(':').collect();
        if fields.len() < 3 {
            return Err(PyValueError::new_err(format!("not enough values to unpack (expected at least 3, got {})", fields.len())));
        }
        again(format!("edit{SEP}section:{}:{}{SEP}{}", fields[1], fields[2], fields.get(3).copied().unwrap_or("")))?;
    } else if key.starts_with("status:") {
        direct(py, super::chrome_inspector::set_status, (win, &groups, tail))?;
    } else if key == "file" && !groups.is_empty() {
        direct(py, super::chrome_inspector::file_to, (win, groups.len()))?;
    } else if key.starts_with("active:") || key.starts_with("isolate:") {
        let (what, node) = partition(key, ':');
        again(format!("{what}{SEP}n:{node}"))?;
    } else if key.starts_with("nodestatus:") {
        let fields: Vec<&str> = key.splitn(3, ':').collect();
        if fields.len() != 3 {
            return Err(PyValueError::new_err(format!("not enough values to unpack (expected 3, got {})", fields.len())));
        }
        direct(py, super::chrome_inspector::set_status, (win, direct(py, super::chrome_tree::tree_groups, (view, fields[1]))?, fields[2]))?;
    } else if key.starts_with("cutlist") {
        if !tail.is_empty() {
            module.call_method1("_select", (win, direct(py, super::chrome_tree::tree_groups, (view, tail))?))?;
        }
        render(py)?.call_method1("show_report_sheet", (win, "cutlist"))?;
    } else if let Some(verb) = key.strip_prefix("lib:") {
        render(py)?.getattr("wgpu_library")?.call_method1("action", (win, view, verb))?;
    } else if let Some(form) = key.strip_prefix("form:") {
        direct(py, super::chrome_actions::open_form, (view, form))?;
    } else if key.starts_with("takeoff_faces:") {
        py.import("ingetrazo_core")?.getattr("InspectorTakeoff")?.call_method1("select_faces", (&viewport, tail))?;
    } else if key == "measure" {
        module.call_method1("_measure", (view, win))?;
        view.setattr("_ui_tree_sig", py.None())?;
    } else if key == "solidrepair" || key.starts_with("solidfault:") {
        let active = key.starts_with("solidfault:active/") && own(view, "_ui_lens")?.is_some_and(|lens| lens.eq(2).unwrap_or(false));
        if !active && groups.len() != 1 {
            return Ok(());
        }
        let placement = if active { none(py) } else { scene.call_method1("occurrence_path", (groups.get_item(0)?,))? };
        let fault = if tail.is_empty() { none(py) } else { tail.into_bound_py_any(py)? };
        let out = core(&viewport, "repair_solid", fields!(py; "placement" => placement, "fault" => fault))?;
        if out.is_truthy()? {
            let error = out.call_method1("get", ("error",))?;
            if error.is_truthy()? {
                flash(&viewport, out.get_item("error")?.str()?.to_str()?)?;
            } else if out.get_item("ok")?.is_truthy()? {
                let done = out.get_item("repaired")?;
                let count = |name: &str| -> PyResult<String> { done.get_item(name)?.str()?.extract() };
                flash(
                    &viewport,
                    format!(
                        "Repaired: {} stray edges, {} duplicated faces, {} holes closed, {} partitions, {} faces turned",
                        count("stray")?,
                        count("duplicates")?,
                        count("holes")?,
                        count("partitions")?,
                        count("reversed")?
                    ),
                )?;
            }
        }
        view.setattr("_ui_insp_sig", py.None())?;
        viewport.call_method0("update")?;
    } else if let Some(link) = key.strip_prefix("lnk:") {
        let (what, node) = partition(link, ':');
        again(format!("{what}{SEP}n:{node}"))?;
    } else if key.starts_with("link") && !groups.is_empty() {
        py.import("core.links")?;
        let first = groups.get_item(0)?;
        let holder = module.getattr("ingetrazo_core")?.call_method1("link_linked_ancestor", (&first,))?;
        if key == "linkfile" {
            if !holder.is_none() {
                module.call_method1("_select", (win, PyList::new(py, [holder])?))?;
            }
        } else {
            module.call_method1("_link_action", (win, view, if holder.is_none() { first } else { holder }, key))?;
        }
    }
    Ok(())
}

/// A row verb's context: the row key taken apart and what the last tree
/// sync kept (`view._ui_tree`).
struct Row<'a, 'py> {
    py: Python<'py>,
    module: Bound<'py, PyModule>,
    win: &'a Obj<'py>,
    view: &'a Obj<'py>,
    viewport: &'a Obj<'py>,
    scene: Obj<'py>,
    exp: Obj<'py>,
    data: Obj<'py>,
    verb: String,
    rk: String,
    extra: String,
    kind: String,
    ident: String,
    /// `ident` of an `n:` row.
    node: Option<String>,
    tree: Obj<'py>,
    linked: bool,
    tnode: Obj<'py>,
}

impl<'a, 'py> Row<'a, 'py> {
    fn new(win: &'a Obj<'py>, view: &'a Obj<'py>, viewport: &'a Obj<'py>, action: &str) -> PyResult<Self> {
        let py = win.py();
        let mut fields = action.split(SEP);
        let verb = fields.next().unwrap_or("").to_owned();
        let rk = fields.next().unwrap_or("").to_owned();
        let extra = fields.next().unwrap_or("").to_owned();
        let (kind, ident) = partition(&rk, ':');
        let (kind, ident) = (kind.to_owned(), ident.to_owned());
        let node = (kind == "n").then(|| ident.clone());
        let data = match attr(view, "_ui_tree")? {
            data if data.is_truthy()? => data,
            _ => PyDict::new(py).into_any(),
        };
        let tree = data.call_method1("get", ("tree",))?;
        let linked = py.import("core.link_structure")?.call_method1("is_linked_key", (node.as_deref(),))?.is_truthy()?;
        let mut row = Self {
            py,
            module: ui(py)?,
            win,
            view,
            viewport,
            scene: viewport.getattr("scene")?,
            exp: view.getattr("__dict__")?.call_method1("setdefault", ("_ui_exp", PySet::empty(py)?))?,
            tnode: PyDict::new(py).into_any(),
            data,
            verb,
            rk,
            extra,
            kind,
            ident,
            node,
            tree,
            linked,
        };
        let known = row.table("nodes")?.call_method1("get", (row.node.as_deref(),))?;
        if known.is_truthy()? {
            row.tnode = known;
        }
        Ok(row)
    }

    /// `data.get(name)`, an empty dict without one.
    fn table(&self, name: &str) -> Reply<'py> {
        match self.data.call_method1("get", (name,))? {
            table if table.is_truthy()? => Ok(table),
            _ => Ok(PyDict::new(self.py).into_any()),
        }
    }
    fn unfiled(&self) -> Reply<'py> {
        self.data.call_method1("get", ("unfiled", empty(self.py)))
    }
    /// The node of an `n:` row with a name (`n:` alone is Unfiled).
    fn named(&self) -> Option<&str> {
        self.node.as_deref().filter(|node| !node.is_empty())
    }
    fn node_kind(&self, names: &[&str]) -> PyResult<bool> {
        let kind = self.tnode.call_method1("get", ("kind",))?;
        for name in names {
            if kind.eq(*name)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn is(&self, names: &[&str]) -> bool {
        is_one(&self.verb, names)
    }
    fn kind_is(&self, names: &[&str]) -> bool {
        is_one(&self.kind, names)
    }
    fn core(&self, command: &str, args: Bound<'py, PyDict>) -> Reply<'py> {
        core(self.viewport, command, args)
    }
    fn changed(&self) -> PyResult<()> {
        changed(self.viewport)
    }
    fn flash(&self, message: impl AsRef<str>) -> PyResult<()> {
        flash(self.viewport, message)
    }
    fn select(&self, groups: impl IntoPyObject<'py>) -> PyResult<()> {
        self.module.call_method1("_select", (self.win, groups)).map(drop)
    }
    fn renderer(&self) -> Reply<'py> {
        self.view.getattr("renderer")
    }
    fn rebuild(&self) -> PyResult<()> {
        self.view.setattr("_ui_tree_sig", self.py.None())?;
        self.view.call_method0("update").map(drop)
    }
    /// The inspector shows this row until the selection moves.
    fn inspect(&self, key: &str) -> PyResult<()> {
        let stamp = direct(self.py, super::chrome_frame::sel_key, (self.scene.getattr("selection")?,))?;
        self.view.setattr("_ui_insp_node", (key, stamp))
    }
    fn expand(&self, key: impl AsRef<str>) -> PyResult<()> {
        self.exp.call_method1("add", (key.as_ref(),)).map(drop)
    }
    fn section_plane(&self) -> Reply<'py> {
        for plane in self.scene.getattr("section_planes")?.try_iter()? {
            let plane = plane?;
            if plane.getattr("uid")?.str()?.to_str()? == self.ident {
                return Ok(plane);
            }
        }
        Ok(none(self.py))
    }

    fn item(&self) -> Reply<'py> {
        if self.kind == "exportitem" {
            let path = loads(self.py, &self.ident)?;
            return if path.is_truthy()? { self.scene.call_method1("resolve_occurrence", (path,)) } else { Ok(none(self.py)) };
        }
        if self.kind_is(&["i", "c", "g"]) {
            direct(self.py, super::chrome_tree::group_by_uid, (self.win, &self.ident))
        } else {
            Ok(none(self.py))
        }
    }
    fn copies(&self) -> PyResult<Bound<'py, PyList>> {
        let out = empty(self.py);
        if self.kind != "d" {
            return Ok(out);
        }
        let (node, mesh) = self.ident.rsplit_once(':').unwrap_or(("", &self.ident));
        let pool = if node.is_empty() { self.unfiled()? } else { self.table("direct")?.call_method1("get", (node, empty(self.py)))? };
        for group in pool.try_iter()? {
            let group = group?;
            if id(&group.getattr("mesh")?).to_string() == mesh {
                out.append(group)?;
            }
        }
        Ok(out)
    }
    /// The copies of a definition row, else the row's own item.
    fn target(&self) -> Reply<'py> {
        if self.kind == "d" { Ok(self.copies()?.into_any()) } else { self.item() }
    }
    fn address(&self, group: &Obj<'py>) -> Reply<'py> {
        if self.tree.is_none() { group.getattr("uid") } else { self.tree.call_method1("address", (group,)) }
    }
    fn addresses(&self, groups: &Bound<'py, PyList>) -> PyResult<Bound<'py, PyList>> {
        let out = empty(self.py);
        for group in groups.iter() {
            out.append(self.address(&group)?)?;
        }
        Ok(out)
    }
    fn members(&self) -> PyResult<Bound<'py, PyList>> {
        match self.node.as_deref() {
            Some("") => list(&self.unfiled()?),
            Some(node) => Ok(direct(self.py, super::chrome_tree::tree_groups, (self.view, node))?.cast_into()?),
            None => {
                let copies = self.copies()?;
                if !copies.is_empty() {
                    return Ok(copies);
                }
                let out = empty(self.py);
                if self.item()?.is_truthy()? {
                    out.append(self.item()?)?;
                }
                Ok(out)
            }
        }
    }
    /// A linked row's own placements: the link placements of the files it
    /// stands for (one item each), never their tens of thousands of parts.
    fn placements(&self) -> PyResult<Bound<'py, PyList>> {
        let out = empty(self.py);
        let Some(node) = self.named().filter(|_| !self.tree.is_none()) else {
            return Ok(out);
        };
        let links = self.tree.getattr("links")?;
        let none_yet = PyTuple::empty(self.py);
        if self.node_kind(&["link"])? {
            out.call_method1("extend", (links.call_method1("get", (node, &none_yet))?,))?;
            return Ok(out);
        }
        let nodes = self.tree.getattr("nodes")?;
        for child in self.tree.getattr("children")?.call_method1("get", (node, &none_yet))?.try_iter()? {
            let child = child?;
            if nodes.get_item(&child)?.get_item("kind")?.eq("link")? {
                out.call_method1("extend", (links.call_method1("get", (child, &none_yet))?,))?;
            }
        }
        Ok(out)
    }
    /// What Isolate, Zoom and the like act on for this row.
    fn shown(&self) -> PyResult<Bound<'py, PyList>> {
        if self.linked && self.node_kind(&["link", "linked"])? { self.placements() } else { self.members() }
    }
    /// The layers the node's items (and its sub-nodes') carry, and the tags
    /// its recipes name.
    fn tags(&self, key: &str) -> PyResult<Bound<'py, PyList>> {
        let py = self.py;
        let seen = PyDict::new(py);
        let (nodes, layers, children) = (self.table("nodes")?, self.table("node_layers")?, self.table("children")?);
        let mut stack = vec![key.into_bound_py_any(py)?];
        while let Some(key) = stack.pop() {
            for layer in layers.call_method1("get", (&key, PyTuple::empty(py)))?.try_iter()? {
                seen.set_item(layer?, py.None())?;
            }
            let node = match nodes.call_method1("get", (&key,))? {
                node if node.is_truthy()? => node,
                _ => PyDict::new(py).into_any(),
            };
            let field = |name: &str| node.call_method1("get", (name,));
            if field("kind")?.eq("host")? {
                let tag = node.call_method1("get", ("recipe", PyDict::new(py)))?.call_method1("get", ("tag",))?;
                let layer = direct(py, super::chrome_tree::layer, (&self.scene, tag))?;
                if !layer.is_none() {
                    seen.set_item(layer.getattr("key")?, py.None())?;
                }
            } else if field("tag")?.is_truthy()? {
                seen.set_item(node.get_item("tag")?, py.None())?;
            }
            for child in children.call_method1("get", (&key, PyTuple::empty(py)))?.try_iter()? {
                stack.push(child?);
            }
        }
        Ok(seen.keys())
    }
    fn link_folders(&self, key: &str) -> PyResult<Bound<'py, PyList>> {
        let py = self.py;
        let nodes = self.table("nodes")?;
        let node = match nodes.call_method1("get", (key,))? {
            node if node.is_truthy()? => node,
            _ => PyDict::new(py).into_any(),
        };
        let kind = node.call_method1("get", ("kind",))?;
        if kind.eq("link")? {
            let folders = node.call_method1("get", ("folders",))?;
            return if folders.is_truthy()? { list(&folders) } else { PyList::new(py, [node.get_item("folder")?]) };
        }
        let out = empty(py);
        if kind.eq("linked")? {
            let nodes = self.data.get_item("nodes")?;
            for child in self.table("children")?.call_method1("get", (key, PyTuple::empty(py)))?.try_iter()? {
                let child = nodes.get_item(child?)?;
                if child.call_method1("get", ("kind",))?.eq("link")? {
                    out.append(child.get_item("folder")?)?;
                }
            }
        } else if kind.eq("tagfolder")? {
            out.append(node.get_item("folder")?)?;
        }
        Ok(out)
    }
    fn structure_nodes(&self) -> Reply<'py> {
        let registry = self.py.import("core.bim_registry")?.call_method1("get", (&self.scene,))?;
        self.py.import("ingetrazo_core.structure")?.call_method1("_nodes", (registry,))
    }
    /// Replace a node's filing rule with `recipe`, against the one it had.
    fn set_rule(&self, node: &str, recipe: Obj<'py>) -> PyResult<()> {
        let expected = self.structure_nodes()?.get_item(node)?.call_method1("get", ("recipe", PyDict::new(self.py)))?;
        self.core("set_filing_rule", fields!(self.py; "node" => node, "recipe" => recipe, "expected" => expected))?;
        self.viewport.call_method0("refresh_after_edit").map(drop)
    }
    fn recipe_of(&self, node: Obj<'py>) -> Reply<'py> {
        self.py.get_type::<PyDict>().call1((node.call_method1("get", ("recipe", PyDict::new(self.py)))?,))
    }
    /// A file the user picked, while this document is still the window's.
    fn file(&self, dialog: &str, title: &str, name: &str, filter: &str) -> PyResult<Option<Obj<'py>>> {
        let path = chosen(self.py, dialog, (self.win, title, name, filter))?;
        Ok(path.filter(|_| self.win.getattr("viewport").and_then(|viewport| viewport.getattr("scene")).is_ok_and(|scene| scene.is(&self.scene))))
    }

    fn run(self) -> PyResult<()> {
        let py = self.py;
        let (win, view, viewport) = (self.win, self.view, self.viewport);
        let scene = self.scene.clone();
        let module = self.module.clone();
        let glue = render(py)?;
        let core_module = py.import("ingetrazo_core")?;
        let structure = py.import("ingetrazo_core.structure")?;
        let link_structure = py.import("core.link_structure")?;
        let execute = || glue.getattr("tree_navigation")?.getattr("execute");
        let (rk, extra, ident) = (self.rk.clone(), self.extra.clone(), self.ident.clone());
        let node = self.node.clone();
        let named = self.named().map(str::to_owned);
        let tag_row = self.kind == "preview-tag";
        if self.is(&["grouprows"]) {
            self.renderer()?.call_method1("ui_set_tree_group_rows", (&rk, glue.call_method1("tree_group_rows", (&scene, self.target()?))?))?;
        } else if self.is(&["groupplan", "cutview"]) {
            let plan = self.verb == "groupplan";
            glue.call_method1("plan_options", (win, if plan { self.item()? } else { self.section_plane()? }, plan))?;
        } else if self.is(&["paintrows"]) {
            let target = glue.getattr("TreePaint")?.call1((&scene, self.target()?))?;
            glue.getattr("wgpu_library")?.call_method("open_library", (win, "materials"), Some(&fields!(py; "scope" => "s:model", "paint_target" => target)))?;
        } else if self.is(&["into", "beside", "root", "outdent"]) {
            let target = if extra.is_empty() { none(py) } else { direct(py, super::chrome_tree::group_by_uid, (win, partition(&extra, ':').1))? };
            if glue.call_method1("tree_move", (viewport, self.item()?, target, &self.verb))?.is_truthy()? {
                if self.verb == "into" {
                    self.expand(&extra)?;
                }
                self.rebuild()?;
            }
        } else if self.is(&["stepout"]) {
            if !scene.getattr("edit_group")?.is_none() {
                viewport.call_method0("end_one_group_edit")?;
                self.rebuild()?;
            }
        } else if self.is(&["sectioncut"]) && self.kind == "section" {
            self.core("edit_section_plane", fields!(py; "uid" => &ident, "field" => "cut"))?;
            self.changed()?;
        } else if self.is(&["headereye", "headerlock"]) {
            glue.call_method1("tree_header_state", (viewport, &rk, self.verb == "headerlock"))?;
            self.rebuild()?;
        } else if self.is(&["batchrename"]) {
            let model = core_module.call_method1("browser_rename", (&scene, &rk))?;
            if !model.is_none() {
                let single = model.call_method0("single_row")?;
                if single.is_none() || !self.renderer()?.call_method1("ui_rename_browser_row", (single,))?.is_truthy()? {
                    let forms = glue.getattr("chrome_forms")?;
                    forms.call_method1("show", (win, forms.getattr("BrowserRenameForm")?.call1((win, model))?))?;
                }
            }
        } else if self.is(&["isolatetags"]) {
            glue.call_method1("isolate_tags", (viewport, loads(py, &rk)?))?;
            self.rebuild()?;
        } else if self.is(&["scenetaghide", "scenetagshow"]) && tag_row {
            core_module.call_method1("tags_in_saved_views", (viewport.getattr("history")?, PyList::new(py, [&ident])?, self.verb == "scenetaghide"))?;
            self.changed()?;
        } else if self.is(&["mergetag"]) && tag_row {
            glue.call_method1("choose_merge_tag", (win, &ident))?;
        } else if self.is(&["purgetags"]) {
            glue.call_method1("tree_purge_tags", (win,))?;
        } else if self.is(&["newtaghidden"]) {
            glue.call_method1("set_new_tags_hidden", (rk == "1",))?;
        } else if self.is(&["newtag", "tagassign", "tagactive", "tagpick", "tagcolours", "modellines"]) || (self.verb == "rename" && tag_row) {
            let (verb, row) = if self.verb == "tagpick" {
                ("tagassign".into_bound_py_any(py)?, "preview-tag:".into_bound_py_any(py)?.add(loads(py, &rk)?)?)
            } else {
                (self.verb.as_str().into_bound_py_any(py)?, rk.as_str().into_bound_py_any(py)?)
            };
            let key = glue.call_method1("tree_edit_tag", (win, verb, row, &extra, &self.exp))?;
            if key.is_truthy()? {
                view.setattr("_ui_filter", "")?;
                let renderer = self.renderer()?;
                if !renderer.is_none() {
                    renderer.call_method1("ui_drawing_applied", ("newtag", key))?;
                }
            }
            self.rebuild()?;
        } else if self.is(&["deletetag"]) && tag_row {
            glue.call_method1("tree_delete_tag", (win, &ident))?;
            self.rebuild()?;
        } else if self.is(&["deleterows"]) {
            let target = if self.kind == "section" { self.section_plane()? } else { self.item()? };
            glue.call_method1("tree_delete_rows", (viewport, target))?;
            self.rebuild()?;
        } else if self.is(&["isolaterows", "pickrow", "roweye", "rowlock", "toggleitem", "group", "component", "explode", "unique", "instances"]) {
            glue.call_method1("tree_group_action", (win, self.target()?, &self.verb))?;
        } else if self.is(&["hosteye"]) {
            let [host, mount] = unpack::<2>(&py.import("views.wgpu_tabs")?.call_method1("source_context", (win,))?)?;
            if !mount.is_none() {
                let host_viewport = host.getattr("viewport")?;
                core(&host_viewport, "set_tag_state", fields!(py; "tags" => PyList::new(py, [&rk])?, "visible" => extra == "1"))?;
                changed(&host_viewport)?;
                view.call_method0("update")?;
            }
        } else if self.is(&["documentroot"]) {
            glue.getattr("tree_navigation")?.call_method1("open_to", (viewport, empty(py)))?;
        } else if self.is(&["revealselection", "revealnode"]) {
            let selected = scene.getattr("selection")?.is_truthy()?;
            let plan = if selected { module.getattr("_tree")?.call((view, win), Some(&fields!(py; "discover" => true)))? } else { none(py) };
            if plan.is_truthy()? {
                let [target, ancestors, limits] = unpack::<3>(&plan)?;
                self.exp.call_method1("update", (ancestors,))?;
                let pages = view.getattr("__dict__")?.call_method1("setdefault", ("_ui_pages", PyDict::new(py)))?;
                let cap = module.getattr("ROW_CAP")?;
                for entry in limits.try_iter()? {
                    let [page, limit] = unpack::<2>(&entry?)?;
                    let shown = pages.call_method1("get", (&page, &cap))?;
                    pages.set_item(&page, py.import("builtins")?.call_method1("max", (shown, limit))?)?;
                }
                view.setattr("_ui_mode", "")?;
                view.setattr("_ui_filter", "")?;
                self.renderer()?.call_method1("ui_reveal_row", (if self.verb == "revealnode" { rk.as_str().into_bound_py_any(py)? } else { target },))?;
                self.rebuild()?;
            } else if selected {
                self.flash("This selection has no occurrence in the Project tree")?;
            }
        } else if self.is(&["item"]) && self.kind_is(&["sheet-page", "sheet-frame", "saved-scene", "sheet-pages", "saved-scenes"]) {
            self.select(empty(py))?;
            self.inspect(&rk)?;
        } else if self.kind == "project-file" && self.is(&["item", "open", "edit", "linkrelink"]) {
            // A project file row: one click shows its tab (opening it);
            // Locate… on a missing one finds where it went.
            let what = match self.verb.as_str() {
                "edit" => "edit",
                "linkrelink" => "locate",
                _ => "open",
            };
            glue.getattr("shell_project")?.call_method1("tree_action", (win, what, &ident))?;
        } else if self.kind == "project-files" && self.is(&["item", "open"]) {
        } else if self.is(&["open"]) && self.kind_is(&["sheet-page", "sheet-frame", "saved-scene"]) {
            let target = view.getattr("_ui_sheet_catalog")?.call_method1("target", (&scene, &rk))?;
            if !target.is_none() {
                let [family, index] = unpack::<2>(&target)?;
                if family.eq("sheet")? {
                    win.call_method1("_show_sheet", (index,))?;
                } else {
                    win.getattr("tray")?.getattr("scenes")?.call_method1("finish_view", (scene.getattr("saved_views")?.get_item(index)?,))?;
                }
            }
        } else if self.is(&["linkselection"]) {
            let sources = core_module.call_method1("link_selection_sources", (win,))?;
            if sources.is_truthy()? {
                // The form keeps the answer's callback until it closes: it
                // holds the window and the document weakly.
                let weak = py.import("weakref")?.getattr("ref")?;
                let (held_win, held_scene, held_sources) = (weak.call1((win,))?.unbind(), weak.call1((&scene,))?.unbind(), sources.clone().unbind());
                let choose = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
                    let py = args.py();
                    let path = args.get_item(0)?;
                    let (win, scene, sources) = (held_win.bind(py).call0()?, held_scene.bind(py).call0()?, held_sources.bind(py));
                    if !win.is_none() && !scene.is_none() && sources.contains(&path)? && win.getattr("viewport")?.getattr("scene")?.is(&scene) {
                        let select = fields!(py; "uids" => sources.get_item(&path)?);
                        win.call_method("_choose_link_contents", (path,), Some(&fields!(py; "select" => select)))?;
                    }
                    Ok(())
                })?;
                glue.getattr("chrome_forms")?.call_method("ask_choice", (win, "Link selection", "Open source document", list(&sources)?), Some(&fields!(py; "done" => choose)))?;
            } else {
                self.flash("Select groups in another saved, open document first")?;
            }
        } else if self.is(&["linkregistry"]) {
            if let Some(path) = chosen(py, "getSaveFileName", (win, "Move registry to project", "", "Built project (*.builtproject)"))? {
                py.import("ingetrazo_core.project")?.call_method1("create", (&scene, path, win.getattr("_current_path")?))?;
                win.call_method1("_relink_done", ("Project registry updated",))?;
            }
        } else if self.is(&["eye", "lock"]) && rk == "geometry:loose" {
            self.core("edit_loose_geometry", fields!(py; "field" => &self.verb))?;
            let kept = empty(py);
            for entity in list(&scene.getattr("selection")?)?.iter() {
                if scene.call_method1("entity_selectable", (&entity,))?.is_truthy()? {
                    kept.append(entity)?;
                }
            }
            scene.call_method1("select", (kept.try_iter()?,))?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["eye", "lock"]) && tag_row {
            let layer = scene.call_method1("layer_by_key", (&ident,))?;
            if !layer.is_none() {
                let args = fields!(py; "tags" => PyList::new(py, [&ident])?);
                if self.verb == "eye" {
                    args.set_item("visible", !layer.getattr("visible")?.is_truthy()?)?;
                } else {
                    args.set_item("locked", !layer.getattr("locked")?.is_truthy()?)?;
                }
                self.core("set_tag_state", args)?;
                self.changed()?;
                self.rebuild()?;
            }
        } else if self.is(&["eye"]) && self.kind == "preview-folder" {
            let visible = module.getattr("ingetrazo_core")?.getattr("tag_edits")?.call_method("folder_visible", (&scene, &ident), Some(&fields!(py; "ancestors" => false)))?;
            self.core("set_tag_state", fields!(py; "folders" => PyList::new(py, [&ident])?, "visible" => !visible.is_truthy()?))?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["item"]) && rk == "geometry:loose" {
            let mesh = scene.getattr("mesh")?;
            self.select(py.import("itertools")?.call_method1("chain", (mesh.getattr("faces")?, mesh.getattr("edges")?))?)?;
        } else if self.is(&["eye"]) && rk == "sections:all" {
            py.import("views.view_commands")?.call_method1("run", (win, "section_planes"))?;
            self.rebuild()?;
        } else if self.is(&["eye"]) && self.kind == "section" {
            self.core("edit_section_plane", fields!(py; "uid" => &ident, "field" => "eye"))?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["item"]) && self.kind_is(&["section", "sections"]) {
            let planes = empty(py);
            for plane in scene.getattr("section_planes")?.try_iter()? {
                let plane = plane?;
                if plane.getattr("uid")?.str()?.to_str()? == ident {
                    planes.append(plane)?;
                }
            }
            self.select(planes)?;
            self.inspect(&rk)?;
        } else if self.is(&["toggle"]) {
            let exp = &self.exp;
            let (add, drop_key) = (|key: &str| exp.call_method1("add", (key,)).map(drop), |key: &str| exp.call_method1("discard", (key,)).map(drop));
            if rk.starts_with("r:") || rk.starts_with("c:inmodel") || rk.starts_with("c:compmodel") {
                // Rules start open when short: remember a collapse explicitly.
                let closed = format!("-{rk}");
                if exp.contains(&rk)? {
                    drop_key(&rk)?;
                    add(&closed)?;
                } else if exp.contains(&closed)? {
                    drop_key(&closed)?;
                    add(&rk)?;
                } else {
                    add(&closed)?;
                }
            } else {
                exp.call_method1("symmetric_difference_update", (PySet::new(py, [&rk])?,))?;
            }
            self.rebuild()?;
        } else if self.is(&["cplace", "cselect", "creplace", "ckeep", "crename"]) {
            direct(py, super::chrome_tree::component_verb, (win, view, &self.verb, &rk))?;
            self.rebuild()?;
        } else if self.is(&["issues"]) {
            view.setattr("_ui_issues", !own(view, "_ui_issues")?.is_some_and(|issues| issues.is_truthy().unwrap_or(false)))?;
            self.rebuild()?;
        } else if self.is(&["mode"]) {
            glue.getattr("wgpu_library")?.call_method1("set_mode", (view, win, &rk))?;
        } else if self.is(&["item"]) && rk == "solid:active" {
            self.select(empty(py))?;
            self.inspect(&rk)?;
            self.rebuild()?;
        } else if self.is(&["filter"]) {
            view.setattr("_ui_filter", &rk)?;
            self.rebuild()?;
        } else if self.is(&["sort"]) {
            let order = py.get_type::<pyo3::types::PyInt>().call1((&rk,))?;
            py.import("ingetrazo_core.settings")?.call_method0("settings")?.call_method1("setValue", ("ui/project_tree_sort", order))?;
        } else if self.is(&["item"]) && self.kind == "exportitem" {
            let group = self.item()?;
            if !group.is_none() {
                self.select(PyList::new(py, [group])?)?;
                win.call_method0("_on_zoom_selection")?;
            }
        } else if self.is(&["exportshow"]) {
            let found = empty(py);
            for path in loads(py, &rk)?.try_iter()? {
                let group = scene.call_method1("resolve_occurrence", (path?,))?;
                if group.is_truthy()? {
                    found.append(group)?;
                }
            }
            self.select(found.try_iter()?)?;
            win.call_method0("_on_zoom_selection")?;
        } else if self.is(&["exportstatus"]) {
            self.core("patch_record_field", loads(py, &rk)?.cast_into()?)?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["importtags", "exporttags"]) {
            let save = self.verb == "exporttags";
            let picked = if save {
                self.file("getSaveFileName", "Export layer template", "layers.json", "Layer template (*.json)")?
            } else {
                self.file("getOpenFileName", "Import layer template", "", "Layer template (*.json)")?
            };
            if let Some(path) = picked {
                let done = (|| {
                    self.flash(core_module.call_method1("tag_template_file", (&scene, path, save))?.str()?.to_str()?)?;
                    if !save {
                        self.changed()?;
                        self.rebuild()?;
                    }
                    Ok(())
                })();
                told(viewport, done, true)?;
            }
        } else if self.is(&["exportpat", "exportlin"]) {
            let ext = &self.verb[6..];
            if let Some(path) = self.file("getSaveFileName", "Export drawing patterns", &format!("patterns.{ext}"), &format!("Pattern files (*.{ext})"))? {
                let done = (|| {
                    let count = core_module.call_method1("export_style_file", (&scene, &path, ext))?;
                    let name = py.import("pathlib")?.getattr("Path")?.call1((&path,))?.getattr("name")?;
                    self.flash(format!("{} patterns written to {}", count.str()?, name.str()?))
                })();
                told(viewport, done, true)?;
            }
        } else if self.is(&["editline", "tagfolder"]) {
            let edit = if self.verb == "editline" { "edit_line_type" } else { "edit_tag_folder" };
            let key = core_module.call_method1(edit, (viewport.getattr("history")?, &rk, &extra))?;
            self.changed()?;
            self.rebuild()?;
            let applied = if self.verb == "tagfolder" && rk == "new" { "foldernew" } else { self.verb.as_str() };
            self.renderer()?.call_method1("ui_drawing_applied", (applied, key))?;
        } else if self.is(&["hatchpref"]) {
            self.renderer()?.call_method1("ui_hatch_preference", (&rk, &extra))?;
        } else if self.is(&["documentpiece"]) {
            core_module.call_method1("document_piece", (viewport.getattr("history")?, &rk, loads(py, &extra)?))?;
            self.renderer()?.call_method1("ui_drawing_applied", (&self.verb, ""))?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["importlin"]) {
            if let Some(path) = self.file("getOpenFileName", "Import line types", "", "Line type files (*.lin)")? {
                let done = (|| {
                    let [count, skipped] = unpack::<2>(&core_module.call_method1("import_line_file", (viewport.getattr("history")?, path))?)?;
                    let omitted: String = ", ".into_bound_py_any(py)?.call_method1("join", (skipped,))?.extract()?;
                    let unsupported = if omitted.is_empty() { String::new() } else { format!(". Unsupported: {omitted}") };
                    self.flash(format!("{} line types imported{unsupported}", count.str()?))?;
                    self.changed()?;
                    self.rebuild()
                })();
                told(viewport, done, false)?;
            }
        } else if self.is(&["importpat"]) {
            if let Some(path) = self.file("getOpenFileName", "Import hatch patterns", "", "Pattern files (*.pat)")? {
                let done = (|| {
                    let count = core_module.call_method1("import_hatch_file", (viewport.getattr("history")?, &path))?;
                    let name = py.import("pathlib")?.getattr("Path")?.call1((&path,))?.getattr("name")?;
                    let message = py.import("ingetrazo_core.i18n")?.getattr("tr")?.call(("{n} hatch patterns imported from {name}",), Some(&fields!(py; "n" => count, "name" => name)))?;
                    self.flash(message.str()?.to_str()?)?;
                    self.rebuild()?;
                    self.changed()
                })();
                told(viewport, done, true)?;
            }
        } else if self.is(&["tagstate"]) {
            self.core("set_tag_state", loads(py, &rk)?.cast_into()?)?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["drawingpatch"]) {
            let [targets, patch] = unpack::<2>(&loads(py, &rk)?)?;
            let value = py.import("json")?.call_method1("dumps", (patch,))?;
            let result = self.core("edit_tag", fields!(py; "tags" => targets, "field" => "style", "value" => value))?;
            if result.is_none() {
                return Ok(());
            }
            for field in ["rejected", "error"] {
                if result.call_method1("get", (field,))?.is_truthy()? {
                    // `result["rejected"] or result["error"]`, as the dock tells it.
                    let rejected = result.get_item("rejected")?;
                    let reason = if rejected.is_truthy()? { rejected } else { result.get_item("error")? };
                    return self.flash(reason.str()?.to_str()?);
                }
            }
            self.renderer()?.call_method1("ui_drawing_applied", (&self.verb, ""))?;
            self.rebuild()?;
            self.changed()?;
        } else if self.is(&["lens"]) {
            let lens = py.get_type::<pyo3::types::PyInt>().call1((if rk.is_empty() { 0.into_bound_py_any(py)? } else { rk.as_str().into_bound_py_any(py)? },))?;
            view.setattr("_ui_lens", lens)?;
            module.call_method1("apply_dock_width", (view,))?;
            self.rebuild()?;
        } else if self.is(&["print", "color", "linetype", "weight", "hatch"]) && named.is_some() && !self.linked {
            let tag = self.data.get_item("recipes")?.get_item(named.as_deref())?.call_method1("get", ("tag",))?;
            let layer = direct(py, super::chrome_tree::layer, (&scene, tag))?;
            if !layer.is_none() {
                self.core("edit_tag", fields!(py; "tags" => PyList::new(py, [layer.getattr("key")?])?, "field" => &self.verb, "value" => &extra))?;
                self.rebuild()?;
                self.changed()?;
            }
        } else if self.is(&["measure"]) {
            module.call_method1("_measure", (view, win))?;
            self.rebuild()?;
        } else if self.is(&["exportcsv"]) {
            direct(py, super::chrome_tree::export_csv, (win,))?;
        } else if self.is(&["expandall"]) {
            self.exp.call_method1("update", (core_module.call_method1("expand_project_folders", (&scene, self.data.call_method1("get", ("nodes", PyDict::new(py)))?))?,))?;
            self.rebuild()?;
        } else if self.is(&["collapseall"]) {
            self.exp.call_method0("clear")?;
            self.rebuild()?;
        } else if self.is(&["loose"]) && self.kind == "m" {
            // "Show more": the next page of that list.
            let pages = view.getattr("__dict__")?.call_method1("setdefault", ("_ui_pages", PyDict::new(py)))?;
            let cap = module.getattr("ROW_CAP")?;
            pages.set_item(&ident, pages.call_method1("get", (&ident, &cap))?.add(&cap)?)?;
            self.rebuild()?;
        } else if self.is(&["node"]) && self.linked {
            // A linked row selects the link placement(s) it stands for (a
            // node inside a file selects nothing): the inspector says what
            // it is. Its parts come with "Select contents".
            view.setattr("_ui_insp_node", py.None())?;
            self.select(if self.node_kind(&["link", "linked"])? { self.placements()? } else { empty(py) })?;
            self.inspect(node.as_deref().unwrap_or(""))?;
        } else if self.is(&["contents"]) {
            self.select(self.members()?)?;
        } else if self.is(&["linkfinder"]) && self.linked {
            if self.tnode.call_method1("get", ("path",))?.is_truthy()? {
                py.import("ingetrazo_core.desktop_open")?.call_method1("reveal", (self.tnode.get_item("path")?,))?;
            }
        } else if self.is(&["linkopen", "linkreload", "linkunlink", "linkrelink", "linkresettags", "linksubset", "linknested", "linkpack"]) && self.linked {
            let groups = self.placements()?;
            if groups.is_empty() {
                return Ok(());
            }
            if self.verb != "linkopen" && self.tnode.call_method1("get", ("via",))?.is_truthy()? {
                let via = self.tnode.get_item("via")?.str()?.extract::<String>()?;
                return self.flash(format!("{} comes inside {via}: reload or unlink {via}", self.tnode.call_method1("get", ("name",))?.str()?));
            }
            if self.is(&["linksubset", "linknested"]) && groups.len() != 1 {
                return self.flash("Select one occurrence of this linked file first");
            }
            if self.verb == "linkresettags" {
                self.core("reset_link_tags", fields!(py; "path" => self.tnode.get_item("path")?))?;
                self.changed()?;
            } else if self.verb == "linkunlink" {
                let addresses = empty(py);
                for group in groups.iter() {
                    addresses.append(self.tree.call_method1("address", (group,))?)?;
                }
                self.core("unlink_placements", fields!(py; "placements" => addresses))?;
                self.changed()?;
            } else {
                module.call_method1("_link_action", (win, view, groups.get_item(0)?, &self.verb))?;
            }
            self.rebuild()?;
        } else if self.is(&["active", "filehere", "file", "newchild", "delete", "kind", "rename", "hidemembers"])
            && (self.linked
                || (self.verb == "file" && link_structure.call_method1("is_linked_key", (if self.kind == "n" { &ident } else { &rk },))?.is_truthy()?))
        {
            self.flash("A linked file's structure is read-only here: open its source to change it")?;
        } else if self.is(&["node", "item", "loose"]) {
            let inspected = named.as_deref().filter(|_| self.verb == "node");
            if inspected.is_some() {
                view.setattr("_ui_insp_node", py.None())?; // set after the selection lands
            }
            self.select(self.members()?)?;
            if let Some(node) = inspected {
                self.inspect(node)?;
            }
        } else if self.is(&["reveal"]) {
            let group = self.item()?;
            let mut key = if group.is_none() { none(py) } else { self.data.call_method1("get", ("node_of", PyDict::new(py)))?.call_method1("get", (id(&group),))? };
            let nodes = self.data.call_method1("get", ("nodes", PyDict::new(py)))?;
            while key.is_truthy()? {
                self.expand(format!("n:{}", key.str()?))?;
                key = nodes.call_method1("get", (&key, PyDict::new(py)))?.call_method1("get", ("parent",))?;
            }
            if !group.is_none() {
                self.select(PyList::new(py, [group])?)?;
            }
            self.rebuild()?;
        } else if self.is(&["isolate"]) {
            let keys = empty(py);
            let by_row = self.tree.is_truthy()?;
            for group in self.shown()?.iter() {
                keys.append(if by_row { self.tree.call_method1("row_id", (group,))? } else { group.getattr("uid")? })?;
            }
            module.call_method1("_isolate", (win, keys))?;
        } else if self.is(&["open"]) {
            let group = self.item()?;
            if !group.is_none() {
                let parents = py.import("core.outliner")?.call_method1("path_to", (scene.getattr("groups")?, &group))?;
                if !parents.is_none() {
                    let path = list(&parents)?;
                    path.append(group)?;
                    glue.getattr("tree_navigation")?.call_method1("open_to", (viewport, path))?;
                }
            }
        } else if self.is(&["zoom"]) {
            self.select(self.shown()?)?;
            win.call_method0("_on_zoom_selection")?;
        } else if self.is(&["eye"]) {
            let edits = || module.getattr("ingetrazo_core")?.getattr("tag_edits");
            let folders = match named.as_deref() {
                Some(node) => self.link_folders(node)?,
                None => empty(py),
            };
            if !folders.is_empty() {
                // A linked file (or one of its tag folders): its folder in
                // the host, so its nested files follow.
                let mut all = true;
                for folder in folders.iter() {
                    if !edits()?.call_method1("folder_visible", (&scene, folder))?.is_truthy()? {
                        all = false;
                        break;
                    }
                }
                self.core("set_tag_state", fields!(py; "folders" => folders, "visible" => !all))?;
            } else if let Some(node) = named.as_deref() {
                let tags = self.tags(node)?;
                if !tags.is_empty() {
                    let layer = scene.call_method1("layer", (tags.get_item(0)?,))?;
                    let shown = if layer.is_none() {
                        false
                    } else if self.linked {
                        edits()?.call_method1("effective_visible", (&scene, &layer))?.is_truthy()?
                    } else {
                        layer.getattr("visible")?.is_truthy()?
                    };
                    self.core("set_tag_state", fields!(py; "tags" => tags, "visible" => !shown))?;
                }
            } else {
                let groups = self.members()?;
                if !groups.is_empty() {
                    let command = if truthy(&groups.get_item(0)?, "hidden")? { "unhide" } else { "hide" };
                    self.core(command, fields!(py; "placements" => self.addresses(&groups)?))?;
                }
            }
            if viewport.hasattr("notify_scene_changed")? {
                self.changed()?;
            } else {
                viewport.call_method0("update")?;
            }
        } else if self.is(&["lock", "lockitem"]) {
            if let Some(node) = named.as_deref() {
                let tags = if !self.tree.is_none() && self.node_kind(&["link", "linked"])? {
                    self.tree.call_method1("file_tags", (node,))?
                } else {
                    self.tags(node)?.into_any()
                };
                if tags.is_truthy()? {
                    let layer = scene.call_method1("layer", (tags.get_item(0)?,))?;
                    self.core("set_tag_state", fields!(py; "tags" => tags, "locked" => !truthy(&layer, "locked")?))?;
                }
            } else {
                let groups = self.members()?;
                if !groups.is_empty() {
                    let locked = truthy(&groups.get_item(0)?, "locked")?;
                    self.core("lock", fields!(py; "placements" => self.addresses(&groups)?, "locked" => !locked))?;
                }
            }
            viewport.call_method0("update")?;
        } else if self.is(&["hidemembers", "hideitem"]) {
            let groups = self.members()?;
            if !groups.is_empty() {
                self.core("hide", fields!(py; "placements" => self.addresses(&groups)?))?;
                viewport.call_method0("update")?;
            }
        } else if self.is(&["fileto"]) {
            self.select(self.members()?)?;
            direct(py, super::chrome_inspector::file_to, (win, self.members()?.len()))?;
        } else if self.is(&["parent"]) && named.is_some() {
            self.core("edit_structure_node", fields!(py; "node" => named.as_deref(), "field" => "parent", "value" => &extra))?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["rename"]) && self.kind == "section" {
            self.core("edit_section_plane", fields!(py; "uid" => &ident, "field" => "name", "value" => &extra))?;
            self.changed()?;
        } else if self.is(&["rename"]) {
            if let Some(node) = named.as_deref() {
                self.core("edit_structure_node", fields!(py; "node" => node, "field" => "name", "value" => &extra))?;
                self.changed()?;
                self.rebuild()?;
            } else if !self.item()?.is_none() {
                self.core("rename_placement", fields!(py; "placement" => self.address(&self.item()?)?, "name" => &extra))?;
                viewport.call_method0("update")?;
            }
        } else if self.is(&["fileitem"]) && !self.item()?.is_none() {
            let group = self.item()?;
            self.core("file_placements", fields!(py; "placements" => PyList::new(py, [self.address(&group)?])?, "node" => &extra))?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["file", "filehere"]) {
            let key = if self.verb == "filehere" {
                node.clone()
            } else if self.kind == "n" {
                Some(ident.clone())
            } else {
                Some(rk.clone())
            };
            let group_type = py.import("core.group")?.getattr("Group")?;
            let placements = empty(py);
            for entity in scene.getattr("selection")?.try_iter()? {
                let entity = entity?;
                if entity.is_instance(&group_type)? {
                    placements.append(self.address(&entity)?)?;
                }
            }
            if let Some(key) = key.filter(|key| !key.is_empty() && !placements.is_empty()) {
                let count = placements.len();
                self.core("file_placements", fields!(py; "placements" => placements, "node" => &key))?;
                self.changed()?;
                self.rebuild()?;
                let registry = py.import("core.bim_registry")?.call_method1("get", (&scene,))?;
                let path: String = " › ".into_bound_py_any(py)?.call_method1("join", (structure.call_method1("path", (registry, &key))?,))?.extract()?;
                self.flash(format!("Filed {count} to {path}"))?;
            }
        } else if self.is(&["active"]) {
            self.core("edit_structure_node", fields!(py; "node" => node.as_deref(), "field" => "active", "value" => ""))?;
            self.changed()?;
            self.rebuild()?;
        } else if self.is(&["kind"]) && tag_row {
            if let Some(class) = self.class_named(&extra)? {
                self.core("promote_tag", fields!(py; "tag" => &ident, "ifc_class" => class))?;
                self.changed()?;
                self.rebuild()?;
            }
        } else if self.is(&["tagselect", "tagisolate"]) && tag_row {
            glue.call_method1("tree_tag_action", (win, &ident, self.verb == "tagisolate"))?;
        } else if self.is(&["kind"]) && named.is_some() {
            let node = named.as_deref().unwrap_or("");
            let recipe = self.recipe_of(self.structure_nodes()?.call_method1("get", (node, PyDict::new(py)))?)?;
            match self.class_named(&extra)? {
                Some(class) => recipe.set_item("class", class)?,
                None => {
                    recipe.call_method1("pop", ("class", py.None()))?;
                }
            }
            self.set_rule(node, recipe)?;
        } else if self.is(&["new", "newchild"]) {
            let registry = py.import("core.bim_registry")?.call_method1("get", (&scene,))?;
            let nodes = registry.call_method1("setdefault", ("structure", PyDict::new(py)))?.call_method1("setdefault", ("nodes", PyDict::new(py)))?;
            let parent = if self.verb == "newchild" { node.as_deref().into_bound_py_any(py)? } else { none(py) };
            let mut order = (-1).into_bound_py_any(py)?;
            for sibling in nodes.call_method0("values")?.try_iter()? {
                let sibling = sibling?;
                if sibling.get_item("parent")?.eq(&parent)? && sibling.get_item("order")?.gt(&order)? {
                    order = sibling.get_item("order")?;
                }
            }
            let uid = py.import("core.group")?.call_method0("new_uid")?;
            nodes.set_item(uid, fields!(py; "name" => "New node", "parent" => &parent, "order" => order.add(1)?))?;
            if parent.is_truthy()? {
                self.expand(format!("n:{}", parent.str()?))?;
            }
            execute()?.call1((viewport, py.import("ingetrazo_core.history")?.getattr("SetRegistryCommand")?.call1((registry,))?))?;
        } else if self.is(&["delete"]) && named.is_some() {
            let node = named.as_deref().unwrap_or("");
            let count = structure.call_method1("members", (&scene, node))?.len()?;
            let name = self.data.call_method1("get", ("nodes", PyDict::new(py)))?.call_method1("get", (node, PyDict::new(py)))?.call_method1("get", ("name", ""))?;
            let body = if count != 0 { format!("Its {count} items move to the parent node.") } else { "It has no items.".to_owned() };
            view.setattr("_ui_pending", ("delete", node))?;
            self.renderer()?.call_method1("ui_set_prompt", (format!("Delete “{}”?", name.str()?), body, PyList::new(py, [("yes", "Delete"), ("no", "Cancel")])?))?;
        } else if self.is(&["propose"]) {
            let registry = py.import("core.bim_registry")?.call_method1("get", (&scene,))?;
            let proposal = structure.call_method1("bootstrap", (&scene, &registry))?;
            view.setattr("_ui_pending", ("propose", &proposal))?;
            let nodes = proposal.get_item("structure")?.get_item("nodes")?.len()?;
            let mut filed = 0;
            for members in proposal.get_item("filing")?.call_method0("values")?.try_iter()? {
                filed += members?.len()?;
            }
            let conflicts = proposal.get_item("conflicts")?.len()?;
            if nodes == 0 {
                // Nothing to propose: say why and offer the way that works.
                view.setattr("_ui_pending", py.None())?;
                self.renderer()?.call_method1(
                    "ui_set_prompt",
                    (
                        "Nothing to propose from tags",
                        "Every item is on the same tag, so there is no grouping to build nodes from. Start one with New node, then File to… from the selection.",
                        PyList::new(py, [("new", "New node"), ("no", "Close")])?,
                    ),
                )?;
                return Ok(());
            }
            let replace = if structure.call_method1("_nodes", (&registry,))?.is_truthy()? { " It replaces the current structure." } else { "" };
            self.renderer()?.call_method1(
                "ui_set_prompt",
                (
                    "Propose structure from tags",
                    format!("{nodes} nodes, {filed} items filed, {conflicts} to review.{replace}"),
                    PyList::new(py, [("yes", "Apply"), ("no", "Cancel")])?,
                ),
            )?;
        } else if self.is(&["prompt"]) && rk == "new" {
            module.call_method1("_tree_action", (win, view, format!("new{SEP}")))?;
        } else if self.is(&["prompt"]) {
            let pending = attr(view, "_ui_pending")?;
            view.setattr("_ui_pending", py.None())?;
            if rk == "yes" && pending.is_truthy()? {
                let [what, arg] = unpack::<2>(&pending)?;
                let what: String = what.extract()?;
                let answer = extra.trim();
                let scenes = || -> Reply<'py> { attr(&win.getattr("tray")?, "scenes") };
                match what.as_str() {
                    "propose" => {
                        execute()?.call1((viewport, structure.call_method1("apply_bootstrap", (&scene, arg))?))?;
                    }
                    "delete" => {
                        direct(py, delete_node, (viewport, arg))?;
                    }
                    "lib" => {
                        glue.getattr("wgpu_library")?.call_method1("confirmed", (win, view, arg, &extra))?;
                    }
                    "visibility_set" => {
                        let [document, operation, key] = unpack::<3>(&arg)?;
                        if document.is(&scene) && self.core("edit_visibility_set", fields!(py; "verb" => operation, "key" => key, "name" => &extra))?.get_item("ok")?.is_truthy()? {
                            self.changed()?;
                            view.call_method0("update")?;
                        }
                    }
                    "scene_rename" if !answer.is_empty() => {
                        let panel = scenes()?;
                        if !panel.is_none() {
                            panel.call_method1("rename_view", (arg, extra.as_str().into_bound_py_any(py)?.call_method0("strip")?))?;
                        }
                    }
                    "scene_delete" => {
                        let panel = scenes()?;
                        if !panel.is_none() {
                            panel.call_method1("delete_views", (arg,))?;
                        }
                    }
                    _ => {}
                }
            }
        } else if self.is(&["edit"]) {
            let (what, arg) = partition(&rk, ':');
            let stripped = extra.as_str().into_bound_py_any(py)?.call_method0("strip")?;
            match what {
                "section" => {
                    let (uid, field) = partition(arg, ':');
                    self.core("edit_section_plane", fields!(py; "uid" => uid, "field" => field, "value" => &extra))?;
                    self.changed()?;
                    self.rebuild()?;
                }
                "nodename" => {
                    self.core("edit_structure_node", fields!(py; "node" => arg, "field" => "name", "value" => &extra))?;
                    self.changed()?;
                    self.rebuild()?;
                }
                "predef" | "objtype" => {
                    let recipe = self.recipe_of(self.structure_nodes()?.get_item(arg)?)?;
                    let field = if what == "predef" { "predefined_type" } else { "object_type" };
                    if stripped.is_truthy()? {
                        recipe.set_item(field, &stripped)?;
                    } else {
                        recipe.call_method1("pop", (field, py.None()))?;
                    }
                    self.set_rule(arg, recipe)?;
                }
                "segments" => {
                    let most = core_module.getattr("curve_segments")?.getattr("MAX_SEGMENTS")?;
                    let selection = list(&scene.getattr("selection")?)?;
                    let shape = direct(py, super::chrome_inspector::curve_of, (&scene, &selection))?;
                    let count = py.get_type::<pyo3::types::PyInt>().call1((&stripped,))?;
                    let cannot = || PyValueError::new_err("This curve cannot be rebuilt");
                    if shape.is_none() || !shape.getattr("flat")?.is_truthy()? {
                        return Err(cannot());
                    }
                    let least = shape.getattr("minimum")?;
                    if least.gt(&count)? || count.gt(&most)? {
                        return Err(PyValueError::new_err(format!("Between {} and {}", least.str()?, most.str()?)));
                    }
                    if count.ne(shape.getattr("segments")?)? {
                        let out = self.core("resegment_curve", fields!(py; "edge" => selection.get_item(0)?.getattr("key")?, "segments" => count))?;
                        if out.is_truthy()? && out.call_method1("get", ("rejected",))?.is_truthy()? {
                            return Err(cannot());
                        }
                        viewport.call_method0("update")?;
                    }
                }
                "libmat" => {
                    glue.getattr("wgpu_library")?.call_method1("edit_field", (win, view, arg, &extra))?;
                }
                "groupname" => {
                    glue.call_method1("inspector_rename", (viewport, arg, &extra))?;
                }
                _ => {}
            }
            view.setattr("_ui_insp_sig", py.None())?;
            self.rebuild()?;
        } else if self.is(&["showall"]) {
            module.call_method1("_show_all", (viewport,))?;
        } else if self.is(&["unfiled"]) {
            self.select(self.unfiled()?)?;
        }
        Ok(())
    }

    /// The IFC class whose plain-language kind is `label`.
    fn class_named(&self, label: &str) -> PyResult<Option<Obj<'py>>> {
        for entry in self.module.getattr("KINDS")?.call_method0("items")?.try_iter()? {
            let [class, kind] = unpack::<2>(&entry?)?;
            if class.is_truthy()? && kind.eq(label)? {
                return Ok(Some(class));
            }
        }
        Ok(None)
    }
}

/// Run a core protocol command (docs/core-protocol.md §1): the UI edits the
/// model through these, by key. A key outside the open context is told, not
/// raised.
pub(super) fn core_command<'py>(vp: &Obj<'py>, command: &Obj<'py>, args: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
    let py = vp.py();
    let commands = py.import("ingetrazo_core.command_protocol")?;
    match commands.getattr("run")?.call((vp.getattr("history")?, command), args) {
        Err(error) if error.is_instance_of::<PyKeyError>(py) => {
            let text: String = error.value(py).str()?.extract()?;
            flash(vp, text.trim_matches(|c| c == '\'' || c == '"'))?;
            Ok(none(py))
        }
        other => other,
    }
}

pub(super) fn delete_node<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (viewport, key) = (a.get_item(0)?, a.get_item(1)?);
    let structure = py.import("ingetrazo_core.structure")?;
    let history = py.import("ingetrazo_core.history")?;
    let scene = viewport.getattr("scene")?;
    let registry = py.import("core.bim_registry")?.call_method1("get", (&scene,))?;
    let section = registry.get_item("structure")?;
    if structure.call_method1("tree", (&registry,))?.get_item(&key)?.is_truthy()? {
        return Err(PyValueError::new_err("Remove its child nodes first"));
    }
    let nodes = section.get_item("nodes")?;
    let parent = nodes.get_item(&key)?.get_item("parent")?;
    let uids = structure.call_method1("members", (&scene, &key))?;
    let commands = empty(py);
    if uids.is_truthy()? {
        if !parent.is_truthy()? {
            return Err(PyValueError::new_err("File its items elsewhere before deleting a top node"));
        }
        commands.append(structure.call_method1("file", (&scene, &uids, parent))?)?;
    }
    nodes.del_item(&key)?;
    if section.call_method1("get", ("active",))?.eq(&key)? {
        section.call_method1("pop", ("active",))?;
    }
    commands.insert(0, history.getattr("SetRegistryCommand")?.call((&registry,), Some(&fields!(py; "refiled_uids" => uids)))?)?;
    render(py)?.getattr("tree_navigation")?.call_method1("execute", (&viewport, history.getattr("CompoundCommand")?.call1((commands,))?))?;
    Ok(none(py))
}

/// An editable inspector value (ui.rs inspector_rows).
fn edit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mark: String = ui(py)?.getattr("ED")?.extract()?;
    format!("{mark}edit{mark}{}{mark}{}", a.get_item(0)?.str()?, a.get_item(1)?.str()?).into_bound_py_any(py)
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_tree_action", Mode::Function, tree_action, &["win", "view", "action"], "", ""),
    ("_edit", Mode::Function, edit, &["id_", "text"], "", ""),
];

pub(super) fn register(namespace: &Bound<PyDict>) -> PyResult<()> {
    install(namespace.as_any(), namespace, FUNCTIONS)
}
