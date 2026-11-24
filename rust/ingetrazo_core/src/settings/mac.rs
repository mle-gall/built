//! Use the same user/application/host tuple as the former QSettings backend.
//! CFPreferences merges per-key writes with the preferences daemon. Never
//! load and rewrite the user's entire plist.
use super::Value;
use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    boolean::CFBoolean,
    data::CFData,
    number::CFNumber,
    string::CFString,
};
use core_foundation_sys::{number::CFNumberIsFloatType, preferences::*};
use pyo3::{
    exceptions::{PyOSError, PyValueError},
    prelude::*,
};
use std::ptr;
const DOMAIN: &str = "com.ingetrazo.IngeTrazo";

fn key(s: &str) -> CFString {
    CFString::new(
        &s.chars()
            .map(|c| match c {
                '/' => '.',
                '.' => '·',
                '·' => '/',
                c => c,
            })
            .collect::<String>(),
    )
}
fn unkey(s: String) -> String {
    s.chars()
        .map(|c| match c {
            '.' => '/',
            '·' => '.',
            '/' => '·',
            c => c,
        })
        .collect()
}
fn string(s: &str) -> CFString {
    if s.starts_with('@') {
        CFString::new(&format!("@{s}"))
    } else {
        CFString::new(s)
    }
}
fn unstring(s: String) -> String {
    if s.starts_with("@@") {
        s[1..].into()
    } else {
        s
    }
}

pub fn get(k: &str) -> PyResult<Option<Value>> {
    let raw = unsafe {
        CFPreferencesCopyValue(
            key(k).as_concrete_TypeRef(),
            CFString::new(DOMAIN).as_concrete_TypeRef(),
            kCFPreferencesCurrentUser,
            kCFPreferencesAnyHost,
        )
    };
    if raw.is_null() {
        return Ok(None);
    }
    let v = unsafe { CFType::wrap_under_create_rule(raw) };
    let value = if let Some(s) = v.downcast::<CFString>() {
        let s = s.to_string();
        if s == "@Invalid()" {
            Value::Null
        } else {
            Value::Text(unstring(s))
        }
    } else if let Some(b) = v.downcast::<CFBoolean>() {
        Value::Bool(b.into())
    } else if let Some(n) = v.downcast::<CFNumber>() {
        if unsafe { CFNumberIsFloatType(n.as_concrete_TypeRef()) } != 0 {
            Value::Float(
                n.to_f64()
                    .ok_or_else(|| PyValueError::new_err("Invalid preference number"))?,
            )
        } else {
            Value::Int(
                n.to_i64()
                    .ok_or_else(|| PyValueError::new_err("Invalid preference integer"))?,
            )
        }
    } else if let Some(a) = v.downcast::<CFArray>() {
        let mut out = Vec::new();
        for raw in a.get_all_values() {
            let elem = unsafe { CFType::wrap_under_get_rule(raw) };
            let s = elem
                .downcast::<CFString>()
                .ok_or_else(|| PyValueError::new_err("Expected a list of preference strings"))?;
            out.push(unstring(s.to_string()));
        }
        Value::Strings(out)
    } else if let Some(d) = v.downcast::<CFData>() {
        Value::Bytes(d.bytes().to_vec())
    } else {
        return Err(PyValueError::new_err(format!(
            "Unsupported preference type for {k}"
        )));
    };
    Ok(Some(value))
}
pub fn keys() -> PyResult<Vec<String>> {
    let keys = unsafe {
        CFPreferencesCopyKeyList(
            CFString::new(DOMAIN).as_concrete_TypeRef(),
            kCFPreferencesCurrentUser,
            kCFPreferencesAnyHost,
        )
    };
    if keys.is_null() {
        return Ok(vec![]);
    }
    let a = unsafe { CFArray::<CFString>::wrap_under_create_rule(keys) };
    Ok(a.iter().map(|s| unkey(s.to_string())).collect())
}
pub fn sync() -> PyResult<()> {
    if unsafe {
        CFPreferencesSynchronize(
            CFString::new(DOMAIN).as_concrete_TypeRef(),
            kCFPreferencesCurrentUser,
            kCFPreferencesAnyHost,
        )
    } == 0
    {
        return Err(PyOSError::new_err(
            "Could not synchronize Built preferences",
        ));
    }
    Ok(())
}
pub fn put(k: &str, v: Option<&Value>) -> PyResult<()> {
    let val = v.map(|v| match v {
        Value::Null => CFString::new("@Invalid()").as_CFType(),
        Value::Text(s) => string(s).as_CFType(),
        Value::Bool(b) => CFBoolean::from(*b).as_CFType(),
        Value::Int(i) => CFNumber::from(*i).as_CFType(),
        Value::Float(f) => CFNumber::from(*f).as_CFType(),
        Value::Strings(l) => {
            CFArray::from_CFTypes(&l.iter().map(|s| string(s)).collect::<Vec<_>>()).as_CFType()
        }
        Value::Bytes(b) => CFData::from_buffer(b).as_CFType(),
    });
    unsafe {
        CFPreferencesSetValue(
            key(k).as_concrete_TypeRef(),
            val.as_ref().map_or(ptr::null(), |v| v.as_CFTypeRef()),
            CFString::new(DOMAIN).as_concrete_TypeRef(),
            kCFPreferencesCurrentUser,
            kCFPreferencesAnyHost,
        );
    }
    sync()
}
pub fn remove(prefix: &str) -> PyResult<()> {
    let group = format!("{prefix}/");
    for k in keys()? {
        if prefix.is_empty() || k == prefix || k.starts_with(&group) {
            put(&k, None)?
        }
    }
    Ok(())
}
