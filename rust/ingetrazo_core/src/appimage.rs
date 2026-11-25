// SPDX-License-Identifier: GPL-3.0-or-later
//! Install the AppImage launcher and icon in the user's XDG data directory.
use pyo3::prelude::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, io};

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

#[pyfunction]
fn appimage_path() -> Option<PathBuf> {
    env_path("APPIMAGE").filter(|p| p.is_file())
}

fn data_home() -> io::Result<PathBuf> {
    env_path("XDG_DATA_HOME")
        .or_else(|| std::env::home_dir().map(|p| p.join(".local/share")))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "cannot locate user's data directory",
            )
        })
}

fn desktop(home: &Path) -> PathBuf {
    home.join("applications/ingetrazo.desktop")
}
fn icon(home: &Path) -> PathBuf {
    home.join("icons/hicolor/256x256/apps/ingetrazo.png")
}

fn desktop_entry(image: &Path) -> String {
    let path = image.to_string_lossy().replace('"', "\\\"");
    format!(concat!("[Desktop Entry]\nType=Application\nName=Built\nGenericName=3D Modeler\n",
        "GenericName[es]=Modelador 3D\nComment=Free 3D modeler for civil engineering and architecture\n",
        "Comment[es]=Modelador 3D libre para ingeniería civil y arquitectura\nExec=\"{0}\" %f\n",
        "TryExec={0}\nIcon=ingetrazo\nTerminal=false\nCategories=Graphics;3DGraphics;Engineering;\n",
        "MimeType=application/x-ingetrazo;model/vnd.collada+xml;application/vnd.sketchup.skp;\n",
        "StartupWMClass=ingetrazo\nX-AppImage-Integrated=true\n"), path)
}

#[pyfunction]
#[pyo3(signature=(appimage=None))]
fn is_integrated(appimage: Option<PathBuf>) -> PyResult<bool> {
    let file = desktop(&data_home()?);
    Ok(match appimage {
        Some(p) => fs::read_to_string(file).is_ok_and(|s| s.contains(p.to_string_lossy().as_ref())),
        None => file.is_file(),
    })
}

fn refresh(home: &Path) {
    let mut launcher = Command::new("update-desktop-database");
    launcher.arg(home.join("applications"));
    let mut icons = Command::new("gtk-update-icon-cache");
    icons
        .args(["-q", "-t", "-f"])
        .arg(home.join("icons/hicolor"));
    for mut cmd in [launcher, icons] {
        let Ok(mut child) = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
            continue;
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }
}

#[pyfunction]
#[pyo3(signature=(appimage, icon_source=None))]
fn integrate(py: Python<'_>, appimage: PathBuf, icon_source: Option<PathBuf>) -> PyResult<PathBuf> {
    let home = data_home()?;
    let file = desktop(&home);
    fs::create_dir_all(home.join("applications"))?;
    fs::write(&file, desktop_entry(&appimage))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&file, fs::Permissions::from_mode(0o755));
    }
    let source = icon_source.or_else(|| env_path("APPDIR").map(|p| p.join("ingetrazo.png")));
    if let Some(source) = source.filter(|p| p.is_file()) {
        let dst = icon(&home);
        fs::create_dir_all(dst.parent().unwrap())?;
        fs::copy(source, dst)?;
    }
    py.detach(|| refresh(&home));
    Ok(file)
}

#[pyfunction]
fn remove(py: Python<'_>) -> PyResult<()> {
    let home = data_home()?;
    for target in [desktop(&home), icon(&home)] {
        let _ = fs::remove_file(target);
    }
    py.detach(|| refresh(&home));
    Ok(())
}

pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let m = PyModule::new(parent.py(), "ingetrazo_core.appimage")?;
    m.add_function(wrap_pyfunction!(appimage_path, &m)?)?;
    m.add_function(wrap_pyfunction!(is_integrated, &m)?)?;
    m.add_function(wrap_pyfunction!(integrate, &m)?)?;
    m.add_function(wrap_pyfunction!(remove, &m)?)?;
    parent.add("appimage", m)
}
