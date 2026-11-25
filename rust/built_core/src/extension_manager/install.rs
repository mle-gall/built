// SPDX-License-Identifier: GPL-3.0-or-later
//! Validate and stage every member before publishing the new package. Retain
//! hand-installed copies and restore their original names on a failed update.
use super::*;
const MAX_DOWNLOAD: usize = 64 << 20;
const MAX_UNPACKED: u64 = 128 << 20;
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Moves {
    entries: Vec<(PathBuf, PathBuf)>,
    done: bool,
}
impl Moves {
    fn new() -> Self {
        Self {
            entries: vec![],
            done: false,
        }
    }
    fn move_aside(&mut self, src: &Path) -> PyResult<()> {
        let dst = files::aside(src)?;
        self.entries.push((src.to_owned(), dst));
        Ok(())
    }
}
impl Drop for Moves {
    fn drop(&mut self) {
        if !self.done {
            for (original, backup) in self.entries.iter().rev() {
                if original.symlink_metadata().is_err() {
                    if let Err(e) = fs::rename(backup, original) {
                        eprintln!(
                            "Cannot restore extension {} from {}: {e}",
                            original.display(),
                            backup.display()
                        )
                    }
                }
            }
        }
    }
}
fn safe_member(name: &str) -> bool {
    // Validate on every platform, including Windows drive names, streams,
    // reserved devices and trailing dots/spaces which alias another path.
    !name.is_empty()
        && !name.contains(['\\', ':', '\0'])
        && name.split('/').all(|p| {
            let stem = p.split('.').next().unwrap_or("").to_ascii_uppercase();
            !p.is_empty()
                && !p.ends_with(['.', ' '])
                && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !["COM", "LPT"].iter().any(|prefix| {
                    stem.strip_prefix(prefix)
                        .is_some_and(|n| n.len() == 1 && "123456789".contains(n))
                })
        })
}
fn extract(zip: &Obj, stage: &Path, id: &str) -> PyResult<()> {
    let mut members = vec![];
    let mut size = 0_u64;
    for info in zip.call_method0("infolist")?.try_iter()? {
        let info = info?;
        let filename = s(&info, "filename")?;
        if info.call_method0("is_dir")?.is_truthy()? || filename.starts_with("__MACOSX/") {
            continue;
        }
        if members.len() == 2000 {
            return Err(err("The extension archive has too many files."));
        }
        let mode = info.getattr("external_attr")?.extract::<u32>()? >> 16;
        if !safe_member(&filename) || mode & 0o170000 == 0o120000 {
            return Err(err("The extension archive contains an unsafe path."));
        }
        size = size.saturating_add(info.getattr("file_size")?.extract::<u64>()?);
        if size > MAX_UNPACKED {
            return Err(err("The unpacked extension is too large."));
        }
        members.push((info, PathBuf::from(filename)));
    }
    let mut packages = members
        .iter()
        .map(|(_, p)| p)
        .filter(|p| name(p) == "__init__.py")
        .filter_map(|p| p.parent())
        .collect::<BTreeSet<_>>();
    let (base, loose) = if let Some(depth) = packages.iter().map(|p| p.components().count()).min() {
        packages.retain(|p| p.components().count() == depth);
        if packages.len() > 1 {
            packages.retain(|p| name(p) == id);
        }
        if packages.len() != 1 {
            return Err(err("The ZIP contains multiple top-level plugin packages."));
        }
        (*packages.first().unwrap(), false)
    } else {
        let target = format!("{id}.py");
        let mut mains = members.iter().filter(|(_, p)| name(p) == target);
        let main = mains.next();
        if main.is_none() || mains.next().is_some() {
            return Err(err(format!(
                "The ZIP must contain one plugin package or a Python file named {id}.py."
            )));
        }
        (main.unwrap().1.parent().unwrap(), true)
    };
    let base = base.to_owned();
    for (info, p) in members {
        let Ok(rel) = p.strip_prefix(&base) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dst = stage.join(rel);
        fs::create_dir_all(dst.parent().unwrap())?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dst)?;
        let bytes = zip.call_method1("read", (info,))?.cast_into::<PyBytes>()?;
        std::io::Write::write_all(&mut f, bytes.as_bytes())?;
    }
    if loose {
        fs::rename(stage.join(format!("{id}.py")), stage.join("__init__.py"))?
    }
    Ok(())
}
fn payload(py: Python, ext: &Obj, stage: &Path, id: &str) -> PyResult<()> {
    let bundled = ext.getattr("bundled")?.extract::<Option<PathBuf>>()?;
    let download = s(ext, "download")?;
    let data = if let Some(local) = bundled {
        if !local.exists() {
            return Err(err(format!("'{id}' is not shipped with this Built.")));
        }
        if local.is_dir() {
            for f in files::descendants(&local) {
                if f.extension().is_some_and(|s| s == "pyc") {
                    continue;
                }
                let dst = stage.join(f.strip_prefix(&local).unwrap());
                fs::create_dir_all(dst.parent().unwrap())?;
                fs::copy(f, dst)?;
            }
            return Ok(());
        }
        PyBytes::new(py, &fs::read(local)?)
    } else {
        let data = module(py)?
            .call_method1("download", (&download, MAX_DOWNLOAD))?
            .cast_into::<PyBytes>()?;
        if digest(py, data.as_bytes())? != s(ext, "sha256")? {
            return Err(err(format!(
                "Checksum mismatch for '{id}'; nothing was installed."
            )));
        }
        data
    };
    let stream = py.import("io")?.call_method1("BytesIO", (&data,))?;
    let zip = py.import("zipfile")?;
    if zip.call_method1("is_zipfile", (&stream,))?.is_truthy()? {
        let archive = zip.call_method1("ZipFile", (stream,))?;
        let result = extract(&archive, stage, id);
        archive.call_method0("close")?;
        return result;
    }
    if download
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .ends_with(".zip")
    {
        return Err(err(format!("'{id}' did not download a valid ZIP package.")));
    }
    if data.as_bytes().is_empty() || data.as_bytes().contains(&0) {
        return Err(err(format!("'{id}' is not a Python extension file.")));
    }
    fs::write(stage.join("__init__.py"), data.as_bytes())?;
    Ok(())
}
fn perform(
    extension: &Obj,
    confirm: Option<&Obj>,
    catalog: Option<&Obj>,
    moves: &mut Moves,
) -> PyResult<()> {
    let py = extension.py();
    let id = s(extension, "id")?;
    require_id(&id)?;
    if !catalog::compatible_with_current(extension)? {
        return Err(err(format!(
            "'{id}' requires Built {} or later.",
            s(extension, "minimum_version")?
        )));
    }
    let (dst, managed) = files::target(py, &id)?;
    let conflict = || {
        err(format!(
            "A plugin named '{id}' already exists and is not managed by the extension manager."
        ))
    };
    if !managed {
        return Err(conflict());
    }
    let dir = dst.parent().unwrap();
    fs::create_dir_all(dir)?;
    let stage = Staging(dir.join(format!(".{id}-{}", crate::builtproject::manifest::new_id())));
    fs::create_dir(&stage.0)?;
    payload(py, extension, &stage.0, &id)?;
    let copies = files::copies(py, &id, &stage.0, catalog)?;
    if !copies.is_empty() {
        let accept = match confirm {
            None => false,
            Some(cb) => cb.call1((paths(py, copies.clone())?,))?.is_truthy()?,
        };
        if !accept {
            return Err(err(format!(
                "'{id}' is already installed by hand in {}; nothing was installed.",
                copies
                    .iter()
                    .map(|p| name(p))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        for p in copies {
            moves.move_aside(&p)?;
        }
    }
    let metadata = serde_json::json!({"id":id,"version":s(extension,"version")?,"sha256":s(extension,"sha256")?,"download":s(extension,"download")?});
    fs::write(
        stage.0.join("extension.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )?;
    if dst.join(".disabled").is_file() {
        fs::write(stage.0.join(".disabled"), "")?
    }
    if files::target(py, &id)? != (dst.clone(), true) {
        return Err(conflict());
    }
    let backup = dir.join(format!(
        ".{id}-backup-{}",
        crate::builtproject::manifest::new_id()
    ));
    if dst.exists() {
        fs::rename(&dst, &backup)?;
        moves.entries.push((dst.clone(), backup.clone()));
    }
    fs::rename(&stage.0, &dst)?;
    moves.done = true;
    if backup.exists() {
        if let Err(e) = fs::remove_dir_all(&backup) {
            catalog::warn(
                py,
                &format!(
                    "could not remove replaced extension backup {}: {e}",
                    backup.display()
                ),
            )?
        }
    }
    Ok(())
}
#[pyfunction(signature=(extension,confirm_manual_copies=None,catalog_ids=None))]
fn install_extension(
    extension: &Obj,
    confirm_manual_copies: Option<&Obj>,
    catalog_ids: Option<&Obj>,
) -> PyResult<()> {
    let _guard = edit_lock(extension.py())?;
    perform(
        extension,
        confirm_manual_copies,
        catalog_ids,
        &mut Moves::new(),
    )
}
#[pyfunction(signature=(extension,confirm_manual_copies=None,catalog_ids=None))]
fn replace_hand_installed<'p>(
    extension: &Obj<'p>,
    confirm_manual_copies: Option<&Obj>,
    catalog_ids: Option<&Obj>,
) -> PyResult<Vec<Obj<'p>>> {
    let _guard = edit_lock(extension.py())?;
    let py = extension.py();
    let id = s(extension, "id")?;
    require_id(&id)?;
    let mut moves = Moves::new();
    for p in files::hand(&root(py)?) {
        if files::ident(&p) == id {
            moves.move_aside(&p)?
        }
    }
    let own = moves.entries.iter().map(|(_, p)| p.clone()).collect();
    perform(extension, confirm_manual_copies, catalog_ids, &mut moves)?;
    paths(py, own)
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(install_extension, m)?)?;
    m.add_function(wrap_pyfunction!(replace_hand_installed, m)?)
}
