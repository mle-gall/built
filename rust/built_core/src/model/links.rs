// SPDX-License-Identifier: GPL-3.0-or-later
//! Linked files (`core/links.py`, `docs/model-port.md` Round 9): another
//! document placed in this one as a live reference.
//!
//! What a link is lives in the Rust document: each placed copy's link
//! record is its `ext["link"]` attribute, read here from the placement's
//! node without building Python containers; the host's links registry (per
//! source: tag overrides, packed, the tags and folders it brought) is the
//! root table `link_registry`, no longer an entry of `plugin_data` (the
//! file still stores it there, `file_scene.rs`).
//!
//! A source's definition is renamed as it is read: the placements' tags and
//! a still-deferred mesh's tables (face attribute rows, edge tag names) are
//! rewritten to the host's names for the source's tags and materials before
//! anything is built, so a big linked model is never built to be linked.
//! The tags the source brings are mirrored in the host (`sync_tags`), and
//! given back to the host when its last placement is unlinked
//! (`release_tags`, `relabel`). The copies of a source's tree on a load, a
//! reload, a copy of a link and Unlink are made here too.
//!
//! Every step reads and writes the scene's tables, tags and placements in
//! the Python's order, so a journalled edit records the same writes
//! (`tests/test_links_golden.py`).

use std::collections::{HashMap, HashSet};

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyString, PyTuple};

use super::module;
use super::tag_edits::{self, DEFAULT_LAYER};
use crate::doc::{self, py::handle_key, value::Value, Data};

type Obj<'py> = Bound<'py, PyAny>;

pub const LINK_KEY: &str = "link";
pub const LINKED_FOLDER: &str = "Linked files";
pub const SEP: &str = " / ";

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn get<'py>(d: &Obj<'py>, key: impl IntoPyObject<'py>) -> PyResult<Obj<'py>> {
    let py = d.py();
    if d.is_exact_instance_of::<PyDict>() {
        // A plain dict: its own lookup, no method call.
        let d = d.cast::<PyDict>()?;
        return Ok(d.get_item(key)?.unwrap_or_else(|| none(py)));
    }
    d.call_method1(pyo3::intern!(py, "get"), (key,))
}

fn get_or<'py>(d: &Obj<'py>, key: impl IntoPyObject<'py>, default: impl IntoPyObject<'py>) -> PyResult<Obj<'py>> {
    let py = d.py();
    d.call_method1(pyo3::intern!(py, "get"), (key, default))
}

fn is_dict(v: &Obj<'_>) -> bool {
    v.is_instance_of::<PyDict>()
}

/// A module attribute looked up once per process.
fn cached<'py>(py: Python<'py>, cell: &'static pyo3::sync::PyOnceLock<Py<PyAny>>, m: &str, name: &str) -> PyResult<Obj<'py>> {
    Ok(cell.get_or_try_init(py, || module(py, m)?.getattr(name).map(Bound::unbind))?.bind(py).clone())
}

fn deepcopy<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    static F: pyo3::sync::PyOnceLock<Py<PyAny>> = pyo3::sync::PyOnceLock::new();
    cached(v.py(), &F, "copy", "deepcopy")?.call1((v,))
}

fn matrix<'py>(py: Python<'py>, m: &Obj<'py>) -> PyResult<Obj<'py>> {
    if m.is_none() {
        return Ok(none(py));
    }
    static F: pyo3::sync::PyOnceLock<Py<PyAny>> = pyo3::sync::PyOnceLock::new();
    cached(py, &F, "built_core", "Matrix4")?.call1((m,))
}

fn str_of(v: &Obj<'_>) -> Option<String> {
    v.cast::<PyString>().ok().and_then(|s| s.to_str().ok().map(str::to_string))
}

// ---- paths and names ------------------------------------------------------------

/// `os.fspath(path)` as a string (None for a bytes path: the Python then).
fn fspath(path: &Obj<'_>) -> PyResult<Option<String>> {
    let py = path.py();
    if let Some(s) = str_of(path) {
        return Ok(Some(s));
    }
    let p = module(path.py(), "os")?.call_method1(pyo3::intern!(py, "fspath"), (path,))?;
    Ok(str_of(&p))
}

/// `posixpath.normpath`.
fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let initial = if p.starts_with("//") && !p.starts_with("///") {
        2
    } else if p.starts_with('/') {
        1
    } else {
        0
    };
    let mut out: Vec<&str> = Vec::new();
    for comp in p.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial == 0 && out.is_empty()) || out.last() == Some(&"..") {
            out.push(comp);
        } else if !out.is_empty() {
            out.pop();
        }
    }
    let s = "/".repeat(initial) + &out.join("/");
    if s.is_empty() { ".".into() } else { s }
}

/// `os.path.normcase(os.path.abspath(os.fspath(path)))`: what a source is
/// known by in the registry.
pub fn norm(path: &Obj<'_>) -> PyResult<String> {
    let py = path.py();
    let s = match fspath(path)? {
        Some(s) if !cfg!(windows) => s,
        _ => {
            let os = module(py, "os")?.getattr(pyo3::intern!(py, "path"))?;
            let abs = os.call_method1(pyo3::intern!(py, "abspath"), (module(py, "os")?.call_method1(pyo3::intern!(py, "fspath"), (path,))?,))?;
            return os.call_method1(pyo3::intern!(py, "normcase"), (abs,))?.str()?.extract();
        }
    };
    if s.starts_with('/') {
        return Ok(normpath(&s));
    }
    let cwd: String = module(py, "os")?.call_method0(pyo3::intern!(py, "getcwd"))?.extract()?;
    let joined = if cwd.ends_with('/') { format!("{cwd}{s}") } else { format!("{cwd}/{s}") };
    Ok(normpath(&joined))
}

/// `pathlib.Path(path).stem` (Python 3.13).
fn stem_str(p: &str) -> String {
    let name = p.split('/').filter(|c| !c.is_empty() && *c != ".").next_back().unwrap_or("");
    if let Some(i) = name.rfind('.') {
        let s = &name[..i];
        if !s.trim_start_matches('.').is_empty() {
            return s.to_string();
        }
    }
    name.to_string()
}

pub fn stem(path: &Obj<'_>) -> PyResult<String> {
    let py = path.py();
    match fspath(path)? {
        Some(s) if !cfg!(windows) => Ok(stem_str(&s)),
        _ => module(path.py(), "pathlib")?.call_method1(pyo3::intern!(py, "Path"), (path,))?.getattr(pyo3::intern!(py, "stem"))?.extract(),
    }
}

fn tag_label(stem: &str, tag: &Obj<'_>) -> PyResult<String> {
    if tag.is_none() {
        return Ok(format!("{stem}{SEP}Untagged"));
    }
    let t: String = tag.extract().map_err(|_| PyTypeError::new_err("can only concatenate str (not \"{}\") to str"))?;
    Ok(if t.is_empty() || t == DEFAULT_LAYER { format!("{stem}{SEP}Untagged") } else { format!("{stem}{SEP}{t}") })
}

/// `tag_name(path, tag)`: the host's name for the source's tag `tag`.
#[pyfunction]
#[pyo3(name = "link_tag_name")]
pub fn tag_name(path: &Obj<'_>, tag: &Obj<'_>) -> PyResult<String> {
    tag_label(&stem(path)?, tag)
}

#[pyfunction]
#[pyo3(name = "link_stem")]
fn py_stem(path: &Obj<'_>) -> PyResult<String> {
    stem(path)
}

#[pyfunction]
#[pyo3(name = "link_norm")]
fn py_norm(path: &Obj<'_>) -> PyResult<String> {
    norm(path)
}

// ---- stable uids --------------------------------------------------------------------

/// Lower-case hex SHA-1 of `data` (`hashlib.sha1(data).hexdigest()`).
pub(crate) fn sha1_hex(data: &[u8]) -> String {
    sha1(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// uuid.NAMESPACE_URL's bytes, then the name's prefix: what uuid5 hashes.
const UID5_PREFIX: &[u8] = b"\x6b\xa7\xb8\x11\x9d\xad\x11\xd1\x80\xb4\x00\xc0\x4f\xd4\x30\xc8ingetrazo:link:v1:";

/// `uuid.uuid5(NAMESPACE_URL, 'ingetrazo:link:v1:' + placement_uid + ':'
/// + source_uid).hex[:16]`: the uid a linked part has in the host. These
/// ids are saved and named by scenes, so they may never change. The first
/// 8 bytes of the SHA-1, with the version nibble (13th hex digit) set to 5
/// as `uuid5` does; the variant bits fall outside these 16 digits.
pub fn stable_uid_of(placement_uid: &str, source_uid: &str) -> String {
    let mut data = UID5_PREFIX.to_vec();
    data.extend_from_slice(placement_uid.as_bytes());
    data.push(b':');
    data.extend_from_slice(source_uid.as_bytes());
    let digest = sha1(&data);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = [0u8; 16];
    for (i, b) in digest[..8].iter().enumerate() {
        out[2 * i] = HEX[(b >> 4) as usize];
        out[2 * i + 1] = HEX[(b & 15) as usize];
    }
    out[12] = b'5';
    String::from_utf8(out.to_vec()).expect("hex")
}

#[pyfunction]
#[pyo3(name = "link_stable_uid")]
pub fn stable_uid(placement_uid: &str, source_uid: &str) -> String {
    stable_uid_of(placement_uid, source_uid)
}

// ---- what is linked ------------------------------------------------------------------

/// Python's truth of a document value (None for one only Python can tell).
fn truthy(v: &Value) -> Option<bool> {
    Some(match v {
        Value::None => false,
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::List(l) | Value::Tuple(l) => !l.is_empty(),
        Value::Dict(d) => !d.is_empty(),
        Value::ElementKey(_) | Value::Vector(_) => true,
        Value::Opaque(_) => return None,
    })
}

fn field<'a>(d: &'a [(Value, Value)], key: &str) -> Option<&'a Value> {
    d.iter().find_map(|(k, v)| matches!(k, Value::Str(s) if s == key).then_some(v))
}

/// What the document says of `group`'s link, without Python: Some(None)
/// when it is not linked, Some(Some(path)) when it is (the path when it
/// is a string), None when only the Python attribute can tell (not a
/// document placement, or an unusual value). The document is current:
/// in-place edits write through, and readers call `doc::py::fresh` for
/// plain containers stored into tracked ones.
fn doc_link(group: &Obj<'_>) -> Option<Option<Option<String>>> {
    static EXT: std::sync::OnceLock<Option<u16>> = std::sync::OnceLock::new();
    let ext = (*EXT.get_or_init(|| doc::attr_id("ext").ok()))?;
    let k = handle_key(group).ok()?;
    let d = doc::lock();
    let node = d.node(k)?;
    if !matches!(node.data, Data::Placement(_)) {
        return None;
    }
    let Some(v) = node.attr(ext) else { return Some(None) };
    let Value::Dict(fields) = v else {
        return matches!(v, Value::None | Value::Str(_) | Value::List(_) | Value::Int(_)).then_some(None);
    };
    match field(fields, LINK_KEY) {
        None => Some(None),
        Some(Value::Dict(rec)) => match field(rec, "path") {
            None => Some(None),
            Some(p) => match truthy(p)? {
                false => Some(None),
                true => Some(Some(if let Value::Str(s) = p { Some(s.clone()) } else { None })),
            },
        },
        Some(_) => Some(None),
    }
}

/// `link_of(group)`: the link record (the tracked dict, editable in place)
/// of a placed linked file, or None.
pub fn link_of<'py>(group: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = group.py();
    crate::doc::py::fresh(py)?;
    if let Some(None) = doc_link(group) {
        return Ok(None);
    }
    let ext = match group.getattr_opt(pyo3::intern!(py, "ext"))? {
        Some(e) if is_dict(&e) => e,
        _ => return Ok(None),
    };
    let link = get(&ext, LINK_KEY)?;
    if is_dict(&link) && get(&link, "path")?.is_truthy()? {
        Ok(Some(link))
    } else {
        Ok(None)
    }
}

pub fn is_linked(group: &Obj<'_>) -> PyResult<bool> {
    crate::doc::py::fresh(group.py())?;
    match doc_link(group) {
        Some(found) => Ok(found.is_some()),
        None => Ok(link_of(group)?.is_some()),
    }
}

#[pyfunction]
#[pyo3(name = "link_of")]
fn py_link_of<'py>(group: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    link_of(group)
}

#[pyfunction]
#[pyo3(name = "link_is_linked")]
fn py_is_linked(group: &Obj<'_>) -> PyResult<bool> {
    is_linked(group)
}

/// `group` or the first group above it that is a linked copy.
pub fn linked_ancestor<'py>(group: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = group.py();
    let mut g = group.clone();
    while !g.is_none() {
        if is_linked(&g)? {
            return Ok(Some(g));
        }
        g = g.getattr_opt(pyo3::intern!(py, "owner"))?.unwrap_or_else(|| none(g.py()));
    }
    Ok(None)
}

#[pyfunction]
#[pyo3(name = "link_linked_ancestor")]
fn py_linked_ancestor<'py>(group: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    linked_ancestor(group)
}

/// The path a linked copy's record names, normalised, read from the
/// document (`doc::py::fresh` first: a plain record stored and edited in
/// place is written back).
fn norm_path_of(group: &Obj<'_>) -> PyResult<Option<String>> {
    match doc_link(group) {
        Some(None) => return Ok(None),
        Some(Some(Some(p))) if !cfg!(windows) && p.starts_with('/') => return Ok(Some(normpath(&p))),
        _ => {}
    }
    match link_of(group)? {
        Some(rec) => Ok(Some(norm(&rec.get_item("path")?)?)),
        None => Ok(None),
    }
}

fn all_groups<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let py = scene.py();
    crate::doc::py::fresh(py)?;
    let groups = match scene.getattr_opt(pyo3::intern!(py, "groups"))? {
        Some(g) if g.is_truthy()? => g,
        _ => return Ok(Vec::new()),
    };
    module(py, "built_core")?.call_method1(pyo3::intern!(py, "purge_iter_groups"), (groups,))?.try_iter()?.collect()
}

/// Every linked copy in the document (at any depth), or only the copies of
/// `path`.
pub fn linked_groups<'py>(scene: &Obj<'py>, path: Option<&Obj<'py>>) -> PyResult<Vec<Obj<'py>>> {
    let want = path.filter(|p| !p.is_none()).map(norm).transpose()?;
    let mut out = Vec::new();
    for g in all_groups(scene)? {
        if let Some(p) = norm_path_of(&g)? {
            if want.as_ref().is_none_or(|w| *w == p) {
                out.push(g);
            }
        }
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(name = "link_linked_groups", signature = (scene, path=None))]
fn py_linked_groups<'py>(scene: &Obj<'py>, path: Option<&Obj<'py>>) -> PyResult<Vec<Obj<'py>>> {
    linked_groups(scene, path)
}

/// The linked source files, each once, in first-placed order.
pub fn sources(scene: &Obj<'_>) -> PyResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for g in all_groups(scene)? {
        if let Some(p) = norm_path_of(&g)? {
            if seen.insert(p.clone()) {
                out.push(p);
            }
        }
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(name = "link_sources")]
fn py_sources(scene: &Obj<'_>) -> PyResult<Vec<String>> {
    sources(scene)
}

// ---- the registry ---------------------------------------------------------------------

/// The links registry to read: the document's root table, or an empty dict
/// that is not attached to it.
pub fn registry_view<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    match scene.getattr_opt(pyo3::intern!(py, "link_registry"))? {
        Some(r) if is_dict(&r) => Ok(r),
        _ => Ok(PyDict::new(scene.py()).into_any()),
    }
}

/// The links registry to write.
pub fn registry<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    match scene.getattr_opt(pyo3::intern!(py, "link_registry"))? {
        Some(r) if is_dict(&r) => Ok(r),
        _ => {
            scene.setattr(pyo3::intern!(py, "link_registry"), PyDict::new(scene.py()))?;
            scene.getattr(pyo3::intern!(py, "link_registry"))
        }
    }
}

#[pyfunction]
#[pyo3(name = "link_registry_view")]
fn py_registry_view<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    registry_view(scene)
}

/// The registry entry of `path`, made if absent, with its overrides.
pub fn entry<'py>(scene: &Obj<'py>, path: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let reg = registry(scene)?;
    let key = norm(path)?;
    let mut e = get(&reg, &key)?;
    if !is_dict(&e) {
        e = PyDict::new(py).into_any();
        reg.set_item(&key, &e)?;
    }
    e.call_method1(pyo3::intern!(py, "setdefault"), ("overrides", PyDict::new(py)))?;
    Ok(e)
}

#[pyfunction]
#[pyo3(name = "link_entry")]
fn py_entry<'py>(scene: &Obj<'py>, path: &Obj<'py>) -> PyResult<Obj<'py>> {
    entry(scene, path)
}

pub fn is_packed(scene: &Obj<'_>, path: &Obj<'_>) -> PyResult<bool> {
    let py = scene.py();
    get_or(&get_or(&registry_view(scene)?, norm(path)?, PyDict::new(py))?, "packed", none(py))?.is_truthy()
}

#[pyfunction]
#[pyo3(name = "link_is_packed")]
fn py_is_packed(scene: &Obj<'_>, path: &Obj<'_>) -> PyResult<bool> {
    is_packed(scene, path)
}

/// Pack (the host file keeps the geometry) or unpack.
#[pyfunction]
#[pyo3(name = "link_set_packed")]
fn set_packed(scene: &Obj<'_>, path: &Obj<'_>, packed: bool) -> PyResult<()> {
    entry(scene, path)?.set_item("packed", packed)
}

/// The source file a host tag belongs to, or None for the host's own.
pub fn source_of_tag<'py>(scene: &Obj<'py>, name: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = scene.py();
    for item in registry_view(scene)?.call_method0(pyo3::intern!(py, "items"))?.try_iter()? {
        let (key, e): (Obj<'py>, Obj<'py>) = item?.extract()?;
        if !is_dict(&e) {
            continue;
        }
        let tags = get(&e, "tags")?;
        if tags.is_truthy()? && tags.contains(name)? {
            return Ok(Some(key));
        }
    }
    Ok(None)
}

#[pyfunction]
#[pyo3(name = "link_source_of_tag")]
fn py_source_of_tag<'py>(scene: &Obj<'py>, name: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    source_of_tag(scene, name)
}

/// The host switched tag `name`: if it is a linked file's, remember it as
/// an override. True when it was.
#[pyfunction]
#[pyo3(name = "link_note_visibility")]
pub fn note_visibility(scene: &Obj<'_>, name: &Obj<'_>, visible: &Obj<'_>) -> PyResult<bool> {
    let Some(path) = source_of_tag(scene, name)? else { return Ok(false) };
    entry(scene, &path)?.get_item("overrides")?.set_item(name, visible.is_truthy()?)?;
    Ok(true)
}

/// Whether the file writer saves `group` as an empty placement (an
/// unpacked link: the source holds the geometry).
#[pyfunction]
#[pyo3(name = "link_save_as_placeholder")]
pub fn save_as_placeholder(group: &Obj<'_>, scene: &Obj<'_>) -> PyResult<bool> {
    match link_of(group)? {
        Some(rec) => Ok(!is_packed(scene, &rec.get_item("path")?)?),
        None => Ok(false),
    }
}

/// The state of one source, shared by file rows, Problems and the link
/// inspector.
#[pyfunction]
#[pyo3(name = "link_state")]
pub fn link_state(scene: &Obj<'_>, path: &Obj<'_>) -> PyResult<String> {
    let py = scene.py();
    let e = get_or(&registry_view(scene)?, norm(path)?, PyDict::new(py))?;
    let p: String = module(py, "os")?.call_method1(pyo3::intern!(py, "fspath"), (path,))?.extract()?;
    let loaded: Option<u128> = get(&e, "loaded_mtime_ns")?.extract()?;
    Ok(crate::project_tree::source_state(&p, get(&e, "packed")?.is_truthy()?, loaded).to_string())
}

/// JSON metadata for the interface and agents, without touching geometry.
#[pyfunction]
#[pyo3(name = "link_query")]
fn query<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = scene.py();
    let pathlib = module(py, "pathlib")?;
    let walk = module(py, "core.group")?.getattr(pyo3::intern!(py, "iter_placements"))?;
    let rows = PyList::empty(py);
    for group in linked_groups(scene, None)? {
        let rec = link_of(&group)?.expect("a linked group");
        let path = pathlib.call_method1(pyo3::intern!(py, "Path"), (rec.get_item("path")?,))?;
        let row = PyDict::new(py);
        row.set_item("uid", group.getattr(pyo3::intern!(py, "uid"))?)?;
        row.set_item("path", path.str()?)?;
        row.set_item("status", link_state(scene, &path)?)?;
        row.set_item("include_nested", get_or(&rec, "include_nested", true)?)?;
        row.set_item("select", get(&rec, "select")?)?;
        row.set_item("stable_uid_count", walk.call1((&group,))?.try_iter()?.count() as i64 - 1)?;
        rows.append(row)?;
    }
    Ok(rows)
}

/// Only source uids or a Navigator expression can define a subset.
#[pyfunction]
#[pyo3(name = "link_validate_select")]
pub fn validate_select(select: &Obj<'_>) -> PyResult<()> {
    let py = select.py();
    if select.is_none() {
        return Ok(());
    }
    let shape = |s: &Obj<'_>| -> PyResult<Option<&'static str>> {
        let d = s.cast::<PyDict>()?;
        let keys: Vec<String> = d.keys().iter().filter_map(|k| str_of(&k)).collect();
        if d.len() != 1 {
            return Ok(None);
        }
        Ok(match keys.first().map(String::as_str) {
            Some("uids") => Some("uids"),
            Some("filter") => Some("filter"),
            _ => None,
        })
    };
    let kind = if is_dict(select) { shape(select)? } else { None };
    let Some(kind) = kind else {
        return Err(PyValueError::new_err("select requires exactly uids or filter"));
    };
    if kind == "uids" {
        let uids = select.get_item("uids")?;
        let ok = uids.is_exact_instance_of::<PyList>() || uids.is_instance_of::<PyList>();
        let good = ok
            && uids.try_iter()?.all(|u| {
                let Ok(u) = u else { return false };
                if u.is_instance_of::<PyString>() {
                    return true;
                }
                u.is_instance_of::<PyList>()
                    && u.is_truthy().unwrap_or(false)
                    && u.try_iter().map(|mut it| it.all(|k| k.is_ok_and(|k| k.is_instance_of::<PyString>()))).unwrap_or(false)
            });
        if !good {
            return Err(PyValueError::new_err("uids must be a list of strings or occurrence UID paths"));
        }
        return Ok(());
    }
    module(py, "built_core.facets")?.call_method1(pyo3::intern!(py, "validate_filter"), (select.get_item("filter")?,))?;
    Ok(())
}

/// Each part of the linked `placement` owned by its parent, as a link's
/// install leaves them: a packed link read back from the file did not go
/// through it, and its parts came back editable.
#[pyfunction]
#[pyo3(name = "link_adopt_parts")]
pub fn adopt_parts(placement: &Obj<'_>) -> PyResult<()> {
    let py = placement.py();
    let mut stack = vec![placement.clone()];
    while let Some(p) = stack.pop() {
        let children = p.getattr(pyo3::intern!(py, "children"))?;
        if children.is_none() {
            continue;
        }
        let kids: Vec<Obj<'_>> = children.try_iter()?.collect::<PyResult<_>>()?;
        for c in kids.iter() {
            c.setattr(pyo3::intern!(py, "owner"), &p)?;
        }
        // Depth first, in order, as the recursion did.
        stack.extend(kids.into_iter().rev());
    }
    Ok(())
}

// ---- renaming a definition's tables ------------------------------------------------

/// A new name for a tag (None: the default) or a material.
enum Rename<'a, 'py> {
    /// `tag_name(path, tag)`, worked out once per name.
    Prefix(&'a str, HashMap<String, Obj<'py>>),
    /// `renames.get(name, name)`, None for the default tag (Unlink's).
    Map(&'a Bound<'py, PyDict>),
}

impl<'py> Rename<'_, 'py> {
    fn tag(&mut self, py: Python<'py>, name: &Obj<'py>) -> PyResult<Obj<'py>> {
        match self {
            Rename::Prefix(stem, names) => {
                let key = if name.is_none() { None } else { str_of(name) };
                if let Some(k) = &key {
                    if let Some(v) = names.get(k) {
                        return Ok(v.clone());
                    }
                }
                let v = PyString::new(py, &tag_label(stem, name)?).into_any();
                if let Some(k) = key {
                    names.insert(k, v.clone());
                }
                Ok(v)
            }
            Rename::Map(map) => {
                let out = match map.get_item(name)? {
                    Some(v) => v,
                    None => name.clone(),
                };
                Ok(if out.eq(DEFAULT_LAYER)? { none(py) } else { out })
            }
        }
    }
}

/// `attrs` (a face's, or a row of a face-attribute table) with its front
/// and back material names moved through `renamed`; the same object when
/// neither is renamed, else a copy.
pub fn renamed_paint<'py>(attrs: &Obj<'py>, renamed: &Bound<'py, PyDict>) -> PyResult<Obj<'py>> {
    let py = attrs.py();
    let mat = get(attrs, "mat")?;
    let back = get(attrs, "back")?;
    let front_hit = renamed.contains(&mat).unwrap_or(false);
    let back_hit = is_dict(&back) && renamed.contains(get(&back, "mat")?).unwrap_or(false);
    if !(front_hit || back_hit) {
        return Ok(attrs.clone());
    }
    let out = PyDict::new(py);
    out.call_method1(pyo3::intern!(py, "update"), (attrs,))?;
    if front_hit {
        out.set_item("mat", renamed.get_item(&mat)?)?;
    }
    if back_hit {
        let b = PyDict::new(py);
        b.call_method1(pyo3::intern!(py, "update"), (&back,))?;
        b.set_item("mat", renamed.get_item(get(&back, "mat")?)?)?;
        out.set_item("back", b)?;
    }
    Ok(out.into_any())
}

#[pyfunction]
#[pyo3(name = "link_renamed_paint")]
fn py_renamed_paint<'py>(attrs: &Obj<'py>, renamed: &Bound<'py, PyDict>) -> PyResult<Obj<'py>> {
    renamed_paint(attrs, renamed)
}

/// Tables renamed once for every mesh that had them (tables are only ever
/// replaced, never edited in place, so sharing them is safe), keeping the
/// originals alive so their address stays a valid key.
#[derive(Default)]
struct Shared {
    hits: HashMap<(u8, usize, bool), (Py<PyAny>, Py<PyAny>)>,
    /// An edge name table: the names each once, and the index map (None
    /// when it is the identity).
    edges: HashMap<(u8, usize, bool), (Py<PyAny>, Py<PyList>, Option<Py<PyAny>>)>,
}

fn is_deferred(py: Python<'_>, mesh: &Obj<'_>) -> PyResult<bool> {
    Ok(mesh.get_type().is(module(py, "built_core.mesh")?.getattr(pyo3::intern!(py, "DeferredMesh"))?))
}

/// The index array `a` copied (`np.full(n, -1, int32)` when absent), with
/// whether it has a negative entry.
fn index_copy<'py>(np: &Obj<'py>, a: &Obj<'py>, n: usize) -> PyResult<(Obj<'py>, bool)> {
    let py = np.py();
    let a = if a.is_none() {
        let kw = PyDict::new(np.py());
        kw.set_item("dtype", np.getattr(pyo3::intern!(py, "int32"))?)?;
        np.call_method(pyo3::intern!(py, "full"), (n, -1), Some(&kw))?
    } else {
        let kw = PyDict::new(np.py());
        kw.set_item("copy", true)?;
        np.call_method(pyo3::intern!(py, "array"), (a,), Some(&kw))?
    };
    let neg = a.getattr(pyo3::intern!(py, "size"))?.is_truthy()? && a.rich_compare(0, pyo3::basic::CompareOp::Lt)?.call_method0(pyo3::intern!(py, "any"))?.is_truthy()?;
    Ok((a, neg))
}

/// Give a deferred mesh new source tables (a copy: the old ones may be
/// shared) and drop what was derived from the old ones: the lazy plan,
/// and the drawing cache's key and bake (the key hashes the tables, so the
/// bake is rebuilt, never shown stale). The store a first touch builds
/// reads these tables (`core.store_mesh.from_deferred`).
///
/// A mesh deferred by hand (no loader block: it builds through its own
/// callback, not the store) builds as before and then has its faces and
/// edges brought to what the new tables say ([`RetaggedBuild`]).
fn retable(mesh: &Obj<'_>, src: &Obj<'_>, tag: Option<OwnedRename>, paint: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
    let py = mesh.py();
    let d = mesh.getattr(pyo3::intern!(py, "__dict__"))?;
    d.set_item("_deferred_source", src)?;
    for key in ["_deferred_plan", "_drawing_staged", "_drawing_source_key"] {
        d.call_method1(pyo3::intern!(py, "pop"), (key, none(py)))?;
    }
    if let Some(original) = d.get_item("_deferred").ok().filter(|b| !b.is_none()) {
        let build = RetaggedBuild { original: original.unbind(), tag, paint: paint.map(|p| p.clone().unbind()) };
        d.set_item("_deferred", Py::new(py, build)?)?;
    }
    Ok(())
}

/// A rename kept past the call that made it.
enum OwnedRename {
    Prefix(String),
    Map(Py<PyDict>),
}

impl OwnedRename {
    fn of(r: &Rename<'_, '_>) -> Self {
        match r {
            Rename::Prefix(stem, _) => OwnedRename::Prefix(stem.to_string()),
            Rename::Map(m) => OwnedRename::Map((*m).clone().unbind()),
        }
    }
}

/// The build of a mesh deferred by hand, then its faces and edges renamed
/// as its tables were.
#[pyclass(module = "built_core", frozen)]
pub struct RetaggedBuild {
    original: Py<PyAny>,
    tag: Option<OwnedRename>,
    paint: Option<Py<PyDict>>,
}

#[pymethods]
impl RetaggedBuild {
    fn __call__(&self, mesh: &Obj<'_>) -> PyResult<()> {
        let py = mesh.py();
        self.original.call1(py, (mesh,))?;
        if let Some(tag) = &self.tag {
            let layers = module(py, "core.layers")?;
            let map;
            let mut rename = match tag {
                OwnedRename::Prefix(stem) => Rename::Prefix(stem, HashMap::new()),
                OwnedRename::Map(m) => {
                    map = m.bind(py).clone();
                    Rename::Map(&map)
                }
            };
            let items: Vec<Obj<'_>> = mesh.getattr(pyo3::intern!(py, "faces"))?.try_iter()?.chain(mesh.getattr(pyo3::intern!(py, "edges"))?.try_iter()?).collect::<PyResult<_>>()?;
            for item in items {
                retag_built(py, &item, &mut rename, &layers)?;
            }
        }
        if let Some(p) = &self.paint {
            restamp_faces(mesh, p.bind(py))?;
        }
        Ok(())
    }

    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.original)?;
        if let Some(OwnedRename::Map(m)) = &self.tag {
            visit.call(m)?;
        }
        if let Some(p) = &self.paint {
            visit.call(p)?;
        }
        Ok(())
    }
}

/// Relabel a mesh the document left deferred in its tables (the face
/// attribute rows and the edge tag names) and move its paint through
/// `paint`, without building it: touching `mesh.faces` builds all of them,
/// and the 1.6 M faces of a facilities model took 46 s to link that way.
/// False for a mesh that is already built.
fn relabel_tables<'py>(
    mesh: &Obj<'py>,
    rename: Option<&mut Rename<'_, 'py>>,
    paint: Option<&Bound<'py, PyDict>>,
    shared: &mut Shared,
) -> PyResult<bool> {
    let py = mesh.py();
    if !is_deferred(py, mesh)? {
        return Ok(false);
    }
    let np = module(py, "numpy")?;
    let d = mesh.getattr(pyo3::intern!(py, "__dict__"))?;
    let src = PyDict::new(py);
    src.call_method1(pyo3::intern!(py, "update"), (d.get_item("_deferred_source")?,))?;
    let meta = PyDict::new(py);
    let m = get(src.as_any(), "meta")?;
    if m.is_truthy()? {
        meta.call_method1(pyo3::intern!(py, "update"), (m,))?;
    }
    let Some(rename) = rename else {
        // Paint only: rows renamed where a material moved, else nothing.
        let paint = paint.expect("a paint map");
        let old = get(meta.as_any(), "face_attrs")?;
        let old = if old.is_truthy()? { old } else { PyList::empty(py).into_any() };
        let rows: Vec<Obj<'py>> = old.try_iter()?.collect::<PyResult<_>>()?;
        let new: Vec<Obj<'py>> = rows.iter().map(|r| renamed_paint(r, paint)).collect::<PyResult<_>>()?;
        if rows.iter().zip(&new).all(|(a, b)| a.is(b)) {
            return Ok(true); // no face of this mesh is renamed
        }
        meta.set_item("face_attrs", PyList::new(py, new)?)?;
        src.set_item("meta", meta)?;
        retable(mesh, src.as_any(), None, Some(paint))?;
        return Ok(true);
    };
    let untagged = rename.tag(py, &none(py))?;
    // Faces: each row of the attribute table, and a row for the faces that
    // had none (they were on the default tag).
    let counts = get(src.as_any(), "counts")?;
    let nfaces = if counts.is_none() { 0 } else { counts.len()? };
    let (fa, neg) = index_copy(&np, &get(src.as_any(), "face_attr")?, nfaces)?;
    let old = get(meta.as_any(), "face_attrs")?;
    let old = if old.is_truthy()? { old } else { PyList::empty(py).into_any() };
    let key = (b'f', old.as_ptr() as usize, neg);
    let table = match shared.hits.get(&key) {
        Some((_, t)) => t.bind(py).clone(),
        None => {
            let rows: Vec<Obj<'py>> = old.try_iter()?.collect::<PyResult<_>>()?;
            let table = PyList::empty(py);
            for r in rows {
                let row = PyDict::new(py);
                row.call_method1(pyo3::intern!(py, "update"), (&r,))?;
                let tag = rename.tag(py, &get(row.as_any(), "layer")?)?;
                set_row_tag(row.as_any(), &tag)?;
                let row = match paint {
                    Some(p) => renamed_paint(row.as_any(), p)?,
                    None => row.into_any(),
                };
                table.append(row)?;
            }
            if neg {
                let row = PyDict::new(py);
                set_row_tag(row.as_any(), &untagged)?;
                table.append(row)?;
            }
            shared.hits.insert(key, (old.clone().unbind(), table.clone().into_any().unbind()));
            table.into_any()
        }
    };
    if neg {
        fa.set_item(fa.rich_compare(0, pyo3::basic::CompareOp::Lt)?, table.len()? - 1)?;
    }
    src.set_item("face_attr", fa)?;
    meta.set_item("face_attrs", &table)?;
    // Edges: the name table, and a name for the untagged ones.
    let edges = get(src.as_any(), "edges")?;
    let nedges = if edges.is_none() { 0 } else { edges.len()? };
    let (lay, neg) = index_copy(&np, &get(src.as_any(), "layers")?, nedges)?;
    let old = get(meta.as_any(), "edge_layers")?;
    let old = if old.is_truthy()? { old } else { PyList::empty(py).into_any() };
    let key = (b'e', old.as_ptr() as usize, neg);
    let (names, remap) = match shared.edges.get(&key) {
        Some((_, n, r)) => (n.bind(py).clone(), r.as_ref().map(|r| r.bind(py).clone())),
        None => {
            // The names as renamed, then each once and none for the default
            // tag: what the store reads (an index per edge, -1 untagged).
            let mut raw: Vec<Obj<'py>> = Vec::new();
            for n in old.try_iter()? {
                raw.push(rename.tag(py, &n?)?);
            }
            if neg {
                raw.push(untagged.clone());
            }
            let names = PyList::empty(py);
            let mut seen: HashMap<String, i64> = HashMap::new();
            let mut remap: Vec<i64> = Vec::with_capacity(raw.len());
            for n in &raw {
                let name = if n.is_none() { None } else { str_of(n).filter(|t| t != DEFAULT_LAYER) };
                remap.push(match name {
                    None => -1,
                    Some(t) => *seen.entry(t.clone()).or_insert_with(|| {
                        let _ = names.append(&t);
                        (names.len() - 1) as i64
                    }),
                });
            }
            let identity = remap.iter().enumerate().all(|(i, &j)| j == i as i64);
            let remap = if identity {
                None
            } else {
                let kw = PyDict::new(py);
                kw.set_item("dtype", np.getattr(pyo3::intern!(py, "int64"))?)?;
                Some(np.call_method(pyo3::intern!(py, "asarray"), (remap,), Some(&kw))?)
            };
            shared.edges.insert(key, (old.clone().unbind(), names.clone().unbind(), remap.as_ref().map(|r| r.clone().unbind())));
            (names, remap)
        }
    };
    let raw_len = old.len()? + usize::from(neg);
    let mut lay = lay;
    if neg {
        lay.set_item(lay.rich_compare(0, pyo3::basic::CompareOp::Lt)?, raw_len - 1)?;
    }
    if let Some(r) = &remap {
        if lay.len()? > 0 {
            lay = r.get_item(&lay)?;
        }
    }
    src.set_item("layers", lay)?;
    meta.set_item("edge_layers", &names)?;
    src.set_item("meta", meta)?;
    retable(mesh, src.as_any(), Some(OwnedRename::of(rename)), paint)?;
    Ok(true)
}

/// A face attribute row on tag `tag` (`assign_layer`'s rule: the default
/// tag is no label).
fn set_row_tag(row: &Obj<'_>, tag: &Obj<'_>) -> PyResult<()> {
    let py = row.py();
    if tag.is_none() || tag.eq(DEFAULT_LAYER)? {
        row.call_method1(pyo3::intern!(py, "pop"), ("layer", none(row.py())))?;
        Ok(())
    } else {
        row.set_item("layer", tag)
    }
}

/// `layer_of` / `assign_layer` for a built element.
fn retag_built<'py>(py: Python<'py>, item: &Obj<'py>, rename: &mut Rename<'_, 'py>, layers: &Obj<'py>) -> PyResult<()> {
    let tag = layers.call_method1(pyo3::intern!(py, "layer_of"), (item,))?;
    let new = rename.tag(py, &tag)?;
    layers.call_method1(pyo3::intern!(py, "assign_layer"), (item, new))?;
    Ok(())
}

fn restamp_faces<'py>(mesh: &Obj<'py>, renamed: &Bound<'py, PyDict>) -> PyResult<()> {
    let py = mesh.py();
    for f in mesh.getattr(pyo3::intern!(py, "faces"))?.try_iter()? {
        let a = f?.getattr(pyo3::intern!(py, "attrs"))?;
        if !a.is_truthy()? {
            continue;
        }
        let mat = get(&a, "mat")?;
        if renamed.contains(&mat)? {
            a.set_item("mat", renamed.get_item(&mat)?)?;
        }
        let back = get(&a, "back")?;
        if is_dict(&back) {
            let bm = get(&back, "mat")?;
            if renamed.contains(&bm)? {
                back.set_item("mat", renamed.get_item(&bm)?)?;
            }
        }
    }
    Ok(())
}

/// Move `mesh`'s faces onto the materials' new names (`renamed`, old →
/// new, from `built_core.document_insert.merge_materials`): a deferred mesh in its face
/// attribute table, unbuilt; a built mesh's faces in place.
#[pyfunction]
#[pyo3(name = "link_restamp_materials")]
pub fn restamp_materials<'py>(mesh: &Obj<'py>, renamed: &Bound<'py, PyDict>) -> PyResult<()> {
    if relabel_tables(mesh, None, Some(renamed), &mut Shared::default())? {
        return Ok(());
    }
    restamp_faces(mesh, renamed)
}

/// A source's definition as read, renamed into the host: every placement
/// below `comp` on the host's names for the source's tags, every mesh's
/// faces and edges too, and the paint on the materials' host names
/// (`renamed`); meshes reused from the previous read (`reused`) are named
/// already. Then each mesh's geometry stamp (what a reload compares) names
/// the source and the material renames.
#[pyfunction]
#[pyo3(name = "link_install_definition")]
pub fn install_definition<'py>(
    comp: &Obj<'py>,
    path: &Obj<'py>,
    reused: &Obj<'py>,
    renamed: &Bound<'py, PyDict>,
) -> PyResult<()> {
    let py = comp.py();
    let layers = module(py, "core.layers")?;
    let stem = stem(path)?;
    let mut rename = Rename::Prefix(&stem, HashMap::new());
    let paint = (!renamed.is_empty()).then_some(renamed);
    let walk = module(py, "core.group")?.getattr(pyo3::intern!(py, "iter_placements"))?;
    let mut seen: HashSet<usize> = HashSet::new();
    let mut shared = Shared::default();
    let mut meshes: Vec<Obj<'py>> = Vec::new();
    for pair in walk.call1((comp,))?.try_iter()? {
        let g = pair?.get_item(0)?;
        if !g.is(comp) {
            retag_built(py, &g, &mut rename, &layers)?;
        }
        if let Some(p) = paint {
            let m = g.getattr(pyo3::intern!(py, "material"))?;
            if m.is_truthy()? {
                g.setattr(pyo3::intern!(py, "material"), renamed_paint(&m, p)?)?;
            }
        }
        let mesh = g.getattr(pyo3::intern!(py, "mesh"))?;
        if !seen.insert(mesh.as_ptr() as usize) {
            continue;
        }
        meshes.push(mesh.clone());
        if reused.contains(&mesh)? {
            continue;
        }
        if relabel_tables(&mesh, Some(&mut rename), paint, &mut shared)? {
            continue;
        }
        for f in mesh.getattr(pyo3::intern!(py, "faces"))?.try_iter()? {
            retag_built(py, &f?, &mut rename, &layers)?;
        }
        for e in mesh.getattr(pyo3::intern!(py, "edges"))?.try_iter()? {
            retag_built(py, &e?, &mut rename, &layers)?;
        }
        if let Some(p) = paint {
            restamp_faces(&mesh, p)?;
        }
    }
    let path_s = module(py, "builtins")?.getattr(pyo3::intern!(py, "str"))?.call1((path,))?;
    let items = PyTuple::new(py, renamed.items().iter())?;
    for mesh in meshes {
        if reused.contains(&mesh)? {
            continue;
        }
        let d = mesh.getattr(pyo3::intern!(py, "__dict__"))?;
        let stamp = get(&d, "_link_geometry")?;
        if !stamp.is_truthy()? {
            continue;
        }
        let mut key = stamp.get_item(0)?;
        if !key.get_item(0)?.eq(&path_s)? {
            key = PyTuple::new(py, [path_s.clone(), items.clone().into_any(), PyString::new(py, "nested").into_any(), key])?.into_any();
        }
        d.set_item("_link_geometry", (key, mesh.getattr(pyo3::intern!(py, "revision"))?))?;
    }
    Ok(())
}

/// Move what `groups` and `meshes` wear onto other tags (`renames`: key →
/// key, `DEFAULT_LAYER` for Untagged). A mesh left deferred is relabelled
/// in its tables, unbuilt, and only when they name a tag that moves.
#[pyfunction]
#[pyo3(name = "link_relabel")]
pub fn relabel<'py>(groups: &Obj<'py>, meshes: &Obj<'py>, renames: &Bound<'py, PyDict>) -> PyResult<()> {
    let py = groups.py();
    if renames.is_empty() {
        return Ok(());
    }
    let layers = module(py, "core.layers")?;
    for g in groups.try_iter()? {
        let g = g?;
        let tag = layers.call_method1(pyo3::intern!(py, "layer_of"), (&g,))?;
        if renames.contains(&tag)? {
            layers.call_method1(pyo3::intern!(py, "assign_layer"), (&g, renames.get_item(&tag)?))?;
        }
    }
    let mut rename = Rename::Map(renames);
    let mut shared = Shared::default();
    let mut seen = HashSet::new();
    let moving: HashSet<String> = renames.keys().iter().filter_map(|k| str_of(&k)).collect();
    for mesh in meshes.try_iter()? {
        let mesh = mesh?;
        if mesh.is_none() || !seen.insert(mesh.as_ptr() as usize) {
            continue;
        }
        if is_deferred(py, &mesh)? {
            let src = mesh.getattr(pyo3::intern!(py, "__dict__"))?.get_item("_deferred_source")?;
            let meta = get(&src, "meta")?;
            let meta = if meta.is_truthy()? { meta } else { PyDict::new(py).into_any() };
            let mut named = false;
            let rows = get(&meta, "face_attrs")?;
            if rows.is_truthy()? {
                for row in rows.try_iter()? {
                    let row = row?;
                    let l = if row.is_truthy()? { get(&row, "layer")? } else { none(py) };
                    named |= str_of(&l).is_some_and(|l| moving.contains(&l));
                }
            }
            let names = get(&meta, "edge_layers")?;
            if names.is_truthy()? {
                for n in names.try_iter()? {
                    named |= str_of(&n?).is_some_and(|l| moving.contains(&l));
                }
            }
            if named {
                relabel_tables(&mesh, Some(&mut rename), None, &mut shared)?;
            }
            continue;
        }
        let items: Vec<Obj<'py>> = mesh.getattr(pyo3::intern!(py, "faces"))?.try_iter()?.chain(mesh.getattr(pyo3::intern!(py, "edges"))?.try_iter()?).collect::<PyResult<_>>()?;
        for item in items {
            let tag = layers.call_method1(pyo3::intern!(py, "layer_of"), (&item,))?;
            if renames.contains(&tag)? {
                layers.call_method1(pyo3::intern!(py, "assign_layer"), (&item, renames.get_item(&tag)?))?;
            }
        }
    }
    Ok(())
}

// ---- the source's tags in the host -------------------------------------------------------

fn folders_of(scene: &Obj<'_>) -> PyResult<Vec<String>> {
    tag_edits::folders_of(scene)
}

fn set_folders(scene: &Obj<'_>, folders: Vec<String>) -> PyResult<()> {
    let py = scene.py();
    scene.setattr(pyo3::intern!(py, "layer_folders"), folders)
}

fn ly_key(ly: &Obj<'_>) -> PyResult<Option<String>> {
    let py = ly.py();
    Ok(str_of(&ly.getattr(pyo3::intern!(py, "key"))?))
}

fn layers_of<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let py = scene.py();
    scene.getattr(pyo3::intern!(py, "layers"))?.try_iter()?.collect()
}

/// The tags something in the document wears.
fn used_tags(scene: &Obj<'_>) -> PyResult<HashSet<String>> {
    let py = scene.py();
    let layer_of = module(py, "core.layers")?.getattr(pyo3::intern!(py, "layer_of"))?;
    let mut out = HashSet::new();
    for e in module(py, "built_core")?.call_method1(pyo3::intern!(py, "purge_layer_carriers"), (scene,))?.try_iter()? {
        if let Some(t) = str_of(&layer_of.call1((e?,))?) {
            out.insert(t);
        }
    }
    Ok(out)
}

/// Keep only the tags `keep` says, as `scene.layers[:] = [...]` did.
fn filter_layers<'py>(scene: &Obj<'py>, keep: impl Fn(&Obj<'py>) -> PyResult<bool>) -> PyResult<()> {
    let py = scene.py();
    let mut kept = Vec::new();
    for ly in layers_of(scene)? {
        if keep(&ly)? {
            kept.push(ly);
        }
    }
    let all = pyo3::types::PySlice::full(py);
    scene.getattr(pyo3::intern!(py, "layers"))?.set_item(all, PyList::new(py, kept)?)
}

/// Bring the source's tags into the host: the folder of the file inside
/// «Linked files», the source's folders below it, each tag as the source
/// shows it unless overridden here. Tags of this file the source no longer
/// has are dropped when nothing uses them, and the empty folders this link
/// made with them.
fn apply_tags<'py>(scene: &Obj<'py>, path: &Obj<'py>, tags: &Obj<'py>) -> PyResult<()> {
    let py = scene.py();
    let layer_cls = module(py, "core.layers")?.getattr(pyo3::intern!(py, "Layer"))?;
    let e = entry(scene, path)?;
    let structure = get(tags, "structure")?;
    if structure.is_truthy()? {
        e.set_item("structure", structure)?;
    } else {
        e.call_method1(pyo3::intern!(py, "pop"), ("structure", none(py)))?;
    }
    let overrides = e.get_item("overrides")?;
    // What the source said last time: a tag the host now shows otherwise
    // was switched here, by any road, and that switch is the host's
    // override from now on.
    let last = e.call_method1(pyo3::intern!(py, "setdefault"), ("source_visible", PyDict::new(py)))?;
    let folder = |name: &str, parent: Option<&str>| -> PyResult<String> {
        if !folders_of(scene)?.iter().any(|f| f == name) {
            let mut all = folders_of(scene)?;
            all.push(name.to_string());
            set_folders(scene, all)?;
        }
        tag_edits::move_folder(scene, &PyString::new(py, name), &parent.map_or_else(|| none(py), |p| PyString::new(py, p).into_any()))?;
        Ok(name.to_string())
    };
    let top = folder(LINKED_FOLDER, None)?;
    let own = folder(&stem(path)?, Some(&top))?;
    let prefix = format!("{own}{SEP}");
    let existing: HashSet<String> = folders_of(scene)?.into_iter().collect();
    let parents = scene.getattr(pyo3::intern!(py, "folder_parents"))?;
    let uids_of = || scene.getattr(pyo3::intern!(py, "layer_folder_uids"));
    // Older hosts recorded only mirrored tags: recover their namespaced
    // folder paths before those tags move, without claiming unrelated
    // folders.
    let mut owned: Vec<Obj<'py>> = Vec::new();
    let owned_set = PySet::empty(py)?;
    if e.contains("folder_uids")? {
        for u in e.get_item("folder_uids")?.try_iter()? {
            owned_set.add(u?)?;
        }
    } else {
        let old_tags = PySet::empty(py)?;
        let t = get(&e, "tags")?;
        if t.is_truthy()? {
            for x in t.try_iter()? {
                old_tags.add(x?)?;
            }
        }
        for ly in layers_of(scene)? {
            let mut f = if old_tags.contains(ly.getattr(pyo3::intern!(py, "key"))?)? { ly.getattr(pyo3::intern!(py, "folder"))? } else { none(py) };
            let mut chain: Vec<String> = Vec::new();
            loop {
                let Some(fs) = str_of(&f) else { break };
                if !existing.contains(&fs) || !fs.starts_with(&prefix) || chain.contains(&fs) {
                    break;
                }
                chain.push(fs.clone());
                f = get(&parents, &fs)?;
            }
            if str_of(&f).as_deref() == Some(own.as_str()) {
                let uids = uids_of()?;
                for name in chain {
                    owned_set.add(uids.get_item(&name)?)?;
                }
            }
        }
    }
    let src_folders = PyDict::new(py);
    let tf = get(tags, "folders")?;
    if tf.is_truthy()? {
        for f in tf.try_iter()? {
            let f = f?;
            let name = format!("{prefix}{}", f.str()?);
            src_folders.set_item(&f, name)?;
        }
    }
    let added: Vec<String> = src_folders.values().iter().filter_map(|v| str_of(&v)).filter(|n| !existing.contains(n)).collect();
    if !added.is_empty() {
        let mut all = folders_of(scene)?;
        all.extend(added.iter().cloned());
        set_folders(scene, all)?;
        folders_of(scene)?; // assign identities before placing children
        let uids = uids_of()?;
        for name in &added {
            owned_set.add(uids.get_item(name)?)?;
        }
    }
    // The host's folder keys are namespaced by the file; their labels are
    // the source's own, so the tree reads as in the source.
    let src_labels = get(tags, "folder_names")?;
    let src_labels = if src_labels.is_truthy()? { src_labels } else { PyDict::new(py).into_any() };
    let labels = PyDict::new(py);
    if let Some(l) = scene.getattr_opt(pyo3::intern!(py, "layer_folder_names"))?.filter(|l| l.is_truthy().unwrap_or(false)) {
        labels.call_method1(pyo3::intern!(py, "update"), (l,))?;
    }
    let src_vis = get_or(tags, "folder_visibility", PyDict::new(py))?;
    let src_parents = get_or(tags, "parents", PyDict::new(py))?;
    for (f, name) in src_folders.iter() {
        let incoming = get_or(&src_vis, &f, true)?;
        let previous = e.call_method1(pyo3::intern!(py, "setdefault"), ("source_folder_visible", PyDict::new(py)))?;
        let vis = scene.getattr(pyo3::intern!(py, "folder_visibility"))?;
        if !previous.contains(&name)? || get_or(&vis, &name, true)?.eq(previous.get_item(&name)?)? {
            vis.set_item(&name, &incoming)?;
        }
        previous.set_item(&name, &incoming)?;
        let parent = get(&src_parents, &f)?;
        let target = get_or(src_folders.as_any(), &parent, &own)?;
        tag_edits::move_folder(scene, &name, &target)?;
        let label = get(&src_labels, &f)?;
        let label = if label.is_truthy()? { label } else { f.clone() };
        if !label.eq(&name)? {
            labels.set_item(&name, label)?;
        }
    }
    scene.setattr(pyo3::intern!(py, "layer_folder_names"), labels)?;

    let wanted = PyList::empty(py);
    let stem_s = stem(path)?;
    let layers_list = scene.getattr(pyo3::intern!(py, "layers"))?;
    let raw_layers = get(tags, "layers")?;
    if raw_layers.is_truthy()? {
        for raw in raw_layers.try_iter()? {
            let src = layer_cls.call_method1(pyo3::intern!(py, "from_dict"), (raw?,))?;
            // Keyed by the source tag's KEY (what the source's entities
            // carry) and labelled like the source shows it.
            let src_key = src.getattr(pyo3::intern!(py, "key"))?;
            let name = PyString::new(py, &tag_label(&stem_s, &src_key)?).into_any();
            wanted.append(&name)?;
            let mut ly = scene.call_method1(pyo3::intern!(py, "layer_by_key"), (&name,))?;
            if ly.is_none() {
                ly = layer_cls.call1((&name,))?;
                layers_list.call_method1(pyo3::intern!(py, "append"), (&ly,))?;
            } else if last.contains(&name)?
                && !overrides.contains(&name)?
                && ly.getattr(pyo3::intern!(py, "visible"))?.is_truthy()? != last.get_item(&name)?.is_truthy()?
            {
                overrides.set_item(&name, ly.getattr(pyo3::intern!(py, "visible"))?.is_truthy()?)?;
            }
            if !src_key.eq(DEFAULT_LAYER)? {
                ly.setattr(pyo3::intern!(py, "name"), src.getattr(pyo3::intern!(py, "name"))?)?;
            }
            last.set_item(&name, src.getattr(pyo3::intern!(py, "visible"))?.is_truthy()?)?;
            for f in [pyo3::intern!(py, "color"), pyo3::intern!(py, "linetype"), pyo3::intern!(py, "lineweight"), pyo3::intern!(py, "opacity")] {
                ly.setattr(f, src.getattr(f)?)?;
            }
            ly.setattr(pyo3::intern!(py, "folder"), get_or(src_folders.as_any(), src.getattr(pyo3::intern!(py, "folder"))?, &own)?)?;
            ly.setattr(pyo3::intern!(py, "visible"), get_or(&overrides, &name, src.getattr(pyo3::intern!(py, "visible"))?)?)?;
        }
    }
    let mut old: HashSet<String> = HashSet::new();
    let t = get(&e, "tags")?;
    if t.is_truthy()? {
        for x in t.try_iter()? {
            if let Some(s) = str_of(&x?) {
                old.insert(s);
            }
        }
    }
    for w in wanted.iter() {
        if let Some(s) = str_of(&w) {
            old.remove(&s);
        }
    }
    e.set_item("tags", &wanted)?;
    if !old.is_empty() {
        let used = used_tags(scene)?;
        filter_layers(scene, |ly| Ok(ly_key(ly)?.is_none_or(|k| !old.contains(&k) || used.contains(&k))))?;
        for name in &old {
            overrides.call_method1(pyo3::intern!(py, "pop"), (name, none(py)))?;
            last.call_method1(pyo3::intern!(py, "pop"), (name, none(py)))?;
        }
    }
    // Remove obsolete leaves first so a live tag or host subfolder protects
    // every ancestor. Only folders this link created qualify.
    let current: HashSet<String> = src_folders.values().iter().filter_map(|v| str_of(&v)).collect();
    let uids = uids_of()?;
    let mut candidates: HashSet<String> = HashSet::new();
    for f in folders_of(scene)? {
        if !current.contains(&f) && f.starts_with(&prefix) && owned_set.contains(uids.get_item(&f)?)? {
            candidates.insert(f);
        }
    }
    let occupied: HashSet<String> = layers_of(scene)?.iter().filter_map(|ly| ly.getattr(pyo3::intern!(py, "folder")).ok().and_then(|f| str_of(&f))).collect();
    while !candidates.is_empty() {
        let parent_names: HashSet<String> =
            scene.getattr(pyo3::intern!(py, "folder_parents"))?.call_method0(pyo3::intern!(py, "values"))?.try_iter()?.filter_map(|v| v.ok().and_then(|v| str_of(&v))).collect();
        let mut leaves: Vec<String> = candidates.iter().filter(|f| !occupied.contains(*f) && !parent_names.contains(*f)).cloned().collect();
        if leaves.is_empty() {
            break;
        }
        leaves.sort();
        for f in &leaves {
            tag_edits::remove_folder(scene, f)?;
            candidates.remove(f);
        }
    }
    let live = PySet::empty(py)?;
    for u in scene.getattr(pyo3::intern!(py, "layer_folder_uids"))?.call_method0(pyo3::intern!(py, "values"))?.try_iter()? {
        live.add(u?)?;
    }
    for u in owned_set.iter() {
        if live.contains(&u)? {
            owned.push(u);
        }
    }
    let kept = PyList::new(py, owned)?;
    kept.sort()?;
    e.set_item("folder_uids", kept)?;
    Ok(())
}

/// What the source's tags look like in this host now: the file's tags,
/// folders and overrides (a reload reconciles only when it moved).
fn mirror<'py>(scene: &Obj<'py>, path: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let e = entry(scene, path)?;
    let wanted: HashSet<String> = {
        let t = get_or(&e, "tags", PyTuple::empty(py))?;
        t.try_iter()?.filter_map(|x| x.ok().and_then(|x| str_of(&x))).collect()
    };
    let own = stem(path)?;
    let prefix = format!("{own}{SEP}");
    let parents = scene.getattr(pyo3::intern!(py, "folder_parents"))?;
    let vis = scene.getattr(pyo3::intern!(py, "folder_visibility"))?;
    let names = scene.getattr(pyo3::intern!(py, "layer_folder_names"))?;
    let uids = scene.getattr(pyo3::intern!(py, "layer_folder_uids"))?;
    let mut folders = Vec::new();
    for f in scene.getattr(pyo3::intern!(py, "layer_folders"))?.try_iter()? {
        let f = f?;
        let fs = f.str()?.to_string();
        if fs == LINKED_FOLDER || fs == own || fs.starts_with(&prefix) {
            folders.push(PyTuple::new(py, [f.clone(), get(&parents, &f)?, get(&vis, &f)?, get(&names, &f)?, get(&uids, &f)?])?);
        }
    }
    let rows = PyList::empty(py);
    for ly in layers_of(scene)? {
        if ly_key(&ly)?.is_some_and(|k| wanted.contains(&k)) {
            rows.append(ly.call_method0(pyo3::intern!(py, "to_dict"))?)?;
        }
    }
    let ov = PyDict::new(py);
    ov.call_method1(pyo3::intern!(py, "update"), (e.get_item("overrides")?,))?;
    Ok(PyTuple::new(py, [rows.into_any(), PyTuple::new(py, folders)?.into_any(), ov.into_any()])?.into_any())
}

/// Reconcile the source's tags with the host, only when the source's
/// table or its mirror in this host moved. True when it did.
#[pyfunction]
#[pyo3(name = "link_sync_tags")]
pub fn sync_tags<'py>(scene: &Obj<'py>, path: &Obj<'py>, tags: &Obj<'py>) -> PyResult<bool> {
    let py = scene.py();
    let key = norm(path)?;
    let cache = scene.getattr(pyo3::intern!(py, "__dict__"))?.call_method1(pyo3::intern!(py, "setdefault"), ("_link_tag_tables", PyDict::new(py)))?;
    let old = get(&cache, &key)?;
    let stale = old.is_none() || old.get_item(0)?.ne(tags)? || old.get_item(1)?.ne(mirror(scene, path)?)?;
    if stale {
        apply_tags(scene, path, tags)?;
        cache.set_item(&key, (tags, mirror(scene, path)?))?;
    }
    Ok(stale)
}

/// The host no longer links `path` (Unlink of its last placement): the
/// file's tags become the host's own, out of «Linked files / <file>» to
/// the top of the tags tree (the same keys, labels, colours and
/// visibility) and the file's folders come along as top-level folders. A
/// tag landing beside one of the host's of the same label is that one, and
/// two folders of one label at one level merge, level by level (as
/// .skp merges a component's tags into the model's by name), unless
/// something outside the unlinked copies wears it (`outside`). The file's
/// copies of the tags of the files linked inside it that nothing wears any
/// more (`used`) go. Returns `{file tag key: host tag key}`.
#[pyfunction]
#[pyo3(name = "link_release_tags")]
pub fn release_tags<'py>(scene: &Obj<'py>, path: &Obj<'py>, used: &Obj<'py>, outside: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let renames = PyDict::new(py);
    let e = get(&registry(scene)?, norm(path)?)?;
    if !is_dict(&e) {
        return Ok(renames);
    }
    let own = stem(path)?;
    if !folders_of(scene)?.contains(&own) || tag_edits::folder_parent(scene, &own, None)?.as_deref() != Some(LINKED_FOLDER) {
        return Ok(renames);
    }
    let mine: HashSet<String> = {
        let t = get(&e, "tags")?;
        if t.is_truthy()? { t.try_iter()?.filter_map(|x| x.ok().and_then(|x| str_of(&x))).collect() } else { HashSet::new() }
    };
    let hidden = !tag_edits::folder_visible(scene, &PyString::new(py, &own), true)?;
    let subfolders = |name: Option<&str>, deep: bool| -> PyResult<Vec<String>> {
        let parents = scene.getattr_opt(pyo3::intern!(py, "folder_parents"))?.filter(|p| p.is_truthy().unwrap_or(false));
        let parents = match parents {
            Some(p) => py.get_type::<PyDict>().call1((p,))?.into_any(),
            None => PyDict::new(py).into_any(),
        };
        super::tag_folders::tag_subfolders(folders_of(scene)?, &parents, name.map(str::to_string), deep)
    };
    let empty = |f: &str| -> PyResult<bool> {
        let held = layers_of(scene)?.iter().any(|ly| ly.getattr(pyo3::intern!(py, "folder")).ok().and_then(|x| str_of(&x)).as_deref() == Some(f));
        Ok(!held && subfolders(Some(f), false)?.is_empty())
    };
    // 1. The copies of the nested files' tags that nothing wears now.
    let nested = format!("{own}{SEP}{LINKED_FOLDER}");
    if folders_of(scene)?.contains(&nested) && tag_edits::folder_parent(scene, &nested, None)?.as_deref() == Some(own.as_str()) {
        let mut below = vec![nested.clone()];
        below.extend(subfolders(Some(&nested), true)?);
        filter_layers(scene, |ly| {
            let k = ly_key(ly)?;
            let f = str_of(&ly.getattr(pyo3::intern!(py, "folder"))?);
            let worn = match &k {
                Some(k) => get(used, k)?.is_truthy()?,
                None => false,
            };
            Ok(!(k.as_ref().is_some_and(|k| mine.contains(k)) && f.is_some_and(|f| below.contains(&f)) && !worn))
        })?;
        for f in below.iter().rev() {
            if empty(f)? {
                tag_edits::remove_folder(scene, f)?;
            }
        }
    }
    // 2. The rest out to the top, merged by label.
    let mut moving: HashSet<String> = subfolders(Some(&own), true)?.into_iter().collect();
    moving.insert(own.clone());
    let untagged = tag_label(&own, &none(py))?;
    fn merge<'py>(
        scene: &Obj<'py>,
        src: &str,
        target: Option<&str>,
        ctx: &Ctx<'_, 'py>,
    ) -> PyResult<()> {
        let py = scene.py();
        let mut theirs: HashMap<String, Obj<'py>> = HashMap::new();
        for ly in layers_of(scene)? {
            let f = ly.getattr(pyo3::intern!(py, "folder"))?;
            let f = if f.is_truthy()? { str_of(&f) } else { None };
            if f.as_deref() == target && ly_key(&ly)?.is_none_or(|k| !ctx.mine.contains(&k)) {
                let name = ly.getattr(pyo3::intern!(py, "name"))?.str()?.to_string();
                theirs.entry(name).or_insert(ly);
            }
        }
        let list = scene.getattr(pyo3::intern!(py, "layers"))?;
        for ly in layers_of(scene)? {
            if str_of(&ly.getattr(pyo3::intern!(py, "folder"))?).as_deref() != Some(src) || !ly_key(&ly)?.is_some_and(|k| ctx.mine.contains(&k)) {
                continue;
            }
            let key = ly.getattr(pyo3::intern!(py, "key"))?;
            let into = if ly_key(&ly)?.as_deref() == Some(ctx.untagged.as_str()) {
                let d = scene.call_method1(pyo3::intern!(py, "layer_by_key"), (DEFAULT_LAYER,))?;
                Some(if d.is_none() { PyString::new(py, DEFAULT_LAYER).into_any() } else { d })
            } else {
                theirs.get(&ly.getattr(pyo3::intern!(py, "name"))?.str()?.to_string()).cloned()
            };
            if let Some(into) = into {
                if !get(ctx.outside, &key)?.is_truthy()? {
                    let k = into.getattr_opt(pyo3::intern!(py, "key"))?.unwrap_or(into.clone());
                    ctx.renames.set_item(&key, k)?;
                    list.call_method1(pyo3::intern!(py, "remove"), (&ly,))?;
                    continue;
                }
            }
            ly.setattr(pyo3::intern!(py, "folder"), target)?;
            if ctx.hidden {
                ly.setattr(pyo3::intern!(py, "visible"), false)?;
            }
        }
        let mut level: HashMap<String, String> = HashMap::new();
        for f in (ctx.subfolders)(target, false)? {
            if !ctx.moving.contains(&f) && f != LINKED_FOLDER {
                let label = tag_edits::folder_label(scene, &PyString::new(py, &f))?.str()?.to_string();
                level.entry(label).or_insert(f);
            }
        }
        for f in (ctx.subfolders)(Some(src), false)? {
            let label = tag_edits::folder_label(scene, &PyString::new(py, &f))?.str()?.to_string();
            match level.get(&label) {
                Some(twin) => {
                    merge(scene, &f, Some(twin), ctx)?;
                    tag_edits::remove_folder(scene, &f)?;
                }
                None => {
                    tag_edits::move_folder(scene, &PyString::new(py, &f), &target.map_or_else(|| none(py), |t| PyString::new(py, t).into_any()))?;
                    if ctx.hidden {
                        scene.getattr(pyo3::intern!(py, "folder_visibility"))?.set_item(&f, false)?;
                    }
                }
            }
        }
        Ok(())
    }
    struct Ctx<'a, 'py> {
        mine: &'a HashSet<String>,
        untagged: String,
        outside: &'a Obj<'py>,
        renames: &'a Bound<'py, PyDict>,
        hidden: bool,
        moving: &'a HashSet<String>,
        subfolders: &'a dyn Fn(Option<&str>, bool) -> PyResult<Vec<String>>,
    }
    let ctx = Ctx { mine: &mine, untagged, outside, renames: &renames, hidden, moving: &moving, subfolders: &subfolders };
    merge(scene, &own, None, &ctx)?;
    if empty(&own)? {
        tag_edits::remove_folder(scene, &own)?;
    }
    if folders_of(scene)?.iter().any(|f| f == LINKED_FOLDER) && empty(LINKED_FOLDER)? {
        tag_edits::remove_folder(scene, LINKED_FOLDER)?;
    }
    Ok(renames)
}

// ---- copies of a source's tree ------------------------------------------------------------

/// Every field a placement has (`core.group.GROUP_FIELDS`).
fn group_fields(py: Python<'_>) -> PyResult<(Obj<'_>, Vec<Bound<'_, PyString>>)> {
    let m = module(py, "core.group")?;
    let names: Vec<String> = m.getattr(pyo3::intern!(py, "GROUP_FIELDS"))?.extract()?;
    let names = names.iter().filter(|n| *n != "children").map(|n| PyString::intern(py, n)).collect();
    Ok((m.getattr(pyo3::intern!(py, "Group"))?, names))
}

/// A shallow copy of `group` (what `copy.copy` gives, without its generic
/// route): every field it has but its children, which every caller gives
/// it.
fn clone_shell<'py>(group: &Obj<'py>, cls: &Obj<'py>, fields: &[Bound<'py, PyString>]) -> PyResult<Obj<'py>> {
    let py = group.py();
    let ty = group.get_type();
    if !ty.is(cls) {
        return module(py, "copy")?.call_method1(pyo3::intern!(py, "copy"), (group,));
    }
    let out = ty.call_method1(pyo3::intern!(py, "__new__"), (&ty,))?;
    for name in fields {
        if let Some(v) = group.getattr_opt(name)? {
            out.setattr(name, v)?;
        }
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(name = "link_clone_shell")]
pub fn clone_shell_of<'py>(group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let (cls, fields) = group_fields(group.py())?;
    clone_shell(group, &cls, &fields)
}

const PLACEMENT_FIELDS: [&str; 12] =
    ["name", "layer", "billboard", "context", "text3d", "hidden", "material", "axes", "component", "exploded", "explode_offset", "locked"];

/// [`PLACEMENT_FIELDS`] as interned names.
fn placement_fields(py: Python<'_>) -> Vec<Bound<'_, PyString>> {
    static NAMES: pyo3::sync::PyOnceLock<Vec<Py<PyString>>> = pyo3::sync::PyOnceLock::new();
    NAMES.get_or_init(py, || PLACEMENT_FIELDS.iter().map(|n| PyString::intern(py, n).unbind()).collect())
        .iter().map(|n| n.bind(py).clone()).collect()
}

struct Populate<'py> {
    placement_uid: String,
    reconcile: bool,
    previous: Bound<'py, PyDict>,
    ambiguous: HashSet<String>,
    installed: HashMap<usize, Obj<'py>>,
    claimed: HashSet<usize>,
    swaps: Vec<(Obj<'py>, usize)>,
    changed: bool,
    cls: Obj<'py>,
    fields: Vec<Bound<'py, PyString>>,
    placement: Vec<Bound<'py, PyString>>,
}

impl<'py> Populate<'py> {
    fn install_mesh(&mut self, mesh: &Obj<'py>, old: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
        let py = mesh.py();
        if !self.reconcile {
            return Ok(mesh.clone());
        }
        if let Some(m) = self.installed.get(&(mesh.as_ptr() as usize)) {
            return Ok(m.clone());
        }
        let stamp = match old {
            Some(o) => get(&o.getattr(pyo3::intern!(py, "__dict__"))?, "_link_geometry")?,
            None => none(mesh.py()),
        };
        let incoming = get(&mesh.getattr(pyo3::intern!(py, "__dict__"))?, "_link_geometry")?;
        let keep = match old {
            Some(o) => {
                stamp.is_truthy()?
                    && incoming.is_truthy()?
                    && stamp.get_item(0)?.eq(incoming.get_item(0)?)?
                    && stamp.get_item(1)?.eq(o.getattr(pyo3::intern!(py, "revision"))?)?
                    && !self.claimed.contains(&(o.as_ptr() as usize))
            }
            None => false,
        };
        let to_use = if keep { old.unwrap().clone() } else { mesh.clone() };
        self.installed.insert(mesh.as_ptr() as usize, to_use.clone());
        self.claimed.insert(to_use.as_ptr() as usize);
        Ok(to_use)
    }

    fn clone(&mut self, group: &Obj<'py>, parent: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = group.py();
        let src_uid: String = group.getattr(pyo3::intern!(py, "uid"))?.extract()?;
        let uid = stable_uid_of(&self.placement_uid, &src_uid);
        let found = if self.ambiguous.contains(&uid) {
            self.previous.call_method1(pyo3::intern!(py, "pop"), ((parent, &uid), none(py)))?
        } else {
            self.previous.call_method1(pyo3::intern!(py, "pop"), (&uid, none(py)))?
        };
        let new = found.is_none();
        let old = if new { None } else { Some(found.getattr(pyo3::intern!(py, "mesh"))?) };
        let result = if new {
            self.changed = true;
            let r = clone_shell(group, &self.cls, &self.fields)?;
            r.setattr(pyo3::intern!(py, "uid"), &uid)?;
            r
        } else {
            for name in self.placement.iter() {
                let value = group.getattr_opt(name)?.unwrap_or_else(|| none(py));
                let mine = found.getattr_opt(name)?.unwrap_or_else(|| none(py));
                if mine.ne(&value)? {
                    self.changed = true;
                    found.setattr(name, deepcopy(&value)?)?;
                }
            }
            found
        };
        let mesh = self.install_mesh(&group.getattr(pyo3::intern!(py, "mesh"))?, old.as_ref())?;
        result.setattr(pyo3::intern!(py, "mesh"), &mesh)?;
        if let Some(o) = &old {
            if !result.getattr(pyo3::intern!(py, "mesh"))?.is(o) {
                self.swaps.push((result.clone(), o.as_ptr() as usize));
            }
        }
        let ext = group.getattr(pyo3::intern!(py, "ext"))?;
        if new || result.getattr(pyo3::intern!(py, "ext"))?.ne(&ext)? {
            result.setattr(pyo3::intern!(py, "ext"), if ext.is_none() { none(py) } else { deepcopy(&ext)? })?;
        }
        let mut ifc = group.getattr(pyo3::intern!(py, "ifc"))?;
        if ifc.is_truthy()? && get(&ifc, "openings")?.is_truthy()? {
            ifc = deepcopy(&ifc)?;
            for opening in ifc.get_item("openings")?.try_iter()? {
                let opening = opening?;
                let g = get(&opening, "group_uid")?;
                if g.is_truthy()? {
                    opening.set_item("group_uid", stable_uid_of(&self.placement_uid, &g.extract::<String>()?))?;
                }
            }
        }
        if new || result.getattr(pyo3::intern!(py, "ifc"))?.ne(&ifc)? {
            result.setattr(pyo3::intern!(py, "ifc"), if ifc.is_none() { none(py) } else { deepcopy(&ifc)? })?;
        }
        let xform = group.getattr(pyo3::intern!(py, "xform"))?;
        if new || result.getattr(pyo3::intern!(py, "xform"))?.ne(&xform)? {
            self.changed = true;
            result.setattr(pyo3::intern!(py, "xform"), matrix(py, &xform)?)?;
        }
        result.setattr(pyo3::intern!(py, "owner"), parent)?;
        result.setattr(pyo3::intern!(py, "source_uid"), &src_uid)?;
        let mut children = Vec::new();
        let kids = group.getattr(pyo3::intern!(py, "children"))?;
        if kids.is_truthy()? {
            for c in kids.try_iter()? {
                children.push(self.clone(&c?, &result)?);
            }
        }
        let children = PyList::new(py, children)?;
        if new || result.getattr(pyo3::intern!(py, "children"))?.ne(&children)? {
            self.changed = true;
            result.setattr(pyo3::intern!(py, "children"), children)?;
        }
        Ok(result)
    }
}

/// Install a source tree (`source`, a definition as read) in `placement`,
/// keeping the host objects that match on a reload (`reconcile`). Returns
/// the changed mesh bindings `(placement, id(old mesh))` when every
/// placement kept its draw state, else None. Parts gone from the source
/// leave `selection`.
#[pyfunction]
#[pyo3(name = "link_populate", signature = (placement, source, *, reconcile=false, selection=None))]
pub fn populate<'py>(placement: &Obj<'py>, source: &Obj<'py>, reconcile: bool, selection: Option<&Obj<'py>>) -> PyResult<Option<Vec<(Obj<'py>, usize)>>> {
    let py = placement.py();
    let (cls, fields) = group_fields(py)?;
    let previous = PyDict::new(py);
    let mut ambiguous = HashSet::new();
    if reconcile {
        let parts = all_parts(py, placement)?;
        for part in &parts {
            let uid = part.getattr(pyo3::intern!(py, "uid"))?;
            if previous.contains(&uid)? {
                if let Some(u) = str_of(&uid) {
                    ambiguous.insert(u);
                }
            }
            previous.set_item(&uid, part)?;
        }
        if !ambiguous.is_empty() {
            // A unique part can move between parents. Repeated uids are
            // meaningful within their own parent occurrence on reload.
            previous.clear();
            for part in &parts {
                let uid = part.getattr(pyo3::intern!(py, "uid"))?;
                if str_of(&uid).is_some_and(|u| ambiguous.contains(&u)) {
                    previous.set_item((part.getattr(pyo3::intern!(py, "owner"))?, &uid), part)?;
                } else {
                    previous.set_item(&uid, part)?;
                }
            }
        }
    }
    let mut p = Populate {
        placement_uid: placement.getattr(pyo3::intern!(py, "uid"))?.extract()?,
        reconcile,
        previous,
        ambiguous,
        installed: HashMap::new(),
        claimed: HashSet::new(),
        swaps: Vec::new(),
        changed: false,
        cls,
        fields,
        placement: placement_fields(py),
    };
    let old_mesh = placement.getattr(pyo3::intern!(py, "mesh"))?;
    let mesh = p.install_mesh(&source.getattr(pyo3::intern!(py, "mesh"))?, Some(&old_mesh))?;
    placement.setattr(pyo3::intern!(py, "mesh"), &mesh)?;
    if !placement.getattr(pyo3::intern!(py, "mesh"))?.is(&old_mesh) {
        p.swaps.push((placement.clone(), old_mesh.as_ptr() as usize));
    }
    let mut children = Vec::new();
    let kids = source.getattr(pyo3::intern!(py, "children"))?;
    if kids.is_truthy()? {
        for c in kids.try_iter()? {
            children.push(p.clone(&c?, placement)?);
        }
    }
    let children = PyList::new(py, children)?;
    if placement.getattr(pyo3::intern!(py, "children"))?.ne(&children)? {
        p.changed = true;
        placement.setattr(pyo3::intern!(py, "children"), children)?;
    }
    if let Some(sel) = selection.filter(|s| !s.is_none()) {
        for removed in p.previous.values().iter() {
            sel.call_method1(pyo3::intern!(py, "discard"), (removed,))?;
        }
    }
    placement.setattr(pyo3::intern!(py, "component"), true)?;
    Ok(if p.changed || !p.previous.is_empty() { None } else { Some(p.swaps) })
}

fn all_parts<'py>(py: Python<'py>, placement: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let kids = placement.getattr(pyo3::intern!(py, "children"))?;
    if !kids.is_truthy()? {
        return Ok(Vec::new());
    }
    module(py, "built_core")?.call_method1(pyo3::intern!(py, "purge_iter_groups"), (kids,))?.try_iter()?.collect()
}

/// The parts of the linked placement `copy`, a copy of `source` (Copy and
/// Paste, a copy made by Move or Rotate, an array, Make Unique of what holds
/// it): what a fresh link of the same file is given, without reading it
/// again. Each part shares its source's geometry, is owned by its parent
/// (what keeps it read-only) and takes the uid a fresh link or the reopened
/// file would give it.
#[pyfunction]
#[pyo3(name = "link_copy_parts")]
pub fn copy_parts<'py>(source: &Obj<'py>, copy: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = source.py();
    // One more occurrence of the definition the document holds for this
    // file: projected from it, as a fresh link of it is.
    if let Some(def) = definition_of(source)? {
        return project_parts(copy, &def.get_item(0)?);
    }
    let (cls, fields) = group_fields(py)?;
    let new_uid = module(py, "core.group")?;
    let copy_uid: String = copy.getattr(pyo3::intern!(py, "uid"))?.extract()?;
    let uids = PyDict::new(py);
    fn clone<'py>(
        group: &Obj<'py>,
        parent: &Obj<'py>,
        cx: &(Obj<'py>, Vec<Bound<'py, PyString>>, Obj<'py>, String, Bound<'py, PyDict>),
    ) -> PyResult<Obj<'py>> {
        let py = group.py();
        let (cls, fields, groups, copy_uid, uids) = cx;
        let out = clone_shell(group, cls, fields)?;
        let origin = group.getattr_opt(pyo3::intern!(py, "source_uid"))?.filter(|o| o.is_truthy().unwrap_or(false));
        let uid = match origin {
            Some(o) => PyString::new(py, &stable_uid_of(copy_uid, &o.extract::<String>()?)).into_any(),
            None => groups.call_method0(pyo3::intern!(py, "new_uid"))?,
        };
        out.setattr(pyo3::intern!(py, "uid"), &uid)?;
        uids.set_item(group.getattr(pyo3::intern!(py, "uid"))?, &uid)?;
        out.setattr(pyo3::intern!(py, "owner"), parent)?;
        out.setattr(pyo3::intern!(py, "context"), none(py))?;
        out.setattr(pyo3::intern!(py, "xform"), matrix(py, &group.getattr(pyo3::intern!(py, "xform"))?)?)?;
        out.setattr(pyo3::intern!(py, "axes"), matrix(py, &group.getattr(pyo3::intern!(py, "axes"))?)?)?;
        for name in [pyo3::intern!(py, "ext"), pyo3::intern!(py, "ifc"), pyo3::intern!(py, "material"), pyo3::intern!(py, "exploded"), pyo3::intern!(py, "text3d")] {
            if let Some(v) = group.getattr_opt(name)?.filter(|v| !v.is_none()) {
                out.setattr(name, deepcopy(&v)?)?;
            }
        }
        let mut kids = Vec::new();
        let ch = group.getattr(pyo3::intern!(py, "children"))?;
        if ch.is_truthy()? {
            for c in ch.try_iter()? {
                kids.push(clone(&c?, &out, cx)?);
            }
        }
        out.setattr(pyo3::intern!(py, "children"), PyList::new(py, kids)?)?;
        Ok(out)
    }
    let cx = (cls, fields, new_uid, copy_uid, uids);
    let mut parts = Vec::new();
    let ch = source.getattr(pyo3::intern!(py, "children"))?;
    if ch.is_truthy()? {
        for c in ch.try_iter()? {
            parts.push(clone(&c?, copy, &cx)?);
        }
    }
    let parts = PyList::new(py, parts)?;
    let all: Vec<Obj<'py>> = if parts.is_empty() {
        Vec::new()
    } else {
        module(py, "built_core")?.call_method1(pyo3::intern!(py, "purge_iter_groups"), (&parts,))?.try_iter()?.collect::<PyResult<_>>()?
    };
    let mut any_ifc = false;
    for part in &all {
        any_ifc |= part.getattr(pyo3::intern!(py, "ifc"))?.is_truthy()?;
    }
    if any_ifc {
        // An opening names the part that fills it: the copy's own.
        for part in &all {
            let ifc = part.getattr(pyo3::intern!(py, "ifc"))?;
            let ifc = if ifc.is_truthy()? { ifc } else { PyDict::new(py).into_any() };
            let openings = get_or(&ifc, "openings", none(py))?;
            if !openings.is_truthy()? {
                continue;
            }
            for opening in openings.try_iter()? {
                let opening = opening?;
                if is_dict(&opening) {
                    let g = get(&opening, "group_uid")?;
                    if cx.4.contains(&g)? {
                        opening.set_item("group_uid", cx.4.get_item(&g)?)?;
                    }
                }
            }
        }
    }
    Ok(parts)
}

/// The configuration a linked file's definition is held under: its
/// normalised path, its subset and its nested-link policy.
#[pyfunction]
#[pyo3(name = "link_definition_key")]
pub fn definition_key(path: &Obj<'_>, select: &Obj<'_>, nested: &Obj<'_>) -> PyResult<String> {
    Ok(format!("{}\0{}\0{}", norm(path)?, select.repr()?, if nested.is_truthy()? { "True" } else { "False" }))
}

/// The scene root `group` sits under (through each node's first owner).
fn root_of<'py>(py: Python<'py>, group: &Obj<'py>) -> Option<Bound<'py, PyAny>> {
    let mut k = handle_key(group).ok()?;
    let d = doc::lock();
    for _ in 0..1_000_000 {
        let n = d.node(k)?;
        if n.kind == doc::Kind::Root {
            return d.handle_ref(py, k).map(|h| h.into_bound(py));
        }
        k = *n.owners.first()?;
    }
    None
}

/// `(definition, tags)`: what the document holds for the file `group`
/// links (with its subset and nested-link policy), or None (a packed link
/// read back, a placement out of any scene, a file not read this session).
#[pyfunction]
#[pyo3(name = "link_definition")]
pub fn definition_of<'py>(group: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = group.py();
    let Some(rec) = link_of(group)? else { return Ok(None) };
    let Some(root) = root_of(py, group) else { return Ok(None) };
    let key = definition_key(&rec.get_item("path")?, &get(&rec, "select")?, &get_or(&rec, "include_nested", true)?)?;
    let def = root.call_method1(pyo3::intern!(py, "_link_definition"), (key,))?;
    Ok((!def.is_none()).then_some(def))
}

/// Release what the root holds for the files `paths` (normalised): no
/// placement links them any more.
pub fn release_definitions(scene: &Obj<'_>, paths: &[String]) -> PyResult<()> {
    let py = scene.py();
    let root = scene.getattr(pyo3::intern!(py, "_root"))?;
    let configs: Vec<String> = root.call_method0(pyo3::intern!(py, "_link_configs"))?.extract()?;
    for c in configs {
        if paths.iter().any(|p| c.split('\0').next() == Some(p.as_str())) {
            root.call_method1(pyo3::intern!(py, "_set_link_definition"), (c,))?;
        }
    }
    Ok(())
}

/// The parts of a new occurrence `copy` of definition `comp`: each part
/// as a fresh link of the file gives it, under the uid
/// `stable_uid(copy uid, source uid)`, sharing the definition's geometry.
fn project_parts<'py>(copy: &Obj<'py>, comp: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = copy.py();
    let (cls, fields) = group_fields(py)?;
    let mut p = Populate {
        placement_uid: copy.getattr(pyo3::intern!(py, "uid"))?.extract()?,
        reconcile: false,
        previous: PyDict::new(py),
        ambiguous: HashSet::new(),
        installed: HashMap::new(),
        claimed: HashSet::new(),
        swaps: Vec::new(),
        changed: false,
        cls,
        fields,
        placement: placement_fields(py),
    };
    let mut parts = Vec::new();
    let kids = comp.getattr(pyo3::intern!(py, "children"))?;
    if kids.is_truthy()? {
        for c in kids.try_iter()? {
            parts.push(p.clone(&c?, copy)?);
        }
    }
    PyList::new(py, parts)
}

/// A copy of `mesh` that shares nothing with it, for content the host now
/// owns. A mesh still deferred stays deferred: the copy builds from the same
/// read-only tables on its own first touch. A built or store-backed mesh
/// is deep-copied (the same elements, flags and attributes, in order).
#[pyfunction]
#[pyo3(name = "link_own_mesh")]
pub fn own_mesh<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let m = module(py, "built_core.mesh")?;
    if is_deferred(py, mesh)? {
        let d = mesh.getattr(pyo3::intern!(py, "__dict__"))?;
        let out = m.getattr(pyo3::intern!(py, "Mesh"))?.call0()?;
        let deferred = m.getattr(pyo3::intern!(py, "DeferredMesh"))?;
        if deferred
            .call_method1(pyo3::intern!(py, "defer"), (&out, d.get_item("_deferred")?, d.get_item("_deferred_nfaces")?, d.get_item("_deferred_source")?))?
            .is_truthy()?
        {
            let od = out.getattr(pyo3::intern!(py, "__dict__"))?;
            if d.contains("_deferred_geom")? {
                od.set_item("_deferred_geom", d.get_item("_deferred_geom")?)?;
            }
            return Ok(out);
        }
    }
    let out = deepcopy(mesh)?;
    out.getattr(pyo3::intern!(py, "__dict__"))?.call_method1(pyo3::intern!(py, "pop"), ("_link_geometry", none(py)))?;
    Ok(out)
}

fn without_link<'py>(py: Python<'py>, ext: &Obj<'py>) -> PyResult<Obj<'py>> {
    if !ext.is_truthy()? {
        return Ok(none(py));
    }
    let rest = PyDict::new(py);
    for (k, v) in ext.call_method0(pyo3::intern!(py, "items"))?.try_iter()?.map(|kv| kv.and_then(|kv| kv.extract::<(Obj<'py>, Obj<'py>)>())).collect::<PyResult<Vec<_>>>()? {
        if !k.eq(LINK_KEY)? {
            rest.set_item(k, deepcopy(&v)?)?;
        }
    }
    Ok(if rest.is_empty() { none(py) } else { rest.into_any() })
}

struct Unlinked<'py> {
    meshes: HashMap<usize, Obj<'py>>,
    pairs: Obj<'py>,
    nested: Option<Obj<'py>>,
    cls: Obj<'py>,
    fields: Vec<Bound<'py, PyString>>,
}

impl<'py> Unlinked<'py> {
    fn own(&mut self, mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = mesh.py();
        if let Some(m) = self.meshes.get(&(mesh.as_ptr() as usize)) {
            return Ok(m.clone());
        }
        let copy = own_mesh(mesh)?;
        self.meshes.insert(mesh.as_ptr() as usize, copy.clone());
        self.pairs.call_method1(pyo3::intern!(py, "append"), (PyTuple::new(py, [mesh.clone(), copy.clone()])?,))?;
        Ok(copy)
    }

    fn shell(&self, group: &Obj<'py>, out: &Obj<'py>) -> PyResult<()> {
        let py = group.py();
        out.setattr(pyo3::intern!(py, "context"), none(py))?;
        out.setattr(pyo3::intern!(py, "xform"), matrix(py, &group.getattr(pyo3::intern!(py, "xform"))?)?)?;
        out.setattr(pyo3::intern!(py, "axes"), matrix(py, &group.getattr(pyo3::intern!(py, "axes"))?)?)?;
        for name in [pyo3::intern!(py, "ifc"), pyo3::intern!(py, "material"), pyo3::intern!(py, "exploded"), pyo3::intern!(py, "text3d")] {
            if let Some(v) = group.getattr_opt(name)?.filter(|v| !v.is_none()) {
                out.setattr(name, deepcopy(&v)?)?;
            }
        }
        Ok(())
    }

    fn linked_part(&self, group: &Obj<'py>, parent: &Obj<'py>, part_pairs: &Bound<'py, PyList>) -> PyResult<Obj<'py>> {
        // Read-only like every part of a link: owned, its mesh shared.
        let py = group.py();
        let out = clone_shell(group, &self.cls, &self.fields)?;
        part_pairs.append((group, &out))?;
        out.setattr(pyo3::intern!(py, "owner"), parent)?;
        self.shell(group, &out)?;
        let ext = group.getattr(pyo3::intern!(py, "ext"))?;
        if !ext.is_none() {
            out.setattr(pyo3::intern!(py, "ext"), deepcopy(&ext)?)?;
        }
        let mut kids = Vec::new();
        let ch = group.getattr(pyo3::intern!(py, "children"))?;
        if ch.is_truthy()? {
            for c in ch.try_iter()? {
                kids.push(self.linked_part(&c?, &out, part_pairs)?);
            }
        }
        out.setattr(pyo3::intern!(py, "children"), PyList::new(py, kids)?)?;
        Ok(out)
    }

    fn clone(&mut self, group: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = group.py();
        let out = clone_shell(group, &self.cls, &self.fields)?;
        self.pairs.call_method1(pyo3::intern!(py, "append"), ((group, &out),))?;
        out.setattr(pyo3::intern!(py, "owner"), none(py))?;
        if out.getattr_opt(pyo3::intern!(py, "source_uid"))?.is_some() {
            let _ = out.delattr(pyo3::intern!(py, "source_uid"));
        }
        self.shell(group, &out)?;
        if is_linked(group)? {
            out.setattr(pyo3::intern!(py, "ext"), deepcopy(&group.getattr(pyo3::intern!(py, "ext"))?)?)?;
            let part_pairs = PyList::empty(py);
            let mut kids = Vec::new();
            let ch = group.getattr(pyo3::intern!(py, "children"))?;
            if ch.is_truthy()? {
                for c in ch.try_iter()? {
                    kids.push(self.linked_part(&c?, &out, &part_pairs)?);
                }
            }
            out.setattr(pyo3::intern!(py, "children"), PyList::new(py, kids)?)?;
            match &self.nested {
                Some(n) => n.call_method1(pyo3::intern!(py, "append"), ((group, &out, &part_pairs),))?,
                None => self.pairs.call_method1(pyo3::intern!(py, "extend"), (&part_pairs,))?,
            };
            return Ok(out);
        }
        out.setattr(pyo3::intern!(py, "mesh"), self.own(&group.getattr(pyo3::intern!(py, "mesh"))?)?)?;
        out.setattr(pyo3::intern!(py, "ext"), without_link(py, &group.getattr(pyo3::intern!(py, "ext"))?)?)?;
        let mut kids = Vec::new();
        let ch = group.getattr(pyo3::intern!(py, "children"))?;
        if ch.is_truthy()? {
            for c in ch.try_iter()? {
                kids.push(self.clone(&c?)?);
            }
        }
        out.setattr(pyo3::intern!(py, "children"), PyList::new(py, kids)?)?;
        Ok(out)
    }
}

/// `(mesh, children, ext)`: what the linked `placement` holds, as the
/// host's own content (Unlink). Every mesh is its own (parts that shared
/// one inside this placement still share their copy) and no geometry is
/// shared with the source or with its other placements. The parts keep
/// their uids, names, tags, paint and matrices and lose their link
/// parentage. A file linked inside the source stays a link: its placement
/// comes across with its link record and its parts as they showed. `pairs`
/// receives `(old, new)` for every part and mesh copied; `nested` receives
/// `(old, new, part_pairs)` for each file linked inside that stays a link.
#[pyfunction]
#[pyo3(name = "link_unlinked_copy", signature = (placement, pairs=None, nested=None))]
pub fn unlinked_copy<'py>(placement: &Obj<'py>, pairs: Option<&Obj<'py>>, nested: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyTuple>> {
    let py = placement.py();
    let (cls, fields) = group_fields(py)?;
    let pairs = match pairs.filter(|p| !p.is_none()) {
        Some(p) => p.clone(),
        None => PyList::empty(py).into_any(),
    };
    let mut u = Unlinked { meshes: HashMap::new(), pairs, nested: nested.filter(|n| !n.is_none()).cloned(), cls, fields };
    let mesh = u.own(&placement.getattr(pyo3::intern!(py, "mesh"))?)?;
    let mut kids = Vec::new();
    let ch = placement.getattr(pyo3::intern!(py, "children"))?;
    if ch.is_truthy()? {
        for c in ch.try_iter()? {
            kids.push(u.clone(&c?)?);
        }
    }
    PyTuple::new(py, [mesh, PyList::new(py, kids)?.into_any(), without_link(py, &placement.getattr(pyo3::intern!(py, "ext"))?)?])
}

/// `(old, new)` for the parts and meshes of a file linked inside a link
/// just unlinked, now read again as the host's own link (`new`): the part
/// read again for each one `old` held. `old`'s parts came through the outer
/// file, under the uids it gave them: its copy of this file named by
/// `old.source_uid` ([`stable_uid_of`]).
pub fn nested_pairs<'py>(old: &Obj<'py>, new: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = old.py();
    let out = PyList::empty(py);
    let (om, nm) = (old.getattr(pyo3::intern!(py, "mesh"))?, new.getattr(pyo3::intern!(py, "mesh"))?);
    if !om.is_none() && !nm.is_none() {
        out.append((&om, &nm))?;
    }
    let Some(origin) = old.getattr_opt(pyo3::intern!(py, "source_uid"))?.filter(|o| o.is_truthy().unwrap_or(false)) else {
        return Ok(out);
    };
    let origin: String = origin.extract()?;
    let mut fresh: HashMap<String, Obj<'py>> = HashMap::new();
    for part in all_parts(py, new)? {
        if let Some(src) = part.getattr_opt(pyo3::intern!(py, "source_uid"))?.filter(|s| s.is_truthy().unwrap_or(false)) {
            fresh.insert(stable_uid_of(&origin, &src.extract::<String>()?), part);
        }
    }
    let mut meshes: HashSet<usize> = HashSet::from([om.as_ptr() as usize]);
    for part in all_parts(py, old)? {
        let key = part.getattr_opt(pyo3::intern!(py, "source_uid"))?.and_then(|s| str_of(&s));
        let Some(twin) = key.and_then(|k| fresh.get(&k)) else { continue };
        out.append((&part, twin))?;
        let pm = part.getattr(pyo3::intern!(py, "mesh"))?;
        if meshes.insert(pm.as_ptr() as usize) {
            out.append((pm, twin.getattr(pyo3::intern!(py, "mesh"))?))?;
        }
    }
    Ok(out)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(tag_name, m)?)?;
    m.add_function(wrap_pyfunction!(py_stem, m)?)?;
    m.add_function(wrap_pyfunction!(py_norm, m)?)?;
    m.add_function(wrap_pyfunction!(stable_uid, m)?)?;
    m.add_function(wrap_pyfunction!(py_link_of, m)?)?;
    m.add_function(wrap_pyfunction!(py_is_linked, m)?)?;
    m.add_function(wrap_pyfunction!(py_linked_ancestor, m)?)?;
    m.add_function(wrap_pyfunction!(py_linked_groups, m)?)?;
    m.add_function(wrap_pyfunction!(py_sources, m)?)?;
    m.add_function(wrap_pyfunction!(py_registry_view, m)?)?;
    m.add_function(wrap_pyfunction!(py_entry, m)?)?;
    m.add_function(wrap_pyfunction!(py_is_packed, m)?)?;
    m.add_function(wrap_pyfunction!(set_packed, m)?)?;
    m.add_function(wrap_pyfunction!(py_source_of_tag, m)?)?;
    m.add_function(wrap_pyfunction!(note_visibility, m)?)?;
    m.add_function(wrap_pyfunction!(save_as_placeholder, m)?)?;
    m.add_function(wrap_pyfunction!(link_state, m)?)?;
    m.add_function(wrap_pyfunction!(query, m)?)?;
    m.add_function(wrap_pyfunction!(validate_select, m)?)?;
    m.add_function(wrap_pyfunction!(adopt_parts, m)?)?;
    m.add_function(wrap_pyfunction!(py_renamed_paint, m)?)?;
    m.add_function(wrap_pyfunction!(restamp_materials, m)?)?;
    m.add_function(wrap_pyfunction!(install_definition, m)?)?;
    m.add_function(wrap_pyfunction!(relabel, m)?)?;
    m.add_function(wrap_pyfunction!(sync_tags, m)?)?;
    m.add_function(wrap_pyfunction!(release_tags, m)?)?;
    m.add_function(wrap_pyfunction!(clone_shell_of, m)?)?;
    m.add_function(wrap_pyfunction!(populate, m)?)?;
    m.add_function(wrap_pyfunction!(copy_parts, m)?)?;
    m.add_function(wrap_pyfunction!(own_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(unlinked_copy, m)?)?;
    m.add_function(wrap_pyfunction!(definition_key, m)?)?;
    m.add_function(wrap_pyfunction!(definition_of, m)?)?;
    m.add_class::<RetaggedBuild>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_uids() {
        assert_eq!(normpath("/a//b/./c/../d/"), "/a/b/d");
        assert_eq!(normpath("//a/../b"), "//b");
        assert_eq!(normpath("///a"), "/a");
        assert_eq!(normpath("a/../../b"), "../b");
        assert_eq!(stem_str("/a/Site.igz"), "Site");
        assert_eq!(stem_str("/a/.hidden"), ".hidden");
        assert_eq!(stem_str("/a/b.c.igz"), "b.c");
        assert_eq!(stem_str("/a/b/"), "b");
        assert_eq!(stable_uid_of("a", "b"), "e0a16d287f2551ce");
    }
}
