// SPDX-License-Identifier: GPL-3.0-or-later
//! `built_core.texture`, served from here (`core/texture.py` is gone): image
//! textures mapped the .skp way for interchange compatibility.
//!
//! A textured face carries `attrs["texture"] = {"path", "sw", "sh"}`, a real
//! file on disk. Images that came in inside something (an imported .skp,
//! an .igz container) live in the app's texture cache, never next to the
//! user's files: content-addressed, so one image shared by several
//! documents is written once. A tinted material bakes its tint into a
//! cached copy and remembers the untinted source under `"base"`, so
//! re-tinting always starts from the source and removing the colour gives
//! it back exactly. The projection maths are `texture_maths.rs`.
//! `tests/test_texture_module_golden.py` pins the module's answers.

use std::path::PathBuf;

use pyo3::exceptions::PyOSError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};

use super::module;

type Obj<'py> = Bound<'py, PyAny>;

pub const NAME_LIMIT: usize = 64;
pub const COLORIZE_SHIFT: i64 = 0;
pub const COLORIZE_TINT: i64 = 1;
/// |Z × n| below which the .skp projection maps a face with the world axes.
pub const VERTICAL_TOLERANCE: f64 = 1e-3;

const IMAGE_MAGIC: [&[u8]; 7] = [b"\x89PNG\r\n\x1a\n", b"\xff\xd8\xff", b"GIF87a", b"GIF89a", b"BM", b"II*\x00", b"MM\x00*"];

fn path<'py>(py: Python<'py>, p: impl IntoPyObject<'py>) -> PyResult<Obj<'py>> {
    module(py, "pathlib")?.call_method1("Path", (p,))
}

fn env<'py>(py: Python<'py>, key: &str) -> PyResult<Option<String>> {
    let v = module(py, "os")?.getattr("environ")?.call_method1("get", (key,))?;
    Ok(v.extract::<Option<String>>()?.filter(|s| !s.is_empty()))
}

/// The platform's per-user data folder, as the native shell names it when
/// it is not loaded (scripts, tests).
#[pyfunction]
#[pyo3(name = "_data_root")]
fn data_root(py: Python<'_>) -> PyResult<Obj<'_>> {
    if let Some(profile) = env(py, "BUILT_SETTINGS_DIR")? {
        return path(py, profile)?.call_method1("__truediv__", ("data",));
    }
    let home = module(py, "pathlib")?.getattr("Path")?.call_method0("home")?;
    let platform: String = module(py, "sys")?.getattr("platform")?.extract()?;
    if platform == "darwin" {
        return home.call_method1("__truediv__", ("Library",))?.call_method1("__truediv__", ("Application Support",));
    }
    let os_name: String = module(py, "os")?.getattr("name")?.extract()?;
    if os_name == "nt" {
        return match env(py, "LOCALAPPDATA")? {
            Some(d) => path(py, d),
            None => path(py, home.call_method1("__truediv__", ("AppData",))?.call_method1("__truediv__", ("Local",))?),
        };
    }
    match env(py, "XDG_DATA_HOME")? {
        Some(x) if std::path::Path::new(&x).is_absolute() => path(py, x),
        _ => home.call_method1("__truediv__", (".local",))?.call_method1("__truediv__", ("share",)),
    }
}

/// Root of the app's texture cache: `<user data>/Built/textures`
/// (`$BUILT_TEXTURE_CACHE` overrides it). `skp/` holds images
/// extracted from an imported .skp, `embedded/` those unpacked from an .igz.
#[pyfunction]
pub fn texture_cache_root(py: Python<'_>) -> PyResult<Obj<'_>> {
    if let Some(over) = env(py, "BUILT_TEXTURE_CACHE")? {
        return path(py, over);
    }
    let shell = py.import("built_render.shell_services")?;
    let base = if shell.call_method0("available")?.is_truthy()? {
        let settings = module(py, "os")?.getattr("environ")?.call_method1("get", ("BUILT_SETTINGS_DIR",))?;
        path(py, py.import("built_render")?.call_method1("local_data_directory", (settings,))?)?
    } else {
        data_root(py)?
    };
    base.call_method1("__truediv__", ("Built",))?.call_method1("__truediv__", ("textures",))
}

/// Every regular file under `dir`, symbolic links to folders not followed.
fn files_under(dir: &std::path::Path, out: &mut Vec<u64>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            files_under(&p, out);
        } else if let Ok(meta) = std::fs::metadata(&p) {
            if meta.is_file() {
                out.push(meta.len());
            }
        }
    }
}

/// The cache root through the module, as the Python read its global:
/// scripts and tests point it elsewhere by replacing
/// `built_core.texture.texture_cache_root`.
fn root_path(py: Python<'_>) -> PyResult<PathBuf> {
    let root = module(py, "built_core.texture")?.call_method0("texture_cache_root")?;
    Ok(PathBuf::from(root.str()?.to_string()))
}

/// `(file count, total bytes)` currently held in the texture cache.
#[pyfunction]
pub fn texture_cache_stats(py: Python<'_>) -> PyResult<(usize, u64)> {
    let root = root_path(py)?;
    let mut sizes = Vec::new();
    if root.is_dir() {
        files_under(&root, &mut sizes);
    }
    Ok((sizes.len(), sizes.iter().sum()))
}

/// Delete the whole texture cache; the number of files removed. Documents
/// saved as .igz containers re-extract their images when opened again.
#[pyfunction]
pub fn clear_texture_cache(py: Python<'_>) -> PyResult<usize> {
    let root = root_path(py)?;
    let count = texture_cache_stats(py)?.0;
    if root.is_dir() {
        let _ = std::fs::remove_dir_all(&root);
    }
    Ok(count)
}

/// Whether the image carries real transparency (a photo sprite rather than
/// an opaque panel), cached per path; an unreadable image reads as opaque.
#[pyfunction]
#[pyo3(signature = (path, cache=None))]
pub fn image_has_cutout<'py>(path: &Obj<'py>, cache: Option<&Bound<'py, PyDict>>) -> PyResult<bool> {
    let py = path.py();
    static CACHE: pyo3::sync::PyOnceLock<Py<PyDict>> = pyo3::sync::PyOnceLock::new();
    let cache = match cache {
        Some(c) => c.clone(),
        None => CACHE.get_or_init(py, || PyDict::new(py).unbind()).bind(py).clone(),
    };
    if let Some(hit) = cache.get_item(path)? {
        if !hit.is_none() {
            return hit.extract();
        }
    }
    let ok: bool = py.import("built_render")?.call_method1("texture_cutout", (path.str()?,))?.extract()?;
    cache.set_item(path, ok)?;
    Ok(ok)
}

/// `pathlib.PurePosixPath(name).name`.
fn last_name(name: &str) -> &str {
    name.split('/').filter(|c| !c.is_empty() && *c != ".").next_back().unwrap_or("")
}

fn trim_dots(s: &str) -> &str {
    s.trim_matches(|c| c == ' ' || c == '.')
}

/// Filesystem-safe base name for an image called `name`: unsafe characters
/// dropped, any content-hash prefix stripped (old saves stacked them one
/// level deeper each time), clipped to `NAME_LIMIT` keeping a short
/// extension.
#[pyfunction]
pub fn texture_file_name(name: &str) -> String {
    let kept: String = last_name(name).chars().filter(|c| c.is_alphanumeric() || " ._-".contains(*c)).collect();
    let mut safe = trim_dots(&kept).to_string();
    // ^(?:[0-9a-f]{16}-)+
    loop {
        let head: Vec<char> = safe.chars().take(17).collect();
        if head.len() == 17 && head[..16].iter().all(|c| c.is_ascii_digit() || ('a'..='f').contains(c)) && head[16] == '-' {
            safe = safe.chars().skip(17).collect();
        } else {
            break;
        }
    }
    safe = trim_dots(&safe).to_string();
    let n = safe.chars().count();
    if n > NAME_LIMIT {
        let (stem, ext) = match safe.rfind('.') {
            Some(i) => (&safe[..i], Some(&safe[i + 1..])),
            None => (safe.as_str(), None),
        };
        let ext_ok = ext.is_some_and(|e| {
            let len = e.chars().count();
            len > 0 && len <= 5 && e.chars().all(char::is_alphanumeric)
        });
        safe = if ext_ok {
            let e = ext.unwrap();
            let keep = NAME_LIMIT - e.chars().count() - 1;
            let s: String = stem.chars().take(keep).collect();
            format!("{}.{}", s.trim_end_matches(|c| c == ' ' || c == '.'), e)
        } else {
            let s: String = safe.chars().take(NAME_LIMIT).collect();
            s.trim_end_matches(|c| c == ' ' || c == '.').to_string()
        };
    }
    if safe.is_empty() { "texture.png".into() } else { safe }
}

fn image_head(head: &[u8]) -> bool {
    let head = &head[..head.len().min(16)];
    IMAGE_MAGIC.iter().any(|m| head.starts_with(m)) || (head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP")
}

/// True when `data` starts like an image file (PNG, JPEG, GIF, BMP, TIFF,
/// WebP).
#[pyfunction]
pub fn is_image_bytes(data: &Obj<'_>) -> PyResult<bool> {
    let py = data.py();
    let head = data.get_item(pyo3::types::PySlice::new(py, 0, 16, 1))?;
    let bytes: Vec<u8> = module(py, "builtins")?.getattr("bytes")?.call1((head,))?.extract()?;
    Ok(image_head(&bytes))
}

/// True when `path` is a readable image: a document names its texture
/// files by path and saving copies them out, so a document pointing at a
/// key file must not get it embedded.
#[pyfunction]
pub fn is_image_file(path: &Obj<'_>) -> PyResult<bool> {
    let py = path.py();
    let p: String = module(py, "os")?.call_method1("fspath", (path,))?.extract()?;
    let mut head = [0u8; 16];
    let n = match std::fs::File::open(&p) {
        Ok(mut f) => {
            use std::io::Read;
            let mut got = 0;
            loop {
                match f.read(&mut head[got..]) {
                    Ok(0) => break,
                    Ok(k) => {
                        got += k;
                        if got == 16 {
                            break;
                        }
                    }
                    Err(_) => return Ok(false),
                }
            }
            got
        }
        Err(_) => return Ok(false),
    };
    if image_head(&head[..n]) {
        return Ok(true);
    }
    if p.to_lowercase().ends_with(".tga") {
        return py.import("built_render")?.call_method1("readable_tga", (self::path(py, &p)?,))?.extract();
    }
    Ok(false)
}

/// Store `data` in the texture cache under `subdir`; its path. The file
/// name carries a hash of the bytes, so the same image is written once.
#[pyfunction]
pub fn cache_image<'py>(data: &Bound<'py, PyBytes>, name: &str, subdir: &str) -> PyResult<Obj<'py>> {
    let py = data.py();
    let bytes = data.as_bytes();
    let digest = super::links::sha1_hex(bytes);
    let safe = texture_file_name(name);
    let out = root_path(py)?.join(subdir).join(format!("{}-{safe}", &digest[..16]));
    let write = || -> std::io::Result<()> {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let same = std::fs::metadata(&out).is_ok_and(|m| m.len() == bytes.len() as u64);
        if !same {
            std::fs::write(&out, bytes)?;
        }
        Ok(())
    };
    match write() {
        Ok(()) => path(py, out),
        Err(_) => {
            let kw = PyDict::new(py);
            kw.set_item("prefix", "built-tex-")?;
            let tmp: String = module(py, "tempfile")?.call_method("mkdtemp", (), Some(&kw))?.extract()?;
            let file = PathBuf::from(tmp).join(&safe);
            std::fs::write(&file, bytes).map_err(|e| PyOSError::new_err(e.to_string()))?;
            path(py, file)
        }
    }
}

/// Re-tint a shared texture as the .skp format defines a colourized
/// material copy: `ctype` 0 ("shift") moves every pixel by the delta
/// between the image average and the colour, 1 ("tint") replaces hue and
/// saturation keeping the per-pixel lightness; alpha kept. PNG bytes, or
/// `data` unchanged on any failure.
#[pyfunction]
#[pyo3(signature = (data, target_rgb, ctype=None))]
pub fn colorize_image<'py>(data: &Obj<'py>, target_rgb: &Obj<'py>, ctype: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let py = data.py();
    let run = || -> PyResult<Option<Obj<'py>>> {
        let raster = py.import("built_render")?.getattr("SurveyRaster")?;
        let raw = module(py, "builtins")?.getattr("bytes")?.call1((data,))?;
        let img = raster.call_method1("decode", (&raw,))?;
        if img.is_none() {
            return Ok(None);
        }
        let rgba = img.call_method0("rgba")?;
        let native = module(py, "built_core")?;
        let mean = native.call_method1("texture_mean_visible", (&rgba,))?;
        let target = PyTuple::new(py, target_rgb.get_item(pyo3::types::PySlice::new(py, 0, 3, 1))?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        let mode: i64 = match ctype {
            Some(c) => module(py, "builtins")?.getattr("int")?.call1((c,))?.extract()?,
            None => COLORIZE_SHIFT,
        };
        let out = native.call_method1("texture_colorize_pixels", (&rgba, mean, target, mode))?;
        let made = raster.call1((img.getattr("width")?, img.getattr("height")?, out))?;
        Ok(Some(made.call_method1("material_png", (raw,))?))
    };
    match run() {
        Ok(Some(png)) => Ok(png),
        Ok(None) => Ok(data.clone()),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => Ok(data.clone()),
        Err(e) => Err(e),
    }
}

fn copy<'py>(d: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(d.py());
    out.call_method1("update", (d,))?;
    Ok(out)
}

/// A copy of the texture entry `tex` re-tinted toward `color` (RGB 0–1):
/// the tint baked into a cached image `"path"` names, the untinted source
/// kept as `"base"`, the recipe as `"tint"` and `"tint_mode"`. `tex`
/// unchanged (a copy) if the source cannot be read or tinted.
#[pyfunction]
#[pyo3(signature = (tex, color, ctype=COLORIZE_SHIFT))]
pub fn tinted_texture<'py>(tex: &Obj<'py>, color: &Obj<'py>, ctype: i64) -> PyResult<Bound<'py, PyDict>> {
    let py = tex.py();
    let mut base = tex.call_method1("get", ("base",))?;
    if !base.is_truthy()? {
        base = tex.call_method1("get", ("path",))?;
    }
    if !base.is_truthy()? {
        return copy(tex);
    }
    let base_path = path(py, &base)?;
    let data = match base_path.call_method0("read_bytes") {
        Ok(d) => d,
        Err(e) if e.is_instance_of::<PyOSError>(py) => return copy(tex),
        Err(e) => return Err(e),
    };
    let comps: Vec<f64> = PyTuple::new(py, color.try_iter()?.take(3).collect::<PyResult<Vec<_>>>()?)?.extract()?;
    let rgb255: Vec<i64> = comps.iter().map(|c| (c.clamp(0.0, 1.0) * 255.0).round_ties_even() as i64).collect();
    let me = module(py, "built_core.texture")?;
    let out = me.call_method1("colorize_image", (&data, PyTuple::new(py, &rgb255)?, ctype))?;
    if out.is(&data) {
        return copy(tex); // colorize failed: leave it alone
    }
    let name: String = base_path.getattr("name")?.extract()?;
    let cached = match me.call_method1("cache_image", (&out, &name, "tinted")) {
        Ok(p) => p,
        Err(e) if e.is_instance_of::<PyOSError>(py) => return copy(tex),
        Err(e) => return Err(e),
    };
    let entry = copy(tex)?;
    entry.set_item("path", cached.str()?)?;
    entry.set_item("base", base.str()?)?;
    entry.set_item("tint", PyList::new(py, comps)?)?;
    entry.set_item("tint_mode", ctype)?;
    Ok(entry)
}

/// The entry with its colour removed: back to the original image.
#[pyfunction]
pub fn untinted_texture<'py>(tex: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = tex.py();
    let base = tex.call_method1("get", ("base",))?;
    let entry = PyDict::new(py);
    for item in tex.call_method0("items")?.try_iter()? {
        let (k, v): (Obj<'py>, Obj<'py>) = item?.extract()?;
        if !(k.eq("base")? || k.eq("tint")? || k.eq("tint_mode")?) {
            entry.set_item(k, v)?;
        }
    }
    if base.is_truthy()? {
        entry.set_item("path", base)?;
    }
    Ok(entry)
}

#[pyfunction]
fn texture_as_dict<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(me.py());
    for k in ["path", "sw", "sh"] {
        d.set_item(k, me.getattr(k)?)?;
    }
    Ok(d)
}

#[pyfunction]
fn texture_from_dict<'py>(d: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = d.py();
    let float = module(py, "builtins")?.getattr("float")?;
    let cls = module(py, "built_core.texture")?.getattr("Texture")?;
    let sw = float.call1((d.call_method1("get", ("sw", 1.0))?,))?;
    let sh = float.call1((d.call_method1("get", ("sh", 1.0))?,))?;
    cls.call1((d.get_item("path")?, sw, sh))
}

/// Fill the module `built_core.texture`.
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__doc__", "Image textures, mapped the .skp way for interchange compatibility (rust/built_core/src/model/texture_api.rs).")?;
    m.add_function(wrap_pyfunction!(data_root, m)?)?;
    m.add_function(wrap_pyfunction!(texture_cache_root, m)?)?;
    m.add_function(wrap_pyfunction!(texture_cache_stats, m)?)?;
    m.add_function(wrap_pyfunction!(clear_texture_cache, m)?)?;
    m.add_function(wrap_pyfunction!(image_has_cutout, m)?)?;
    m.add_function(wrap_pyfunction!(texture_file_name, m)?)?;
    m.add_function(wrap_pyfunction!(is_image_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(is_image_file, m)?)?;
    m.add_function(wrap_pyfunction!(cache_image, m)?)?;
    m.add_function(wrap_pyfunction!(colorize_image, m)?)?;
    m.add_function(wrap_pyfunction!(tinted_texture, m)?)?;
    m.add_function(wrap_pyfunction!(untinted_texture, m)?)?;
    m.add("NAME_LIMIT", NAME_LIMIT)?;
    m.add("COLORIZE_SHIFT", COLORIZE_SHIFT)?;
    m.add("COLORIZE_TINT", COLORIZE_TINT)?;
    m.add("VERTICAL_TOLERANCE", VERTICAL_TOLERANCE)?;
    let native = module(py, "built_core")?;
    m.add("projection_basis", native.getattr("texture_projection_basis")?)?;
    m.add("face_uv_axes", native.getattr("texture_face_uv_axes")?)?;
    // The Texture record: a dataclass, as it was.
    let dc = py.import("dataclasses")?;
    let field = dc.getattr("field")?;
    let def = |v: f64| -> PyResult<Obj<'_>> {
        let kw = PyDict::new(py);
        kw.set_item("default", v)?;
        field.call((), Some(&kw))
    };
    let builtins = py.import("builtins")?;
    let fields = PyList::new(py, [
        PyTuple::new(py, [pyo3::types::PyString::new(py, "path").into_any(), builtins.getattr("str")?])?.into_any(),
        PyTuple::new(py, [pyo3::types::PyString::new(py, "sw").into_any(), builtins.getattr("float")?, def(1.0)?])?.into_any(),
        PyTuple::new(py, [pyo3::types::PyString::new(py, "sh").into_any(), builtins.getattr("float")?, def(1.0)?])?.into_any(),
    ])?;
    let ns = PyDict::new(py);
    ns.set_item("as_dict", py.import("functools")?.call_method1("partialmethod", (wrap_pyfunction!(texture_as_dict, m)?,))?)?;
    ns.set_item("from_dict", builtins.getattr("staticmethod")?.call1((wrap_pyfunction!(texture_from_dict, m)?,))?)?;
    let kw = PyDict::new(py);
    kw.set_item("namespace", ns)?;
    kw.set_item("module", "built_core.texture")?;
    let cls = dc.call_method("make_dataclass", ("Texture", fields), Some(&kw))?;
    m.add("Texture", cls)?;
    Ok(())
}
