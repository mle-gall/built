use super::*;
pub(super) fn warn(py: Python, text: &str) -> PyResult<()> {
    py.import("logging")?
        .call_method1("getLogger", ("built.extensions",))?
        .call_method1("warning", (text,))?;
    Ok(())
}
fn parts(v: &str) -> Vec<u64> {
    let v = v.trim_start();
    let end = v
        .bytes()
        .take_while(|c| c.is_ascii_digit() || *c == b'.')
        .count();
    v[..end].split('.').map_while(|n| n.parse().ok()).collect()
}
#[pyfunction]
pub(super) fn compatible_with_current(extension: &Obj) -> PyResult<bool> {
    let mut required = parts(&s(extension, "minimum_version")?);
    let mut current = parts(&version(extension.py())?);
    if current.is_empty() || required.is_empty() {
        return Ok(false);
    }
    let width = current.len().max(required.len());
    current.resize(width, 0);
    required.resize(width, 0);
    Ok(required <= current)
}
