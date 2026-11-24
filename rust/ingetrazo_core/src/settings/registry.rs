//! Use the user's existing Windows registry domain. winreg is the platform
//! binding shipped with the embedded interpreter.
use super::Value;
use pyo3::{exceptions::PyFileNotFoundError, prelude::*, types::PyBytes};
const ROOT: &str = r"Software\IngeTrazo\IngeTrazo";
fn parts(key: &str) -> (String, &str) {
    let (path, name) = key.rsplit_once('/').unwrap_or(("", key));
    let path = if path.is_empty() {
        ROOT.to_owned()
    } else {
        format!("{ROOT}\\{}", path.replace('/', "\\"))
    };
    (path, name)
}
pub fn get(key: &str) -> PyResult<Option<Value>> {
    Python::attach(|py| {
        let reg = py.import("winreg")?;
        let (path, name) = parts(key);
        let read = || -> PyResult<Value> {
            let h = reg.call_method1("OpenKey", (reg.getattr("HKEY_CURRENT_USER")?, path))?;
            let pair = reg.call_method1("QueryValueEx", (h, name))?;
            let obj = pair.get_item(0)?;
            if let Ok(s) = obj.extract::<String>() {
                return Ok(if s == "@Invalid()" {
                    Value::Null
                } else {
                    Value::Text(
                        s.strip_prefix("@@")
                            .map(|tail| format!("@{tail}"))
                            .unwrap_or(s),
                    )
                });
            }
            Value::from_py(&obj)
        };
        match read() {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.is_instance_of::<PyFileNotFoundError>(py) => Ok(None),
            Err(e) => Err(e),
        }
    })
}
pub fn put(key: &str, value: Option<&Value>) -> PyResult<()> {
    Python::attach(|py| {
        let reg = py.import("winreg")?;
        let (path, name) = parts(key);
        let h = reg.call_method1("CreateKey", (reg.getattr("HKEY_CURRENT_USER")?, path))?;
        if let Some(v) = value {
            let (kind, obj) = match v {
                Value::Bytes(v) => ("REG_BINARY", PyBytes::new(py, v).into_any()),
                Value::Strings(_) => ("REG_MULTI_SZ", v.to_py(py)?),
                Value::Int(i) if *i >= 0 && *i <= u32::MAX as i64 => ("REG_DWORD", v.to_py(py)?),
                Value::Int(i) if *i >= 0 => ("REG_QWORD", v.to_py(py)?),
                other => {
                    let s = match other {
                        Value::Null => "@Invalid()".into(),
                        Value::Text(s) if s.starts_with('@') => format!("@{s}"),
                        Value::Text(s) => s.clone(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Int(i) => i.to_string(),
                        _ => unreachable!(),
                    };
                    ("REG_SZ", s.into_pyobject(py)?.into_any())
                }
            };
            reg.call_method1("SetValueEx", (h, name, 0, reg.getattr(kind)?, obj))?;
        } else {
            reg.call_method1("DeleteValue", (h, name))?;
        }
        Ok(())
    })
}
fn walk(
    reg: &Bound<'_, PyModule>,
    h: &Bound<'_, PyAny>,
    prefix: &str,
    out: &mut Vec<String>,
) -> PyResult<()> {
    let info = reg.call_method1("QueryInfoKey", (h,))?;
    for i in 0..info.get_item(1)?.extract::<usize>()? {
        let name: String = reg
            .call_method1("EnumValue", (h, i))?
            .get_item(0)?
            .extract()?;
        out.push(format!("{prefix}{name}"))
    }
    for i in 0..info.get_item(0)?.extract::<usize>()? {
        let name: String = reg.call_method1("EnumKey", (h, i))?.extract()?;
        let sub = reg.call_method1("OpenKey", (h, &name))?;
        walk(reg, &sub, &format!("{prefix}{name}/"), out)?;
    }
    Ok(())
}
pub fn keys() -> PyResult<Vec<String>> {
    Python::attach(|py| {
        let reg = py.import("winreg")?;
        let mut out = vec![];
        match reg.call_method1("OpenKey", (reg.getattr("HKEY_CURRENT_USER")?, ROOT)) {
            Ok(h) => walk(&reg, &h, "", &mut out)?,
            Err(e) if e.is_instance_of::<PyFileNotFoundError>(py) => {}
            Err(e) => return Err(e),
        }
        Ok(out)
    })
}
pub fn remove(key: &str) -> PyResult<()> {
    let prefix = format!("{key}/");
    for entry in keys()? {
        if key.is_empty() || entry == key || entry.starts_with(&prefix) {
            put(&entry, None)?
        }
    }
    Ok(())
}
