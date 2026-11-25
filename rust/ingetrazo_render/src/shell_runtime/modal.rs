//! Synchronous questions pump only the active window's chrome and timers.
use super::*;
use pyo3::exceptions::PyOSError;
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[pyfunction]
fn host(py: Python<'_>) -> PyResult<Obj<'_>> {
    super::windows::front(py)
}
pub(crate) fn live<'p>(py: Python<'p>, parent: &Obj<'p>) -> PyResult<Obj<'p>> {
    let h = host(py)?;
    if h.is_none() {
        return Ok(none(py));
    }
    let win = module(py, "chrome_dialogs")?.call_method1("window_of", (parent,))?;
    if !win.is(&h.getattr("window")?) || flag(&win, "_closed")? || !flag(&h, "ns_view")? {
        return Ok(none(py));
    }
    let v = h.getattr("view")?;
    if flag(&v, "_ui_attached")? && !attr(&v, "renderer")?.is_none() && !flag(&v, "_drawing_frame")?
    {
        Ok(v)
    } else {
        Ok(none(py))
    }
}
fn turn(h: &Obj<'_>, ms: f64) -> PyResult<()> {
    for event in module(h.py(), "shell_services")?
        .call_method1("pump", (h.getattr("ns_view")?, ms))?
        .try_iter()?
    {
        dispatch(h, &event?)?;
    }
    h.call_method0("modal_frame")?;
    Ok(())
}
fn until(
    h: &Obj<'_>,
    mut done: impl FnMut() -> PyResult<bool>,
    timeout: Option<f64>,
) -> PyResult<bool> {
    if h.is_none() || !flag(h, "ns_view")? {
        return Ok(false);
    }
    let start = Instant::now();
    while !done()? {
        if timeout.is_some_and(|s| start.elapsed().as_secs_f64() > s) {
            return Ok(false);
        }
        turn(h, 16.)?;
        if flag(h, "quit_now")? {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction(signature=(done,timeout_s=None))]
fn run_until(done: &Obj<'_>, timeout_s: Option<f64>) -> PyResult<bool> {
    until(&host(done.py())?, || done.call0()?.is_truthy(), timeout_s)
}
thread_local! {static LAST:Cell<Option<Instant>>=const{Cell::new(None)};}
#[pyfunction(signature=(min_interval=1./30.))]
fn tick(py: Python<'_>, min_interval: f64) -> PyResult<()> {
    let h = host(py)?;
    if h.is_none() || !flag(&h, "ns_view")? {
        return Ok(());
    }
    let now = Instant::now();
    if LAST.with(|c| {
        if c.get()
            .is_some_and(|last| now.duration_since(last).as_secs_f64() < min_interval)
        {
            false
        } else {
            c.set(Some(now));
            true
        }
    }) {
        turn(&h, 0.)?;
    }
    Ok(())
}
#[pyfunction]
fn dispatch(h: &Obj<'_>, ev: &Obj<'_>) -> PyResult<()> {
    let py = h.py();
    let (kind, x, y, button, mods, key, text): (String, f64, f64, i32, u32, i32, String) =
        ev.extract()?;
    let win = h.getattr("window")?;
    let input = h.getattr("input_state")?;
    match kind.as_str() {
        "press" | "release" | "move" => {
            let view = h.getattr("view")?;
            if !flag(&view, "_ui_attached")? {
                return Ok(());
            }
            let held = input.call_method1("modal_pointer", (&kind, x, y, button, mods))?;
            let event = core(py)?.call_method1("MouseEvent", (&kind, x, y, button, held, mods))?;
            h.call_method1(
                "_guard",
                (
                    "modal pointer",
                    py.import("views.wgpu_ui")?.getattr("pointer")?,
                    &view,
                    kind,
                    event,
                ),
            )?;
        }
        "key" => {
            input.setattr("modifiers", mods)?;
            let cd = module(py, "chrome_dialogs")?;
            let event = core(py)?.call_method1("KeyEvent", (true, key, mods, text))?;
            if !cd.call_method1("current_sheet", (&win,))?.is_none() {
                h.call_method1(
                    "_guard",
                    ("sheet key", cd.getattr("sheet_key")?, win, event),
                )?;
            } else if key == 0x01000000 {
                h.call_method1("_guard", ("job key", cd.getattr("job_key")?, win, event))?;
            }
        }
        _ => return Ok(()),
    }
    h.call_method0("request_frame")?;
    Ok(())
}
#[pyclass(module = "ingetrazo_render.shell_modal")]
pub(crate) struct ModalWait {
    done: bool,
}
#[pymethods]
impl ModalWait {
    #[new]
    pub(crate) fn new() -> Self {
        Self { done: false }
    }
    fn quit(&mut self) {
        self.done = true;
    }
    fn completed(&self) -> bool {
        self.done
    }
    fn exec(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let env = py.import("os")?.getattr("environ")?;
        if let Some(queue) = env
            .call_method1("get", ("BUILT_ANSWERS",))?
            .extract::<Option<String>>()?
        {
            let mut answers = queue.split(',').map(str::trim).filter(|s| !s.is_empty());
            if let Some(answer) = answers.next() {
                env.set_item("BUILT_ANSWERS", answers.collect::<Vec<_>>().join(","))?;
                answer_when_drawn(slf, answer, &env)?;
            }
        }
        module(py, "shell_modal")?.call_method1("run_until", (slf.getattr("completed")?,))?;
        Ok(())
    }
}
static ASKED: AtomicU64 = AtomicU64::new(0);
fn answer_when_drawn(wait: &Bound<'_, ModalWait>, answer: &str, env: &Obj<'_>) -> PyResult<()> {
    let py = wait.py();
    let h = host(py)?;
    let win = h.getattr("window")?;
    let cd = module(py, "chrome_dialogs")?;
    let sheet = cd.call_method1("current_sheet", (&win,))?;
    let n = ASKED.fetch_add(1, Ordering::Relaxed) + 1;
    let mut frames = 0;
    until(
        &h,
        || {
            if wait.borrow().done {
                return Ok(true);
            }
            if !sheet.is_none() && flag(&sheet, "shown")? {
                frames += 1;
                h.call_method0("request_frame")?;
            }
            Ok(frames >= 3)
        },
        Some(5.),
    )?;
    if wait.borrow().done {
        return Ok(());
    }
    if let Some(dir) = env
        .call_method1("get", ("BUILT_SHOTS",))?
        .extract::<Option<String>>()?
        .filter(|p| !p.is_empty())
    {
        let title = if sheet.is_none() {
            "sheet".into()
        } else {
            sheet.getattr("title")?.extract::<String>()?
        };
        let slug: String = title
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        capture(
            &h,
            std::path::Path::new(&dir).join(format!(
                "sheet-{n}-{}.png",
                slug.trim_matches('-').chars().take(40).collect::<String>()
            )),
        )?;
    }
    if let Some(button) = answer.strip_prefix("id:") {
        cd.call_method1("answer", (win, button))?;
    } else {
        let (code, chars, cmd) = match answer {
            "return" => (36, "\r", false),
            "cmd-d" => (2, "d", true),
            _ => (53, "\x1b", false),
        };
        if !module(py, "shell_services")?
            .call_method1("post_key", (h.getattr("ns_view")?, code, chars, cmd))?
            .is_truthy()?
            && !sheet.is_none()
        {
            cd.call_method1(
                "answer",
                (
                    win,
                    sheet.getattr(if answer == "return" {
                        "default"
                    } else {
                        "cancel"
                    })?,
                ),
            )?;
        }
    }
    Ok(())
}
#[pyfunction]
fn capture(h: &Obj<'_>, path: std::path::PathBuf) -> PyResult<bool> {
    let r = h.getattr("view")?.getattr("renderer")?;
    if r.is_none() {
        return Ok(false);
    }
    r.call_method0("request_capture")?;
    h.call_method0("frame")?;
    match r.call_method1("save_capture", (path,)) {
        Ok(v) => v.is_truthy(),
        Err(e) if e.is_instance_of::<PyOSError>(h.py()) => Ok(false),
        Err(e) => Err(e),
    }
}
pub(super) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = &create(parent, "shell_modal")?;

    m.add_class::<ModalWait>()?;
    for f in [
        wrap_pyfunction!(host, m)?,
        wrap_pyfunction!(super::windows::activate, m)?,
        wrap_pyfunction!(run_until, m)?,
        wrap_pyfunction!(tick, m)?,
        wrap_pyfunction!(dispatch, m)?,
        wrap_pyfunction!(capture, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
