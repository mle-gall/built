//! Measurements for loose geometry in the selection inspector.
use pyo3::prelude::*;

type Rows = Vec<(String, String)>;

#[pyfunction]
pub fn inspector_totals(scene: &Bound<'_, PyAny>, selection: &Bound<'_, PyAny>) -> PyResult<Rows> {
    if selection.len()? > 20_000 {
        return Ok(vec![]);
    }
    let py = scene.py();
    let mesh = py.import("core.mesh")?;
    let face = mesh.getattr("Face")?;
    let edge = mesh.getattr("Edge")?;
    let net_area = py.import("core.bim")?.getattr("face_net_area")?;
    let units = py.import("core.units")?;
    let settings = scene.getattr("units")?;
    let mut rows = Rows::new();
    let single = selection.len()? == 1;
    let mut area = None;
    let mut length = None;
    for entity in selection.try_iter()? {
        let entity = entity?;
        if entity.is_instance(&face)? {
            if single {
                rows.push((
                    "Vertices".into(),
                    entity.getattr("vertices")?.len()?.to_string(),
                ));
                let attrs = entity.getattr("attrs")?;
                let texture = attrs.call_method1("get", ("texture",))?;
                let path = if texture.is_truthy()? {
                    texture
                        .call_method1("get", ("path",))?
                        .extract::<Option<String>>()?
                } else {
                    None
                };
                let material = if let Some(path) = path.filter(|p| !p.is_empty()) {
                    std::path::Path::new(&path)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else if let Some(rgb) = attrs
                    .call_method1("get", ("color",))?
                    .extract::<Option<Vec<f64>>>()?
                    .filter(|v| v.len() >= 3)
                {
                    let byte =
                        |i: usize| (rgb[i] * 255.0).round_ties_even().clamp(0.0, 255.0) as u8;
                    format!(
                        "\u{1d}swatch\u{1d}#{:02X}{:02X}{:02X}",
                        byte(0),
                        byte(1),
                        byte(2)
                    )
                } else {
                    "—".into()
                };
                rows.push(("Material".into(), material));
            }
            *area.get_or_insert(0.0) += net_area.call1((entity,))?.extract::<f64>()?;
        } else if entity.is_instance(&edge)? {
            let chord = entity.getattr("b")?.sub(entity.getattr("a")?)?;
            *length.get_or_insert(0.0) += chord.call_method0("length")?.extract::<f64>()?;
        }
    }
    for (label, formatter, value) in [
        ("Area", "fmt_area_with", area),
        ("Length", "fmt_len_with", length),
    ] {
        if let Some(value) = value {
            rows.push((
                label.into(),
                units
                    .call_method1(formatter, (value, &settings))?
                    .extract()?,
            ));
        }
    }
    Ok(rows)
}

#[pyfunction]
pub fn inspector_curve(
    scene: &Bound<'_, PyAny>,
    shape: &Bound<'_, PyAny>,
) -> PyResult<(String, Rows)> {
    let n = shape.getattr("segments")?.extract::<usize>()?;
    let closed = shape.getattr("closed")?.is_truthy()?;
    let title = if !closed {
        "Arc"
    } else if n < 12 {
        "Polygon"
    } else {
        "Circle"
    };
    let radius = shape.getattr("radius")?.extract::<f64>()?;
    let verts = shape.getattr("verts")?;
    let side = verts
        .get_item(1)?
        .getattr("position")?
        .sub(verts.get_item(0)?.getattr("position")?)?
        .call_method0("length")?
        .extract::<f64>()?;
    let settings = scene.getattr("units")?;
    let fmt = scene.py().import("core.units")?.getattr("fmt_len_with")?;
    let length = |v: f64| fmt.call1((v, &settings))?.extract::<String>();
    let count = if shape.getattr("flat")?.is_truthy()? {
        format!("\u{1d}edit\u{1d}segments\u{1d}{n}")
    } else {
        format!("{n} (not flat)")
    };
    let mut rows = vec![
        ((if closed { "Sides" } else { "Segments" }).into(), count),
        ("Radius".into(), length(radius)?),
    ];
    rows.push(match title {
        "Polygon" => ("Side".into(), length(side)?),
        "Circle" => ("Diameter".into(), length(2. * radius)?),
        _ => (
            "Angle".into(),
            format!(
                "{:.1}°",
                shape.getattr("sweep")?.extract::<f64>()?.to_degrees()
            ),
        ),
    });
    rows.push((
        (if closed { "Perimeter" } else { "Length" }).into(),
        length(side * n as f64)?,
    ));
    Ok((title.into(), rows))
}

#[pyfunction]
pub fn inspector_rename(vp: &Bound<'_, PyAny>, uid: &str, text: &str) -> PyResult<()> {
    let name = text.trim();
    let scene = vp.getattr("scene")?;
    let sel = scene.getattr("selection")?;
    if name.is_empty() || sel.len()? != 1 {
        return Ok(());
    }
    let group = sel.try_iter()?.next().transpose()?;
    let Some(group) = group else { return Ok(()) };
    if !group.is_instance(&vp.py().import("core.group")?.getattr("Group")?)?
        || group.getattr("uid")?.extract::<String>()? != uid
        || group.getattr("name")?.eq(name)?
    {
        return Ok(());
    }
    let verbs = vp.py().import("views.verbs")?;
    let kind = verbs.call_method1("kind_of", (&scene, &sel))?;
    if !super::verbs::verbs_name_editable(&kind.extract::<String>()?) {
        return Ok(());
    }
    let path = scene.call_method1("occurrence_path", (&group,))?;
    if !path.is_none() {
        vp.py()
            .import("core.protocol.commands")?
            .call_method1("rename_placement", (vp.getattr("history")?, path, name))?;
        vp.call_method0("update")?;
    }
    Ok(())
}
