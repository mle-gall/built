//! One clipboard for the process, shared across windows and project tabs.
//! The existing document snapshot codec supplies the inter-process payload.
use pyo3::{prelude::*, types::PyDict, PyTraverseError, PyVisit};
use std::cell::RefCell;

const MIME: &str = "com.trolltech.anymime.application--x-ingetrazo-clipboard";

#[pyclass(unsendable)]
#[derive(Default)]
pub struct ModelClipboard {
    source: RefCell<Option<(i64, Py<PyAny>)>>,
}
fn install(
    vp: &Bound<'_, PyAny>,
    source: &Bound<'_, PyAny>,
    clip: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let old = vp.getattr("clipboard")?;
    if old.is_truthy()? {
        if let Some(drop) = vp
            .getattr("_drop_clip_protos")
            .ok()
            .filter(|v| v.is_callable())
        {
            drop.call1((old,))?;
        }
    }
    vp.setattr("clipboard", clip)?;
    vp.setattr("_clip_adopted_from", source)?;
    Ok(())
}

// The widget host uses the platform offer supplied by its Qt application. Reuse
// its identity for repeated pastes, but decode new offers into the receiving
// document's own groups, materials and meshes.
fn adopt_platform_offer(vp: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = vp.py();
    let adapter = py.import("formats.clip")?;
    let offer = adapter.call_method0("current_offer")?;
    let mime = adapter.getattr("MIME")?;
    if offer.is_none() || !offer.call_method1("hasFormat", (&mime,))?.is_truthy()? {
        return Ok(());
    }
    let source = py
        .import("builtins")?
        .call_method1("getattr", (&offer, "clip", &offer))?;
    if vp.getattr("clipboard")?.is(&source)
        || vp
            .getattr("_clip_adopted_from")
            .is_ok_and(|v| v.is(&source))
    {
        return Ok(());
    }
    let data = offer.call_method1("data", (mime,))?.call_method0("data")?;
    let other = adapter.call_method1("decode", (data,))?;
    if !other.is_none() {
        install(vp, &source, &other)?;
    }
    Ok(())
}

#[pymethods]
impl ModelClipboard {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    #[staticmethod]
    fn available(py: Python<'_>) -> PyResult<bool> {
        let native = py.import("app_shell.native")?;
        if native.call_method0("available")?.is_truthy()? {
            let (types, _): (Vec<String>, i64) =
                native.call_method0("pasteboard_types")?.extract()?;
            return Ok(types.iter().any(|t| t == MIME));
        }
        py.import("formats.clip")?
            .call_method0("available")?
            .is_truthy()
    }

    /// Copy once, even when the other tab's adopted payload was decoded into
    /// distinct objects. Replacing it also releases any cached preview mesh.
    fn adopt(&self, win: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = win.py();
        let native = py.import("app_shell.native")?;
        let vp = win.getattr("viewport")?;
        if !native.call_method0("available")?.is_truthy()? {
            return adopt_platform_offer(&vp);
        }
        let (types, count): (Vec<String>, i64) =
            native.call_method0("pasteboard_types")?.extract()?;
        if !types.iter().any(|t| t == MIME) {
            return Ok(());
        }
        let old = vp.getattr("clipboard")?;
        let source = self
            .source
            .borrow()
            .as_ref()
            .and_then(|(n, s)| (*n == count).then(|| s.clone_ref(py)));
        if source.as_ref().is_some_and(|s| {
            old.is(s.bind(py))
                || vp
                    .getattr("_clip_adopted_from")
                    .is_ok_and(|v| v.is(s.bind(py)))
        }) {
            return Ok(());
        }
        let data = native.call_method1("pasteboard_read", (MIME,))?;
        if !data.is_truthy()? {
            return Ok(());
        }
        let other = py
            .import("formats.clip_payload")?
            .call_method1("decode", (data,))?;
        if other.is_none() {
            return Ok(());
        }
        if let Some(source) = source {
            install(&vp, source.bind(py), &other)?;
        } else {
            install(&vp, py.None().bind(py), &other)?;
            self.source.replace(Some((count, other.unbind())));
        }
        Ok(())
    }
    /// Publish an existing document snapshot from either a native command or a script.
    fn publish(&self, win: &Bound<'_, PyAny>, clip: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = win.py();
        let native = py.import("app_shell.native")?;
        let encode = py.import("formats.clip_payload")?.getattr("encode")?;
        let data = win
            .getattr("_host")?
            .call_method1("_guard", ("clipboard encode", encode, clip))?;
        if data.is_none() {
            return Ok(());
        }
        if native
            .call_method1("pasteboard_write", (vec![(MIME, data)],))?
            .is_truthy()?
        {
            let count = native
                .call_method0("pasteboard_types")?
                .get_item(1)?
                .extract()?;
            self.source.replace(Some((count, clip.clone().unbind())));
        }
        Ok(())
    }
    fn run(&self, win: &Bound<'_, PyAny>, verb: &str) -> PyResult<()> {
        let py = win.py();
        let doc = win.getattr("_doc")?;
        if doc.is_none() {
            return Ok(());
        }
        let view = doc.getattr("stand_in")?;
        let renderer = view.getattr("renderer").ok().filter(|r| !r.is_none());
        let field = renderer.as_ref().is_some_and(|r| {
            r.call_method1("ui_claims_key", ("",))
                .is_ok_and(|v| v.is_truthy().unwrap_or(false))
        });
        let native = py.import("app_shell.native")?;
        if field {
            if matches!(verb, "paste" | "paste_in_place") {
                let data = native.call_method1("pasteboard_read", ("public.utf8-plain-text",))?;
                if data.is_truthy()? {
                    let text = data.call_method1("decode", ("utf-8", "replace"))?;
                    if let Some(r) = renderer {
                        r.call_method1("ui_text", (text,))?;
                    }
                    view.call_method0("update")?;
                }
            }
            return Ok(());
        }
        match verb {
            "copy" | "cut" => {
                let vp = win.getattr("viewport")?;
                let method = if verb == "copy" {
                    "copy_selection"
                } else {
                    "cut_selection"
                };
                let native_board =
                    native.call_method0("available")?.is_truthy()?;
                let copied = if native_board {
                    let options = PyDict::new(py);
                    options.set_item("publish", false)?;
                    vp.call_method(method, (), Some(&options))?
                } else {
                    vp.call_method0(method)?
                };
                if !copied.is_truthy()? || !native_board {
                    return Ok(());
                }
                let clip = vp.getattr("clipboard")?;
                if !clip.is_truthy()? {
                    return Ok(());
                }
                self.publish(win, &clip)?;
            }
            "paste" | "paste_in_place" => {
                super::model_edit::paste_model(win, verb == "paste_in_place")?;
                view.getattr("tools")?.call_method0("sync")?;
                view.call_method0("update")?;
            }
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "unknown clipboard command",
                ))
            }
        }
        Ok(())
    }
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Ok(source) = self.source.try_borrow() {
            if let Some((_, clip)) = &*source {
                visit.call(clip)?;
            }
        }
        Ok(())
    }
    fn __clear__(&mut self) {
        self.source.get_mut().take();
    }
}
