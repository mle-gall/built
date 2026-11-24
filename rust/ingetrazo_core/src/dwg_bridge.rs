// SPDX-License-Identifier: GPL-3.0-or-later
//! Locate LibreDWG and repair its tag stream before opening the drawing.
use pyo3::{prelude::*, types::PyDict};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.dwg_bridge")
}
fn path_obj(py: Python<'_>, p: PathBuf) -> PyResult<Obj<'_>> {
    py.import("pathlib")?.call_method1("Path", (p,))
}
fn issue(py: Python<'_>, text: String, kind: &str) -> PyErr {
    match api(py).and_then(|m| m.call_method1("DwgBridgeError", (text, kind))) {
        Ok(e) => PyErr::from_value(e),
        Err(e) => e,
    }
}
fn executable(path: &Path, os: &str) -> bool {
    let mut head = [0; 4];
    let Ok(n) = fs::File::open(path).and_then(|mut f| f.read(&mut head)) else {
        return false;
    };
    match os {
        "win32" => head[..n].starts_with(b"MZ"),
        "linux" => head == *b"\x7fELF",
        "darwin" => matches!(
            u32::from_be_bytes(head),
            0xcffaedfe | 0xcefaedfe | 0xfeedfacf | 0xfeedface | 0xcafebabe | 0xbebafeca
        ),
        _ => true,
    }
}
#[pyfunction]
fn dwg_native_executable(py: Python<'_>, path: PathBuf) -> PyResult<bool> {
    Ok(executable(
        &path,
        &py.import("sys")?.getattr("platform")?.extract::<String>()?,
    ))
}
#[pyfunction]
fn dwg_find_tool<'p>(py: Python<'p>, name: &str) -> PyResult<Option<Obj<'p>>> {
    let module = api(py)?;
    let os = py.import("sys")?.getattr("platform")?.extract::<String>()?;
    let file = if os == "win32" {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    for key in ["_VENDOR_BIN", "_SIBLING_BIN"] {
        let p = module.getattr(key)?.extract::<PathBuf>()?.join(&file);
        if p.is_file() && executable(&p, &os) {
            return Ok(Some(path_obj(py, p)?));
        }
    }
    let system = py
        .import("shutil")?
        .call_method1("which", (name,))?
        .extract::<Option<PathBuf>>()?;
    system
        .filter(|p| executable(p, &os))
        .map(|p| path_obj(py, p))
        .transpose()
}
#[pyfunction]
fn dwg_find_converter(py: Python<'_>) -> PyResult<Obj<'_>> {
    api(py)?.call_method1("_find_tool", ("dwg2dxf",))
}
#[pyfunction]
fn dwg_supported(py: Python<'_>) -> PyResult<bool> {
    Ok(!api(py)?.call_method0("find_dwg2dxf")?.is_none())
}
#[pyfunction]
fn dwg_check_output(
    py: Python<'_>,
    out_path: PathBuf,
    returncode: &Obj<'_>,
    log: &str,
) -> PyResult<()> {
    if out_path
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() > 0)
    {
        return Ok(());
    }
    let lines: Vec<_> = log.trim().lines().collect();
    let mut tail = lines[lines.len().saturating_sub(3)..].join(" | ");
    if tail.is_empty() {
        tail = format!("rc={}", returncode.str()?);
    }
    Err(issue(
        py,
        format!("conversion produced no output: {tail}"),
        "failed",
    ))
}
#[pyfunction]
fn dwg_run(py: Python<'_>, cmd: Vec<String>, out_path: PathBuf) -> PyResult<String> {
    let proc = py.import("subprocess")?;
    let kw = PyDict::new(py);
    kw.set_item("capture_output", true)?;
    kw.set_item("text", true)?;
    kw.set_item("encoding", "utf-8")?;
    kw.set_item("errors", "replace")?;
    kw.set_item("timeout", api(py)?.getattr("_TIMEOUT")?)?;
    kw.set_item("check", false)?;
    let output = match proc.call_method("run", (&cmd,), Some(&kw)) {
        Err(e) if e.value(py).is_instance(&proc.getattr("TimeoutExpired")?)? => {
            let err = issue(
                py,
                format!("converter timed out: {}", cmd.join(" ")),
                "timeout",
            );
            err.set_cause(py, Some(e));
            return Err(err);
        }
        other => other?,
    };
    let stderr = output
        .getattr("stderr")?
        .extract::<Option<String>>()?
        .unwrap_or_default();
    let log = if stderr.is_empty() {
        output
            .getattr("stdout")?
            .extract::<Option<String>>()?
            .unwrap_or_default()
    } else {
        stderr.clone()
    };
    dwg_check_output(py, out_path, &output.getattr("returncode")?, &log)?;
    Ok(stderr)
}
fn repair(path: &Path, null: bool) -> std::io::Result<usize> {
    // The format is byte-oriented. C1 bytes (including NEL) are legal data.
    let raw = fs::read(path)?;
    let mut lines: Vec<_> = raw.split(|&b| b == b'\n').collect();
    if null {
        let mut n = 0;
        let mut out = Vec::new();
        let mut tags = lines.chunks_exact(2);
        for pair in tags.by_ref() {
            if pair[0].trim_ascii() == b"5" && pair[1].trim_ascii() == b"0" {
                n += 1;
            } else {
                out.extend_from_slice(pair);
            }
        }
        out.extend_from_slice(tags.remainder());
        if n > 0 {
            fs::write(path, out.join(&b'\n'))?;
        }
        return Ok(n);
    }
    let mut used = HashSet::new();
    let mut collisions = vec![];
    let mut seed = None;
    let mut maximum = 0u64;
    let mut section = None;
    let mut expecting = false;
    for i in (0..lines.len().saturating_sub(1)).step_by(2) {
        let value = lines[i + 1].trim_ascii();
        match lines[i].trim_ascii() {
            b"0" => {
                if value == b"ENDSEC" {
                    section = None;
                }
                expecting = !matches!(value, b"SECTION" | b"ENDSEC" | b"EOF");
            }
            b"2" if section.is_none() => section = Some(value),
            b"9" if value == b"$HANDSEED" && i + 3 < lines.len() => {
                seed = (lines[i + 2].trim_ascii() == b"5").then_some(i + 3)
            }
            b"5" if expecting
                && matches!(
                    section,
                    Some(b"TABLES" | b"BLOCKS" | b"ENTITIES" | b"OBJECTS")
                ) =>
            {
                if !used.insert(value) {
                    collisions.push(i + 1);
                }
                if let Some(h) = std::str::from_utf8(value)
                    .ok()
                    .and_then(|s| u64::from_str_radix(s, 16).ok())
                    .filter(|&n| n < 1 << 32)
                {
                    maximum = maximum.max(h);
                }
                expecting = false;
            }
            _ => (),
        }
    }
    let n = collisions.len();
    if n == 0 {
        return Ok(0);
    }
    let mut next = maximum + 1;
    let mut changed = vec![];
    for i in collisions {
        loop {
            let key = format!("{next:X}");
            next += 1;
            if !used.contains(key.as_bytes()) {
                changed.push((i, key));
                break;
            }
        }
    }
    if let Some(i) = seed {
        changed.push((i, format!("{next:X}")));
    }
    for (i, key) in &changed {
        lines[*i] = key.as_bytes();
    }
    fs::write(path, lines.join(&b'\n'))?;
    Ok(n)
}
#[pyfunction]
fn dwg_strip_null(py: Python<'_>, dxf_path: PathBuf) -> PyResult<()> {
    py.detach(|| repair(&dxf_path, true))?;
    Ok(())
}
#[pyfunction]
fn dwg_dedupe(py: Python<'_>, dxf_path: PathBuf) -> PyResult<usize> {
    Ok(py.detach(|| repair(&dxf_path, false))?)
}
#[pyfunction]
fn dwg_repair(py: Python<'_>, dxf_path: PathBuf) -> PyResult<()> {
    py.detach(|| {
        repair(&dxf_path, true)?;
        repair(&dxf_path, false)
    })?;
    Ok(())
}
#[pyfunction]
fn dwg_command(tool: &Obj<'_>, dwg_path: &Obj<'_>, out_dxf: &Obj<'_>) -> PyResult<Vec<String>> {
    Ok(vec![
        tool.str()?.to_string(),
        "-y".into(),
        "-o".into(),
        out_dxf.str()?.to_string(),
        dwg_path.str()?.to_string(),
    ])
}
#[pyfunction]
fn dwg_new_output(py: Python<'_>) -> PyResult<(Obj<'_>, Obj<'_>)> {
    let tool = api(py)?.call_method0("find_dwg2dxf")?;
    if tool.is_none() {
        return Err(issue(
            py,
            "LibreDWG (dwg2dxf) is not available".into(),
            "unavailable",
        ));
    }
    let kw = PyDict::new(py);
    kw.set_item("prefix", "ingetrazo-dwg-")?;
    let dir = py
        .import("tempfile")?
        .call_method("mkdtemp", (), Some(&kw))?
        .extract::<PathBuf>()?;
    Ok((tool, path_obj(py, dir.join("converted.dxf"))?))
}
#[pyfunction]
fn dwg_discard(dxf_path: PathBuf) {
    if let Some(p) = dxf_path.parent().filter(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("ingetrazo-dwg-"))
    }) {
        let _ = fs::remove_dir_all(p);
    }
}
#[pyfunction]
fn dwg_convert(py: Python<'_>, dwg_path: PathBuf) -> PyResult<Obj<'_>> {
    let m = api(py)?;
    let (tool, out) = m.call_method0("_new_output")?.extract::<(Obj, Obj)>()?;
    let result = (|| {
        let cmd = m.call_method1("_converter_command", (tool, dwg_path, &out))?;
        m.call_method1("_run", (cmd, &out))?;
        m.call_method1("_repair_dxf", (&out,))?;
        Ok(())
    })();
    if let Err(e) = result {
        m.call_method1("discard_temp_dxf", (&out,))?;
        return Err(e);
    }
    Ok(out)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,m)?)?;)*}}
    add!(
        dwg_native_executable,
        dwg_find_tool,
        dwg_find_converter,
        dwg_supported,
        dwg_check_output,
        dwg_run,
        dwg_strip_null,
        dwg_dedupe,
        dwg_repair,
        dwg_command,
        dwg_new_output,
        dwg_discard,
        dwg_convert
    );
    Ok(())
}
