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

    pub fn shared(&self, other: &Self, kind: Kind) -> bool {
        self.column(kind)
            .is_none_or(|c| c.shared(other.column(kind).unwrap()))
    }

    pub fn changed_slots<'a>(
        &'a self,
        other: &'a Self,
        kind: Kind,
    ) -> impl Iterator<Item = u32> + 'a {
        let a = self.column(kind).unwrap();
        let b = other.column(kind).unwrap();
        a.rows()
            .iter()
            .filter(move |&&(s, r)| b.get(s) != r)
            .map(|r| r.0)
            .chain(
                b.rows()
                    .iter()
                    .filter(move |&&(s, _)| a.get(s) == NONE)
                    .map(|r| r.0),
            )
    }

    pub fn same_live(&self, other: &Self, kind: Kind, live: impl Fn(u32) -> bool) -> bool {
        if self.shared(other, kind) {
            return true;
        }
        let a = self.column(kind).unwrap().rows();
        let b = other.column(kind).unwrap().rows();
        a.iter()
            .filter(|r| live(r.0))
            .eq(b.iter().filter(|r| live(r.0)))
    }

    pub fn alloc(&mut self, kind: Kind, slot: u32) {
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
        self.face_surface.0.is_none() && self.edge_exact_curve.0.is_none()
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
