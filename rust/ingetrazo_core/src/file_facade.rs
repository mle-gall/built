// SPDX-License-Identifier: GPL-3.0-or-later
//! Precision and archive operations exposed to script-facing subclasses.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::PyDict,
};
use std::path::PathBuf;
type Obj<'p> = Bound<'p, PyAny>;
#[pyfunction]
fn is_built(path: PathBuf) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("built"))
}
#[pyfunction]
fn built_displacement<'p>(
    py: Python<'p>,
    points: &Obj<'p>,
    compact: &Obj<'p>,
    placements: &Obj<'p>,
) -> PyResult<f64> {
    let np = py.import("numpy")?;
    let array = |o: &Obj<'p>| np.call_method1("asarray", (o, "float64"));
    let p = array(points)?;
    let q = array(compact)?;
    let p = p.extract::<numpy::PyReadonlyArray2<f64>>()?;
    let q = q.extract::<numpy::PyReadonlyArray2<f64>>()?;
    let p = p.as_array();
    let q = q.as_array();
    if p.shape() != q.shape() || p.ncols() != 3 {
        return Err(PyValueError::new_err(
            "expected matching N by 3 coordinates",
        ));
    }
    let mut maximum = 0f64;
    for pair in placements.try_iter()? {
        let pair = pair?;
        let a = array(&pair.get_item(0)?)?;
        let b = array(&pair.get_item(1)?)?;
        let a = a.extract::<numpy::PyReadonlyArray2<f64>>()?;
        let b = b.extract::<numpy::PyReadonlyArray2<f64>>()?;
        let a = a.as_array();
        let b = b.as_array();
        if a.shape() != [4, 4] || b.shape() != [4, 4] {
            return Err(PyValueError::new_err("expected 4 by 4 placements"));
        }
        for i in 0..p.nrows() {
            let mut delta = [0.; 3];
            for r in 0..3 {
                delta[r] = a[[r, 3]] - b[[r, 3]];
                for k in 0..3 {
                    delta[r] +=
                        (p[[i, k]] - q[[i, k]]) * a[[r, k]] + q[[i, k]] * (a[[r, k]] - b[[r, k]]);
                }
            }
            maximum = maximum.max(glam::DVec3::from_array(delta).length());
        }
    }
    Ok(maximum * 1000.)
}
#[pyfunction]
fn document_native_io(py: Python<'_>) -> PyResult<Option<Obj<'_>>> {
    if std::env::var_os("INGETRAZO_RUST_IO").as_deref() == Some(std::ffi::OsStr::new("0")) {
        return Ok(None);
    }
    let module = py.import("core._native")?.call_method0("available")?;
    Ok(module.hasattr("FileGroups")?.then_some(module))
}
#[pyfunction]
fn igz_members_init(members: &Obj<'_>) -> PyResult<()> {
    members.setattr("checked", PyDict::new(members.py()))
}
#[pyfunction(signature=(archive,name,pwd=None))]
fn document_archive_read<'p>(
    py: Python<'p>,
    archive: &Obj<'p>,
    name: &Obj<'p>,
    pwd: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let zip = py.import("zipfile")?;
    let native = document_native_io(py)?.unwrap_or_else(||py.None().into_bound(py));
    let Some(reader) = native.getattr_opt("igz_read_member")? else {
        return zip
            .getattr("ZipFile")?
            .call_method1("read", (archive, name, pwd));
    };
    let info = if name.is_instance(&zip.getattr("ZipInfo")?)? {
        name.clone()
    } else {
        archive.call_method1("getinfo", (name,))?
    };
    let kw = PyDict::new(py);
    kw.set_item("pwd", pwd)?;
    let member = archive.call_method("open", (&info,), Some(&kw))?;
    let result = (|| {
        if let Some(offset) = member
            .getattr_opt("_orig_compress_start")?
            .filter(|v| !v.is_none())
        {
            if info.getattr("flag_bits")?.extract::<u32>()? & 1 == 0 {
                let blob = reader.call1((
                    archive.getattr("fp")?.call_method0("fileno")?,
                    offset,
                    info.getattr("compress_size")?,
                    info.getattr("file_size")?,
                    info.getattr("compress_type")?,
                    info.getattr("CRC")?,
                ))?;
                if !blob.is_none() {
                    return Ok(blob);
                }
            }
        }
        member.call_method0("read")
    })();
    member.call_method0("close")?;
    result
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,m)?)?;)*}}
    add!(
        is_built,
        built_displacement,
        igz_members_init,
        document_archive_read,
        document_native_io
    );
    Ok(())
}
