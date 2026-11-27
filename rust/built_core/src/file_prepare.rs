// SPDX-License-Identifier: GPL-3.0-or-later
//! Serialize the loose mesh, placements and shared prototypes. Recovery
//! supplies a cooperative mesh capture; explicit saves can spool each block.
use crate::file_snapshot::{drive, iterator, next, pause, steps, Steps};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::{collections::HashMap, future::Future, pin::Pin};
type Owned = Py<PyAny>;
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_core.formats.igz")
}
fn attr<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    match o.getattr(k) {
        Ok(v) => Ok(v),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(o.py()) => {
            Ok(o.py().None().into_bound(o.py()))
        }
        Err(e) => Err(e),
    }
}
fn mesh<'p>(s: &Obj<'p>) -> PyResult<Obj<'p>> {
    if s.hasattr("mesh")? {
        s.getattr("mesh")
    } else {
        Ok(s.clone())
    }
}
/// A geometry block names its definition (docs/persistent-ids.md §5.2).
pub(crate) fn name_definition(block: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<()> {
    if let Ok(d) = block.cast::<PyDict>() {
        if let Some(g) = d.get_item("geom")? {
            if let Ok(g) = g.cast::<PyDict>() {
                g.set_item("uid", crate::element_ids::definition_uid(mesh)?)?;
            }
        }
    }
    Ok(())
}
#[pyclass]
struct FileBlock {
    geom: Owned,
    stream: Option<Owned>,
    placements: Option<Owned>,
}
#[pymethods]
impl FileBlock {
    fn __call__<'p>(&self, m: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = m.py();
        let codec = api(py)?;
        if let (Some(p), Some(s)) = (&self.placements, &self.stream) {
            s.bind(py).setattr(
                "current_placements",
                p.bind(py)
                    .call_method1("get", (m.as_ptr() as usize, PyTuple::empty(py)))?,
            )?
        }
        let block = codec.call_method1("_mesh_json", (m, self.geom.bind(py)))?;
        let built = match &self.stream {
            Some(s) => s.bind(py).getattr_opt("built")?.map(|b| b.is_truthy()).transpose()?.unwrap_or(false),
            None => false,
        };
        if built {
            name_definition(&block, m)?;
        }
        if let Some(s) = &self.stream {
            let spool = codec.getattr("_spool_json")?;
            for (k, v) in block.cast::<PyDict>()?.iter() {
                block.set_item(k, spool.call1((v, s.bind(py)))?)?
            }
        }
        Ok(block)
    }
}
struct Prepare {
    scene: Owned,
    capture: Option<Owned>,
    stream: Option<Owned>,
    block: Py<FileBlock>,
    precise: bool,
    compat: bool,
    native: bool,
    protos: Py<PyList>,
    indices: HashMap<usize, usize>,
    writer: Option<Owned>,
}
impl Prepare {
    async fn block(&self, m: Owned) -> PyResult<Owned> {
        if let Some(c) = &self.capture {
            let it = Python::attach(|py| c.call1(py, (m,)))?;
            drive(it).await
        } else {
            Python::attach(|py| self.block.call1(py, (m,)))
        }
    }
    fn entry<'a>(
        &'a mut self,
        g: Owned,
    ) -> Pin<Box<dyn Future<Output = PyResult<Owned>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(writer) = &self.writer {
                let row = Python::attach(|py| writer.call_method1(py, "entry", (g,)))?;
                pause().await;
                return Ok(row);
            }
            let (direct, matrix, placeholder, m) = Python::attach(|py| -> PyResult<_> {
                let g = g.bind(py);
                let direct = self.native && attr(g, "_doc_key")?.is_truthy()?;
                let xf = if direct {
                    g.call_method1("_file_xform", (self.precise,))?
                } else {
                    attr(g, "xform")?
                };
                let placeholder = !xf.is_none()
                    && crate::model::links::save_as_placeholder(g, self.scene.bind(py))?;
                let m = if placeholder {
                    api(py)?.call_method0("_empty_mesh")?
                } else {
                    g.getattr("mesh")?
                };
                Ok((
                    direct,
                    (!xf.is_none()).then(|| xf.unbind()),
                    placeholder,
                    m.unbind(),
                ))
            })?;
            let entry = if let Some(xf) = matrix {
                let id = m.as_ptr() as usize;
                let i = if let Some(i) = self.indices.get(&id) {
                    *i
                } else {
                    let i = Python::attach(|py| -> PyResult<_> {
                        let p = self.protos.bind(py);
                        let i = p.len();
                        p.append(py.None())?;
                        Ok(i)
                    })?;
                    self.indices.insert(id, i);
                    let saved = self.block(m).await?;
                    Python::attach(|py| self.protos.bind(py).set_item(i, saved))?;
                    i
                };
                Python::attach(|py| -> PyResult<_> {
                    let built = py.import("built_core.formats.built")?;
                    let xf = xf.bind(py);
                    let values = if direct {
                        xf.clone()
                    } else if self.precise {
                        built.call_method1("placement_values", (xf,))?
                    } else {
                        built.call_method1("compact_values", (xf.call_method0("data")?,))?
                    };
                    let row = PyDict::new(py);
                    row.set_item("proto", i)?;
                    row.set_item("xform", values)?;
                    let component = g
                        .bind(py)
                        .getattr("component")
                        .map(|v| v.is_truthy())
                        .unwrap_or(Ok(true))?;
                    row.set_item("component", component)?;
                    Ok(row.into_any().unbind())
                })?
            } else {
                self.block(m).await?
            };
            let children = Python::attach(|py| -> PyResult<_> {
                let row = entry.bind(py);
                let g = g.bind(py);
                let codec = api(py)?;
                if direct {
                    let ifc = codec.call_method1("_group_ifc_record", (g,))?;
                    row.call_method1(
                        "update",
                        (g.call_method1("_file_fields", (ifc.is_truthy()?.then_some(ifc),))?,),
                    )?;
                } else {
                    legacy_fields(g, row)?;
                }
                if self.compat {
                    if crate::model::links::link_of(g)?.is_some() {
                        let link = row.get_item("ext")?.get_item(crate::model::links::LINK_KEY)?;
                        link.call_method1("pop", ("rel", py.None()))?;
                        link.call_method1(
                            "update",
                            (crate::model::link_io::link_record(
                                &link.get_item("path")?,
                                Some(&self.stream.as_ref().unwrap().bind(py).getattr("destination")?),
                            )?,),
                        )?;
                    }
                }
                if placeholder {
                    return Ok(None);
                }
                let kids = attr(g, "children")?;
                if !kids.is_truthy()? {
                    return Ok(None);
                }
                let out = PyList::empty(py);
                row.set_item("children", &out)?;
                Ok(Some((iterator(&kids)?, out.unbind())))
            })?;
            if let Some((kids, out)) = children {
                while let Some(k) = next(&kids)? {
                    let child = self.entry(k).await?;
                    Python::attach(|py| out.bind(py).append(child))?;
                }
            }
            pause().await;
            Ok(entry)
        })
    }
}
fn legacy_fields(g: &Obj<'_>, row: &Obj<'_>) -> PyResult<()> {
    let py = g.py();
    let builtins = py.import("builtins")?;
    let name = attr(g, "name")?;
    if name.is_truthy()? {
        row.set_item("name", name)?
    }
    let axes = attr(g, "axes")?;
    if !axes.is_none() {
        row.set_item(
            "axes",
            builtins.call_method1("list", (axes.call_method0("data")?,))?,
        )?
    }
    let layer = attr(g, "layer")?;
    if !layer.is_none() {
        row.set_item("layer", layer)?
    }
    let ifc = api(py)?.call_method1("_group_ifc_record", (g,))?;
    if ifc.is_truthy()? {
        row.set_item("ifc", ifc)?
    }
    for k in [
        "billboard",
        "material",
        "text3d",
        "ext",
        "uid",
        "hidden",
        "locked",
        "exploded",
        "explode_offset",
    ] {
        let v = attr(g, k)?;
        if !v.is_truthy()? {
            continue;
        }
        let value = match k {
            "material" | "text3d" | "exploded" => builtins.call_method1("dict", (v,))?,
            "ext" => {
                let json = py.import("json")?;
                let kw = PyDict::new(py);
                kw.set_item("default", builtins.getattr("str")?)?;
                json.call_method1("loads", (json.call_method("dumps", (v,), Some(&kw))?,))?
            }
            "hidden" | "locked" => true.into_pyobject(py)?.to_owned().into_any(),
            "explode_offset" => builtins.call_method1("list", (v,))?,
            _ => v,
        };
        row.set_item(k, value)?;
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(scene,mesh_steps=None,*,stream=None,large=None,progress=None))]
fn igz_prepare_scene_steps(
    scene: Owned,
    mesh_steps: Option<Owned>,
    stream: Option<Owned>,
    large: Option<bool>,
    progress: Option<Owned>,
) -> Steps {
    steps(async move {
        let (mut state, m, large, geom) = Python::attach(|py| -> PyResult<_> {
            let s = scene.bind(py);
            let codec = api(py)?;
            let large = match large {
                Some(v) => v,
                None => {
                    mesh_steps.is_some()
                        || codec.call_method1("_large_document", (s,))?.is_truthy()?
                }
            };
            let st = stream.as_ref().map(|v| v.bind(py));
            let geom = if let Some(st) = st {
                st.clone()
            } else if large {
                PyDict::new(py).into_any()
            } else {
                py.None().into_bound(py)
            };
            let compat = st
                .map(|v| attr(v, "compatibility")?.is_truthy())
                .transpose()?
                .unwrap_or(false);
            let precise = mesh_steps.is_some()
                || st
                    .map(|v| attr(v, "built")?.is_truthy())
                    .transpose()?
                    .unwrap_or(false);
            let placements = if compat {
                Some(
                    py.import("built_core.formats.built")?
                        .call_method1("export_placements", (s,))?
                        .unbind(),
                )
            } else {
                None
            };
            let block = Py::new(
                py,
                FileBlock {
                    geom: geom.clone().unbind(),
                    stream: st.map(|s| s.clone().unbind()),
                    placements,
                },
            )?;
            let io = codec.call_method0("native_io")?;
            let state = Prepare {
                scene: scene.clone_ref(py),
                stream,
                capture: mesh_steps,
                block,
                precise,
                compat,
                native: !io.is_none(),
                protos: PyList::empty(py).unbind(),
                indices: HashMap::new(),
                writer: None,
            };
            let root = mesh(s)?;
            crate::element_ids::set_definition_uid(&root, crate::element_ids::ROOT_DEFINITION)?;
            Ok((state, root.unbind(), large, geom.unbind()))
        })?;
        let payload = state.block(m).await?;
        let groups = Python::attach(|py| -> PyResult<_> {
            let scene = scene.bind(py);
            let groups = attr(scene, "groups")?;
            let kept = attr(scene, "kept_components")?;
            if !groups.is_truthy()? && !kept.is_truthy()? {
                return Ok(None);
            }
            let io = api(py)?.call_method0("native_io")?;
            let writer = attr(&io, "FileGroupWriter")?;
            if writer.is_truthy()? && state.capture.is_none() && !attr(scene, "_root")?.is_none() {
                let dest = if state.compat {
                    state
                        .stream
                        .as_ref()
                        .unwrap()
                        .bind(py)
                        .getattr("destination")?
                } else {
                    py.None().into_bound(py)
                };
                let w = writer.call1((scene, &state.block, state.precise, dest))?;
                state.protos = w.getattr("protos")?.cast_into::<PyList>()?.unbind();
                state.writer = Some(w.unbind());
            }
            Ok(Some((groups.unbind(), kept.unbind())))
        })?;
        if let Some((groups, kept)) = groups {
            for (key, list) in [("groups", groups), ("kept_components", kept)] {
                let data = Python::attach(|py| -> PyResult<_> {
                    let l = list.bind(py);
                    let has = l.is_truthy()?;
                    if !has && key == "kept_components" {
                        return Ok(None);
                    }
                    let out = PyList::empty(py);
                    payload.bind(py).set_item(key, &out)?;
                    Ok(if has {
                        Some((out.unbind(), iterator(l)?, l.len()?))
                    } else {
                        None
                    })
                })?;
                if let Some((out, it, n)) = data {
                    let mut i = 0;
                    while let Some(g) = next(&it)? {
                        let row = state.entry(g).await?;
                        Python::attach(|py| -> PyResult<()> {
                            out.bind(py).append(row)?;
                            if key == "groups" && i % 64 == 0 {
                                if let Some(p) = &progress {
                                    p.call1(
                                        py,
                                        (
                                            0.05 + 0.65 * (i + 1) as f64 / n as f64,
                                            "Preparing placements…",
                                        ),
                                    )?;
                                }
                            }
                            Ok(())
                        })?;
                        i += 1;
                    }
                }
            }
            Python::attach(|py| -> PyResult<()> {
                if !state.protos.bind(py).is_empty() {
                    payload.bind(py).set_item("protos", &state.protos)?
                }
                Ok(())
            })?;
        }
        let survey = Python::attach(|py| -> PyResult<_> {
            let scene = scene.bind(py);
            api(py)?.call_method1(
                "igz_archive_scene_fields",
                (scene, payload.bind(py), &state.stream),
            )?;
            let s = attr(scene, "photo_mesh")?;
            Ok((!s.is_none() && attr(&s, "triangle_count")?.is_truthy()?).then(|| s.unbind()))
        })?;
        let blobs = if let Some(s) = survey {
            if state.capture.is_some() {
                let it = Python::attach(|py| -> PyResult<_> {
                    Ok(py
                        .import("built_core")?
                        .call_method1("survey_steps", (s,))?
                        .unbind())
                })?;
                drive(it).await?
            } else {
                Python::attach(|py| -> PyResult<_> {
                    let (meta, blobs): (Owned, Owned) = py
                        .import("built_core.georef.photomesh")?
                        .call_method1("pack_mesh", (s,))?
                        .extract()?;
                    payload.bind(py).set_item("photo_mesh", meta)?;
                    Ok(blobs)
                })?
            }
        } else {
            Python::attach(|py| PyDict::new(py).into_any().unbind())
        };
        Python::attach(|py| -> PyResult<_> {
            let png = attr(scene.bind(py), "_home_thumbnail_png")?;
            if png.is_truthy()? {
                if blobs
                    .bind(py)
                    .is_instance_of::<crate::file_snapshot::Survey>()
                {
                    blobs.bind(py).setattr("thumbnail", png)?;
                } else {
                    blobs.bind(py).set_item("thumbnail.png", png)?
                }
            }
            Ok((payload, geom, large, blobs)
                .into_pyobject(py)?
                .into_any()
                .unbind())
        })
    })
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_prepare_scene_steps, m)?)
}
