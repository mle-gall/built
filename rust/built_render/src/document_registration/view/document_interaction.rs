// SPDX-License-Identifier: GPL-3.0-or-later
//! Host bindings for document_interaction.
use super::*;
fn document_interaction_sync_axes<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let _axes = py.import("built_core.axes")?.into_any();
    _axes.call_method1(
        "sync",
        (optional(&view.getattr("scene")?, "tool_drawing_frame")?,),
    )?;
    Ok(none(py))
}

fn document_interaction_format_dim_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let metres = a.get_item(0)?;
    let style = a.get_item(1)?;
    let format_length = py.import("built_core.units")?.getattr("format_length")?;
    format_length.call1((
        &metres,
        style.call_method1("get", ("units", "m"))?,
        py.get_type::<pyo3::types::PyInt>()
            .into_any()
            .call1((style.call_method1("get", ("decimals", 2))?,))?,
        (style
            .call_method1("get", ("show_unit", true))?
            .is_truthy()?)
        .into_bound_py_any(py)?,
    ))
}

fn document_interaction_chunk_cache_dir<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let texture_cache_root = py.import("built_core.texture")?.getattr("texture_cache_root")?;
    let d = texture_cache_root
        .call0()?
        .getattr("parent")?
        .div("chunks")?;
    d.call_method(
        "mkdir",
        (),
        Some(&fields!(py; "parents" => true, "exist_ok" => true)),
    )?;
    Ok(d.clone())
}

fn document_interaction_edit_rest_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    view.getattr("_edit_rest_mode")
}

fn document_interaction_edit_rest_fade<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let strength = match a.get_item(0)?.getattr_opt("_edit_rest_fade_strength")? {
        Some(value) => value,
        None => py
            .import("views.document_interaction")?
            .getattr("EDIT_REST_FADE")?,
    };
    py.get_type::<pyo3::types::PyFloat>().call1((strength,))
}

fn document_interaction_dwell_on<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let point = a.get_item(1)?;
    let view = a.get_item(0)?;
    view.getattr("inference")?
        .call_method1("_dwell_on", (&point,))?;
    Ok(none(py))
}

fn document_interaction_encourage_dwelt<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    view.getattr("inference")?
        .call_method0("_encourage_dwelt")?;
    Ok(none(py))
}

fn document_interaction_encourage_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let point = a.get_item(1)?;
    let view = a.get_item(0)?;
    view.getattr("inference")?
        .call_method1("encourage_point", (&point,))?;
    Ok(none(py))
}

fn document_interaction_axis_source_cue<'py>(a: &Args<'py>) -> Reply<'py> {
    let px_x = a.get_item(2)?;
    let px_y = a.get_item(3)?;
    let view = a.get_item(0)?;
    let snap = a.get_item(1)?;
    view.getattr("inference")?
        .call_method1("_axis_source_cue", (&snap, &px_x, &px_y))
}

fn document_interaction_extension_snap<'py>(a: &Args<'py>) -> Reply<'py> {
    let px_x = a.get_item(2)?;
    let px_y = a.get_item(3)?;
    let view = a.get_item(0)?;
    let snap = a.get_item(1)?;
    view.getattr("inference")?
        .call_method1("_extension_snap", (&snap, &px_x, &px_y))
}

fn document_interaction_handle_value_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.document_interaction")?;
    let ev = a.get_item(1)?;
    let view = a.get_item(0)?;
    module.getattr("_value_key")?.call1((&view, &ev))
}

fn document_interaction_parse_value_buffer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let buffer = a.get_item(0)?;
    let comma_lists = a.get_item(1)?;
    let parse_value_box = py.import("built_core.units")?.getattr("parse_value_box")?;
    parse_value_box.call1((&buffer, &comma_lists))
}

#[rustfmt::skip]
pub(super) const DOCUMENT_INTERACTION: &[Method] = &[
    ("_sync_axes", Mode::Method, document_interaction_sync_axes, &["self"], "None", "Make the drawing axes the current context's (issue #44) — the\nworld's at the top level, the open group's own inside it."),
    ("_format_dim_value", Mode::Static, document_interaction_format_dim_value, &["metres:float", "style:dict"], "str", "Format a length (metres) per the dimension style: unit + precision\n(see :mod:`built_core.units` — metric, decimal or fractional imperial),\nwith or without the unit symbol (#374)."),
    ("_chunk_cache_dir", Mode::Static, document_interaction_chunk_cache_dir, &[], "", ""),
    ("edit_rest_mode", Mode::Property, document_interaction_edit_rest_mode, &["self"], "str", "``normal`` / ``fade`` / ``hide`` — how the model outside the group\nbeing edited reads. See :data:`EDIT_REST_MODES`."),
    ("edit_rest_fade", Mode::Property, document_interaction_edit_rest_fade, &["self"], "float", "0..1 strength of the grey wash applied outside the edit context."),
    ("_dwell_on", Mode::Method, document_interaction_dwell_on, &["self", "point:Optional[QVector3D]"], "None", ""),
    ("_encourage_dwelt", Mode::Method, document_interaction_encourage_dwelt, &["self"], "None", ""),
    ("encourage_point", Mode::Method, document_interaction_encourage_point, &["self", "point:QVector3D"], "None", ""),
    ("_axis_source_cue", Mode::Method, document_interaction_axis_source_cue, &["self", "snap", "px_x:float", "px_y:float"], "", ""),
    ("_extension_snap", Mode::Method, document_interaction_extension_snap, &["self", "snap", "px_x:float", "px_y:float"], "", ""),
    ("_handle_value_key", Mode::Method, document_interaction_handle_value_key, &["self", "ev"], "bool", "Route a value-box key to its document tool or the camera lens."),
    ("_parse_value_buffer", Mode::Static, document_interaction_parse_value_buffer, &["buffer:str", "comma_lists:bool=False"], "", "The value box's text → a float, a 2-tuple ``(w, h)``, a 3-tuple\n``(dx, dy, dz)``, an array, radius, profile, segments or slope\ntuple, or None (``built_core.units.parse_value_box``: the grammar is\nRust's, ``rust/built_core/src/units.rs``). Each tool's\n``on_value`` takes the arity it understands. ``comma_lists``: a\ncomma separates fields when the entry has no ``;`` and no space\n(#152)."),
];

fn entity_owner<'py>(a: &Args<'py>) -> Reply<'py> {
    let entities = a.get_item(0)?;
    let i = a.get_item(1)?;
    let at = optional(&entities, "owner_at")?;
    Ok(if !at.is_none() {
        at.call1((&i,))?
    } else {
        entities.get_item(&i)?.get_item(1)?
    })
}

fn array_bake<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.document_interaction")?;
    Ok({
        let value = module
            .getattr("sys")?
            .getattr("modules")?
            .call_method1("get", ("views.array_bake",))?;
        if value.is_truthy()? {
            value
        } else {
            module
                .getattr("importlib")?
                .call_method1("import_module", ("views.array_bake",))?
        }
    })
}

fn qv_hypot3<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.document_interaction")?;
    module.getattr("_QV_HYPOT3")
}

fn box_edge_vertices<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.document_interaction")?;
    let corners = a.get_item(0)?;
    let box_edge_vertices = py.import("built_core")?.getattr("box_edge_vertices")?;
    let coords = module.getattr("array")?.call1(("f",))?;
    coords.call_method1(
        "frombytes",
        (py.import("builtins")?
            .getattr("memoryview")?
            .call1((box_edge_vertices.call1((&corners,))?,))?
            .call_method1("cast", ("B",))?,),
    )?;
    Ok(coords.clone())
}

fn guide_vertices<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.document_interaction")?;
    let centre = a.get_item(3)?;
    let coords: Obj<'py>;
    let eye = a.get_item(5)?;
    let guides = a.get_item(0)?;
    let reach = a.get_item(4)?;
    let selection = a.get_item(2)?;
    let spacing = a.get_item(1)?;
    let spans: Obj<'py>;
    let axes = py.import("built_core.axes")?.into_any();
    let guide_vertices = py.import("built_core")?.getattr("guide_vertices")?;
    {
        let [part0, part1] = unpack::<2>(&guide_vertices.call1((
            ({
                let value = guides.clone();
                if value.is_truthy()? {
                    value
                } else {
                    PyTuple::empty(py).into_any()
                }
            }),
            &spacing,
            ({
                let value = selection.clone();
                if value.is_truthy()? {
                    value
                } else {
                    PyTuple::empty(py).into_any()
                }
            }),
            &centre,
            &reach,
            &eye,
            (if axes.call_method0("is_world")?.is_truthy()? {
                none(py)
            } else {
                axes.call_method0("frame")?
            }),
        ))?)?;
        coords = part0;
        spans = part1;
    }
    let data = module.getattr("array")?.call1(("f",))?;
    data.call_method1(
        "frombytes",
        (py.import("builtins")?
            .getattr("memoryview")?
            .call1((&coords,))?
            .call_method1("cast", ("B",))?,),
    )?;
    (&data, &spans).into_bound_py_any(py)
}

fn parse_number<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tok = a.get_item(0)?;
    let _number = py.import("built_core.units")?.getattr("_number")?;
    _number.call1((&tok,))
}

fn parse_length_field<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let field = a.get_item(0)?;
    let parse_length_field = py.import("built_core.units")?.getattr("parse_length_field")?;
    parse_length_field.call1((&field,))
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_entity_owner", Mode::Function, entity_owner, &["entities", "i"], "", "The owner half of pick-index entity ``i``: a lazy piece of the index\nanswers it without building its faces (see views.array_bake)."),
    ("_array_bake", Mode::Function, array_bake, &[], "", "views.array_bake, which imports back here (its import statement ran\nper placement, per frame pass)."),
    ("_qv_hypot3", Mode::Function, qv_hypot3, &[], "", ""),
    ("_box_edge_vertices", Mode::Function, box_edge_vertices, &["corners"], "", ""),
    ("_guide_vertices", Mode::Function, guide_vertices, &["guides", "spacing:float", "selection=()", "centre=None", "reach:float=0.0", "eye=None"], "", ""),
    ("_parse_number", Mode::Function, parse_number, &["tok:str"], "", "``2``, ``2.5``, ``.5``, ``3/4`` (a fraction), ``1-1/2`` (a mixed\nnumber, the hyphen form) → float, else None — ``built_core.units`` reads it,\nrefusing a number too long for a float or too big for a model (#185)."),
    ("_parse_length_field", Mode::Function, parse_length_field, &["field:str"], "", "One typed length in metres: metric or imperial, the usual forms.\n\n``2`` (the document's unit, issue #33) · ``30cm`` · ``1500mm`` ·\n``1m20`` · ``2\"`` / ``2in`` · ``1'`` / ``1ft`` · ``1'6\"`` · ``3/4\"`` ·\n``1'3/4\"``. A leading minus is kept. Returns None when the field is not\na length. The reading itself lives in ``built_core.units.parse``, the one\nevery length field shares."),
];
