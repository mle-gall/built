// SPDX-License-Identifier: GPL-3.0-or-later
//! Respect a Windows user's existing GPU choice, and otherwise ask for the discrete GPU.
use pyo3::{exceptions::PyFileNotFoundError, prelude::*};
use std::path::{Path, PathBuf};

const PREFS: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";
const HIGH_PERFORMANCE: &str = "GpuPreference=2;";

fn set_preference(reg: &Bound<PyAny>, exe: &Path) -> PyResult<&'static str> {
    let py = reg.py();
    let rights =
        reg.getattr("KEY_READ")?.extract::<u32>()? | reg.getattr("KEY_WRITE")?.extract::<u32>()?;
    let key = reg.call_method1(
        "CreateKeyEx",
        (reg.getattr("HKEY_CURRENT_USER")?, PREFS, 0, rights),
    )?;
    let name = exe.to_string_lossy();
    let result = match reg.call_method1("QueryValueEx", (&key, name.as_ref())) {
        Ok(_) => Ok("kept"),
        Err(e) if e.is_instance_of::<PyFileNotFoundError>(py) => (|| {
            reg.call_method1(
                "SetValueEx",
                (
                    &key,
                    name.as_ref(),
                    0,
                    reg.getattr("REG_SZ")?,
                    HIGH_PERFORMANCE,
                ),
            )?;
            Ok("set")
        })(),
        Err(e) => Err(e),
    };
    let _ = reg.call_method1("CloseKey", (key,));
    result
}

#[pyfunction]
#[pyo3(signature=(exe=None, *, winreg=None, platform=None))]
pub(crate) fn ensure_high_performance_gpu(
    py: Python<'_>,
    exe: Option<PathBuf>,
    winreg: Option<Bound<'_, PyAny>>,
    platform: Option<&str>,
) -> PyResult<&'static str> {
    let sys = py.import("sys")?;
    if !platform.map_or(cfg!(windows), |s| s.starts_with("win")) {
        return Ok("skipped");
    }
    let exe = match exe {
        Some(p) => p,
        None if sys
            .getattr("frozen")
            .is_ok_and(|v| v.is_truthy().unwrap_or(false)) =>
        {
            sys.getattr("executable")?.extract()?
        }
        None => return Ok("skipped"),
    };
    let reg = match winreg {
        Some(reg) => reg,
        None => match py.import("winreg") {
            Ok(reg) => reg.into_any(),
            Err(_) => return Ok("failed"),
        },
    };
    Ok(set_preference(&reg, &exe).unwrap_or("failed"))
}
