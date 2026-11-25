// SPDX-License-Identifier: GPL-3.0-or-later
//! Survey downloads keep callback delivery on the requesting window's event loop.
mod dem;
use super::Obj;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyList, PySet, PyTuple},
};
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.georef.transport")
}
fn partial<'p>(py: Python<'p>, args: impl pyo3::call::PyCallArgs<'p>) -> PyResult<Obj<'p>> {
    py.import("functools")?.call_method1("partial", args)
}
#[pyfunction]
fn survey_download<'p>(py: Python<'p>, url: &str, agent: &Obj<'p>) -> PyResult<Obj<'p>> {
    if !["https://", "http://"]
        .iter()
        .any(|p| url.to_ascii_lowercase().starts_with(p))
    {
        return Err(PyValueError::new_err("Survey URL must use HTTP or HTTPS"));
    }
    let m = py.import("urllib.request")?;
    let members = PyDict::new(py);
    members.set_item(
        "redirect_request",
        py.import("functools")?
            .call_method1("partialmethod", (wrap_pyfunction!(survey_redirect, py)?,))?,
    )?;
    let redirect = py.import("builtins")?.call_method1(
        "type",
        (
            "SurveyRedirect",
            (m.getattr("HTTPRedirectHandler")?,),
            members,
        ),
    )?;
    let kw = PyDict::new(py);
    kw.set_item(
        "context",
        py.import("ingetrazo_core.python_tls")?.call_method0("https_context")?,
    )?;
    let handler = m.call_method("HTTPSHandler", (), Some(&kw))?;
    let opener = m.call_method1("build_opener", (handler, redirect.call0()?))?;
    let headers = PyDict::new(py);
    headers.set_item("User-Agent", agent.call_method0("decode")?)?;
    let kw = PyDict::new(py);
    kw.set_item("headers", headers)?;
    let req = m.call_method("Request", (url,), Some(&kw))?;
    let kw = PyDict::new(py);
    kw.set_item("timeout", 15)?;
    let reply = opener.call_method("open", (req,), Some(&kw))?;
    let data = reply.call_method0("read");
    reply.call_method0("close")?;
    data
}
#[pyfunction]
fn survey_redirect<'p>(
    owner: &Obj<'p>,
    req: &Obj<'p>,
    fp: &Obj<'p>,
    code: i32,
    msg: &Obj<'p>,
    headers: &Obj<'p>,
    newurl: &str,
) -> PyResult<Obj<'p>> {
    if req.getattr("type")?.eq("https")? && !newurl.to_ascii_lowercase().starts_with("https://") {
        return Err(PyValueError::new_err(
            "Refusing to downgrade an HTTPS survey request",
        ));
    }
    owner
        .py()
        .import("urllib.request")?
        .getattr("HTTPRedirectHandler")?
        .call_method1(
            "redirect_request",
            (owner, req, fp, code, msg, headers, newurl),
        )
}
#[pyfunction]
fn survey_request<'p>(
    owner: &Obj<'p>,
    url: &str,
    agent: &Obj<'p>,
    ready: &Obj<'p>,
    error: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let m = module(owner.py())?;
    let workers = match m.getattr_opt("_workers")? {
        Some(pool) => pool,
        None => {
            let py = owner.py();
            let kw = PyDict::new(py);
            kw.set_item("thread_name_prefix", "survey")?;
            let pool = py.import("concurrent.futures")?.call_method(
                "ThreadPoolExecutor",
                (8,),
                Some(&kw),
            )?;
            m.dict().call_method1("setdefault", ("_workers", pool))?
        }
    };
    let future = workers.call_method1("submit", (m.getattr("_download")?, url, agent))?;
    owner
        .py()
        .import("ingetrazo_render")?
        .getattr("WindowJob")?
        .call_method1("submit", (owner, future, ready, error))
}
fn user_agent(py: Python<'_>) -> PyResult<Bound<'_, PyBytes>> {
    let version = py.import("ingetrazo_core.version")?.getattr("__version__")?;
    Ok(PyBytes::new(
        py,
        format!("IngeTrazo/{version} (https://github.com/ingelibre/ingetrazo)").as_bytes(),
    ))
}
fn init_service(o: &Obj<'_>, parent: Option<&Obj<'_>>) -> PyResult<()> {
    module(o.py())?
        .getattr("Service")?
        .call_method1("bind_host", (o, parent))?;
    Ok(())
}
#[pyfunction(signature=(owner,parent=None))]
fn geocoder_init(owner: &Obj<'_>, parent: Option<&Obj<'_>>) -> PyResult<()> {
    init_service(owner, parent)
}
#[pyfunction]
fn geo_get<'p>(
    owner: &Obj<'p>,
    url: &str,
    on_bytes: &Obj<'p>,
    on_error: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let py = owner.py();
    module(py)?.call_method1("request", (owner, url, user_agent(py)?, on_bytes, on_error))
}
#[pyfunction(signature=(query,limit=8))]
fn geocode_url(py: Python<'_>, query: &str, limit: i64) -> PyResult<String> {
    let q = py.import("urllib.parse")?.call_method1("quote", (query,))?;
    let base = py.import("ingetrazo_core.georef.geocode")?.getattr("NOMINATIM")?;
    Ok(format!("{base}?q={q}&format=jsonv2&limit={limit}"))
}
fn json<'p>(data: &Obj<'p>) -> Option<Obj<'p>> {
    let py = data.py();
    let b = py.get_type::<PyBytes>().call1((data,)).ok()?;
    py.import("json").ok()?.call_method1("loads", (b,)).ok()
}
fn number(d: &Obj<'_>, key: &str) -> PyResult<f64> {
    d.py()
        .get_type::<pyo3::types::PyFloat>()
        .call1((d.get_item(key)?,))?
        .extract()
}
#[pyfunction]
fn parse_places<'p>(py: Python<'p>, data: &Obj<'p>) -> PyResult<Obj<'p>> {
    let list = PyList::empty(py);
    let place = py.import("ingetrazo_core.georef.geocode")?.getattr("Place")?;
    if let Some(val) = json(data) {
        if let Ok(arr) = val.cast::<PyList>() {
            for d in arr {
                if let (Ok(lat), Ok(lon)) = (number(&d, "lat"), number(&d, "lon")) {
                    list.append(place.call1((
                        d.call_method1("get", ("display_name", ""))?,
                        lat,
                        lon,
                    ))?)?
                }
            }
        }
    }
    Ok(list.into_any())
}
#[pyfunction]
fn parse_ip(data: &Obj<'_>) -> PyResult<Option<(f64, f64, String)>> {
    let Some(d) = json(data) else { return Ok(None) };
    let (Ok(lat), Ok(lon)) = (number(&d, "latitude"), number(&d, "longitude")) else {
        return Ok(None);
    };
    let mut names = vec![];
    for k in ["city", "region", "country_name"] {
        let v = d.call_method1("get", (k,))?;
        if v.is_truthy()? {
            names.push(v.str()?.to_string())
        }
    }
    Ok(Some((lat, lon, names.join(", "))))
}
#[pyfunction]
fn geocode_received(o: &Obj<'_>, data: &Obj<'_>) -> PyResult<()> {
    o.getattr("resultsReady")?
        .call_method1("emit", (parse_places(o.py(), data)?,))?;
    Ok(())
}
#[pyfunction]
fn ip_received(o: &Obj<'_>, data: &Obj<'_>) -> PyResult<()> {
    if let Some(args) = parse_ip(data)? {
        o.getattr("located")?.call_method1("emit", args)?
    } else {
        o.getattr("failed")?
            .call_method1("emit", ("no location in response",))?
    };
    Ok(())
}
#[pyfunction]
fn geo_search(o: &Obj<'_>, query: Option<&str>) -> PyResult<()> {
    let py = o.py();
    let q = query.unwrap_or("").trim();
    if q.is_empty() {
        o.getattr("resultsReady")?
            .call_method1("emit", (PyList::empty(py),))?;
    } else {
        o.call_method1(
            "_get",
            (
                geocode_url(py, q, 8)?,
                o.getattr("_received")?,
                o.getattr("failed")?.getattr("emit")?,
            ),
        )?;
    }
    Ok(())
}
#[pyfunction]
fn geo_locate(o: &Obj<'_>) -> PyResult<()> {
    o.call_method1(
        "_get",
        (
            o.py().import("ingetrazo_core.georef.geocode")?.getattr("IPAPI")?,
            o.getattr("_received")?,
            o.getattr("failed")?.getattr("emit")?,
        ),
    )?;
    Ok(())
}
fn tile_api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.georef.tile_fetcher")
}
#[pyfunction(signature=(o,cache=None,parent=None))]
fn fetch_init(o: &Obj<'_>, cache: Option<Obj<'_>>, parent: Option<&Obj<'_>>) -> PyResult<()> {
    let py = o.py();
    init_service(o, parent)?;
    let m = tile_api(py)?;
    let cache = match cache {
        Some(v) => v,
        None => m.call_method1("TileCache", (m.call_method0("default_cache_dir")?,))?,
    };
    o.setattr("_cache", cache)?;

    o.setattr("_inflight", PyDict::new(py))?;
    o.setattr("_queue", PyList::empty(py))?;
    o.setattr("_queued", PySet::empty(py)?)
}
fn key<'p>(
    py: Python<'p>,
    source: &Obj<'p>,
    x: i64,
    y: i64,
    z: i64,
) -> PyResult<Bound<'p, PyTuple>> {
    (source.getattr("id")?, x, y, z).into_pyobject(py)
}
#[pyfunction]
fn tile_request<'p>(
    o: &Obj<'p>,
    source: &Obj<'p>,
    x: i64,
    y: i64,
    z: i64,
) -> PyResult<Option<Obj<'p>>> {
    let py = o.py();
    if z < 0 || z > source.getattr("max_zoom")?.extract::<i64>()? {
        return Ok(None);
    }
    let k = key(py, source, x, y, z)?;
    let bytes = o.getattr("_cache")?.call_method1("get", &k)?;
    if !bytes.is_none() {
        let img = py
            .import("ingetrazo_render")?
            .getattr("SurveyRaster")?
            .call_method1("decode", (bytes,))?;
        if !img.is_none() {
            return Ok(Some(img));
        }
    }
    let active = o.getattr("_inflight")?;
    let queued = o.getattr("_queued")?;
    if !active.contains(&k)? && !queued.contains(&k)? {
        if active.len()? < o.getattr("_MAX_INFLIGHT")?.extract()? {
            o.call_method1("_start_download", (source, x, y, z))?;
        } else {
            o.getattr("_queue")?
                .call_method1("append", ((source, x, y, z),))?;
            queued.call_method1("add", (k,))?;
        }
    }
    Ok(None)
}
#[pyfunction]
fn tile_pump(o: &Obj<'_>) -> PyResult<()> {
    let q = o.getattr("_queue")?;
    let a = o.getattr("_inflight")?;
    let max = o.getattr("_MAX_INFLIGHT")?.extract()?;
    while q.len()? > 0 && a.len()? < max {
        let (source, x, y, z) = q
            .call_method1("pop", (0,))?
            .extract::<(Obj, i64, i64, i64)>()?;
        o.getattr("_queued")?
            .call_method1("discard", (key(o.py(), &source, x, y, z)?,))?;
        o.call_method1("_start_download", (source, x, y, z))?;
    }
    Ok(())
}
#[pyfunction]
fn tile_failed(o: &Obj<'_>, key: &Obj<'_>, reason: &Obj<'_>) -> PyResult<()> {
    o.call_method1("_completed", (key, o.py().None(), reason))?;
    Ok(())
}
#[pyfunction]
fn tile_start(o: &Obj<'_>, source: &Obj<'_>, x: i64, y: i64, z: i64) -> PyResult<()> {
    let py = o.py();
    let k = key(py, source, x, y, z)?;
    let url = source.call_method1("url", (x, y, z))?;
    let active = o.getattr("_inflight")?;
    let ok = partial(py, (o.getattr("_completed")?, &k))?;
    let fail = partial(py, (wrap_pyfunction!(tile_failed, py)?, o, &k))?;
    let job = module(py)?.call_method1("request", (o, url, user_agent(py)?, ok, fail))?;
    active.set_item(k, job)
}
#[pyfunction(signature=(o,key,data,error=""))]
fn tile_completed(o: &Obj<'_>, key: &Obj<'_>, data: &Obj<'_>, error: &str) -> PyResult<()> {
    let py = o.py();
    let (s, x, y, z) = key.extract::<(String, i64, i64, i64)>()?;
    o.getattr("_inflight")?
        .call_method1("pop", (key, py.None()))?;
    o.call_method0("_pump")?;
    let img = if data.is_none() {
        py.None().into_bound(py)
    } else {
        py.import("ingetrazo_render")?
            .getattr("SurveyRaster")?
            .call_method1("decode", (data,))?
    };
    if img.is_none() {
        o.getattr("tileFailed")?.call_method1(
            "emit",
            (
                s,
                x,
                y,
                z,
                if error.is_empty() {
                    "undecodable tile data"
                } else {
                    error
                },
            ),
        )?;
    } else {
        o.getattr("_cache")?
            .call_method1("put", (&s, x, y, z, data))?;
        o.getattr("tileReady")?
            .call_method1("emit", (s, x, y, z, img))?;
    }
    Ok(())
}
#[pyfunction]
fn tile_cancel(o: &Obj<'_>) -> PyResult<()> {
    let a = o.getattr("_inflight")?;
    let jobs = o
        .py()
        .get_type::<PyList>()
        .call1((a.call_method0("values")?,))?;
    o.getattr("_queue")?.call_method0("clear")?;
    o.getattr("_queued")?.call_method0("clear")?;
    a.call_method0("clear")?;
    for job in jobs.try_iter()? {
        job?.call_method0("abort")?;
    }
    Ok(())
}
/// Service notifications are owned by the object and execute on the window timer queue.
#[pyclass(subclass, dict, weakref)]
struct SurveyService;
#[pymethods]
impl SurveyService {
    #[new]
    #[pyo3(signature=(*_args,**_kwargs))]
    fn new(_args: &Bound<'_, pyo3::types::PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    #[pyo3(signature=(parent=None))]
    fn bind_host(slf: &Bound<'_, Self>, parent: Option<&Obj<'_>>) -> PyResult<()> {
        let py = slf.py();
        let inherited = parent
            .map(|p| p.getattr_opt("_host"))
            .transpose()?
            .flatten()
            .filter(|p| !p.is_none());
        let host = match inherited {
            Some(h) => h,
            None => {
                let active = py.import("ingetrazo_render.shell_modal")?.call_method0("host")?;
                if active.is_none() {
                    let kw = PyDict::new(py);
                    kw.set_item(
                        "timers",
                        py.import("ingetrazo_render")?
                            .call_method0("WindowTimers")?,
                    )?;
                    py.import("types")?
                        .call_method("SimpleNamespace", (), Some(&kw))?
                } else {
                    active
                }
            }
        };
        slf.setattr("_host", host)
    }
    #[pyo3(name = "deleteLater")]
    fn dispose(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("_host")?
            .getattr("timers")?
            .call_method1("cancel_context", (slf,))?;
        Ok(())
    }
}
#[pyclass]
struct SurveySignal {
    name: String,
}
#[pymethods]
impl SurveySignal {
    #[new]
    #[pyo3(signature=(*_types))]
    fn new(_types: &Bound<'_, pyo3::types::PyTuple>) -> Self {
        Self {
            name: String::new(),
        }
    }
    fn __set_name__(&mut self, _owner: &Obj<'_>, name: String) {
        self.name = name;
    }
    fn __get__<'p>(
        slf: &Bound<'p, Self>,
        owner: Option<&Obj<'p>>,
        _cls: Option<&Obj<'p>>,
    ) -> PyResult<Obj<'p>> {
        let Some(owner) = owner else {
            return Ok(slf.clone().into_any());
        };
        let signal = slf
            .py()
            .import("ingetrazo_render")?
            .call_method0("ChangeCallbacks")?;
        owner
            .getattr("__dict__")?
            .set_item(&slf.borrow().name, &signal)?;
        Ok(signal)
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SurveyService>()?;
    m.add_class::<SurveySignal>()?;
    functions!(m;survey_download,survey_redirect,survey_request,geocoder_init,geo_get,geocode_url,parse_places,parse_ip,geocode_received,ip_received,geo_search,geo_locate,fetch_init,tile_request,tile_pump,tile_failed,tile_start,tile_completed,tile_cancel);
    dem::register(m)
}
