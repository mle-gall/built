// SPDX-License-Identifier: GPL-3.0-or-later
//! File choices and cancellable placement in the originating survey document.
use super::*;
use crate::settings_rows::{func, obj, tr, trf};
use pyo3::{
    types::PyDict,
    IntoPyObjectExt,
};
fn choose<'p>(win: &Obj<'p>, title: &str, filter: &str) -> PyResult<Option<Obj<'p>>> {
    let py = win.py();
    let path = py
        .import("ingetrazo_render.shell_files")?
        .getattr("ShellFileDialog")?
        .call_method1(
            "getOpenFileName",
            (win, tr(py, title)?, "", tr(py, filter)?),
        )?
        .get_item(0)?;
    if !path.is_truthy()? {
        return Ok(None);
    }
    Ok(Some(py.import("pathlib")?.call_method1("Path", (path,))?))
}
fn status(win: &Obj, message: String, ms: u32) -> PyResult<()> {
    win.call_method0("statusBar")?
        .call_method1("showMessage", (message, ms))?;
    Ok(())
}
fn report(win: &Obj, level: &str, title: &str, message: String) -> PyResult<()> {
    let py = win.py();
    py.import("ingetrazo_render.chrome_dialogs")?
        .call_method1(level, (win, tr(py, title)?, message))?;
    Ok(())
}
fn same(win: &Obj, vp: &Obj, scene: &Obj) -> PyResult<bool> {
    Ok(win.getattr("viewport")?.is(vp) && vp.getattr("scene")?.is(scene))
}
fn progress<'p>(win: &Obj<'p>, path: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>)> {
    let title = trf(
        win.py(),
        "Importing {name}…",
        &[("name", path.getattr("name")?)],
    )?;
    win.call_method1("_import_progress", (title,))?.extract()
}
fn failed(win: &Obj, bar: &Obj, title: &str, err: &Obj) -> PyResult<()> {
    win.py()
        .import("ingetrazo_render.chrome_dialogs")?
        .call_method1("close_failed", (bar, err))?;
    report(win, "critical", title, err.str()?.to_string())
}
pub(crate) fn alignment(win: &Obj) -> PyResult<()> {
    let py = win.py();
    let Some(path) = choose(
        win,
        "Import georef",
        "Georef (*.kml *.kmz *.geojson *.json);;All files (*)",
    )?
    else {
        return Ok(());
    };
    let count = py
        .import("ingetrazo_core")?
        .call_method1("open_alignment", (&path,))
        .and_then(|features| place_alignment(win, &features));
    match count {
        Ok(0) => status(
            win,
            tr(py, "No lines or polygons found in the file.")?,
            4000,
        ),
        Ok(n) => status(
            win,
            trf(
                py,
                "Imported {n} feature(s) from {name}",
                &[
                    ("n", n.into_bound_py_any(py)?),
                    ("name", path.getattr("name")?),
                ],
            )?,
            4000,
        ),
        Err(e) => report(
            win,
            "critical",
            "Import failed",
            e.value(py).str()?.to_string(),
        ),
    }
}
pub(crate) fn points(win: &Obj) -> PyResult<()> {
    let py = win.py();
    let title = "Import survey points CSV";
    let Some(path) = choose(win, title, "CSV (*.csv *.txt);;All files (*)")? else {
        return Ok(());
    };
    let read = || -> PyResult<Obj> {
        let data = std::fs::read(path.extract::<std::path::PathBuf>()?)?;
        py.import("georef.points")?
            .call_method1("parse_points_csv", (String::from_utf8_lossy(&data),))
    };
    let rows = match read() {
        Ok(rows) => rows.unbind(),
        Err(_) => {
            return status(
                win,
                tr(
                    py,
                    "Could not read survey points (expected P,N,E,Z,desc in UTM)",
                )?,
                5000,
            )
        }
    };
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let choose_zone = scene.getattr("georef")?.is_none();
    let owner = win.clone().unbind();
    let vp = vp.unbind();
    let scene = scene.unbind();
    let place = func(py, move |py, args| {
        let win = owner.bind(py);
        let vp = vp.bind(py);
        if same(win, vp, scene.bind(py))? {
            let opts = args.get_item(0)?.cast_into::<PyDict>()?;
            let zone = opts
                .get_item("zone")?
                .map(|v| v.extract())
                .transpose()?
                .unwrap_or(18);
            let northern = opts
                .get_item("northern")?
                .map(|v| v.extract())
                .transpose()?
                .unwrap_or(false);
            match place_survey_points(vp, rows.bind(py), zone, northern) {
                Ok(n) => status(
                    win,
                    trf(
                        py,
                        "{n} survey points imported",
                        &[("n", n.into_bound_py_any(py)?)],
                    )?,
                    4000,
                )?,
                Err(e) => report(
                    win,
                    "critical",
                    "Import failed",
                    e.value(py).str()?.to_string(),
                )?,
            }
        }
        Ok(py.None())
    })?;
    if choose_zone {
        let forms = py.import("ingetrazo_render.chrome_forms")?;
        let model = py
            .import("ingetrazo_render")?
            .getattr("OperationForm")?
            .call_method0("survey")?;
        let form = forms.call_method1("OperationForm", (tr(py, title)?, model, place))?;
        forms.call_method1("show", (win, form))?;
    } else {
        place.call1(py, (PyDict::new(py),))?;
    }
    Ok(())
}
pub(crate) fn photomesh(win: &Obj) -> PyResult<()> {
    let py = win.py();
    let title = "Import photogrammetric mesh";
    let Some(path) = choose(win, title, "ODM textured model (*.obj);;All files (*)")? else {
        return Ok(());
    };
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let Some(datum) = survey_mesh_datum(py, path.extract()?, &scene.getattr("georef")?)? else {
        return report(
            win,
            "warning",
            title,
            tr(
                py,
                "Set a location in Settings or choose an ODM export with a georeferencing sidecar.",
            )?,
        );
    };
    let (bar, progress) = progress(win, &path)?;
    let loaded = win.call_method1("_load_photomesh_threaded", (&path, &datum, progress));
    if bar.call_method0("wasCanceled")?.is_truthy()? || !same(win, &vp, &scene)? {
        bar.call_method0("close")?;
        return Ok(());
    }
    let (mesh, images, error): (Obj, Obj, Obj) = match loaded.and_then(|v| v.extract()) {
        Ok(result) => result,
        Err(e) => return failed(win, &bar, "Import failed", e.value(py)),
    };
    if !error.is_none() {
        return failed(win, &bar, "Import failed", &error);
    }
    if mesh.is_none() || !mesh.getattr("triangle_count")?.is_truthy()? {
        bar.call_method0("close")?;
        return status(win, tr(py, "No triangles found in the model.")?, 4000);
    }
    if let Err(e) = place_survey_mesh(win, &mesh, &images, &datum) {
        return failed(win, &bar, "Import failed", e.value(py));
    }
    bar.call_method0("close")?;
    let n = mesh.getattr("missing_textures")?.len()?;
    let name = path.getattr("name")?;
    let msg = if n > 0 {
        trf(
            py,
            "Imported {name} — {n} texture(s) not found",
            &[("name", name), ("n", n.into_bound_py_any(py)?)],
        )?
    } else {
        let count = py
            .import("builtins")?
            .call_method1("format", (mesh.getattr("triangle_count")?, ","))?;
        trf(
            py,
            "Imported {name} — {t} triangles",
            &[("name", name), ("t", count)],
        )?
    };
    status(win, msg, 6000)
}
pub(crate) fn orthophoto(win: &Obj) -> PyResult<()> {
    let py = win.py();
    let title = "Import orthomosaic";
    let Some(path) = choose(win, title, "GeoTIFF (*.tif *.tiff);;All files (*)")? else {
        return Ok(());
    };
    let tiff = py.import("georef.geotiff")?;
    let info = match tiff.call_method1("read_info", (&path,)) {
        Ok(v) => v,
        Err(e) => return report(win, "critical", title, e.value(py).str()?.to_string()),
    };
    let name = path.getattr("name")?;
    if !info.getattr("georeferenced")?.is_truthy()? {
        return report(win,"warning",title,trf(py,"{name} carries no georeference (no map transform or an unknown coordinate system). Import it with File ▸ Import ▸ Image and size it by hand.",&[("name",name)])?);
    }
    let reason = tiff.call_method1("unsupported_reason", (&info,))?;
    if reason.is_truthy()? {
        return report(
            win,
            "critical",
            title,
            trf(
                py,
                "{name}: {reason} is not supported by this reader.",
                &[("name", name), ("reason", reason)],
            )?,
        );
    }
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let corners: [(f64, f64); 4] = info.call_method0("corners_geodetic")?.extract()?;
    let first = scene.getattr("georef")?.is_none();
    let datum = if first {
        let (lat, lon) = corners
            .iter()
            .fold((0., 0.), |(lat, lon), (y, x)| (lat + y / 4., lon + x / 4.));
        py.import("georef.datum")?
            .call_method1("SceneDatum", (lat, lon))?
    } else {
        scene.getattr("georef")?
    };
    let (bar, cb) = progress(win, &path)?;
    let max = vp
        .call_method0("max_texture_size")
        .and_then(|v| v.extract::<i64>())
        .unwrap_or(8192)
        .clamp(1024, 8192);
    let reading = {
        let (bar, cb, win, vp, scene) = (
            bar.clone().unbind(),
            cb.clone().unbind(),
            win.clone().unbind(),
            vp.clone().unbind(),
            scene.clone().unbind(),
        );
        func(py, move |py, args| {
            cb.call1(
                py,
                (
                    args.get_item(0)?.extract::<f64>()? * 0.9,
                    "Decoding the orthomosaic…",
                ),
            )?;
            obj(
                py,
                !bar.bind(py).call_method0("wasCanceled")?.is_truthy()?
                    && same(win.bind(py), vp.bind(py), scene.bind(py))?,
            )
        })?
    };
    let cancelled = || -> PyResult<bool> {
        Ok(bar.call_method0("wasCanceled")?.is_truthy()? || !same(win, &vp, &scene)?)
    };
    let store = || -> PyResult<Option<(Obj, i64)>> {
        let kw = PyDict::new(py);
        kw.set_item("max_px", max)?;
        kw.set_item("progress", reading)?;
        let (rgba, factor): (Obj, i64) = tiff
            .getattr("read_rgba")?
            .call((&info,), Some(&kw))?
            .extract()?;
        cb.call1((0.92, "Storing the picture…"))?;
        if cancelled()? {
            return Ok(None);
        }
        let bytes = py
            .import("ingetrazo_core")?
            .call_method1("survey_webp", (rgba,))?;
        let cached = py.import("core.texture")?.call_method1(
            "cache_image",
            (
                bytes,
                format!("{}.webp", path.getattr("stem")?.str()?),
                "imported",
            ),
        )?;
        Ok(Some((cached, factor)))
    };
    let result = store();
    if cancelled()? {
        bar.call_method0("close")?;
        return Ok(());
    }
    let (cached, factor) = match result {
        Ok(Some(v)) => v,
        Ok(None) => {
            bar.call_method0("close")?;
            return Ok(());
        }
        Err(e) => return failed(win, &bar, title, e.value(py)),
    };
    bar.call_method0("close")?;
    let [bl, br, _, tl] = corners.map(|p| datum.call_method1("geodetic_to_local", p));
    let bl = bl?;
    let u = br?.sub(&bl)?;
    let v = tl?.sub(&bl)?;
    let width: f64 = u.call_method0("length")?.extract()?;
    let height: f64 = v.call_method0("length")?.extract()?;
    let kw = PyDict::new(py);
    kw.set_item("aspect", if width > 1e-9 { height / width } else { 1. })?;
    kw.set_item("name", path.getattr("stem")?)?;
    kw.set_item("locked", true)?;
    let plane = py
        .import("core.image_plane")?
        .getattr("ImagePlane")?
        .call((cached.str()?, bl, u, v), Some(&kw))?;
    let cmd = py
        .import("core.history")?
        .call_method1("AddImagePlaneCommand", (&plane,))?;
    vp.getattr("history")?.call_method1("execute", (cmd,))?;
    if first {
        scene.setattr("georef", &datum)?;
        let mut bounds = Bounds::new();
        for p in plane.call_method0("corners")?.try_iter()? {
            bounds.add(&p?)?;
        }
        let (lo, hi) = bounds.vectors(py, 0.)?;
        win.call_method1("_set_survey_capture", (datum, &lo, &hi))?;
        let camera = vp.getattr("camera")?;
        camera.call_method1("set_view", ("top",))?;
        camera.call_method1("fit_to", (lo, hi))?;
    }
    vp.call_method0("update")?;
    let reduced = if factor > 1 {
        trf(
            py,
            ", reduced {k}×",
            &[("k", factor.into_bound_py_any(py)?)],
        )?
    } else {
        String::new()
    };
    status(
        win,
        trf(
            py,
            "Imported {name} — {w:.0f} × {h:.0f} m, {info}{reduced}",
            &[
                ("name", name),
                ("w", width.into_bound_py_any(py)?),
                ("h", height.into_bound_py_any(py)?),
                ("info", tiff.call_method1("describe", (info,))?),
                ("reduced", reduced.into_bound_py_any(py)?),
            ],
        )?,
        8000,
    )
}
