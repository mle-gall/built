//! The model clipboard: one for the process, shared across windows and
//! project tabs, and the only reader and writer of the platform pasteboard's
//! model type. The document snapshot codec (`formats.clip_payload`) supplies
//! the bytes. Without a shell (a headless host) the board is this object's
//! own, so a copy still reaches the other documents of the process.
use pyo3::{prelude::*, types::{PyBytes, PyDict}, PyTraverseError, PyVisit};
use std::cell::RefCell;

/// The type a copy is written as.
const MIME: &str = "application/x-ingetrazo-clipboard";
/// What builds with the Qt window wrote on macOS (Qt's name for a custom
/// MIME type): still read, so a copy made in an older build pastes here.
const QT_ERA_MIME: &str = "com.trolltech.anymime.application--x-ingetrazo-clipboard";

#[pyclass(unsendable)]
#[derive(Default)]
pub struct ModelClipboard {
    /// The pasteboard change a copy of this process answers, and that copy.
    source: RefCell<Option<(i64, Py<PyAny>)>>,
    /// The board when no shell is attached: its bytes and change count.
    local: RefCell<(Option<Py<PyBytes>>, i64)>,
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

/// The shell's pasteboard, when a shell is attached.
fn shell(py: Python<'_>) -> PyResult<Option<Bound<'_, PyModule>>> {
    let native = py.import("ingetrazo_render.shell_services")?;
    Ok(native.call_method0("available")?.is_truthy()?.then_some(native))
}

impl ModelClipboard {
    /// The model type on the board (ours first, then the Qt-era one) and
    /// the board's change count.
    fn offered(&self, py: Python<'_>) -> PyResult<(Option<&'static str>, i64)> {
        let Some(native) = shell(py)? else {
            let local = self.local.borrow();
            return Ok((local.0.is_some().then_some(MIME), local.1));
        };
        let (types, count): (Vec<String>, i64) = native.call_method0("pasteboard_types")?.extract()?;
        Ok(([MIME, QT_ERA_MIME].into_iter().find(|m| types.iter().any(|t| t == m)), count))
    }
    fn read<'py>(&self, py: Python<'py>, mime: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
        let data = match shell(py)? {
            Some(native) => native.call_method1("pasteboard_read", (mime,))?,
            None => match &self.local.borrow().0 {
                Some(bytes) => bytes.bind(py).clone().into_any(),
                None => return Ok(None),
            },
        };
        Ok(data.is_truthy()?.then_some(data))
    }
    /// Replace the board's contents; the new change count when it took them.
    fn write(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Option<i64>> {
        let Some(native) = shell(py)? else {
            let mut local = self.local.borrow_mut();
            local.0 = Some(data.clone().cast_into::<PyBytes>()?.unbind());
            local.1 += 1;
            return Ok(Some(local.1));
        };
        if !native.call_method1("pasteboard_write", (vec![(MIME, data)],))?.is_truthy()? {
            return Ok(None);
        }
        Ok(Some(native.call_method0("pasteboard_types")?.get_item(1)?.extract()?))
    }
}

#[pymethods]
impl ModelClipboard {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    /// Whether the board holds a model copy (of this or another process).
    fn available(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(self.offered(py)?.0.is_some())
    }

    /// Copy once, even when the other tab's adopted payload was decoded into
    /// distinct objects. Replacing it also releases any cached preview mesh.
    fn adopt(&self, win: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = win.py();
        let vp = win.getattr("viewport")?;
        let (Some(mime), count) = self.offered(py)? else {
            return Ok(());
        };
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
        let Some(data) = self.read(py, mime)? else {
            return Ok(());
        };
        let other = py
            .import("ingetrazo_core.formats.clip_payload")?
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
        let encode = py.import("ingetrazo_core.formats.clip_payload")?.getattr("encode")?;
        // A window's host reports a failed encode; a bare document has none.
        let data = match win.getattr_opt("_host")?.filter(|h| !h.is_none()) {
            Some(host) => host.call_method1("_guard", ("clipboard encode", encode, clip))?,
            None => encode.call1((clip,))?,
        };
        if data.is_none() {
            return Ok(());
        }
        if let Some(count) = self.write(py, &data)? {
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
        let native = py.import("ingetrazo_render.shell_services")?;
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
                let options = PyDict::new(py);
                options.set_item("publish", false)?;
                if !vp.call_method(method, (), Some(&options))?.is_truthy()? {
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
        self.local.get_mut().0.take();
    }
}
