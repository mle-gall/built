// SPDX-License-Identifier: GPL-3.0-or-later
//! A local rebake for large store meshes with vertex-colour paint. Each face slot remembers
//! its range in the last resident pack. Untouched faces copy their baked
//! corners in document order. Only new rings and faces incident on moved
//! vertices run the triangulator again.
use crate::facegeom::{resident_geometry, Hypot, V3};
use crate::store::{keys::Kind, py::PyMeshStore, State, Store};
use numpy::{PyArray1, PyReadonlyArray1, PyReadwriteArray1};
use pyo3::{
    prelude::*,
    types::{PyDict, PySlice},
};
use rayon::prelude::*;

type Obj<'p> = Bound<'p, PyAny>;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub(super) fn token() -> u64 {
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The snap index can carry forward an unchanged native triangle or
/// segment only when it consumed the exact preceding resident draw.
#[pyclass(frozen)]
pub(crate) struct SnapPatch {
    pub code: usize,
    pub before: Option<u64>,
    pub after: u64,
    pub lineage: u64,
    pub revision: u64,
    pub anchor: [f64; 3],
    pub faces: usize,
    pub edges: usize,
    pub triangles: Vec<(usize, usize, usize)>,
    pub segments: Vec<(usize, usize, usize)>,
}

pub(super) struct Layout {
    token: u64,
    code: usize,
    lineage: u64,
    revision: u64,
    state: State,
    spans: Vec<(u32, u32)>,
    anchor: [f64; 3],
    edges: Vec<u32>,
    colors: Vec<[u8; 4]>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    color: [u8; 4],
}

fn encoded(vertices: &[Vertex]) -> &[u8] {
    const {
        assert!(std::mem::size_of::<Vertex>() == 28);
    }
    // SAFETY: the repr(C) fields fill all 28 bytes and are initialized.
    unsafe { std::slice::from_raw_parts(vertices.as_ptr().cast(), vertices.len() * 28) }
}

fn face(store: &Store, points: &[V3], slot: u32, hypot: Hypot, color: [u8; 4]) -> Vec<Vertex> {
    let rings = store.face_rings(slot);
    let refs: Vec<_> = rings.iter().map(Vec::as_slice).collect();
    let (triangles, n) = resident_geometry(points, &refs, hypot);
    triangles
        .into_iter()
        .flatten()
        .map(|i| {
            let V3(x, y, z) = points[i as usize];
            Vertex {
                pos: [x, y, z],
                normal: [n.0, n.1, n.2],
                color,
            }
        })
        .collect()
}

/// Runs of unchanged bytes in document order, and the newly encoded
/// vertices between them. All offsets are relative to this prototype.
#[derive(Default)]
pub(super) struct Edit {
    pub(super) copies: Vec<(usize, usize, usize)>,
    writes: Vec<(usize, usize, usize)>,
    pub(super) data: Vec<u8>,
    pub(super) len: usize,
}

impl Edit {
    fn run(runs: &mut Vec<(usize, usize, usize)>, src: usize, dst: usize, len: usize) {
        if len == 0 {
            return;
        }
        if let Some((a, b, n)) = runs.last_mut() {
            if *a + *n == src && *b + *n == dst {
                *n += len;
                return;
            }
        }
        runs.push((src, dst, len));
    }
    pub(super) fn keep(&mut self, src: u32, len: usize) {
        Self::run(&mut self.copies, src as usize, self.len, len);
        self.len += len;
    }
    pub(super) fn write(&mut self, data: &[u8]) {
        Self::run(&mut self.writes, self.data.len(), self.len, data.len());
        self.data.extend_from_slice(data);
        self.len += data.len();
    }
    fn apply_writes(&self, target: &mut [u8]) {
        for &(s, d, n) in &self.writes {
            target[d..d + n].copy_from_slice(&self.data[s..s + n]);
        }
    }
    pub(super) fn ordered(&self) -> bool {
        self.copies.windows(2).all(|w| w[0].0 + w[0].2 <= w[1].0)
    }
    pub(super) fn in_place(&self, target: &mut [u8]) {
        // Source and destination runs are both monotone. Left shifts can
        // overwrite only already-consumed left shifts; right shifts mirror it.
        for &(s, d, n) in &self.copies {
            if d < s {
                target.copy_within(s..s + n, d);
            }
        }
        for &(s, d, n) in self.copies.iter().rev() {
            if d > s {
                target.copy_within(s..s + n, d);
            }
        }
        self.apply_writes(target);
    }
    pub(super) fn reverse(&self, original: &[u8]) -> Self {
        let mut undo = Self::default();
        for &(s, d, n) in &self.copies {
            let from = undo.len;
            undo.write(&original[from..s]);
            undo.keep(d as u32, n);
        }
        let from = undo.len;
        undo.write(&original[from..]);
        undo
    }
    pub(super) fn expanded(&self, original: &[u8]) -> Vec<u8> {
        if self.copies.is_empty() {
            return self.data.clone();
        }
        let mut out = vec![0; self.len];
        for &(s, d, n) in &self.copies {
            out[d..d + n].copy_from_slice(&original[s..s + n]);
        }
        self.apply_writes(&mut out);
        out
    }
}

struct Bake {
    edit: Edit,
    faces: usize,
    bounds: [f32; 8],
    spans: Vec<(u32, u32)>,
    edges: Vec<u32>,
}

fn bake(store: &Store, anchor: [f64; 3], hypot: Hypot, previous: Option<&Layout>, colors: &[[u8; 4]]) -> Option<Bake> {
    let st = &store.state;
    let shift: [f64; 3] = std::array::from_fn(|i| st.frame.origin[i] - anchor[i]);
    let points: Vec<_> = st
        .vpos
        .iter()
        .map(|p| {
            let q: [f32; 3] = std::array::from_fn(|k| {
                if st.frame.origin == anchor {
                    p[k] as f32
                } else {
                    (p[k] + shift[k]) as f32
                }
            });
            V3(q[0], q[1], q[2])
        })
        .collect();
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::INFINITY,
        0.,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
        0.,
    ];
    for i in st.slots[0].live_slots() {
        let V3(x, y, z) = points[i as usize];
        for (k, value) in [x, y, z].into_iter().enumerate() {
            bounds[k] = bounds[k].min(value);
            bounds[k + 4] = bounds[k + 4].max(value);
        }
    }
    let previous = previous
        .filter(|p| p.lineage == store.lineage && p.anchor == anchor && p.state.frame == st.frame);
    let mut dirty = vec![previous.is_none(); st.slots[2].len()];
    let mut edge_dirty = vec![previous.is_none(); st.slots[1].len()];
    if let Some(layout) = previous {
        let change = st.changes_since(&layout.state);
        for &key in change.created[1].iter().chain(&change.modified[1]) {
            edge_dirty[store.slot(Kind::Edge, key)? as usize] = true;
        }
        for &key in change.created[2].iter().chain(&change.modified[2]) {
            dirty[store.slot(Kind::Face, key)? as usize] = true;
        }
        for key in change.moved {
            for e in store.edges_at(store.slot(Kind::Vertex, key)?) {
                edge_dirty[e as usize] = true;
                for &f in store.edge_faces(e) {
                    dirty[f as usize] = true;
                }
            }
        }
    }
    let color = |slot: u32| colors.get(slot as usize).copied().unwrap_or(DEFAULT_COLOR);
    if let Some(layout) = previous {
        if !(colors.is_empty() && layout.colors.is_empty()) {
            for (slot, changed) in dirty.iter_mut().enumerate() {
                *changed |= color(slot as u32) != layout.colors.get(slot).copied().unwrap_or(DEFAULT_COLOR);
            }
        }
    }
    let faces = store.live_slots(Kind::Face);
    let mut spans = vec![(0, 0); dirty.len()];
    let mut edit = Edit::default();
    let cold = previous.is_none().then(|| {
        faces
            .par_iter()
            .map(|&slot| face(store, &points, slot, hypot, color(slot)))
            .collect::<Vec<_>>()
    });
    if cold.is_some() {
        edit.data
            .reserve((faces.len() * 6 + st.slots[1].live * 2) * 28);
    }
    for (i, &slot) in faces.iter().enumerate() {
        let start = edit.len;
        if let Some(all) = &cold {
            edit.write(encoded(&all[i]));
        } else if dirty[slot as usize] {
            edit.write(encoded(&face(store, &points, slot, hypot, color(slot))));
        } else {
            let &(a, b) = previous?.spans.get(slot as usize)?;
            edit.keep(a, (b - a) as usize);
        }
        spans[slot as usize] = (start.try_into().ok()?, edit.len.try_into().ok()?);
    }
    let faces = edit.len / 28;
    let mut edges = vec![u32::MAX; edge_dirty.len()];
    for e in store.live_slots(Kind::Edge) {
        let ends = *st.ends.get(e as usize);
        if ends[0] == ends[1] {
            continue;
        }
        edges[e as usize] = edit.len.try_into().ok()?;
        if !edge_dirty[e as usize] {
            let from = *previous?.edges.get(e as usize)?;
            if from == u32::MAX {
                return None;
            }
            edit.keep(from, 56);
            continue;
        }
        let vertices = ends.map(|i| {
            let V3(x, y, z) = points[i as usize];
            Vertex {
                pos: [x, y, z],
                normal: [0., 0., 1.],
                color: [0; 4],
            }
        });
        edit.write(encoded(&vertices));
    }
    Some(Bake {
        edit,
        edges,
        faces,
        bounds,
        spans,
    })
}

const DEFAULT_COLOR: [u8; 4] = [210, 210, 210, 0];

/// Flat opaque paint has no secondary material record. Keep the resolved
/// bytes by face slot: in-place edits of an attribute dict need no native
/// geometry revision to invalidate the colour in the previous draw.
fn opaque_color(attrs: &Obj<'_>) -> PyResult<Option<[u8; 4]>> {
    let attrs = attrs.cast::<PyDict>()?;
    for key in ["back", "pbr", "texture", "opacity", "hidden"] {
        if attrs.get_item(key)?.is_some_and(|v| !v.is_none()) {
            return Ok(None);
        }
    }
    let Some(color) = attrs.get_item("color")?.filter(|c| !c.is_none()) else {
        return Ok(Some(DEFAULT_COLOR));
    };
    let [r,g,b] = super::sources::rgb_bytes(&color)?;
    Ok(Some([r,g,b,255]))
}

fn plain_store<'p>(
    tracker: &Obj<'p>,
    packed: &Obj<'p>,
    mesh: &Obj<'p>,
    code: usize,
) -> PyResult<Option<(Bound<'p, PyMeshStore>, Vec<[u8; 4]>)>> {
    let Some(own) = mesh.getattr_opt("__dict__")? else {
        return Ok(None);
    };
    let own = own.cast_into::<PyDict>()?;
    let Some(store_obj) = own.get_item("_store")? else {
        return Ok(None);
    };
    if !store_obj.is_exact_instance_of::<PyMeshStore>()
        || own.contains("_deferred_exact")?
        || own
            .get_item("_deferred_source")?
            .is_some_and(|s| !s.is_none())
        || mesh.getattr_opt("_instance_definition")?.is_some()
        || packed.getattr("bake_focus")?.is_truthy()?
        || tracker.getattr("_hidden_tags")?.is_truthy()?
    {
        return Ok(None);
    }
    let store = store_obj.cast::<PyMeshStore>()?.borrow();
    let native = &store.inner;
    let st = &native.state;
    if st.slots[2].live < 32768
        || st.exact.native.is_some()
        || st.exact.curved.is_some()
        || st.exact.arcs.is_some()
        || st.eflags.iter().any(|f| *f != 0)
        // Push/Pull marks interior cap faces for the topology kernel.
        // The resident bake renders them using the same face attributes.
        || st.fflags.iter().any(|f| *f & !crate::store::INTERIOR != 0)
    {
        return Ok(None);
    }
    let mut palette = vec![DEFAULT_COLOR];
    for attrs in mesh.getattr("_attr_table")?.try_iter()? {
        let Some(color) = opaque_color(&attrs?)? else { return Ok(None) };
        palette.push(color);
    }
    let mut colors = if palette.len() == 1 {
        if st.fattr.iter().any(|f| *f != 0) { return Ok(None); }
        Vec::new()
    } else {
        let Some(colors) = st.fattr.iter().map(|&f| palette.get(f as usize).copied()).collect::<Option<Vec<_>>>() else { return Ok(None) };
        colors
    };
    for (key, attrs) in mesh.getattr("_attr_own")?.cast::<PyDict>()?.iter() {
        if attrs.is_none() { continue; }
        let Some(color) = opaque_color(&attrs)? else { return Ok(None) };
        if colors.is_empty() && color == DEFAULT_COLOR { continue; }
        if let Some(slot) = key.extract::<u128>().ok().and_then(|k| native.slot(Kind::Face,k)) {
            if colors.is_empty() { colors.resize(st.slots[2].len(), DEFAULT_COLOR); }
            colors[slot as usize] = color;
        }
    }
    for row in packed.getattr("profile_exclusions")?.try_iter()? {
        if row?.get_item(0)?.extract::<usize>()? == code {
            return Ok(None);
        }
    }
    drop(store);
    let variant = tracker
        .getattr("variants")?
        .get_item(mesh.as_ptr() as usize)?
        .get_item(code)?;
    if variant.get_item(0)?.is_truthy()? || variant.get_item(1)?.is_truthy()? {
        return Ok(None);
    }
    Ok(Some((store_obj.cast_into::<PyMeshStore>()?, colors)))
}

/// Seed a layout from an unmodified resident draw. A single-ring triangle
/// or quad can produce at most one or two triangles. Equality of the sum
/// with the resident draw count proves that every face reached its bound.
/// The source stamp excludes a deferred stream that has since welded.
pub(super) fn prime(tracker: &Bound<'_, super::updates::InstanceUpdates>) -> PyResult<()> {
    if tracker.borrow().plain.is_some() {
        return Ok(());
    }
    let t = tracker.as_any();
    let Some(mesh) = t.getattr("scene")?.getattr_opt("mesh")? else {
        return Ok(());
    };
    let meshes = t.getattr("meshes")?.cast_into::<PyDict>()?;
    let Some(entry) = meshes.get_item(mesh.as_ptr() as usize)? else {
        return Ok(());
    };
    let code = entry.get_item(2)?.extract::<i64>()?;
    if code < 0 {
        return Ok(());
    }
    let code = code as usize;
    let packed = t.getattr("packed")?;
    let command = packed.getattr("commands")?.get_item(code)?;
    let faces: u32 = command.get_item("count")?.extract()?;
    if faces < 32768 * 3 || !entry.get_item(1)?.eq(mesh.getattr("_mut_serial")?)? {
        return Ok(());
    }
    let Some((st, colors)) = plain_store(t, &packed, &mesh, code)? else {
        return Ok(());
    };
    let store = st.borrow();
    let Some(sources) = packed.getattr_opt("_edit_sources")? else {
        return Ok(());
    };
    let Some(stamp) = sources.cast::<PyDict>()?.get_item(code)? else {
        return Ok(());
    };
    if stamp.extract::<(u64, u64)>()? != (store.inner.lineage, store.inner.revision) {
        return Ok(());
    }
    let state = &store.inner.state;
    let mut spans = vec![(0, 0); state.slots[2].len()];
    let mut count = 0u32;
    for f in store.inner.live_slots(Kind::Face) {
        let r = state.frings.get(f as usize);
        if r.n_rings != 1 {
            return Ok(());
        }
        let n = *state.ring_sizes.get(r.ring_start as usize);
        if !(3..=4).contains(&n) {
            return Ok(());
        }
        let Some(end) = count.checked_add((n - 2) * 84) else {
            return Ok(());
        };
        spans[f as usize] = (count, end);
        count = end;
    }
    if count / 28 != faces {
        return Ok(());
    }
    let mut edges = vec![u32::MAX; state.slots[1].len()];
    for e in store.inner.live_slots(Kind::Edge) {
        let [a, b] = *state.ends.get(e as usize);
        if a != b {
            edges[e as usize] = count;
            let Some(end) = count.checked_add(56) else {
                return Ok(());
            };
            count = end;
        }
    }
    let lines: u32 = packed
        .getattr("edge_commands")?
        .get_item(code)?
        .get_item("count")?
        .extract()?;
    if count / 28 != faces + lines {
        return Ok(());
    }
    let anchor = packed
        .getattr("prototype_origins")?
        .get_item(code)?
        .extract()?;
    store.prepare_edit_queries(t.py())?;
    tracker.borrow_mut().plain = Some(Layout {
        code,
        token: token(),
        lineage: store.inner.lineage,
        revision: store.inner.revision,
        state: state.clone(),
        spans,
        edges,
        anchor,
        colors,
    });
    Ok(())
}

/// Push/Pull suppresses the original face during its preview. These are
/// the triangle ordinals already drawn by the GPU for the requested keys.
pub(super) fn face_ranges<'p>(
    tracker: &Bound<'p, super::updates::InstanceUpdates>,
    mesh: &Obj<'p>,
    faces: &Obj<'p>,
) -> PyResult<Option<Vec<(usize, u32, u32)>>> {
    let t = tracker.borrow();
    let Some(l) = &t.plain else { return Ok(None) };
    let Some(store) = mesh
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_store")?
    else {
        return Ok(None);
    };
    let Ok(store) = store.cast::<PyMeshStore>() else {
        return Ok(None);
    };
    let store = store.borrow();
    if store.inner.lineage != l.lineage || store.inner.revision != l.revision {
        return Ok(None);
    }
    let variants = tracker
        .as_any()
        .getattr("variants")?
        .cast_into::<PyDict>()?;
    let Some(codes) = variants.get_item(mesh.as_ptr() as usize)? else {
        return Ok(None);
    };
    if codes.len()? != 1 || !codes.contains(l.code)? {
        return Ok(None);
    }
    let mut ranges = Vec::new();
    for face in faces.try_iter()? {
        let key = face?.getattr("key")?.extract()?;
        if let Some(f) = store.inner.slot(Kind::Face, key) {
            let (a, b) = l.spans[f as usize];
            ranges.push((l.code, a / 84, (b - a) / 84));
        }
    }
    Ok(Some(ranges))
}

/// Face attributes are mutable document dictionaries. Reading them through
/// Face.attrs creates an override, so resolving a preview by walking every
/// face view changes how the next edit has to look up its paint. Read the
/// store columns instead and verify the triangle bound against each draw.
pub(super) fn painted_face_ranges<'p>(
    tracker: &Bound<'p, super::updates::InstanceUpdates>,
    mesh: &Obj<'p>,
    faces: &Obj<'p>,
) -> PyResult<Option<Vec<(usize, u32, u32)>>> {
    let Some(owner) = mesh.getattr_opt("__dict__")? else { return Ok(None) };
    let owner = owner.cast_into::<PyDict>()?;
    let Some(raw) = owner.get_item("_store")? else { return Ok(None) };
    if !raw.is_exact_instance_of::<PyMeshStore>() || owner.get_item("_deferred_source")?.is_some_and(|v| !v.is_none()) {
        return Ok(None);
    }
    let raw = raw.cast_into::<PyMeshStore>()?;
    let store = raw.borrow();
    let st = &store.inner.state;
    if st.slots[2].live < 32768 || st.exact.native.is_some() { return Ok(None); }
    let t = tracker.as_any();
    let variants = t.getattr("variants")?.cast_into::<PyDict>()?;
    let Some(codes) = variants.get_item(mesh.as_ptr() as usize)? else { return Ok(None) };
    let packed = t.getattr("packed")?;
    let commands = packed.getattr("commands")?;
    let tags = t.getattr("_hidden_tags")?;
    let hidden = |attrs: &Obj<'p>| -> PyResult<bool> {
        Ok(attrs.call_method1("get", ("hidden",))?.is_truthy()? ||
            tags.contains(attrs.call_method1("get", ("layer",))?)?)
    };
    let mut exclude = vec![false];
    for attrs in mesh.getattr("_attr_table")?.try_iter()? { exclude.push(hidden(&attrs?)?); }
    let mut skip: Vec<_> = st.fattr.iter().map(|row| exclude.get(*row as usize).copied().unwrap_or(false)).collect();
    for (key, attrs) in mesh.getattr("_attr_own")?.cast::<PyDict>()?.iter() {
        if let Some(slot) = key.extract::<u128>().ok().and_then(|key| store.inner.slot(Kind::Face, key)) {
            if !attrs.is_none() { skip[slot as usize] = hidden(&attrs)?; }
        }
    }
    let mut wanted = std::collections::HashSet::new();
    for f in faces.try_iter()? {
        if let Some(slot) = store.inner.slot(Kind::Face, f?.getattr("key")?.extract()?) { wanted.insert(slot); }
    }
    let mut spans = Vec::new();
    let mut first = 0u32;
    for f in store.inner.live_slots(Kind::Face) {
        if skip[f as usize] { continue; }
        let r = st.frings.get(f as usize);
        // Clipping ears or splitting a loop emits at most n-2 triangles.
        // Each bridged hole adds its corners plus two bridge endpoints.
        // Equality with the total draw below proves every bound was reached.
        if r.n_rings == 0 { return Ok(None); }
        let mut size = 0u32;
        for i in 0..r.n_rings {
            let n = *st.ring_sizes.get((r.ring_start + i) as usize);
            if n < 3 { return Ok(None); }
            let Some(total) = size.checked_add(n) else { return Ok(None); };
            size = total;
        }
        let Some(extra) = (r.n_rings - 1).checked_mul(2) else { return Ok(None); };
        let Some(count) = (size - 2).checked_add(extra) else { return Ok(None); };
        if wanted.contains(&f) { spans.push((first, count)); }
        let Some(end) = first.checked_add(count) else { return Ok(None) };
        first = end;
    }
    let mut result = Vec::new();
    for code in codes.try_iter()? {
        let code: usize = code?.extract()?;
        let variant = codes.get_item(code)?;
        if variant.get_item(0)?.is_truthy()? || variant.get_item(1)?.is_truthy()? { return Ok(None); }
        if commands.get_item(code)?.get_item("count")?.extract::<usize>()? != first as usize * 3 { return Ok(None); }
        result.extend(spans.iter().map(|&(a,n)| (code,a,n)));
    }
    Ok(Some(result))
}

pub(super) fn apply<'p>(
    tracker: &Obj<'p>,
    changed: &Obj<'p>,
    renderer: Option<&Obj<'p>>,
    cache: &mut Option<Layout>,
) -> PyResult<bool> {
    let py = tracker.py();
    let old = cache.take();
    if changed.len()? != 1 {
        return Ok(false);
    }
    let entry = changed.get_item(0)?;
    let mesh = entry.get_item(0)?;
    let code = entry.get_item(2)?.extract::<usize>()?;
    let packed = tracker.getattr("packed")?;
    let Some((store_obj, colors)) = plain_store(tracker, &packed, &mesh, code)? else {
        return Ok(false);
    };
    let store = store_obj.borrow();
    let native = &store.inner;
    let st = &native.state;
    let origins = packed.getattr("prototype_origins")?;
    if code >= origins.len()? {
        return Ok(false);
    }
    let anchor = origins.get_item(code)?.extract::<[f64; 3]>()?;
    let command = packed.getattr("commands")?.get_item(code)?;
    let base = command.get_item("base")?.extract::<isize>()?;
    let count = command.get_item("count")?.extract::<isize>()?;
    if base < 0 || count == 0 {
        return Ok(false);
    }
    let np = py.import("numpy")?;
    let raw = |a: &Obj<'p>| {
        a.call_method1("view", (np.getattr("uint8")?,))?
            .call_method1("reshape", (-1,))
    };
    let previous = old.as_ref().filter(|l| l.code == code);
    let h = crate::hypot_of(
        &py.import("core._native")?
            .call_method0("qt_hypot")?
            .extract::<String>()?,
    )?;
    let Some(result) = bake(native, anchor, h, previous, &colors) else {
        return Ok(false);
    };
    let layout = Layout {
        code,
        token: token(),
        lineage: native.lineage,
        revision: native.revision,
        state: st.clone(),
        spans: result.spans,
        edges: result.edges,
        anchor,
        colors,
    };
    drop(store);
    let edit = result.edit;
    let total = edit.len / 28;
    let faces = result.faces;
    let mut snap = SnapPatch {
        code,
        before: None,
        after: layout.token,
        lineage: layout.lineage,
        revision: layout.revision,
        anchor,
        faces: faces / 3,
        edges: (total - faces) / 2,
        triangles: Vec::new(),
        segments: Vec::new(),
    };
    let old_edges: usize = packed
        .getattr("edge_commands")?
        .get_item(code)?
        .get_item("count")?
        .extract()?;
    if let Some(old) = previous.filter(|p| {
        p.lineage == layout.lineage
            && p.anchor == anchor
            && p.state.frame == layout.state.frame
            && p.state.slots[1].live * 2 == old_edges
    }) {
        if snap.edges == layout.state.slots[1].live {
            snap.before = Some(old.token);
            let old_end = count as usize * 28;
            let new_end = faces * 28;
            for &(s, d, n) in &edit.copies {
                // A run may join the last untouched triangle and first edge.
                let tri = n.min(new_end.saturating_sub(d));
                if tri != 0 {
                    snap.triangles.push((s / 84, d / 84, tri / 84));
                }
                if tri < n {
                    snap.segments.push((
                        (s + tri - old_end) / 56,
                        (d + tri - new_end) / 56,
                        (n - tri) / 56,
                    ));
                }
            }
        }
    }
    let bounds = PyArray1::from_slice(py, &result.bounds).call_method1("reshape", (2, 4))?;
    let empty = PyArray1::<u8>::from_vec(py, Vec::new());
    let upload = |vertices: &Obj<'p>, indices: &Obj<'p>| -> PyResult<()> {
        if let Some(r) = renderer {
            r.call_method1(
                "replace_prototype",
                (code, raw(vertices)?, raw(indices)?, raw(&bounds)?, faces),
            )?;
            r.call_method1("update_prototype_materials", (code, &empty))?;
        }
        Ok(())
    };
    packed.call_method1("adopt_reserved", (true,))?;
    let pack = packed.cast::<super::packed_scene::PackedScene>()?;
    let Some((start, owned, room)) = super::packed_scene::owned_room(pack, code)? else {
        return Ok(false);
    };
    if start != base as usize || edit.copies.iter().any(|&(s, _, n)| s + n > owned * 28) {
        return Ok(false);
    }
    let slice = |name: &str, n: usize| {
        packed
            .getattr(name)?
            .get_item(PySlice::new(py, base, base + n as isize, 1))
    };
    let base = if !edit.copies.is_empty() && total <= room && edit.ordered() {
        let vertices = slice("vertices", owned.max(total))?;
        let bytes = raw(&vertices)?;
        let indices = slice("indices", total)?;
        let undo = {
            let mut dst: PyReadwriteArray1<u8> = bytes.extract()?;
            let dst = dst.as_slice_mut()?;
            let undo = renderer.is_some().then(|| edit.reverse(&dst[..owned * 28]));
            edit.in_place(dst);
            undo
        };
        if total > owned {
            let mut ix: PyReadwriteArray1<u32> = indices.extract()?;
            for (j, x) in ix.as_slice_mut()?[owned..].iter_mut().enumerate() {
                *x = (owned + j) as u32;
            }
        }
        if let Err(e) = upload(&slice("vertices", total)?, &indices) {
            if let Some(undo) = undo {
                let mut dst: PyReadwriteArray1<u8> = bytes.extract()?;
                undo.in_place(dst.as_slice_mut()?);
            }
            return Err(e);
        }
        start
    } else {
        let data = if edit.copies.is_empty() {
            edit.data
        } else {
            let v = raw(&slice("vertices", owned)?)?;
            let previous: PyReadonlyArray1<u8> = v.extract()?;
            edit.expanded(previous.as_slice()?)
        };
        let vertices = PyArray1::from_vec(py, data)
            .call_method1("view", (super::packed_scene::dtype(py, "VERTEX")?,))?;
        let indices = PyArray1::from_vec(py, (0..total as u32).collect());
        upload(&vertices, indices.as_any())?;
        packed
            .call_method1("place", (code, vertices, indices))?
            .extract()?
    };
    for (name, n, start) in [
        ("commands", faces, 0),
        ("edge_commands", total - faces, faces),
        ("hidden_commands", 0, total),
        ("hidden_edge_commands", 0, total),
    ] {
        let cmd = packed.getattr(name)?.get_item(code)?;
        cmd.set_item("base", base)?;
        cmd.set_item("first", base + start)?;
        cmd.set_item("count", n)?;
    }
    packed.getattr("bounds")?.set_item(code, bounds)?;
    packed
        .getattr("material_updates")?
        .set_item(code, (0, empty))?;
    tracker.getattr("meshes")?.set_item(
        mesh.as_ptr() as usize,
        (&mesh, mesh.getattr("_mut_serial")?, code),
    )?;
    packed.setattr("_snap_edit", Bound::new(py, snap)?)?;
    *cache = Some(layout);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::Edit;

    #[test]
    fn shifted_spans_and_overwrites_can_be_rolled_back() {
        let original = b"0123456789";
        let mut edit = Edit::default();
        edit.write(b"ab");
        edit.keep(0, 2);
        edit.keep(5, 3);
        edit.write(b"!");
        edit.keep(9, 1);
        let undo = edit.reverse(original);
        let mut out = *original;
        edit.in_place(&mut out);
        assert_eq!(&out[..edit.len], b"ab01567!9");
        undo.in_place(&mut out);
        assert_eq!(out, *original);
    }
}
