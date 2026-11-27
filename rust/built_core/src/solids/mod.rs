// SPDX-License-Identifier: GPL-3.0-or-later
//! Boolean solid operations. World-space inputs carry face and curve provenance
//! through manifold3d, then return to the document as editable polygon faces.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    exceptions::{PyException, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyTuple},
    IntoPyObjectExt,
};
use std::collections::{HashMap, HashSet};
type Obj<'p> = Bound<'p, PyAny>;
mod bindings;
mod command;
mod output;
mod surface;
use surface::Surface;
pyo3::create_exception!(solids, SolidError, PyException);
fn error(_: Python<'_>, reason: &str) -> PyErr {
    SolidError::new_err(reason.to_owned())
}
const OPERATIONS: [(&str, &str); 6] = [
    ("outer_shell", "Outer shell"),
    ("union", "Union"),
    ("subtract", "Difference"),
    ("trim", "Trimmed"),
    ("intersect", "Intersection"),
    ("split", "Split"),
];
fn items<'p>(seq: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    seq.try_iter()?.collect()
}
fn none(py: Python) -> Obj {
    py.None().into_bound(py)
}
fn world<'p>(group: &Obj<'p>) -> PyResult<Obj<'p>> {
    group
        .py()
        .import("built_core")?
        .call_method1("group_world_mesh", (group,))
}
fn lazily_open(mesh: &Obj) -> PyResult<bool> {
    let plan = mesh
        .py()
        .import("built_core")?
        .call_method1("plan_of", (mesh,))?;
    if plan.is_none() {
        return Ok(false);
    }
    open_plan(&plan)
}

pub(crate) fn open_plan(plan: &Obj) -> PyResult<bool> {
    let offsets = plan.getattr("ef_start")?;
    use numpy::PyReadonlyArray1;
    macro_rules! open_offsets {
        ($ty:ty) => {
            if let Ok(array) = offsets.extract::<PyReadonlyArray1<$ty>>() {
                let offsets = array.as_array();
                return Ok(offsets
                    .iter()
                    .zip(offsets.iter().skip(1))
                    .any(|(&a, &b)| i128::from(b) - i128::from(a) != 2));
            }
        };
    }
    open_offsets!(i32);
    open_offsets!(i64);
    open_offsets!(u32);
    open_offsets!(u64);
    let starts = offsets.extract::<Vec<i64>>()?;
    Ok(starts.windows(2).any(|w| w[1] - w[0] != 2))
}
fn store(mesh: &Obj) -> PyResult<bool> {
    mesh.is_instance(
        &mesh
            .py()
            .import("built_core.store_mesh")?
            .getattr("StoreMesh")?,
    )
}
fn installed_hypot<'p>(mesh: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    if !store(mesh)? {
        return Ok(None);
    }
    let hypot = mesh.py().import("core._native")?.call_method0("qt_hypot")?;
    Ok(hypot.is_truthy()?.then_some(hypot))
}
#[pyfunction]
fn solid_report(mesh: &Obj) -> PyResult<(bool, Option<f64>)> {
    if lazily_open(mesh)? || !mesh.getattr("faces")?.is_truthy()? {
        return Ok((false, None));
    }
    if let Some(h) = installed_hypot(mesh)? {
        let v = mesh
            .getattr("_store")?
            .call_method0("snapshot")?
            .call_method1("solid_volume", (h,))?
            .extract::<Option<f64>>()?;
        return Ok((v.is_some(), v));
    }
    let skin = |face: &Obj| -> PyResult<bool> { Ok(!face.getattr("interior")?.is_truthy()?) };
    for e in mesh.getattr("edges")?.try_iter()? {
        let mut n = 0;
        for f in e?.getattr("faces")?.try_iter()? {
            if skin(&f?)? {
                n += 1;
            }
        }
        if n != 2 {
            return Ok((false, None));
        }
    }
    let py = mesh.py();
    let faces = PyList::empty(py);
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if skin(&f)? {
            faces.append(f)?;
        }
    }
    let vol = py
        .import("core.bim")?
        .call_method1("face_set_volume", (faces,))?
        .extract::<Option<f64>>()?
        .filter(|v| *v > 1e-12);
    Ok((vol.is_some(), vol))
}
#[pyfunction]
fn solid_volume(group: &Obj) -> PyResult<Option<f64>> {
    if !group.is_instance(&group.py().import("built_core.group")?.getattr("Group")?)?
        || group.getattr("children")?.is_truthy()?
        || lazily_open(&group.getattr("mesh")?)?
    {
        return Ok(None);
    }
    let mesh = group.getattr("mesh")?;
    let transform = group.getattr("xform")?;
    let scale = if transform.is_none() {
        1.
    } else {
        crate::math::mat(&transform)?.determinant().abs()
    };
    Ok(solid_report(&mesh)?
        .1
        .map(|v| v * scale)
        .filter(|v| *v > 1e-12))
}
#[pyfunction]
fn is_solid(group: &Obj) -> PyResult<bool> {
    Ok(solid_volume(group)?.is_some())
}
struct EdgeFlag<'p> {
    a: DVec3,
    b: DVec3,
    curve: Option<(usize, i64)>,
    layer: Obj<'p>,
    hidden: bool,
}
#[derive(Default)]
struct Sources<'p> {
    live: Option<(String, usize)>,
    roles: Vec<(String, u64, bool)>,
    attrs: Vec<Bound<'p, PyDict>>,
    donors: Vec<bindings::Donor>,
    soft: HashSet<(usize, usize)>,
    flat: HashSet<(usize, usize)>,
    edges: Vec<EdgeFlag<'p>>,
}
fn order(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}
fn point_key(p: DVec3) -> [u64; 3] {
    p.to_array().map(|v| if v == 0. { 0 } else { v.to_bits() })
}
impl<'p> Sources<'p> {
    fn operand(&mut self, group: &Obj<'p>, operand: usize) -> PyResult<Obj<'p>> {
        let py = group.py();
        let definition = group.getattr("mesh")?;
        if let Some((uid, tool)) = &self.live {
            use crate::store::{keys::Kind, py::PyMeshStore};
            let handle = definition.getattr("_store")?.cast_into::<PyMeshStore>()?;
            let borrow = handle.try_borrow()?;
            let st = &borrow.inner;
            for slot in st.live_slots(Kind::Face) {
                if st.state.fflags.get(slot as usize) & crate::store::INTERIOR != 0 {
                    continue;
                }
                let pid = st.pid(Kind::Face, slot);
                let role = crate::element_ids::to_text('f', pid);
                self.roles.push((
                    if operand == *tool {
                        format!("cut/{uid}/{role}")
                    } else {
                        role
                    },
                    pid,
                    operand == *tool,
                ));
            }
        }
        let xf = group.getattr("xform")?;
        let placement = (!xf.is_none()).then(|| crate::math::mat(&xf)).transpose()?;
        let native = definition.getattr_opt("_store")?.is_some_and(|st| {
            st.getattr("native_exact")
                .and_then(|v| v.extract::<bool>())
                .unwrap_or(false)
        });
        let sample_placement = native.then_some(placement).flatten();
        let mesh = if native {
            definition.clone()
        } else {
            world(group)?
        };
        let mut donors = bindings::donors(&definition, placement)?.into_iter();
        let mat = py
            .import("built_core.group")?
            .call_method1("effective_material", (group,))?;
        let paint = py
            .import("built_core.materials")?
            .getattr("effective_attrs")?;
        let mut soup = Surface::empty();
        let ids = PyDict::new(py);
        let mut index = HashMap::new();
        let indexed = installed_hypot(&mesh)?;
        let mut skin = Vec::new();
        if let Some(h) = &indexed {
            let packet = mesh
                .getattr("_store")?
                .call_method0("snapshot")?
                .call_method1("indexed_surface", (h,))?;
            soup = Surface::read(
                &packet.get_item(0)?,
                &packet.get_item(1)?,
                &packet.get_item(2)?,
            )?;
            for f in &mut soup.owners {
                *f += self.attrs.len() as u32;
            }
            let class = py.import("built_core.store_mesh")?.getattr("FaceView")?;
            for key in packet.get_item(3)?.try_iter()? {
                skin.push(class.call1((&mesh, key?))?);
            }
        } else {
            for f in mesh.getattr("faces")?.try_iter()? {
                let f = f?;
                if !f.getattr("interior")?.is_truthy()? {
                    skin.push(f);
                }
            }
        }
        for face in skin {
            let fid = self.attrs.len();
            ids.set_item(face.getattr("key")?, fid)?;
            let dict = py.get_type::<PyDict>();
            let attr = face.getattr("attrs")?;
            let raw = if attr.is_truthy()? {
                dict.call1((attr,))?
            } else {
                PyDict::new(py).into_any()
            };
            let attrs = paint.call1((raw, &mat))?;
            let saved = PyDict::new(py);
            for (k, v) in attrs.cast::<PyDict>()?.iter() {
                if let Ok(d) = v.cast::<PyDict>() {
                    saved.set_item(k, d.copy()?)?;
                } else {
                    saved.set_item(k, v)?;
                }
            }
            self.attrs.push(saved);
            self.donors.push(donors.next().unwrap_or_default());
            if indexed.is_none() {
                for tri in face.call_method0("triangulate")?.try_iter()? {
                    let points: [Obj; 3] = tri?.extract()?;
                    let mut corners = [0; 3];
                    for (dst, p) in corners.iter_mut().zip(points) {
                        let p = xyz(&p)?;
                        *dst = *index.entry(point_key(p)).or_insert_with(|| {
                            let i = soup.points.len();
                            soup.points.push(p);
                            i as u32
                        });
                    }
                    soup.triangles.push(corners);
                    soup.owners.push(fid as u32);
                }
            }
        }
        for edge in mesh.getattr("edges")?.try_iter()? {
            let e = edge?;
            if e.getattr("soft")?.is_truthy()? {
                let mut faces = Vec::new();
                for f in e.getattr("faces")?.try_iter()? {
                    if let Some(id) = ids.get_item(f?.getattr("key")?)? {
                        faces.push(id.extract()?);
                    }
                }
                if let [a, b] = faces[..] {
                    self.soft.insert(order(a, b));
                    if e.getattr("flat")?.is_truthy()? {
                        self.flat.insert(order(a, b));
                    }
                }
            }
            let curve = e
                .getattr("curve")?
                .extract::<Option<i64>>()?
                .map(|id| (operand, id));
            let layer = e.getattr("layer")?;
            let hidden = e.getattr("hidden")?.is_truthy()?;
            if curve.is_some() || !layer.is_none() || hidden {
                self.edges.push(EdgeFlag {
                    a: sample_placement
                        .unwrap_or(glam::DMat4::IDENTITY)
                        .transform_point3(xyz(&e.getattr("a")?)?),
                    b: sample_placement
                        .unwrap_or(glam::DMat4::IDENTITY)
                        .transform_point3(xyz(&e.getattr("b")?)?),
                    curve,
                    layer,
                    hidden,
                });
            }
        }
        if let Some(m) = sample_placement {
            for p in &mut soup.points {
                *p = m.transform_point3(*p);
            }
            if m.determinant() < 0. {
                for t in &mut soup.triangles {
                    t.swap(1, 2);
                }
            }
        }
        soup.manifold(py)
    }
}
#[pyfunction]
fn run<'p>(op: &str, groups: &Obj<'p>) -> PyResult<(Bound<'p, PyList>, Bound<'p, PyList>)> {
    operation(op, groups, None, 0.)
}

fn expanded<'p>(manifold: &Obj<'p>, clearance: f64) -> PyResult<Obj<'p>> {
    let ball = manifold
        .py()
        .import("manifold3d")?
        .getattr("Manifold")?
        .call_method1("sphere", (clearance / 1000., 12))?;
    manifold.call_method1("minkowski_sum", (ball,))
}

/// The IFC opening uses the same offset solid as the evaluated subtraction.
pub(crate) fn clearance_operand<'p>(tool: &Obj<'p>, clearance: f64) -> PyResult<Obj<'p>> {
    let mut sources = Sources::default();
    let manifold = expanded(&sources.operand(tool, 0)?, clearance)?;
    output::group(
        &manifold,
        &sources,
        &tool.getattr("name")?,
        &tool.getattr("layer")?,
        false,
    )?
    .ok_or_else(|| error(tool.py(), "empty clearance operand"))
}

/// Keep intermediates in the manifold kernel. Only the final solid needs
/// editable polygon faces, exact donor bindings and persistent element IDs.
pub(crate) fn body_stack<'p>(
    subject: &Obj<'p>,
    tools: Vec<(String, String, f64, Obj<'p>)>,
    active: &mut Option<String>,
    skipped: &mut Vec<(String, &'static str)>,
) -> PyResult<Result<Obj<'p>, &'static str>> {
    let mut sources = Sources {
        live: Some((String::new(), usize::MAX)),
        ..Default::default()
    };
    let mut result = sources.operand(subject, 0)?;
    for (index, (uid, op, gap, tool)) in tools.into_iter().enumerate() {
        *active = Some(uid.clone());
        if !is_solid(&tool)? {
            skipped.push((uid, "boolean_failed"));
            continue;
        }
        sources.live = Some((uid.clone(), index + 1));
        let mut cutter = sources.operand(&tool, index + 1)?;
        if gap > 0. {
            cutter = expanded(&cutter, gap)?;
        }
        let next = match op.as_str() {
            "union" => result.add(&cutter)?,
            "intersect" => result.bitxor(&cutter)?,
            _ => {
                if result
                    .bitxor(&cutter)?
                    .call_method0("is_empty")?
                    .is_truthy()?
                {
                    skipped.push((uid, "cut_misses"));
                    continue;
                }
                result.sub(&cutter)?
            }
        };
        if next.call_method0("is_empty")?.is_truthy()? {
            skipped.push((
                uid,
                if op == "intersect" {
                    "cut_misses"
                } else {
                    "empty_cut"
                },
            ));
            continue;
        }
        result = next;
    }
    let name = subject.getattr("name")?;
    let mesh = output::group(&result, &sources, &name, &subject.getattr("layer")?, false)?;
    match mesh {
        Some(g) => Ok(Ok(g.getattr("mesh")?)),
        None => Ok(Err("empty_cut")),
    }
}

pub(crate) fn operation<'p>(
    op: &str,
    groups: &Obj<'p>,
    cut: Option<&str>,
    clearance: f64,
) -> PyResult<(Bound<'p, PyList>, Bound<'p, PyList>)> {
    let py = groups.py();
    let Some((_, label)) = OPERATIONS.iter().find(|(k, _)| *k == op) else {
        return Err(PyValueError::new_err(op.to_owned()));
    };
    let gs = items(groups)?;
    if gs.len() < 2 {
        return Err(error(py, "two solids"));
    }
    let tool = usize::from(!matches!(op, "subtract" | "trim"));
    let mut sources = Sources {
        live: cut.map(|id| (id.to_owned(), tool)),
        ..Default::default()
    };

    for g in &gs {
        if !is_solid(g)? {
            return Err(error(py, "not a solid"));
        }
    }
    let mut mans = gs
        .iter()
        .enumerate()
        .map(|(i, g)| sources.operand(g, i))
        .collect::<PyResult<Vec<_>>>()?;
    if clearance > 0. {
        mans[tool] = expanded(&mans[tool], clearance)?;
    }
    let result = PyList::empty(py);
    let name = py
        .import("built_core.i18n")?
        .call_method1("tr", (*label,))?;
    let add = |man: &Obj, operand: usize, outer: bool| -> PyResult<bool> {
        if let Some(g) = output::group(man, &sources, &name, &gs[operand].getattr("layer")?, outer)?
        {
            result.append(g)?;
            return Ok(true);
        }
        Ok(false)
    };
    let lib = py.import("manifold3d")?;
    let removed = match op {
        "union" | "outer_shell" | "intersect" => {
            let intersect = op == "intersect";
            let mode =
                lib.getattr("OpType")?
                    .getattr(if intersect { "Intersect" } else { "Add" })?;
            let man = lib
                .getattr("Manifold")?
                .call_method1("batch_boolean", (PyList::new(py, &mans)?, mode))?;
            if !add(&man, 0, op == "outer_shell")? {
                return Err(error(py, if intersect { "no overlap" } else { "empty" }));
            }
            PyList::new(py, gs)?
        }
        _ => {
            let (a, b) = (&mans[0], &mans[1]);
            let overlap = a.bitxor(b)?;
            if overlap.call_method0("is_empty")?.is_truthy()? {
                return Err(error(py, "no overlap"));
            }
            let second = add(&b.sub(a)?, 1, false)?;
            if op == "split" {
                add(&a.sub(b)?, 0, false)?;
                add(&overlap, 0, false)?;
            } else if !second {
                return Err(error(py, "empty"));
            }
            PyList::new(py, if op == "trim" { &gs[1..2] } else { &gs[..2] })?
        }
    };
    Ok((removed, result))
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let names = PyDict::new(py);
    for (op, label) in OPERATIONS {
        m.add(op.to_uppercase(), op)?;
        names.set_item(op, label)?;
    }
    m.add("RESULT_NAMES", names)?;
    m.add("OPS", PyTuple::new(py, OPERATIONS.map(|(k, _)| k))?)?;
    let ex = py.get_type::<SolidError>();
    ex.setattr("__module__", "built_core.solids")?;
    m.add("SolidError", ex)?;
    for f in [
        wrap_pyfunction!(solid_report, m)?,
        wrap_pyfunction!(solid_volume, m)?,
        wrap_pyfunction!(is_solid, m)?,
        wrap_pyfunction!(run, m)?,
        wrap_pyfunction!(surface::_conform_t_junctions, m)?,
    ] {
        m.add_function(f)?;
    }
    command::install(m)
}
