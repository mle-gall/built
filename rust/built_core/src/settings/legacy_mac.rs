//! Copy legacy preferences once, retaining destination values and their CF types.
use super::{DOMAIN, CFString, TCFType, CFType, CFArray};
use core_foundation_sys::preferences::*;
use pyo3::{exceptions::PyOSError, prelude::*};
use std::{fs, path::PathBuf};

pub(crate) fn migrate() -> PyResult<()> {
    static DONE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if DONE.get().is_some() { return Ok(()); }
    let home = std::env::var_os("CFFIXED_USER_HOME").or_else(|| std::env::var_os("HOME"))
        .ok_or_else(|| PyOSError::new_err("Missing preferences home"))?;
    let dir = PathBuf::from(home).join("Library/Application Support/Built");
    fs::create_dir_all(&dir)?;
    let marker = dir.join(".legacy-preferences-copied-v1");
    if !marker.try_exists()? {
        let lock = fs::OpenOptions::new().create(true).truncate(false).write(true)
            .open(dir.join(".preferences-migration.lock"))?;
        lock.lock()?;
        if !marker.try_exists()? {
            let old = CFString::new("com.ingetrazo.IngeTrazo");
            let new = CFString::new(DOMAIN);
            unsafe {
                let user = kCFPreferencesCurrentUser;
                let host = kCFPreferencesAnyHost;
                let keys = CFPreferencesCopyKeyList(old.as_concrete_TypeRef(), user, host);
                if !keys.is_null() {
                    let keys = CFArray::<CFString>::wrap_under_create_rule(keys);
                    for key in keys.iter() {
                        let key = key.as_concrete_TypeRef();
                        let current = CFPreferencesCopyValue(key, new.as_concrete_TypeRef(), user, host);
                        if !current.is_null() {
                            drop(CFType::wrap_under_create_rule(current));
                            continue;
                        }
                        let value = CFPreferencesCopyValue(key, old.as_concrete_TypeRef(), user, host);
                        if !value.is_null() {
                            let value = CFType::wrap_under_create_rule(value);
                            CFPreferencesSetValue(key, value.as_CFTypeRef(), new.as_concrete_TypeRef(), user, host);
                        }
                    }
                }
                if CFPreferencesSynchronize(new.as_concrete_TypeRef(), user, host) == 0 {
                    return Err(PyOSError::new_err("Could not migrate Built preferences"));
                }
            }
            fs::File::create_new(marker)?.sync_all()?;
        }
    }
    DONE.get_or_init(|| ());
    Ok(())
}
