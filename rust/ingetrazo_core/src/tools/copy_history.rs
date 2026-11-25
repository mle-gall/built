// SPDX-License-Identifier: GPL-3.0-or-later
//! Selection belongs to the edit context, outside the geometry journal. A
//! copied stroke may have been divided by the edge planner during this step.
use crate::{
    math::{xyz, Vec3},
    store::{keys::Kind, py::PyMeshStore, Store},
};
use glam::DVec3;
use pyo3::{prelude::*, types::PySet};
use std::collections::HashSet;

fn on_span(p: DVec3, a: DVec3, ab: DVec3, length2: f64) -> bool {
    let t = (p - a).dot(ab) / length2;
    (0.0..=1.0).contains(&t) && p.distance(a + t * ab) <= 1e-4
}
fn store_edges(store: &Store, segments: &[(DVec3, DVec3)]) -> HashSet<u128> {
    let mut edges = HashSet::new();
    for &(a, b) in segments {
        let (Some(first), Some(last)) = (store.lookup(a.into()), store.lookup(b.into())) else {
            continue;
        };
        let ab = b - a;
        let length2 = ab.length_squared();
        if first == last || length2 < 1e-18 {
            continue;
        }
        let mut todo = vec![first];
        let mut seen = HashSet::from([first]);
        while let Some(v) = todo.pop() {
            for e in store.edges_at(v) {
                let [left, right] = *store.state.ends.get(e as usize);
                let other = if left == v { right } else { left };
                if other != first
                    && other != last
                    && !on_span(
                        DVec3::from(*store.state.vpos.get(other as usize)),
                        a,
                        ab,
                        length2,
                    )
                {
                    continue;
                }
                edges.insert(store.key(Kind::Edge, e));
                if seen.insert(other) {
                    todo.push(other);
                }
            }
        }
    }
    edges
}
/// Legacy extension meshes expose incidence through handles instead of slots.
fn object_edges(
    mesh: &Bound<'_, PyAny>,
    segments: &[(DVec3, DVec3)],
    picked: &Bound<'_, PySet>,
) -> PyResult<()> {
    for &(a, b) in segments {
        let ab = b - a;
        let length2 = ab.length_squared();
        if length2 < 1e-18 {
            continue;
        }
        let first = mesh.call_method1("vertex_at", (Vec3::from(a),))?;
        let last = mesh.call_method1("vertex_at", (Vec3::from(b),))?;
        if first.is_none() || last.is_none() || first.eq(&last)? {
            continue;
        }
        let mut todo = vec![first.clone()];
        let seen = PySet::empty(mesh.py())?;
        seen.add(&first)?;
        while let Some(v) = todo.pop() {
            for edge in v.getattr("edges")?.try_iter()? {
                let edge = edge?;
                let other = edge.call_method1("other", (&v,))?;
                if !other.eq(&first)?
                    && !other.eq(&last)?
                    && !on_span(xyz(&other.getattr("position")?)?, a, ab, length2)
                {
                    continue;
                }
                picked.add(edge)?;
                if !seen.contains(&other)? {
                    seen.add(&other)?;
                    todo.push(other);
                }
            }
        }
    }
    Ok(())
}

#[pyclass(module = "ingetrazo_core")]
pub struct CopySelection {
    records: Vec<Py<PyAny>>,
    faces: Vec<Py<PyAny>>,
    segments: Vec<(DVec3, DVec3)>,
    before: Option<Py<PySet>>,
    context: Option<Py<PyAny>>,
}
fn selection<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PySet>> {
    scene
        .py()
        .get_type::<PySet>()
        .call1((scene.getattr("selection")?,))?
        .cast_into()
        .map_err(Into::into)
}
fn select(scene: &Bound<'_, PyAny>, set: &Bound<'_, PySet>) -> PyResult<()> {
    scene.setattr("selection", set)?;
    scene.call_method0("bump_view")?;
    Ok(())
}
#[pymethods]
impl CopySelection {
    #[new]
    fn new(
        mut groups: Vec<Py<PyAny>>,
        guides: Vec<Py<PyAny>>,
        faces: Vec<Py<PyAny>>,
        segments: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        groups.extend(guides);
        let segments = segments
            .try_iter()?
            .map(|s| {
                let s = s?;
                Ok((xyz(&s.get_item(0)?)?, xyz(&s.get_item(1)?)?))
            })
            .collect::<PyResult<_>>()?;
        Ok(Self {
            records: groups,
            faces,
            segments,
            before: None,
            context: None,
        })
    }
    fn execute(&mut self, scene: &Bound<'_, PyAny>, apply: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = scene.py();
        let before = selection(scene)?;
        scene.setattr("selection", before.call_method0("copy")?)?;
        if let Err(err) = apply.call1((scene,)) {
            select(scene, &before)?;
            return Err(err);
        }
        let context = scene.getattr("edit_group")?;
        if self.before.is_none() {
            self.before = Some(before.clone().unbind());
            self.context = Some(context.clone().unbind());
        }
        if !self.context.as_ref().unwrap().bind(py).is(&context) {
            return select(scene, &before);
        }
        let picked = PySet::new(py, &self.records)?;
        let mesh = scene.getattr("mesh")?;
        if let Some(handle) = mesh.getattr_opt("_store")?.filter(|s| !s.is_none()) {
            let edge_keys = {
                let store = PyMeshStore::read(&handle)?;
                for cmd in &self.faces {
                    let f = cmd.bind(py).getattr("face")?;
                    if !f.is_none()
                        && store
                            .inner
                            .slot(Kind::Face, f.getattr("key")?.extract()?)
                            .is_some()
                    {
                        picked.add(f)?;
                    }
                }
                store_edges(&store.inner, &self.segments)
            };
            let class = py.import("ingetrazo_core.store_mesh")?.getattr("EdgeView")?;
            for key in edge_keys {
                picked.add(class.call1((&mesh, key))?)?;
            }
        } else {
            let live = mesh.getattr("faces")?;
            for cmd in &self.faces {
                let f = cmd.bind(py).getattr("face")?;
                if live.contains(&f)? {
                    picked.add(f)?;
                }
            }
            object_edges(&mesh, &self.segments, &picked)?;
        }
        select(scene, &picked)
    }
    fn undo(&self, scene: &Bound<'_, PyAny>, revert: &Bound<'_, PyAny>) -> PyResult<()> {
        let context = scene.getattr("edit_group")?;
        let picked = if self
            .context
            .as_ref()
            .is_some_and(|c| c.bind(scene.py()).is(&context))
        {
            self.before
                .as_ref()
                .unwrap()
                .bind(scene.py())
                .call_method0("copy")?
                .cast_into::<PySet>()?
        } else {
            selection(scene)?
        };
        revert.call1((scene,))?;
        select(scene, &picked)
    }
}

/// Frozen source membership for Move copies, including a later replacement
/// array. Plan each member against the original scene and the other members;
/// rediscovering intersections inside one imported member changes its topology.
#[pyclass(module = "ingetrazo_core")]
pub struct MoveCopies {
    scene: Py<PyAny>,
    groups: Vec<Py<PyAny>>,
    faces: Vec<Py<PyAny>>,
    edges: Vec<Py<PyAny>>,
    guides: Vec<Py<PyAny>>,
}
use pyo3::types::{PyDict, PyList};
struct Member<'py> {
    edges: Vec<Bound<'py, PyAny>>,
    faces: Vec<Bound<'py, PyAny>>,
}
fn translated(ring: &Bound<'_, PyAny>, delta: DVec3) -> PyResult<Vec<Vec3>> {
    ring.try_iter()?
        .map(|p| Ok(Vec3::from(xyz(&p?)? + delta)))
        .collect()
}
impl MoveCopies {
    fn member<'py>(&self, py: Python<'py>, d: DVec3) -> PyResult<Member<'py>> {
        let geometry = py.import("core.geometry")?;
        let edge = geometry.getattr("Edge")?;
        let face = geometry.getattr("Face")?;
        let edges = self
            .edges
            .iter()
            .map(|e| {
                let (a, b) = (
                    xyz(&e.bind(py).getattr("a")?)?,
                    xyz(&e.bind(py).getattr("b")?)?,
                );
                edge.call1((Vec3::from(a + d), Vec3::from(b + d)))
            })
            .collect::<PyResult<_>>()?;
        let faces = self
            .faces
            .iter()
            .map(|f| {
                let f = f.bind(py);
                let outer = translated(&f.getattr("vertices")?, d)?;
                let holes = f
                    .getattr("holes")?
                    .try_iter()?
                    .map(|h| translated(&h?, d))
                    .collect::<PyResult<Vec<_>>>()?;
                face.call1((outer, holes))
            })
            .collect::<PyResult<_>>()?;
        Ok(Member { edges, faces })
    }
    fn contacts<'py>(
        &self,
        py: Python<'py>,
        i: usize,
        members: &[Member<'py>],
    ) -> PyResult<Bound<'py, PyAny>> {
        let hard = PyList::empty(py);
        let scene = self.scene.bind(py);
        if scene.is_none() {
            return Ok(hard.into_any());
        }
        for (e, new) in self.edges.iter().zip(&members[i].edges) {
            let e = e.bind(py);
            if e.getattr("curve")?.is_none() && !e.getattr("soft")?.is_truthy()? {
                hard.append((new.getattr("a")?, new.getattr("b")?))?;
            }
        }
        if hard.is_empty() {
            return Ok(hard.into_any());
        }
        let list = py.get_type::<PyList>();
        let edges = list.call1((scene.getattr("edges")?,))?;
        let faces = list.call1((scene.getattr("faces")?,))?;
        for (j, other) in members.iter().enumerate() {
            if i == j {
                continue;
            }
            edges.call_method1("extend", (&other.edges,))?;
            faces.call_method1("extend", (&other.faces,))?;
        }
        py.import("core.edits")?
            .call_method1("touching_segments", (hard, edges, faces))
    }
}
#[pymethods]
impl MoveCopies {
    #[new]
    fn new(
        scene: Py<PyAny>,
        groups: &Bound<'_, PyAny>,
        faces: &Bound<'_, PyAny>,
        edges: &Bound<'_, PyAny>,
        guides: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        let collect = |items: &Bound<'_, PyAny>| {
            items
                .try_iter()?
                .map(|p| p.map(Bound::unbind))
                .collect::<PyResult<Vec<_>>>()
        };
        Ok(Self {
            scene,
            groups: collect(groups)?,
            faces: collect(faces)?,
            edges: collect(edges)?,
            guides: collect(guides)?,
        })
    }
    fn __call__<'py>(
        &self,
        py: Python<'py>,
        deltas: &Bound<'py, PyAny>,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        let deltas = deltas
            .try_iter()?
            .map(|d| xyz(&d?))
            .collect::<PyResult<Vec<_>>>()?;
        let members = deltas
            .iter()
            .map(|&d| self.member(py, d))
            .collect::<PyResult<Vec<_>>>()?;
        let history = py.import("ingetrazo_core.history")?;
        let ops = py.import("core.move_ops")?;
        let group = py.import("core.group")?;
        let new_attrs = py
            .import("core.ifc_identity")?
            .getattr("copied_face_attrs")?;
        let next_curve = py
            .import("core.mesh")?
            .getattr("Mesh")?
            .getattr("next_curve_id")?;
        let key = py.import("core.topology")?.getattr("_key")?;
        let segments = PyList::empty(py);
        let (mut commands, mut groups, mut guides, mut faces) = (vec![], vec![], vec![], vec![]);
        for (i, (&d, member)) in deltas.iter().zip(&members).enumerate() {
            let planned = self.contacts(py, i, &members)?;
            let keys = PySet::empty(py)?;
            for pair in planned.try_iter()? {
                let pair = pair?;
                keys.add((
                    key.call1((pair.get_item(0)?,))?,
                    key.call1((pair.get_item(1)?,))?,
                ))?;
            }
            let matrix = Py::new(
                py,
                crate::math::Matrix4::from(glam::DMat4::from_translation(d)),
            )?;
            let attrs = new_attrs.call0()?;
            for g in &self.groups {
                let clone = group.call_method1("copy_group", (g, Vec3::from(d)))?;
                commands.push(history.call_method1("InsertGroupCommand", (&clone,))?);
                groups.push(clone);
            }
            for g in &self.guides {
                let clone = ops.call_method1("copied_guide", (g, &matrix))?;
                commands.push(history.call_method1("AddGuideCommand", (&clone,))?);
                guides.push(clone);
            }
            for (src, placed) in self.faces.iter().zip(&member.faces) {
                let src = src.bind(py);
                let cmd = copied_face(
                    &history,
                    &group,
                    &attrs,
                    matrix.bind(py).as_any(),
                    src,
                    &placed.getattr("vertices")?,
                    &placed.getattr("holes")?,
                    true,
                )?;
                faces.push(cmd.clone());
                commands.push(cmd);
            }
            let mut curves = std::collections::HashMap::<i64, i64>::new();
            for (src, placed) in self.edges.iter().zip(&member.edges) {
                let (a, b) = (placed.getattr("a")?, placed.getattr("b")?);
                segments.append((&a, &b))?;
                let src = src.bind(py);
                let soft = src.getattr("soft")?.is_truthy()?;
                let curve = src.getattr("curve")?.extract::<Option<i64>>()?;
                if !keys.is_empty()
                    && !soft
                    && curve.is_none()
                    && keys.contains((key.call1((&a,))?, key.call1((&b,))?))?
                {
                    continue;
                }
                let kw = PyDict::new(py);
                if soft {
                    kw.set_item("soft", true)?;
                }
                if let Some(id) = curve {
                    let id = match curves.entry(id) {
                        std::collections::hash_map::Entry::Occupied(e) => *e.get(),
                        std::collections::hash_map::Entry::Vacant(e) => {
                            *e.insert(next_curve.call0()?.extract()?)
                        }
                    };
                    kw.set_item("curve", id)?;
                }
                commands.push(history.getattr("AddEdgeCommand")?.call((a, b), Some(&kw))?);
            }
            if !keys.is_empty() {
                commands.push(ops.call_method1("_PlannedWhenRun", (planned,))?);
            }
        }
        if commands.is_empty() {
            return Ok(None);
        }
        Ok(Some(ops.call_method1(
            "_MoveCopiesCommand",
            (commands, groups, guides, faces, segments),
        )?))
    }
}

/// Keep material coordinates and IFC identity remapping common to linear and polar copies.
pub(super) fn copied_face<'py>(
    history: &Bound<'py, PyModule>,
    group: &Bound<'py, PyModule>,
    attrs: &Bound<'py, PyAny>,
    matrix: &Bound<'py, PyAny>,
    source: &Bound<'py, PyAny>,
    vertices: &Bound<'py, PyAny>,
    holes: &Bound<'py, PyAny>,
    carry_interior: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let kw = PyDict::new(history.py());
    if holes.is_truthy()? {
        kw.set_item("holes", holes)?;
    }
    kw.set_item("auto", false)?;
    let transformed =
        group.call_method1("transformed_attrs", (source.getattr("attrs")?, matrix))?;
    kw.set_item("attrs", attrs.call1((transformed,))?)?;
    if carry_interior {
        kw.set_item("interior", source.getattr("interior")?)?;
    }
    history
        .getattr("AddFaceCommand")?
        .call((vertices,), Some(&kw))
}
