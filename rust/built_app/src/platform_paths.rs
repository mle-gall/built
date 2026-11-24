//! Platform locations used by native recovery and file dialogs.
use pyo3::prelude::*;
use std::{env, path::PathBuf};

#[pyfunction]
pub fn preferred_languages() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        return objc2_foundation::NSLocale::preferredLanguages()
            .iter()
            .map(|s| s.to_string())
            .collect();
    }
    #[cfg(windows)]
    {
        use windows::core::PWSTR;
        use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
        let (mut count, mut size) = (0, 0);
        unsafe {
            if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, PWSTR::null(), &mut size)
                .is_ok()
            {
                let mut buf = vec![0; size as usize];
                if GetUserPreferredUILanguages(
                    MUI_LANGUAGE_NAME,
                    &mut count,
                    PWSTR(buf.as_mut_ptr()),
                    &mut size,
                )
                .is_ok()
                {
                    return buf
                        .split(|c| *c == 0)
                        .take_while(|s| !s.is_empty())
                        .map(String::from_utf16_lossy)
                        .collect();
                }
            }
        }
        Vec::new()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(env::var_os)
            .flat_map(|v| {
                v.to_string_lossy()
                    .split(':')
                    .map(|s| s.split(['.', '@']).next().unwrap_or(s).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

#[cfg(windows)]
fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    };
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let buf = std::ffi::OsString::from_wide(path.as_wide());
        CoTaskMemFree(Some(path.as_ptr().cast()));
        Some(buf.into())
    }
}

#[pyfunction]
pub fn documents_directory() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask};
        let urls = NSFileManager::defaultManager().URLsForDirectory_inDomains(
            NSSearchPathDirectory::DocumentDirectory,
            NSSearchPathDomainMask::UserDomainMask,
        );
        return urls
            .firstObject()?
            .path()
            .map(|s| PathBuf::from(s.to_string()));
    }
    #[cfg(windows)]
    {
        known_folder(&windows::Win32::UI::Shell::FOLDERID_Documents)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        std::process::Command::new("xdg-user-dir")
            .arg("DOCUMENTS")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| PathBuf::from(s.trim_end()))
            .filter(|p| p.is_absolute())
            .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join("Documents")))
    }
}

#[pyfunction]
pub fn recovery_directory(profile: Option<PathBuf>) -> Option<PathBuf> {
    let root = data_root(profile, true)?;
    Some(root.join("IngeTrazo").join("IngeTrazo"))
}

#[pyfunction]
pub fn local_data_directory(profile: Option<PathBuf>) -> Option<PathBuf> {
    data_root(profile, false)
}

fn data_root(profile: Option<PathBuf>, _roaming: bool) -> Option<PathBuf> {
    profile.map(|p| p.join("data")).or_else(|| {
        #[cfg(target_os = "macos")]
        {
            env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"))
        }
        #[cfg(windows)]
        {
            if !_roaming {
                return env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .or_else(|| known_folder(&windows::Win32::UI::Shell::FOLDERID_LocalAppData));
            }
            env::var_os("APPDATA")
                .map(PathBuf::from)
                .or_else(|| known_folder(&windows::Win32::UI::Shell::FOLDERID_RoamingAppData))
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        }
    })
}

#[pyfunction]
pub fn cache_directory(profile: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(profile) = profile {
        return Some(profile.join("cache"));
    }
    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Caches"))
    }
    #[cfg(windows)]
    {
        data_root(None, false)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
    }
}
