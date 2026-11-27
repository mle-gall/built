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

    pub fn shared(&self, other: &Self, kind: Kind) -> bool {
        if kind==Kind::Edge && !match (&self.arcs,&other.arcs){(None,None)=>true,(Some(a),Some(b))=>Arc::ptr_eq(a,b),_=>false}{return false}
        self.column(kind).is_none_or(|c| {
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
        self.arcs.is_none() && self.native.is_none() && self.face_surface.0.is_none() && self.edge_exact_curve.0.is_none()
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
