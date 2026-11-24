// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The document model: placements, scene roots and tags in one arena,
//! known by `(slot, generation)` keys, with a journal that makes every
//! History step a transaction (docs/rust-document-model.md).
//!
//! One arena per process. Every write goes through [`Doc::write`] or the
//! list ops, which journal it while a transaction is open on the thread
//! that opened it; undo and redo apply a closed transaction's ops
//! backwards or forwards.
//!
//! Python handles. A node is the data of a Python handle
//! (`ingetrazo_core.Placement` / `Tag`); the node lives exactly as long as
//! its handle. A list that names a node holds a strong reference to the
//! node's handle (`strong`, taken when the first owner appears), which is
//! how `g.children.append(c)` keeps `c` alive the way a Python list did.
//!
//! The lock rule: while the arena's lock is held, nothing may allocate,
//! release or call into Python (an allocation can run the GC, which can
//! free a handle, whose `Drop` needs the lock). Increfs are fine. Releases
//! are collected in a `Trash` and dropped after unlocking.

mod assembly;
mod definition;
mod geometry;
use definition::DefinitionRef;
use std::sync::Arc;
mod anchor;
mod history;
mod paint;
mod restamp;
mod settings;
mod feed;
mod execute;
pub mod commands;
mod steps;
mod group_steps;
mod file_view;
pub(crate) mod drawing;
pub mod occurrences;
pub mod py;
pub mod value;

use pyo3::prelude::*;
use pyo3::ffi;
use std::cell::UnsafeCell;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::ThreadId;
use value::Value;

/// A small multiplicative hasher for keys (`u64`) and `(key, field)`
/// pairs: the journal's maps are hit once per write, and SipHash was most
/// of a bulk write's cost.
#[derive(Default, Clone, Copy)]
pub struct KeyHasher(u64);

impl Hasher for KeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.write_u64(u64::from_le_bytes(c.try_into().expect("8 bytes")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut b = [0u8; 8];
            b[..rest.len()].copy_from_slice(rest);
            self.write_u64(u64::from_le_bytes(b) ^ ((rest.len() as u64) << 59));
        }
    }
    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn write_u32(&mut self, v: u32) {
        self.write_u64(v as u64)
    }
    #[inline]
    fn write_u16(&mut self, v: u16) {
        self.write_u64(v as u64)
    }
    #[inline]
    fn write_u8(&mut self, v: u8) {
        self.write_u64(v as u64)
    }
    #[inline]
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64)
    }
}

/// The tree invariant (slice 2): a placement is in one list (a group's
/// children, a scene's top level or its kept definitions), once. 0 = not
/// checked, 1 = reported (`PARENT_REPORTS`, turned into Python warnings by
/// the list ops), 2 = refused (`ValueError`). Undo and redo replay
/// journals unchecked.
pub static PARENT_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
pub static PARENT_REPORTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<KeyHasher>>;
pub type FastSet<K> = HashSet<K, BuildHasherDefault<KeyHasher>>;

pub type Key = u64;

#[inline]
pub fn make_key(slot: u32, gen: u32) -> Key {
    ((gen as u64) << 32) | slot as u64
}
#[inline]
fn slot_of(k: Key) -> usize {
    (k & 0xffff_ffff) as usize
}
#[inline]
fn gen_of(k: Key) -> u32 {
    (k >> 32) as u32
}

/// A placement matrix: the 16 binary64 model values (column-major, as
/// `QMatrix4x4.data()`), and for a `.built` placement the binary64 values
/// it was read with plus their projection (`formats/built._built_source`).
#[derive(Clone, Debug, PartialEq)]
pub struct Xform {
    pub f: [f64; 16],
    pub src: Option<Box<([f64; 16], [f64; 16])>>,
}

impl Xform {
    pub fn doubles(&self) -> [f64; 16] {
        std::array::from_fn(|j| {
            let current = self.f[j];
            match &self.src {
                Some(saved) if saved.1[j] == current => saved.0[j],
                _ => current,
            }
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Root,
    Placement,
    Tag,
    Material,
    Record,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Root => "root",
            Kind::Placement => "placement",
            Kind::Tag => "tag",
            Kind::Material => "material",
            Kind::Record => "record",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Field {
    Definition,
    Billboard,
    Uid,
    Name,
    Tag,
    Xform,
    Axes,
    Component,
    Hidden,
    Locked,
    Material,
    Key,
    Visible,
    Color,
    Folder,
    Texture,
    Opacity,
    Finish,
    Pbr,
    /// A named attribute ([`attr_id`]): a custom attribute of a placement
    /// (`ifc`, `ext`, `text3d`), a tag's sheet style, a scene table (tag
    /// folders, the active tag, plugin data). Absent reads as None.
    Attr(u16),
}

/// The attribute names, by id (interned once per process: the Python
/// classes declare theirs at import).
static ATTR_NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// The id of attribute `name`, made on first use.
pub fn attr_id(name: &str) -> Result<u16, String> {
    let mut names = ATTR_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = names.iter().position(|&n| n == name) {
        return Ok(i as u16);
    }
    if names.len() >= u16::MAX as usize {
        return Err("too many document attributes".into());
    }
    names.push(Box::leak(name.to_owned().into_boxed_str()));
    Ok((names.len() - 1) as u16)
}

pub fn attr_name(id: u16) -> &'static str {
    let names = ATTR_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    names.get(id as usize).copied().unwrap_or("?")
}

impl Field {
    /// The field a Python attribute name stands for (`layer` is the tag).
    pub fn from_name(name: &str) -> Option<Field> {
        Some(match name {
            "uid" => Field::Uid,
            "name" => Field::Name,
            "layer" => Field::Tag,
            "xform" => Field::Xform,
            "axes" => Field::Axes,
            "billboard" => Field::Billboard,
            "component" => Field::Component,
            "hidden" => Field::Hidden,
            "locked" => Field::Locked,
            "material" => Field::Material,
            "key" => Field::Key,
            "visible" => Field::Visible,
            "color" => Field::Color,
            "folder" => Field::Folder,
            "texture" => Field::Texture,
            "opacity" => Field::Opacity,
            "finish" => Field::Finish,
            "pbr" => Field::Pbr,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Field::Definition => "definition",
            Field::Uid => "uid",
            Field::Name => "name",
            Field::Tag => "layer",
            Field::Xform => "xform",
            Field::Axes => "axes",
            Field::Billboard => "billboard",
            Field::Component => "component",
            Field::Hidden => "hidden",
            Field::Locked => "locked",
            Field::Material => "material",
            Field::Key => "key",
            Field::Visible => "visible",
            Field::Color => "color",
            Field::Folder => "folder",
            Field::Texture => "texture",
            Field::Opacity => "opacity",
            Field::Finish => "finish",
            Field::Pbr => "pbr",
            Field::Attr(a) => attr_name(a),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FVal {
    Definition(Option<Arc<DefinitionRef>>),
    Str(Option<String>),
    Bool(bool),
    Xf(Option<Xform>),
    Val(Option<Value>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ListId {
    Children = 0,
    Tags = 1,
    /// A scene's kept component definitions (`scene.kept_components`).
    Kept = 2,
    /// A scene's materials registry (`scene.materials`), with its names.
    Mats = 3,
}

pub const LISTS: [ListId; 4] = [ListId::Children, ListId::Tags, ListId::Kept, ListId::Mats];

impl ListId {
    pub fn from_u8(v: u8) -> ListId {
        match v {
            1 => ListId::Tags,
            2 => ListId::Kept,
            3 => ListId::Mats,
            _ => ListId::Children,
        }
    }
    fn member(self) -> Kind {
        match self {
            ListId::Children | ListId::Kept => Kind::Placement,
            ListId::Tags => Kind::Tag,
            ListId::Mats => Kind::Material,
        }
    }
    /// Who may own this list.
    pub fn owned_by(self, k: Kind) -> bool {
        match self {
            ListId::Children => matches!(k, Kind::Root | Kind::Placement),
            _ => k == Kind::Root,
        }
    }
}

#[derive(Default)]
pub struct PlacementData {
    pub definition: Option<Arc<DefinitionRef>>,
    pub uid: Option<String>,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub xform: Option<Xform>,
    pub axes: Option<Xform>,
    pub billboard: Option<Box<Value>>,
    pub component: bool,
    pub hidden: bool,
    pub locked: bool,
    pub material: Option<Value>,
}

#[derive(Default)]
pub struct TagData {
    pub uid: Option<String>,
    pub name: Option<String>,
    pub key: Option<String>,
    pub visible: bool,
    pub locked: bool,
    pub color: Option<Value>,
    pub folder: Option<String>,
}

/// A named paint recipe (`core.materials.Material`). The registry that
/// names it (`scene.materials`) stays a Python dict in slice 1.
#[derive(Default)]
pub struct MaterialData {
    pub name: Option<String>,
    pub color: Option<Value>,
    pub texture: Option<Value>,
    pub opacity: Option<Value>,
    pub finish: Option<String>,
    pub pbr: Option<Value>,
}

pub enum Data {
    Record,
    Root { edit_path: Vec<(Key, Py<PyAny>)> },
    Placement(Box<PlacementData>),
    Tag(Box<TagData>),
    Material(Box<MaterialData>),
}

/// Borrowed pointer to the node's Python handle (null until a method with
/// the handle in hand records it). Valid while the node exists: the node
/// is freed by the handle's own `Drop`.
#[derive(Clone, Copy)]
pub struct HPtr(pub *mut ffi::PyObject);
unsafe impl Send for HPtr {}

/// Cached Python views a node hands out: the xform and axes matrices and
/// its list views (children, tags).
pub const VIEW_XFORM: usize = 0;
pub const VIEW_AXES: usize = 1;
/// The paint dict a placement's `material` read returned last.
pub const VIEW_MATERIAL: usize = 2;
/// The Python strings its string fields were last read as (a read is then
/// an incref, not a new `str`): slot by field, see [`str_view`].
pub const VIEW_STR0: usize = 3;
pub const N_VIEWS: usize = 6;

/// The view slot caching a string field's Python `str`, if it has one.
#[inline]
pub fn str_view(kind: Kind, f: Field) -> Option<usize> {
    match (kind, f) {
        (Kind::Placement, Field::Name) | (Kind::Tag, Field::Name) | (Kind::Material, Field::Name) => Some(VIEW_STR0),
        (Kind::Placement, Field::Uid) | (Kind::Tag, Field::Uid) => Some(VIEW_STR0 + 1),
        (Kind::Placement, Field::Tag) | (Kind::Tag, Field::Key) => Some(VIEW_STR0 + 2),
        _ => None,
    }
}

pub struct Node {
    pub kind: Kind,
    pub data: Data,
    pub children: Vec<Key>,
    pub assembly: Option<Arc<Mutex<assembly::Assembly>>>,
    pub tags: Vec<Key>,
    pub kept: Vec<Key>,
    pub mats: Vec<Key>,
    /// The registry name of each of `mats` (a dict's keys).
    pub mat_names: Vec<String>,
    /// The nodes whose lists name this one, one entry per occurrence.
    pub owners: Vec<Key>,
    pub handle: HPtr,
    pub strong: Option<Py<PyAny>>,
    pub views: [Option<Py<PyAny>>; N_VIEWS],
    /// The live `HandleList` of each list (children, tags), borrowed: the
    /// view holds its owner's handle (a list keeps what it lists alive),
    /// and clears this when it goes.
    pub list_views: [HPtr; 4],
    /// Named attributes ([`Field::Attr`]), set ones only.
    pub attrs: Vec<(u16, Value)>,
    /// The tracked Python container an attribute was last read as
    /// (`core.doc_model.TrackedDict`): edited in place it writes back.
    pub attr_views: Vec<(u16, Py<PyAny>)>,
}

impl Node {
    fn new(kind: Kind) -> Node {
        let data = match kind {
            Kind::Root => Data::Root { edit_path: Vec::new() },
            Kind::Placement => Data::Placement(Box::new(PlacementData { component: true, ..Default::default() })),
            Kind::Tag => Data::Tag(Box::new(TagData { visible: true, ..Default::default() })),
            Kind::Record => Data::Record,
            Kind::Material => Data::Material(Box::default()),
        };
        Node {
            kind,
            data,
            children: Vec::new(),
            assembly: None,
            tags: Vec::new(),
            kept: Vec::new(),
            mats: Vec::new(),
            mat_names: Vec::new(),
            owners: Vec::new(),
            handle: HPtr(std::ptr::null_mut()),
            strong: None,
            views: Default::default(),
            list_views: [HPtr(std::ptr::null_mut()); 4],
            attrs: Vec::new(),
            attr_views: Vec::new(),
        }
    }

    /// Forget the cached Python view of `f` (its value changed).
    #[inline]
    pub fn drop_view(&mut self, f: Field, trash: &mut Trash) {
        if let Field::Attr(a) = f {
            if let Some(i) = self.attr_views.iter().position(|(x, _)| *x == a) {
                trash.0.push(self.attr_views.swap_remove(i).1);
            }
            return;
        }
        let slot = match f {
            Field::Xform => Some(VIEW_XFORM),
            Field::Axes => Some(VIEW_AXES),
            Field::Material => Some(VIEW_MATERIAL),
            _ => str_view(self.kind, f),
        };
        if let Some(slot) = slot {
            trash.push(self.views[slot].take());
        }
    }

    pub fn list(&self, l: ListId) -> &Vec<Key> {
        match l {
            ListId::Children => &self.children,
            ListId::Tags => &self.tags,
            ListId::Kept => &self.kept,
            ListId::Mats => &self.mats,
        }
    }

    fn list_mut(&mut self, l: ListId) -> &mut Vec<Key> {
        match l {
            ListId::Children => &mut self.children,
            ListId::Tags => &mut self.tags,
            ListId::Kept => &mut self.kept,
            ListId::Mats => &mut self.mats,
        }
    }

    pub fn attr(&self, a: u16) -> Option<&Value> {
        self.attrs.iter().find(|(x, _)| *x == a).map(|(_, v)| v)
    }

    pub fn attr_view(&self, a: u16) -> Option<&Py<PyAny>> {
        self.attr_views.iter().find(|(x, _)| *x == a).map(|(_, v)| v)
    }

    pub fn get(&self, f: Field) -> Result<FVal, String> {
        if let Field::Attr(a) = f {
            return Ok(FVal::Val(self.attr(a).cloned()));
        }
        let bad = || Err(format!("a {} has no field {}", self.kind.name(), f.name()));
        Ok(match (&self.data, f) {
            (Data::Placement(p), Field::Definition) => FVal::Definition(p.definition.clone()),
            (Data::Placement(p), Field::Uid) => FVal::Str(p.uid.clone()),
            (Data::Placement(p), Field::Name) => FVal::Str(p.name.clone()),
            (Data::Placement(p), Field::Tag) => FVal::Str(p.tag.clone()),
            (Data::Placement(p), Field::Xform) => FVal::Xf(p.xform.clone()),
            (Data::Placement(p), Field::Axes) => FVal::Xf(p.axes.clone()),
            (Data::Placement(p), Field::Billboard) => FVal::Val(Some(p.billboard.as_deref().cloned().unwrap_or(Value::Bool(false)))),
            (Data::Placement(p), Field::Component) => FVal::Bool(p.component),
            (Data::Placement(p), Field::Hidden) => FVal::Bool(p.hidden),
            (Data::Placement(p), Field::Locked) => FVal::Bool(p.locked),
            (Data::Placement(p), Field::Material) => FVal::Val(p.material.clone()),
            (Data::Tag(t), Field::Uid) => FVal::Str(t.uid.clone()),
            (Data::Tag(t), Field::Name) => FVal::Str(t.name.clone()),
            (Data::Tag(t), Field::Key) => FVal::Str(t.key.clone()),
            (Data::Tag(t), Field::Visible) => FVal::Bool(t.visible),
            (Data::Tag(t), Field::Locked) => FVal::Bool(t.locked),
            (Data::Tag(t), Field::Color) => FVal::Val(t.color.clone()),
            (Data::Tag(t), Field::Folder) => FVal::Str(t.folder.clone()),
            (Data::Material(m), Field::Name) => FVal::Str(m.name.clone()),
            (Data::Material(m), Field::Color) => FVal::Val(m.color.clone()),
            (Data::Material(m), Field::Texture) => FVal::Val(m.texture.clone()),
            (Data::Material(m), Field::Opacity) => FVal::Val(m.opacity.clone()),
            (Data::Material(m), Field::Finish) => FVal::Str(m.finish.clone()),
            (Data::Material(m), Field::Pbr) => FVal::Val(m.pbr.clone()),
            _ => return bad(),
        })
    }

    /// Store `v` in `f`, returning what was there.
    fn put(&mut self, f: Field, v: FVal) -> Result<FVal, String> {
        if let Field::Attr(a) = f {
            let FVal::Val(v) = v else {
                return Err(format!("attribute {} holds values", attr_name(a)));
            };
            let i = self.attrs.iter().position(|(x, _)| *x == a);
            let old = match (i, v) {
                (Some(i), Some(v)) => Some(std::mem::replace(&mut self.attrs[i].1, v)),
                (Some(i), None) => Some(self.attrs.swap_remove(i).1),
                (None, Some(v)) => {
                    self.attrs.push((a, v));
                    None
                }
                (None, None) => None,
            };
            return Ok(FVal::Val(old));
        }
        let kind = self.kind;
        let bad = || Err(format!("a {} has no field {} of that type", kind.name(), f.name()));
        Ok(match (&mut self.data, f, v) {
            (Data::Placement(p), Field::Definition, FVal::Definition(d)) => FVal::Definition(std::mem::replace(&mut p.definition, d)),
            (Data::Placement(p), Field::Uid, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut p.uid, s)),
            (Data::Placement(p), Field::Name, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut p.name, s)),
            (Data::Placement(p), Field::Tag, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut p.tag, s)),
            (Data::Placement(p), Field::Xform, FVal::Xf(x)) => FVal::Xf(std::mem::replace(&mut p.xform, x)),
            (Data::Placement(p), Field::Axes, FVal::Xf(x)) => FVal::Xf(std::mem::replace(&mut p.axes, x)),
            (Data::Placement(p), Field::Billboard, FVal::Val(v)) => {
                let value = match v {
                    Some(Value::Bool(false)) => None,
                    value => Some(Box::new(value.unwrap_or(Value::None))),
                };
                let old = std::mem::replace(&mut p.billboard, value);
                FVal::Val(Some(old.map_or(Value::Bool(false), |v| *v)))
            }
            (Data::Placement(p), Field::Component, FVal::Bool(b)) => FVal::Bool(std::mem::replace(&mut p.component, b)),
            (Data::Placement(p), Field::Hidden, FVal::Bool(b)) => FVal::Bool(std::mem::replace(&mut p.hidden, b)),
            (Data::Placement(p), Field::Locked, FVal::Bool(b)) => FVal::Bool(std::mem::replace(&mut p.locked, b)),
            (Data::Placement(p), Field::Material, FVal::Val(x)) => FVal::Val(std::mem::replace(&mut p.material, x)),
            (Data::Tag(t), Field::Uid, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut t.uid, s)),
            (Data::Tag(t), Field::Name, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut t.name, s)),
            (Data::Tag(t), Field::Key, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut t.key, s)),
            (Data::Tag(t), Field::Visible, FVal::Bool(b)) => FVal::Bool(std::mem::replace(&mut t.visible, b)),
            (Data::Tag(t), Field::Locked, FVal::Bool(b)) => FVal::Bool(std::mem::replace(&mut t.locked, b)),
            (Data::Tag(t), Field::Color, FVal::Val(x)) => FVal::Val(std::mem::replace(&mut t.color, x)),
            (Data::Tag(t), Field::Folder, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut t.folder, s)),
            (Data::Material(m), Field::Name, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut m.name, s)),
            (Data::Material(m), Field::Color, FVal::Val(x)) => FVal::Val(std::mem::replace(&mut m.color, x)),
            (Data::Material(m), Field::Texture, FVal::Val(x)) => FVal::Val(std::mem::replace(&mut m.texture, x)),
            (Data::Material(m), Field::Opacity, FVal::Val(x)) => FVal::Val(std::mem::replace(&mut m.opacity, x)),
            (Data::Material(m), Field::Finish, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut m.finish, s)),
            (Data::Material(m), Field::Pbr, FVal::Val(x)) => FVal::Val(std::mem::replace(&mut m.pbr, x)),
            _ => return bad(),
        })
    }
}

pub struct Slot {
    pub gen: u32,
    pub node: Option<Node>,
}

/// One journalled change.
#[derive(Clone, Debug)]
pub enum Op {
    Set { k: Key, f: Field, old: FVal, new: FVal },
    /// One field set to one value over many nodes (a bulk write: Lock,
    /// Hide, Tag of a selection): one op, not one per node.
    SetMany { f: Field, keys: Vec<Key>, old: Vec<FVal>, new: FVal },
    Ins { owner: Key, list: ListId, at: usize, k: Key },
    Rem { owner: Key, list: ListId, at: usize, k: Key },
    Rep { owner: Key, list: ListId, old: Vec<Key>, new: Vec<Key> },
    /// A materials registry replaced (`scene.materials`): names and
    /// handles before and after. Writes in one transaction coalesce.
    Reg { owner: Key, old: Vec<(String, Key)>, new: Vec<(String, Key)> },
    /// One entry of attribute ``a`` of ``k`` (``path`` into its value)
    /// written: before and after (None: absent). A tracked container's
    /// keyed edit, journalled without copying the whole value.
    Patch { k: Key, a: u16, path: Vec<Value>, old: Option<Value>, new: Option<Value> },
}

/// The open transaction (one at a time; nested `begin`s are savepoints).
pub struct Open {
    document: Option<Key>,
    pub thread: ThreadId,
    pub ops: Vec<Op>,
    /// Start of each nested level's ops (`marks[0] == 0`).
    pub marks: Vec<usize>,
    /// `(node, field)` → index of its `Set` op, for coalescing; only
    /// entries at or past the innermost mark are coalesced into.
    pub index: FastMap<(Key, Field), usize>,
    pub pinned: FastSet<Key>,
    /// Nodes made while this transaction is open: their field writes are
    /// not journalled (nothing named them before; undo takes them out of
    /// the lists that hold them, and their fields keep the values redo and
    /// the change record need, a new tag's key for one).
    pub born: FastSet<Key>,
    pub pins: Vec<Py<PyAny>>,
}

/// Reserve the journal across callbacks, including gaps between compound parts.
pub(super) struct Writer;

impl Writer {
    fn acquire(py: Python<'_>, document: Option<Key>) -> PyResult<Self> {
        {
            let mut doc = lock();
            if doc.accepts_writer(document) {
                let entry = doc.writer.get_or_insert((std::thread::current().id(), document, 0));
                entry.1 = entry.1.or(document);
                entry.2 += 1;
                return Ok(Self);
            }
        }
        let reason = py.import("core.history_commands")?.getattr("CommandRejected")?
            .call1(("Another document transaction is active.",))?;
        Err(PyErr::from_value(reason))
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let mut doc = lock();
        let (_, _, count) = doc.writer.as_mut().expect("writer reserved");
        *count -= 1;
        if *count == 0 {
            doc.writer = None;
        }
    }
}

/// Python references to release once the lock is gone.
#[derive(Default)]
pub struct Trash(pub Vec<Py<PyAny>>, Vec<Arc<DefinitionRef>>);

impl Trash {
    fn push(&mut self, p: Option<Py<PyAny>>) {
        if let Some(p) = p {
            self.0.push(p);
        }
    }
}

#[derive(Default)]
pub struct Doc {
    writer: Option<(ThreadId, Option<Key>, usize)>,
    pub slots: Vec<Slot>,
    pub free: Vec<u32>,
    pub live: [usize; 5],
    pub open: Option<Open>,
    /// Every write (journalled or not) bumps this: a cheap "anything
    /// changed?" for caches keyed on the document.
    pub serial: u64,
    pub reader_epochs: FastMap<Key, u64>,
    /// Set while a journal is replayed (undo, redo, rollback): the tree
    /// invariant was checked when it was recorded.
    pub replaying: bool,
    dirty_assemblies: Vec<Key>,
    projecting: bool,
    implicit_assembly: bool,
    /// Tracked containers whose attribute a replay (undo, redo, rollback)
    /// wrote: kept as the attribute's view and refilled in place with the
    /// value once the lock is gone (`py::run_refills`), so a reference held
    /// across the undo (`entry = links.registry(scene)[path]`) sees it, as
    /// a Python command's own undo editing the dict in place showed it.
    pub refills: Vec<(Key, u16, Py<PyAny>, Value)>,
}

/// The arena's lock. Every caller holds the GIL and nothing calls into
/// Python while holding it (the lock rule above), so it is never contended
/// in practice: an atomic flag (a few ns) instead of a pthread mutex (most
/// of a field read's cost through the facade).
pub struct DocCell {
    locked: AtomicBool,
    doc: UnsafeCell<Option<Doc>>,
}

unsafe impl Sync for DocCell {}

static DOC: DocCell = DocCell { locked: AtomicBool::new(false), doc: UnsafeCell::new(None) };
/// Handles freed while the lock was held elsewhere (never in practice
/// under the lock rule; kept as a safety net). Drained by every op.
static PENDING: Mutex<Vec<Key>> = Mutex::new(Vec::new());

pub struct DocGuard(&'static DocCell);

impl Drop for DocGuard {
    fn drop(&mut self) {
        self.0.locked.store(false, Ordering::Release);
    }
}

impl std::ops::Deref for DocGuard {
    type Target = Doc;
    fn deref(&self) -> &Doc {
        // SAFETY: the flag is held by this guard.
        unsafe { (*self.0.doc.get()).as_ref().expect("document initialised") }
    }
}
impl std::ops::DerefMut for DocGuard {
    fn deref_mut(&mut self) -> &mut Doc {
        // SAFETY: the flag is held by this guard.
        unsafe { (*self.0.doc.get()).as_mut().expect("document initialised") }
    }
}

fn guard() -> DocGuard {
    let g = DocGuard(&DOC);
    // SAFETY: the flag is held.
    unsafe {
        let d = &mut *DOC.doc.get();
        if d.is_none() {
            *d = Some(Doc::default());
        }
    }
    g
}

pub fn lock() -> DocGuard {
    let mut spins = 0u32;
    while DOC.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        spins += 1;
        if spins < 64 {
            std::hint::spin_loop();
        } else {
            std::thread::yield_now();
        }
    }
    guard()
}

pub fn try_lock() -> Option<DocGuard> {
    if DOC.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
        Some(guard())
    } else {
        None
    }
}

/// Free the nodes whose handles went while the lock was busy.
pub fn drain_pending() {
    loop {
        let keys: Vec<Key> = {
            let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            if p.is_empty() {
                return;
            }
            std::mem::take(&mut *p)
        };
        let mut trash = Trash::default();
        {
            let mut d = lock();
            for k in keys {
                d.free_node(k, &mut trash);
            }
        }
        drop(trash);
    }
}

/// A handle's `Drop`: free its node (deferred when the lock is busy).
pub fn handle_dropped(k: Key) {
    let mut trash = Trash::default();
    match try_lock() {
        Some(mut d) => d.free_node(k, &mut trash),
        None => {
            PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(k);
            return;
        }
    }
    drop(trash);
}

impl Doc {
    pub fn alloc(&mut self, kind: Kind) -> Key {
        self.live[kind as usize] += 1;
        let node = Node::new(kind);
        let k = if let Some(slot) = self.free.pop() {
            let s = &mut self.slots[slot as usize];
            s.node = Some(node);
            make_key(slot, s.gen)
        } else {
            let slot = self.slots.len() as u32;
            self.slots.push(Slot { gen: 1, node: Some(node) });
            make_key(slot, 1)
        };
        if self.journal_open() {
            self.open.as_mut().expect("open").born.insert(k);
        }
        k
    }

    pub fn node(&self, k: Key) -> Option<&Node> {
        let s = self.slots.get(slot_of(k))?;
        if s.gen != gen_of(k) {
            return None;
        }
        s.node.as_ref()
    }

    pub fn node_mut(&mut self, k: Key) -> Option<&mut Node> {
        let s = self.slots.get_mut(slot_of(k))?;
        if s.gen != gen_of(k) {
            return None;
        }
        s.node.as_mut()
    }

    pub fn need(&self, k: Key) -> Result<&Node, String> {
        self.node(k).ok_or_else(|| format!("no document node {k:#x}"))
    }

    pub fn set_handle(&mut self, k: Key, ptr: *mut ffi::PyObject) {
        if let Some(n) = self.node_mut(k) {
            if n.handle.0.is_null() {
                n.handle = HPtr(ptr);
            }
        }
    }

    /// A new strong reference to `k`'s handle (an incref: allowed under
    /// the lock).
    pub fn handle_ref(&self, py: Python<'_>, k: Key) -> Option<Py<PyAny>> {
        let n = self.node(k)?;
        if n.handle.0.is_null() {
            return None;
        }
        Some(unsafe { Bound::from_borrowed_ptr(py, n.handle.0) }.unbind())
    }

    fn free_node(&mut self, k: Key, trash: &mut Trash) {
        let slot = slot_of(k);
        let Some(s) = self.slots.get_mut(slot) else { return };
        if s.gen != gen_of(k) || s.node.is_none() {
            return;
        }
        let mut node = s.node.take().expect("checked");
        // A slot is never reused at a generation a key already named.
        s.gen = s.gen.wrapping_add(1).max(1);
        if s.gen != u32::MAX {
            self.free.push(slot as u32);
        }
        if node.kind == Kind::Root { self.reader_epochs.remove(&k); }
        self.live[node.kind as usize] -= 1;
        self.serial += 1;
        for v in node.views.iter_mut() {
            trash.push(v.take());
        }
        for (_, v) in node.attr_views.drain(..) {
            trash.0.push(v);
        }
        if let Data::Placement(p) = &mut node.data {
            if let Some(def) = p.definition.take() {
                def.0.get().placements.lock().unwrap().remove(&k);
                trash.1.push(def);
            }
        }
        if let Data::Root { edit_path } = &mut node.data {
            trash.0.extend(edit_path.drain(..).map(|(_, level)| level));
        }
        trash.push(node.strong.take());
        // Members of its lists lose an owner.
        node.mat_names.clear();
        for l in LISTS {
            for c in std::mem::take(node.list_mut(l)) {
                self.drop_owner(c, k, trash);
            }
        }
        // A handle never dies while a list names it (the list holds it);
        // after a GC clear the owner bookkeeping is already gone.
        debug_assert!(node.owners.is_empty() || true);
    }

    fn add_owner(&mut self, py: Python<'_>, k: Key, owner: Key) {
        let first = {
            let n = self.node_mut(k).expect("member exists");
            n.owners.push(owner);
            n.owners.len() == 1 && n.strong.is_none()
        };
        if first {
            let h = self.handle_ref(py, k);
            self.node_mut(k).expect("member exists").strong = h;
        }
    }

    fn drop_owner(&mut self, k: Key, owner: Key, trash: &mut Trash) {
        if let Some(n) = self.node_mut(k) {
            if let Some(i) = n.owners.iter().position(|&o| o == owner) {
                n.owners.remove(i);
            }
            if n.owners.is_empty() {
                trash.push(n.strong.take());
            }
        }
    }

    // ---------------------------------------------------------- journal

    fn journal_open(&self) -> bool {
        match &self.open {
            Some(o) => o.thread == std::thread::current().id(),
            None => false,
        }
    }

    fn pin(&mut self, py: Python<'_>, k: Key) {
        let open = self.open.as_ref().expect("journal open");
        if open.pinned.contains(&k) {
            return;
        }
        let h = self.handle_ref(py, k);
        let open = self.open.as_mut().expect("journal open");
        open.pinned.insert(k);
        if let Some(h) = h {
            open.pins.push(h);
        }
    }

    fn accepts_writer(&self, document: Option<Key>) -> bool {
        let matches = |thread: ThreadId, owner: Option<Key>| {
            thread == std::thread::current().id()
                && (document.is_none() || owner.is_none() || owner == document)
        };
        self.open.as_ref().is_none_or(|o| matches(o.thread, o.document))
            && self.writer.as_ref().is_none_or(|(thread, owner, _)| matches(*thread, *owner))
    }

    pub fn begin(&mut self, document: Option<Key>) -> Option<usize> {
        if !self.accepts_writer(document) {
            return None;
        }
        let here = std::thread::current().id();
        match &mut self.open {
            Some(o) => {
                o.document = o.document.or(document);
                o.marks.push(o.ops.len());
                Some(o.marks.len())
            }
            None => {
                self.open = Some(Open {
                    thread: here,
                    document,
                    ops: Vec::new(),
                    marks: vec![0],
                    index: FastMap::default(),
                    pinned: FastSet::default(),
                    born: FastSet::default(),
                    pins: Vec::new(),
                });
                Some(1)
            }
        }
    }

    pub fn depth(&self) -> usize {
        match &self.open {
            Some(o) if o.thread == std::thread::current().id() => o.marks.len(),
            _ => 0,
        }
    }

    /// Close the innermost level. The outermost returns the transaction.
    pub fn commit(&mut self) -> Option<(Vec<Op>, Vec<Py<PyAny>>)> {
        if !self.journal_open() {
            return None;
        }
        let o = self.open.as_mut().expect("open");
        o.marks.pop();
        if !o.marks.is_empty() {
            // The coalescing index may point into the closed level: it now
            // belongs to the outer one, which may coalesce into it too.
            return None;
        }
        let o = self.open.take().expect("open");
        let ops = o
            .ops
            .into_iter()
            .filter_map(|op| match op {
                Op::Set { ref old, ref new, .. } if old == new => None,
                Op::Reg { ref old, ref new, .. } if old == new => None,
                Op::Patch { ref old, ref new, .. } if old == new => None,
                Op::SetMany { f, keys, old, new } => {
                    let (keys, old): (Vec<Key>, Vec<FVal>) =
                        keys.into_iter().zip(old).filter(|(_, o)| *o != new).unzip();
                    if keys.is_empty() { None } else { Some(Op::SetMany { f, keys, old, new }) }
                }
                op => Some(op),
            })
            .collect();
        Some((ops, o.pins))
    }

    /// Undo the innermost level's ops and close it.
    pub fn rollback(&mut self, py: Python<'_>, trash: &mut Trash) -> Result<(), String> {
        if !self.journal_open() {
            return Ok(());
        }
        let (start, outer) = {
            let o = self.open.as_mut().expect("open");
            let start = o.marks.pop().unwrap_or(0);
            (start, !o.marks.is_empty())
        };
        let undo: Vec<Op> = {
            let o = self.open.as_mut().expect("open");
            let tail: Vec<Op> = o.ops.drain(start..).collect();
            o.index.retain(|_, i| *i < start);
            tail
        };
        // Applied without journalling: the level is gone.
        let saved = self.open.take();
        let r = self.apply(py, &undo, true, trash);
        self.open = saved;
        if !outer {
            self.dirty_assemblies.clear();
            if let Some(o) = self.open.take() {
                trash.0.extend(o.pins);
            }
        }
        r
    }

    /// Apply `ops` backwards (undo) or forwards (redo). Journalled when a
    /// transaction is open on this thread.
    pub fn apply(&mut self, py: Python<'_>, ops: &[Op], backwards: bool, trash: &mut Trash) -> Result<(), String> {
        let was = std::mem::replace(&mut self.replaying, true);
        let r = self.apply_ops(py, ops, backwards, trash);
        self.replaying = was;
        r
    }

    fn apply_ops(&mut self, py: Python<'_>, ops: &[Op], backwards: bool, trash: &mut Trash) -> Result<(), String> {
        let mut run = |d: &mut Doc, op: &Op| -> Result<(), String> {
            match (op, backwards) {
                (Op::Set { k, f, old, new }, undo) => {
                    let (from, to) = if undo { (new, old) } else { (old, new) };
                    let node = d.need(*k)?;
                    let value = match (node.kind, f, from, to) {
                        (Kind::Root, Field::Attr(id), FVal::Val(Some(before)), FVal::Val(Some(after))) => {
                            match node.attr(*id) {
                                Some(current) => FVal::Val(Some(value::replay_table(current, before, after))),
                                None => to.clone(),
                            }
                        }
                        _ => to.clone(),
                    };
                    d.write(py, *k, *f, value, trash).map(|_| ())
                }
                (Op::Patch { k, a, path, old, new }, undo) => {
                    d.patch(py, *k, *a, path, if undo { old.clone() } else { new.clone() })
                }
                (Op::SetMany { f, keys, old, .. }, true) => {
                    for (k, o) in keys.iter().zip(old.iter()).rev() {
                        d.write(py, *k, *f, o.clone(), trash)?;
                    }
                    Ok(())
                }
                (Op::SetMany { f, keys, new, .. }, false) => d.write_many(py, keys, *f, new, trash),
                (Op::Ins { owner, list, at, k }, true) | (Op::Rem { owner, list, at, k }, false) => {
                    let got = d.list_remove(py, *owner, *list, *at, trash)?;
                    if got != *k {
                        return Err(format!("journal out of step: {got:#x} at {at}, expected {k:#x}"));
                    }
                    Ok(())
                }
                (Op::Ins { owner, list, at, k }, false) | (Op::Rem { owner, list, at, k }, true) => {
                    d.list_insert(py, *owner, *list, *at, *k)
                }
                (Op::Rep { owner, list, old, .. }, true) => d.list_replace(py, *owner, *list, old.clone(), trash),
                (Op::Rep { owner, list, new, .. }, false) => d.list_replace(py, *owner, *list, new.clone(), trash),
                (Op::Reg { owner, old, .. }, true) => d.registry_replace(py, *owner, old.clone(), trash),
                (Op::Reg { owner, new, .. }, false) => d.registry_replace(py, *owner, new.clone(), trash),
            }
        };
        if backwards {
            for op in ops.iter().rev() {
                run(self, op)?;
            }
        } else {
            for op in ops {
                run(self, op)?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ writes

    /// Write a field; cached views of the old value go to `trash`.
    fn assembly_changed(&mut self, owner: Key) {
        if self.replaying || self.projecting { return; }
        let mut current = Some(owner);
        let mut remaining = self.slots.len();
        while let Some(k) = current {
            let Some(n) = self.node(k) else { break };
            if remaining == 0 { break; }
            remaining -= 1;
            current = n.owners.first().copied();
            if n.assembly.is_some() && !self.dirty_assemblies.contains(&k) {
                self.dirty_assemblies.push(k);
            }
        }
    }

    /// Track writes to reachable model data. Constructing a detached paste
    /// candidate or releasing a temporary record cannot invalidate a panel.
    fn note_reader_write(&mut self, mut key: Key) {
        if self.reader_epochs.is_empty() { return; }
        // The usual case is a single parent. Shared ownership can occur in
        // compatibility mode, including cycles, so bound both traversals.
        for _ in 0..self.slots.len() {
            if let Some(epoch) = self.reader_epochs.get_mut(&key) {
                *epoch = self.serial;
                return;
            }
            let Some(node) = self.node(key) else { return };
            match node.owners.as_slice() {
                [] => return,
                [parent] => key = *parent,
                parents => {
                    let mut pending = parents.to_vec();
                    let mut seen = FastSet::default();
                    while let Some(k) = pending.pop() {
                        if !seen.insert(k) { continue; }
                        if let Some(epoch) = self.reader_epochs.get_mut(&k) { *epoch = self.serial; }
                        else if let Some(n) = self.node(k) { pending.extend_from_slice(&n.owners); }
                    }
                    return;
                }
            }
        }
    }

    pub fn write(&mut self, py: Python<'_>, k: Key, f: Field, v: FVal, trash: &mut Trash) -> Result<FVal, String> {
        let layout_parent = if matches!(f, Field::Definition | Field::Xform | Field::Axes | Field::Component) {
            self.node(k).and_then(|n| n.owners.first().copied())
        } else { None };
        if let Some(parent) = layout_parent {
            assembly::prepare(self, parent);
        }
        let journal = self.journal_open() && !self.open.as_ref().is_some_and(|o| o.born.contains(&k));
        let new = if journal { Some(v.clone()) } else { None };
        let changed = match (&f, &v) {
            (Field::Attr(a), FVal::Val(value)) => self.node(k).and_then(|n| n.attr(*a)) != value.as_ref(),
            _ => true,
        };
        let replaying = self.replaying;
        let mut refill = None;
        let old = {
            let n = self.node_mut(k).ok_or_else(|| format!("no document node {k:#x}"))?;
            let keep = match (&f, &v) {
                (Field::Attr(a), FVal::Val(Some(val @ (Value::Dict(_) | Value::List(_))))) if replaying => {
                    n.attr_view(*a).map(|view| (*a, view.clone_ref(py), val.clone()))
                }
                _ => None,
            };
            let old = n.put(f, v)?;
            match keep {
                Some(r) => refill = Some(r),
                None => n.drop_view(f, trash),
            }
            old
        };
        if let Some((a, view, val)) = refill {
            self.refills.push((k, a, view, val));
        }
        if let FVal::Definition(previous) = &old {
            if let Some(def) = previous {
                def.0.get().placements.lock().unwrap().remove(&k);
                trash.1.push(def.clone());
            }
            if let Data::Placement(p) = &self.node(k).unwrap().data {
                if let Some(def) = &p.definition {
                    def.0.get().placements.lock().unwrap().insert(k);
                }
            }
        }
        if let Some(parent) = layout_parent {
            self.assembly_changed(parent);
        }
        self.serial += 1;
        if changed { self.note_reader_write(k); }
        if let Some(new) = new {
            self.pin(py, k);
            let o = self.open.as_mut().expect("open");
            let floor = *o.marks.last().unwrap_or(&0);
            match o.index.get(&(k, f)) {
                Some(&i) if i >= floor => {
                    if let Op::Set { new: n, .. } = &mut o.ops[i] {
                        *n = new;
                    }
                }
                _ => {
                    o.index.insert((k, f), o.ops.len());
                    o.ops.push(Op::Set { k, f, old: old.clone(), new });
                }
            }
        }
        Ok(old)
    }

    /// Like `write`, keeping the cached matrix view (the view itself was
    /// edited in place and is the new value's face).
    pub fn write_keep_view(&mut self, py: Python<'_>, k: Key, f: Field, v: FVal, trash: &mut Trash) -> Result<(), String> {
        if let Field::Attr(a) = f {
            let view = self.node_mut(k).and_then(|n| {
                let i = n.attr_views.iter().position(|(x, _)| *x == a)?;
                Some(n.attr_views.swap_remove(i))
            });
            let r = self.write(py, k, f, v, trash);
            if let (Some(view), Some(n)) = (view, self.node_mut(k)) {
                n.attr_views.push(view);
            }
            return r.map(|_| ());
        }
        let slot = match f {
            Field::Xform => VIEW_XFORM,
            Field::Axes => VIEW_AXES,
            _ => return self.write(py, k, f, v, trash).map(|_| ()),
        };
        let view = self.node_mut(k).and_then(|n| n.views[slot].take());
        let r = self.write(py, k, f, v, trash);
        if let Some(n) = self.node_mut(k) {
            n.views[slot] = view;
        }
        r.map(|_| ())
    }

    /// Put ``new`` at ``path`` in attribute ``a`` of ``k`` (None: take the
    /// entry out), journalled as an ``Op::Patch``. The Python view, which
    /// made the edit, is kept; replaying (undo, redo, rollback) refills it.
    pub fn patch(&mut self, py: Python<'_>, k: Key, a: u16, path: &[Value], new: Option<Value>) -> Result<(), String> {
        let journal = self.journal_open() && !self.open.as_ref().is_some_and(|o| o.born.contains(&k));
        let replaying = self.replaying;
        let (old, refill) = {
            let n = self.node_mut(k).ok_or_else(|| format!("no document node {k:#x}"))?;
            let i = n.attrs.iter().position(|(x, _)| *x == a)
                .ok_or_else(|| format!("attribute {} is not set", attr_name(a)))?;
            let old = value::put_at(&mut n.attrs[i].1, path, new.clone())?;
            if old == new {
                return Ok(());
            }
            let refill = if replaying {
                n.attr_view(a).map(|view| (a, view.clone_ref(py), n.attrs[i].1.clone()))
            } else { None };
            (old, refill)
        };
        if let Some((a, view, val)) = refill {
            self.refills.push((k, a, view, val));
        }
        self.serial += 1;
        self.note_reader_write(k);
        if journal {
            self.pin(py, k);
            let o = self.open.as_mut().expect("open");
            // A later whole write must not coalesce into one made before
            // this patch: replayed forwards it would come first.
            o.index.remove(&(k, Field::Attr(a)));
            o.ops.push(Op::Patch { k, a, path: path.to_vec(), old, new });
        }
        Ok(())
    }

    /// Write one value into `f` of every node of `keys`: journalled as one
    /// `SetMany` op (nodes born in this transaction are left out of it).
    pub fn write_many(&mut self, py: Python<'_>, keys: &[Key], f: Field, v: &FVal, trash: &mut Trash) -> Result<(), String> {
        for &k in keys {
            self.need(k)?.get(f)?;
        }
        let journal = self.journal_open();
        let mut jkeys = Vec::new();
        let mut olds = Vec::new();
        for &k in keys {
            let n = self.node_mut(k).expect("checked");
            let old = n.put(f, v.clone())?;
            n.drop_view(f, trash);
            if journal && !self.open.as_ref().is_some_and(|o| o.born.contains(&k)) {
                jkeys.push(k);
                olds.push(old);
            }
        }
        self.serial += 1;
        for &k in keys { self.note_reader_write(k); }
        if !jkeys.is_empty() {
            for &k in &jkeys {
                self.pin(py, k);
            }
            let o = self.open.as_mut().expect("open");
            // A later single write of one of these nodes must not coalesce
            // into an op older than this one.
            if !o.index.is_empty() {
                for &k in &jkeys {
                    o.index.remove(&(k, f));
                }
            }
            o.ops.push(Op::SetMany { f, keys: jkeys, old: olds, new: v.clone() });
        }
        Ok(())
    }

    fn check_member(&self, owner: Key, list: ListId, k: Key) -> Result<(), String> {
        let o = self.need(owner)?;
        let ok_owner = list.owned_by(o.kind);
        if !ok_owner {
            return Err(format!("a {} has no such list", o.kind.name()));
        }
        let m = self.need(k)?;
        if m.kind != list.member() {
            return Err(format!("this list holds {}s, not a {}", list.member().name(), m.kind.name()));
        }
        Ok(())
    }

    fn journal_list(&mut self, py: Python<'_>, op: Op, touched: &[Key]) {
        if !self.journal_open() {
            return;
        }
        for &k in touched {
            self.pin(py, k);
        }
        self.open.as_mut().expect("open").ops.push(op);
    }

    /// A placement, named for a message.
    fn label(&self, k: Key) -> String {
        match self.node(k).map(|n| &n.data) {
            Some(Data::Placement(p)) => format!("{:?} ({})", p.name.as_deref().unwrap_or(""), p.uid.as_deref().unwrap_or("")),
            Some(_) => "the scene".to_owned(),
            None => format!("{k:#x}"),
        }
    }

    /// The tree invariant for `adding` going into `owner`'s `list`, which
    /// held `old` (a replace) — see [`PARENT_MODE`].
    fn one_parent(&self, owner: Key, list: ListId, adding: &[Key], old: Option<&[Key]>) -> Result<(), String> {
        let mode = PARENT_MODE.load(Ordering::Relaxed);
        if mode == 0 || self.replaying || !matches!(list, ListId::Children | ListId::Kept) {
            return Ok(());
        }
        let mut bad: Option<(Key, Option<Key>)> = None;
        let mut added = FastSet::default();
        let mut prior_counts = FastMap::default();
        for &key in old.unwrap_or_default() {
            *prior_counts.entry(key).or_insert(0usize) += 1;
        }
        for &k in adding {
            if adding.len() > 1 && !added.insert(k) {
                bad = Some((k, Some(owner)));
                break;
            }
            let Some(n) = self.node(k) else { continue };
            let mut prior = prior_counts.get(&k).copied().unwrap_or(0);
            if let Some(&other) = n.owners.iter().find(|&&parent| {
                if parent == owner && prior > 0 {
                    prior -= 1;
                    false
                } else {
                    true
                }
            }) {
                bad = Some((k, Some(other)));
                break;
            }
        }
        let Some((k, other)) = bad else { return Ok(()) };
        let msg = format!(
            "one parent per placement: {} is already in the list of {} (take it out before putting it in {})",
            self.label(k),
            other.map_or_else(|| "another".to_owned(), |o| self.label(o)),
            self.label(owner)
        );
        if mode >= 2 {
            return Err(msg);
        }
        PARENT_REPORTS.lock().unwrap_or_else(|e| e.into_inner()).push(msg);
        Ok(())
    }

    pub fn list_insert(&mut self, py: Python<'_>, owner: Key, list: ListId, at: usize, k: Key) -> Result<(), String> {
        self.check_member(owner, list, k)?;
        self.one_parent(owner, list, &[k], None)?;
        if list == ListId::Children { assembly::prepare(self, owner); }
        let at = {
            let l = self.node_mut(owner).expect("checked").list_mut(list);
            let at = at.min(l.len());
            l.insert(at, k);
            at
        };
        self.add_owner(py, k, owner);
        if list == ListId::Children { self.assembly_changed(owner); }
        self.serial += 1;
        self.note_reader_write(owner);
        self.journal_list(py, Op::Ins { owner, list, at, k }, &[owner, k]);
        Ok(())
    }

    pub fn list_remove(&mut self, py: Python<'_>, owner: Key, list: ListId, at: usize, trash: &mut Trash) -> Result<Key, String> {
        if list == ListId::Children { assembly::prepare(self, owner); }
        let k = {
            let n = self.node_mut(owner).ok_or_else(|| format!("no document node {owner:#x}"))?;
            let l = n.list_mut(list);
            if at >= l.len() {
                return Err("list index out of range".into());
            }
            l.remove(at)
        };
        // Pin before the owner goes: the journal must keep it alive.
        self.journal_list(py, Op::Rem { owner, list, at, k }, &[owner, k]);
        self.drop_owner(k, owner, trash);
        if list == ListId::Children { self.assembly_changed(owner); }
        self.serial += 1;
        self.note_reader_write(owner);
        Ok(k)
    }

    pub fn list_replace(&mut self, py: Python<'_>, owner: Key, list: ListId, new: Vec<Key>, trash: &mut Trash) -> Result<(), String> {
        for &k in &new {
            self.check_member(owner, list, k)?;
        }
        if new.is_empty() {
            self.need(owner)?;
        }
        {
            let old = self.need(owner)?.list(list).clone();
            self.one_parent(owner, list, &new, Some(&old))?;
        }
        if list == ListId::Children { assembly::prepare(self, owner); }
        let old = std::mem::replace(self.node_mut(owner).expect("checked").list_mut(list), new.clone());
        if old == new {
            return Ok(());
        }
        let mut touched = Vec::with_capacity(old.len() + new.len() + 1);
        touched.push(owner);
        touched.extend_from_slice(&old);
        touched.extend_from_slice(&new);
        self.journal_list(py, Op::Rep { owner, list, old: old.clone(), new: new.clone() }, &touched);
        for &k in &new {
            self.add_owner(py, k, owner);
        }
        for &k in &old {
            self.drop_owner(k, owner, trash);
        }
        if list == ListId::Children { self.assembly_changed(owner); }
        self.serial += 1;
        self.note_reader_write(owner);
        Ok(())
    }

    /// The registry of root `owner` as `(name, material)` pairs.
    pub fn registry(&self, owner: Key) -> Result<Vec<(String, Key)>, String> {
        let n = self.need(owner)?;
        Ok(n.mat_names.iter().cloned().zip(n.mats.iter().copied()).collect())
    }

    /// Replace the materials registry of root `owner` (journalled; the
    /// writes of one transaction level coalesce into one op).
    pub fn registry_replace(&mut self, py: Python<'_>, owner: Key, new: Vec<(String, Key)>, trash: &mut Trash) -> Result<(), String> {
        for (_, k) in &new {
            self.check_member(owner, ListId::Mats, *k)?;
        }
        if new.is_empty() {
            self.check_member_owner(owner, ListId::Mats)?;
        }
        let old = self.registry(owner)?;
        if old == new {
            return Ok(());
        }
        {
            let n = self.node_mut(owner).expect("checked");
            n.mat_names = new.iter().map(|(s, _)| s.clone()).collect();
            n.mats = new.iter().map(|(_, k)| *k).collect();
        }
        if self.journal_open() && !self.open.as_ref().is_some_and(|o| o.born.contains(&owner)) {
            self.pin(py, owner);
            for (_, k) in old.iter().chain(new.iter()) {
                self.pin(py, *k);
            }
            let o = self.open.as_mut().expect("open");
            let floor = *o.marks.last().unwrap_or(&0);
            let ix = (owner, Field::Attr(u16::MAX));
            match o.index.get(&ix) {
                Some(&i) if i >= floor => {
                    if let Op::Reg { new: n, .. } = &mut o.ops[i] {
                        *n = new.clone();
                    }
                }
                _ => {
                    o.index.insert(ix, o.ops.len());
                    o.ops.push(Op::Reg { owner, old: old.clone(), new: new.clone() });
                }
            }
        }
        for (_, k) in &new {
            self.add_owner(py, *k, owner);
        }
        for (_, k) in &old {
            self.drop_owner(*k, owner, trash);
        }
        self.serial += 1;
        self.note_reader_write(owner);
        Ok(())
    }

    fn check_member_owner(&self, owner: Key, list: ListId) -> Result<(), String> {
        let o = self.need(owner)?;
        if !list.owned_by(o.kind) {
            return Err(format!("a {} has no such list", o.kind.name()));
        }
        Ok(())
    }

    /// Take placement `k` out of every list that holds it (a group's
    /// children, a scene's top level or kept definitions), journalled:
    /// what `Group.adopt` does first, so adopting keeps one parent.
    pub fn detach(&mut self, py: Python<'_>, k: Key, trash: &mut Trash) -> Result<usize, String> {
        let owners = self.need(k)?.owners.clone();
        let mut n = 0;
        for owner in owners {
            for list in [ListId::Children, ListId::Kept] {
                while let Some(at) = self.node(owner).and_then(|o| o.list(list).iter().position(|&x| x == k)) {
                    self.list_remove(py, owner, list, at, trash)?;
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// GC `tp_clear` of a handle: release what its node holds.
    pub fn clear_node(&mut self, k: Key, trash: &mut Trash) {
        let lists = match self.node_mut(k) {
            Some(n) => {
                for v in n.views.iter_mut() {
                    trash.push(v.take());
                }
                for (_, v) in n.attr_views.drain(..) {
                    trash.0.push(v);
                }
                if let Data::Placement(p) = &mut n.data {
                    if let Some(def) = p.definition.take() {
                        def.0.get().placements.lock().unwrap().remove(&k);
                        trash.1.push(def);
                    }
                }
                if let Data::Root { edit_path } = &mut n.data {
                    trash.0.extend(edit_path.drain(..).map(|(_, level)| level));
                }
                n.mat_names.clear();
                [
                    std::mem::take(&mut n.children),
                    std::mem::take(&mut n.tags),
                    std::mem::take(&mut n.kept),
                    std::mem::take(&mut n.mats),
                ]
            }
            None => return,
        };
        for l in lists {
            for c in l {
                self.drop_owner(c, k, trash);
            }
        }
        self.serial += 1;
    }
}

// ---------------------------------------------------------------- changes

impl Doc {
    /// The placements whose `fields` (any field when empty) `ops` changed,
    /// or whose child list they edited when `lists` is set: each once, in
    /// the order first written. What the renderer re-validates after a
    /// step (docs/rust-document-model.md §5).
    pub fn written_placements(&self, ops: &[Op], fields: &[Field], lists: bool) -> Vec<Key> {
        let mut seen: FastSet<Key> = FastSet::default();
        let mut out = Vec::new();
        let want = |f: &Field| fields.is_empty() || fields.contains(f);
        let mut add = |k: Key, out: &mut Vec<Key>| {
            if self.node(k).is_some_and(|n| n.kind == Kind::Placement) && seen.insert(k) {
                out.push(k);
            }
        };
        for op in ops {
            match op {
                Op::Set { k, f, old, new } if old != new && want(f) => add(*k, &mut out),
                Op::Patch { k, a, old, new, .. } if old != new && want(&Field::Attr(*a)) => add(*k, &mut out),
                Op::SetMany { f, keys, old, new } if want(f) => {
                    for (k, o) in keys.iter().zip(old) {
                        if o != new {
                            add(*k, &mut out);
                        }
                    }
                }
                Op::Ins { owner, k, .. } | Op::Rem { owner, k, .. } if lists => {
                    add(*owner, &mut out);
                    add(*k, &mut out);
                }
                Op::Rep { owner, old, new, .. } if lists => {
                    add(*owner, &mut out);
                    for k in old.iter().chain(new.iter()) {
                        add(*k, &mut out);
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// What a transaction did, by stable keys (`docs/core-protocol.md` §4):
/// the nodes, each named once by its uid (placements), key (tags) or name
/// (materials); [`Doc::names`] gives the strings.
#[derive(Default, Debug)]
pub struct Changes {
    pub created: Vec<Key>,
    pub removed: Vec<Key>,
    pub modified: Vec<Key>,
    pub tags_created: Vec<Key>,
    pub tags_removed: Vec<Key>,
    pub tags_modified: Vec<Key>,
    pub materials_modified: Vec<Key>,
    /// Attributes written on the scene root (tag folders, the active tag,
    /// plugin data), by name.
    pub records_modified: Vec<Key>,
    pub scene_modified: Vec<&'static str>,
    /// Registry names added, taken out, or given another material.
    pub materials_created: Vec<String>,
    pub materials_removed: Vec<String>,
    pub materials_replaced: Vec<String>,
}

impl Node {
    /// The name the change record gives this node (uid, tag key, material
    /// name) and the view slot caching its Python `str`.
    pub fn record_name(&self) -> Option<(&str, usize)> {
        match &self.data {
            Data::Placement(p) => p.uid.as_deref().map(|u| (u, VIEW_STR0 + 1)),
            Data::Tag(t) => t.key.as_deref().map(|k| (k, VIEW_STR0 + 2)),
            Data::Material(m) => m.name.as_deref().map(|n| (n, VIEW_STR0)),
            Data::Root { .. } | Data::Record => None,
        }
    }
}

impl Doc {
    /// `keys` that have a record name, keeping the first of each name
    /// (`seen`: names already taken by another list of the record).
    fn unique_named<'a>(&'a self, keys: impl IntoIterator<Item = Key>, seen: &mut FastSet<&'a str>) -> Vec<Key> {
        let mut out = Vec::new();
        for k in keys {
            if let Some((name, _)) = self.node(k).and_then(|n| n.record_name()) {
                if seen.insert(name) {
                    out.push(k);
                }
            }
        }
        out
    }

    /// The record of `ops`; `root` limits created/removed placements and
    /// tags to that scene root's lists (None: every root).
    pub fn changes(&self, ops: &[Op], inverse: bool, root: Option<Key>) -> Changes {
        let mut net: FastMap<(ListId, Key), i64> = FastMap::default();
        let mut order: Vec<(ListId, Key)> = Vec::new();
        let mut modified: Vec<Key> = Vec::new();
        let mut tags_mod: Vec<Key> = Vec::new();
        let mut mats_mod: Vec<Key> = Vec::new();
        let mut bump = |l: ListId, k: Key, by: i64, net: &mut FastMap<(ListId, Key), i64>| {
            let e = net.entry((l, k)).or_insert_with(|| {
                order.push((l, k));
                0
            });
            *e += by;
        };
        let mut records = Vec::new();
        let mut seen_records = FastSet::default();
        let mut scene_mod: Vec<&'static str> = Vec::new();
        let mut field_written = |k: Key, f: Field, modified: &mut Vec<Key>, tags_mod: &mut Vec<Key>, mats_mod: &mut Vec<Key>| {
            match self.node(k).map(|n| n.kind) {
                Some(Kind::Placement) => modified.push(k),
                Some(Kind::Record) if seen_records.insert(k) => records.push(k),
                Some(Kind::Tag) => tags_mod.push(k),
                Some(Kind::Material) => mats_mod.push(k),
                Some(Kind::Root) if root.is_none_or(|r| r == k) => {
                    if let Field::Attr(a) = f {
                        let name = attr_name(a);
                        if !scene_mod.contains(&name) {
                            scene_mod.push(name);
                        }
                    }
                }
                _ => {}
            }
        };
        // Only the model's placements and the tag list are named as created
        // and removed (kept definitions are not in the model).
        let counted = |l: ListId| matches!(l, ListId::Children | ListId::Tags);
        let mut reg: FastMap<Key, Vec<(String, Key)>> = FastMap::default();
        let mut reg_new: FastMap<Key, Vec<(String, Key)>> = FastMap::default();
        for op in ops {
            match op {
                Op::Set { k, f, old, new } if old != new => field_written(*k, *f, &mut modified, &mut tags_mod, &mut mats_mod),
                Op::Set { .. } => {}
                Op::Patch { k, a, old, new, .. } if old != new =>
                    field_written(*k, Field::Attr(*a), &mut modified, &mut tags_mod, &mut mats_mod),
                Op::Patch { .. } => {}
                Op::SetMany { f, keys, old, new } => {
                    for (k, o) in keys.iter().zip(old) {
                        if o != new {
                            field_written(*k, *f, &mut modified, &mut tags_mod, &mut mats_mod);
                        }
                    }
                }
                Op::Ins { owner, list, k, .. } | Op::Rem { owner, list, k, .. } => {
                    let by = if matches!(op, Op::Ins { .. }) { 1 } else { -1 };
                    match self.node(*owner).map(|n| n.kind) {
                        Some(Kind::Root) if counted(*list) && root.is_none_or(|r| r == *owner) => bump(*list, *k, by, &mut net),
                        Some(Kind::Placement) => modified.push(*owner),
                        _ => {}
                    }
                }
                Op::Reg { owner, old, new } => {
                    if root.is_none_or(|r| r == *owner) {
                        reg.entry(*owner).or_insert_with(|| old.clone());
                        reg_new.insert(*owner, new.clone());
                    }
                }
                Op::Rep { owner, list, old, new } => match self.node(*owner).map(|n| n.kind) {
                    Some(Kind::Root) if counted(*list) && root.is_none_or(|r| r == *owner) => {
                        for k in old {
                            bump(*list, *k, -1, &mut net);
                        }
                        for k in new {
                            bump(*list, *k, 1, &mut net);
                        }
                    }
                    Some(Kind::Placement) => modified.push(*owner),
                    _ => {}
                },
            }
        }
        let mut lists: [Vec<Key>; 4] = Default::default();
        for (l, k) in order {
            let n = net[&(l, k)];
            if n == 0 {
                continue;
            }
            let added = (n > 0) != inverse;
            let i = match (l, added) {
                (ListId::Children, true) => 0,
                (ListId::Children, false) => 1,
                (ListId::Tags, true) => 2,
                (ListId::Tags, false) => 3,
                _ => continue,
            };
            lists[i].push(k);
        }
        let [created, removed, tags_created, tags_removed] = lists;
        let mut c = Changes::default();
        let (mut s1, mut s2, mut s4, mut s5) = (FastSet::default(), FastSet::default(), FastSet::default(), FastSet::default());
        c.created = self.unique_named(created, &mut s1);
        c.removed = self.unique_named(removed, &mut s2);
        c.tags_created = self.unique_named(tags_created, &mut s4);
        c.tags_removed = self.unique_named(tags_removed, &mut s5);
        // Made or taken away in this step: named as such, not as modified.
        let mut s3: FastSet<&str> = s1.union(&s2).copied().collect();
        c.modified = self.unique_named(modified, &mut s3);
        c.tags_modified = self.unique_named(tags_mod, &mut FastSet::default());
        c.materials_modified = self.unique_named(mats_mod, &mut FastSet::default());
        c.scene_modified = scene_mod;
        c.records_modified = records;
        // The registry: names that came, went, or now name another material.
        for (owner, first) in reg {
            let last = &reg_new[&owner];
            let (before, after) = if inverse { (last, &first) } else { (&first, last) };
            for (name, k) in after {
                match before.iter().find(|(n, _)| n == name) {
                    None => c.materials_created.push(name.clone()),
                    Some((_, k0)) if k0 != k => c.materials_replaced.push(name.clone()),
                    _ => {}
                }
            }
            for (name, _) in before {
                if !after.iter().any(|(n, _)| n == name) {
                    c.materials_removed.push(name.clone());
                }
            }
        }
        c
    }
}
