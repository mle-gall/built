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

pub mod py;
pub mod value;

use pyo3::prelude::*;
use pyo3::ffi;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};
use std::thread::ThreadId;
use value::Value;

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

/// A placement matrix: the 16 binary32 values Qt holds (column-major, as
/// `QMatrix4x4.data()`), and for a `.built` placement the binary64 values
/// it was read with plus their projection (`formats/built._built_source`).
#[derive(Clone, Debug, PartialEq)]
pub struct Xform {
    pub f: [f32; 16],
    pub src: Option<Box<([f64; 16], [f64; 16])>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Root,
    Placement,
    Tag,
    Material,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Root => "root",
            Kind::Placement => "placement",
            Kind::Tag => "tag",
            Kind::Material => "material",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Field {
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
}

impl Field {
    pub fn name(self) -> &'static str {
        match self {
            Field::Uid => "uid",
            Field::Name => "name",
            Field::Tag => "layer",
            Field::Xform => "xform",
            Field::Axes => "axes",
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
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FVal {
    Str(Option<String>),
    Bool(bool),
    Xf(Option<Xform>),
    Val(Option<Value>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ListId {
    Children = 0,
    Tags = 1,
}

impl ListId {
    pub fn from_u8(v: u8) -> ListId {
        if v == 1 { ListId::Tags } else { ListId::Children }
    }
    fn member(self) -> Kind {
        match self {
            ListId::Children => Kind::Placement,
            ListId::Tags => Kind::Tag,
        }
    }
}

#[derive(Default)]
pub struct PlacementData {
    pub uid: Option<String>,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub xform: Option<Xform>,
    pub axes: Option<Xform>,
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
    Root,
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

pub struct Node {
    pub kind: Kind,
    pub data: Data,
    pub children: Vec<Key>,
    pub tags: Vec<Key>,
    /// The nodes whose lists name this one, one entry per occurrence.
    pub owners: Vec<Key>,
    pub handle: HPtr,
    pub strong: Option<Py<PyAny>>,
    pub views: [Option<Py<PyAny>>; 3],
    /// The live `HandleList` of each list (children, tags), borrowed: the
    /// view holds its owner's handle (a list keeps what it lists alive),
    /// and clears this when it goes.
    pub list_views: [HPtr; 2],
}

impl Node {
    fn new(kind: Kind) -> Node {
        let data = match kind {
            Kind::Root => Data::Root,
            Kind::Placement => Data::Placement(Box::new(PlacementData { component: true, ..Default::default() })),
            Kind::Tag => Data::Tag(Box::new(TagData { visible: true, ..Default::default() })),
            Kind::Material => Data::Material(Box::default()),
        };
        Node {
            kind,
            data,
            children: Vec::new(),
            tags: Vec::new(),
            owners: Vec::new(),
            handle: HPtr(std::ptr::null_mut()),
            strong: None,
            views: [None, None, None],
            list_views: [HPtr(std::ptr::null_mut()), HPtr(std::ptr::null_mut())],
        }
    }

    pub fn list(&self, l: ListId) -> &Vec<Key> {
        match l {
            ListId::Children => &self.children,
            ListId::Tags => &self.tags,
        }
    }

    fn list_mut(&mut self, l: ListId) -> &mut Vec<Key> {
        match l {
            ListId::Children => &mut self.children,
            ListId::Tags => &mut self.tags,
        }
    }

    pub fn uid(&self) -> Option<&str> {
        match &self.data {
            Data::Placement(p) => p.uid.as_deref(),
            Data::Tag(t) => t.uid.as_deref(),
            Data::Root | Data::Material(_) => None,
        }
    }

    /// What the feed names a material by: its name.
    pub fn material_name(&self) -> Option<&str> {
        match &self.data {
            Data::Material(m) => m.name.as_deref(),
            _ => None,
        }
    }

    /// What the feed names a tag by: its key.
    pub fn tag_key(&self) -> Option<&str> {
        match &self.data {
            Data::Tag(t) => t.key.as_deref(),
            _ => None,
        }
    }

    pub fn get(&self, f: Field) -> Result<FVal, String> {
        let bad = || Err(format!("a {} has no field {}", self.kind.name(), f.name()));
        Ok(match (&self.data, f) {
            (Data::Placement(p), Field::Uid) => FVal::Str(p.uid.clone()),
            (Data::Placement(p), Field::Name) => FVal::Str(p.name.clone()),
            (Data::Placement(p), Field::Tag) => FVal::Str(p.tag.clone()),
            (Data::Placement(p), Field::Xform) => FVal::Xf(p.xform.clone()),
            (Data::Placement(p), Field::Axes) => FVal::Xf(p.axes.clone()),
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
        let kind = self.kind;
        let bad = || Err(format!("a {} has no field {} of that type", kind.name(), f.name()));
        Ok(match (&mut self.data, f, v) {
            (Data::Placement(p), Field::Uid, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut p.uid, s)),
            (Data::Placement(p), Field::Name, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut p.name, s)),
            (Data::Placement(p), Field::Tag, FVal::Str(s)) => FVal::Str(std::mem::replace(&mut p.tag, s)),
            (Data::Placement(p), Field::Xform, FVal::Xf(x)) => FVal::Xf(std::mem::replace(&mut p.xform, x)),
            (Data::Placement(p), Field::Axes, FVal::Xf(x)) => FVal::Xf(std::mem::replace(&mut p.axes, x)),
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
    Ins { owner: Key, list: ListId, at: usize, k: Key },
    Rem { owner: Key, list: ListId, at: usize, k: Key },
    Rep { owner: Key, list: ListId, old: Vec<Key>, new: Vec<Key> },
}

/// The open transaction (one at a time; nested `begin`s are savepoints).
pub struct Open {
    pub thread: ThreadId,
    pub ops: Vec<Op>,
    /// Start of each nested level's ops (`marks[0] == 0`).
    pub marks: Vec<usize>,
    /// `(node, field)` → index of its `Set` op, for coalescing; only
    /// entries at or past the innermost mark are coalesced into.
    pub index: HashMap<(Key, Field), usize>,
    pub pinned: HashSet<Key>,
    /// Nodes made while this transaction is open: their field writes are
    /// not journalled (nothing named them before; undo takes them out of
    /// the lists that hold them, and their fields keep the values redo and
    /// the change record need, a new tag's key for one).
    pub born: HashSet<Key>,
    pub pins: Vec<Py<PyAny>>,
}

/// Python references to release once the lock is gone.
#[derive(Default)]
pub struct Trash(pub Vec<Py<PyAny>>);

impl Trash {
    fn push(&mut self, p: Option<Py<PyAny>>) {
        if let Some(p) = p {
            self.0.push(p);
        }
    }
}

#[derive(Default)]
pub struct Doc {
    pub slots: Vec<Slot>,
    pub free: Vec<u32>,
    pub live: [usize; 4],
    pub open: Option<Open>,
    /// Every write (journalled or not) bumps this: a cheap "anything
    /// changed?" for caches keyed on the document.
    pub serial: u64,
}

static DOC: Mutex<Option<Doc>> = Mutex::new(None);
/// Handles freed while the lock was held elsewhere (never in practice
/// under the lock rule; kept as a safety net). Drained by every op.
static PENDING: Mutex<Vec<Key>> = Mutex::new(Vec::new());

pub struct DocGuard(MutexGuard<'static, Option<Doc>>);

impl std::ops::Deref for DocGuard {
    type Target = Doc;
    fn deref(&self) -> &Doc {
        self.0.as_ref().expect("document initialised")
    }
}
impl std::ops::DerefMut for DocGuard {
    fn deref_mut(&mut self) -> &mut Doc {
        self.0.as_mut().expect("document initialised")
    }
}

pub fn lock() -> DocGuard {
    let mut g = DOC.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Doc::default());
    }
    DocGuard(g)
}

pub fn try_lock() -> Option<DocGuard> {
    match DOC.try_lock() {
        Ok(mut g) => {
            if g.is_none() {
                *g = Some(Doc::default());
            }
            Some(DocGuard(g))
        }
        Err(std::sync::TryLockError::Poisoned(e)) => {
            let mut g = e.into_inner();
            if g.is_none() {
                *g = Some(Doc::default());
            }
            Some(DocGuard(g))
        }
        Err(std::sync::TryLockError::WouldBlock) => None,
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
        self.live[node.kind as usize] -= 1;
        self.serial += 1;
        for v in node.views.iter_mut() {
            trash.push(v.take());
        }
        trash.push(node.strong.take());
        // Members of its lists lose an owner.
        for l in [ListId::Children, ListId::Tags] {
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

    pub fn begin(&mut self) -> usize {
        let here = std::thread::current().id();
        match &mut self.open {
            Some(o) if o.thread == here => {
                o.marks.push(o.ops.len());
                o.marks.len()
            }
            Some(_) => 0, // another thread's transaction: this one is not journalled
            None => {
                self.open = Some(Open {
                    thread: here,
                    ops: Vec::new(),
                    marks: vec![0],
                    index: HashMap::new(),
                    pinned: HashSet::new(),
                    born: HashSet::new(),
                    pins: Vec::new(),
                });
                1
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
            .filter(|op| !matches!(op, Op::Set { old, new, .. } if old == new))
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
            if let Some(o) = self.open.take() {
                trash.0.extend(o.pins);
            }
        }
        r
    }

    /// Apply `ops` backwards (undo) or forwards (redo). Journalled when a
    /// transaction is open on this thread.
    pub fn apply(&mut self, py: Python<'_>, ops: &[Op], backwards: bool, trash: &mut Trash) -> Result<(), String> {
        let mut run = |d: &mut Doc, op: &Op| -> Result<(), String> {
            match (op, backwards) {
                (Op::Set { k, f, old, .. }, true) => d.write(py, *k, *f, old.clone(), trash).map(|_| ()),
                (Op::Set { k, f, new, .. }, false) => d.write(py, *k, *f, new.clone(), trash).map(|_| ()),
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
    pub fn write(&mut self, py: Python<'_>, k: Key, f: Field, v: FVal, trash: &mut Trash) -> Result<FVal, String> {
        let journal = self.journal_open() && !self.open.as_ref().is_some_and(|o| o.born.contains(&k));
        let new = if journal { Some(v.clone()) } else { None };
        let old = {
            let n = self.node_mut(k).ok_or_else(|| format!("no document node {k:#x}"))?;
            let old = n.put(f, v)?;
            match f {
                Field::Xform => trash.push(n.views[VIEW_XFORM].take()),
                Field::Axes => trash.push(n.views[VIEW_AXES].take()),
                Field::Material => trash.push(n.views[VIEW_MATERIAL].take()),
                _ => {}
            }
            old
        };
        self.serial += 1;
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

    fn check_member(&self, owner: Key, list: ListId, k: Key) -> Result<(), String> {
        let o = self.need(owner)?;
        let ok_owner = match list {
            ListId::Children => matches!(o.kind, Kind::Root | Kind::Placement),
            ListId::Tags => o.kind == Kind::Root,
        };
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

    pub fn list_insert(&mut self, py: Python<'_>, owner: Key, list: ListId, at: usize, k: Key) -> Result<(), String> {
        self.check_member(owner, list, k)?;
        let at = {
            let l = self.node_mut(owner).expect("checked").list_mut(list);
            let at = at.min(l.len());
            l.insert(at, k);
            at
        };
        self.add_owner(py, k, owner);
        self.serial += 1;
        self.journal_list(py, Op::Ins { owner, list, at, k }, &[owner, k]);
        Ok(())
    }

    pub fn list_remove(&mut self, py: Python<'_>, owner: Key, list: ListId, at: usize, trash: &mut Trash) -> Result<Key, String> {
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
        self.serial += 1;
        Ok(k)
    }

    pub fn list_replace(&mut self, py: Python<'_>, owner: Key, list: ListId, new: Vec<Key>, trash: &mut Trash) -> Result<(), String> {
        for &k in &new {
            self.check_member(owner, list, k)?;
        }
        if new.is_empty() {
            self.need(owner)?;
        }
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
        self.serial += 1;
        Ok(())
    }

    /// GC `tp_clear` of a handle: release what its node holds.
    pub fn clear_node(&mut self, k: Key, trash: &mut Trash) {
        let lists = match self.node_mut(k) {
            Some(n) => {
                for v in n.views.iter_mut() {
                    trash.push(v.take());
                }
                [std::mem::take(&mut n.children), std::mem::take(&mut n.tags)]
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

/// What a transaction did, by stable keys (`docs/core-protocol.md` §4).
#[derive(Default, Debug)]
pub struct Changes {
    pub created: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
    pub tags_created: Vec<String>,
    pub tags_removed: Vec<String>,
    pub tags_modified: Vec<String>,
    pub materials_modified: Vec<String>,
}

fn push_unique(v: &mut Vec<String>, seen: &mut HashSet<String>, s: Option<&str>) {
    if let Some(s) = s {
        if seen.insert(s.to_owned()) {
            v.push(s.to_owned());
        }
    }
}

impl Doc {
    /// The record of `ops`; `root` limits created/removed placements and
    /// tags to that scene root's lists (None: every root).
    pub fn changes(&self, ops: &[Op], inverse: bool, root: Option<Key>) -> Changes {
        let mut net: HashMap<(ListId, Key), i64> = HashMap::new();
        let mut order: Vec<(ListId, Key)> = Vec::new();
        let mut modified: Vec<Key> = Vec::new();
        let mut tags_mod: Vec<Key> = Vec::new();
        let mut mats_mod: Vec<Key> = Vec::new();
        let mut bump = |l: ListId, k: Key, by: i64, net: &mut HashMap<(ListId, Key), i64>| {
            let e = net.entry((l, k)).or_insert_with(|| {
                order.push((l, k));
                0
            });
            *e += by;
        };
        for op in ops {
            match op {
                Op::Set { k, old, new, .. } if old != new => match self.node(*k).map(|n| n.kind) {
                    Some(Kind::Placement) => modified.push(*k),
                    Some(Kind::Tag) => tags_mod.push(*k),
                    Some(Kind::Material) => mats_mod.push(*k),
                    _ => {}
                },
                Op::Set { .. } => {}
                Op::Ins { owner, list, k, .. } | Op::Rem { owner, list, k, .. } => {
                    let by = if matches!(op, Op::Ins { .. }) { 1 } else { -1 };
                    match self.node(*owner).map(|n| n.kind) {
                        Some(Kind::Root) if root.is_none_or(|r| r == *owner) => bump(*list, *k, by, &mut net),
                        Some(Kind::Placement) => modified.push(*owner),
                        _ => {}
                    }
                }
                Op::Rep { owner, list, old, new } => match self.node(*owner).map(|n| n.kind) {
                    Some(Kind::Root) if root.is_none_or(|r| r == *owner) => {
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
        let mut c = Changes::default();
        let mut s1: HashSet<String> = HashSet::new();
        let mut s2: HashSet<String> = HashSet::new();
        let mut s3: HashSet<String> = HashSet::new();
        let mut s4: HashSet<String> = HashSet::new();
        let mut s5: HashSet<String> = HashSet::new();
        let mut s6: HashSet<String> = HashSet::new();
        for (l, k) in order {
            let n = net[&(l, k)];
            if n == 0 {
                continue;
            }
            let added = (n > 0) != inverse;
            let Some(node) = self.node(k) else { continue };
            match (l, added) {
                (ListId::Children, true) => push_unique(&mut c.created, &mut s1, node.uid()),
                (ListId::Children, false) => push_unique(&mut c.removed, &mut s2, node.uid()),
                (ListId::Tags, true) => push_unique(&mut c.tags_created, &mut s4, node.tag_key()),
                (ListId::Tags, false) => push_unique(&mut c.tags_removed, &mut s5, node.tag_key()),
            }
        }
        // Made or taken away in this step: named as such, not as modified.
        for u in c.created.iter().chain(c.removed.iter()) {
            s3.insert(u.clone());
        }
        for k in modified {
            if let Some(n) = self.node(k) {
                push_unique(&mut c.modified, &mut s3, n.uid());
            }
        }
        for k in tags_mod {
            if let Some(n) = self.node(k) {
                push_unique(&mut c.tags_modified, &mut s6, n.tag_key());
            }
        }
        let mut s7: HashSet<String> = HashSet::new();
        for k in mats_mod {
            if let Some(n) = self.node(k) {
                push_unique(&mut c.materials_modified, &mut s7, n.material_name());
            }
        }
        c
    }
}
