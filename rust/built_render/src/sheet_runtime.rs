// SPDX-License-Identifier: GPL-3.0-or-later
//! The large sheet on the window: `built_render.chrome_sheet`.
//!
//! One surface for every place where you browse, pick or set something
//! bigger than a menu (Library, start page, Settings, Export, Extensions,
//! About). `ui/large_sheet.rs` draws it; the pattern is in
//! `docs/ui-patterns.md`.
//!
//! A surface is a **provider**: it builds the sheet ([`Sheet`], [`Pane`],
//! [`item`], [`verb`]) from its own state each time something changed, and
//! answers what the sheet reports (`act(ctx, verb, args)`):
//!
//! | verb | args |
//! |---|---|
//! | `search` | the search field's text |
//! | `side` / `fold` | a side row, its chevron |
//! | `tab` | a header tab |
//! | `verb` | a header verb, footer link or buttons entry |
//! | `focus` / `open` | an item clicked or arrowed to / double-clicked, Enter |
//! | `menu` | item, entry: a right-click menu entry |
//! | `pick` | a row of the detail pane |
//! | `headmenu` | menu, option |
//!
//! A `fields` block holds a `built_render.chrome_forms.Form`: its edits apply
//! through the form's `on_change`, its inline buttons through `on_button`.
//! Providers written in Python subclass [`Provider`]; Rust ones (Export,
//! About…) answer the same calls.
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList, PySet, PyString, PyTuple},
};

pub const SEP: &str = "\x1f";

fn opt<'py>(o: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    o.getattr(name).ok().filter(|v| !v.is_none())
}

/// This submodule, through its attributes (tests replace `in_shell`).
fn this(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("built_render")?.getattr("chrome_sheet")
}

fn none(py: Python<'_>) -> Bound<'_, PyAny> {
    py.None().into_bound(py)
}

// ---- building a sheet ------------------------------------------------------------------

/// A tile, a list row or a side row. `icon`: a texture key sent with
/// `Sheet.icon`; `hex`: a colour when there is no picture; `dot`: ok | warn
/// | error | accent, with `dot_text` after it; `menu`: [(entry id, label)]
/// for a right click ("-" is a rule); `fixed`: drawn but never focused.
#[pyfunction]
#[pyo3(signature = (id, label, *, sub=None, right=None, icon=None, hex=None, badge=None, dot=None, dot_text=None, depth=None, on=false, dim=false, fixed=false, tip=None, menu=None, kind=None, fold=None))]
#[allow(clippy::too_many_arguments)]
pub fn item<'py>(
    id: &Bound<'py, PyAny>,
    label: &Bound<'py, PyAny>,
    sub: Option<Bound<'py, PyAny>>,
    right: Option<Bound<'py, PyAny>>,
    icon: Option<Bound<'py, PyAny>>,
    hex: Option<Bound<'py, PyAny>>,
    badge: Option<Bound<'py, PyAny>>,
    dot: Option<Bound<'py, PyAny>>,
    dot_text: Option<Bound<'py, PyAny>>,
    depth: Option<Bound<'py, PyAny>>,
    on: bool,
    dim: bool,
    fixed: bool,
    tip: Option<Bound<'py, PyAny>>,
    menu: Option<Bound<'py, PyAny>>,
    kind: Option<Bound<'py, PyAny>>,
    fold: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyDict>> {
    let py = id.py();
    let d = PyDict::new(py);
    d.set_item("id", id.str()?)?;
    d.set_item("label", label.str()?)?;
    for (k, v) in [("sub", sub), ("right", right), ("icon", icon), ("hex", hex), ("badge", badge), ("dot", dot), ("dot_text", dot_text), ("tip", tip), ("kind", kind)] {
        if let Some(v) = v.filter(|v| v.is_truthy().unwrap_or(false)) {
            d.set_item(k, v.str()?)?;
        }
    }
    if let Some(depth) = depth.filter(|v| v.is_truthy().unwrap_or(false)) {
        d.set_item("depth", py.import("builtins")?.getattr("int")?.call1((depth,))?)?;
    }
    for (k, v) in [("on", on), ("dim", dim), ("fixed", fixed)] {
        if v {
            d.set_item(k, true)?;
        }
    }
    if let Some(menu) = menu.filter(|v| v.is_truthy().unwrap_or(false)) {
        let rows = PyList::empty(py);
        for entry in menu.try_iter()? {
            let entry = entry?;
            rows.append(PyList::new(py, [entry.get_item(0)?.str()?, entry.get_item(1)?.str()?])?)?;
        }
        d.set_item("menu", rows)?;
    }
    if let Some(fold) = fold {
        d.set_item("fold", if fold.is_truthy()? { 1 } else { 0 })?;
    }
    Ok(d)
}

/// A button: `primary` is the accent one, `quiet` a text button.
#[pyfunction]
#[pyo3(signature = (id, label, *, key=None, primary=false, quiet=false, enabled=None, tip=None))]
#[allow(clippy::too_many_arguments)]
pub fn verb<'py>(
    id: &Bound<'py, PyAny>,
    label: &Bound<'py, PyAny>,
    key: Option<Bound<'py, PyAny>>,
    primary: bool,
    quiet: bool,
    enabled: Option<Bound<'py, PyAny>>,
    tip: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyDict>> {
    let py = id.py();
    let d = PyDict::new(py);
    d.set_item("id", id.str()?)?;
    d.set_item("label", label)?;
    d.set_item("key", key.unwrap_or_else(|| "".into_pyobject(py).expect("str").into_any()))?;
    d.set_item("role", if primary { 1 } else if quiet { 3 } else { 0 })?;
    d.set_item("enabled", match enabled {
        Some(e) => e.is_truthy()?,
        None => true,
    })?;
    d.set_item("tip", tip.unwrap_or_else(|| "".into_pyobject(py).expect("str").into_any()))?;
    Ok(d)
}

fn verb_of<'py>(id: &Bound<'py, PyAny>, label: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    verb(id, label, None, false, false, None, None)
}

/// The content or detail pane: blocks, top to bottom.
#[pyclass(module = "built_render.chrome_sheet", subclass, dict)]
pub struct Pane {
    #[pyo3(get, set)]
    title: Py<PyAny>,
    #[pyo3(get, set)]
    sub: Py<PyAny>,
    #[pyo3(get, set)]
    empty: Py<PyAny>,
    #[pyo3(get, set)]
    width: f64,
    #[pyo3(get, set)]
    blocks: Py<PyAny>,
    #[pyo3(get, set)]
    forms: Py<PyAny>,
}

fn empty_str(py: Python<'_>) -> Py<PyAny> {
    "".into_pyobject(py).expect("str").into_any().unbind()
}

impl Pane {
    fn make(py: Python<'_>, title: Option<Py<PyAny>>, sub: Option<Py<PyAny>>, empty: Option<Py<PyAny>>, width: f64) -> Self {
        Self {
            title: title.unwrap_or_else(|| empty_str(py)),
            sub: sub.unwrap_or_else(|| empty_str(py)),
            empty: empty.unwrap_or_else(|| empty_str(py)),
            width,
            blocks: PyList::empty(py).into_any().unbind(),
            forms: PyList::empty(py).into_any().unbind(),
        }
    }

    /// A block of `kind` with the given values, leaving out the empty ones
    /// (None, "", [], 0), as the chrome's defaults.
    fn add<'py>(slf: &Bound<'py, Self>, kind: &str, kw: Vec<(&str, Bound<'py, PyAny>)>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let empties = PyTuple::new(py, [none(py), "".into_pyobject(py)?.into_any(), PyList::empty(py).into_any(), 0i32.into_pyobject(py)?.into_any(), 0.0f64.into_pyobject(py)?.into_any()])?;
        let b = PyDict::new(py);
        b.set_item("kind", kind)?;
        for (k, v) in kw {
            if !empties.contains(&v)? {
                b.set_item(k, v)?;
            }
        }
        slf.borrow().blocks.bind(py).call_method1("append", (b,))?;
        Ok(slf.clone())
    }
}

fn s<'py>(py: Python<'py>, v: &str) -> Bound<'py, PyAny> {
    PyString::new(py, v).into_any()
}

#[pymethods]
impl Pane {
    #[new]
    #[pyo3(signature = (title=None, sub=None, empty=None, width=0.0))]
    fn new(py: Python<'_>, title: Option<Py<PyAny>>, sub: Option<Py<PyAny>>, empty: Option<Py<PyAny>>, width: f64) -> Self {
        Self::make(py, title, sub, empty, width)
    }

    #[pyo3(signature = (text, right=None))]
    fn caption<'py>(slf: &Bound<'py, Self>, text: Bound<'py, PyAny>, right: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        Pane::add(slf, "caption", vec![("text", text), ("right", right.unwrap_or_else(|| s(py, "")))])
    }

    /// A paragraph; `tone`: dim (default), faint, warn, error, ok.
    #[pyo3(signature = (text, tone=None))]
    fn text<'py>(slf: &Bound<'py, Self>, text: Bound<'py, PyAny>, tone: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        Pane::add(slf, "text", vec![("text", text), ("tone", tone.unwrap_or_else(|| s(py, "")))])
    }

    /// Text to copy as it stands (a command): monospace in a well.
    fn code<'py>(slf: &Bound<'py, Self>, text: Bound<'py, PyAny>) -> PyResult<Bound<'py, Self>> {
        Pane::add(slf, "code", vec![("text", text)])
    }

    fn separator<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        Pane::add(slf, "separator", vec![])
    }

    /// The detail's name line: a swatch or picture, the name (★ when
    /// `star`), quiet words on the right, a line under it.
    #[pyo3(signature = (text, *, sub=None, right=None, star=false, icon=None, hex=None))]
    fn heading<'py>(slf: &Bound<'py, Self>, text: Bound<'py, PyAny>, sub: Option<Bound<'py, PyAny>>, right: Option<Bound<'py, PyAny>>, star: bool, icon: Option<Bound<'py, PyAny>>, hex: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let e = || s(py, "");
        Pane::add(slf, "heading", vec![("text", text), ("sub", sub.unwrap_or_else(e)), ("right", right.unwrap_or_else(e)), ("tone", s(py, if star { "accent" } else { "" })), ("icon", icon.unwrap_or_else(e)), ("hex", hex.unwrap_or_else(e))])
    }

    #[pyo3(signature = (icon=None, hex=None, size=0.0))]
    fn picture<'py>(slf: &Bound<'py, Self>, icon: Option<Bound<'py, PyAny>>, hex: Option<Bound<'py, PyAny>>, size: f64) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        Pane::add(slf, "picture", vec![("icon", icon.unwrap_or_else(|| s(py, ""))), ("hex", hex.unwrap_or_else(|| s(py, ""))), ("size", size.into_pyobject(py)?.into_any())])
    }

    /// (label, value) or (label, value, tone) rows.
    fn kv<'py>(slf: &Bound<'py, Self>, rows: &Bound<'py, PyAny>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let out = PyList::empty(py);
        for r in rows.try_iter()? {
            let r = r?;
            let tone = if r.len()? > 2 { r.get_item(2)?.str()? } else { PyString::new(py, "") };
            out.append(PyList::new(py, [r.get_item(0)?.str()?, r.get_item(1)?.str()?, tone])?)?;
        }
        Pane::add(slf, "kv", vec![("rows", out.into_any())])
    }

    /// `verb()` dicts; align right (default), left, or fill (stacked).
    #[pyo3(signature = (buttons, align=None))]
    fn buttons<'py>(slf: &Bound<'py, Self>, buttons: &Bound<'py, PyAny>, align: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let list = PyList::new(py, buttons.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        Pane::add(slf, "buttons", vec![("buttons", list.into_any()), ("align", align.unwrap_or_else(|| s(py, "right")))])
    }

    #[pyo3(signature = (items, size=96.0, id=None))]
    fn tiles<'py>(slf: &Bound<'py, Self>, items: &Bound<'py, PyAny>, size: f64, id: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let list = PyList::new(py, items.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        Pane::add(slf, "tiles", vec![("items", list.into_any()), ("size", size.into_pyobject(py)?.into_any()), ("id", id.unwrap_or_else(|| s(py, "")))])
    }

    #[pyo3(signature = (items, id=None, thumbs=false))]
    fn rows<'py>(slf: &Bound<'py, Self>, items: &Bound<'py, PyAny>, id: Option<Bound<'py, PyAny>>, thumbs: bool) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let list = PyList::new(py, items.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        Pane::add(slf, "rows", vec![("items", list.into_any()), ("id", id.unwrap_or_else(|| s(py, ""))), ("tone", s(py, if thumbs { "thumbs" } else { "" }))])
    }

    /// A `built_render.chrome_forms.Form`'s fields, drawn as in a form. Field ids
    /// must be unique in the sheet.
    #[pyo3(signature = (form, id=None))]
    fn fields<'py>(slf: &Bound<'py, Self>, form: &Bound<'py, PyAny>, id: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let rows = form.call_method0("wire")?.get_item(4)?;
        let forms = slf.borrow().forms.bind(py).clone();
        forms.call_method1("append", (form,))?;
        let fields = PyList::empty(py);
        for r in rows.try_iter()? {
            fields.append(PyList::new(py, r?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?)?;
        }
        let id = match id.filter(|i| i.is_truthy().unwrap_or(false)) {
            Some(i) => i,
            None => s(py, &format!("f{}", forms.len()?)),
        };
        Pane::add(slf, "fields", vec![("fields", fields.into_any()), ("id", id)])
    }

    fn wire<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("title", &self.title)?;
        d.set_item("sub", &self.sub)?;
        d.set_item("empty", &self.empty)?;
        d.set_item("width", self.width)?;
        d.set_item("blocks", &self.blocks)?;
        Ok(d)
    }
}

/// What one sheet shows, built fresh by its provider.
#[pyclass(module = "built_render.chrome_sheet", subclass, dict)]
pub struct Sheet {
    #[pyo3(get, set)]
    id: Py<PyAny>,
    #[pyo3(get, set)]
    title: Py<PyAny>,
    #[pyo3(get, set)]
    closable: bool,
    #[pyo3(get, set)]
    width: f64,
    #[pyo3(get, set)]
    tabs: Py<PyAny>,
    #[pyo3(get, set)]
    backdrop_icon: Py<PyAny>,
    #[pyo3(get, set)]
    default_verb: Py<PyAny>,
    #[pyo3(get, set)]
    verbs: Py<PyAny>,
    #[pyo3(get, set)]
    menus: Py<PyAny>,
    #[pyo3(get, set)]
    side: Py<PyAny>,
    #[pyo3(get, set)]
    side_width: f64,
    search_spec: Option<Py<PyDict>>,
    content_pane: Py<Pane>,
    detail_pane: Option<Py<Pane>>,
    footer_spec: Option<Py<PyDict>>,
    #[pyo3(get, set)]
    focused: Py<PyAny>,
    #[pyo3(get, set)]
    force_focus: bool,
    #[pyo3(get, set)]
    icons: Py<PyAny>,
}

impl Sheet {
    fn make(py: Python<'_>, sid: &Bound<'_, PyAny>, title: Option<Py<PyAny>>, closable: bool, width: f64) -> PyResult<Self> {
        Ok(Self {
            id: sid.str()?.into_any().unbind(),
            title: title.unwrap_or_else(|| empty_str(py)),
            closable,
            width,
            tabs: PyList::empty(py).into_any().unbind(),
            backdrop_icon: empty_str(py),
            default_verb: empty_str(py),
            verbs: PyList::empty(py).into_any().unbind(),
            menus: PyList::empty(py).into_any().unbind(),
            side: PyList::empty(py).into_any().unbind(),
            side_width: 0.0,
            search_spec: None,
            content_pane: Py::new(py, Pane::make(py, None, None, None, 0.0))?,
            detail_pane: None,
            footer_spec: None,
            focused: py.None(),
            force_focus: false,
            icons: PyDict::new(py).into_any().unbind(),
        })
    }
}

#[pymethods]
impl Sheet {
    #[new]
    #[pyo3(signature = (sid, title=None, *, closable=true, width=0.0))]
    fn new(py: Python<'_>, sid: &Bound<'_, PyAny>, title: Option<Py<PyAny>>, closable: bool, width: f64) -> PyResult<Self> {
        Self::make(py, sid, title, closable, width)
    }

    // header
    #[pyo3(signature = (id, label, count=None, on=false, enabled=true))]
    fn tab(&self, py: Python<'_>, id: &Bound<'_, PyAny>, label: &Bound<'_, PyAny>, count: Option<Bound<'_, PyAny>>, on: bool, enabled: bool) -> PyResult<()> {
        let d = PyDict::new(py);
        d.set_item("id", id.str()?)?;
        d.set_item("label", label)?;
        let count = match count.filter(|c| c.is_truthy().unwrap_or(false)) {
            Some(c) => c.str()?,
            None => PyString::new(py, ""),
        };
        d.set_item("count", count)?;
        d.set_item("on", on)?;
        d.set_item("enabled", enabled)?;
        self.tabs.bind(py).call_method1("append", (d,))?;
        Ok(())
    }

    #[pyo3(signature = (id, label, **kw))]
    fn verb(&self, py: Python<'_>, id: &Bound<'_, PyAny>, label: &Bound<'_, PyAny>, kw: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let v = this(py)?.getattr("verb")?.call((id, label), kw)?;
        self.verbs.bind(py).call_method1("append", (v,))?;
        Ok(())
    }

    #[pyo3(signature = (placeholder, key=None))]
    fn search(&mut self, py: Python<'_>, placeholder: &Bound<'_, PyAny>, key: Option<Bound<'_, PyAny>>) -> PyResult<()> {
        let d = PyDict::new(py);
        d.set_item("placeholder", placeholder)?;
        d.set_item("key", key.unwrap_or_else(|| s(py, "⌘F")))?;
        d.set_item("text", "")?;
        d.set_item("force", false)?;
        self.search_spec = Some(d.unbind());
        Ok(())
    }

    /// A header menu: [(option id, label)], the checked one `on`; `left`:
    /// among the verbs (a way to make something), else beside the search
    /// (a way to look: sort).
    #[pyo3(signature = (id, label, options, on=None, left=false))]
    fn menu(&self, py: Python<'_>, id: &Bound<'_, PyAny>, label: &Bound<'_, PyAny>, options: &Bound<'_, PyAny>, on: Option<Bound<'_, PyAny>>, left: bool) -> PyResult<()> {
        let d = PyDict::new(py);
        d.set_item("id", id.str()?)?;
        d.set_item("label", label)?;
        d.set_item("on", on.map_or_else(|| Ok(PyString::new(py, "")), |o| o.str())?)?;
        d.set_item("left", left)?;
        let opts = PyList::empty(py);
        for o in options.try_iter()? {
            let o = o?;
            opts.append(verb_of(&o.get_item(0)?, &o.get_item(1)?)?)?;
        }
        d.set_item("options", opts)?;
        self.menus.bind(py).call_method1("append", (d,))?;
        Ok(())
    }

    // side
    fn side_caption(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<()> {
        let d = [("id", s(py, "")), ("label", label.clone()), ("kind", s(py, "caption"))].into_py_dict(py)?;
        self.side.bind(py).call_method1("append", (d,))?;
        Ok(())
    }

    #[pyo3(signature = (id, label, **kw))]
    fn side_item(&self, py: Python<'_>, id: &Bound<'_, PyAny>, label: &Bound<'_, PyAny>, kw: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let it = this(py)?.getattr("item")?.call((id, label), kw)?;
        self.side.bind(py).call_method1("append", (it,))?;
        Ok(())
    }

    // panes
    #[pyo3(signature = (title=None, sub=None, empty=None))]
    fn content(&mut self, py: Python<'_>, title: Option<Py<PyAny>>, sub: Option<Py<PyAny>>, empty: Option<Py<PyAny>>) -> PyResult<Py<Pane>> {
        self.content_pane = Py::new(py, Pane::make(py, title, sub, empty, 0.0))?;
        Ok(self.content_pane.clone_ref(py))
    }

    #[pyo3(signature = (width=340.0, title=None, empty=None))]
    fn detail(&mut self, py: Python<'_>, width: f64, title: Option<Py<PyAny>>, empty: Option<Py<PyAny>>) -> PyResult<Py<Pane>> {
        let pane = Py::new(py, Pane::make(py, title, None, empty, width))?;
        self.detail_pane = Some(pane.clone_ref(py));
        Ok(pane)
    }

    #[pyo3(signature = (links=None, hint=None))]
    fn footer(&mut self, py: Python<'_>, links: Option<Bound<'_, PyAny>>, hint: Option<Bound<'_, PyAny>>) -> PyResult<()> {
        let out = PyList::empty(py);
        if let Some(links) = links {
            for l in links.try_iter()? {
                let l = l?;
                out.append(verb_of(&l.get_item(0)?, &l.get_item(1)?)?)?;
            }
        }
        let d = PyDict::new(py);
        d.set_item("links", out)?;
        d.set_item("hint", hint.unwrap_or_else(|| s(py, "")))?;
        self.footer_spec = Some(d.unbind());
        Ok(())
    }

    fn footer_action(&mut self, py: Python<'_>, action: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(f) = &self.footer_spec {
            f.bind(py).set_item("action", action)?;
        }
        Ok(())
    }

    /// The focused item: taken when the sheet opens, or now with `force`.
    #[pyo3(signature = (item_id, force=false))]
    fn focus(&mut self, item_id: Py<PyAny>, force: bool) {
        self.focused = item_id;
        self.force_focus = force;
    }

    /// A picture this sheet needs (sent once per renderer).
    fn icon<'py>(&self, py: Python<'py>, key: Bound<'py, PyAny>, w: i64, h: i64, rgba: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        self.icons.bind(py).set_item(&key, (&key, w, h, rgba))?;
        Ok(key)
    }

    fn forms<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::new(py, self.content_pane.borrow(py).forms.bind(py).try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        if let Some(d) = &self.detail_pane {
            for f in d.borrow(py).forms.bind(py).try_iter()? {
                out.append(f?)?;
            }
        }
        Ok(out)
    }

    #[pyo3(signature = (search_text=None, search_force=false))]
    fn wire<'py>(&self, py: Python<'py>, search_text: Option<Bound<'py, PyAny>>, search_force: bool) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        out.set_item("id", &self.id)?;
        out.set_item("title", &self.title)?;
        out.set_item("closable", self.closable)?;
        out.set_item("width", self.width)?;
        out.set_item("backdrop_icon", &self.backdrop_icon)?;
        out.set_item("default_verb", &self.default_verb)?;
        out.set_item("tabs", &self.tabs)?;
        out.set_item("verbs", &self.verbs)?;
        out.set_item("menus", &self.menus)?;
        out.set_item("side", &self.side)?;
        out.set_item("side_width", self.side_width)?;
        out.set_item("content", self.content_pane.borrow(py).wire(py)?)?;
        out.set_item("detail", match &self.detail_pane {
            Some(d) => d.borrow(py).wire(py)?.into_any(),
            None => none(py),
        })?;
        out.set_item("footer", self.footer_spec.as_ref().map(|f| f.clone_ref(py)))?;
        out.set_item("focus", &self.focused)?;
        out.set_item("force_focus", self.force_focus)?;
        if let Some(search) = &self.search_spec {
            let d = search.bind(py).copy()?;
            if let Some(text) = search_text {
                d.set_item("text", text)?;
                d.set_item("force", search_force)?;
            }
            out.set_item("search", d)?;
        }
        Ok(out)
    }
}

/// A sheet whose spec a Rust view model built: the provider only adds the
/// pictures it names.
#[pyclass(module = "built_render.chrome_sheet", extends = Sheet, dict)]
pub struct Spec {
    #[pyo3(get, set)]
    spec: Py<PyAny>,
}

#[pymethods]
impl Spec {
    #[new]
    fn new(py: Python<'_>, spec: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        fn get<'py>(spec: &Bound<'py, PyAny>, k: &str, d: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
            spec.call_method1("get", (k, d))
        }
        let get = |k: &str, d| get(spec, k, d);
        let mut base = Sheet::make(py, &get("id", s(py, ""))?, Some(get("title", s(py, ""))?.unbind()), get("closable", true.into_pyobject(py)?.to_owned().into_any())?.is_truthy()?, 0.0)?;
        base.focused = get("focus", none(py))?.unbind();
        base.force_focus = get("force_focus", none(py))?.is_truthy()?;
        Ok(PyClassInitializer::from(base).add_subclass(Spec { spec: spec.clone().unbind() }))
    }

    #[pyo3(signature = (search_text=None, search_force=false))]
    fn wire<'py>(slf: PyRef<'py, Self>, search_text: Option<Bound<'py, PyAny>>, search_force: bool) -> PyResult<Bound<'py, PyDict>> {
        let py = slf.py();
        let base = slf.as_super();
        let out = py.import("builtins")?.getattr("dict")?.call1((slf.spec.bind(py),))?.cast_into::<PyDict>()?;
        out.set_item("focus", &base.focused)?;
        out.set_item("force_focus", base.force_focus)?;
        let search = out.get_item("search")?.filter(|s| !s.is_none());
        if let (Some(search), Some(text)) = (search, search_text) {
            let d = search.cast::<PyDict>()?.copy()?;
            d.set_item("text", text)?;
            d.set_item("force", search_force)?;
            out.set_item("search", d)?;
        }
        Ok(out)
    }
}

// ---- providers --------------------------------------------------------------------------

/// One surface on the sheet. Override `build` and `act`.
#[pyclass(module = "built_render.chrome_sheet", subclass, dict)]
pub struct Provider;

#[pymethods]
impl Provider {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Provider
    }

    #[classattr]
    fn id() -> &'static str {
        ""
    }

    fn build(&self, _ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        Err(pyo3::exceptions::PyNotImplementedError::new_err(()))
    }

    fn act(&self, _ctx: &Bound<'_, PyAny>, _verb: &Bound<'_, PyAny>, _args: &Bound<'_, PyAny>) {}

    /// A value that changes when what the sheet shows changes outside its
    /// own actions (the model, a worker's news). Checked each frame.
    fn signature(&self, _ctx: &Bound<'_, PyAny>) -> Option<Py<PyAny>> {
        None
    }

    /// The sheet went: Esc, ×, `close`, or another sheet opened over it
    /// (`ctx.replaced`).
    fn closed(&self, _ctx: &Bound<'_, PyAny>) {}
}

/// What a provider reads: the window and the view in front.
#[pyclass(module = "built_render.chrome_sheet", dict)]
pub struct Ctx {
    #[pyo3(get, set)]
    win: Py<PyAny>,
    #[pyo3(get, set)]
    view: Py<PyAny>,
    #[pyo3(get, set)]
    replaced: bool,
}

#[pymethods]
impl Ctx {
    #[new]
    fn new(win: Py<PyAny>, view: Py<PyAny>) -> Self {
        Self { win, view, replaced: false }
    }

    fn refresh(&self, py: Python<'_>) -> PyResult<()> {
        refresh(self.win.bind(py))
    }
}

/// The sheet on a window (`win._chrome_sheet`).
#[pyclass(module = "built_render.chrome_sheet", dict)]
pub struct SheetState {
    #[pyo3(get, set)]
    provider: Py<PyAny>,
    #[pyo3(get, set)]
    dirty: bool,
    #[pyo3(get, set)]
    sent: Option<String>,
    #[pyo3(get, set)]
    sig: Py<PyAny>,
    #[pyo3(get, set)]
    built: Py<PyAny>,
    #[pyo3(get, set)]
    icons_sent: Py<PyAny>,
    #[pyo3(get, set)]
    renderer: Py<PyAny>,
    #[pyo3(get, set)]
    search: Py<PyAny>,
    #[pyo3(get, set)]
    force_search: bool,
    #[pyo3(get, set)]
    focus: Py<PyAny>,
    #[pyo3(get, set)]
    drawn: bool,
    #[pyo3(get, set)]
    asked: f64,
    #[pyo3(get, set)]
    closing: bool,
}

// ---- on the window ----------------------------------------------------------------------

/// The view in front of the window (the document's stand-in).
#[pyfunction]
#[pyo3(name = "_view")]
fn view_in_front<'py>(win: &Bound<'py, PyAny>) -> Option<Bound<'py, PyAny>> {
    opt(win, "_doc").and_then(|d| opt(&d, "stand_in"))
}

/// Through the module (tests replace `_view`).
fn view_of<'py>(win: &Bound<'py, PyAny>) -> Option<Bound<'py, PyAny>> {
    let py = win.py();
    match this(py).and_then(|m| m.getattr("_view")).and_then(|f| f.call1((win,))) {
        Ok(v) => Some(v).filter(|v| !v.is_none()),
        Err(_) => view_in_front(win),
    }
}

fn state_of<'py>(win: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, SheetState>>> {
    if win.is_none() {
        return Ok(None);
    }
    let Ok(d) = win.getattr("__dict__") else { return Ok(None) };
    let st = d.call_method1("get", ("_chrome_sheet",))?;
    Ok(st.cast_into::<SheetState>().ok())
}

/// Whether this is the `built` binary.
#[pyfunction]
fn in_shell(py: Python<'_>) -> PyResult<bool> {
    py.import("sys")?.getattr("builtin_module_names")?.contains("built_shell")
}

#[pyfunction]
pub fn shown(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(state_of(win)?.is_some_and(|st| !st.borrow().closing))
}

/// The provider on the sheet, or None.
#[pyfunction]
pub fn current<'py>(win: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    Ok(state_of(win)?.filter(|st| !st.borrow().closing).map(|st| st.borrow().provider.bind(win.py()).clone()))
}

fn repaint(win: &Bound<'_, PyAny>) {
    if let Some(view) = view_of(win) {
        // A view already gone (RuntimeError) has nothing to repaint.
        let _ = view.call_method0("update");
    }
}

/// Tell the provider its sheet went; its tidy-up never keeps the sheet up.
fn closed_now(win: &Bound<'_, PyAny>, st: &Bound<'_, SheetState>, replaced: bool) -> PyResult<()> {
    let py = win.py();
    let view = view_of(win).map(Bound::unbind).unwrap_or_else(|| py.None());
    let ctx = Py::new(py, Ctx { win: win.clone().unbind(), view, replaced })?;
    let provider = st.borrow().provider.clone_ref(py);
    if let Some(closed) = provider.bind(py).getattr_opt("closed")? {
        if let Err(e) = closed.call1((ctx,)) {
            e.print(py);
        }
    }
    Ok(())
}

/// Install the sheet model for the next frame, including offscreen hosts.
#[pyfunction]
#[pyo3(signature = (win, provider, *, search=None, focus=None))]
pub fn open<'py>(win: &Bound<'py, PyAny>, provider: &Bound<'py, PyAny>, search: Option<Bound<'py, PyAny>>, focus: Option<Bound<'py, PyAny>>) -> PyResult<bool> {
    let py = win.py();
    let old = state_of(win)?;
    if let Some(old) = &old {
        if old.borrow().provider.bind(py).is(provider) && !old.borrow().closing {
            let mut o = old.borrow_mut();
            if let Some(search) = &search {
                o.search = search.clone().unbind();
                o.force_search = true;
            }
            if let Some(focus) = &focus {
                o.focus = focus.clone().unbind();
            }
            o.dirty = true;
            drop(o);
            repaint(win);
            return Ok(true);
        }
        if !old.borrow().closing {
            closed_now(win, old, true)?;
        }
    }
    let st = Bound::new(py, SheetState {
        provider: provider.clone().unbind(),
        dirty: true,
        sent: None,
        sig: py.None(),
        built: py.None(),
        icons_sent: PySet::empty(py)?.into_any().unbind(),
        renderer: py.None(),
        force_search: search.is_some(),
        search: search.map_or_else(|| py.None(), Bound::unbind),
        focus: focus.map_or_else(|| py.None(), Bound::unbind),
        drawn: false,
        asked: py.import("time")?.call_method0("monotonic")?.extract()?,
        closing: false,
    })?;
    win.setattr("_chrome_sheet", &st)?;
    repaint(win);
    Ok(true)
}

/// Take the sheet down (animated); the model's view gets the keys back.
#[pyfunction]
pub fn close(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let Some(st) = state_of(win)? else { return Ok(()) };
    if st.borrow().closing {
        return Ok(());
    }
    closed_now(win, &st, false)?;
    {
        let mut s = st.borrow_mut();
        s.closing = true;
        s.dirty = true;
    }
    repaint(win);
    Ok(())
}

/// Build the sheet again before the next frame.
#[pyfunction]
pub fn refresh(win: &Bound<'_, PyAny>) -> PyResult<()> {
    if let Some(st) = state_of(win)? {
        st.borrow_mut().dirty = true;
        repaint(win);
    }
    Ok(())
}

// ---- frames -----------------------------------------------------------------------------

/// Send the sheet when it changed (cheap when it did not).
#[pyfunction]
pub fn before_frame(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let Some(st) = state_of(win)? else { return Ok(()) };
    let Some(renderer) = opt(view, "renderer").filter(|r| r.hasattr("ui_set_large_sheet").unwrap_or(false)) else { return Ok(()) };
    if !st.borrow().renderer.bind(py).is(&renderer) {
        let mut s = st.borrow_mut();
        s.renderer = renderer.clone().unbind();
        s.icons_sent = PySet::empty(py)?.into_any().unbind();
        s.sent = None;
    }
    if st.borrow().closing {
        renderer.call_method1("ui_set_large_sheet", ("",))?;
        if state_of(win)?.is_some_and(|s| s.is(&st)) {
            win.delattr("_chrome_sheet")?;
        }
        return Ok(());
    }
    let ctx = Py::new(py, Ctx { win: win.clone().unbind(), view: view.clone().unbind(), replaced: false })?;
    let provider = st.borrow().provider.clone_ref(py);
    let provider = provider.bind(py);
    let sig = provider.call_method1("signature", (&ctx,)).unwrap_or_else(|_| none(py));
    if sig.ne(st.borrow().sig.bind(py))? {
        let mut s = st.borrow_mut();
        s.sig = sig.unbind();
        s.dirty = true;
    }
    st.borrow_mut().drawn = true;
    if !st.borrow().dirty && st.borrow().sent.is_some() {
        return Ok(());
    }
    st.borrow_mut().dirty = false;
    let sheet = match provider.call_method1("build", (&ctx,)) {
        Ok(s) => s,
        Err(e) => {
            // A broken provider closes its sheet, not the window.
            e.print(py);
            return this(py)?.call_method1("close", (win,)).map(|_| ());
        }
    };
    let focus = st.borrow().focus.clone_ref(py);
    if !focus.is_none(py) && sheet.getattr("focused")?.is_none() {
        sheet.call_method("focus", (focus,), Some(&[("force", true)].into_py_dict(py)?))?;
    }
    {
        let mut s = st.borrow_mut();
        s.focus = py.None();
        s.built = sheet.clone().unbind();
    }
    let sent = st.borrow().icons_sent.clone_ref(py);
    let sent = sent.bind(py);
    let fresh = PyList::empty(py);
    for (k, i) in sheet.getattr("icons")?.cast_into::<PyDict>()?.iter() {
        if !sent.contains(&k)? {
            fresh.append(i)?;
        }
    }
    if !fresh.is_empty() {
        renderer.call_method1("ui_set_icons", (&fresh,))?;
        for i in fresh.iter() {
            sent.call_method1("add", (i.get_item(0)?,))?;
        }
    }
    let (search, force) = {
        let s = st.borrow();
        (if s.force_search { s.search.clone_ref(py).into_bound(py) } else { none(py) }, s.force_search)
    };
    let wire = sheet.call_method1("wire", (search, force))?;
    st.borrow_mut().force_search = false;
    let kwargs = PyDict::new(py);
    kwargs.set_item("ensure_ascii", false)?;
    kwargs.set_item("separators", (",", ":"))?;
    let text: String = py.import("json")?.getattr("dumps")?.call((wire,), Some(&kwargs))?.extract()?;
    if let Some(rows) = sheet.getattr_opt("_library_rows")? {
        let rows = rows.extract::<PyRef<crate::library_host::LibraryRows>>()?;
        let mut spec: crate::ui::large_sheet::Spec = serde_json::from_str(&text)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let checked = sheet.getattr_opt("_checked")?;
        for block in &mut spec.content.blocks {
            if let Some(index) = block.id.strip_prefix("library-group:").and_then(|s| s.parse::<usize>().ok()) {
                block.items = rows.groups[index].clone();
                if checked.is_some() { block.tone = "components".into(); }
            }
        }
        if let Some(ui) = &mut renderer.cast::<crate::Renderer>()?.borrow_mut().ui {
            ui.set_component_checks(checked.map(|v| v.extract()).transpose()?.unwrap_or_default());
            ui.set_sheet_spec(spec);
        }
        st.borrow_mut().sent = Some(text);
        return Ok(());
    }
    if st.borrow().sent.as_deref() != Some(text.as_str()) {
        if let Err(e) = renderer.call_method1("ui_set_large_sheet", (&text,)) {
            if e.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                // The chrome could not read it: say why, and try again next frame.
                eprintln!("[chrome_sheet] {}: {}", sheet.getattr("id")?.str()?, e.value(py).str()?);
                st.borrow_mut().dirty = true;
                return Ok(());
            }
            return Err(e);
        }
        st.borrow_mut().sent = Some(text);
    }
    Ok(())
}

/// Raise ValueError when the chrome would refuse `text` (tests; the schema
/// is `ui/large_sheet.rs` `Spec`).
#[pyfunction]
fn check_wire(text: &str) -> PyResult<()> {
    if text.is_empty() {
        return Ok(());
    }
    if let Err(err) = serde_json::from_str::<crate::ui::large_sheet::Spec>(text) {
        return Err(pyo3::exceptions::PyValueError::new_err(format!("the chrome cannot read this sheet: {err}")));
    }
    Ok(())
}

fn form_of<'py>(st: &Bound<'py, SheetState>, fid: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = st.py();
    let sheet = st.borrow().built.clone_ref(py).into_bound(py);
    if sheet.is_none() {
        return Ok(None);
    }
    for f in sheet.call_method0("forms")?.try_iter()? {
        let f = f?;
        if !f.call_method1("field", (fid,))?.is_none() {
            return Ok(Some(f));
        }
    }
    Ok(None)
}

/// `lsheet:<sheet id>\x1f<verb>[\x1f<arg>…]` from the chrome.
#[pyfunction]
pub fn action(win: &Bound<'_, PyAny>, arg: &str) -> PyResult<()> {
    let py = win.py();
    let (sid, rest) = arg.split_once(SEP).unwrap_or((arg, ""));
    let Some(st) = state_of(win)? else { return Ok(()) };
    let built = st.borrow().built.clone_ref(py).into_bound(py);
    if st.borrow().closing || built.is_none() || built.getattr("id")?.str()?.to_str()? != sid {
        return Ok(()); // a click for a sheet already gone
    }
    let parts: Vec<&str> = rest.split(SEP).collect();
    let (verb, args) = (parts[0], parts[1..].iter().map(|a| a.to_string()).collect::<Vec<_>>());
    if verb == "close" {
        return this(py)?.call_method1("close", (win,)).map(|_| ());
    }
    if (verb == "v" || verb == "b") && !args.is_empty() {
        if let Some(form) = form_of(&st, &args[0])? {
            if verb == "v" {
                form.call_method1("receive", (&args[0], args.get(1).map(String::as_str).unwrap_or("")))?;
            } else {
                form.call_method1("press", (&args[0],))?;
            }
            st.borrow_mut().dirty = true;
            repaint(win);
            return Ok(());
        }
        // A raw Spec can own fields directly, without a registered form.
    }
    if verb == "search" {
        st.borrow_mut().search = args.first().map(String::as_str).unwrap_or("").into_pyobject(py)?.into_any().unbind();
    }
    let view = view_of(win).map(Bound::unbind).unwrap_or_else(|| py.None());
    let ctx = Py::new(py, Ctx { win: win.clone().unbind(), view, replaced: false })?;
    let provider = st.borrow().provider.clone_ref(py);
    if verb == "focus" && built.hasattr("_library_rows")? && args.first().is_some_and(|k| !k.starts_with("online:")) {
        provider.bind(py).call_method1("focus_component", (&ctx, &built, &args[0]))?;
        repaint(win);
        return Ok(());
    }
    if let Err(e) = provider.bind(py).call_method1("act", (ctx, verb, args)) {
        if !(e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)) {
            return Err(e);
        }
        if let Some(vp) = opt(win, "viewport").filter(|v| v.hasattr("flash_status").unwrap_or(false)) {
            let text = e.value(py).str()?.to_string();
            vp.call_method1("flash_status", (text.trim_matches(|c| c == '\'' || c == '"'),))?;
        }
    }
    if !matches!((sid,verb), ("library" | "tree-material", "component-previews" | "family-previews")) {
        st.borrow_mut().dirty = true;
    }
    repaint(win);
    Ok(())
}

#[pyfunction]
fn search_text(win: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = win.py();
    Ok(match state_of(win)? {
        Some(st) => {
            let s = st.borrow().search.clone_ref(py).into_bound(py);
            if s.is_truthy()? { s.str()?.to_string() } else { String::new() }
        }
        None => String::new(),
    })
}

/// ⌘/⌃ combinations stay the window's (⌘K, ⌘O, ⌘W, ⌘Q…), except the ones
/// a text field uses and ⌘F (the sheet's search).
fn window_key(event: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = event.py();
    let qt = py.import("built_core")?.getattr("Input")?;
    let mask = qt.getattr("ControlModifier")?.bitor(qt.getattr("MetaModifier")?)?;
    if !event.call_method0("modifiers")?.bitand(mask)?.is_truthy()? {
        return Ok(false);
    }
    let key = event.call_method0("key")?;
    for name in ["Key_F", "Key_A", "Key_C", "Key_V", "Key_X", "Key_Z"] {
        if key.eq(qt.getattr(name)?)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// True: the sheet's key; False: the window's; None: no sheet up.
#[pyfunction]
fn claims_key(view: &Bound<'_, PyAny>, event: &Bound<'_, PyAny>) -> PyResult<Option<bool>> {
    let py = view.py();
    let win = py.import("views.wgpu_ui")?.call_method1("_window", (view,))?;
    if !shown(&win)? {
        return Ok(None);
    }
    Ok(Some(!window_key(event)?))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "chrome_sheet")?;
    sub.add("SEP", SEP)?;
    sub.add_class::<Pane>()?;
    sub.add_class::<Sheet>()?;
    sub.add_class::<Spec>()?;
    sub.add_class::<Provider>()?;
    sub.add_class::<Ctx>()?;
    sub.add_class::<SheetState>()?;
    for f in [
        wrap_pyfunction!(item, &sub)?,
        wrap_pyfunction!(verb, &sub)?,
        wrap_pyfunction!(in_shell, &sub)?,
        wrap_pyfunction!(shown, &sub)?,
        wrap_pyfunction!(current, &sub)?,
        wrap_pyfunction!(open, &sub)?,
        wrap_pyfunction!(close, &sub)?,
        wrap_pyfunction!(refresh, &sub)?,
        wrap_pyfunction!(before_frame, &sub)?,
        wrap_pyfunction!(check_wire, &sub)?,
        wrap_pyfunction!(action, &sub)?,
        wrap_pyfunction!(search_text, &sub)?,
        wrap_pyfunction!(claims_key, &sub)?,
        wrap_pyfunction!(view_in_front, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.add_submodule(&sub)?;
    // `import built_render.chrome_sheet` finds it too.
    py.import("sys")?.getattr("modules")?.set_item("built_render.chrome_sheet", &sub)?;
    Ok(())
}
