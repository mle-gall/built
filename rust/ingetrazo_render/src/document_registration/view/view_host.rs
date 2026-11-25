// SPDX-License-Identifier: GPL-3.0-or-later
//! Host bindings for view_host.
use super::*;
fn widget_shim_width<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    Ok(if view.getattr("_host")?.is_truthy()? {
        view.getattr("_host")?.getattr("width")?
    } else {
        (1280).into_bound_py_any(py)?
    })
}

fn widget_shim_height<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    Ok(if view.getattr("_host")?.is_truthy()? {
        view.getattr("_host")?.getattr("height")?
    } else {
        (800).into_bound_py_any(py)?
    })
}

fn widget_shim_size<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_host")?;
    let view = a.get_item(0)?;
    module
        .getattr("QSize")?
        .call1((view.call_method0("width")?, view.call_method0("height")?))
}

fn widget_shim_rect<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_host")?;
    let view = a.get_item(0)?;
    module.getattr("QRect")?.call1((
        0,
        0,
        view.call_method0("width")?,
        view.call_method0("height")?,
    ))
}

fn widget_shim_device_pixel_ratio_f<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    Ok(if view.getattr("_host")?.is_truthy()? {
        py.get_type::<pyo3::types::PyFloat>()
            .into_any()
            .call1((view.getattr("_host")?.getattr("scale")?,))?
    } else {
        (2.0).into_bound_py_any(py)?
    })
}

fn widget_shim_device_pixel_ratio<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    view.call_method0("devicePixelRatioF")
}

fn widget_shim_map_to_global<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_host")?;
    let p = a.get_item(1)?;
    Ok(if p.is_instance(&module.getattr("QPointF")?)? {
        module.getattr("QPointF")?.call1((&p,))?
    } else {
        module.getattr("QPoint")?.call1((&p,))?
    })
}

fn widget_shim_map_from_global<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_host")?;
    let p = a.get_item(1)?;
    Ok(if p.is_instance(&module.getattr("QPointF")?)? {
        module.getattr("QPointF")?.call1((&p,))?
    } else {
        module.getattr("QPoint")?.call1((&p,))?
    })
}

fn widget_shim_map_to<'py>(a: &Args<'py>) -> Reply<'py> {
    let p = a.get_item(2)?;
    Ok(p.clone())
}

fn widget_shim_map_from<'py>(a: &Args<'py>) -> Reply<'py> {
    let p = a.get_item(2)?;
    Ok(p.clone())
}

fn widget_shim_window<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    Ok(if view.getattr("_host")?.is_truthy()? {
        view.getattr("_host")?.getattr("window")?
    } else {
        none(py)
    })
}

fn widget_shim_window_handle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(none(py))
}

fn widget_shim_parent_widget<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(none(py))
}

fn widget_shim_is_visible<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    view.getattr("_shim_visible")
}

fn widget_shim_is_hidden<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    (!(view.getattr("_shim_visible")?.is_truthy()?)).into_bound_py_any(py)
}

fn widget_shim_is_enabled<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    view.getattr("_enabled")
}

fn widget_shim_set_enabled<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let on = a.get_item(1)?;
    let view = a.get_item(0)?;
    view.setattr("_enabled", (on.is_truthy()?).into_bound_py_any(py)?)?;
    Ok(none(py))
}

fn widget_shim_updates_enabled<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    (true).into_bound_py_any(py)
}

fn widget_shim_under_mouse<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    (true).into_bound_py_any(py)
}

fn widget_shim_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    view.getattr("_cursor")
}

fn widget_shim_set_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let cursor = a.get_item(1)?;
    let view = a.get_item(0)?;
    view.setattr("_cursor", &cursor)?;
    if !view.getattr("_host")?.is_none() {
        view.getattr("_host")?
            .call_method1("cursor_changed", (&cursor,))?;
    }
    Ok(none(py))
}

fn widget_shim_unset_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_host")?;
    let view = a.get_item(0)?;
    view.call_method1(
        "setCursor",
        (module.getattr("Qt")?.getattr("ArrowCursor")?,),
    )?;
    Ok(none(py))
}

fn widget_shim_clear_focus<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    view.setattr("_focus", false)?;
    Ok(none(py))
}

fn widget_shim_has_focus<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    (true).into_bound_py_any(py)
}

fn widget_shim_set_attribute<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let attr = a.get_item(1)?;
    let on = a.get_item(2)?;
    let view = a.get_item(0)?;
    view.getattr("__dict__")?
        .call_method1("setdefault", ("_shim_attrs", PyDict::new(py).into_any()))?
        .set_item(&attr, (on.is_truthy()?).into_bound_py_any(py)?)?;
    Ok(none(py))
}

fn widget_shim_test_attribute<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_host")?;
    let attr = a.get_item(1)?;
    let view = a.get_item(0)?;
    if attr.eq(module.getattr("Qt")?.getattr("WA_SetCursor")?)? {
        return (!view
            .getattr("_cursor")?
            .is(&module.getattr("Qt")?.getattr("ArrowCursor")?))
        .into_bound_py_any(py);
    }
    view.getattr("__dict__")?
        .call_method1("get", ("_shim_attrs", PyDict::new(py).into_any()))?
        .call_method1("get", (&attr, false))
}

#[rustfmt::skip]
pub(super) const WIDGET_SHIM: &[Method] = &[
    ("width", Mode::Method, widget_shim_width, &["self"], "", ""),
    ("height", Mode::Method, widget_shim_height, &["self"], "", ""),
    ("size", Mode::Method, widget_shim_size, &["self"], "", ""),
    ("rect", Mode::Method, widget_shim_rect, &["self"], "", ""),
    ("devicePixelRatioF", Mode::Method, widget_shim_device_pixel_ratio_f, &["self"], "", ""),
    ("devicePixelRatio", Mode::Method, widget_shim_device_pixel_ratio, &["self"], "", ""),
    ("mapToGlobal", Mode::Method, widget_shim_map_to_global, &["self", "p"], "", ""),
    ("mapFromGlobal", Mode::Method, widget_shim_map_from_global, &["self", "p"], "", ""),
    ("mapTo", Mode::Method, widget_shim_map_to, &["self", "_other", "p"], "", ""),
    ("mapFrom", Mode::Method, widget_shim_map_from, &["self", "_other", "p"], "", ""),
    ("window", Mode::Method, widget_shim_window, &["self"], "", ""),
    ("windowHandle", Mode::Method, widget_shim_window_handle, &["self"], "", ""),
    ("parentWidget", Mode::Method, widget_shim_parent_widget, &["self"], "", ""),
    ("isVisible", Mode::Method, widget_shim_is_visible, &["self"], "", ""),
    ("isHidden", Mode::Method, widget_shim_is_hidden, &["self"], "", ""),
    ("isEnabled", Mode::Method, widget_shim_is_enabled, &["self"], "", ""),
    ("setEnabled", Mode::Method, widget_shim_set_enabled, &["self", "on"], "", ""),
    ("updatesEnabled", Mode::Method, widget_shim_updates_enabled, &["self"], "", ""),
    ("underMouse", Mode::Method, widget_shim_under_mouse, &["self"], "", ""),
    ("cursor", Mode::Method, widget_shim_cursor, &["self"], "", ""),
    ("setCursor", Mode::Method, widget_shim_set_cursor, &["self", "cursor"], "", ""),
    ("unsetCursor", Mode::Method, widget_shim_unset_cursor, &["self"], "", ""),
    ("clearFocus", Mode::Method, widget_shim_clear_focus, &["self"], "", ""),
    ("hasFocus", Mode::Method, widget_shim_has_focus, &["self"], "", ""),
    ("setAttribute", Mode::Method, widget_shim_set_attribute, &["self", "attr", "on=True"], "", ""),
    ("testAttribute", Mode::Method, widget_shim_test_attribute, &["self", "attr"], "", ""),
];
