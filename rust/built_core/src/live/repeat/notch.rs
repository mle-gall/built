// SPDX-License-Identifier: GPL-3.0-or-later
//! Rectangular members notched by one planar box commonly keep the same
//! topology across an entire bay. Reuse the kernel's donor roles and loops
//! only while the intersection of the two convex solids has the same corners.
use super::*;

#[derive(Clone, Copy)]
struct Plane {
    normal: DVec3,
    offset: f64,
}

fn sides(mesh: &Obj<'_>, pose: DMat4) -> PyResult<Option<Vec<Plane>>> {
    let handle = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let borrow = handle.try_borrow()?;
    let store = &borrow.inner;
    if store.n(Kind::Face) != 6
        || store.n(Kind::Vertex) != 8
        || store.n(Kind::Edge) != 12
        || store.state.exact.curved.is_some()
        || store.state.exact.arcs.is_some()
        || store.state.exact.native.is_some()
    {
        return Ok(None);
    }
    let vertex = |v: u32| {
        pose.transform_point3(
            store
                .state
                .frame
                .definition_point(*store.state.vpos.get(v as usize))
                .into(),
        )
    };
    let vertices: Vec<_> = store
        .live_slots(Kind::Vertex)
        .into_iter()
        .map(vertex)
        .collect();
    let centre = vertices.iter().copied().sum::<DVec3>() / 8.;
    let mut sides = Vec::new();
    for f in store.live_slots(Kind::Face) {
        let rings = store.face_rings(f);
        if rings.len() != 1 || rings[0].len() != 4 {
            return Ok(None);
        }
        let p: Vec<_> = rings[0].iter().map(|v| vertex(*v)).collect();
        let Some(mut n) = (p[1] - p[0]).cross(p[2] - p[0]).try_normalize() else {
            return Ok(None);
        };
        if n.dot(centre - p[0]) > 0. {
            n = -n;
        }
        if n.dot(p[3] - p[0]).abs() > 1e-9 || vertices.iter().any(|v| n.dot(*v - p[0]) > 1e-9) {
            return Ok(None);
        }
        sides.push(Plane {
            normal: n,
            offset: n.dot(p[0]),
        });
    }
    Ok(Some(sides))
}

fn intersect(a: Plane, b: Plane, c: Plane) -> Option<DVec3> {
    let bc = b.normal.cross(c.normal);
    let det = a.normal.dot(bc);
    (det.abs() > 1e-9).then(|| {
        (a.offset * bc + b.offset * c.normal.cross(a.normal) + c.offset * a.normal.cross(b.normal))
            / det
    })
}

struct Corners {
    // A point may lie in four or more supporting planes.
    overlap: Vec<u16>,
    vertices: Vec<(u16, DVec3)>,
}

impl Corners {
    fn from(planes: &[Plane]) -> Self {
        let mut overlap = Vec::new();
        let mut vertices = Vec::new();
        for k in 2..planes.len() {
            for j in 1..k {
                for i in 0..j {
                    let Some(p) = intersect(planes[i], planes[j], planes[k]) else {
                        continue;
                    };
                    let mut mask = 0u16;
                    let mut outside = [false; 2];
                    for (ix, plane) in planes.iter().enumerate() {
                        let side = plane.normal.dot(p) - plane.offset;
                        if side.abs() < 1e-8 {
                            mask |= 1 << ix;
                        }
                        if side > 1e-8 {
                            outside[ix / 6] = true;
                        }
                    }
                    if !outside[0] && (!outside[1] || k < 6) {
                        if !outside[1] {
                            overlap.push(mask);
                        }
                        vertices.push((mask, p));
                    }
                }
            }
        }
        overlap.sort_unstable();
        overlap.dedup();
        Self { overlap, vertices }
    }
}

#[derive(Default)]
pub(super) struct Cache<'p> {
    template: Option<(Obj<'p>, Vec<u16>, Vec<(u32, u16)>)>,
}

impl<'p> Cache<'p> {
    pub(super) fn part(
        &mut self,
        scene: &Obj<'p>,
        table: &LiveTable,
        recipe: &Row,
        stock: &Obj<'p>,
        tool: &Obj<'p>,
    ) -> PyResult<Result<Obj<'p>, &'static str>> {
        let target = reference::frame(table.groups[&recipe.owner]);
        let pose = target.inverse() * reference::frame(tool.getattr("_doc_key")?.extract()?);
        let planes = sides(stock, DMat4::IDENTITY)?.zip(sides(&tool.getattr("mesh")?, pose)?);
        let corners = planes.map(|(mut a, b)| {
            a.extend(b);
            Corners::from(&a)
        });
        if let (Some(c), Some((mesh, signature, bindings))) = (&corners, &self.template) {
            if !c.overlap.is_empty() && signature == &c.overlap {
                let pts: HashMap<_, _> = c.vertices.iter().copied().collect();
                if let Some(positions) = bindings
                    .iter()
                    .map(|(v, mask)| pts.get(mask).map(|p| (*v, *p)))
                    .collect::<Option<Vec<_>>>()
                {
                    let mesh = scene
                        .py()
                        .import("copy")?
                        .call_method1("deepcopy", (mesh,))?;
                    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                    let mut borrow = store.try_borrow_mut()?;
                    for (slot, pos) in positions {
                        borrow.inner.place_vertex(slot, pos.to_array());
                    }
                    return Ok(Ok(mesh));
                }
            }
        }
        let mut skipped = Vec::new();
        let result = cut::bodies(scene, table, recipe, stock.clone(), &mut None, &mut skipped)?;
        if let Some((_, state)) = skipped.first() {
            return Ok(Err(*state));
        }
        if let (Some(c), Ok(mesh)) = (corners, &result) {
            if !c.overlap.is_empty() {
                let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                let borrow = store.try_borrow()?;
                let s = &borrow.inner;
                if s.state.frame.is_model() && s.state.exact.planes.is_none() {
                    let bindings = s
                        .live_slots(Kind::Vertex)
                        .into_iter()
                        .map(|v| {
                            let p = DVec3::from(*s.state.vpos.get(v as usize));
                            c.vertices
                                .iter()
                                .find(|(_, q)| q.distance(p) < 1e-7)
                                .map(|(key, _)| (v, *key))
                        })
                        .collect::<Option<Vec<_>>>();
                    if let Some(bindings) = bindings {
                        self.template = Some((mesh.clone(), c.overlap, bindings));
                    }
                }
            }
        }
        Ok(result)
    }
}
