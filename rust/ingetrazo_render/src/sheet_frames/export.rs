//! Rebuild each frame at delivery time and pass its content to the sheet writers.
use super::*;
type Content<'py> = (
    Bound<'py, PyDict>,
    Bound<'py, PyDict>,
    Option<Bound<'py, PyDict>>,
    Vec<String>,
);
#[pyfunction]
#[pyo3(signature=(view_widget,comps,ground=None,providers=None))]
fn frame_contents<'py>(
    view_widget: &Obj<'py>,
    comps: &Obj<'py>,
    ground: Option<&Obj<'py>>,
    providers: Option<&Obj<'py>>,
) -> PyResult<Content<'py>> {
    let py = view_widget.py();
    let source = view_widget.getattr("source_viewport")?;
    let scene = source.getattr("scene")?;
    let contents = PyDict::new(py);
    let pictures = PyDict::new(py);
    let profiles = PyDict::new(py);
    let mut troubled = Vec::new();
    for comp in comps.try_iter()? {
        let comp = comp?;
        if let Some(g) = ground {
            let p = g.call_method1("profiles", (&comp, &scene))?;
            if p.is_truthy()? {
                profiles.call_method1("update", (p,))?;
            }
        }
        for f in comp.getattr("frames")?.try_iter()? {
            let f = f?;
            let key = f.getattr("_doc_key")?;
            let result = (|| -> PyResult<()> {
                let view = saved_view(&scene, &f)?;
                if absent(&view) {
                    let missing = PyDict::new(py);
                    let name: String = f.getattr("view_key")?.extract()?;
                    missing.set_item("missing", name.strip_prefix("scene:").unwrap_or(&name))?;
                    contents.set_item(&key, missing)?;
                    return Ok(());
                }
                let record =
                    if f.getattr("style")?.eq("vectorial")? && !flag(&f, "perspective", false)? {
                        geometry::drawing(&source, &f)?
                    } else if let Some((w, h, data)) = picture(view_widget, &f)? {
                        pictures.set_item(format!("frame:{}", key.str()?), (w, h, data))?;
                        let entry = PyDict::new(py);
                        entry.set_item("image", (w, h))?;
                        Some(entry)
                    } else {
                        None
                    };
                if let Some(record) = record {
                    contents.set_item(&key, &record)?;
                    record.set_item("annots", records::annotations(&source, &f, providers)?)?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                e.print(py);
                let name = comp.getattr("name")?.extract()?;
                if !troubled.contains(&name) {
                    troubled.push(name);
                }
            }
        }
    }
    Ok((
        contents,
        pictures,
        (!profiles.is_empty()).then_some(profiles),
        troubled,
    ))
}
#[pyfunction]
#[pyo3(signature=(view_widget,comps,path,ground=None,doc_path=None,providers=None))]
fn export_pdf(
    view_widget: &Obj<'_>,
    comps: &Obj<'_>,
    path: &Obj<'_>,
    ground: Option<&Obj<'_>>,
    doc_path: Option<&Obj<'_>>,
    providers: Option<&Obj<'_>>,
) -> PyResult<Vec<String>> {
    let comps = list(comps)?;
    let (c, p, profiles, troubled) = frame_contents(view_widget, &comps, ground, providers)?;
    core(path.py())?.call_method1(
        "sheet_pdf",
        (
            path.str()?,
            comps,
            c,
            p,
            profiles,
            view_widget.getattr("source_viewport")?.getattr("scene")?,
            doc_path.map(|p| p.str()).transpose()?,
        ),
    )?;
    Ok(troubled)
}
#[pyfunction]
#[pyo3(signature=(view_widget,comp,path,dpi=200.,ground=None,doc_path=None,providers=None))]
fn export_image(
    view_widget: &Obj<'_>,
    comp: &Obj<'_>,
    path: &Obj<'_>,
    dpi: f64,
    ground: Option<&Obj<'_>>,
    doc_path: Option<&Obj<'_>>,
    providers: Option<&Obj<'_>>,
) -> PyResult<Vec<String>> {
    let (c, p, profiles, troubled) = frame_contents(
        view_widget,
        PyList::new(path.py(), [comp])?.as_any(),
        ground,
        providers,
    )?;
    core(path.py())?.call_method1(
        "sheet_image",
        (
            path.str()?,
            comp,
            dpi,
            c,
            p,
            profiles,
            view_widget.getattr("source_viewport")?.getattr("scene")?,
            doc_path.map(|p| p.str()).transpose()?,
        ),
    )?;
    Ok(troubled)
}
struct LineGroups<'py> {
    groups: Obj<'py>,
    texts: Bound<'py, PyList>,
    pass: geometry::FramePass<'py>,
}
fn line_groups<'py>(source: &Obj<'py>, frame: &Obj<'py>) -> PyResult<Option<LineGroups<'py>>> {
    let py = source.py();
    let Some(pass) = geometry::frame_pass(source, frame, true)? else {
        return Ok(None);
    };
    let hidden = flag(frame, "hidden_lines", false)?;
    let native = core(py)?;
    let styles = native.getattr("tag_styles")?;
    let by_layer = styles
        .call_method1("layer_lines_wanted", (&pass.scene,))?
        .is_truthy()?
        || opt(&pass.scene, "layers")
            .map(|v| v.len())
            .transpose()?
            .unwrap_or(0)
            > 1;
    let kw = PyDict::new(py);
    let geo = if by_layer {
        pass.geometry.clone()
    } else {
        pass.geometry.get_slice(0, 4)
    };
    kw.set_item("geometry", geo)?;
    kw.set_item("layers", by_layer)?;
    kw.set_item("profiles", flag(frame, "profiles", true)?)?;
    kw.set_item("fills", false)?;
    kw.set_item("hidden", hidden)?;
    let d = py
        .import("core.hlr")?
        .getattr("hlr_drawing")?
        .call((&pass.scene, &pass.camera), Some(&kw))?;
    let name: String = py
        .import("core.composition")?
        .call_method1("frame_view_name", (frame,))?
        .extract()?;
    let layer = name.split(" — ").next().unwrap_or_default();
    let texts = PyList::empty(py);
    let dxf = py.import("formats.dxf_views")?;
    let groups = if !d.getattr("layers")?.is_none() {
        d.setattr(
            "styles",
            styles.call_method1("line_styles", (&pass.scene, true))?,
        )?;
        d.setattr(
            "labels",
            native
                .getattr("tag_edits")?
                .call_method1("export_labels", (&pass.scene,))?,
        )?;
        dxf.call_method1("dxf_groups_by_layer", (d, layer, hidden, &texts))?
    } else {
        dxf.call_method1("dxf_groups_by_class", (d, layer, hidden))?
    };
    Ok(Some(LineGroups {
        groups,
        texts,
        pass,
    }))
}
fn positive_default(frame: &Obj<'_>, name: &str, default: f64) -> PyResult<f64> {
    let v = opt(frame, name)
        .map(|v| v.extract::<f64>())
        .transpose()?
        .unwrap_or(default);
    Ok(if v == 0. { default } else { v })
}
#[pyfunction]
fn export_dxf<'py>(
    source: &Obj<'py>,
    frame: &Obj<'py>,
    path: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    let py = source.py();
    let Some(lines) = line_groups(source, frame)? else {
        return Ok(None);
    };
    let mut solids = PyList::empty(py).into_any();
    if !opt(frame, "cut_fill").is_some_and(|v| v.eq("none").unwrap_or(false)) {
        let scale = positive_default(frame, "scale_n", 100.)?;
        let h = number(frame, "h_mm")?;
        let height = py
            .import("core.composition")?
            .call_method1("model_height_for_frame", (h, scale))?
            .extract::<f64>()?;
        let kw = PyDict::new(py);
        kw.set_item("geometry", lines.pass.geometry)?;
        let cut = py
            .import("formats.dxf_views")?
            .getattr("dxf_cut_layers")?
            .call(
                (
                    lines.pass.scene,
                    lines.pass.camera,
                    scale,
                    h / height,
                    (positive_default(frame, "cut_hatch_mm", 1.5)? / 1.5).max(0.2),
                ),
                Some(&kw),
            )?;
        lines.groups.call_method1("extend", (cut.get_item(0)?,))?;
        solids = cut.get_item(1)?;
    }
    Ok(Some(py.import("formats.dxf_out")?.call_method1(
        "save_dxf_layers",
        (path, lines.groups, solids, lines.texts),
    )?))
}
#[pyfunction]
fn export_svg<'py>(
    source: &Obj<'py>,
    frame: &Obj<'py>,
    path: &Obj<'py>,
) -> PyResult<Option<usize>> {
    let py = source.py();
    let Some(lines) = line_groups(source, frame)? else {
        return Ok(None);
    };
    let pens = PyDict::new(py);
    for (k, field, def) in [
        ("cut", "pen_cut_mm", 0.5),
        ("profile", "pen_profile_mm", 0.35),
        ("edge", "pen_edge_mm", 0.18),
    ] {
        pens.set_item(k, positive_default(frame, field, def)?.max(0.05))?;
    }
    let (text, n): (String, usize) = py
        .import("formats.svg_out")?
        .call_method1(
            "svg_text",
            (
                lines.groups,
                positive_default(frame, "scale_n", 100.)?,
                pens,
                py.None(),
                true,
                py.None(),
                py.None(),
            ),
        )?
        .extract()?;
    std::fs::write(path.extract::<std::path::PathBuf>()?, text)?;
    Ok(Some(n))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(frame_contents, m)?,
        wrap_pyfunction!(export_pdf, m)?,
        wrap_pyfunction!(export_image, m)?,
        wrap_pyfunction!(export_dxf, m)?,
        wrap_pyfunction!(export_svg, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
