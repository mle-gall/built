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
    py.import("georef.transport")
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
    let m = module(py)?;
    let kw = PyDict::new(py);
    kw.set_item(
        "context",
        py.import("core.tls")?.call_method0("https_context")?,
    )?;
    let handler = m.call_method("HTTPSHandler", (), Some(&kw))?;
    let opener = m.call_method1("build_opener", (handler, m.call_method0("_Redirect")?))?;
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
    module(owner.py())?
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
    let future = m
        .getattr("_workers")?
        .call_method1("submit", (m.getattr("_download")?, url, agent))?;
    m.getattr("WindowJob")?
        .call_method1("submit", (owner, future, ready, error))
}
fn init_service(o: &Obj<'_>, parent: Option<&Obj<'_>>) -> PyResult<()> {
    module(o.py())?
        .getattr("Service")?
        .call_method1("__init__", (o, parent))?;
    Ok(())
}
fn init_net(o: &Obj<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    if !m.getattr("NATIVE")?.is_truthy()? {
        o.setattr("_nam", m.call_method1("QNetworkAccessManager", (o,))?)?
    }
    Ok(())
}
fn qt_get<'p>(m: &Bound<'p, PyModule>, o: &Obj<'p>, url: &Obj<'p>) -> PyResult<Obj<'p>> {
    let cls = m.getattr("QNetworkRequest")?;
    let r = cls.call1((m.call_method1("QUrl", (url,))?,))?;
    r.call_method1(
        "setRawHeader",
        (
            PyBytes::new(o.py(), b"User-Agent"),
            m.getattr("_USER_AGENT")?,
        ),
    )?;
    r.call_method1(
        "setAttribute",
        (
            cls.getattr("Attribute")?
                .getattr("RedirectPolicyAttribute")?,
            cls.getattr("RedirectPolicy")?
                .getattr("NoLessSafeRedirectPolicy")?,
        ),
    )?;
    o.getattr("_nam")?.call_method1("get", (r,))
}
#[pyfunction(signature=(owner,parent=None))]
fn geocoder_init(owner: &Obj<'_>, parent: Option<&Obj<'_>>) -> PyResult<()> {
    init_service(owner, parent)?;
    init_net(owner, &owner.py().import("georef.geocode")?)
}
#[pyfunction]
fn geo_get<'p>(
    owner: &Obj<'p>,
    url: &str,
    on_bytes: &Obj<'p>,
    on_error: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let py = owner.py();
    let m = py.import("georef.geocode")?;
    if m.getattr("NATIVE")?.is_truthy()? {
        return module(py)?.call_method1(
            "request",
            (owner, url, m.getattr("_USER_AGENT")?, on_bytes, on_error),
        );
    }
    let r = qt_get(&m, owner, &url.into_pyobject(py)?.into_any())?;
    let cb = partial(py, (m.getattr("_finish_reply")?, &r, on_bytes, on_error))?;
    r.getattr("finished")?.call_method1("connect", (cb,))?;
    Ok(py.None().into_bound(py))
}
#[pyfunction]
fn geo_reply(reply: &Obj<'_>, on_bytes: &Obj<'_>, on_error: &Obj<'_>) -> PyResult<()> {
    let py = reply.py();
    let result = (|| {
        let no = py
            .import("georef.geocode")?
            .getattr("QNetworkReply")?
            .getattr("NetworkError")?
            .getattr("NoError")?;
        if reply.call_method0("error")?.eq(no)? {
            on_bytes.call1((py
                .get_type::<PyBytes>()
                .call1((reply.call_method0("readAll")?,))?,))?
        } else {
            on_error.call1((reply.call_method0("errorString")?,))?
        };
        Ok(())
    })();
    reply.call_method0("deleteLater")?;
    result
}
#[pyfunction(signature=(query,limit=8))]
fn geocode_url(py: Python<'_>, query: &str, limit: i64) -> PyResult<String> {
    let q = py.import("urllib.parse")?.call_method1("quote", (query,))?;
    let base = py.import("georef.geocode")?.getattr("NOMINATIM")?;
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
    let place = py.import("georef.geocode")?.getattr("Place")?;
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
            o.py().import("georef.geocode")?.getattr("IPAPI")?,
            o.getattr("_received")?,
            o.getattr("failed")?.getattr("emit")?,
        ),
    )?;
    Ok(())
}
fn tile_api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("georef.tile_fetcher")
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
    init_net(o, &m)?;
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
        let img = tile_api(py)?
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
    let m = tile_api(py)?;
    let k = key(py, source, x, y, z)?;
    let url = source.call_method1("url", (x, y, z))?;
    let active = o.getattr("_inflight")?;
    if m.getattr("NATIVE")?.is_truthy()? {
        let ok = partial(py, (o.getattr("_completed")?, &k))?;
        let fail = partial(py, (m.getattr("_tile_failed")?, o, &k))?;
        let job =
            module(py)?.call_method1("request", (o, url, m.getattr("_USER_AGENT")?, ok, fail))?;
        active.set_item(k, job)?
    } else {
        let reply = qt_get(&m, o, &url)?;
        active.set_item(k, &reply)?;
        let cb = partial(
            py,
            (
                o.getattr("_on_finished")?,
                source.getattr("id")?,
                x,
                y,
                z,
                &reply,
            ),
        )?;
        reply.getattr("finished")?.call_method1("connect", (cb,))?;
    }
    Ok(())
}
#[pyfunction]
fn tile_finished(
    o: &Obj<'_>,
    source_id: &str,
    x: i64,
    y: i64,
    z: i64,
    reply: &Obj<'_>,
) -> PyResult<()> {
    let py = o.py();
    let result = (|| {
        let k = (source_id, x, y, z);
        let no = tile_api(py)?
            .getattr("QNetworkReply")?
            .getattr("NetworkError")?
            .getattr("NoError")?;
        if reply.call_method0("error")?.eq(no)? {
            let data = py
                .get_type::<PyBytes>()
                .call1((reply.call_method0("readAll")?,))?;
            o.call_method1("_completed", (k, data))?
        } else {
            o.call_method1(
                "_completed",
                (k, py.None(), reply.call_method0("errorString")?),
            )?
        };
        Ok(())
    })();
    reply.call_method0("deleteLater")?;
    result
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
        tile_api(py)?
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
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    functions!(m;survey_download,survey_redirect,survey_request,geocoder_init,geo_get,geo_reply,geocode_url,parse_places,parse_ip,geocode_received,ip_received,geo_search,geo_locate,fetch_init,tile_request,tile_pump,tile_failed,tile_start,tile_finished,tile_completed,tile_cancel);
    dem::register(m)
}
