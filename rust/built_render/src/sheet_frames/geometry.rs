//! Project the hidden-line drawing to paper, keeping the corresponding world points.
use super::*;
use numpy::{IntoPyArray, PyArrayMethods, PyReadonlyArrayDyn};

pub(super) struct FramePass<'py> {
    pub scene: Obj<'py>,
    pub camera: Obj<'py>,
    pub geometry: Bound<'py, PyTuple>,
}
pub(super) fn frame_pass<'py>(
    source: &Obj<'py>,
    frame: &Obj<'py>,
    tags: bool,
) -> PyResult<Option<FramePass<'py>>> {
    let py = frame.py();
    let scene = source.getattr("scene")?;
    let view = saved_view(&scene, frame)?;
    if absent(&view) {
        return Ok(None);
    }
    let facade = frame_scene(&scene, frame, &view)?;
    let camera = frame_camera(&source.getattr("camera")?, frame, &view, &scene, None)?;
    let hypot = py
        .import("core._native")?
        .call_method0("qt_hypot")?
        .extract::<Option<String>>()?
        .unwrap_or_else(|| "libc++-fma".into());
    let (data, tag_data): (Bound<PyTuple>, Obj) = core(py)?
        .call_method1(
            "sheet_frame_geometry",
            (
                scene.getattr("_root")?,
                list(&scene.getattr("groups")?)?,
                scene.getattr("loose_mesh")?,
                view,
                hypot,
                tags,
            ),
        )?
        .extract()?;
    let geometry = if tags {
        let mut arrays: Vec<_> = data.iter().collect();
        let np = py.import("numpy")?;
        for t in tag_data.try_iter()? {
            arrays.push(np.call_method1("array", (t?, "object"))?);
        }
        PyTuple::new(py, arrays)?
    } else {
        data
    };
    Ok(Some(FramePass {
        scene: facade,
        camera,
        geometry,
    }))
}
pub(super) struct Paper {
    half_h: f64,
    half_w: f64,
    pub scale: f64,
}
impl Paper {
    pub fn of(frame: &Obj<'_>) -> PyResult<Self> {
        let h = number(frame, "h_mm")?;
        let height = frame
            .py()
            .import("core.composition")?
            .call_method1("model_height_for_frame", (h, frame.getattr("scale_n")?))?
            .extract::<f64>()?;
        Ok(Self {
            half_h: height * 0.5,
            half_w: height * 0.5 * (number(frame, "w_mm")? / h),
            scale: h / height,
        })
    }
    fn xy(&self, x: f64, y: f64) -> [f64; 2] {
        [
            (x + self.half_w) * self.scale,
            (self.half_h - y) * self.scale,
        ]
    }
    fn project(&self, values: &mut [f64]) {
        for xy in values.chunks_exact_mut(2) {
            let p = self.xy(xy[0], xy[1]);
            xy.copy_from_slice(&p);
        }
    }
}
fn values(o: &Obj<'_>) -> PyResult<Vec<f64>> {
    if let Ok(arr) = o.extract::<PyReadonlyArrayDyn<f64>>() {
        return Ok(arr.as_array().iter().copied().collect());
    }
    let arr = o
        .py()
        .import("numpy")?
        .call_method1("asarray", (o, "float64"))?;
    Ok(arr
        .extract::<PyReadonlyArrayDyn<f64>>()?
        .as_array()
        .iter()
        .copied()
        .collect())
}
fn array<'py>(py: Python<'py>, v: Vec<f64>, width: usize) -> PyResult<Obj<'py>> {
    let n = v.len() / width;
    Ok(v.into_pyarray(py).reshape([n, width])?.into_any())
}
fn run_hlr<'py>(pass: &FramePass<'py>, tags: bool, kw: &Bound<'py, PyDict>) -> PyResult<Obj<'py>> {
    kw.set_item("geometry", &pass.geometry)?;
    kw.set_item("layers", tags)?;
    kw.py()
        .import("core.hlr")?
        .getattr("hlr_drawing")?
        .call((&pass.scene, &pass.camera), Some(kw))
}
#[pyfunction]
pub(super) fn drawing<'py>(
    source: &Obj<'py>,
    frame: &Obj<'py>,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let py = frame.py();
    let scene = source.getattr("scene")?;
    let view = saved_view(&scene, frame)?;
    if absent(&view) {
        return Ok(None);
    }
    let facade = frame_scene(&scene, frame, &view)?;
    let styles = core(py)?.getattr("tag_styles")?;
    let layers = py.import("core.layers")?;
    let coloured = flag(frame, "layer_colours", false)?;
    let filled = !opt(frame, "cut_fill").is_some_and(|v| v.eq("none").unwrap_or(false));
    let fill_layer = filled && flag(frame, "cut_fill_by_layer", false)?;
    let hatches = if filled {
        styles.call_method1("cut_hatches", (&facade,))?
    } else {
        PyDict::new(py).into_any()
    };
    let by_layer = styles
        .call_method1("layer_lines_wanted", (&facade,))?
        .is_truthy()?
        || coloured
        || fill_layer
        || hatches.is_truthy()?;
    let Some(pass) = frame_pass(source, frame, by_layer)? else {
        return Ok(None);
    };
    let kw = PyDict::new(py);
    kw.set_item("profiles", flag(frame, "profiles", true)?)?;
    kw.set_item("fills", filled)?;
    kw.set_item("hidden", flag(frame, "hidden_lines", false)?)?;
    if by_layer {
        kw.set_item(
            "depth_layers",
            layers.call_method1("grain_layers", (&facade,))?,
        )?;
    }
    let d = run_hlr(&pass, by_layer, &kw)?;
    let paper = Paper::of(frame)?;
    let out = PyDict::new(py);
    let mut segs = values(&d.getattr("segs")?)?;
    paper.project(&mut segs);
    out.set_item("segs", array(py, segs, 4)?)?;
    out.set_item(
        "kinds",
        py.import("numpy")?
            .call_method1("asarray", (d.getattr("kinds")?, "int8"))?,
    )?;
    let rings = PyList::empty(py);
    for r in d.getattr("loops")?.try_iter()? {
        let mut pts = values(&r?)?;
        paper.project(&mut pts);
        rings.append(array(py, pts, 2)?)?;
    }
    out.set_item("fills", rings)?;
    if let Some(tags) = opt(&d, "layers") {
        let style = opt(&facade, "display_style");
        let colour = coloured
            || style
                .as_ref()
                .is_some_and(|s| flag(s, "color_by_layer", false).unwrap_or(false));
        let looks = styles.call_method1("line_styles", (&facade, colour))?;
        let mapped = PyDict::new(py);
        let types = layers.getattr("LINETYPES")?;
        for (tag, st) in looks.cast::<PyDict>()?.iter() {
            let dash = st.get_item(4)?;
            let dash = if dash.is_none() {
                types.call_method1("get", (st.get_item(1)?, PyTuple::empty(py)))?
            } else {
                dash
            };
            mapped.set_item(
                tag,
                (
                    st.get_item(0)?,
                    st.get_item(2)?,
                    st.get_item(3)?,
                    list(&dash)?,
                    st.get_item(5).ok(),
                ),
            )?;
        }
        let labels = PyList::empty(py);
        for tag in tags.try_iter()? {
            labels.append(tag?.str()?)?;
        }
        out.set_item("tags", labels)?;
        out.set_item("styles", mapped)?;
    }
    if fill_layer || hatches.is_truthy()? {
        if let Some(tags) = opt(&d, "loop_layers") {
            let colors = if fill_layer {
                styles.call_method1("cut_fill_colours", (&facade,))?
            } else {
                PyDict::new(py).into_any()
            };
            let grounds = styles.call_method1("cut_hatch_grounds", (&facade,))?;
            let inks = styles.call_method1("cut_hatch_inks", (&facade,))?;
            let scale = number(frame, "scale_n")?;
            let forms = styles.call_method1(
                "cut_hatch_forms",
                (&facade, if scale == 0. { 100. } else { scale }),
            )?;
            let cut = layers.call_method1("section_cut_kind", (&facade,))?;
            let depths = opt(&d, "loop_depths").filter(|v| v.is_truthy().unwrap_or(false));
            let fills = PyList::empty(py);
            for (i, tag) in tags.try_iter()?.enumerate() {
                let tag = tag?;
                let rgb = |t: &Obj<'py>| -> PyResult<Option<Obj<'py>>> {
                    if !t.contains(&tag)? {
                        return Ok(None);
                    }
                    let color = t.get_item(&tag)?;
                    Ok(Some(list(
                        &color.get_item(pyo3::types::PySlice::new(py, 0, 3, 1))?,
                    )?))
                };
                let form = forms.call_method1("get", (&tag,))?;
                let form = if form.is_none() {
                    form
                } else {
                    let f = form.call_method0("copy")?;
                    f.set_item("cut", &cut)?;
                    if let Some(ref vals) = depths {
                        let depth = vals.get_item(i)?;
                        if !depth.is_none() {
                            f.set_item(
                                "depth",
                                py.import("builtins")?.call_method1(
                                    "round",
                                    (depth.extract::<f64>()? * paper.scale, 4),
                                )?,
                            )?;
                        }
                    }
                    f
                };
                fills.append((
                    rgb(&colors)?,
                    hatches.call_method1("get", (&tag,))?,
                    rgb(&grounds)?,
                    rgb(&inks)?,
                    form,
                ))?;
            }
            out.set_item("fill_looks", fills)?;
        }
    }
    Ok(Some(out))
}
#[pyfunction]
pub(super) fn snaps<'py>(
    source: &Obj<'py>,
    frame: &Obj<'py>,
) -> PyResult<Option<Bound<'py, PyTuple>>> {
    let py = source.py();
    if flag(frame, "perspective", false)? {
        return Ok(None);
    }
    let Some(pass) = frame_pass(source, frame, false)? else {
        return Ok(None);
    };
    let kw = PyDict::new(py);
    for k in ["profiles", "fills", "hidden"] {
        kw.set_item(k, false)?;
    }
    let d = run_hlr(&pass, false, &kw)?;
    let mut segs = values(&d.getattr("segs")?)?;
    Paper::of(frame)?.project(&mut segs);
    let (x, y) = (number(frame, "x_mm")?, number(frame, "y_mm")?);
    let (x0, y0, x1, y1) = (
        x - 0.5,
        y - 0.5,
        x + number(frame, "w_mm")? + 0.5,
        y + number(frame, "h_mm")? + 0.5,
    );
    for xy in segs.chunks_exact_mut(2) {
        xy[0] += x;
        xy[1] += y;
    }
    let world = values(&d.getattr("world")?)?;
    let mut pts = Vec::new();
    let mut wpts = Vec::new();
    let mut edges = Vec::new();
    let mut wedges = Vec::new();
    for i in 0..3 {
        for (s, w) in segs.chunks_exact(4).zip(world.chunks_exact(6)) {
            let (p, q) = match i {
                0 => ([s[0], s[1]], [w[0], w[1], w[2]]),
                1 => ([s[2], s[3]], [w[3], w[4], w[5]]),
                _ => (
                    [(s[0] + s[2]) * 0.5, (s[1] + s[3]) * 0.5],
                    [
                        (w[0] + w[3]) * 0.5,
                        (w[1] + w[4]) * 0.5,
                        (w[2] + w[5]) * 0.5,
                    ],
                ),
            };
            if (x0..=x1).contains(&p[0]) && (y0..=y1).contains(&p[1]) {
                pts.extend(p);
                wpts.extend(q);
            }
            if i == 0
                && s[0].max(s[2]) >= x0
                && s[0].min(s[2]) <= x1
                && s[1].max(s[3]) >= y0
                && s[1].min(s[3]) <= y1
            {
                edges.extend_from_slice(s);
                wedges.extend_from_slice(w);
            }
        }
    }
    let n = wedges.len() / 6;
    Ok(Some(PyTuple::new(
        py,
        [
            array(py, pts, 2)?,
            array(py, wpts, 3)?,
            array(py, edges, 4)?,
            wedges.into_pyarray(py).reshape([n, 2, 3])?.into_any(),
            circles(source, frame)?,
        ],
    )?))
}
fn xyz(v: &Obj<'_>) -> PyResult<[f64; 3]> {
    v.call_method0("toTuple")?.extract()
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[pyfunction]
fn circles<'py>(source: &Obj<'py>, frame: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = source.py();
    if flag(frame, "perspective", false)? {
        return array(py, vec![], 3);
    }
    let scene = source.getattr("scene")?;
    let cache = this(py)?.getattr("_circles")?;
    let stamp = (scene.as_ptr() as usize, opt(&scene, "version")).into_pyobject(py)?;
    if !cache.call_method1("get", ("stamp",))?.eq(&stamp)? {
        let data = py
            .import("core.hlr")?
            .call_method1("collect_circles", (&scene,))?;
        cache.set_item("values", data)?;
        cache.set_item("stamp", stamp)?;
    }
    let view = saved_view(&scene, frame)?;
    if absent(&view) {
        return array(py, vec![], 3);
    }
    let camera = frame_camera(&source.getattr("camera")?, frame, &view, &scene, None)?;
    let basis = core(py)?.call_method1("hlr_camera_basis", (camera,))?;
    let eye: [f64; 3] = basis.get_item(0)?.extract()?;
    let right = basis.get_item(1)?.extract()?;
    let up = basis.get_item(2)?.extract()?;
    let fwd = basis.get_item(3)?.extract()?;
    let paper = Paper::of(frame)?;
    let x = number(frame, "x_mm")?;
    let y = number(frame, "y_mm")?;
    let mut out = vec![];
    for circle in cache.get_item("values")?.try_iter()? {
        let (c, r, n): (Obj, f64, Obj) = circle?.extract()?;
        if dot(xyz(&n)?, fwd).abs() < 0.9995 {
            continue;
        }
        let c = xyz(&c)?;
        let project = |p: [f64; 3]| {
            let v = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
            paper.xy(dot(v, right), dot(v, up))
        };
        let p = project(c);
        let q = project([
            c[0] + right[0] * r,
            c[1] + right[1] * r,
            c[2] + right[2] * r,
        ]);
        out.extend([x + p[0], y + p[1], (q[0] - p[0]).hypot(q[1] - p[1])]);
    }
    array(py, out, 3)
}
#[pyfunction]
fn level_at(source: &Obj<'_>, frame: &Obj<'_>, x: f64, y: f64) -> PyResult<Option<f64>> {
    let h = number(frame, "h_mm")?;
    if flag(frame, "perspective", false)? || h <= 0. {
        return Ok(None);
    }
    let scene = source.getattr("scene")?;
    let view = saved_view(&scene, frame)?;
    if absent(&view) {
        return Ok(None);
    }
    let camera = frame_camera(&source.getattr("camera")?, frame, &view, &scene, None)?;
    let basis = core(source.py())?.call_method1("hlr_camera_basis", (camera,))?;
    let z = |i| -> PyResult<f64> { basis.get_item(i)?.get_item(2)?.extract() };
    if z(3)?.abs() > 1e-3 {
        return Ok(None);
    }
    let p = Paper::of(frame)?;
    if p.half_h <= 1e-9 {
        return Ok(None);
    }
    Ok(Some(
        z(0)?
            + z(1)? * ((x - number(frame, "x_mm")?) / p.scale - p.half_w)
            + z(2)? * (p.half_h - (y - number(frame, "y_mm")?) / p.scale),
    ))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("_circles", PyDict::new(m.py()))?;
    for f in [
        wrap_pyfunction!(drawing, m)?,
        wrap_pyfunction!(snaps, m)?,
        wrap_pyfunction!(circles, m)?,
        wrap_pyfunction!(level_at, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
