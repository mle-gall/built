// SPDX-License-Identifier: GPL-3.0-or-later
//! A store refusal is reported against the restored mesh. Deduplicate repeated
//! attempts on the same preview frame before writing a diagnostic.
use super::*;
pub(super) fn write(mesh: &Obj, op: &Obj, reason: &str) -> PyResult<()> {
    let py = mesh.py();
    let builtin = py.import("builtins")?;
    let st = mesh.getattr("_store")?;
    let arrays = PyDict::new(py);
    for (k, v) in st.call_method0("arrays")?.cast::<PyDict>()? {
        arrays.set_item(k, v.call_method0("tolist")?)?;
    }
    let indices = PyDict::new(py);
    for (i, k) in st.call_method1("keys", ("face",))?.try_iter()?.enumerate() {
        indices.set_item(k?, i)?;
    }
    let index = |face: &Obj| indices.call_method1("get", (face.getattr_opt("key")?,))?.extract::<Option<usize>>();
    let push = PyDict::new(py);
    push.set_item("face", index(&op.getattr("base_face")?)?)?;
    push.set_item("distance", op.getattr("extrusion")?)?;
    push.set_item("keep_base", flag(op, "_keep_base")?)?;
    let n = op.getattr("_normal")?;
    push.set_item(
        "normal",
        if n.is_none() {
            n
        } else {
            py.get_type::<PyList>()
                .call1((n.call_method0("toTuple")?,))?
        },
    )?;
    let extra = op
        .getattr("_extra_faces")?
        .try_iter()?
        .map(|f| index(&f?))
        .collect::<PyResult<Vec<_>>>()?;
    push.set_item("extra_faces", extra)?;
    let facade = py.import("ingetrazo_core.store_mesh")?;
    let owner = facade.call_method1("_attribute_owner", (mesh,))?;
    let attrs = PyDict::new(py);
    for (key, v) in owner.get_item("_attr_own")?.cast::<PyDict>()? {
        if let Some(i) = indices.get_item(key)? {
            attrs.set_item(i, v)?;
        }
    }
    let data = PyDict::new(py);
    data.set_item("reason", reason)?;
    data.set_item("push", push)?;
    data.set_item("mesh", arrays)?;
    data.set_item("attr_table", owner.get_item("_attr_table")?)?;
    data.set_item("own_attrs", attrs)?;
    let kw = PyDict::new(py);
    kw.set_item("sort_keys", true)?;
    kw.set_item("default", builtin.getattr("repr")?)?;
    let text = py
        .import("json")?
        .call_method("dumps", (data,), Some(&kw))?;
    let digest = text.hash()?;
    let written = module(py)?.getattr("_REPROS")?;
    if written.contains(digest)? {
        return Ok(());
    }
    written.call_method1("add", (digest,))?;
    let folder = match py.import("ingetrazo_core.paths")?.call_method0("user_log_dir") {
        Ok(d) => d.extract::<std::path::PathBuf>()?.join("push-refusals"),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) => return Ok(()),
        Err(e) => return Err(e),
    };
    let stamp = py
        .import("time")?
        .call_method1("strftime", ("%Y%m%d-%H%M%S",))?
        .extract::<String>()?;
    let path = folder.join(format!("{stamp}-{}.json", written.len()?));
    let contents = text.extract::<String>()?;
    if std::fs::create_dir_all(&folder)
        .and_then(|_| std::fs::write(&path, contents))
        .is_err()
    {
        return Ok(());
    }
    if let Some(log) = std::env::var_os("INGETRAZO_REFUSALS").filter(|l| !l.is_empty()) {
        let kw = PyDict::new(py);
        kw.set_item("repro", path.to_string_lossy())?;
        kw.set_item("reason", reason.chars().take(200).collect::<String>())?;
        facade.call_method("_log_refusal", (log, "push", py.None()), Some(&kw))?;
    }
    Ok(())
}
