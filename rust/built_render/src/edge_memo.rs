//! A big prototype's edge expansion kept between rebakes and patched.
//!
//! `edges::expand` hashes every side of every face triangle to find which
//! triangles meet along an edge (their normals make the edge records and
//! the profile candidates). An edit of a 200k-face prototype rebakes it
//! whole and changes a few triangles: the side table is kept here, with
//! every triangle's normal on each side (not only the first and last), so
//! the triangles and hard edges the new bake no longer has are taken out
//! of it and the new ones put in. The streams are paired element by
//! element (`align::runs`): what is paired is equal, so whatever was
//! derived from it is still right.
//!
//! The records of a fresh memo are `expand`'s, byte for byte. After an
//! update the hard records are still in input order and the same as a
//! fresh expansion's; the profile candidates are the same set, the new
//! ones last (drawing order among them does not change a pixel).
use crate::edges::{key, shard_of, words, FastMap, Words};

/// Past this many triangles a prototype's expansion is kept.
pub(crate) const KEEP_PAST: usize = 1 << 16;
const R: usize = 48; // one edge record
const WINDOW: usize = 512;

#[derive(Default, Clone)]
struct Entry {
    key: [u8; 24],
    /// The normals of the triangles on this side with their triangles'
    /// ranks (stream order), ascending; the first two inline.
    ns: [(u64, [f32; 3]); 2],
    len: u32,
    more: Vec<(u64, [f32; 3])>,
    /// Hard edges with this key.
    hard: u32,
}

impl Entry {
    fn get(&self, i: usize) -> (u64, [f32; 3]) {
        if i < 2 { self.ns[i] } else { self.more[i - 2] }
    }
    fn set(&mut self, i: usize, v: (u64, [f32; 3])) {
        if i < 2 { self.ns[i] = v } else { self.more[i - 2] = v }
    }
    /// In rank order (a rebuild meets triangles in that order).
    fn push(&mut self, rank: u64, n: [f32; 3]) {
        let len = self.len as usize;
        if len >= 2 {
            self.more.push((0, [0.; 3]));
        }
        self.len += 1;
        let mut at = len;
        while at > 0 && self.get(at - 1).0 > rank {
            let prev = self.get(at - 1);
            self.set(at, prev);
            at -= 1;
        }
        self.set(at, (rank, n));
    }
    fn remove(&mut self, rank: u64) {
        let len = self.len as usize;
        let Some(at) = (0..len).find(|&i| self.get(i).0 == rank) else { return };
        for i in at..len - 1 {
            let next = self.get(i + 1);
            self.set(i, next);
        }
        if len > 2 {
            self.more.pop();
        } else {
            self.ns[len - 1] = (0, [0.; 3]);
        }
        self.len -= 1;
    }
    /// The two normals `expand` keeps: the first non-zero one, then the
    /// last one met after it.
    fn normals(&self) -> [[f32; 3]; 2] {
        let len = self.len as usize;
        let Some(k) = (0..len).find(|&i| self.get(i).1 != [0.; 3]) else { return [[0.; 3]; 2] };
        [self.get(k).1, if len - 1 > k { self.get(len - 1).1 } else { [0.; 3] }]
    }
    fn profile(&self) -> bool {
        let n = self.normals();
        n[0] != [0.; 3] && self.hard == 0
            && (n[1] == [0.; 3] || glam::Vec3::from(n[0]).dot(glam::Vec3::from(n[1])) < 0.99999)
    }
}

/// Ranks of a fresh stream's triangles, spaced so that later insertions
/// fit between.
const SPACING: u64 = 1 << 24;

#[derive(Default)]
struct Shard {
    lookup: FastMap<Words, u32>,
    entries: Vec<Entry>,
}

pub(crate) struct Memo {
    /// The expanded input's positions: each face triangle's three corners,
    /// each hard edge's two ends (what the records follow from).
    tris: Vec<[u8; 36]>,
    pairs: Vec<[u8; 24]>,
    shards: Vec<Shard>,
    /// The edge records: one per hard edge in input order, then the
    /// profile candidates (`soft`, by key).
    pub(crate) records: Vec<u8>,
    pub(crate) hard: usize,
    soft: Vec<[u8; 24]>,
    soft_at: FastMap<Words, u32>,
    /// Each triangle's rank, in stream order.
    ranks: Vec<u64>,
}

fn point(v: &[u8], vertex: usize) -> [u8; 12] {
    v[vertex * 28..vertex * 28 + 12].try_into().unwrap()
}

fn corners(v: &[u8], t: usize) -> [u8; 36] {
    let mut out = [0u8; 36];
    for k in 0..3 {
        out[k * 12..k * 12 + 12].copy_from_slice(&v[(3 * t + k) * 28..(3 * t + k) * 28 + 12]);
    }
    out
}

fn ends(v: &[u8], faces: usize, e: usize) -> [u8; 24] {
    let mut out = [0u8; 24];
    out[..12].copy_from_slice(&v[(faces + 2 * e) * 28..(faces + 2 * e) * 28 + 12]);
    out[12..].copy_from_slice(&v[(faces + 2 * e + 1) * 28..(faces + 2 * e + 1) * 28 + 12]);
    out
}

fn normal(points: &[[u8; 12]; 3]) -> [f32; 3] {
    let p = |i: usize| glam::Vec3::from_array(bytemuck::pod_read_unaligned::<[f32; 3]>(&points[i]));
    (p(1) - p(0)).cross(p(2) - p(0)).normalize_or_zero().to_array()
}

fn record(out: &mut [u8], k: &[u8; 24], ends: (&[u8], &[u8]), normals: [[f32; 3]; 2], soft: bool) {
    out[..12].copy_from_slice(ends.0);
    out[12..24].copy_from_slice(ends.1);
    let _ = k;
    let n0 = normals[0].map(|x| if soft { x * 2. } else { x });
    out[24..36].copy_from_slice(bytemuck::cast_slice(&n0));
    out[36..48].copy_from_slice(bytemuck::cast_slice(&normals[1]));
}

impl Memo {
    fn find(&self, w: &Words) -> Option<&Entry> {
        let s = &self.shards[shard_of(w, self.shards.len())];
        s.lookup.get(w).map(|&at| &s.entries[at as usize])
    }

    fn entry(&mut self, k: &[u8; 24]) -> &mut Entry {
        let w = words(k);
        let n = self.shards.len();
        let s = &mut self.shards[shard_of(&w, n)];
        let at = *s.lookup.entry(w).or_insert_with(|| {
            s.entries.push(Entry { key: *k, ..Entry::default() });
            (s.entries.len() - 1) as u32
        });
        &mut s.entries[at as usize]
    }

    /// The expansion of `v` (`faces` face vertices, the rest hard-edge
    /// pairs, indices trivial): `expand`'s records, and the memo.
    pub(crate) fn build(v: &[u8], faces: usize) -> Memo {
        let shards = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 8);
        let tris = faces / 3;
        let pairs = (v.len() / 28 - faces) / 2;
        let build = |shard: usize| -> Shard {
            let mut own = Shard {
                lookup: FastMap::with_capacity_and_hasher(tris * 2 / shards + 16, Default::default()),
                entries: Vec::with_capacity(tris * 2 / shards + 16),
            };
            for t in 0..tris {
                let pts = [point(v, 3 * t), point(v, 3 * t + 1), point(v, 3 * t + 2)];
                let mut n = None;
                for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                    let k = key(&pts[a], &pts[b]);
                    let w = words(&k);
                    if shard_of(&w, shards) != shard {
                        continue;
                    }
                    let n = *n.get_or_insert_with(|| normal(&pts));
                    let at = *own.lookup.entry(w).or_insert_with(|| {
                        own.entries.push(Entry { key: k, ..Entry::default() });
                        (own.entries.len() - 1) as u32
                    });
                    own.entries[at as usize].push(t as u64 * SPACING, n);
                }
            }
            for e in 0..pairs {
                let k = key(&point(v, faces + 2 * e), &point(v, faces + 2 * e + 1));
                let w = words(&k);
                if shard_of(&w, shards) == shard {
                    if let Some(&at) = own.lookup.get(&w) {
                        own.entries[at as usize].hard += 1;
                    }
                }
            }
            own
        };
        let shards: Vec<Shard> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..shards).map(|s| scope.spawn(move || build(s))).collect();
            handles.into_iter().map(|h| h.join().expect("an edge shard")).collect()
        });
        let mut memo = Memo {
            tris: (0..tris).map(|t| corners(v, t)).collect(),
            pairs: (0..pairs).map(|e| ends(v, faces, e)).collect(),
            shards,
            records: Vec::new(),
            hard: pairs,
            soft: Vec::new(),
            soft_at: FastMap::default(),
            ranks: (0..tris as u64).map(|t| t * SPACING).collect(),
        };
        let soft: Vec<[u8; 24]> = memo.shards.iter().flat_map(|s| s.entries.iter()).filter(|e| e.profile()).map(|e| e.key).collect();
        for (i, k) in soft.iter().enumerate() {
            memo.soft_at.insert(words(k), i as u32);
        }
        memo.soft = soft;
        let mut records = vec![0u8; (pairs + memo.soft.len()) * R];
        for e in 0..pairs {
            let (a, b) = (&v[(faces + 2 * e) * 28..(faces + 2 * e) * 28 + 12], &v[(faces + 2 * e + 1) * 28..(faces + 2 * e + 1) * 28 + 12]);
            let k = key(a, b);
            let n = memo.find(&words(&k)).map_or([[0.; 3]; 2], Entry::normals);
            record(&mut records[e * R..(e + 1) * R], &k, (a, b), n, false);
        }
        for (i, k) in memo.soft.iter().enumerate() {
            let n = memo.find(&words(k)).map_or([[0.; 3]; 2], Entry::normals);
            record(&mut records[(pairs + i) * R..(pairs + i + 1) * R], k, (&k[..12], &k[12..]), n, true);
        }
        memo.records = records;
        memo
    }

    /// This memo brought to the next bake `v` of the same prototype: the
    /// byte ranges of the records that differ from what was there before at
    /// the same offsets (what an upload into the same buffer range must
    /// write), or None when too much changed.
    pub(crate) fn update(&mut self, v: &[u8], faces: usize) -> Option<Vec<std::ops::Range<usize>>> {
        let (nto, ntn, npo) = (self.tris.len(), faces / 3, self.pairs.len());
        let npn = (v.len() / 28 - faces) / 2;
        let same_tri = |i: usize, j: usize| (0..3).all(|k| self.tris[i][k * 12..k * 12 + 12] == v[(3 * j + k) * 28..(3 * j + k) * 28 + 12]);
        let same_pair = |i: usize, j: usize| self.pairs[i][..12] == v[(faces + 2 * j) * 28..(faces + 2 * j) * 28 + 12]
            && self.pairs[i][12..] == v[(faces + 2 * j + 1) * 28..(faces + 2 * j + 1) * 28 + 12];
        let tri_runs = crate::align::runs_by(nto, ntn, same_tri, WINDOW, 4)?;
        let pair_runs = crate::align::runs_by(npo, npn, same_pair, WINDOW, 4)?;
        let unpaired = |n: usize, runs: &[(usize, usize, usize)], old: bool| -> Vec<usize> {
            let mut paired = vec![false; n];
            for &(o, k, len) in runs {
                let at = if old { o } else { k };
                paired[at..at + len].iter_mut().for_each(|p| *p = true);
            }
            (0..n).filter(|&i| !paired[i]).collect()
        };
        let (gone_t, new_ts) = (unpaired(nto, &tri_runs, true), unpaired(ntn, &tri_runs, false));
        let (gone_p, new_ps) = (unpaired(npo, &pair_runs, true), unpaired(npn, &pair_runs, false));
        if gone_t.len() + new_ts.len() + gone_p.len() + new_ps.len() > 4096 + ntn / 16 {
            return None;
        }
        // Ranks: kept by the paired triangles, spread between their
        // neighbours' for the new ones (none to spare: rebuild).
        let mut ranks = vec![u64::MAX; ntn];
        for &(o, k, len) in &tri_runs {
            ranks[k..k + len].copy_from_slice(&self.ranks[o..o + len]);
        }
        let mut j = 0;
        while j < ntn {
            if ranks[j] != u64::MAX {
                j += 1;
                continue;
            }
            let start = j;
            while j < ntn && ranks[j] == u64::MAX {
                j += 1;
            }
            let low = if start == 0 { 0 } else { ranks[start - 1] + 1 };
            let high = if j == ntn { low.saturating_add(SPACING * (j - start + 1) as u64) } else { ranks[j] };
            let count = (j - start) as u64;
            if high <= low || high - low < count + 1 {
                return None;
            }
            let step = (high - low) / (count + 1);
            for (i, r) in ranks[start..j].iter_mut().enumerate() {
                *r = low + step * (i as u64 + 1) - 1;
            }
        }
        // Each side touched, with what its records showed before.
        let mut before: FastMap<Words, ([[f32; 3]; 2], bool)> = FastMap::default();
        let mut touch = |memo: &mut Memo, k: &[u8; 24]| -> Words {
            let w = words(k);
            if !before.contains_key(&w) {
                let was = memo.find(&w).map_or(([[0.; 3]; 2], false), |e| (e.normals(), e.profile()));
                before.insert(w, was);
            }
            w
        };
        let sides = |c: &[u8; 36]| -> ([[u8; 12]; 3], [[u8; 24]; 3]) {
            let pts: [[u8; 12]; 3] = std::array::from_fn(|k| c[k * 12..k * 12 + 12].try_into().unwrap());
            (pts, [key(&pts[0], &pts[1]), key(&pts[1], &pts[2]), key(&pts[2], &pts[0])])
        };
        let old_tris = std::mem::take(&mut self.tris);
        let old_pairs = std::mem::take(&mut self.pairs);
        for &t in &gone_t {
            let (_, ks) = sides(&old_tris[t]);
            let rank = self.ranks[t];
            for k in &ks {
                touch(self, k);
                self.entry(k).remove(rank);
            }
        }
        for &t in &new_ts {
            let (pts, ks) = sides(&corners(v, t));
            let n = normal(&pts);
            for k in &ks {
                touch(self, k);
                self.entry(k).push(ranks[t], n);
            }
        }
        let pair_key = |s: &[u8], first: usize, e: usize| key(&point(s, first + 2 * e), &point(s, first + 2 * e + 1));
        for &e in &gone_p {
            let k = key(&old_pairs[e][..12], &old_pairs[e][12..]);
            touch(self, &k);
            let entry = self.entry(&k);
            entry.hard = entry.hard.saturating_sub(1);
        }
        for &e in &new_ps {
            let k = pair_key(v, faces, e);
            touch(self, &k);
            self.entry(&k).hard += 1;
        }
        // Sides whose records must change: new normals, or in or out of the
        // profile candidates.
        let mut renormal: FastMap<Words, [[f32; 3]; 2]> = FastMap::default();
        let mut leave: Vec<Words> = Vec::new();
        let mut join: Vec<[u8; 24]> = Vec::new();
        for (w, (was, soft)) in &before {
            let (now, profile, k) = match self.find(w) {
                Some(e) => (e.normals(), e.profile(), e.key),
                None => ([[0.; 3]; 2], false, [0; 24]),
            };
            if now.map(|n| n.map(f32::to_bits)) != was.map(|n| n.map(f32::to_bits)) {
                renormal.insert(*w, now);
            }
            match (*soft, profile) {
                (true, false) => leave.push(*w),
                (false, true) => join.push(k),
                _ => {}
            }
        }
        // The hard records, in the new input order.
        let old_records = std::mem::take(&mut self.records);
        let old_soft = &old_records[self.hard * R..];
        let mut records = vec![0u8; npn * R];
        let mut fresh = vec![true; npn];
        for &(o, k, len) in &pair_runs {
            records[k * R..(k + len) * R].copy_from_slice(&old_records[o * R..(o + len) * R]);
            fresh[k..k + len].iter_mut().for_each(|f| *f = false);
        }
        for e in 0..npn {
            if !fresh[e] && renormal.is_empty() {
                continue;
            }
            let k = pair_key(v, faces, e);
            let w = words(&k);
            let n = match renormal.get(&w) {
                Some(n) => *n,
                None if !fresh[e] => continue,
                None => self.find(&w).map_or([[0.; 3]; 2], Entry::normals),
            };
            let (a, b) = (&v[(faces + 2 * e) * 28..(faces + 2 * e) * 28 + 12], &v[(faces + 2 * e + 1) * 28..(faces + 2 * e + 1) * 28 + 12]);
            record(&mut records[e * R..(e + 1) * R], &k, (a, b), n, false);
        }
        // The profile candidates: those that left go (the last one takes
        // their place), the new ones go last, the rest keep their records
        // unless their normals moved.
        let mut soft_records: Vec<u8> = old_soft.to_vec();
        for w in leave {
            if let Some(at) = self.soft_at.remove(&w) {
                let at = at as usize;
                let last = self.soft.len() - 1;
                self.soft.swap(at, last);
                self.soft.pop();
                if at != last {
                    let (lo, hi) = soft_records.split_at_mut(last * R);
                    lo[at * R..(at + 1) * R].copy_from_slice(&hi[..R]);
                    self.soft_at.insert(words(&self.soft[at]), at as u32);
                }
                soft_records.truncate(last * R);
            }
        }
        for k in join {
            self.soft_at.insert(words(&k), self.soft.len() as u32);
            self.soft.push(k);
            soft_records.extend_from_slice(&[0u8; R]);
            renormal.entry(words(&k)).or_insert_with(|| [[0.; 3]; 2]);
        }
        for (w, _) in renormal.iter() {
            if let Some(&at) = self.soft_at.get(w) {
                let k = self.soft[at as usize];
                let n = self.find(w).map_or([[0.; 3]; 2], Entry::normals);
                record(&mut soft_records[at as usize * R..(at as usize + 1) * R], &k, (&k[..12], &k[12..]), n, true);
            }
        }
        records.extend_from_slice(&soft_records);
        // What an upload over the old ranges must write.
        let changed = |old: &[u8], new: &[u8], unit: usize| -> Vec<std::ops::Range<usize>> {
            let mut out: Vec<std::ops::Range<usize>> = Vec::new();
            let n = new.len() / unit;
            let mut i = 0;
            while i < n {
                let same = |i: usize| (i + 1) * unit <= old.len() && old[i * unit..(i + 1) * unit] == new[i * unit..(i + 1) * unit];
                if same(i) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < n && !same(i) {
                    i += 1;
                }
                match out.last_mut() {
                    Some(last) if start * unit <= last.end + 4096 => last.end = i * unit,
                    _ => out.push(start * unit..i * unit),
                }
            }
            out
        };
        let record_changes = changed(&old_records, &records, R);
        let mut tris = vec![[0u8; 36]; ntn];
        let mut fresh_t = vec![true; ntn];
        for &(o, k, len) in &tri_runs {
            tris[k..k + len].copy_from_slice(&old_tris[o..o + len]);
            fresh_t[k..k + len].iter_mut().for_each(|f| *f = false);
        }
        for t in 0..ntn {
            if fresh_t[t] {
                tris[t] = corners(v, t);
            }
        }
        let mut pairs = vec![[0u8; 24]; npn];
        for &(o, k, len) in &pair_runs {
            pairs[k..k + len].copy_from_slice(&old_pairs[o..o + len]);
        }
        for e in 0..npn {
            if fresh[e] {
                pairs[e] = ends(v, faces, e);
            }
        }
        self.tris = tris;
        self.pairs = pairs;
        self.hard = npn;
        self.records = records;
        self.ranks = ranks;
        Some(record_changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Face vertices of an `n`x`n` grid of quads (two triangles each) at
    /// heights `z(i, j)`, then the quads' first sides as hard edges.
    fn bake(n: usize, z: impl Fn(usize, usize) -> f32, skip: &[usize]) -> (Vec<u8>, usize) {
        let vtx = |x: f32, y: f32, z: f32| -> Vec<u8> {
            let mut b = bytemuck::cast_slice(&[x, y, z, 0., 0., 1.]).to_vec();
            b.extend_from_slice(&[9, 9, 9, 255]);
            b
        };
        let (mut faces, mut pairs) = (Vec::new(), Vec::new());
        for i in 0..n {
            for j in 0..n {
                if skip.contains(&(i * n + j)) {
                    continue;
                }
                let (x, y) = (i as f32, j as f32);
                let c = [(x, y, z(i, j)), (x + 1., y, z(i + 1, j)), (x + 1., y + 1., z(i + 1, j + 1)), (x, y + 1., z(i, j + 1))];
                for t in [[0, 1, 2], [0, 2, 3]] {
                    for k in t {
                        faces.extend(vtx(c[k].0, c[k].1, c[k].2));
                    }
                }
                pairs.extend(vtx(c[0].0, c[0].1, c[0].2));
                pairs.extend(vtx(c[1].0, c[1].1, c[1].2));
            }
        }
        let count = faces.len() / 28;
        faces.extend(pairs);
        (faces, count)
    }

    fn expanded(v: &[u8], faces: usize) -> Vec<u8> {
        let ix: Vec<u32> = (0..(v.len() / 28) as u32).collect();
        let source = [crate::Command { count: faces as u32, instances: 0, first: 0, base: 0, start: 0 },
            crate::Command { count: (v.len() / 28 - faces) as u32, instances: 0, first: faces as u32, base: 0, start: 0 }];
        crate::edges::expand(v, bytemuck::cast_slice(&ix), bytemuck::cast_slice(&source), 1).unwrap().0
    }

    fn split(records: &[u8], hard: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut soft: Vec<Vec<u8>> = records[hard * R..].chunks(R).map(|c| c.to_vec()).collect();
        soft.sort();
        (records[..hard * R].to_vec(), soft)
    }

    #[test]
    fn a_patched_expansion_is_a_fresh_one() {
        // A bumpy grid: profile candidates on every crease.
        let bump = |i: usize, j: usize| ((i * 7 + j * 3) % 5) as f32 * 0.1;
        let (v, faces) = bake(200, bump, &[]);
        let mut memo = Memo::build(&v, faces);
        assert_eq!(memo.records, expanded(&v, faces), "a fresh memo is expand's");
        // Cells gone, a corner raised: some triangles and edges new.
        let raised = |i: usize, j: usize| if (i, j) == (40, 40) { 2.0 } else { bump(i, j) };
        let (v2, faces2) = bake(200, raised, &[5, 6, 9000]);
        let rc = memo.update(&v2, faces2).expect("a small edit carries");
        let fresh = expanded(&v2, faces2);
        assert_eq!(memo.records.len(), fresh.len());
        let (a, b) = (split(&memo.records, memo.hard), split(&fresh, memo.hard));
        if a.0 != b.0 {
            let i = (0..memo.hard).find(|&i| a.0[i * R..(i + 1) * R] != b.0[i * R..(i + 1) * R]).unwrap();
            let f = |r: &[u8]| -> Vec<f32> { bytemuck::cast_slice::<u8, f32>(&r.to_vec()).to_vec() };
            panic!("hard record {i} of {}: memo {:?} fresh {:?}", memo.hard, f(&a.0[i * R..(i + 1) * R]), f(&b.0[i * R..(i + 1) * R]));
        }
        assert_eq!(a.1.len(), b.1.len(), "profile candidates");
        assert!(a.1 == b.1, "profile candidates differ");
        assert!(!rc.is_empty());
        // And back.
        memo.update(&v, faces).expect("undo carries");
        assert_eq!(split(&memo.records, memo.hard), split(&expanded(&v, faces), memo.hard));
    }
}
