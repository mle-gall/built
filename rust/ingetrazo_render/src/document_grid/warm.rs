// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PyDict;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

enum Job {
    Proto(Py<PyAny>, Py<PyAny>),
    Grid(Py<PyAny>, Py<PyDict>),
    Bounds(Py<PyAny>, Py<PyDict>),
}

#[pyclass(frozen)]
struct Worker {
    jobs: Mutex<VecDeque<Job>>,
    remaining: AtomicUsize,
    engine: Py<PyAny>,
    local_bounds: Py<PyAny>,
    protos: Py<PyDict>,
    grids: Py<PyList>,
    bounds: Py<PyList>,
}
impl Worker {
    fn run_job(&self, py: Python, job: Job) -> PyResult<()> {
        match job {
            Job::Proto(k, b) => self.protos.bind(py).set_item(
                k,
                host(py)?.call_method1("_inst_proto_of", (self.engine.bind(py), b))?,
            ),
            Job::Grid(c, s) => {
                host(py)?.call_method1("_split_chunk_spans", (&s,))?;
                self.grids.bind(py).append((c, s))
            }
            Job::Bounds(c, s) => {
                self.local_bounds.call1(py, (&s,))?;
                self.bounds.bind(py).append((c, s))
            }
        }
    }
}
#[pymethods]
impl Worker {
    fn __call__(&self, py: Python) -> PyResult<()> {
        loop {
            let job = self.jobs.lock().expect("pick queue").pop_front();
            let Some(job) = job else { break };
            // Failed speculative jobs are retried by the foreground pick.
            let _ = self.run_job(py, job);
        }
        if self.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
            py.import("ingetrazo_core")?.call_method0("trim_heap")?;
        }
        Ok(())
    }
}
#[pyclass(frozen)]
struct PickWarm {
    #[pyo3(get)]
    protos: Py<PyDict>,
    #[pyo3(get)]
    grids: Py<PyList>,
    #[pyo3(get)]
    bounds: Py<PyList>,
    #[pyo3(get)]
    threads: Py<PyList>,
}
#[pymethods]
impl PickWarm {
    #[new]
    #[pyo3(signature=(engine,protos,grids,bounds=None))]
    fn new(engine: &Obj, protos: &Obj, grids: &Obj, bounds: Option<&Obj>) -> PyResult<Self> {
        let py = engine.py();
        let mut jobs = VecDeque::new();
        for p in protos.try_iter()? {
            let p = p?;
            jobs.push_back(Job::Proto(p.get_item(0)?.unbind(), p.get_item(1)?.unbind()));
        }
        for (input, grid) in [(Some(grids), true), (bounds, false)] {
            let Some(input) = input else { continue };
            for c in input.try_iter()? {
                let c = c?;
                let s = PyDict::new(py);
                for k in ["v0", "e1", "e2", if grid { "tri_ent" } else { "edges" }] {
                    s.set_item(k, c.get_item(k)?)?;
                }
                if grid {
                    s.set_item("rev", c.call_method1("get", ("rev",))?)?;
                    jobs.push_back(Job::Grid(c.unbind(), s.unbind()));
                } else {
                    jobs.push_back(Job::Bounds(c.unbind(), s.unbind()));
                }
            }
        }
        let n = py
            .import("os")?
            .call_method0("cpu_count")?
            .extract::<Option<usize>>()?
            .unwrap_or(2)
            .clamp(1, 8);
        let result = Self {
            protos: PyDict::new(py).unbind(),
            grids: PyList::empty(py).unbind(),
            bounds: PyList::empty(py).unbind(),
            threads: PyList::empty(py).unbind(),
        };
        let worker = Py::new(
            py,
            Worker {
                jobs: Mutex::new(jobs),
                remaining: AtomicUsize::new(n),
                engine: engine.clone().unbind(),
                local_bounds: py
                    .import("tools.select_host")?
                    .getattr("_local_bounds")?
                    .unbind(),
                protos: result.protos.clone_ref(py),
                grids: result.grids.clone_ref(py),
                bounds: result.bounds.clone_ref(py),
            },
        )?;
        let cls = py.import("threading")?.getattr("Thread")?;
        let kw = PyDict::new(py);
        kw.set_item("target", worker)?;
        kw.set_item("daemon", true)?;
        for _ in 0..n {
            result.threads.bind(py).append(cls.call((), Some(&kw))?)?;
        }
        for t in result.threads.bind(py).iter() {
            t.call_method0("start")?;
        }
        Ok(result)
    }
    fn join(&self, py: Python) -> PyResult<()> {
        for t in self.threads.bind(py).iter() {
            t.call_method0("join")?;
        }
        Ok(())
    }
}
#[pyfunction]
fn pick_warm_take(v: &Obj) -> PyResult<()> {
    let py = v.py();
    let w = opt(v, "_pick_warm")?;
    if w.is_none() {
        return Ok(());
    }
    v.setattr("_pick_warm", py.None())?;
    w.call_method0("join")?;
    let mut c = opt(v, "_inst_proto_cache")?;
    if c.is_none() {
        c = PyDict::new(py).into_any();
        v.setattr("_inst_proto_cache", &c)?;
    }
    c.call_method1("update", (w.getattr("protos")?,))?;
    for (field, key) in [("grids", "_span_grid"), ("bounds", "_box_bounds")] {
        for item in w.getattr(field)?.try_iter()? {
            let item = item?;
            let c = item.get_item(0)?;
            let s = item.get_item(1)?;
            if c.contains(key)? {
                continue;
            }
            if field == "grids"
                && !(c.call_method1("get", ("rev",))?.eq(s.get_item("rev")?)?
                    && c.get_item("v0")?.is(&s.get_item("v0")?))
            {
                continue;
            }
            c.set_item(key, s.get_item(key)?)?;
        }
    }
    Ok(())
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<PickWarm>()?;
    m.add_function(wrap_pyfunction!(pick_warm_take, m)?)
}
