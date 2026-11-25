//! File and component placement share the active view's transient tool path.
use super::{model_edit::translated, window_document::transient_tool_class};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::path::PathBuf;

fn file_picker<'py>(
    win: &Bound<'py, PyAny>,
    method: &str,
    title: &str,
    initial: &str,
    filter: &str,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = win.py();
    let filename: String = py
        .import("ingetrazo_render.shell_files")?
        .getattr("ShellFileDialog")?
        .call_method1(
            method,
            (
                win,
                translated(py, title, None)?,
                initial,
                translated(py, filter, None)?,
            ),
        )?
        .get_item(0)?
        .extract()?;
    Ok(if filename.is_empty() {
        None
    } else {
        Some(py.import("pathlib")?.call_method1("Path", (filename,))?)
    })
}
fn activate(
    win: &Bound<'_, PyAny>,
    tool: &Bound<'_, PyAny>,
    label: &str,
    hint: &str,
    ms: u32,
) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    vp.call_method1("set_active_tool", (tool,))?;
    for a in win
        .getattr("_tool_actions")?
        .call_method0("values")?
        .try_iter()?
    {
        a?.call_method1("setChecked", (false,))?;
    }
    let vars = PyDict::new(py);
    vars.set_item("name", translated(py, label, None)?)?;
    win.setattr("tool_label", translated(py, "Tool: {name}", Some(&vars))?)?;
    vp.call_method1("flash_status", (translated(py, hint, None)?, ms))?;
    vp.call_method0("update")?;
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(win,group,align_to_face=false,anchor=None))]
fn start_placement(
    win: &Bound<'_, PyAny>,
    group: &Bound<'_, PyAny>,
    align_to_face: bool,
    anchor: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let py = win.py();
    let kw = PyDict::new(py);
    kw.set_item("align_to_face", align_to_face)?;
    kw.set_item("anchor", anchor)?;
    let tool = transient_tool_class(py, "place_group")?.call((group,), Some(&kw))?;
    activate(
        win,
        &tool,
        "Place component",
        "Click to place the component (Esc cancels)",
        4000,
    )
}
#[pyfunction]
#[pyo3(signature=(win,path,at=None))]
fn insert_document<'py>(
    win: &Bound<'py, PyAny>,
    path: PathBuf,
    at: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let p = py.import("pathlib")?.call_method1("Path", (path,))?;
    let temp = py.import("core.scene")?.call_method0("Scene")?;
    py.import("formats.igz")?
        .call_method1("load_into", (&temp, &p))?;
    let vp = win.getattr("viewport")?;
    vp.call_method0("end_group_edit")?;
    let group = py.import("core.insert")?.call_method1(
        "import_document_as_component",
        (vp.getattr("scene")?, temp, p.getattr("stem")?),
    )?;
    if group.is_none() {
        return Ok(group);
    }
    let vec = py.import("ingetrazo_core")?.getattr("Vec3")?;
    let kw = PyDict::new(py);
    kw.set_item("anchor", vec.call0()?)?;
    if let Some(at) = at {
        transient_tool_class(py, "place_group")?
            .call((&group,), Some(&kw))?
            .call_method1("place_at", (vp, vec.call1((at,))?))?;
    } else {
        win.call_method("_start_place", (&group,), Some(&kw))?;
    }
    Ok(group)
}
#[pyfunction]
fn place_text(win: &Bound<'_, PyAny>, params: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    win.getattr("viewport")?.call_method0("end_group_edit")?;
    let group = py
        .import("core.text3d")?
        .call_method1("make_text_group", (params,))?;
    if group.is_none() {
        py.import("ingetrazo_render.chrome_dialogs")?.call_method1(
            "warning",
            (
                win,
                translated(py, "3D Text", None)?,
                translated(py, "Could not build geometry for that text.", None)?,
            ),
        )?;
    } else {
        let kw = PyDict::new(py);
        kw.set_item("align_to_face", true)?;
        win.call_method("_start_place", (group,), Some(&kw))?;
    }
    Ok(())
}
#[pyfunction]
fn import_picture(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let Some(path) = file_picker(
        win,
        "getOpenFileName",
        "Import image",
        "",
        "Images (*.png *.jpg *.jpeg *.bmp *.tif *.tiff *.webp);;All files (*)",
    )?
    else {
        return Ok(());
    };
    let (aspect, w, h): (f64, i64, i64) = py
        .import("core.image_plane")?
        .call_method1("image_aspect", (&path,))?
        .extract()?;
    let dialogs = py.import("ingetrazo_render.chrome_dialogs")?;
    let vars = PyDict::new(py);
    vars.set_item("name", path.getattr("name")?)?;
    if w <= 0 || h <= 0 {
        dialogs.call_method1(
            "critical",
            (
                win,
                translated(py, "Import image failed", None)?,
                translated(
                    py,
                    "{name} is not an image this build can read.",
                    Some(&vars),
                )?,
            ),
        )?;
        return Ok(());
    }
    let cache = || -> PyResult<Bound<'_, PyAny>> {
        let bytes = std::fs::read(path.extract::<PathBuf>()?)?;
        py.import("core.texture")?.call_method1(
            "cache_image",
            (PyBytes::new(py, &bytes), path.getattr("name")?, "imported"),
        )
    };
    let cached = match cache() {
        Ok(p) => p,
        Err(e) => {
            dialogs.call_method1(
                "critical",
                (
                    win,
                    translated(py, "Import image failed", None)?,
                    e.value(py).str()?,
                ),
            )?;
            return Ok(());
        }
    };
    let tool = py.import("tools.image")?.call_method0("ImageTool")?;
    let kw = PyDict::new(py);
    kw.set_item("label", path.getattr("stem")?)?;
    tool.call_method("load", (cached.str()?, aspect), Some(&kw))?;
    activate(
        win,
        &tool,
        "Image",
        "Click a corner and drag to size the image, or type a width and press Enter (Esc cancels)",
        6000,
    )
}
#[pyfunction]
fn pick_mesh_file(win: &Bound<'_, PyAny>, stl: bool) -> PyResult<()> {
    let (title, filter) = if stl {
        ("Import STL", "STL mesh (*.stl);;All files (*)")
    } else {
        ("Import OBJ", "Wavefront OBJ (*.obj);;All files (*)")
    };
    if let Some(path) = file_picker(win, "getOpenFileName", title, "", filter)? {
        win.call_method1("_mesh_import_options", (path,))?;
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(win,path,open_tab=None))]
fn ask_mesh_units(
    win: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    open_tab: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<String>> {
    let py = win.py();
    let doc = win.call_method0("active_document")?;
    let stl = path
        .getattr("suffix")?
        .extract::<String>()?
        .eq_ignore_ascii_case(".stl");
    let model = py.import("ingetrazo_render")?.getattr("OperationForm")?;
    let model = if stl {
        let store = py.import("ingetrazo_core.settings")?.call_method0("settings")?;
        model.call_method1(
            "mesh_import",
            (
                store
                    .call_method1("value", ("import/stl_unit", "mm"))?
                    .str()?,
                store
                    .call_method1("value", ("import/stl_simplify_mode", "principal"))?
                    .str()?,
            ),
        )?
    } else {
        model.call_method1(
            "mesh_import",
            (py.import("formats.obj")?
                .call_method1("suggest_unit", (path,))?,),
        )?
    };
    let done = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(mesh_units_answer, py)?,
            win,
            doc,
            path,
            stl,
            open_tab,
        ),
    )?;
    let forms = py.import("ingetrazo_render.chrome_forms")?;
    let form = forms.call_method1(
        "OperationForm",
        (
            translated(py, if stl { "Import STL" } else { "Import OBJ" }, None)?,
            model,
            done,
        ),
    )?;
    forms.call_method1("show", (win, form))?.extract()
}
#[pyfunction]
fn mesh_units_answer(
    win: &Bound<'_, PyAny>,
    doc: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    stl: bool,
    open_tab: &Bound<'_, PyAny>,
    values: &Bound<'_, PyDict>,
) -> PyResult<()> {
    if !win.call_method0("active_document")?.is(doc) {
        return Ok(());
    }
    let py = win.py();
    let unit = values.as_any().get_item("unit")?;
    let partial = py.import("functools")?.getattr("partial")?;
    let load = if stl {
        let mode = values.as_any().get_item("simplify")?;
        let store = py.import("ingetrazo_core.settings")?.call_method0("settings")?;
        store.call_method1("setValue", ("import/stl_unit", &unit))?;
        store.call_method1("setValue", ("import/stl_simplify_mode", &mode))?;
        partial.call1((
            win.getattr("_apply_stl_import")?,
            path,
            py.import("formats.stl")?
                .getattr("STL_UNITS")?
                .get_item(unit)?,
            mode,
        ))?
    } else {
        partial.call1((
            win.getattr("_apply_obj_import")?,
            path,
            py.import("formats.obj")?
                .getattr("OBJ_UNITS")?
                .get_item(unit)?,
        ))?
    };
    if open_tab.is_none() {
        load.call0()?;
    } else {
        open_tab.call1((load,))?;
    }
    Ok(())
}
#[pyfunction]
fn choose_image_export(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let current = win.getattr("_current_path")?;
    let stem = if current.is_none() {
        "untitled".to_owned()
    } else {
        current.getattr("stem")?.extract()?
    };
    if let Some(path) = file_picker(
        win,
        "getSaveFileName",
        "Export Image",
        &format!("{stem}.png"),
        "PNG image (*.png);;JPEG image (*.jpg)",
    )? {
        win.call_method1("_export_image_to", (path,))?;
    }
    Ok(())
}
#[pyfunction]
fn image_export_form(win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let path = match path
        .getattr("suffix")?
        .extract::<String>()?
        .to_lowercase()
        .as_str()
    {
        ".png" | ".jpg" | ".jpeg" => path.clone(),
        _ => path.call_method1("with_suffix", (".png",))?,
    };
    let model = py
        .import("ingetrazo_render")?
        .getattr("OperationForm")?
        .call_method1("image", (path.getattr("suffix")?,))?;
    let done = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(image_export_answer, py)?,
            win,
            win.call_method0("_active_view")?,
            path,
        ),
    )?;
    let forms = py.import("ingetrazo_render.chrome_forms")?;
    forms.call_method1(
        "show",
        (
            win,
            forms.call_method1(
                "OperationForm",
                (translated(py, "Export Image", None)?, model, done),
            )?,
        ),
    )?;
    Ok(())
}
#[pyfunction]
fn image_export_answer(
    win: &Bound<'_, PyAny>,
    view: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    values: &Bound<'_, PyDict>,
) -> PyResult<()> {
    win.call_method("_save_export_image", (view, path), Some(values))?;
    Ok(())
}
#[pyfunction]
fn save_export_image(
    win: &Bound<'_, PyAny>,
    view: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    width: u32,
    transparent: bool,
) -> PyResult<()> {
    let py = win.py();
    let kw = PyDict::new(py);
    kw.set_item("sharp_shadows", true)?;
    kw.set_item("transparent", transparent)?;
    kw.set_item("crop_to_frame", true)?;
    let native = py.import("ingetrazo_render.shell_services")?.call_method0("available")?.is_truthy()?;
    let method = if native { "render_raster" } else { "render_image" };
    let image = view.call_method(method, (width,), Some(&kw))?;
    let ready = !image.is_none() && (native || !image.call_method0("isNull")?.is_truthy()?);
    if !ready || !image.call_method1("save", (path.str()?,))?.is_truthy()? {
        py.import("ingetrazo_render.chrome_dialogs")?.call_method1(
            "critical",
            (
                win,
                translated(py, "Export Image failed", None)?,
                translated(py, "Could not render or save the image.", None)?,
            ),
        )?;
    } else {
        let vars = PyDict::new(py);
        for (key, name) in [("w", "width"), ("h", "height")] {
            vars.set_item(key, if native { image.getattr(name)? } else { image.call_method0(name)? })?;
        }
        vars.set_item("name", path.getattr("name")?)?;
        win.call_method0("statusBar")?.call_method1(
            "showMessage",
            (
                translated(py, "Exported image {w}×{h} → {name}", Some(&vars))?,
                4000,
            ),
        )?;
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(win,image="person_billboard.png",height=1.75,name=None))]
fn place_figure(
    win: &Bound<'_, PyAny>,
    image: &str,
    height: f64,
    name: Option<&str>,
) -> PyResult<()> {
    let py = win.py();
    win.getattr("viewport")?.call_method0("end_group_edit")?;
    let group = win.call_method1("_make_billboard_person", (image, height, name))?;
    if group.is_none() {
        let args = PyDict::new(py);
        args.set_item("p", image)?;
        py.import("ingetrazo_render.chrome_dialogs")?.call_method1(
            "warning",
            (
                win,
                translated(py, "Insert component", None)?,
                translated(py, "Component file missing: {p}", Some(&args))?,
            ),
        )?;
    } else {
        win.call_method1("_start_place", (group,))?;
    }
    Ok(())
}

#[pyfunction]
fn choose_billboard(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    vp.call_method0("end_group_edit")?;
    let scene = vp.getattr("scene")?;
    let Some(path) = file_picker(
        win,
        "getOpenFileName",
        "Face-me image",
        "",
        "Images (*.png *.webp);;All files (*)",
    )?
    else {
        return Ok(());
    };
    let caption = translated(py, "Face-me image", None)?;
    let dialogs = py.import("ingetrazo_render.chrome_dialogs")?;
    let Some((w, h, alpha)) =
        super::image_assets::billboard_image(py, &path.extract::<PathBuf>()?)?
    else {
        dialogs.call_method1(
            "warning",
            (
                win,
                &caption,
                translated(py, "Could not read the image.", None)?,
            ),
        )?;
        return Ok(());
    };
    if !alpha {
        dialogs.call_method1("information",(win,&caption,translated(py,"The image has no transparency — it will show as a solid rectangle. A PNG with transparent background works best.",None)?))?;
    }
    let opts = PyDict::new(py);
    opts.set_item("suffix", "m")?;
    opts.set_item(
        "done",
        py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(billboard_answer, py)?,
                win,
                scene,
                &path,
                f64::from(w) / f64::from(h),
            ),
        )?,
    )?;
    py.import("ingetrazo_render.chrome_forms")?
        .getattr("ask_number")?
        .call(
            (
                win,
                &caption,
                translated(py, "Real height (m):", None)?,
                1.75,
                0.05,
                500.0,
                2,
            ),
            Some(&opts),
        )?;
    Ok(())
}
#[pyfunction]
fn billboard_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    aspect: f64,
    height: Option<f64>,
) -> PyResult<()> {
    let Some(height) = height.filter(|h| (0.05..=500.0).contains(h)) else {
        return Ok(());
    };
    if win.getattr("viewport")?.getattr("scene")?.is(scene) {
        let group = win.py().import("core.group")?.call_method1(
            "make_billboard_group",
            (path.str()?, height, path.getattr("stem")?, aspect),
        )?;
        win.call_method1("_start_place", (group,))?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(start_placement, m)?)?;
    m.add_function(wrap_pyfunction!(insert_document, m)?)?;
    m.add_function(wrap_pyfunction!(place_text, m)?)?;
    m.add_function(wrap_pyfunction!(place_figure, m)?)?;
    m.add_function(wrap_pyfunction!(choose_billboard, m)?)?;
    m.add_function(wrap_pyfunction!(import_picture, m)?)?;
    m.add_function(wrap_pyfunction!(pick_mesh_file, m)?)?;
    m.add_function(wrap_pyfunction!(ask_mesh_units, m)?)?;
    m.add_function(wrap_pyfunction!(choose_image_export, m)?)?;
    m.add_function(wrap_pyfunction!(image_export_form, m)?)?;
    m.add_function(wrap_pyfunction!(save_export_image, m)?)?;
    Ok(())
}
