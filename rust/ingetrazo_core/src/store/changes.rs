// SPDX-License-Identifier: GPL-3.0-or-later
//! Net geometry change of a compound edit (including its stitch and orientation).
use super::{edit::Change, Store};
#[cfg(test)]
use super::keys::Kind;

impl Store {
    #[cfg(test)]
    fn same_references(
        &self,
        before: &Store,
        kind: Kind,
        a: impl Iterator<Item = u32>,
        b: impl Iterator<Item = u32>,
    ) -> bool {
        a.map(|s| self.key(kind, s))
            .eq(b.map(|s| before.key(kind, s)))
    }

    /// The net change from `before` to this store, by key: `State::changes_since`
    /// (store/diff.rs), which skips the chunks both still share, so a History
    /// step costs O(chunks it wrote) rather than O(elements).
    pub fn changes_since(&self, before: &Store) -> Change {
        let mut change = self.state.changes_since(&before.state);
        change.revision = self.revision;
        change
    }

    /// Element by element (the first version): kept as the oracle the
    /// chunked diff is tested against.
    #[cfg(test)]
    pub fn changes_since_full(&self, before: &Store) -> Change {
        let mut change = Change {
            revision: self.revision,
            ..Default::default()
        };
        let a = &self.state;
        let b = &before.state;
        for kind in Kind::ALL {
            let k = kind as usize;
            for s in before.live_slots(kind) {
                let key = before.key(kind, s);
                if self.slot(kind, key).is_none() {
                    change.removed[k].push(key);
                }
            }
            for s in self.live_slots(kind) {
                let i = s as usize;
                let key = self.key(kind, s);
                if before.slot(kind, key).is_none() {
                    change.created[k].push(key);
                    continue;
                }
                let modified = match kind {
                    Kind::Vertex => {
                        if a.vpos.get(i) != b.vpos.get(i) {
                            change.moved.push(key);
                        }
                        !self.same_references(
                            before,
                            Kind::Edge,
                            self.edges_at(s),
                            before.edges_at(s),
                        )
                    }
                    Kind::Edge => {
                        a.eflags.get(i) != b.eflags.get(i)
                            || a.ecurve.get(i) != b.ecurve.get(i)
                            || a.elayer.get(i) != b.elayer.get(i)
                            || !self.same_references(
                                before,
                                Kind::Vertex,
                                a.ends.get(i).iter().copied(),
                                b.ends.get(i).iter().copied(),
                            )
                            || !self.same_references(
                                before,
                                Kind::Face,
                                self.edge_faces(s).iter().copied(),
                                before.edge_faces(s).iter().copied(),
                            )
                    }
                    Kind::Face => {
                        let left = self.face_rings(s);
                        let right = before.face_rings(s);
                        a.fflags.get(i) != b.fflags.get(i)
                            || a.fattr.get(i) != b.fattr.get(i)
                            || left.len() != right.len()
                            || left.iter().zip(&right).any(|(x, y)| {
                                !self.same_references(
                                    before,
                                    Kind::Vertex,
                                    x.iter().copied(),
                                    y.iter().copied(),
                                )
                            })
                    }
                };
                if modified {
                    change.modified[k].push(key);
                }
            }
        }
        change
    }
}
