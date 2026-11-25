// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::{PyDict, PyList};
use std::{collections::BTreeMap, env, fs, path::PathBuf};
fn pathlib(py: Python<'_>, p: PathBuf) -> PyResult<Obj<'_>> {
    py.import("pathlib")?.call_method1("Path", (p,))
}
fn formats(py: Python<'_>) -> PyResult<Module<'_>> {
    py.import("formats.built")
}
#[pyfunction]
fn _app_data_base(py: Python<'_>) -> PyResult<Obj<'_>> {
    let available = py
        .import("ingetrazo_render.shell_services")?
        .call_method0("available")?
        .is_truthy()?;
    let found = if !available
        && py.import("sys")?.getattr("platform")?.eq("win32")?
        && env::var_os("APPDATA").is_some_and(|s| !s.is_empty())
    {
        return pathlib(
            py,
            PathBuf::from(env::var_os("APPDATA").unwrap()).join("IngeTrazo/IngeTrazo"),
        );
    } else {
        py.import("ingetrazo_render")?.call_method1(
            "recovery_directory",
            (if available {
                env::var_os("BUILT_SETTINGS_DIR").filter(|s| !s.is_empty())
            } else {
                None
            },),
        )?
    };
    if found.is_truthy()? {
        py.import("pathlib")?.call_method1("Path", (found,))
    } else {
        py.import("pathlib")?
            .getattr("Path")?
            .call_method0("home")?
            .call_method1("joinpath", (".ingetrazo",))
    }
}
fn subdir<'p>(m: &Module<'p>, base: &str, part: &str) -> PyResult<Obj<'p>> {
    let dir = m.call_method0(base)?.extract::<PathBuf>()?.join(part);
    fs::create_dir_all(&dir)?;
    pathlib(m.py(), dir)
}
#[pyfunction(pass_module)]
fn autosave_dir<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    subdir(m, "_app_data_base", "autosave")
}
#[pyfunction(pass_module)]
fn discarded_dir<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    subdir(m, "autosave_dir", "descartados")
}
#[pyfunction(pass_module,signature=(path,untitled=1))]
fn slot_for<'p>(m: &Module<'p>, path: Option<&Obj<'p>>, untitled: i64) -> PyResult<Obj<'p>> {
    let py = m.py();
    let stem = if let Some(p) = path {
        let full = py
            .import("pathlib")?
            .call_method1("Path", (p,))?
            .call_method0("absolute")?;
        let bytes = full.str()?.call_method1("encode", ("utf-8",))?;
        let hash = py
            .import("hashlib")?
            .call_method1("sha1", (bytes,))?
            .call_method0("hexdigest")?
            .extract::<String>()?;
        format!("{}-{}", full.getattr("stem")?, &hash[..8])
    } else if untitled <= 1 {
        "untitled".to_owned()
    } else {
        format!("untitled-{untitled}")
    };
    m.call_method0("autosave_dir")?.call_method1(
        "joinpath",
        (format!("{stem}{}", formats(py)?.getattr("BUILT_SUFFIX")?),),
    )
}
#[pyfunction(pass_module,signature=(scene,path,untitled=1))]
fn write<'p>(
    m: &Module<'p>,
    scene: &Obj<'p>,
    path: Option<&Obj<'p>>,
    untitled: i64,
) -> PyResult<Obj<'p>> {
    let slot = m.call_method1("slot_for", (path, untitled))?;
    let kw = PyDict::new(m.py());
    kw.set_item("backup", false)?;
    m.py()
        .import("formats.igz")?
        .call_method("save_scene", (scene, &slot), Some(&kw))?;
    Ok(slot)
}
#[pyfunction(pass_module,signature=(path,untitled=1))]
fn pending<'p>(m: &Module<'p>, path: Option<&Obj<'p>>, untitled: i64) -> PyResult<Option<Obj<'p>>> {
    let slot = m.call_method1("slot_for", (path, untitled))?;
    let old = slot.call_method1("with_suffix", (".igz",))?;
    for p in [slot, old] {
        if p.call_method0("is_file")?.is_truthy()? {
            return Ok(Some(p));
        }
    }
    Ok(None)
}
#[pyfunction(pass_module)]
fn pending_untitled<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    let fmt = formats(m.py())?;
    let suff = fmt.getattr("DOCUMENT_SUFFIXES")?;
    let mut found = BTreeMap::new();
    for slot in m
        .call_method0("autosave_dir")?
        .call_method1("glob", ("untitled*",))?
        .try_iter()?
    {
        let slot = slot?;
        let stem = slot.getattr("stem")?.extract::<String>()?;
        let n = if stem == "untitled" {
            1
        } else if let Some(n) = stem
            .strip_prefix("untitled-")
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|v| v.parse::<u64>().ok())
        {
            n
        } else {
            continue;
        };
        if suff.contains(slot.getattr("suffix")?)?
            && slot.call_method0("is_file")?.is_truthy()?
            && (!found.contains_key(&n) || fmt.call_method1("is_built", (&slot,))?.is_truthy()?)
        {
            found.insert(n, slot);
        }
    }
    Ok(PyList::new(m.py(), found)?.into_any())
}
#[pyfunction(pass_module)]
fn slot_epoch(m: &Module<'_>, slot: &Obj<'_>) -> PyResult<i64> {
    m.getattr("_slot_epochs")?
        .call_method1("get", (slot, 0))?
        .extract()
}
#[pyfunction(pass_module,signature=(path,untitled=1))]
fn clear(m: &Module<'_>, path: Option<&Obj<'_>>, untitled: i64) -> PyResult<()> {
    let slot = m.call_method1("slot_for", (path, untitled))?;
    m.getattr("_slot_epochs")?
        .set_item(&slot, slot_epoch(m, &slot)? + 1)?;
    let legacy = slot.call_method1("with_suffix", (".igz",))?;
    for p in [slot, legacy] {
        if p.call_method0("is_file")?.is_truthy()? {
            m.call_method1("_retire", (p,))?;
        }
    }
    Ok(())
}
#[pyfunction(pass_module)]
fn _retire(m: &Module<'_>, slot: &Obj<'_>) -> PyResult<()> {
    let py = m.py();
    let dir = m.call_method0("discarded_dir")?.extract::<PathBuf>()?;
    let stamp = py
        .import("datetime")?
        .getattr("datetime")?
        .call_method0("now")?
        .call_method1("strftime", ("%Y%m%d-%H%M%S",))?;
    let target = dir.join(format!(
        "{}-{stamp}{}",
        slot.getattr("stem")?,
        slot.getattr("suffix")?
    ));
    let original = slot.extract::<PathBuf>()?;
    if let Err(e) = slot.call_method1("replace", (target,)) {
        if !e.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
            return Err(e);
        }
        if original.exists() {
            fs::remove_file(original)?;
        }
        return Ok(());
    }
    let suffixes = formats(py)?.getattr("DOCUMENT_SUFFIXES")?;
    let mut kept = Vec::new();
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if let Some(s) = p.extension().and_then(|s| s.to_str()) {
            if suffixes.contains(format!(".{s}"))? {
                kept.push((fs::metadata(&p)?.modified()?, p));
            }
        }
    }
    kept.sort_by(|a, b| b.0.cmp(&a.0));
    let cap = m.getattr("KEEP_DISCARDED")?.extract::<usize>()?;
    for (_, p) in kept.into_iter().skip(cap) {
        let _ = fs::remove_file(p);
    }
    Ok(())
}
pub(super) fn install(m: &Module<'_>) -> PyResult<()> {
    m.add("_slot_epochs", PyDict::new(m.py()))?;
    m.add("KEEP_DISCARDED", 20)?;
    for f in [
        wrap_pyfunction!(_app_data_base, m)?,
        wrap_pyfunction!(autosave_dir, m)?,
        wrap_pyfunction!(discarded_dir, m)?,
        wrap_pyfunction!(slot_for, m)?,
        wrap_pyfunction!(write, m)?,
        wrap_pyfunction!(pending, m)?,
        wrap_pyfunction!(pending_untitled, m)?,
        wrap_pyfunction!(slot_epoch, m)?,
        wrap_pyfunction!(clear, m)?,
        wrap_pyfunction!(_retire, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
