// SPDX-License-Identifier: GPL-3.0-or-later
//! Native steps that also change host state (docs/commands-port.md): the
//! north angle, Straighten Model and survey points with a datum.
//!
//! The datum (`scene.georef`) and the terrain are host objects, so these
//! steps keep explicit inverses, exactly as the Python commands did. A
//! survey import without a datum is document state only and is replayed by
//! its transaction.
use super::steps::{version, After, Obj, RecordStep};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::PyClassInitializer;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn datum_class(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("built_core.georef.datum")?.getattr("SceneDatum")
}

/// A datum like `datum` with another north angle.
fn datum_with_north<'py>(py: Python<'py>, datum: &Obj<'py>, lat: Obj<'py>, lon: Obj<'py>, north: f64) -> PyResult<Obj<'py>> {
    let kw = PyDict::new(py);
    kw.set_item("alt", datum.getattr("alt")?)?;
    kw.set_item("north", north)?;
    datum_class(py)?.call((lat, lon), Some(&kw))
}

// ------------------------------------------------------------ north

/// Memory: `[old north_deg, old datum, new datum, terrain]` (None where unused).
fn run_north(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let degrees: f64 = s.arg(py, 0).extract()?;
    let datum = scene.getattr_opt("georef")?.unwrap_or_else(|| none(py));
    let previous_new = s.remembered(py, 2);
    if datum.is_none() {
        let old = scene.getattr_opt("north_deg")?.filter(|v| v.is_truthy().unwrap_or(false));
        let old = old.map_or(Ok(0.0), |v| v.extract::<f64>())?;
        scene.setattr("north_deg", degrees)?;
        s.remember(vec![old.into_pyobject(py)?.into_any(), none(py), none(py), none(py)]);
    } else {
        // A new datum object: the map geometry is cached by datum identity.
        let new = match previous_new.filter(|d| !d.is_none()) {
            Some(d) => d,
            None => datum_with_north(py, &datum, datum.getattr("lat")?, datum.getattr("lon")?, degrees)?,
        };
        scene.setattr("georef", &new)?;
        let terrain = scene.getattr_opt("terrain")?.unwrap_or_else(|| none(py));
        if !terrain.is_none() {
            scene.setattr("terrain", none(py))?; // display only; rebuilt from the datum
        }
        s.remember(vec![none(py), datum, new, terrain]);
    }
    version(scene)
}

fn after_north(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase != Some("undo") || !s.is_direct() {
        return Ok(());
    }
    let old_datum = s.remembered(py, 1).unwrap_or_else(|| none(py));
    if old_datum.is_none() {
        if let Some(old) = s.remembered(py, 0) {
            scene.setattr("north_deg", old)?;
        }
    } else {
        scene.setattr("georef", old_datum)?;
        let terrain = s.remembered(py, 3).unwrap_or_else(|| none(py));
        if !terrain.is_none() {
            scene.setattr("terrain", terrain)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------ straighten

/// Memory: `[placement, degrees, old datum, new datum, terrain]`; `_snaps`
/// (an argument) keeps the meshes' before and after states.
fn run_straighten(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let group = s.arg(py, 0);
    if !scene.getattr("groups")?.contains(&group)? || group.getattr("xform")?.is_none() {
        return Err(PyValueError::new_err("pick a placed component at the top level"));
    }
    let datum = scene.getattr_opt("georef")?.unwrap_or_else(|| none(py));
    if datum.is_none() {
        return Err(PyValueError::new_err("the scene has no location yet"));
    }
    let matrix4 = py.import("built_core")?.getattr("Matrix4")?;
    if s.remembered(py, 0).is_none() {
        let degrees = crate::doc::history_support::placement_rotation_deg(&group.getattr("xform")?)?;
        let placement = matrix4.call1((group.getattr("xform")?,))?;
        let t = placement.call_method1("column", (3,))?;
        let at = py.import("built_core")?.getattr("Vec3")?.call1((t.call_method0("x")?, t.call_method0("y")?, 0.0))?;
        let geo = datum.call_method1("local_to_geodetic", (at,))?;
        let north: f64 = datum.getattr("north")?.extract()?;
        let new = datum_with_north(py, &datum, geo.get_item(0)?, geo.get_item(1)?, north + degrees)?;
        s.remember(vec![placement, degrees.into_pyobject(py)?.into_any(), datum.clone(), new, none(py)]);
    }
    let (placement, degrees) = (s.remembered(py, 0).unwrap(), s.remembered(py, 1).unwrap().extract::<f64>()?);
    let inverted = placement.call_method0("inverted")?;
    if !inverted.get_item(1)?.is_truthy()? {
        return Err(PyValueError::new_err("the placement is not invertible"));
    }
    let snaps = s.arg(py, 1).cast_into::<PyDict>()?;
    crate::placement_transform::reframe_world(scene, &inverted.get_item(0)?, -degrees, true, &snaps)?;
    version(scene)?;
    group.setattr("xform", matrix4.call0()?)?; // exactly on its axes, no float dust
    scene.setattr("georef", s.remembered(py, 3).unwrap())?;
    let terrain = scene.getattr_opt("terrain")?.unwrap_or_else(|| none(py));
    let mut memory: Vec<Obj<'_>> = (0..4).map(|i| s.remembered(py, i).unwrap()).collect();
    memory.push(terrain);
    s.remember(memory);
    scene.setattr("terrain", none(py)) // display only; rebuilt from the datum
}

fn after_straighten(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase != Some("undo") || !s.is_direct() {
        return Ok(());
    }
    let (Some(placement), Some(degrees)) = (s.remembered(py, 0), s.remembered(py, 1)) else {
        return Ok(());
    };
    let snaps = s.arg(py, 1).cast_into::<PyDict>()?;
    crate::placement_transform::reframe_world(scene, &placement, degrees.extract()?, false, &snaps)?;
    version(scene)?;
    let matrix4 = py.import("built_core")?.getattr("Matrix4")?;
    s.arg(py, 0).setattr("xform", matrix4.call1((placement,))?)?;
    scene.setattr("georef", s.remembered(py, 2).unwrap_or_else(|| none(py)))?;
    scene.setattr("terrain", s.remembered(py, 4).unwrap_or_else(|| none(py)))
}

// ------------------------------------------------------------ survey points

fn run_geo_points(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let datum = s.arg(py, 1);
    if !datum.is_none() {
        s.remember(vec![scene.getattr_opt("georef")?.unwrap_or_else(|| none(py))]);
        scene.setattr("georef", &datum)?;
    }
    scene.getattr("geo_points")?.call_method1("extend", (s.arg(py, 0),))?;
    version(scene)
}

fn after_geo_points(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let datum = s.arg(py, 1);
    if phase == Some("undo") && s.is_direct() && !datum.is_none() {
        let points = scene.getattr("geo_points")?;
        for p in s.items(py, 0)? {
            if points.contains(&p)? {
                points.call_method1("remove", (p,))?;
            }
        }
        scene.setattr("georef", s.remembered(py, 0).unwrap_or_else(|| none(py)))?;
    }
    version(scene)
}

// ------------------------------------------------------------ the classes

macro_rules! steps {
    ($($ty:ident = $name:literal;)*) => {
        $(
            #[pyclass(module = "built_core", name = $name, extends = RecordStep, frozen)]
            pub struct $ty;
        )*
        pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
            $(m.add_class::<$ty>()?;)*
            Ok(())
        }
    };
}

steps! {
    SetNorth = "SetNorthCommand";
    StraightenModel = "StraightenModelCommand";
    AddGeoPoints = "AddGeoPointsCommand";
}

#[pymethods]
impl SetNorth {
    /// Tools ▸ North: true north in degrees clockwise from green, folded
    /// into (−180, 180]. The model never moves.
    #[new]
    fn new(py: Python<'_>, degrees: f64) -> PyResult<PyClassInitializer<Self>> {
        let degrees = py.import("built_core.sun")?.call_method1("wrap_north_deg", (degrees,))?;
        let step = RecordStep::new("SetNorthCommand", run_north, vec![("degrees", degrees)])
            .explicit_inverse()
            .hook(After(after_north));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl StraightenModel {
    /// Put a model turned and dragged onto its site back on its own axes
    /// and turn the map under it instead.
    #[new]
    fn new(py: Python<'_>, group: Obj<'_>) -> PyClassInitializer<Self> {
        let step = RecordStep::new("StraightenModelCommand", run_straighten, vec![("group", group), ("_snaps", PyDict::new(py).into_any())])
            .explicit_inverse()
            .hook(After(after_straighten));
        PyClassInitializer::from(step).add_subclass(Self)
    }

    /// The turn the straightening took off (degrees), once it ran.
    #[getter]
    fn degrees(slf: PyRef<'_, Self>) -> PyResult<f64> {
        let py = slf.py();
        let step: &RecordStep = &*slf.as_super();
        step.remembered(py, 1).map_or(Ok(0.0), |d| d.extract())
    }
}

#[pymethods]
impl AddGeoPoints {
    /// Import survey points; a datum anchored by the same import travels
    /// with the step, and undo restores the previous one.
    #[new]
    #[pyo3(signature = (points, datum = None))]
    fn new(py: Python<'_>, points: Obj<'_>, datum: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let datum = datum.unwrap_or_else(|| none(py));
        let points = PyList::new(py, points.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any();
        let anchored = !datum.is_none();
        let mut step = RecordStep::new("AddGeoPointsCommand", run_geo_points, vec![("_points", points), ("_datum", datum)])
            .hook(After(after_geo_points));
        if anchored {
            step = step.explicit_inverse();
        }
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}
