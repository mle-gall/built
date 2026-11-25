// SPDX-License-Identifier: GPL-3.0-or-later
//! Stage every output of a delivery before publishing the primary file and sidecars.
use super::{delivery_scoped, delivery_validate, Obj};
use crate::formats::path_obj;
use pyo3::{
    exceptions::{PyIsADirectoryError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyString},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
fn opt<'p>(opts: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    opts.call_method1("get", (k,))
}
fn chosen<'p>(items: &Obj<'p>, names: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let out = PyList::empty(items.py());
    let filter = names.is_truthy()?;
    let mut missing = BTreeSet::new();
    if filter {
        for n in names.try_iter()? {
            missing.insert(n?.extract::<String>()?);
        }
    }
    for v in items.try_iter()? {
        let v = v?;
        let name = v.getattr("name")?;
        if !filter || names.contains(&name)? {
            if filter {
                missing.remove(&name.extract::<String>()?);
            }
            out.append(v)?
        }
    }
    if !missing.is_empty() {
        return Err(PyValueError::new_err(format!(
            "Missing scene or sheet: {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    if out.is_empty() {
        return Err(PyValueError::new_err(
            "No scenes or sheets in this delivery",
        ));
    }
    Ok(out)
}
struct View<'p> {
    camera: Obj<'p>,
    picture: Obj<'p>,
    mm: Obj<'p>,
}
impl<'p> View<'p> {
    fn render(
        &self,
        model: &Obj<'p>,
        record: &Obj<'p>,
        path: Option<&Path>,
        view: &Obj<'p>,
    ) -> PyResult<Obj<'p>> {
        let py = model.py();
        let copy = py.import("copy")?;
        let cam = if self.camera.is_none() {
            py.import("core.camera")?.call_method0("OrbitCamera")?
        } else {
            copy.call_method1("copy", (&self.camera,))?
        };
        let mut label = "VISTA".to_string();
        if view.is_truthy()? {
            let planes = py
                .get_type::<PyList>()
                .call1((model.getattr("section_planes")?,))?;
            model.setattr("section_planes", copy.call_method1("deepcopy", (planes,))?)?;
            model.setattr(
                "shadows",
                copy.call_method1("deepcopy", (model.getattr("shadows")?,))?,
            )?;
            for k in ["tile_layer", "terrain", "photo_mesh"] {
                if let Some(v) = model.getattr_opt(k)?.filter(|v| !v.is_none()) {
                    model.setattr(k, copy.call_method1("copy", (v,))?)?
                }
            }
            view.call_method1("apply", (model, &cam))?;
            label = view.getattr("name")?.extract()?;
        }
        let frame = opt(&record.get_item("options")?, "frame")?;
        if !frame.is_truthy()? {
            return self.drawing(model, path, &cam, &label, record, None);
        }
        let scope = py
            .import("ingetrazo_render.view_export")?
            .call_method1("framed", (&cam, frame))?;
        scope.call_method0("__enter__")?;
        let result = self.drawing(model, path, &cam, &label, record, None);
        if let Err(e) = &result {
            scope.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?;
        } else {
            scope.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
        }
        result
    }
    fn drawing(
        &self,
        model: &Obj<'p>,
        path: Option<&Path>,
        cam: &Obj<'p>,
        label: &str,
        record: &Obj<'p>,
        geom: Option<&Obj<'p>>,
    ) -> PyResult<Obj<'p>> {
        let py = model.py();
        let svg = record.get_item("format")?.eq("svg")?;
        let opts = record.get_item("options")?;
        let texts = PyList::empty(py);
        let mut solids = PyList::empty(py).into_any();
        let kw = PyDict::new(py);
        kw.set_item("geometry", geom)?;
        let hlr = py.import("core.hlr")?;
        let groups = if cam.getattr("perspective")?.is_truthy()? {
            PyList::new(
                py,
                [(
                    label,
                    hlr.call_method("hlr_perspective", (model, cam), Some(&kw))?,
                )],
            )?
            .into_any()
        } else {
            kw.set_item("layers", true)?;
            let draw = hlr.call_method("hlr_drawing", (model, cam), Some(&kw))?;
            let layers = py.import("core.layers")?;
            draw.setattr("styles", crate::model::tag_styles::line_styles(py, model, true)?)?;
            draw.setattr("labels", crate::model::module(layers.py(), "ingetrazo_core")?.getattr("tag_edits")?.call_method1("export_labels", (model,))?)?;
            let ls = model.getattr("layers")?;
            if !svg && ls.len()? == 1 {
                draw.getattr("labels")?
                    .set_item(ls.get_item(0)?.getattr("key")?, label)?
            }
            let dxf = py.import("formats.dxf_views")?;
            let args = PyDict::new(py);
            args.set_item("texts", &texts)?;
            let groups =
                dxf.call_method("dxf_groups_by_layer", (draw, label, false), Some(&args))?;
            if !svg {
                kw.del_item("layers")?;
                let (cuts, s) = dxf
                    .call_method("dxf_cut_layers", (model, cam), Some(&kw))?
                    .extract::<(Obj, Obj)>()?;
                groups.call_method1("extend", (cuts,))?;
                solids = s;
            }
            groups
        };
        if !svg {
            let p = path.ok_or_else(|| PyValueError::new_err("DXF requires an output path"))?;
            return py
                .import("formats.dxf_out")?
                .call_method1("save_dxf_layers", (path_obj(py, p)?, groups, solids, texts));
        }
        let faces = opt(&opts, "faces")?;
        let vista = py.import("core.vista_caras")?;
        let fills = if !faces.is_none() {
            vista.call_method("view_faces", (model, cam), Some(faces.cast::<PyDict>()?))?
        } else {
            py.None().into_bound(py)
        };
        let frame = opt(&opts, "frame")?;
        let img = if opt(&opts, "picture")?.is_truthy()? {
            if self.picture.is_none() || !record.get_item("scope")?.eq("visible")? {
                return Err(PyValueError::new_err(
                    "A viewport picture requires the visible scope and an active viewport",
                ));
            }
            self.picture
                .call1((opts.call_method1("get", ("axes", true))?, &frame))?
        } else {
            py.None().into_bound(py)
        };
        let mut paper = opt(&opts, "paper")?;
        let screen = paper.eq("screen")?;
        if screen {
            if self.mm.is_none() {
                return Err(PyValueError::new_err(
                    "Screen size requires viewport dimensions",
                ));
            }
            paper = self.mm.clone()
        }
        let kw = PyDict::new(py);
        kw.set_item("scale_n", opts.call_method1("get", ("scale", 100))?)?;
        kw.set_item("faces", fills)?;
        kw.set_item("image", img)?;
        kw.set_item("paper", paper)?;
        kw.set_item(
            "window",
            if frame.is_truthy()? || screen {
                Some(vista.call_method1("view_rect", (cam,))?)
            } else {
                None
            },
        )?;
        kw.set_item(
            "line_colours",
            if faces.is_truthy()? {
                faces.call_method1("get", ("materials", true))?
            } else {
                true.into_pyobject(py)?.to_owned().into_any()
            },
        )?;
        let text = py
            .import("formats.svg_out")?
            .call_method("svg_text", (groups,), Some(&kw))?
            .get_item(0)?;
        if let Some(p) = path {
            path_obj(py, p)?.call_method1("write_text", (&text, "utf-8"))?;
            Ok(py.None().into_bound(py))
        } else {
            Ok(text)
        }
    }
}
#[pyfunction(signature=(scene,record,*,camera,svg_picture=None,screen_mm=None))]
fn preview_svg<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    record: &Obj<'p>,
    camera: Obj<'p>,
    svg_picture: Option<Obj<'p>>,
    screen_mm: Option<Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let view = if record.get_item("scope")?.eq("scenes")? {
        chosen(
            &scene.getattr("saved_views")?,
            &opt(&record.get_item("options")?, "names")?,
        )?
        .get_item(0)?
    } else {
        py.None().into_bound(py)
    };
    let ctx = View {
        camera,
        picture: svg_picture.unwrap_or_else(|| py.None().into_bound(py)),
        mm: screen_mm.unwrap_or_else(|| py.None().into_bound(py)),
    };
    ctx.render(&delivery_scoped(py, scene, record)?, record, None, &view)
}
struct Stage {
    dir: Py<PyAny>,
    root: PathBuf,
    target: PathBuf,
    project: Option<PathBuf>,
    views: Vec<Py<PyAny>>,
    next: usize,
}
impl Stage {
    fn publish(&self) -> PyResult<()> {
        static WRITERS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = WRITERS.lock().unwrap_or_else(|e| e.into_inner());
        let outputs = fs::read_dir(&self.root)?.collect::<Result<Vec<_>, _>>()?;
        if outputs.is_empty() {
            return Err(PyValueError::new_err("The exporter produced no files"));
        }
        let parent = self
            .target
            .parent()
            .ok_or_else(|| PyValueError::new_err("missing delivery directory"))?;
        let backup = self.root.join(".previous");
        fs::create_dir(&backup)?;
        let mut installed = vec![];
        let mut originals = vec![];
        let result = (|| -> PyResult<()> {
            for f in outputs {
                let name = f.file_name();
                let dest = parent.join(&name);
                if dest.is_dir() {
                    return Err(PyIsADirectoryError::new_err(dest));
                }
                if dest.symlink_metadata().is_ok() {
                    let saved = backup.join(&name);
                    fs::rename(&dest, &saved)?;
                    originals.push((saved, dest.clone()));
                }
                fs::rename(f.path(), &dest)?;
                installed.push(dest);
            }
            Ok(())
        })();
        if result.is_err() {
            for dst in installed.into_iter().rev() {
                fs::remove_file(dst)?
            }
            for (src, dst) in originals.into_iter().rev() {
                fs::rename(src, dst)?
            }
        }
        result
    }
}
#[pyclass]
struct DeliverySteps {
    scene: Py<PyAny>,
    record: Py<PyAny>,
    camera: Py<PyAny>,
    pdf: Py<PyAny>,
    picture: Py<PyAny>,
    mm: Py<PyAny>,
    project: Option<PathBuf>,
    geometry: Py<PyAny>,
    stage: Option<Stage>,
    closed: bool,
}
impl DeliverySteps {
    fn start(&self, py: Python<'_>) -> PyResult<Stage> {
        let r = self.record.bind(py);
        let s = self.scene.bind(py);
        delivery_validate(r)?;
        let project = match &self.project {
            Some(p) => Some(p.clone()),
            None => s
                .getattr_opt("_project_host_path")?
                .filter(|v| v.is_truthy().unwrap_or(false))
                .map(|p| p.extract::<PathBuf>())
                .transpose()?
                .and_then(|p| p.parent().map(Path::to_owned)),
        };
        let raw = py
            .import("pathlib")?
            .call_method1("Path", (r.get_item("destination")?,))?
            .call_method0("expanduser")?
            .extract::<PathBuf>()?;
        let target = if raw.is_absolute() {
            raw
        } else {
            project
                .as_ref()
                .ok_or_else(|| {
                    PyValueError::new_err("Choose an absolute destination for an unsaved document")
                })?
                .join(raw)
        };
        let views = if r.get_item("scope")?.eq("scenes")? {
            chosen(
                &s.getattr("saved_views")?,
                &opt(&r.get_item("options")?, "names")?,
            )?
            .iter()
            .map(Bound::unbind)
            .collect()
        } else {
            vec![py.None()]
        };
        let kw = PyDict::new(py);
        kw.set_item("dir", target.parent())?;
        let dir = py
            .import("tempfile")?
            .call_method("TemporaryDirectory", (), Some(&kw))?;
        let root = dir.getattr("name")?.extract()?;
        Ok(Stage {
            dir: dir.unbind(),
            root,
            target,
            project,
            views,
            next: 0,
        })
    }
    fn write(&self, py: Python<'_>, st: &Stage, index: usize) -> PyResult<()> {
        let r = self.record.bind(py);
        let s = self.scene.bind(py);
        let fmt = r.get_item("format")?.extract::<String>()?;
        let opts = r.get_item("options")?;
        let name = st
            .target
            .file_name()
            .ok_or_else(|| PyValueError::new_err("missing file name"))?;
        let p = if st.views.len() == 1 {
            st.root.join(name)
        } else {
            let target = path_obj(py, &st.target)?;
            st.root.join(format!(
                "{}-{:02}{}",
                target.getattr("stem")?,
                index + 1,
                target.getattr("suffix")?
            ))
        };
        let path = path_obj(py, &p)?;
        let ctx = View {
            camera: self.camera.bind(py).clone(),
            picture: self.picture.bind(py).clone(),
            mm: self.mm.bind(py).clone(),
        };
        if fmt == "pdf" {
            let writer = self.pdf.bind(py);
            let sheets = chosen(&s.getattr("compositions")?, &opt(&opts, "names")?)?;
            if writer.is_none() {
                return Err(PyValueError::new_err("Sheet export requires a composer"));
            }
            let failed = writer.call1((path.str()?, sheets))?;
            if failed.is_truthy()? {
                return Err(PyValueError::new_err(format!(
                    "Frames failed on sheets: {}",
                    PyString::new(py, ", ").call_method1("join", (failed,))?
                )));
            }
            return Ok(());
        }
        if fmt == "dxf" && r.get_item("scope")?.eq("visible")? && !self.geometry.bind(py).is_none()
        {
            let kw = PyDict::new(py);
            kw.set_item("tags", !ctx.camera.getattr("perspective")?.is_truthy()?)?;
            let g = self.geometry.bind(py).call((), Some(&kw))?;
            ctx.drawing(s, Some(&p), &ctx.camera, "VISTA", r, Some(&g))?;
            return Ok(());
        }
        let model = delivery_scoped(py, s, r)?;
        let kw = PyDict::new(py);
        match fmt.as_str() {
            "ifc" => {
                kw.set_item("profile", opt(&opts, "profile")?)?;
                kw.set_item(
                    "project_directory",
                    st.project.as_deref().map(|p| path_obj(py, p)).transpose()?,
                )?;
                py.import("formats.ifc")?
                    .call_method("save_ifc", (model, path), Some(&kw))?
            }
            "glb" => {
                kw.set_item(
                    "embed_textures",
                    opts.call_method1("get", ("textures", true))?,
                )?;
                py.import("formats.gltf")?
                    .call_method("save_glb", (model, path), Some(&kw))?
            }
            "obj" => py
                .import("formats.obj")?
                .call_method1("save_obj", (model, path))?,
            "dae" => py
                .import("formats.dae")?
                .call_method1("save_dae", (model, path))?,
            "stl" => py.import("formats.stl")?.call_method1(
                "save_stl",
                (model, path, opts.call_method1("get", ("unit", "mm"))?),
            )?,
            _ => ctx.render(&model, r, Some(&p), st.views[index].bind(py))?,
        };
        Ok(())
    }
    fn step<'p>(&mut self, py: Python<'p>) -> PyResult<Option<(f64, Obj<'p>)>> {
        if self.closed {
            return Ok(None);
        }
        if self.stage.is_none() {
            self.stage = Some(self.start(py)?)
        }
        let st = self.stage.as_ref().unwrap();
        if st.next > 0 {
            self.write(py, st, st.next - 1)?
        }
        if st.next == st.views.len() {
            st.publish()?;
            return Ok(None);
        }
        let st = self.stage.as_mut().unwrap();
        let i = st.next;
        st.next += 1;
        let view = st.views[i].bind(py);
        Ok(Some((
            i as f64 / st.views.len() as f64,
            if view.is_none() {
                PyString::new(py, "Writing").into_any()
            } else {
                view.getattr("name")?
            },
        )))
    }
}
#[pymethods]
impl DeliverySteps {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn close(&mut self, py: Python<'_>) -> PyResult<()> {
        self.closed = true;
        if let Some(st) = self.stage.take() {
            st.dir.bind(py).call_method0("cleanup")?;
        }
        Ok(())
    }
    fn __next__<'p>(&mut self, py: Python<'p>) -> PyResult<Option<(f64, Obj<'p>)>> {
        let result = self.step(py);
        if !matches!(result, Ok(Some(_))) {
            self.close(py)?
        }
        result
    }
}
#[pyfunction(signature=(scene,record,*,camera=None,pdf_writer=None,svg_picture=None,screen_mm=None,project_directory=None,view_geometry=None))]
fn write_steps(
    py: Python<'_>,
    scene: Py<PyAny>,
    record: Py<PyAny>,
    camera: Option<Py<PyAny>>,
    pdf_writer: Option<Py<PyAny>>,
    svg_picture: Option<Py<PyAny>>,
    screen_mm: Option<Py<PyAny>>,
    project_directory: Option<PathBuf>,
    view_geometry: Option<Py<PyAny>>,
) -> DeliverySteps {
    let optional = |o: Option<Py<PyAny>>| o.unwrap_or_else(|| py.None());
    DeliverySteps {
        scene,
        record,
        camera: optional(camera),
        pdf: optional(pdf_writer),
        picture: optional(svg_picture),
        mm: optional(screen_mm),
        geometry: optional(view_geometry),
        project: project_directory,
        stage: None,
        closed: false,
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    functions!(m;preview_svg,write_steps);
    Ok(())
}
