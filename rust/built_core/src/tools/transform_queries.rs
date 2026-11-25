// SPDX-License-Identifier: GPL-3.0-or-later
//! Transform target resolution and the immutable wire preview of a copy.
use super::view_queries::{classify, hit};
use crate::{
    math::{xyz, Vec3},
    store::{commands::cell, keys::Kind, py::PyMeshStore},
};
use glam::{DMat4, DVec3};
use pyo3::{prelude::*, types::PyDict};
use std::collections::HashSet;

fn pixel(ctx: &Bound<'_, PyAny>) -> PyResult<(f64, f64)> {
    let xy = ctx.getattr("screen")?;
    Ok((
        xy.call_method0("x")?.extract()?,
        xy.call_method0("y")?.extract()?,
    ))
}

fn entities<'py>(ctx: &Bound<'py, PyAny>, picks: &[&str]) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let view = ctx.getattr("viewport")?;
    let selection = view.getattr("scene")?.getattr("selection")?;
    if selection.is_truthy()? || picks.is_empty() {
        return selection.try_iter()?.collect();
    }
    let px = pixel(ctx)?;
    for name in picks {
        if let Some(entity) = hit(&view, name, px)? {
            return Ok(vec![entity]);
        }
    }
    Ok(Vec::new())
}

#[derive(Default)]
struct GrabGeometry {
    groups: Vec<Py<PyAny>>,
    positions: Vec<Vec3>,
    vertices: Vec<Py<PyAny>>,
    images: Vec<Py<PyAny>>,
}

fn grab_geometry<'py>(
    mesh: &Bound<'py, PyAny>,
    entities: Vec<Bound<'py, PyAny>>,
) -> PyResult<GrabGeometry> {
    let mut result = GrabGeometry::default();
    let mut keys = HashSet::new();
    let mut cells = HashSet::new();
    let data = mesh.getattr("__dict__")?;
    let store = data
        .cast::<PyDict>()?
        .get_item("_store")?
        .filter(|s| !s.is_none());
    let mut vertex_type = None;
    for entity in entities {
        let kind = match classify(&entity)? {
            "group" => {
                result.groups.push(entity.unbind());
                continue;
            }
            "image_plane" => {
                result.images.push(entity.unbind());
                continue;
            }
            "edge" => Kind::Edge,
            "face" => Kind::Face,
            _ => continue,
        };
        // A nested pick may reference another prototype. Only read slots
        // belonging to this editing mesh. Stand-ins keep the position lookup.
        if let Some(st) = store.as_ref().filter(|_| {
            entity
                .getattr_opt("_m")
                .ok()
                .flatten()
                .is_some_and(|m| m.is(mesh))
        }) {
            let st = PyMeshStore::read(st)?;
            if let Some(slot) = st.inner.slot(kind, entity.getattr("key")?.extract()?) {
                let slots = if kind == Kind::Edge {
                    st.inner.state.ends.get(slot as usize).to_vec()
                } else {
                    st.inner.face_rings(slot).into_iter().flatten().collect()
                };
                let class = match &vertex_type {
                    Some(class) => class,
                    None => vertex_type
                        .insert(mesh.py().import("built_core.store_mesh")?.getattr("VertexView")?),
                };
                for v in slots {
                    let key = st.inner.key(Kind::Vertex, v);
                    if keys.insert(key) {
                        let p = *st.inner.state.vpos.get(v as usize);
                        cells.insert(cell(p));
                        result.positions.push(Vec3::from(DVec3::from(p)));
                        result.vertices.push(class.call1((mesh, key))?.unbind());
                    }
                }
                continue;
            }
        }
        let mut add = |point: Bound<'_, PyAny>| -> PyResult<()> {
            let p = xyz(&point)?;
            if cells.insert(cell(p.to_array())) {
                result.positions.push(Vec3::from(p));
                let v = mesh.call_method1("vertex_at", (point,))?;
                if !v.is_none() {
                    result.vertices.push(v.unbind());
                }
            }
            Ok(())
        };
        if kind == Kind::Edge {
            add(entity.getattr("a")?)?;
            add(entity.getattr("b")?)?;
        } else {
            for ring in std::iter::once(Ok(entity.getattr("vertices")?))
                .chain(entity.getattr("holes")?.try_iter()?)
            {
                for p in ring?.try_iter()? {
                    add(p?)?;
                }
            }
        }
    }
    Ok(result)
}

#[pyfunction]
pub fn transform_targets(ctx: &Bound<'_, PyAny>) -> PyResult<(Vec<Py<PyAny>>, Vec<Vec3>)> {
    let mesh = ctx.getattr("viewport")?.getattr("scene")?.getattr("mesh")?;
    let grab = grab_geometry(
        &mesh,
        entities(ctx, &["pick_group", "pick_edge", "pick_face"])?,
    )?;
    Ok((grab.groups, grab.positions))
}

#[pyfunction]
pub fn scale_grab(scene: &Bound<'_, PyAny>, tool: &Bound<'_, PyAny>) -> PyResult<()> {
    let grab = grab_geometry(
        &scene.getattr("mesh")?,
        scene
            .getattr("selection")?
            .try_iter()?
            .collect::<PyResult<_>>()?,
    )?;
    tool.setattr("_groups", grab.groups)?;
    tool.setattr("_images", grab.images)?;
    tool.setattr("_positions", grab.positions)?;
    tool.setattr("_verts", grab.vertices)
}

#[pyfunction]
pub fn transform_records<'py>(
    ctx: &Bound<'py, PyAny>,
    kind: &str,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let view = ctx.getattr("viewport")?;
    let scene = view.getattr("scene")?;
    let selection = scene.getattr("selection")?;
    if selection.is_truthy()? {
        let mut records = Vec::new();
        for entity in selection.try_iter()? {
            let entity = entity?;
            if classify(&entity)? == kind {
                records.push(entity);
            }
        }
        return Ok(records);
    }
    let name = format!("pick_{kind}");
    let Some(pick) = view.getattr_opt(name)? else {
        return Ok(Vec::new());
    };
    let px = pixel(ctx)?;
    let kwargs = PyDict::new(ctx.py());
    if kind == "text_label" {
        kwargs.set_item("rect_only", true)?;
    }
    let entity = pick.call(px, Some(&kwargs))?;
    if entity.is_none()
        || (kind == "guide"
            && (!scene
                .call_method1("entity_selectable", (&entity,))?
                .is_truthy()?
                || hit(&view, "pick_edge", px)?.is_some()))
    {
        return Ok(Vec::new());
    }
    Ok(vec![entity])
}

type Segment = [DVec3; 2];

#[pyclass(module = "built_core")]
#[derive(Default)]
pub struct CopyWire {
    loose: Vec<Segment>,
    placements: Vec<Segment>,
    pending_groups: Vec<Py<PyAny>>,
}

#[pymethods]
impl CopyWire {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    #[staticmethod]
    fn gather(
        ctx: &Bound<'_, PyAny>,
        guides: &Bound<'_, PyAny>,
        pick_loose: bool,
    ) -> PyResult<(Vec<Py<PyAny>>, Vec<Py<PyAny>>, Self)> {
        let mut wire = Self::default();
        let (mut faces, mut edges) = (Vec::new(), Vec::new());
        let camera = ctx.getattr("viewport")?.getattr_opt("camera")?;
        let reach = camera
            .map(|c| c.getattr_opt("distance"))
            .transpose()?
            .flatten()
            .map(|v| v.extract::<f64>())
            .transpose()?
            .unwrap_or(10.)
            .max(1e-3);
        for guide in guides.try_iter()? {
            let guide = guide?;
            let p = xyz(&guide.getattr("point")?)?;
            let axis = guide.getattr("direction")?;
            if !axis.is_none() {
                let d = xyz(&axis)? * reach;
                wire.loose.push([p - d, p + d]);
            } else {
                for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                    let d = axis * reach * 0.02;
                    wire.loose.push([p - d, p + d]);
                }
                let origin = guide.getattr("origin")?;
                if !origin.is_none() {
                    wire.loose.push([xyz(&origin)?, p]);
                }
            }
        }
        let picks: &[&str] = if pick_loose {
            &["pick_edge", "pick_face"]
        } else {
            &[]
        };
        for entity in entities(ctx, picks)? {
            match classify(&entity)? {
                "face" => {
                    let holes = entity.getattr("holes")?;
                    for ring in
                        std::iter::once(Ok(entity.getattr("vertices")?)).chain(holes.try_iter()?)
                    {
                        let points = ring?
                            .try_iter()?
                            .map(|p| xyz(&p?))
                            .collect::<PyResult<Vec<_>>>()?;
                        wire.loose.extend(
                            points
                                .iter()
                                .zip(points.iter().cycle().skip(1))
                                .map(|(&a, &b)| [a, b]),
                        );
                    }
                    faces.push(entity.unbind());
                }
                "edge" => {
                    wire.loose
                        .push([xyz(&entity.getattr("a")?)?, xyz(&entity.getattr("b")?)?]);
                    edges.push(entity.unbind());
                }
                _ => {}
            }
        }
        Ok((faces, edges, wire))
    }

    #[staticmethod]
    pub(super) fn capture(tool: &Bound<'_, PyAny>, ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        let groups = tool.getattr("_groups")?;
        let guides = tool.getattr("_moved_guides")?;
        let (faces, edges, mut wire) =
            Self::gather(ctx, &guides, !groups.is_truthy()? && !guides.is_truthy()?)?;
        wire.pending_groups = groups.extract()?;
        wire.pending_groups.reverse();
        tool.setattr("_sel_faces", faces)?;
        tool.setattr("_sel_edges", edges)?;
        tool.setattr("_base_segments", wire)
    }

    fn fill_groups(&mut self) -> PyResult<()> {
        while let Some(group) = self.pending_groups.pop() {
            Python::attach(|py| self.add_group(group.bind(py)))?;
        }
        Ok(())
    }

    fn move_lines(
        &mut self,
        grab: &Bound<'_, PyAny>,
        hover: &Bound<'_, PyAny>,
        copy: bool,
    ) -> PyResult<Vec<(Vec3, Vec3)>> {
        if grab.is_none() || hover.is_none() {
            return Ok(vec![]);
        }
        let (g, h) = (xyz(grab)?, xyz(hover)?);
        let mut lines = vec![(g.into(), h.into())];
        if copy {
            self.fill_groups()?;
            lines.extend(self.lines(None, Some((h - g).into())));
        }
        Ok(lines)
    }

    fn add_group(&mut self, group: &Bound<'_, PyAny>) -> PyResult<()> {
        let placements = group
            .py()
            .import("core.group")?
            .call_method1("iter_placements", (group,))?;
        for p in placements.try_iter()? {
            let p = p?;
            let matrix = p.get_item(1)?;
            self.append_mesh(
                &p.get_item(0)?.getattr("mesh")?,
                if matrix.is_none() {
                    None
                } else {
                    Some(matrix.call_method0("data")?.extract()?)
                },
                usize::MAX,
            )?;
        }
        Ok(())
    }
    #[pyo3(signature=(matrix=None, delta=None))]
    fn lines(&self, matrix: Option<[f64; 16]>, delta: Option<[f64; 3]>) -> Vec<(Vec3, Vec3)> {
        let m = matrix
            .as_ref()
            .map(DMat4::from_cols_array)
            .unwrap_or_else(|| DMat4::from_translation(delta.unwrap_or_default().into()));
        self.placements
            .iter()
            .chain(&self.loose)
            .map(|&[a, b]| (m.transform_point3(a).into(), m.transform_point3(b).into()))
            .collect()
    }
    fn __len__(&self) -> usize {
        self.placements.len() + self.loose.len()
    }
}

impl CopyWire {
    fn append_mesh(
        &mut self,
        mesh: &Bound<'_, PyAny>,
        matrix: Option<[f64; 16]>,
        limit: usize,
    ) -> PyResult<()> {
        let m = matrix
            .as_ref()
            .map(DMat4::from_cols_array)
            .unwrap_or(DMat4::IDENTITY);
        let d = mesh.getattr("__dict__")?;
        if let Some(store) = d
            .cast::<PyDict>()?
            .get_item("_store")?
            .filter(|s| !s.is_none())
        {
            let store = PyMeshStore::read(&store)?;
            for slot in store.inner.live_slots(Kind::Edge).into_iter().take(limit) {
                let segment =
                    store.inner.state.ends.get(slot as usize).map(|v| {
                        m.transform_point3((*store.inner.state.vpos.get(v as usize)).into())
                    });
                self.placements.push(segment);
            }
        } else {
            for edge in mesh.getattr("edges")?.try_iter()?.take(limit) {
                let edge = edge?;
                self.placements.push([
                    m.transform_point3(xyz(&edge.getattr("a")?)?),
                    m.transform_point3(xyz(&edge.getattr("b")?)?),
                ]);
            }
        }
        Ok(())
    }
}

/// Pending component template. Deferred meshes and large assemblies display
/// their bounds; a single materialized mesh keeps its first 2,000 edges.
#[pyfunction]
#[pyo3(signature=(group, aligned, anchor=None))]
pub fn placement_template(
    group: &Bound<'_, PyAny>,
    aligned: bool,
    anchor: Option<&Bound<'_, PyAny>>,
) -> PyResult<(Vec3, Vec<[[f64; 3]; 2]>)> {
    const MAX_EDGES: usize = 2000;
    let bounds = super::host_geometry::ToolBounds::placement_bounds(group)?;
    let a = anchor
        .map(xyz)
        .transpose()?
        .unwrap_or_else(|| bounds.anchor(aligned));
    let py = group.py();
    let lazy = py.import("core.mesh")?.getattr("DeferredMesh")?;
    let is_tree = group
        .getattr_opt("children")?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(false);
    let mut wire = CopyWire::default();
    if is_tree {
        let mut total = 0;
        let mut draw = true;
        for place in py
            .import("core.group")?
            .call_method1("iter_placements", (group,))?
            .try_iter()?
        {
            let mesh = place?.get_item(0)?.getattr("mesh")?;
            if mesh.get_type().is(&lazy) {
                draw = false;
                break;
            }
            total += mesh.getattr("edges")?.len()?;
            if total > MAX_EDGES {
                draw = false;
                break;
            }
        }
        if draw {
            wire.add_group(group)?;
        }
    } else {
        let mesh = group.getattr("mesh")?;
        if !mesh.get_type().is(&lazy) {
            wire.append_mesh(&mesh, None, MAX_EDGES)?;
        }
    }
    let lines = if wire.placements.is_empty() {
        bounds.box_outline(a)
    } else {
        wire.placements
            .into_iter()
            .map(|segment| segment.map(|p| (p - a).to_array()))
            .collect()
    };
    Ok((a.into(), lines))
}

/// Resolve all parts of one grab once. Annotation priority is significant
/// when clicking through text, a section frame, a guide or a dimension.
#[pyfunction]
pub fn transform_grab(
    tool: &Bound<'_, PyAny>,
    ctx: &Bound<'_, PyAny>,
    mode: &str,
) -> PyResult<bool> {
    let py = ctx.py();
    let view = ctx.getattr("viewport")?;
    let scene = view.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let selection = scene.getattr("selection")?;
    let direct = !selection.is_truthy()?;
    let endpoint = if mode == "move" {
        move_endpoint(ctx)?
    } else {
        None
    };
    let (mut groups, mut positions, mut verts) = if let Some(v) = endpoint {
        (
            vec![],
            vec![Vec3::from(xyz(&v.getattr("position")?)?)],
            vec![v.unbind()],
        )
    } else {
        let geometry = grab_geometry(
            &mesh,
            entities(ctx, &["pick_group", "pick_edge", "pick_face"])?,
        )?;
        (geometry.groups, geometry.positions, geometry.vertices)
    };
    let sections = transform_records(ctx, "section_plane")?;
    let mut guides = if mode == "move" || !direct {
        transform_records(ctx, "guide")?
    } else {
        Vec::new()
    };
    if direct && !sections.is_empty() {
        groups.clear();
        positions.clear();
        guides.clear();
    }
    let labels = if mode == "move" {
        transform_records(ctx, "text_label")?
    } else {
        vec![]
    };
    if direct && !labels.is_empty() {
        groups.clear();
        positions.clear();
        guides.clear();
    }
    let on_guide = direct && !guides.is_empty();
    if on_guide {
        groups.clear();
        positions.clear();
    }
    let images = if on_guide {
        vec![]
    } else {
        transform_records(ctx, "image_plane")?
    };
    if positions.is_empty() {
        verts.clear();
    }
    let dims = if mode == "move" {
        let dims = transform_records(ctx, "dimension")?;
        let dims = py
            .import("built_core.move_ops")?
            .call_method1("dimension_modes", (dims, &groups, &verts))?;
        if direct && dims.is_truthy()? {
            groups.clear();
            positions.clear();
            verts.clear();
            guides.clear();
        }
        dims
    } else {
        py.None().into_bound(py)
    };
    if groups.is_empty()
        && positions.is_empty()
        && labels.is_empty()
        && guides.is_empty()
        && sections.is_empty()
        && images.is_empty()
        && !dims.is_truthy()?
    {
        return Ok(false);
    }
    tool.setattr("_groups", groups)?;
    tool.setattr("_positions", positions)?;
    tool.setattr("_verts", verts)?;
    tool.setattr("_moved_guides", guides)?;
    tool.setattr("_splanes", sections)?;
    tool.setattr("_images", images)?;
    if mode == "move" {
        tool.setattr("_labels", labels)?;
        tool.setattr("_dims", dims)?;
    }
    Ok(true)
}

#[pyfunction]
pub fn move_endpoint<'py>(ctx: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let scene = ctx.getattr("viewport")?.getattr("scene")?;
    if scene.getattr("selection")?.is_truthy()? {
        return Ok(None);
    }
    if let Some(s) = ctx.getattr_opt("snap")?.filter(|s| !s.is_none()) {
        if s.getattr("kind")?.eq("endpoint")? && !s.getattr("context")?.is_truthy()? {
            return Ok(Some(
                scene
                    .getattr("mesh")?
                    .call_method1("vertex_at", (s.getattr("point")?,))?,
            )
            .filter(|v| !v.is_none()));
        }
    }
    Ok(None)
}
