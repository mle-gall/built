//! Status messages shown by the chrome and published by extensions.
use super::*;

#[pyclass(module = "built_render", dict)]
pub struct ShellStatusBar;
#[pymethods]
#[allow(non_snake_case)]
impl ShellStatusBar {
    #[new]
    fn new(py: Python<'_>, host: &Obj<'_>) -> PyResult<Py<Self>> {
        let status = Bound::new(py, Self)?;
        status.setattr("_base", "")?;
        status.setattr("flash", "")?;
        status.setattr("_host", host)?;
        status.setattr(
            "_timer",
            host.getattr("timers")?
                .call_method1("timer", (host.getattr("request_frame")?,))?,
        )?;
        Ok(status.unbind())
    }
    #[pyo3(signature=(text,msec=0,tone="info"))]
    fn showMessage(slf: &Bound<'_, Self>, text: &str, msec: i64, tone: &str) -> PyResult<()> {
        if !text.is_empty() {
            let win = slf.getattr("_window")?.call0()?;
            if !win.is_none() {
                win.call_method1("notify", (tone, text))?;
            }
        }
        if msec != 0 {
            slf.setattr("flash", text)?;
            slf.getattr("_timer")?.call_method1("start", (msec,))?;
        } else {
            slf.setattr("_base", text)?;
        }
        slf.getattr("_host")?.call_method0("request_frame")?;
        Ok(())
    }
    fn currentMessage<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let left: i64 = slf
            .getattr("_timer")?
            .call_method0("remainingTime")?
            .extract()?;
        slf.getattr(if left > 0 { "flash" } else { "_base" })
    }
    fn clearMessage(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("_timer")?.call_method0("stop")?;
        Ok(())
    }
}
