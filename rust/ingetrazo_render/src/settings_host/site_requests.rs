// SPDX-License-Identifier: GPL-3.0-or-later
//! Requests belong to the Settings window; closing the map disconnects updates.
use crate::settings_rows::trf;
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyList, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
#[pyclass]
pub(super) struct SiteRequests {
    draft: Py<PyAny>,
    source: Py<PyAny>,
    refresh: Option<Py<PyAny>>,
    #[pyo3(get)]
    tiles: Py<PyAny>,
    #[pyo3(get)]
    searcher: Py<PyAny>,
    #[pyo3(get)]
    locator: Py<PyAny>,
}
const SUBSCRIPTIONS: [(&str, &str, &str); 5] = [
    ("tiles", "tileReady", "tile_ready"),
    ("searcher", "resultsReady", "places"),
    ("searcher", "failed", "error"),
    ("locator", "located", "located"),
    ("locator", "failed", "error"),
];
pub(super) fn create<'p>(
    draft: &Obj<'p>,
    source: Obj<'p>,
    refresh: Obj<'p>,
    win: Obj<'p>,
) -> PyResult<Bound<'p, SiteRequests>> {
    let py = draft.py();
    let services = py.import("ingetrazo_core.georef.geocode")?;
    let tiles = py
        .import("ingetrazo_core.georef.tile_fetcher")?
        .getattr("TileFetcher")?
        .call((), Some(&[("parent", &win)].into_py_dict(py)?))?;
    let s = Bound::new(
        py,
        SiteRequests {
            draft: draft.clone().unbind(),
            source: source.unbind(),
            refresh: Some(refresh.unbind()),
            tiles: tiles.unbind(),
            searcher: services.call_method1("Geocoder", (&win,))?.unbind(),
            locator: services.call_method1("IpLocator", (&win,))?.unbind(),
        },
    )?;
    for (service, signal, callback) in SUBSCRIPTIONS {
        s.getattr(service)?
            .getattr(signal)?
            .call_method1("connect", (s.getattr(callback)?,))?;
    }
    Ok(s)
}
#[pymethods]
impl SiteRequests {
    #[pyo3(signature=(*_args))]
    fn tile_ready(&self, py: Python, _args: &Bound<PyTuple>) -> PyResult<()> {
        if let Some(refresh) = &self.refresh {
            self.draft.bind(py).call_method0("tile_ready")?;
            refresh.call0(py)?;
        }
        Ok(())
    }
    fn request(&self, py: Python, tiles: Vec<String>) -> PyResult<()> {
        for tile in tiles {
            let xyz = tile
                .split(',')
                .map(str::parse::<i32>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
            let [x, y, z] = xyz.as_slice() else {
                return Err(pyo3::exceptions::PyValueError::new_err("Invalid map tile"));
            };
            let hit = self
                .tiles
                .bind(py)
                .call_method1("request", (self.source.bind(py), x, y, z))?;
            if !hit.is_none() {
                self.tile_ready(py, &PyTuple::empty(py))?;
            }
        }
        Ok(())
    }
    fn places(&self, items: &Obj) -> PyResult<()> {
        let py = items.py();
        if let Some(refresh) = &self.refresh {
            let rows = PyList::empty(py);
            for p in items.try_iter()? {
                let p = p?;
                rows.append((p.getattr("name")?, p.getattr("lat")?, p.getattr("lon")?))?;
            }
            self.draft.bind(py).call_method1("places", (rows,))?;
            refresh.call0(py)?;
        }
        Ok(())
    }
    fn located(&self, py: Python, lat: f64, lon: f64, label: &Obj) -> PyResult<()> {
        if let Some(refresh) = &self.refresh {
            let draft = self.draft.bind(py);
            draft.call_method1("move_pin", (lat, lon, 11))?;
            draft.setattr(
                "note",
                trf(
                    py,
                    "Near {label} (approximate — refine on the map).",
                    &[("label", label.clone())],
                )?,
            )?;
            refresh.call0(py)?;
        }
        Ok(())
    }
    fn error(&self, py: Python, message: &str) -> PyResult<()> {
        if let Some(refresh) = &self.refresh {
            self.draft.bind(py).setattr("note", message)?;
            refresh.call0(py)?;
        }
        Ok(())
    }
    fn close(slf: &Bound<Self>) -> PyResult<()> {
        if slf.borrow_mut().refresh.take().is_none() {
            return Ok(());
        }
        for (service, signal, callback) in SUBSCRIPTIONS {
            slf.getattr(service)?
                .getattr(signal)?
                .call_method1("disconnect", (slf.getattr(callback)?,))?;
        }
        slf.getattr("tiles")?.call_method0("cancel_all")?;
        for key in ["tiles", "searcher", "locator"] {
            slf.getattr(key)?.call_method0("deleteLater")?;
        }
        Ok(())
    }
}
