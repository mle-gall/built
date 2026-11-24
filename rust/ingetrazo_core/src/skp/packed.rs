// SPDX-License-Identifier: GPL-3.0-or-later
//! Read-only coordinate spans, ordered soft flags and per-adaptation geometry ownership.
use pyo3::{
    exceptions::{PyAttributeError, PyIndexError},
    prelude::*,
    types::{PyDict, PyList, PySlice, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
fn list<'p>(o: &Obj<'p>) -> PyResult<Obj<'p>> {
    o.py().get_type::<PyList>().call1((o,))
}
#[pyfunction]
fn skp_packed_repr<'p>(owner: &Obj<'p>) -> PyResult<String> {
    Ok(list(owner)?.repr()?.to_string())
}
#[pyfunction]
fn skp_packed_eq<'p>(owner: &Obj<'p>, other: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = owner.py();
    if !other.is_instance_of::<PyList>() && !other.is_instance(&owner.get_type())? {
        return Ok(py.NotImplemented().into_bound(py));
    }
    Ok(list(owner)?
        .eq(list(other)?)?
        .into_pyobject(py)?
        .to_owned()
        .into_any())
}
#[pyfunction]
fn skp_rows_get<'p>(owner: &Obj<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = owner.py();
    let indices = owner.getattr("_indices")?.get_item(key)?;
    let array = owner.getattr("_array")?;
    if key.is_instance_of::<PySlice>() {
        let result = py
            .import("builtins")?
            .getattr("object")?
            .call_method1("__new__", (owner.get_type(),))?;
        result.setattr("_array", array)?;
        result.setattr("_indices", indices)?;
        Ok(result)
    } else {
        array.get_item(indices)?.call_method0("tolist")
    }
}
#[pyfunction]
fn skp_rows_iter<'p>(owner: &Obj<'p>) -> PyResult<Obj<'p>> {
    let b = owner.py().import("builtins")?;
    b.call_method1(
        "map",
        (
            owner.getattr("__getitem__")?,
            b.call_method1("range", (owner.len()?,))?,
        ),
    )
}
fn pair<'p>(value: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = value.py();
    let row = |i| {
        py.get_type::<PyTuple>()
            .call1((value.get_item(i)?.call_method0("tolist")?,))
    };
    Ok((row(0)?, row(1)?).into_pyobject(py)?.into_any())
}
#[pyfunction]
fn skp_segment_pair<'p>(value: &Obj<'p>) -> PyResult<Obj<'p>> {
    pair(value)
}
#[pyfunction]
fn skp_segments_iter<'p>(owner: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = owner.py();
    let raw = py
        .import("itertools")?
        .getattr("chain")?
        .call_method1("from_iterable", (owner.getattr("chunks")?,))?;
    py.import("builtins")?
        .call_method1("map", (wrap_pyfunction!(skp_segment_pair, py)?, raw))
}
#[pyfunction]
fn skp_segments_get<'p>(owner: &Obj<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = owner.py();
    let size = owner.len()?;
    if key.is_instance_of::<PySlice>() {
        let range = py
            .import("builtins")?
            .call_method1("range", (size,))?
            .get_item(key)?;
        let out = PyList::empty(py);
        for i in range.try_iter()? {
            out.append(owner.get_item(i?)?)?;
        }
        return Ok(out.into_any());
    }
    let mut i = key.extract::<isize>()?;
    if i < 0 {
        i += size as isize;
    }
    if i >= 0 {
        for chunk in owner.getattr("chunks")?.try_iter()? {
            let chunk = chunk?;
            let n = chunk.len()? as isize;
            if i < n {
                return pair(&chunk.get_item(i)?);
            }
            i -= n;
        }
    }
    Err(PyIndexError::new_err(key.str()?.to_string()))
}
struct Span<'p> {
    array: Obj<'p>,
    start: isize,
    step: isize,
    n: usize,
}
#[pyfunction]
fn skp_flatten_rings<'p>(py: Python<'p>, rings: &Obj<'p>, size: usize) -> PyResult<Obj<'p>> {
    let rings = rings.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let np = py.import("numpy")?;
    let kind = py.import("formats.skp_coordinates")?.getattr("PointRows")?;
    let f32 = np.getattr("float32")?;
    let mut narrow = true;
    for ring in &rings {
        if !ring.is_instance(&kind)? || !ring.getattr("_array")?.getattr("dtype")?.eq(&f32)? {
            narrow = false;
            break;
        }
    }
    let out = np.call_method1(
        "empty",
        ((size, 3), if narrow { f32 } else { np.getattr("float64")? }),
    )?;
    let slice = py.get_type::<PySlice>();
    let mut offset = 0usize;
    let mut copy = |src: Option<Span<'p>>, ring: Option<&Obj<'p>>| -> PyResult<()> {
        let (data, n) = if let Some(s) = src {
            let stop = s.start + s.step * s.n as isize;
            let sub = slice.call1((s.start, (s.step > 0 || stop >= 0).then_some(stop), s.step))?;
            (s.array.get_item(sub)?, s.n)
        } else if let Some(r) = ring {
            let points = if r.get_item(0)?.hasattr("x")? {
                let l = PyList::empty(py);
                for p in r.try_iter()? {
                    l.append(p?.call_method0("toTuple")?)?;
                }
                l.into_any()
            } else {
                r.clone()
            };
            (points, r.len()?)
        } else {
            return Ok(());
        };
        out.set_item(slice.call1((offset, offset + n))?, data)?;
        offset += n;
        Ok(())
    };
    let mut pending: Option<Span> = None;
    for r in rings {
        if r.len()? == 0 {
            continue;
        }
        if let Some(provider) = r.getattr_opt("_coordinate_span")? {
            let (arr, range) = provider.call0()?.extract::<(Obj, Obj)>()?;
            let next = Span {
                array: arr,
                start: range.getattr("start")?.extract()?,
                step: range.getattr("step")?.extract()?,
                n: range.len()?,
            };
            if let Some(s) = pending.as_mut() {
                if s.array.is(&next.array)
                    && s.step == next.step
                    && s.start + s.step * s.n as isize == next.start
                {
                    s.n += next.n;
                    continue;
                }
            }
            copy(pending.replace(next), None)?;
        } else {
            copy(pending.take(), None)?;
            copy(None, Some(&r))?;
        }
    }
    copy(pending, None)?;
    Ok(out)
}
#[pyclass(dict)]
struct GeometryCache {
    #[pyo3(get)]
    limit: usize,
    #[pyo3(get)]
    size: usize,
    #[pyo3(get)]
    entries: Py<PyAny>,
}
#[pymethods]
impl GeometryCache {
    #[new]
    fn new(py: Python<'_>, limit: usize) -> PyResult<Self> {
        Ok(Self {
            limit,
            size: 0,
            entries: py
                .import("collections")?
                .call_method0("OrderedDict")?
                .unbind(),
        })
    }
    fn forget(&mut self, definition: &Obj<'_>, attribute: &str) -> PyResult<()> {
        let py = definition.py();
        let old = self.entries.bind(py).call_method1(
            "pop",
            ((definition.as_ptr() as usize, attribute), py.None()),
        )?;
        if !old.is_none() {
            self.size -= old.get_item(3)?.extract::<usize>()?;
        }
        Ok(())
    }
    fn remember(
        &mut self,
        definition: &Obj<'_>,
        attribute: &str,
        value: &Obj<'_>,
        size: usize,
    ) -> PyResult<()> {
        self.forget(definition, attribute)?;
        let py = definition.py();
        self.entries.bind(py).set_item(
            (definition.as_ptr() as usize, attribute),
            (definition, attribute, value, size),
        )?;
        self.size += size;
        while self.size > self.limit {
            self.evict(py)?;
        }
        Ok(())
    }
    fn clear(&mut self, py: Python<'_>) -> PyResult<()> {
        while self.entries.bind(py).len()? > 0 {
            self.evict(py)?;
        }
        Ok(())
    }
}
impl GeometryCache {
    fn evict(&mut self, py: Python<'_>) -> PyResult<()> {
        let item = self
            .entries
            .bind(py)
            .call_method1("popitem", (false,))?
            .get_item(1)?;
        let (obj, attr, v, n) = item.extract::<(Obj, String, Obj, usize)>()?;
        self.size -= n;
        if obj.getattr_opt(attr.as_str())?.is_some_and(|a| a.is(&v)) {
            obj.delattr(attr.as_str())?;
        }
        Ok(())
    }
}
#[pyfunction]
fn skp_remember(
    definition: &Obj<'_>,
    attribute: &str,
    value: &Obj<'_>,
    size: usize,
) -> PyResult<()> {
    let py = definition.py();
    if let Err(e) = definition.setattr(attribute, value) {
        return if e.is_instance_of::<PyAttributeError>(py) {
            Ok(())
        } else {
            Err(e)
        };
    }
    let cache = py
        .import("formats.skp_consumption")?
        .getattr("_geometry_cache")?
        .call_method0("get")?;
    if !cache.is_none() {
        cache.call_method1("remember", (definition, attribute, value, size))?;
    }
    Ok(())
}
fn paused<T>(py: Python<'_>, work: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    let gc = py.import("gc")?;
    let enabled = gc.call_method0("isenabled")?.is_truthy()?;
    gc.call_method0("disable")?;
    let result = work();
    if enabled {
        gc.call_method0("enable")?;
    }
    result
}
#[pyfunction(signature=(model,name,skp_path=None,*,consume=false))]
fn skp_adapt<'p>(
    py: Python<'p>,
    model: &Obj<'p>,
    name: &Obj<'p>,
    skp_path: Option<&Obj<'p>>,
    consume: bool,
) -> PyResult<Obj<'p>> {
    let api = py.import("formats.skp_reader")?;
    let cache = api.call_method1("_GeometryCache", (api.getattr("_GEOMETRY_CACHE_BYTES")?,))?;
    let context = api.getattr("_geometry_cache")?;
    let token = context.call_method1("set", (&cache,))?;
    let kw = PyDict::new(py);
    kw.set_item("consume", consume)?;
    let result = paused(py, || {
        api.call_method("_adapt_inner", (model, name, skp_path), Some(&kw))
    });
    let cleanup = cache.call_method0("clear");
    context.call_method1("reset", (token,))?;
    cleanup?;
    result
}
#[pyfunction]
fn skp_parse_raw<'p>(
    py: Python<'p>,
    openskp: &Obj<'p>,
    openskp_compat: &Obj<'p>,
    path: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    paused(py, || {
        let model = openskp
            .getattr("SkpFile")?
            .call_method1("open", (path.str()?,))?
            .call_method0("parse")?;
        openskp_compat.call_method1("resolve_face_layers", (&model,))?;
        Ok(model)
    })
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,m)?)?;)*}}
    m.add_class::<GeometryCache>()?;
    add!(
        skp_remember,
        skp_packed_repr,
        skp_packed_eq,
        skp_rows_get,
        skp_rows_iter,
        skp_segments_iter,
        skp_segments_get,
        skp_flatten_rings,
        skp_adapt,
        skp_parse_raw
    );
    Ok(())
}
