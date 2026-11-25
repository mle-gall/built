// SPDX-License-Identifier: GPL-3.0-or-later
//! Sample terrain when its path or elevation source changes. Camera movement
//! reprojects the cached elevations without resampling the terrain.
use super::*;
type Obj<'p> = Bound<'p, PyAny>;
#[pyclass]
struct Profile {
    path: Py<PyAny>,
    surface: Option<Py<PyAny>>,
    sampler: Option<Py<PyAny>>,
    samples: Option<Py<PyAny>>,
    version: Option<u64>,
    projected: Option<(Vec<f64>, i32, i32)>,
    vertical: &'static str,
    altitude: f64,
    #[pyo3(get, set)]
    dirty: bool,
}
fn detach(slf: &Bound<Profile>, view: &Obj) -> PyResult<()> {
    let py = slf.py();
    let sampler = slf.borrow_mut().sampler.take();
    if let Some(s) = sampler {
        if let Some(changed) = s.bind(py).getattr_opt("changed")? {
            changed.call_method1("disconnect", (slf.getattr("changed")?,))?;
            changed.call_method1("disconnect", (view.getattr("update")?,))?;
        }
    }
    Ok(())
}
#[pymethods]
impl Profile {
    fn changed(&mut self) {
        self.dirty = true;
    }
    fn sync(slf: &Bound<Self>, view: &Obj, vp: &Obj) -> PyResult<()> {
        let py = slf.py();
        let scene = vp.getattr("scene")?;
        let datum = scene.getattr("georef")?;
        let path = slf.borrow().path.clone_ref(py).into_bound(py);
        if datum.is_none() || !scene.getattr("geo_paths")?.contains(&path)? {
            return action(view, vp, "close");
        }
        let survey = opt(&scene, "photo_mesh")
            .filter(|s| opt(s, "visible").is_some_and(|v| v.is_truthy().unwrap_or(false)));
        let surface = (&survey, &datum).into_pyobject(py)?.into_any();
        let same = slf
            .borrow()
            .surface
            .as_ref()
            .map(|v| v.bind(py).eq(&surface))
            .transpose()?
            .unwrap_or(false);
        if !same {
            detach(slf, view)?;
            let sampler = if let Some(survey) = &survey {
                py.import("built_core.georef.photomesh")?
                    .call_method1("PhotoMeshSampler", (survey, &datum))?
            } else {
                let args = PyDict::new(py);
                if view.hasattr("_host")? {
                    args.set_item("parent", view)?;
                }
                let sampler = py
                    .import("built_core.georef.dem")?
                    .getattr("DEMSampler")?
                    .call((&datum,), Some(&args))?;
                let changed = sampler.getattr("changed")?;
                changed.call_method1("connect", (slf.getattr("changed")?,))?;
                changed.call_method1("connect", (view.getattr("update")?,))?;
                sampler
            };
            let mut p = slf.borrow_mut();
            p.sampler = Some(sampler.unbind());
            p.surface = Some(surface.unbind());
            p.dirty = true;
        }
        let version = scene.getattr("version")?.extract()?;
        if slf.borrow().dirty || slf.borrow().version != Some(version) {
            slf.borrow_mut().dirty = false;
            let sampler = slf.borrow().sampler.as_ref().unwrap().clone_ref(py);
            let profile = py.import("built_core.georef.profile")?.call_method1(
                "sample_profile",
                (path.call_method0("profile_points")?, sampler),
            )?;
            let data = PyDict::new(py);
            let name = path.getattr("name")?;
            data.set_item(
                "name",
                if name.is_truthy()? {
                    name.extract::<String>()?
                } else {
                    "Path".into()
                },
            )?;
            data.set_item(
                "source",
                if survey.is_some() {
                    "Field survey"
                } else {
                    "Global DEM"
                },
            )?;
            let samples = PyList::empty(py);
            for s in profile.getattr("samples")?.try_iter()? {
                let s = s?;
                samples.append((s.getattr("station")?, s.getattr("elevation")?))?;
            }
            data.set_item("samples", samples)?;
            send(view, "terrain", data.as_any())?;
            let mut p = slf.borrow_mut();
            p.samples = Some(profile.unbind());
            p.vertical = if survey.is_some() { "odm" } else { "dem" };
            p.altitude = datum.getattr("alt")?.extract()?;
            p.version = Some(version);
            p.projected = None;
        }
        let cam = vp.getattr("camera")?;
        let mat = cam
            .call_method0("projection_matrix")?
            .mul(cam.call_method0("view_matrix")?)?;
        let key = (
            mat.call_method0("data")?.extract()?,
            view.call_method0("width")?.extract()?,
            view.call_method0("height")?.extract()?,
        );
        if slf.borrow().projected.as_ref() != Some(&key) {
            let points = PyList::empty(py);
            let vec = py.import("built_core")?.getattr("Vec3")?;
            let samples = slf
                .borrow()
                .samples
                .as_ref()
                .unwrap()
                .bind(py)
                .getattr("samples")?;
            for s in samples.try_iter()? {
                let s = s?;
                let pos = vec.call1((s.getattr("x")?, s.getattr("y")?, 0.))?;
                points.append(vp.call_method1(
                    "_world_to_pixel",
                    (vp.call_method1("drape", (pos,))?, &mat),
                )?)?;
            }
            send(view, "projected", points.as_any())?;
            slf.borrow_mut().projected = Some(key);
        }
        Ok(())
    }
}
pub(super) fn action(view: &Obj, vp: &Obj, verb: &str) -> PyResult<()> {
    let py = view.py();
    let scene = vp.getattr("scene")?;
    let previous = opt(view, "_profile_binding")
        .map(|p| p.cast_into::<Profile>())
        .transpose()?;
    if verb == "close" {
        if let Some(p) = previous {
            detach(&p, view)?;
        }
        view.setattr("_profile_binding", py.None())?;
        return send(view, "terrain", &py.None().into_bound(py));
    }
    if verb == "open" {
        let path = match &previous {
            Some(p) => p.borrow().path.clone_ref(py).into_bound(py),
            None => py
                .import("built_core.georef.profile")?
                .call_method1("selected_geopath", (&scene,))?,
        };
        if path.is_none()
            || path.getattr("points")?.len()? < 2
            || scene.getattr("georef")?.is_none()
        {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Select a path and set the base map location first",
            ));
        }
        if let Some(p) = previous {
            detach(&p, view)?;
        }
        let p = Bound::new(
            py,
            Profile {
                path: path.unbind(),
                surface: None,
                sampler: None,
                samples: None,
                version: None,
                projected: None,
                vertical: "dem",
                altitude: 0.,
                dirty: true,
            },
        )?;
        view.setattr("_profile_binding", &p)?;
        return Profile::sync(&p, view, vp);
    }
    if let Some(p) = previous {
        let p = p.borrow();
        let Some(profile) = &p.samples else {
            return Ok(());
        };
        let (ext, data) = if verb == "csv" {
            (
                "csv",
                py.import("built_core.georef.profile")?
                    .call_method1("profile_to_csv", (profile.bind(py), p.vertical, p.altitude))?
                    .extract::<String>()?
                    .into_bytes(),
            )
        } else {
            (
                "png",
                view.getattr("renderer")?
                    .call_method0("ui_profile_png")?
                    .extract()?,
            )
        };
        let path = py
            .import("built_render.shell_files")?
            .getattr("ShellFileDialog")?
            .call_method1(
                "getSaveFileName",
                (
                    py.None(),
                    "Export terrain profile",
                    format!("profile.{ext}"),
                    format!("*.{ext}"),
                ),
            )?
            .get_item(0)?;
        if path.is_truthy()? {
            std::fs::write(path.extract::<std::path::PathBuf>()?, data)?;
        }
    }
    Ok(())
}
