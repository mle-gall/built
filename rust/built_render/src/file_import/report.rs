// SPDX-License-Identifier: GPL-3.0-or-later
//! User-facing exchange results and the technical record behind Details.
use crate::settings_rows::{tr, trf};
use pyo3::{prelude::*, types::PyDict};
type Obj<'a> = Bound<'a, PyAny>;

pub(crate) fn silence_progress(bar: &Obj) -> PyResult<()> {
    if let Some(job) = bar.getattr_opt("job")? {
        let kw = PyDict::new(bar.py());
        kw.set_item("announce", false)?;
        job.call_method("end", (), Some(&kw))?;
    }
    Ok(())
}
pub(crate) fn finish_progress(bar: &Obj) -> PyResult<()> {
    silence_progress(bar)?;
    bar.call_method0("close")?;
    Ok(())
}

pub fn log(py: Python, name: &str, details: &str) -> PyResult<()> {
    use std::{fs::OpenOptions, io::Write, path::PathBuf};
    let folder = py
        .import("built_core.paths")?
        .call_method0("user_log_dir")?
        .extract::<PathBuf>()?;
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(folder.join("built-import.log"))
    {
        let _ = file.write_all(format!("{name}\n{details}\n").as_bytes());
    }
    py.import("logging")?
        .call_method1("getLogger", ("formats.import",))?
        .call_method1("warning", ("%s\n%s", name, details))?;
    Ok(())
}

fn count(doc: &Obj, key: &str) -> PyResult<usize> {
    doc.get_item(key)?.extract()
}

pub fn summary(doc: &Obj) -> PyResult<String> {
    let py = doc.py();
    let unavailable = count(doc, "rejected")? + count(doc, "missing")?;
    let total = unavailable + count(doc, "exact")? + count(doc, "faceted")?;
    let mut lines = vec![if unavailable == 0 {
        tr(py, "Some STEP records could not be read.")?
    } else {
        trf(
            py,
            "{n} of {total} bodies can’t be imported.",
            &[
                ("n", unavailable.into_pyobject(py)?.into_any()),
                ("total", total.into_pyobject(py)?.into_any()),
            ],
        )?
    }];
    let issues = doc.call_method1("get", ("body_issues", Vec::<String>::new()))?;
    for row in issues.try_iter()?.take(4) {
        let (part, kind): (Obj, String) = row?.extract()?;
        let args = [("part", part)];
        lines.push(match kind.as_str() {
            "shell" => trf(py, "{part} has a damaged closed shell.", &args)?,
            "boundary" => trf(
                py,
                "{part} has a face boundary that could not be verified.",
                &args,
            )?,
            _ => trf(py, "{part} contains a shape that could not be read.", &args)?,
        });
    }
    Ok(lines.join("\n"))
}

#[pyfunction]
fn dismissed(_answer: &Obj) {}

#[pyfunction]
fn show_report(win: &Obj, title: &str, body: &str, details: &str) -> PyResult<()> {
    let py = win.py();
    let kw = PyDict::new(py);
    kw.set_item("details", details)?;
    kw.set_item("default", "ok")?;
    kw.set_item("cancel", "ok")?;
    kw.set_item("tone", "warning")?;
    py.import("built_render.chrome_dialogs")?.call_method(
        "ask_later",
        (
            win,
            title,
            body,
            [("ok", tr(py, "OK")?)],
            wrap_pyfunction!(dismissed, py)?,
        ),
        Some(&kw),
    )?;
    Ok(())
}

fn notice(win: &Obj, title: &str, warning: Option<(&str, &str)>) -> PyResult<()> {
    let py = win.py();
    let kw = PyDict::new(py);
    if let Some((body, details)) = warning {
        let action = py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(show_report, py)?,
                win,
                title,
                body,
                details,
            ),
        )?;
        kw.set_item("tone", "warning")?;
        kw.set_item("action_label", tr(py, "Show")?)?;
        kw.set_item("action", action)?;
    } else {
        kw.set_item("tone", "success")?;
    }
    py.import("built_render.chrome_dialogs")?
        .call_method("notify", (win, title, ""), Some(&kw))?;
    Ok(())
}

pub(crate) fn failed(win: &Obj, name: &str, detail: &str) -> PyResult<()> {
    let py = win.py();
    log(py, name, detail)?;
    let title = trf(
        py,
        "Couldn’t import {name}",
        &[("name", name.into_pyobject(py)?.into_any())],
    )?;
    notice(
        win,
        &title,
        Some((
            &tr(py, "The file could not be added to the model.")?,
            detail,
        )),
    )
}

pub fn success(win: &Obj, name: &str, doc: &Obj) -> PyResult<()> {
    let py = win.py();
    let mut parts = vec![trf(
        py,
        "Imported {name}",
        &[("name", name.into_pyobject(py)?.into_any())],
    )?];
    for key in ["exact", "faceted", "rejected", "missing"] {
        let n = count(doc, key)?;
        let args = [("n", n.into_pyobject(py)?.into_any())];
        parts.push(match (key, n) {
            (_, 0) => continue,
            ("exact", 1) => tr(py, "1 exact body")?,
            ("exact", _) => trf(py, "{n} exact bodies", &args)?,
            ("faceted", 1) => tr(py, "1 faceted body")?,
            ("faceted", _) => trf(py, "{n} faceted bodies", &args)?,
            ("rejected", 1) => tr(py, "1 rejected body")?,
            ("rejected", _) => trf(py, "{n} rejected bodies", &args)?,
            (_, 1) => tr(py, "1 missing body")?,
            _ => trf(py, "{n} missing bodies", &args)?,
        });
    }
    if doc.get_item("synthetic_file_name")?.is_truthy()? {
        parts.push(tr(py, "Supplied missing STEP file name")?);
    }
    let title = parts.join(" · ");
    let details = doc
        .get_item("problems")?
        .extract::<Vec<String>>()?
        .join("\n");
    if details.is_empty() {
        notice(win, &title, None)
    } else {
        notice(win, &title, Some((&summary(doc)?, &details)))
    }
}
