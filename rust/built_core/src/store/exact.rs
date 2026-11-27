// SPDX-License-Identifier: GPL-3.0-or-later
//! Sparse binding columns for one mesh. Tombstones retain their binding
//! for relink; allocating a new generation clears it. Snapshots share rows.
use super::keys::{Kind, NONE};
use std::sync::Arc;

#[derive(Clone, Default, Debug)]
struct Column(Option<Arc<Vec<(u32, u32)>>>);

impl Column {
    fn rows(&self) -> &[(u32, u32)] {
        self.0.as_deref().map(Vec::as_slice).unwrap_or_default()
    }

    fn get(&self, slot: u32) -> u32 {
        let rows = self.rows();
        rows.binary_search_by_key(&slot, |r| r.0)
            .map_or(NONE, |i| rows[i].1)
    }

    fn set(&mut self, slot: u32, shape: u32) {
        match self.rows().binary_search_by_key(&slot, |r| r.0) {
            Ok(i) if self.rows()[i].1 == shape => (),
            Ok(_) if shape == NONE && self.rows().len() == 1 => self.0 = None,
            Ok(i) => {
                let rows = Arc::make_mut(self.0.as_mut().unwrap());
                if shape == NONE {
                    rows.remove(i);
                } else {
                    rows[i].1 = shape;
                }
            }
            Err(i) if shape != NONE => {
                Arc::make_mut(self.0.get_or_insert_with(Arc::default)).insert(i, (slot, shape));
            }
            Err(_) => (),
        }
    }

    fn shared(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExactColumns {
    pub(crate) arcs: Option<Arc<std::collections::BTreeMap<u32,super::source_arcs::ArcBinding>>>,
    pub(crate) native: Option<Arc<super::native::NativePatch>>,
    pub(crate) planes: Option<Arc<Vec<crate::exact::Frame>>>,
    face_surface: Column,
    edge_exact_curve: Column,
    /// Curves and curved surfaces bound to ordinary store elements
    /// (`exact_bound.rs`), sorted by slot.
    pub(crate) curved: Option<Arc<Curved>>,
}

/// Slots per copy-on-write chunk of a binding map.
const CHUNK_BITS: u32 = 9;

/// A sparse map from slot to binding in copy-on-write chunks of 512 slots:
/// cloning it (a snapshot, the start of a step) shares every chunk, and a
/// step that edits a few bindings copies only their chunks. Two maps that
/// share a chunk compare it by pointer.
#[derive(Clone, Debug)]
pub(crate) struct ChunkMap<T: Clone> {
    chunks: Vec<Option<Arc<std::collections::BTreeMap<u32, T>>>>,
    len: usize,
}

impl<T: Clone> Default for ChunkMap<T> {
    fn default() -> Self {
        Self { chunks: Vec::new(), len: 0 }
    }
}

impl<T: Clone> ChunkMap<T> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(crate) fn get(&self, slot: &u32) -> Option<&T> {
        self.chunks.get((*slot >> CHUNK_BITS) as usize)?.as_ref()?.get(slot)
    }
    pub(crate) fn contains_key(&self, slot: &u32) -> bool {
        self.get(slot).is_some()
    }
    pub(crate) fn get_mut(&mut self, slot: &u32) -> Option<&mut T> {
        let chunk = self.chunks.get_mut((*slot >> CHUNK_BITS) as usize)?.as_mut()?;
        if !chunk.contains_key(slot) {
            return None;
        }
        Arc::make_mut(chunk).get_mut(slot)
    }
    pub(crate) fn insert(&mut self, slot: u32, value: T) -> Option<T> {
        let c = (slot >> CHUNK_BITS) as usize;
        if self.chunks.len() <= c {
            self.chunks.resize(c + 1, None);
        }
        let old = Arc::make_mut(self.chunks[c].get_or_insert_with(Arc::default)).insert(slot, value);
        if old.is_none() {
            self.len += 1;
        }
        old
    }
    pub(crate) fn remove(&mut self, slot: &u32) -> Option<T> {
        let c = (*slot >> CHUNK_BITS) as usize;
        let chunk = self.chunks.get_mut(c)?.as_mut()?;
        if !chunk.contains_key(slot) {
            return None;
        }
        let old = Arc::make_mut(chunk).remove(slot);
        if chunk.is_empty() {
            self.chunks[c] = None;
        }
        self.len -= 1;
        old
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&u32, &T)> + '_ {
        self.chunks.iter().flatten().flat_map(|c| c.iter())
    }
    pub(crate) fn keys(&self) -> impl Iterator<Item = u32> + '_ {
        self.iter().map(|(k, _)| *k)
    }
    pub(crate) fn extend(&mut self, items: impl IntoIterator<Item = (u32, T)>) {
        for (k, v) in items {
            self.insert(k, v);
        }
    }
    /// Slots whose entry differs between the maps, skipping shared chunks.
    pub(crate) fn changed(&self, other: &Self) -> Vec<u32>
    where
        T: PartialEq,
    {
        let mut out = Vec::new();
        for c in 0..self.chunks.len().max(other.chunks.len()) {
            let (a, b) = (self.chunks.get(c).cloned().flatten(), other.chunks.get(c).cloned().flatten());
            match (a, b) {
                (Some(a), Some(b)) if Arc::ptr_eq(&a, &b) => (),
                (a, b) => {
                    let empty = std::collections::BTreeMap::new();
                    let (a, b) = (a.as_deref().unwrap_or(&empty), b.as_deref().unwrap_or(&empty));
                    for (k, v) in a {
                        if b.get(k) != Some(v) {
                            out.push(*k);
                        }
                    }
                    out.extend(b.keys().filter(|k| !a.contains_key(k)).copied());
                }
            }
        }
        out.sort_unstable();
        out
    }
}

impl<T: Clone + PartialEq> PartialEq for ChunkMap<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.changed(other).is_empty()
    }
}

impl<T: Clone> FromIterator<(u32, T)> for ChunkMap<T> {
    fn from_iter<I: IntoIterator<Item = (u32, T)>>(iter: I) -> Self {
        let mut out = Self::default();
        out.extend(iter);
        out
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Curved {
    pub edges: ChunkMap<crate::exact::bound::EdgeBind>,
    pub faces: ChunkMap<crate::exact::bound::Support>,
}

impl Curved {
    pub(crate) fn edge(&self, slot: u32) -> Option<&crate::exact::bound::EdgeBind> {
        self.edges.get(&slot)
    }
    pub(crate) fn face(&self, slot: u32) -> Option<&crate::exact::bound::Support> {
        self.faces.get(&slot)
    }
    fn has(&self, kind: Kind, slot: u32) -> bool {
        match kind {
            Kind::Edge => self.edge(slot).is_some(),
            Kind::Face => self.face(slot).is_some(),
            Kind::Vertex => false,
        }
    }
    fn changed(&self, other: &Self, kind: Kind) -> Vec<u32> {
        match kind {
            Kind::Edge => self.edges.changed(&other.edges),
            Kind::Face => self.faces.changed(&other.faces),
            Kind::Vertex => Vec::new(),
        }
    }
}

impl ExactColumns {
    fn column(&self, kind: Kind) -> Option<&Column> {
        match kind {
            Kind::Face => Some(&self.face_surface),
            Kind::Edge => Some(&self.edge_exact_curve),
            Kind::Vertex => None,
        }
    }

    fn same_binding(&self, other: &Self, kind: Kind, slot: u32) -> bool {
        if kind==Kind::Edge && self.arcs.as_ref().and_then(|a|a.get(&slot))!=other.arcs.as_ref().and_then(|a|a.get(&slot)){return false}
        let a = self.get(kind, slot);
        let b = other.get(kind, slot);
        if kind == Kind::Face && a != NONE && b != NONE {
            let plane = |columns: &Self, index| {
                columns
                    .planes
                    .as_deref()
                    .and_then(|p| p.get(index as usize))
                    .cloned()
            };
            match (plane(self, a), plane(other, b)) {
                (Some(a), Some(b)) => return a == b,
                (None, None) => (),
                _ => return false,
            }
        }
        a == b
    }

    fn curved_shared(&self, other: &Self) -> bool {
        match (&self.curved, &other.curved) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    /// Slots whose curved binding differs between the two columns.
    fn curved_changes(&self, other: &Self, kind: Kind) -> Vec<u32> {
        if self.curved_shared(other) {
            return Vec::new();
        }
        let empty = Curved::default();
        let a = self.curved.as_deref().unwrap_or(&empty);
        let b = other.curved.as_deref().unwrap_or(&empty);
        a.changed(b, kind)
    }

    pub(crate) fn curved_edge(&self, slot: u32) -> Option<&crate::exact::bound::EdgeBind> {
        self.curved.as_deref()?.edge(slot)
    }
    pub(crate) fn curved_face(&self, slot: u32) -> Option<&crate::exact::bound::Support> {
        self.curved.as_deref()?.face(slot)
    }

    pub fn shared(&self, other: &Self, kind: Kind) -> bool {
        if kind==Kind::Edge && !match (&self.arcs,&other.arcs){(None,None)=>true,(Some(a),Some(b))=>Arc::ptr_eq(a,b),_=>false}{return false}
        (kind == Kind::Vertex || self.curved_shared(other)) && self.column(kind).is_none_or(|c| {
            c.shared(other.column(kind).unwrap())
                && (kind != Kind::Face
                    || match (&self.planes, &other.planes) {
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => c.rows().is_empty(),
                    })
        })
    }

    pub fn changed_slots<'a>(
        &'a self,
        other: &'a Self,
        kind: Kind,
    ) -> impl Iterator<Item = u32> + 'a {
        let a = self.column(kind).unwrap();
        let b = other.column(kind).unwrap();
        let arcs=|c:&'a Self| c.arcs.iter().filter(move |_|kind==Kind::Edge).flat_map(|a|a.keys().copied());
        arcs(self).chain(arcs(other)).filter(move |&s|!self.same_binding(other,kind,s)).chain(a.rows()
            .iter()
            .filter(move |&&(s, _)| !self.same_binding(other, kind, s))
            .map(|r| r.0)
            .chain(
                b.rows()
                    .iter()
                    .filter(move |&&(s, _)| a.get(s) == NONE)
                    .map(|r| r.0),
            ))
            .chain(self.curved_changes(other, kind))
    }

    pub fn same_live(&self, other: &Self, kind: Kind, live: impl Fn(u32) -> bool) -> bool {
        match (&self.native, &other.native) {
            (None, None) => (),
            (Some(a), Some(b)) if Arc::ptr_eq(a, b) => (),
            (Some(a), Some(b))
                if serde_json::to_vec(&a.rows).ok() == serde_json::to_vec(&b.rows).ok() =>
            {
                ()
            }
            _ => return false,
        }
        if kind==Kind::Edge && self.arcs!=other.arcs && self.changed_slots(other,kind).any(&live){return false}
        if self.shared(other, kind) {
            return true;
        }
        if self.curved_changes(other, kind).into_iter().any(&live) {
            return false;
        }
        let a = self.column(kind).unwrap().rows();
        let b = other.column(kind).unwrap().rows();
        a.iter()
            .filter(|r| live(r.0))
            .map(|r| r.0)
            .eq(b.iter().filter(|r| live(r.0)).map(|r| r.0))
            && a.iter()
                .filter(|r| live(r.0))
                .all(|r| self.same_binding(other, kind, r.0))
    }

    pub fn alloc(&mut self, kind: Kind, slot: u32) {
        if kind==Kind::Edge {
            if let Some(arcs)=self.arcs.as_mut().filter(|a|a.contains_key(&slot)){
                Arc::make_mut(arcs).remove(&slot);
                if arcs.is_empty(){self.arcs=None}
            }
        }
        self.inherit(kind, slot, NONE);
        if self.curved.as_deref().is_some_and(|c| c.has(kind, slot)) {
            let c = Arc::make_mut(self.curved.as_mut().unwrap());
            match kind {
                Kind::Edge => {
                    c.edges.remove(&slot);
                }
                Kind::Face => {
                    c.faces.remove(&slot);
                }
                Kind::Vertex => (),
            }
            if c.edges.is_empty() && c.faces.is_empty() {
                self.curved = None;
            }
        }
    }

    pub fn get(&self, kind: Kind, slot: u32) -> u32 {
        self.column(kind).map_or(NONE, |c| c.get(slot))
    }

    pub fn inherit(&mut self, kind: Kind, slot: u32, shape: u32) {
        match kind {
            Kind::Face => self.face_surface.set(slot, shape),
            Kind::Edge => self.edge_exact_curve.set(slot, shape),
            Kind::Vertex => (),
        }
    }

    pub fn is_faceted(&self) -> bool {
        self.arcs.is_none()
            && self.native.is_none()
            && self.face_surface.0.is_none()
            && self.edge_exact_curve.0.is_none()
            && self.curved.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_binding_cost_at_ride_scale() {
        const FACES: u32 = 4_000_000;
        for (name, stride, count) in [
            ("faceted", 1, 0),
            ("distributed", 100, FACES / 100),
            ("clustered", 1, FACES / 100),
        ] {
            let mut exact = ExactColumns::default();
            for i in 0..FACES {
                exact.alloc(Kind::Face, i);
            }
            for i in 0..count {
                exact.inherit(Kind::Face, i * stride, i);
            }
            assert_eq!(exact.face_surface.rows().len(), count as usize);
            let capacity = exact
                .face_surface
                .0
                .as_ref()
                .map_or(0, |r| r.capacity() * std::mem::size_of::<(u32, u32)>());
            let now = std::time::Instant::now();
            let before = exact.clone();
            exact.inherit(Kind::Face, FACES - 1, 7);
            let edit = now.elapsed();
            assert_eq!(before.get(Kind::Face, FACES - 1), NONE);
            exact = before;
            assert_eq!(exact.is_faceted(), count == 0);
            println!("{name}: {count} face bindings, {capacity} bytes vector capacity, one snapshot + subsequent edit {edit:?}");
        }
    }
}

#[cfg(test)]
mod change_tests {
    use super::*;
    use crate::store::{edit::Change, Store, Weld};
    #[test]
    fn only_the_bound_face_is_modified() {
        let mut store = Store::empty(20, Weld::default());
        let face = store
            .add_face(
                &[vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]],
                &mut Change::default(),
            )
            .unwrap();
        let before = store.state.clone();
        store.state.exact.inherit(Kind::Face, face, 0);
        let diff = store.state.changes_since(&before);
        assert_eq!(diff.modified[2], vec![store.key(Kind::Face, face)]);
        assert!(diff.modified[1].is_empty());
        assert!(!store.state.same_as(&before));
    }
}

/// A plane-bound polygon can use the store's existing ordered rings as
/// implicit linear pcurves. Keep the pool root with the sparse bindings.
impl super::Store {
    pub(crate) fn validate_plane(
        &self,
        face: u32,
        plane: &crate::exact::Frame,
    ) -> Result<(), &'static str> {
        let rings = self.face_rings(face);
        let mut projected = Vec::new();
        for ring in rings {
            let mut uv = Vec::new();
            for slot in ring {
                let p = glam::DVec3::from(
                    self.state
                        .frame
                        .definition_point(*self.state.vpos.get(slot as usize)),
                );
                let d = p - plane.origin;
                // This implicit binding is for an already planar polygon.
                // Allow only roundoff at its definition-local coordinate scale.
                let roundoff = 64.
                    * f64::EPSILON
                    * (1. + p.abs().max_element() + plane.origin.abs().max_element());
                if d.dot(plane.axis).abs() > roundoff {
                    return Err("face misses its exact plane");
                }
                uv.push(glam::DVec2::new(d.dot(plane.reference), d.dot(plane.y())));
            }
            projected.push(uv);
        }
        crate::exact::validate_polygon(&projected)
    }
    pub(crate) fn bind_plane(
        &mut self,
        slot: u32,
        plane: crate::exact::Frame,
    ) -> Result<(), &'static str> {
        plane.validate()?;
        self.validate_plane(slot, &plane)?;
        self.state.exact.install_plane(slot, plane)?;
        self.revision += 1;
        Ok(())
    }
    pub(crate) fn bound_planes(&self) -> impl Iterator<Item = (u32, &crate::exact::Frame)> {
        self.state
            .exact
            .face_surface
            .rows()
            .iter()
            .filter_map(|&(slot, id)| {
                let p = self.state.exact.planes.as_ref()?.get(id as usize)?;
                (self.alive(Kind::Face, slot) && self.validate_plane(slot, p).is_ok())
                    .then_some((slot, p))
            })
    }
}

impl ExactColumns {
    pub(super) fn install_plane(
        &mut self,
        slot: u32,
        plane: crate::exact::Frame,
    ) -> Result<(), &'static str> {
        let exact = self;
        let planes = exact.planes.get_or_insert_with(Arc::default);
        let index = match planes.iter().position(|p| *p == plane) {
            Some(i) => i,
            None => {
                if planes.len() >= NONE as usize {
                    return Err("exact plane pool is full");
                }
                let v = Arc::make_mut(planes);
                v.push(plane);
                v.len() - 1
            }
        };
        exact.inherit(Kind::Face, slot, index as u32);
        Ok(())
    }
}
impl super::Store {
    pub(super) fn discard_invalid_planes(&mut self) {
        if let Some(arcs)=&self.state.exact.arcs{
            let invalid:Vec<_>=arcs.iter().filter(|(e,a)|self.alive(Kind::Edge,**e)&&!self.arc_matches(**e,a)).map(|(&e,_)|e).collect();
            for e in invalid {self.state.exact.alloc(Kind::Edge,e)}
        }
        let Some(planes) = &self.state.exact.planes else {
            return;
        };
        let invalid: Vec<_> = self
            .state
            .exact
            .face_surface
            .rows()
            .iter()
            .filter_map(|&(slot, r)| {
                (self.alive(Kind::Face, slot)
                    && planes
                        .get(r as usize)
                        .is_none_or(|p| self.validate_plane(slot, p).is_err()))
                .then_some(slot)
            })
            .collect();
        for f in invalid {
            self.state.exact.inherit(Kind::Face, f, NONE);
        }
    }
}

#[cfg(test)]
mod curved_cost {
    use super::*;
    use crate::exact::bound::Support;
    use crate::exact::{Frame, Surface};

    /// 4 M face slots, 1 % bound (distributed and clustered): a step that
    /// binds one more face copies one chunk, and comparing the two states
    /// visits only that chunk.
    #[test]
    fn one_binding_edit_at_ride_scale_copies_one_chunk() {
        let support = Support::new(Surface::Cylinder {
            frame: Frame { origin: glam::DVec3::ZERO, axis: glam::DVec3::Z, reference: glam::DVec3::X },
            radius: 1.,
        });
        for (name, stride) in [("distributed", 100u32), ("clustered", 1)] {
            let mut c = Curved::default();
            for i in 0..40_000u32 {
                c.faces.insert(i * stride, support.clone());
            }
            assert_eq!(c.faces.len(), 40_000);
            assert!(!c.faces.is_empty());
            let before = Arc::new(c);
            let now = std::time::Instant::now();
            let mut next = (*before).clone();
            next.faces.insert(3_999_999, support.clone());
            let edit = now.elapsed();
            let now = std::time::Instant::now();
            let changed = next.changed(&before, Kind::Face);
            let diff = now.elapsed();
            assert_eq!(changed, vec![3_999_999]);
            let full = std::time::Instant::now();
            let _copy: Vec<(u32, Support)> = before.faces.iter().map(|(k, v)| (*k, v.clone())).collect();
            let copy = full.elapsed();
            println!("{name}: snapshot+edit {edit:?}, diff {diff:?} (a full copy of the map: {copy:?})");
            assert!(edit < copy && diff < copy);
        }
    }
}

/// Carry analytic parameters through a rigid change of definition coordinates.
/// UVs, curve intervals and the slot-to-support maps keep their meaning.
impl ExactColumns {
    pub(super) fn change_axes(&mut self, m: glam::DMat4) -> Result<(), &'static str> {
        use crate::exact::{Curve, Frame, Surface};
        let point = |p| m.transform_point3(p);
        let vector = |v| m.transform_vector3(v);
        let frame = |f: &mut Frame| {
            f.origin = point(f.origin);
            f.axis = vector(f.axis);
            f.reference = vector(f.reference);
        };
        let curve = |c: &mut Curve| match c {
            Curve::Line { origin, direction } => {
                *origin = point(*origin);
                *direction = vector(*direction);
            }
            Curve::Ellipse { frame: f, .. } | Curve::Circle { frame: f, .. } => frame(f),
            Curve::Spline { controls, .. } => controls
                .iter_mut()
                .for_each(|p| *p = point(p.truncate()).extend(p.w)),
        };
        let surface = |s: &mut Surface| match s {
            Surface::Plane { frame: f }
            | Surface::Sphere { frame: f, .. }
            | Surface::Cone { frame: f, .. }
            | Surface::Cylinder { frame: f, .. }
            | Surface::Torus { frame: f, .. } => frame(f),
            Surface::Spline { controls, .. } => controls
                .iter_mut()
                .for_each(|p| *p = point(p.truncate()).extend(p.w)),
            Surface::Extrusion { vector: v, .. } => *v = vector(*v),
            Surface::Revolution { origin, axis, .. } => {
                *origin = point(*origin);
                *axis = vector(*axis);
            }
        };
        if let Some(native) = &mut self.native {
            let mut rows = native.rows.clone();
            if let Some(t) = &mut rows.topology {
                for p in t.vertices.values_mut() {
                    *p = point(*p);
                }
            }
            rows.curves.iter_mut().for_each(curve);
            rows.surfaces.iter_mut().for_each(surface);
            *native = Arc::new(super::native::NativePatch::new(rows)?);
        }
        if let Some(planes) = &mut self.planes {
            Arc::make_mut(planes).iter_mut().for_each(frame);
        }
        if let Some(arcs) = &mut self.arcs {
            for arc in Arc::make_mut(arcs).values_mut() {
                frame(Arc::make_mut(&mut arc.frame));
            }
        }
        if let Some(curved) = &mut self.curved {
            let c = Arc::make_mut(curved);
            for chunk in c.edges.chunks.iter_mut().flatten() {
                for e in Arc::make_mut(chunk).values_mut() {
                    curve(&mut e.curve);
                }
            }
            for chunk in c.faces.chunks.iter_mut().flatten() {
                for f in Arc::make_mut(chunk).values_mut() {
                    surface(&mut f.surface);
                    if let Some(g) = &mut f.generator {
                        curve(g);
                    }
                }
            }
        }
        Ok(())
    }
}
