// SPDX-License-Identifier: GPL-3.0-or-later
//! Bound inflated member and aggregate sizes before reading external ZIP data.
use pyo3::{prelude::*, types::PyType};
const MAX_TOTAL: u128 = 4 << 30;
const RATIO_FROM: u128 = 32 << 20;
fn too_dense(size: u128, packed: u128) -> bool {
    size > 200 * packed.max(1)
}
fn refused(py: Python<'_>, message: String) -> PyErr {
    match py
        .import("built_core")
        .and_then(|m| m.getattr("ArchiveTooLarge"))
        .and_then(|ty| ty.call1((message,)))
    {
        Ok(err) => PyErr::from_value(err),
        Err(e) => e,
    }
}
fn sizes(info: &Bound<PyAny>, name: &str, member: Option<&str>) -> PyResult<(u128, u128)> {
    let size = u128::from(info.getattr("file_size")?.extract::<u64>()?);
    let packed = u128::from(info.getattr("compress_size")?.extract::<u64>()?);
    if size > RATIO_FROM && too_dense(size, packed) {
        let file = match member {
            Some(m) => m.into(),
            None => format!("{name}: {}", info.getattr("filename")?.str()?),
        };
        return Err(refused(
            info.py(),
            format!("{file} unpacks to {size} bytes from {packed}; refused."),
        ));
    }
    Ok((size, packed))
}
#[pyfunction(signature=(archive,name=""))]
pub(crate) fn check_archive(archive: &Bound<PyAny>, name: &str) -> PyResult<()> {
    let label = if name.is_empty() { "the archive" } else { name };
    let (mut total, mut compressed) = (0, 0);
    for info in archive.call_method0("infolist")?.try_iter()? {
        let (size, packed) = sizes(&info?, label, None)?;
        total += size;
        compressed += packed;
    }
    if total > MAX_TOTAL && too_dense(total, compressed) {
        return Err(refused(
            archive.py(),
            format!("{label} unpacks to {total} bytes, more than {MAX_TOTAL}; refused."),
        ));
    }
    Ok(())
}
pub(crate) fn read_member<'p>(
    archive: &Bound<'p, PyAny>,
    member: &str,
) -> PyResult<Bound<'p, PyAny>> {
    sizes(
        &archive.call_method1("getinfo", (member,))?,
        "",
        Some(member),
    )?;
    archive.call_method1("read", (member,))
}
pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let exc = PyErr::new_type(
        py,
        c"built_core.ArchiveTooLarge",
        None,
        Some(
            py.import("zipfile")?
                .getattr("BadZipFile")?
                .cast::<PyType>()?,
        ),
        None,
    )?;
    m.add("ArchiveTooLarge", exc)?;
    m.add_function(wrap_pyfunction!(check_archive, m)?)
}
