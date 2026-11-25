// SPDX-License-Identifier: GPL-3.0-or-later
//! Expose the sheet painter's geometry on the editable record handles.
use super::*;
type Point = (f64, f64);
type Segment = (Point, Point);
fn segment((a, b): ([f64; 2], [f64; 2])) -> Segment {
    (a.into(), b.into())
}
pub(super) fn install_methods(name: &str, ns: &Bound<PyDict>) -> PyResult<()> {
    let py = ns.py();
    match name {
        "BarraEscala" => {
            methods!(ns,py;"segment_m"=>segment_m,"segment_mm"=>segment_mm);
            properties!(ns,py;"w_mm"=>bar_width,"h_mm"=>bar_height);
        }
        "FlechaNorte" => properties!(ns,py;"w_mm"=>size_mm,"h_mm"=>size_mm),
        "Leyenda" => properties!(ns,py;"h_mm"=>legend_height),
        "EtiquetaItem" => properties!(ns,py;"anchored"=>anchored,"h_mm"=>label_height),
        "LlamadaItem" => properties!(ns,py;"bubble_mm"=>bubble_mm),
        "NivelItem" => {
            properties!(ns,py;"anchored"=>anchored,"symbol_mm"=>symbol_mm);
            methods!(ns,py;"level_m"=>level_m,"value_text"=>level_text,"label"=>level_label);
        }
        "CotaAngularItem" => {
            methods!(ns,py;"angles"=>angles,"angle_deg"=>angle_deg,"auto_label"=>angle_label,"label"=>label);
            properties!(ns,py;"w_mm"=>angular_width,"h_mm"=>angular_height);
        }
        "CotaItem" => {
            properties!(ns,py;"anchored"=>anchored_pair,"w_mm"=>width,"h_mm"=>height);
            methods!(ns,py;"normal"=>normal,"line_points"=>linear_points,"label_extent_mm"=>label_extent_mm,"text_tail"=>text_tail,"measured_mm"=>measured_mm,"real_distance_m"=>real_distance_m,"auto_label"=>linear_label,"label"=>label);
        }
        "CotaRadialItem" => {
            properties!(ns,py;"w_mm"=>radial_size,"h_mm"=>radial_size);
            methods!(ns,py;"symbol"=>symbol,"measured_mm"=>radial_measured,"real_distance_m"=>real_distance_m,"heading"=>heading,"line_points"=>radial_points,"label_width_mm"=>label_width_mm,"outside"=>outside,"text_anchor"=>text_anchor,"tail_end"=>tail_end,"auto_label"=>radial_label,"label"=>label);
        }
        _ => {}
    }
    Ok(())
}
#[pyfunction]
fn segment_m(item: &Obj) -> PyResult<f64> {
    Ok(items::scalebar_segment_m(
        num(item, "segments")?,
        num(item, "scale_n")?,
    ))
}
#[pyfunction]
fn segment_mm(item: &Obj) -> PyResult<f64> {
    Ok(segment_m(item)? * 1000. / num(item, "scale_n")?)
}
#[pyfunction]
fn bar_width(item: &Obj) -> PyResult<f64> {
    Ok(segment_mm(item)? * num(item, "segments")?)
}
#[pyfunction]
fn bar_height(_item: &Obj) -> f64 {
    8.
}
#[pyfunction]
fn size_mm(item: &Obj) -> PyResult<f64> {
    num(item, "size_mm")
}
#[pyfunction]
fn legend_height(item: &Obj) -> PyResult<f64> {
    Ok(7.5 + 5.5 * item.getattr("rows")?.len()?.max(1) as f64)
}
#[pyfunction]
fn anchored(item: &Obj) -> PyResult<bool> {
    Ok(flag(item, "anchor_uid")? && flag(item, "a_world")?)
}
#[pyfunction]
fn anchored_pair(item: &Obj) -> PyResult<bool> {
    Ok(anchored(item)? && flag(item, "b_world")?)
}
#[pyfunction]
fn label_height(item: &Obj) -> PyResult<f64> {
    Ok((num(item, "size_pt")?
        * (25.4 / 72.)
        * 1.4
        * (string(item, "text")?.matches('\n').count() + 1) as f64)
        .max(6.))
}
#[pyfunction]
fn bubble_mm(item: &Obj) -> PyResult<f64> {
    Ok((num(item, "size_mm")? * 2.4).max(3.))
}
#[pyfunction]
fn symbol_mm(item: &Obj) -> PyResult<f64> {
    Ok((num(item, "size_mm")? * 1.2).max(1.5))
}
#[pyfunction]
fn level_m(item: &Obj) -> PyResult<f64> {
    let p = item.getattr("a_world")?;
    let z = if anchored(item)? && p.len()? > 2 {
        p.get_item(2)?.extract()?
    } else {
        num(item, "z_m")?
    };
    Ok(z - num(item, "datum_m")?)
}
fn unit(item: &Obj) -> PyResult<String> {
    let unit = string(item, "units")?;
    Ok(if unit.is_empty() { "m".into() } else { unit })
}
#[pyfunction]
fn level_text(item: &Obj) -> PyResult<String> {
    item.py()
        .import("core.units")?
        .call_method1(
            "format_signed_level",
            (level_m(item)?, unit(item)?, item.getattr("decimals")?),
        )?
        .extract()
}
#[pyfunction]
fn level_label(item: &Obj) -> PyResult<String> {
    let text = string(item, "text")?;
    let text = if text.is_empty() {
        "{z}".into()
    } else if text.contains("{z}") {
        text
    } else {
        format!("{} {{z}}", text.trim_end())
    };
    Ok(text.replace("{z}", &level_text(item)?))
}
#[pyfunction]
fn angles(item: &Obj) -> PyResult<Point> {
    Ok(items_b::angles(&fields(item)?))
}
#[pyfunction]
fn angle_deg(item: &Obj) -> PyResult<f64> {
    Ok(angles(item)?.1.to_degrees().abs())
}
#[pyfunction]
fn angle_label(item: &Obj) -> PyResult<String> {
    let n = num(item, "decimals")?.clamp(0., 4.) as usize;
    Ok(format!("{:.n$}°", angle_deg(item)?))
}
#[pyfunction]
fn label(item: &Obj) -> PyResult<String> {
    let s = string(item, "text")?;
    let auto = item.call_method0("auto_label")?.extract::<String>()?;
    Ok(if s.is_empty() {
        auto
    } else {
        s.replace("<>", &auto)
    })
}
#[pyfunction]
fn angular_width(item: &Obj) -> PyResult<f64> {
    Ok(items_b::angular_size(&fields(item)?).0)
}
#[pyfunction]
fn angular_height(item: &Obj) -> PyResult<f64> {
    Ok(items_b::angular_size(&fields(item)?).1)
}
#[pyfunction]
fn width(item: &Obj) -> PyResult<f64> {
    Ok(num(item, "dx_mm")?.abs().max(2.))
}
#[pyfunction]
fn height(item: &Obj) -> PyResult<f64> {
    Ok(num(item, "dy_mm")?.abs().max(2.))
}
#[pyfunction]
fn radial_size(item: &Obj) -> PyResult<f64> {
    Ok((2. * num(item, "radius_mm")?).max(2.))
}
macro_rules! geometry {
    ($name:ident,$ty:ident,$method:ident,$result:ty,$transform:expr,$label:expr) => {
        #[pyfunction]
        fn $name(item: &Obj) -> PyResult<$result> {
            let f = fields(item)?;
            let label = if $label {
                item.call_method0("label")?.extract()?
            } else {
                String::new()
            };
            Ok(($transform)(items_b::$ty { f: &f, label }.$method()))
        }
    };
}
geometry!(normal, Cota, normal, Point, std::convert::identity, false);
geometry!(linear_points, Cota, line_points, Segment, segment, false);
geometry!(
    label_extent_mm,
    Cota,
    label_extent_mm,
    f64,
    std::convert::identity,
    true
);
geometry!(
    text_tail,
    Cota,
    text_tail,
    Option<Segment>,
    |s: Option<_>| s.map(segment),
    true
);
geometry!(radial_points, Radial, line_points, Segment, segment, false);
geometry!(
    heading,
    Radial,
    heading,
    Point,
    std::convert::identity,
    false
);
geometry!(
    label_width_mm,
    Radial,
    label_width_mm,
    f64,
    std::convert::identity,
    true
);
geometry!(outside, Radial, outside, bool, std::convert::identity, true);
geometry!(
    text_anchor,
    Radial,
    text_anchor,
    Point,
    std::convert::identity,
    true
);
geometry!(
    tail_end,
    Radial,
    tail_end,
    Point,
    std::convert::identity,
    true
);
#[pyfunction]
pub(super) fn cota_line_deg(ct: &Obj) -> PyResult<f64> {
    let ((x, y), (u, v)): Segment = ct.call_method0("line_points")?.extract()?;
    Ok(items_b::readable_deg(u - x, v - y))
}
#[pyfunction]
fn measured_mm(item: &Obj) -> PyResult<f64> {
    let dx = num(item, "dx_mm")?;
    let dy = num(item, "dy_mm")?;
    Ok(match string(item, "axis")?.as_str() {
        "h" => dx.abs(),
        "v" => dy.abs(),
        _ => crate::sheets::pymath::hypot(dx, dy),
    })
}
#[pyfunction]
fn radial_measured(item: &Obj) -> PyResult<f64> {
    Ok(num(item, "radius_mm")?.abs()
        * if string(item, "kind")? == "diameter" {
            2.
        } else {
            1.
        })
}
#[pyfunction]
fn real_distance_m(item: &Obj) -> PyResult<f64> {
    Ok(item.call_method0("measured_mm")?.extract::<f64>()? * num(item, "scale_n")? / 1000.)
}
#[pyfunction]
fn linear_label(item: &Obj) -> PyResult<String> {
    let d = real_distance_m(item)?;
    let unit = unit(item)?;
    let show = flag(item, "show_unit")?;
    if unit == "m" && d >= 1000. && show {
        return Ok(format!("{:.3} km", d / 1000.));
    }
    item.py()
        .import("core.units")?
        .call_method1(
            "format_length",
            (d, unit, num(item, "decimals")?.clamp(0., 6.) as i32, show),
        )?
        .extract()
}
#[pyfunction]
fn symbol(item: &Obj) -> PyResult<&'static str> {
    Ok(if string(item, "kind")? == "diameter" {
        "Ø"
    } else {
        "R"
    })
}
#[pyfunction]
fn radial_label(item: &Obj) -> PyResult<String> {
    Ok(format!("{}{}", symbol(item)?, linear_label(item)?))
}
