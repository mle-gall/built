// SPDX-License-Identifier: GPL-3.0-or-later
//! Face and edge slots in the four reserved draws of a materialized mesh.
//! Only the faces touched by an edit and their smooth corner fans are baked.
use super::{
    plain_edit::Edit,
    sources::{self, Points},
};
use crate::{
    facegeom::{Hypot, V3},
    store::{
        keys::{Kind, NONE},
        py::PyMeshStore,
        State, Store,
    },
};
use numpy::{ndarray::Array2, PyArray1, PyReadonlyArray1, PyReadonlyArray2, PyReadwriteArray1};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList, PySlice},
};
use std::collections::{HashMap, HashSet};

type Obj<'p> = Bound<'p, PyAny>;
type Span = (u32, u32);
type Set<T> = HashSet<T, crate::store::cow::FxBuild>;
type Map<K, V> = HashMap<K, V, crate::store::cow::FxBuild>;

pub(super) struct Layout {
    code: usize,
    lineage: u64,
    revision: u64,
    state: State,
    anchor: [f64; 3],
    faces: Vec<Span>,
    edges: Vec<Span>,
    attrs: Vec<u32>,
    signatures: Vec<String>,
    draw: Vec<u8>,
    material: Py<PyAny>,
    material_len: usize,
    material_flag: u32,
    positions: Set<[u32; 3]>,
    classes: Vec<u8>,
    token: u64,
    visible_faces: u32,
    visible_end: u32,
}

struct Rows<'p> {
    attrs: Vec<u32>,
    rows: Vec<Obj<'p>>,
    signatures: Vec<String>,
    colors: Vec<[u8; 4]>,
    // 0 visible, 1 reveal-only, 2 suppressed by tag
    draw: Vec<u8>,
    edge_layers: Vec<bool>,
}

fn rows<'p>(mesh: &Obj<'p>, st: &State, tags: &Obj<'p>) -> PyResult<Rows<'p>> {
    let py = mesh.py();
    let mut rows = vec![PyDict::new(py).into_any()];
    rows.extend(
        mesh.getattr("_attr_table")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?,
    );
    let mut attrs = st.fattr.iter().copied().collect::<Vec<_>>();
    let mut signatures = rows
        .iter()
        .map(|v| Ok(v.repr()?.to_string()))
        .collect::<PyResult<Vec<_>>>()?;
    let mut intern: HashMap<_, _> = signatures
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, s)| (s, i as u32))
        .collect();
    for (key, row) in mesh.getattr("_attr_own")?.cast::<PyDict>()?.iter() {
        if row.is_none() {
            continue;
        }
        let Ok(key) = key.extract::<u128>() else {
            continue;
        };
        let Some((ns, Kind::Face, g, f)) = crate::store::keys::split_key(key) else {
            continue;
        };
        if ns != st.ns || !st.slots[2].alive(f) || st.slots[2].gen(f) != g {
            continue;
        }
        let repr = row.repr()?.to_string();
        attrs[f as usize] = *intern.entry(repr.clone()).or_insert_with(|| {
            signatures.push(repr);
            rows.push(row);
            (rows.len() - 1) as u32
        });
    }
    if attrs.iter().any(|&a| a as usize >= rows.len()) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "face attribute out of range",
        ));
    }
    let (palette, hidden) = sources::palette(&rows)?;
    let mut draw = Vec::new();
    for (row, hide) in rows.iter().zip(hidden) {
        draw.push(if tags.contains(row.call_method1("get", ("layer",))?)? {
            2
        } else {
            u8::from(hide)
        });
    }
    let colors = palette
        .rows()
        .into_iter()
        .map(|r| [r[0], r[1], r[2], r[3]])
        .collect();
    let edge_layers = st
        .layers
        .iter()
        .map(|l| tags.contains(l))
        .collect::<PyResult<_>>()?;
    Ok(Rows {
        attrs,
        rows,
        signatures,
        colors,
        draw,
        edge_layers,
    })
}

impl Rows<'_> {
    fn face(&self, f: u32) -> u8 {
        self.draw[self.attrs[f as usize] as usize]
    }
    fn edge(&self, st: &State, e: u32) -> u8 {
        let layer = *st.elayer.get(e as usize);
        if layer != NONE && self.edge_layers[layer as usize] {
            2
        } else {
            u8::from(st.eflags.get(e as usize) & 3 != 0)
        }
    }
}

fn point(st: &State, v: u32, anchor: [f64; 3]) -> [f64; 3] {
    let p = *st.vpos.get(v as usize);
    if st.frame.origin == anchor {
        p
    } else {
        std::array::from_fn(|k| p[k] + (st.frame.origin[k] - anchor[k]))
    }
}

fn position_key(st: &State, v: u32, anchor: [f64; 3]) -> [u32; 3] {
    point(st, v, anchor).map(|x| {
        let x = x as f32;
        if x == 0. {
            0
        } else {
            x.to_bits()
        }
    })
}

fn store_of<'p>(
    s: &Obj<'p>,
    mesh: &Obj<'p>,
    code: usize,
) -> PyResult<Option<Bound<'p, PyMeshStore>>> {
    let Some(own) = mesh.getattr_opt("__dict__")? else {
        return Ok(None);
    };
    let own = own.cast::<PyDict>()?;
    let Some(raw) = own.get_item("_store")? else {
        return Ok(None);
    };
    let Ok(raw) = raw.cast_into::<PyMeshStore>() else {
        return Ok(None);
    };
    let store = raw.borrow();
    if store.inner.n(Kind::Face) < 32768
        || store.inner.state.exact.native.is_some()
        || own.contains("_deferred_exact")?
        || own
            .get_item("_deferred_source")?
            .is_some_and(|v| !v.is_none())
        || mesh.getattr_opt("_instance_definition")?.is_some()
        || s.getattr("packed")?.getattr("bake_focus")?.is_truthy()?
    {
        return Ok(None);
    }
    let v = s
        .getattr("variants")?
        .get_item(mesh.as_ptr() as usize)?
        .get_item(code)?;
    if v.get_item(0)?.is_truthy()? || v.get_item(1)?.is_truthy()? {
        return Ok(None);
    }
    drop(store);
    Ok(Some(raw))
}

const TABLES: [&str; 4] = [
    "commands",
    "edge_commands",
    "hidden_commands",
    "hidden_edge_commands",
];

pub(super) fn seed(
    s: &Obj<'_>,
    mesh: &Obj<'_>,
    code: usize,
    initial: bool,
) -> PyResult<Option<Layout>> {
    let Some(raw) = store_of(s, mesh, code)? else {
        return Ok(None);
    };
    let borrow = raw.borrow();
    let store = &borrow.inner;
    let st = &store.state;
    let p = s.getattr("packed")?;
    if initial {
        let Some(stamps) = p.getattr_opt("_edit_sources")? else {
            return Ok(None);
        };
        let Some(stamp) = stamps.cast::<PyDict>()?.get_item(code)? else {
            return Ok(None);
        };
        if stamp.extract::<(u64, u64)>()? != (store.lineage, store.revision) {
            return Ok(None);
        }
    }
    let rows = rows(mesh, st, &s.getattr("_hidden_tags")?)?;
    let anchor = p.getattr("prototype_origins")?.get_item(code)?.extract()?;
    let mut counts = [0u32; 4];
    for (i, name) in TABLES.iter().enumerate() {
        counts[i] = p
            .getattr(*name)?
            .get_item(code)?
            .get_item("count")?
            .extract()?;
    }
    if counts.iter().map(|&n| u64::from(n)).sum::<u64>() > u64::from(u32::MAX) / 32 {
        return Ok(None);
    }
    let mut cursor = [
        0,
        counts[0],
        counts[0] + counts[1],
        counts[..3].iter().sum(),
    ];
    let starts = cursor;
    let mut faces = vec![(0, 0); st.slots[2].len()];
    let mut edges = vec![(0, 0); st.slots[1].len()];
    let mut positions = Set::default();
    for v in st.slots[0].live_slots() {
        if !positions.insert(position_key(st, v, anchor)) {
            return Ok(None);
        }
    }
    let ordered_faces = store.live_slots(Kind::Face);
    // Ear clipping emits at most n-2 triangles, with two extra bridge
    // endpoints per hole. When both draw counts reach their sum, every
    // face reached its bound and its span is known without triangulation.
    for &f in &ordered_faces {
        let draw = rows.face(f);
        if draw == 2 {
            continue;
        }
        let r = st.frings.get(f as usize);
        if r.n_rings == 0 {
            return Ok(None);
        }
        let corners = (0..r.n_rings)
            .map(|i| *st.ring_sizes.get((r.ring_start + i) as usize))
            .sum::<u32>();
        if corners < 3 {
            return Ok(None);
        }
        let n = (corners - 2 + 2 * (r.n_rings - 1)) * 3;
        let pos = &mut cursor[draw as usize * 2];
        faces[f as usize] = (*pos, n);
        *pos += n;
    }
    if [0, 2]
        .into_iter()
        .any(|i| cursor[i] - starts[i] != counts[i])
    {
        cursor = starts;
        let h = crate::hypot_of(
            &s.py()
                .import("core._native")?
                .call_method0("qt_hypot")?
                .extract::<String>()?,
        )?;
        let points: Vec<_> = (0..st.vpos.len())
            .map(|v| {
                let [x, y, z] = point(st, v as u32, anchor).map(|x| x as f32);
                V3(x, y, z)
            })
            .collect();
        for &f in &ordered_faces {
            let draw = rows.face(f);
            if draw == 2 {
                continue;
            }
            let rings = store.face_rings(f);
            let refs = rings.iter().map(Vec::as_slice).collect::<Vec<_>>();
            let n = crate::facegeom::resident_geometry(&points, &refs, h)
                .0
                .len() as u32
                * 3;
            let pos = &mut cursor[draw as usize * 2];
            faces[f as usize] = (*pos, n);
            *pos += n;
        }
    }
    // The full bake collapses duplicate pairs and synthesizes missing ring
    // sides. Counts prove the latter are absent only if pairs are unique.
    let mut pairs = Set::default();
    for e in store.live_slots(Kind::Edge) {
        let [a, b] = *st.ends.get(e as usize);
        if a != b && !pairs.insert((a.min(b), a.max(b))) {
            return Ok(None);
        }
        let draw = rows.edge(st, e);
        if draw == 2 || a == b {
            continue;
        }
        let pos = &mut cursor[draw as usize * 2 + 1];
        edges[e as usize] = (*pos, 2);
        *pos += 2;
    }
    if (0..4).any(|k| cursor[k] - starts[k] != counts[k]) {
        return Ok(None);
    }
    let py = s.py();
    let updates = p.getattr("material_updates")?;
    let (flag, bytes) = if let Some(row) = updates.cast::<PyDict>()?.get_item(code)? {
        (row.get_item(0)?.extract()?, row.get_item(1)?)
    } else {
        let mat = p.getattr("materials")?;
        let r: PyReadonlyArray1<u8> = mat.extract()?;
        let buf = r.as_slice()?;
        let word = |n: usize| {
            u32::from_le_bytes(
                buf[code * 16 + n * 4..code * 16 + n * 4 + 4]
                    .try_into()
                    .unwrap(),
            )
        };
        let start = word(1) as usize * 16;
        let len = word(2) as usize * 32;
        (
            word(3),
            mat.get_item(PySlice::new(py, start as isize, (start + len) as isize, 1))?,
        )
    };
    let len = bytes.len()?;
    let mut backing = vec![0u8; if len > 0 { len + 512 * 1024 } else { 0 }];
    let src: PyReadonlyArray1<u8> = bytes.extract()?;
    backing[..len].copy_from_slice(src.as_slice()?);
    let mut classes = vec![0; faces.len()];
    for (i, &(first, count)) in faces.iter().enumerate() {
        if count == 0 {
            continue;
        }
        let color = rows.colors[rows.attrs[i] as usize];
        let paint = if flag != 0 {
            [u32::from_le_bytes(color), 0, 0]
        } else if len != 0 {
            let at = first as usize * 32 + 16;
            std::array::from_fn(|k| {
                u32::from_le_bytes(backing[at + 4 * k..at + 4 * k + 4].try_into().unwrap())
            })
        } else {
            [0; 3]
        };
        classes[i] =
            u8::from(paint != [0; 3]) | (u8::from(paint == [u32::from_le_bytes(color), 0, 0]) << 1);
    }
    borrow.prepare_edit_queries(py)?;
    Ok(Some(Layout {
        code,
        lineage: store.lineage,
        revision: store.revision,
        state: st.clone(),
        anchor,
        faces,
        edges,
        material: PyArray1::from_vec(py, backing).into_any().unbind(),
        material_len: len,
        material_flag: flag,
        positions,
        classes,
        token: super::plain_edit::token(),
        visible_faces: counts[0],
        visible_end: counts[0] + counts[1],
        attrs: rows.attrs,
        signatures: rows.signatures,
        draw: rows.draw,
    }))
}

#[derive(Default)]
struct FaceData {
    vertices: Vec<u8>,
    materials: Vec<u8>,
    special: bool,
    matching: bool,
}

fn patch_faces<'p>(
    store: &Store,
    anchor: [f64; 3],
    rows: &Rows<'p>,
    wanted: &Set<u32>,
    paths: &Obj<'p>,
    h: Hypot,
) -> PyResult<Option<Map<u32, FaceData>>> {
    let st = &store.state;
    let py = paths.py();
    let mut neighbourhood = wanted.clone();
    for &f in wanted {
        for v in store.face_rings(f).into_iter().flatten() {
            for e in store.edges_at(v) {
                neighbourhood.extend(store.edge_faces(e));
            }
        }
    }
    let mut faces: Vec<_> = neighbourhood
        .into_iter()
        .filter(|&f| rows.face(f) != 2)
        .collect();
    faces.sort_unstable_by_key(|&f| st.list_rank(Kind::Face, f));
    let mut verts = Vec::new();
    let mut ids = Map::default();
    let mut corner = Vec::new();
    let mut sizes = Vec::new();
    let mut counts = Vec::new();
    let mut edge_set = Set::default();
    let mut exact = Vec::new();
    for &f in &faces {
        let rings = store.face_rings(f);
        counts.push(rings.len() as i64);
        let start = corner.len();
        for r in rings {
            sizes.push(r.len() as i64);
            for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                let id = *ids.entry(a).or_insert_with(|| {
                    verts.push(point(st, a, anchor));
                    (verts.len() - 1) as u32
                });
                corner.push(id);
                if let Some(e) = store.find_edge(a, b) {
                    edge_set.insert(e);
                } else if a != b {
                    return Ok(None);
                }
            }
        }
        if let Some(ns) = store.face_facet_normals(f) {
            exact.extend(ns.into_iter().map(|n| n.map(|x| x as f32)))
        } else {
            exact.resize(exact.len() + corner.len() - start, [f32::NAN; 3]);
        }
    }
    let mut edges: Vec<_> = edge_set.into_iter().collect();
    edges.sort_unstable_by_key(|&e| st.list_rank(Kind::Edge, e));
    let ends = edges
        .iter()
        .map(|&e| st.ends.get(e as usize).map(|v| ids[&v]))
        .collect::<Vec<_>>();
    let flags = edges
        .iter()
        .map(|&e| *st.eflags.get(e as usize) & 7)
        .collect::<Vec<_>>();
    let points: Vec<_> = verts
        .iter()
        .map(|p| V3(p[0] as f32, p[1] as f32, p[2] as f32))
        .collect();
    let attributes = faces
        .iter()
        .map(|&f| rows.attrs[f as usize] as i64)
        .collect::<Vec<_>>();
    let colors = attributes
        .iter()
        .map(|&a| rows.colors[a as usize])
        .collect::<Vec<_>>();
    let geo = crate::scene_bake::prepare_with(
        &points,
        &corner,
        &sizes,
        &counts,
        &colors,
        &vec![false; faces.len()],
        &ends,
        &flags,
        h,
        true,
        Some(&exact),
        None,
    );
    let ids = PyDict::new(py);
    for (i, path) in paths.try_iter()?.enumerate() {
        ids.set_item(path?, i + 1)?;
    }
    let wide = Points::Double(Array2::from_shape_fn((verts.len(), 3), |(i, j)| {
        verts[i][j]
    }));
    let (maps, paints) = sources::material_columns(
        py,
        &wide,
        &corner.iter().map(|&c| i64::from(c)).collect::<Vec<_>>(),
        &sizes,
        &counts,
        &rows.rows,
        &attributes,
        &ids,
        anchor,
    )?;
    if ids.len() != paths.len()? {
        return Ok(None);
    }
    let maps: PyReadonlyArray2<f32> = maps.extract()?;
    let maps = maps.as_array();
    let paints: PyReadonlyArray2<u32> = paints.extract()?;
    let paints = paints.as_array();
    let mut data = Map::<u32, FaceData>::default();
    for ((tri, ns, col), i) in geo.triangles.into_iter().zip(geo.faces) {
        let f = faces[i];
        if !wanted.contains(&f) {
            continue;
        }
        let d = data.entry(f).or_default();
        let mapping = std::array::from_fn(|k| maps[[i, k]]);
        let paint = std::array::from_fn(|k| paints[[i, k]]);
        d.special = paint != [0; 3];
        d.matching = paint == [u32::from_le_bytes(col), 0, 0];
        for (v, n) in tri.into_iter().zip(ns) {
            let p = points[v as usize];
            for x in [p.0, p.1, p.2, n.0, n.1, n.2] {
                d.vertices.extend(x.to_le_bytes());
            }
            d.vertices.extend(col);
            d.materials
                .extend(crate::scene_bake::material_vertex(p, &mapping, paint));
        }
    }
    Ok(Some(data))
}

pub(super) fn ranges(
    layout: &Layout,
    mesh: &Obj<'_>,
    faces: &Obj<'_>,
) -> PyResult<Option<Vec<(usize, u32, u32)>>> {
    let Some(raw) = mesh
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_store")?
    else {
        return Ok(None);
    };
    let Ok(raw) = raw.cast_into::<PyMeshStore>() else {
        return Ok(None);
    };
    let store = raw.borrow();
    if store.inner.lineage != layout.lineage || store.inner.revision != layout.revision {
        return Ok(None);
    }
    let mut result = Vec::new();
    for f in faces.try_iter()? {
        if let Some(slot) = store.inner.slot(Kind::Face, f?.getattr("key")?.extract()?) {
            let (start, n) = layout.faces[slot as usize];
            if n != 0 && start < layout.visible_faces {
                result.push((layout.code, start / 3, n / 3));
            }
        }
    }
    Ok(Some(result))
}

pub(super) fn apply(
    s: &Obj<'_>,
    changed: &Obj<'_>,
    renderer: Option<&Obj<'_>>,
    l: &mut Layout,
) -> PyResult<bool> {
    if changed.len()? != 1 {
        return Ok(false);
    }
    let entry = changed.get_item(0)?;
    let mesh = entry.get_item(0)?;
    let code = entry.get_item(2)?.extract::<usize>()?;
    if code != l.code {
        return Ok(false);
    }
    let Some(raw) = store_of(s, &mesh, code)? else {
        return Ok(false);
    };
    let borrow = raw.borrow();
    let store = &borrow.inner;
    let st = &store.state;
    let p = s.getattr("packed")?;
    let py = s.py();
    if store.lineage != l.lineage
        || st.frame != l.state.frame
        || p.getattr("prototype_origins")?
            .get_item(code)?
            .extract::<[f64; 3]>()?
            != l.anchor
    {
        return Ok(false);
    }
    let rows = rows(&mesh, st, &s.getattr("_hidden_tags")?)?;
    let changes = st.changes_since(&l.state);
    let mut dirty = Set::default();
    let mut dirty_edges = Set::default();
    let mut affected = Set::default();
    for (state, keys) in [
        (&l.state, &changes.removed),
        (st, &changes.created),
        (&l.state, &changes.modified),
        (st, &changes.modified),
    ] {
        for key in &keys[2] {
            let (_, _, _, f) = crate::store::keys::split_key(*key).unwrap();
            let r = state.frings.get(f as usize);
            let n = (0..r.n_rings)
                .map(|i| *state.ring_sizes.get((r.ring_start + i) as usize))
                .sum::<u32>();
            affected.extend(
                (r.corner_start..r.corner_start + n).map(|i| *state.corners.get(i as usize)),
            );
            if st.slots[2].alive(f) {
                dirty.insert(f);
            }
        }
        for key in &keys[1] {
            let (_, _, _, e) = crate::store::keys::split_key(*key).unwrap();
            affected.extend(*state.ends.get(e as usize));
            if st.slots[1].alive(e) {
                dirty_edges.insert(e);
            }
        }
    }
    for key in &changes.moved {
        let v = store.slot(Kind::Vertex, *key).unwrap();
        affected.insert(v);
        for e in store.edges_at(v) {
            dirty_edges.insert(e);
            for &f in store.edge_faces(e) {
                affected.extend(store.face_rings(f).into_iter().flatten());
            }
        }
    }
    for &e in &dirty_edges {
        let [a, b] = *st.ends.get(e as usize);
        if a != b
            && store
                .edges_at(a)
                .any(|other| other != e && st.ends.get(other as usize).contains(&b))
        {
            return Ok(false);
        }
    }
    let faces = store.live_slots(Kind::Face);
    let edges = store.live_slots(Kind::Edge);
    let row_changed: Vec<_> = rows
        .signatures
        .iter()
        .enumerate()
        .map(|(i, r)| l.signatures.get(i) != Some(r) || l.draw.get(i) != rows.draw.get(i))
        .collect();
    for &f in &faces {
        let a = rows.attrs[f as usize];
        if l.attrs.get(f as usize) != Some(&a) || row_changed[a as usize] {
            dirty.insert(f);
            affected.extend(store.face_rings(f).into_iter().flatten());
        }
    }
    for v in affected {
        if st.slots[0].alive(v) {
            for e in store.edges_at(v) {
                dirty.extend(store.edge_faces(e));
            }
        }
    }
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
    // A soft fan welds vertices at identical display coordinates. Use the
    // general pack when float narrowing merges distinct store vertices.
    for key in changes.removed[0].iter().chain(&changes.moved) {
        let (_, _, _, v) = crate::store::keys::split_key(*key).unwrap();
        l.positions.remove(&position_key(&l.state, v, l.anchor));
    }
    for key in changes.created[0].iter().chain(&changes.moved) {
        let v = store.slot(Kind::Vertex, *key).unwrap();
        if !l.positions.insert(position_key(st, v, l.anchor)) {
            return Ok(false);
        }
    }
    for v in st.slots[0].live_slots() {
        let pos = point(st, v, l.anchor).map(|x| x as f32);
        for k in 0..3 {
            bounds[k] = bounds[k].min(pos[k]);
            bounds[k + 4] = bounds[k + 4].max(pos[k]);
        }
    }
    let h = crate::hypot_of(
        &py.import("core._native")?
            .call_method0("qt_hypot")?
            .extract::<String>()?,
    )?;
    let Some(data) = patch_faces(
        store,
        l.anchor,
        &rows,
        &dirty,
        &p.getattr("texture_paths")?,
        h,
    )?
    else {
        return Ok(false);
    };
    let explicit = l.material_len > 0;
    let (mut edit, mut mat) = (Edit::default(), Edit::default());
    let mut face_spans = vec![(0, 0); st.slots[2].len()];
    let mut edge_spans = vec![(0, 0); st.slots[1].len()];
    let mut counts = [0usize; 4];
    let mut classes = l.classes.clone();
    classes.resize(st.slots[2].len(), 0);
    let hidden = faces.iter().any(|&f| {
        rows.face(f) == 1
            && if dirty.contains(&f) {
                data.contains_key(&f)
            } else {
                l.faces[f as usize].1 > 0
            }
    });
    let mut dirty_face_slots = vec![false; face_spans.len()];
    for &f in &dirty {
        dirty_face_slots[f as usize] = true;
    }
    let mut dirty_edge_slots = vec![false; edge_spans.len()];
    for &e in &dirty_edges {
        dirty_edge_slots[e as usize] = true;
    }
    let mut exclusions = Vec::<f32>::new();
    let mut has_special = false;
    let mut all_matching = true;
    let mut reveal_faces = Vec::new();
    let mut reveal_edges = Vec::new();
    for draw in 0..2 {
        // On the second draw only the reveal-only items collected in the
        // first pass remain. Their order is the document order.
        let deferred_faces;
        let face_list = if draw == 0 {
            &faces
        } else {
            deferred_faces = std::mem::take(&mut reveal_faces);
            &deferred_faces
        };
        for &f in face_list {
            if draw == 0 {
                let group = rows.face(f);
                if group == 1 {
                    reveal_faces.push(f);
                }
                if group != 0 {
                    continue;
                }
            }
            let start = edit.len / 28;
            if dirty_face_slots[f as usize] {
                if let Some(d) = data.get(&f) {
                    edit.write(&d.vertices);
                    if explicit {
                        mat.write(&d.materials);
                    }
                    classes[f as usize] = u8::from(d.special) | (u8::from(d.matching) << 1);
                }
            } else {
                let (src, n) = l.faces[f as usize];
                edit.keep(src * 28, n as usize * 28);
                if explicit {
                    mat.keep(src * 32, n as usize * 32);
                }
            }
            let n = edit.len / 28 - start;
            face_spans[f as usize] = (start as u32, n as u32);
            if n != 0 {
                has_special |= classes[f as usize] & 1 != 0;
                all_matching &= classes[f as usize] & 2 != 0;
            }
            counts[draw as usize * 2] += n;
        }
        let deferred_edges;
        let edge_list = if draw == 0 {
            &edges
        } else {
            deferred_edges = std::mem::take(&mut reveal_edges);
            &deferred_edges
        };
        for &e in edge_list {
            let ends = *st.ends.get(e as usize);
            let position = |v| point(st, v, l.anchor).map(|x| x as f32);
            if draw == 0 && st.eflags.get(e as usize) & 3 != 0 {
                for v in ends {
                    exclusions.extend(position(v));
                }
            }
            if draw == 0 {
                let group = rows.edge(st, e);
                if group == 1 {
                    reveal_edges.push(e);
                }
                if group != 0 {
                    continue;
                }
            }
            if ends[0] == ends[1] {
                continue;
            }
            let start = edit.len / 28;
            let old = l.edges.get(e as usize).copied().unwrap_or_default();
            if !dirty_edge_slots[e as usize] && old.1 == 2 {
                edit.keep(old.0 * 28, 56);
            } else {
                for v in ends {
                    for x in position(v).into_iter().chain([0., 0., 1.]) {
                        edit.write(&x.to_le_bytes());
                    }
                    edit.write(&[0; 4]);
                }
            }
            if explicit && hidden && draw == 0 {
                if old.1 == 2
                    && old.0 >= l.visible_faces
                    && old.0 + 2 <= l.visible_end
                    && (old.0 as usize + 2) * 32 <= l.material_len
                {
                    mat.keep(old.0 * 32, 64);
                } else {
                    mat.write(&[0; 64]);
                }
            }
            edge_spans[e as usize] = (start as u32, 2);
            counts[draw as usize * 2 + 1] += 2;
        }
    }
    let flag = u32::from(has_special && all_matching);
    if (has_special && !all_matching) != explicit || flag != l.material_flag {
        return Ok(false);
    }
    p.call_method1("adopt_reserved", (true,))?;
    let pack = p.cast::<super::packed_scene::PackedScene>()?;
    let Some((base, owned, room)) = super::packed_scene::owned_room(pack, code)? else {
        return Ok(false);
    };
    let total = edit.len / 28;
    let visible = counts[0] + counts[1];
    if total > room
        || !edit.ordered()
        || !mat.ordered()
        || mat.len > l.material.bind(py).len()?
        || edit.copies.iter().any(|&(s, _, n)| s + n > owned * 28)
        || mat.copies.iter().any(|&(s, _, n)| s + n > l.material_len)
    {
        return Ok(false);
    }
    let np = py.import("numpy")?;
    let slice = |a: usize, b: usize| PySlice::new(py, a as isize, b as isize, 1);
    let raw = |v| raw_bytes(&np, v);

    let vertices = p
        .getattr("vertices")?
        .get_item(slice(base, base + owned.max(total)))?;
    let bytes = vertices
        .call_method1("view", (np.getattr("uint8")?,))?
        .call_method1("reshape", (-1,))?;
    let indices = p.getattr("indices")?.get_item(slice(base, base + total))?;
    if total > owned {
        let mut ix: PyReadwriteArray1<u32> = indices.extract()?;
        for (j, v) in ix.as_slice_mut()?[owned..].iter_mut().enumerate() {
            *v = (owned + j) as u32;
        }
    }
    let change = |target: &Obj<'_>, e: &Edit, n: usize| -> PyResult<Option<Edit>> {
        let mut dst: PyReadwriteArray1<u8> = target.extract()?;
        let dst = dst.as_slice_mut()?;
        let undo = renderer.is_some().then(|| e.reverse(&dst[..n]));
        e.in_place(dst);
        Ok(undo)
    };
    let vb = change(&bytes, &edit, owned * 28)?;
    let mb = change(l.material.bind(py), &mat, l.material_len)?;
    let live = l.material.bind(py).get_item(slice(0, mat.len))?;
    let bounds = PyArray1::from_slice(py, &bounds).call_method1("reshape", (2, 4))?;
    let pairs = PyList::empty(py);
    for old in p.getattr("profile_exclusions")?.try_iter()? {
        let old = old?;
        if old.get_item(0)?.extract::<usize>()? != code {
            pairs.append(old)?;
        }
    }
    let local = PyList::empty(py);
    if !exclusions.is_empty() {
        let endpoints = py.import("array")?.getattr("array")?.call1(("f",))?;
        let floats = PyArray1::from_vec(py, exclusions);
        endpoints.call_method1("frombytes", (floats.call_method0("tobytes")?,))?;
        pairs.append((code, &endpoints))?;
        local.append((code, endpoints))?;
    }
    let upload = (|| -> PyResult<()> {
        if let Some(r) = renderer {
            r.call_method1(
                "replace_prototype",
                (
                    code,
                    raw(vertices.get_item(slice(0, visible))?)?,
                    raw(indices.get_item(slice(0, visible))?)?,
                    raw(bounds.clone())?,
                    counts[0],
                ),
            )?;
            // A matching back uses the existing vertex colour through the
            // material header. Only legacy renderers need expanded rows.
            let native_patch = r.hasattr("patch_prototype_materials")?;
            let materials = if flag != 0 && !native_patch {
                let b = vertices
                    .get_item(slice(0, counts[0]))?
                    .get_item("color")?
                    .call_method0("tobytes")?;
                let mut rec = vec![0u8; counts[0] * 32];
                for (dst, col) in rec
                    .chunks_exact_mut(32)
                    .zip(b.cast::<PyBytes>()?.as_bytes().chunks_exact(4))
                {
                    dst[16..20].copy_from_slice(col);
                }
                PyArray1::from_vec(py, rec).into_any()
            } else {
                live.clone()
            };
            if native_patch {
                let runs: Vec<_> = mat
                    .copies
                    .iter()
                    .map(|&(s, d, n)| (s / 32, d / 32, n / 32))
                    .collect();
                r.call_method1(
                    "patch_prototype_materials",
                    (code, materials, runs, flag != 0),
                )?;
            } else {
                r.call_method1("update_prototype_materials", (code, materials))?;
            }
            let partly_transparent = |a: u8| a > 0 && a < 255;
            if data.values().any(|face| {
                face.vertices
                    .chunks_exact(28)
                    .any(|v| partly_transparent(v[27]))
                    || face
                        .materials
                        .chunks_exact(32)
                        .any(|m| partly_transparent(m[19]))
            }) && r.hasattr("set_material_transparency")?
            {
                r.call_method1("set_material_transparency", (true,))?;
            }
            if r.hasattr("set_profile_exclusions")? {
                if r.getattr_opt("incremental_profile_exclusions")?
                    .is_some_and(|a| a.is_truthy().unwrap_or(false))
                {
                    if !local.is_empty() {
                        r.call_method1("set_profile_exclusions", (&local,))?;
                    }
                } else {
                    r.call_method1("set_profile_exclusions", (&pairs,))?;
                }
            }
        }
        Ok(())
    })();
    if let Err(err) = upload {
        for (u, dst) in [(vb, &bytes), (mb, l.material.bind(py))] {
            if let Some(u) = u {
                let mut b: PyReadwriteArray1<u8> = dst.extract()?;
                u.in_place(b.as_slice_mut()?);
            }
        }
        return Err(err);
    }
    let mut first = base;
    for (name, n) in TABLES.iter().zip(counts) {
        let cmd = p.getattr(*name)?.get_item(code)?;
        cmd.set_item("first", first)?;
        cmd.set_item("count", n)?;
        first += n;
    }
    p.setattr("profile_exclusions", pairs.to_tuple())?;
    p.getattr("bounds")?.set_item(code, bounds)?;
    p.getattr("material_updates")?
        .set_item(code, (flag, live))?;
    s.getattr("meshes")?.set_item(
        mesh.as_ptr() as usize,
        (&mesh, mesh.getattr("_mut_serial")?, code),
    )?;
    let next = super::plain_edit::token();
    let mut snap = super::plain_edit::SnapPatch {
        before: Some(l.token),
        after: next,
        code,
        lineage: store.lineage,
        revision: store.revision,
        anchor: l.anchor,
        faces: counts[0] / 3,
        edges: edges.len(),
        triangles: Vec::new(),
        segments: Vec::new(),
    };
    for &(src, dst, n) in &edit.copies {
        let bytes = n.min((counts[0] * 28).saturating_sub(dst));
        if bytes > 0 {
            snap.triangles.push((src / 84, dst / 84, bytes / 84));
        }
    }
    let old_order = l.state.order[1].dense_of_slot(&l.state.slots[1]);
    for (i, &e) in edges.iter().enumerate() {
        if dirty_edge_slots[e as usize] {
            continue;
        }
        let from = old_order[e as usize] as usize;
        if let Some((s, d, n)) = snap.segments.last_mut() {
            if *s + *n == from && *d + *n == i {
                *n += 1;
                continue;
            }
        }
        snap.segments.push((from, i, 1));
    }
    p.setattr("_snap_edit", Bound::new(py, snap)?)?;
    l.token = next;
    l.state = st.clone();
    l.revision = store.revision;
    l.faces = face_spans;
    l.edges = edge_spans;
    l.attrs = rows.attrs;
    l.signatures = rows.signatures;
    l.draw = rows.draw;
    l.material_len = mat.len;
    l.classes = classes;
    l.visible_faces = counts[0] as u32;
    l.visible_end = visible as u32;
    Ok(true)
}

fn raw_bytes<'p>(np: &Obj<'p>, v: Obj<'p>) -> PyResult<Obj<'p>> {
    v.call_method1("view", (np.getattr("uint8")?,))?
        .call_method1("reshape", (-1,))
}
