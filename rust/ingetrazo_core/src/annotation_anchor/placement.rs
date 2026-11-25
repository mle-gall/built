// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn create<'p>(
    cls: &Obj<'p>,
    scene: &Obj<'p>,
    path: &Obj<'p>,
    attachment: &Obj<'p>,
) -> PyResult<Option<Obj<'p>>> {
    let hold = cls.call0()?;
    hold.setattr("path", path.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
    hold.setattr("attachment", attachment)?;
    let f = current_frame(&hold, scene)?;
    hold.setattr("frame", &f)?;
    Ok((!f.is_none()).then_some(hold))
}
#[pyfunction]
fn current_frame<'p>(s: &Obj<'p>, scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let mut path = s.getattr("path")?.extract::<Vec<String>>()?;
    path.pop();
    let parent = if path.is_empty() {
        None
    } else {
        Some(scene.call_method1("resolve_occurrence", (path,))?)
    };
    s.getattr("attachment")?
        .call_method1("placement_frame", (matrix(scene, parent.as_ref())?,))
}
#[pyfunction]
fn follow(s: &Obj, scene: &Obj) -> PyResult<Option<bool>> {
    let sign = scene.call_method1("resolve_occurrence", (s.getattr("path")?,))?;
    if sign.is_none() {
        return Ok(None);
    }
    let frame = current_frame(s, scene)?;
    if !frame.is_none() {
        let mut x = sign.getattr("xform")?;
        if x.is_none() {
            x = s.py().import("ingetrazo_core")?.call_method0("Matrix4")?;
        }
        let pose = s.getattr("attachment")?.getattr("binding")?.call_method1(
            "follow_placement",
            (s.getattr("frame")?, &frame, x.call_method0("data")?),
        )?;
        if !pose.is_none() {
            sign.setattr("xform", pose)?;
            s.setattr("frame", frame)?;
            return Ok(Some(true));
        }
    }
    Ok(Some(false))
}
#[pyfunction]
fn archive<'p>(s: &Obj<'p>, scene: &Obj<'p>) -> PyResult<Option<Bound<'p, PyDict>>> {
    let path = s.getattr("path")?;
    if scene
        .call_method1("resolve_occurrence", (&path,))?
        .is_none()
    {
        return Ok(None);
    }
    let a = s.getattr("attachment")?;
    let source = a.call_method0("archive")?;
    let point = a.call_method0("position")?;
    if source.is_none() || point.is_none() {
        return Ok(None);
    }
    let out = PyDict::new(s.py());
    out.set_item("sign", path.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
    out.set_item("source", source)?;
    out.set_item("point", point.call_method0("toTuple")?)?;
    Ok(Some(out))
}
#[pyfunction]
fn restore<'p>(cls: &Obj<'p>, scene: &Obj<'p>, raw: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let result = (|| {
        let path = raw.get_item("sign")?;
        if !path.is_truthy()? || occurrence(scene, &path)?.is_none() {
            return Ok(None);
        }
        let point = Bound::new(scene.py(), Vec3::from(xyz(&raw.get_item("point")?)?))?;
        let Some(a) = Attachment::restore(scene, &raw.get_item("source")?, &point)? else {
            return Ok(None);
        };
        create(cls, scene, &path, Bound::new(scene.py(), a)?.as_any())
    })();
    match result {
        Err(e) if malformed(scene.py(), &e) => Ok(None),
        r => r,
    }
}
#[pyfunction(name = "__getattr__")]
fn lazy<'p>(py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
    if name != "TextPlacementAnchor" {
        return Err(PyAttributeError::new_err(name.to_owned()));
    }
    let m = py.import(MODULE)?;
    crate::record_class::build(
        &m,
        name,
        &["path", "attachment", "frame"],
        false,
        vec![
            wrap_pyfunction!(create, &m)?,
            wrap_pyfunction!(current_frame, &m)?,
            wrap_pyfunction!(follow, &m)?,
            wrap_pyfunction!(archive, &m)?,
            wrap_pyfunction!(restore, &m)?,
        ],
        &["create", "restore"],
    )
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(lazy, m)?)
}
