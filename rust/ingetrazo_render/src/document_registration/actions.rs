// SPDX-License-Identifier: GPL-3.0-or-later
//! Adapt controller callbacks to the arguments expected by tools and extensions.
use super::*;
use crate::document_host::{host, none, opt};

type Args<'py> = Bound<'py, PyTuple>;
type Result<'py> = PyResult<Obj<'py>>;

fn pick_edge_mode<'py>(a: &Args<'py>, visible: bool, linework: bool) -> Result<'py> {
    let view = a.get_item(0)?;
    let options = PyDict::new(a.py());
    options.set_item("visible_only", visible)?;
    if linework {
        options.set_item("hidden", view.getattr("linework_hidden")?)?;
        options.set_item("with_distance", true)?;
    }
    view.call_method(
        "_pick_edge",
        (a.get_item(1)?, a.get_item(2)?),
        Some(&options),
    )
}
fn pick_edge<'py>(a: &Args<'py>) -> Result<'py> {
    pick_edge_mode(a, false, false)
}
fn pick_visible_edge<'py>(a: &Args<'py>) -> Result<'py> {
    pick_edge_mode(a, true, false)
}
fn pick_linework_edge<'py>(a: &Args<'py>) -> Result<'py> {
    pick_edge_mode(a, true, true)
}

fn face_context<'py>(
    a: &Args<'py>,
    source: &str,
    placement: bool,
    clear_missing: bool,
) -> Result<'py> {
    let py = a.py();
    let controller = host(py)?.getattr("DocumentInteraction")?;
    let result = controller.call_method1(
        "_face_any",
        (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, source),
    )?;
    // Host providers return a tuple. Extension overrides may return any iterable.
    let tuple = match result.cast::<PyTuple>() {
        Ok(tuple) => tuple.clone(),
        Err(_) => py.get_type::<PyTuple>().call1((result,))?.cast_into()?,
    };
    if tuple.len() != 4 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "expected four face hit fields",
        ));
    }
    let face = tuple.get_item(0)?;
    let context = if clear_missing && face.is_none() {
        none(py)
    } else if placement {
        tuple.get_item(2)?
    } else {
        tuple.get_item(1)?
    };
    Ok((face, context).into_pyobject(py)?.into_any())
}
fn pick_face_any<'py>(a: &Args<'py>) -> Result<'py> {
    face_context(a, "sel", false, false)
}
fn pick_face_placement<'py>(a: &Args<'py>) -> Result<'py> {
    face_context(a, "sel", true, true)
}
fn pick_face_inference<'py>(a: &Args<'py>) -> Result<'py> {
    face_context(a, "infer", true, false)
}

fn projection<'py>(a: &Args<'py>, operation: &str) -> Result<'py> {
    let view = a.get_item(0)?;
    let camera = view.getattr("camera")?;
    camera.call_method1(operation, a.get_slice(1, a.len()))?;
    view.call_method0("_sync_projection")?;
    view.call_method0("update")?;
    Ok(none(a.py()))
}
fn set_projection<'py>(a: &Args<'py>) -> Result<'py> {
    projection(a, "set_projection")
}
fn toggle_projection<'py>(a: &Args<'py>) -> Result<'py> {
    projection(a, "toggle_projection")
}
fn toggle_two_point<'py>(a: &Args<'py>) -> Result<'py> {
    projection(a, "toggle_two_point")
}

fn world_to_pixels<'py>(a: &Args<'py>) -> Result<'py> {
    let py = a.py();
    let dtype = py.get_type::<pyo3::types::PyFloat>();
    let rows = py
        .import("numpy")?
        .call_method1("asarray", (a.get_item(1)?, dtype))?;
    a.get_item(0)?
        .call_method1("_project_px", (rows.call_method1("reshape", (-1, 3))?,))
}
fn world_to_pixel<'py>(a: &Args<'py>) -> Result<'py> {
    a.get_item(0)?
        .call_method1("_world_to_pixel", (a.get_item(1)?,))
}
fn pick_image_plane<'py>(a: &Args<'py>) -> Result<'py> {
    let options = PyDict::new(a.py());
    options.set_item("selectable_only", true)?;
    a.get_item(0)?.call_method(
        "image_plane_at",
        (a.get_item(1)?, a.get_item(2)?),
        Some(&options),
    )
}
fn finish_image_placement<'py>(a: &Args<'py>) -> Result<'py> {
    let win = a.get_item(0)?.call_method0("window")?;
    if let Some(select) = win.getattr_opt("activate_select_tool")? {
        select.call0()?;
    }
    Ok(none(a.py()))
}
fn refresh_after_edit<'py>(a: &Args<'py>) -> Result<'py> {
    let view = a.get_item(0)?;
    view.getattr("scene")?.call_method0("bump_view")?;
    view.call_method0("notify_scene_changed")?;
    Ok(none(a.py()))
}
fn set_groups_preview_offset<'py>(a: &Args<'py>) -> Result<'py> {
    let view = a.get_item(0)?;
    view.setattr(
        "_preview_offset",
        host(a.py())?.call_method1("QVector3D", (a.get_item(1)?,))?,
    )?;
    view.setattr("_preview_matrix", a.py().None())?;
    view.call_method0("update")?;
    Ok(none(a.py()))
}
fn set_groups_preview_matrix<'py>(a: &Args<'py>) -> Result<'py> {
    let view = a.get_item(0)?;
    view.setattr(
        "_preview_matrix",
        host(a.py())?.call_method1("QMatrix4x4", (a.get_item(1)?,))?,
    )?;
    view.call_method0("update")?;
    Ok(none(a.py()))
}
fn linework_hidden<'py>(a: &Args<'py>) -> Result<'py> {
    let view = a.get_item(0)?;
    let point = host(a.py())?.call_method1(
        "linework_slack_point",
        (view.getattr("camera")?, a.get_item(1)?),
    )?;
    view.call_method1("_is_occluded", (point,))
}
fn linework_groups<'py>(a: &Args<'py>, around: bool) -> Result<'py> {
    a.py().import("ingetrazo_core")?.call_method1(
        "linework_groups",
        (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, around),
    )
}
fn pick_linework_groups<'py>(a: &Args<'py>) -> Result<'py> {
    linework_groups(a, false)
}
fn linework_groups_around<'py>(a: &Args<'py>) -> Result<'py> {
    linework_groups(a, true)
}

fn ray_near<'py>(a: &Args<'py>) -> Result<'py> {
    if let Some(native) = a.get_item(0)?.getattr_opt("_lazy_native")? {
        if !native.call0()?.is_none() {
            return Ok(PyString::intern(a.py(), "skip").into_any());
        }
    }
    Ok(("px", a.get_item(1)?, a.get_item(2)?)
        .into_pyobject(a.py())?
        .into_any())
}
fn face_hits<'py>(a: &Args<'py>, infer: bool) -> Result<'py> {
    a.py().import("ingetrazo_core")?.call_method1(
        "cached_face_hits",
        (
            a.get_item(0)?,
            a.get_item(1)?,
            a.get_item(2)?,
            a.get_item(3)?,
            infer,
        ),
    )
}
fn hover_hits<'py>(a: &Args<'py>) -> Result<'py> {
    face_hits(a, false)
}
fn infer_hits<'py>(a: &Args<'py>) -> Result<'py> {
    face_hits(a, true)
}
fn owner_of<'py>(a: &Args<'py>) -> Result<'py> {
    let group = a.get_item(1)?;
    let owner = opt(&group, "owner")?;
    Ok(if owner.is_truthy()? { owner } else { group })
}
fn work_plane_normal<'py>(a: &Args<'py>) -> Result<'py> {
    let plane = opt(&a.get_item(0)?, "_last_work_plane")?;
    if plane.is_none() {
        Ok(plane)
    } else {
        plane.get_item(1)
    }
}
fn rest_is_hidden<'py>(a: &Args<'py>) -> Result<'py> {
    let v = a.get_item(0)?;
    if v.getattr("scene")?.getattr("edit_group")?.is_none() {
        return Ok(false.into_pyobject(a.py())?.to_owned().into_any());
    }
    v.getattr("_edit_rest_mode")?
        .rich_compare("hide", pyo3::basic::CompareOp::Eq)
}
fn view_style<'py>(a: &Args<'py>) -> Result<'py> {
    let v = a.get_item(0)?;
    let style = opt(&v, "view_style")?;
    if style.is_truthy()? {
        Ok(style)
    } else {
        opt(&v.getattr("scene")?, "display_style")
    }
}
fn ctrl_tapped<'py>(a: &Args<'py>) -> Result<'py> {
    Ok(a.get_item(0)?
        .getattr_opt("_ctrl_alone")?
        .unwrap_or_else(|| {
            pyo3::types::PyBool::new(a.py(), false)
                .to_owned()
                .into_any()
        }))
}
fn visible_spans<'py>(a: &Args<'py>, always_vector: bool) -> Result<'py> {
    let spans = a.get_item(1)?;
    let planes = a.get_item(2)?;
    let vector = always_vector
        || (spans
            .len()?
            .into_pyobject(a.py())?
            .ge(host(a.py())?.getattr("_SPAN_VECTOR_MIN")?)?
            && planes.is_truthy()?);
    a.py().import("ingetrazo_core")?.call_method1(
        "visible_draw_spans",
        (a.get_item(0)?, spans, planes, a.get_item(3)?, vector),
    )
}
fn cull_spans<'py>(a: &Args<'py>) -> Result<'py> {
    visible_spans(a, false)
}
fn cull_spans_np<'py>(a: &Args<'py>) -> Result<'py> {
    visible_spans(a, true)
}

fn section_plane_frame<'py>(a: &Args<'py>) -> Result<'py> {
    let plane = a.get_item(1)?;
    let axes = host(a.py())?.call_method1("plane_axes", (plane.getattr("normal")?,))?;
    let bounds = a.get_item(0)?.getattr("scene")?.call_method0("bounds")?;
    a.py()
        .import("ingetrazo_core")?
        .call_method1("section_frame", (plane.getattr("point")?, axes, bounds))
}
fn mvp<'py>(v: &Obj<'py>) -> Result<'py> {
    let cam = v.getattr("camera")?;
    cam.call_method0("projection_matrix")?
        .mul(cam.call_method0("view_matrix")?)
}
fn clip_segment_front<'py>(a: &Args<'py>) -> Result<'py> {
    a.py().import("ingetrazo_core")?.call_method1(
        "clip_annotation_front",
        (mvp(&a.get_item(0)?)?, a.get_item(1)?, a.get_item(2)?),
    )
}
fn guide_snap_segment<'py>(a: &Args<'py>) -> Result<'py> {
    let g = a.get_item(1)?;
    let reach = a.py().import("ingetrazo_core.guide")?.getattr("GUIDE_HALF_LEN")?;
    a.py().import("ingetrazo_core")?.call_method1(
        "clip_snap_guide",
        (
            mvp(&a.get_item(0)?)?,
            g.getattr("point")?,
            g.getattr("direction")?,
            reach,
            a.get_item(2)?,
        ),
    )
}
fn clip_pixel_line<'py>(a: &Args<'py>) -> Result<'py> {
    let v = a.get_item(0)?;
    a.py().import("ingetrazo_core")?.call_method1(
        "clip_annotation_line",
        (
            a.get_item(1)?,
            a.get_item(2)?,
            v.call_method0("width")?,
            v.call_method0("height")?,
            a.get_item(3)?,
        ),
    )
}
fn project_samples<'py>(a: &Args<'py>) -> Result<'py> {
    let v = a.get_item(0)?;
    a.py()
        .import("ingetrazo_core")?
        .call_method1("annotation_samples", (&v, a.get_item(1)?, mvp(&v)?))
}
fn segment_to_pixels<'py>(a: &Args<'py>) -> Result<'py> {
    let v = a.get_item(0)?;
    a.py().import("ingetrazo_render")?.call_method1(
        "segment_to_pixels",
        (
            mvp(&v)?,
            a.get_item(1)?,
            a.get_item(2)?,
            v.call_method0("_clip_w")?,
            v.call_method0("width")?,
            v.call_method0("height")?,
        ),
    )
}
fn proto_base_chunk<'py>(a: &Args<'py>) -> Result<'py> {
    let wrapped = host(a.py())?.call_method1("_proto_wrapper", a)?;
    a.get_item(0)?.call_method1("_group_chunk", (wrapped,))
}
fn placement_bbox<'py>(a: &Args<'py>) -> Result<'py> {
    a.get_item(0)?
        .call_method1("_placement_frame", (a.get_item(1)?,))?
        .get_item(0)
}
fn shown_guides<'py>(a: &Args<'py>) -> Result<'py> {
    host(a.py())?.call_method1("shown_guides", (a.get_item(0)?.getattr("scene")?,))
}
fn faceme_dir<'py>(a: &Args<'py>) -> Result<'py> {
    a.py().import("ingetrazo_render")?.call_method1(
        "faceme_dir",
        (a.get_item(0)?.getattr("camera")?, a.get_item(1)?),
    )
}

#[rustfmt::skip]
pub(super) const METHODS: &[AdapterEntry] = &[
    ("_proto_base_chunk", proto_base_chunk, &["self", "mesh", "paint=None"], "", "The prototype's chunk in LOCAL coordinates (the same wrapper\n``_instance_chunk`` derives from). ``paint`` is the container\nmaterial the variant is baked in (issue #47): a painted instance\nhas its own bake of the prototype, unpainted siblings share the\nplain one."),
    ("_placement_bbox", placement_bbox, &["self", "g"], "", "World AABB of an instanced placement from its PROTOTYPE's local\nbox and the placement matrix — eight corners, no bake. Reading it\noff ``_group_chunk(g)`` baked the whole placement to world\ncoordinates only to take its box: on the model of issue #158 that\nwas 21 000 baked copies of geometry the instanced pass never uses."),
    ("pick_image_plane", pick_image_plane, &["self", "screen_x:float", "screen_y:float"], "", "Selection entry point: like :meth:`image_plane_at` but skipping\nlocked images, so a click on an aligned scan falls through to the\ngeometry being drawn on it."),
    ("finish_image_placement", finish_image_placement, &["self"], "None", "Called by the Image tool once a picture is placed: drop back to\nSelect, the classic behaviour after a one-shot placement."),
    ("refresh_after_edit", refresh_after_edit, &["self"], "None", "Refresh an externally executed command without discarding geometry.\n\nCommands must bump the scene version and use the mesh mutation APIs,\njust as interactive tools do. The renderer compares mesh, material\nand placement signatures and rebuilds only changed payloads. Keep\nreset_document_caches() for New/Open, not individual bridge edits.\nCall once after a batch, outside an active transform preview.\nThis does not create an undo step or mark a clean document dirty.\n"),
    ("set_groups_preview_offset", set_groups_preview_offset, &["self", "delta:QVector3D"], "None", ""),
    ("set_groups_preview_matrix", set_groups_preview_matrix, &["self", "m"], "None", "Full-matrix variant (Rotate/Scale previews): the scratch copies\ndraw through ``m`` — same zero-churn frame as the translation."),
    ("_faceme_dir", faceme_dir, &["self", "anchor:QVector3D"], "QVector3D", ""),
    ("section_plane_frame", section_plane_frame, &["self", "sp"], "list", ""),
    ("_clip_segment_front", clip_segment_front, &["self", "a:QVector3D", "b:QVector3D"], "Optional[tuple[QVector3D, QVector3D]]", ""),
    ("_guide_snap_segment", guide_snap_segment, &["self", "g", "margin:float=1.5"], "Optional[tuple[QVector3D, QVector3D]]", ""),
    ("_clip_pixel_line", clip_pixel_line, &["self", "p0:tuple[float, float]", "p1:tuple[float, float]", "margin:float=64.0"], "Optional[tuple[tuple[float, float], tuple[float, float]]]", ""),
    ("_project_samples", project_samples, &["self", "pts:list[QVector3D]"], "", ""),
    ("_work_plane_normal", work_plane_normal, &["self"], "Optional[QVector3D]", "The normal of the plane the last pixel→world used, or ``None``."),
    ("_segment_to_pixels", segment_to_pixels, &["self", "p0:QVector3D", "p1:QVector3D"], "", ""),
    ("world_to_pixel", world_to_pixel, &["self", "world:QVector3D"], "Optional[tuple[float, float]]", "Public API (extensions): world point (metres) → widget pixel\n``(x, y)`` in logical pixels, or ``None`` when the point is behind\nthe camera. The same projection the host's own overlays use."),
    ("world_to_pixels", world_to_pixels, &["self", "pts"], "", "Public API (extensions): an ``(N, 3)`` float array of world points\n(metres) → ``(px, py, in_front)`` NumPy arrays, vectorised — for\noverlays with thousands of points, where a Python loop over\n:meth:`world_to_pixel` would cost frames. Points with\n``in_front == False`` are behind the camera; skip them."),
    ("_visible_spans", cull_spans, &["self", "spans", "planes", "split=None"], "", ""),
    ("_visible_spans_np", cull_spans_np, &["self", "spans", "planes", "split=None"], "", ""),
    ("_ray_near", ray_near, &["self", "x:float", "y:float"], "", "What a ray pick at widget pixel ``(x, y)`` asks the pick index\nto bake: nothing with the engine, which answers for the lazy\nplacements itself (see _face_hits), the placements the ray meets\notherwise."),
    ("_hover_hits", hover_hits, &["self", "idx", "origin", "direction"], "", ""),
    ("_infer_hits", infer_hits, &["self", "idx", "origin", "direction"], "", ""),
    ("pick_edge", pick_edge, &["self", "screen_x:float", "screen_y:float"], "", "Return the edge closest to ``(screen_x, screen_y)`` within threshold."),
    ("pick_visible_edge", pick_visible_edge, &["self", "screen_x:float", "screen_y:float"], "", "The edge a CLICK means: like :meth:`pick_edge`, but an edge the\nuser cannot see never wins — a soft edge (the seams of a smooth\nsurface, which are not drawn) unless hidden geometry is shown, and\nan edge behind the model. The usual rule, and what made a\ncylinder's side unclickable (@pacaeiro, issue #71: the invisible\nseams, front and back, took up to 90 % of the side at normal zoom).\nTools that want the tube's seam as a reference keep pick_edge."),
    ("linework_hidden", linework_hidden, &["self", "world:QVector3D"], "bool", "Whether a line through ``world`` is hidden for the Select tool:\n:meth:`_is_occluded` with a little depth slack, so a line lying ON\na face — or a hair above or below it, where the drawn line\nz-fights the face (shown at one angle, gone at the next) — is\nstill the line the cursor is on. See :func:`linework_slack_point`."),
    ("pick_linework_edge", pick_linework_edge, &["self", "screen_x:float", "screen_y:float"], "", "``(edge, pixels)``: the loose edge the Select tool takes before a\nface under the cursor — the nearest one within the pick threshold\nthat :meth:`linework_hidden` does not hide — or ``(None, inf)``."),
    ("pick_linework_groups", pick_linework_groups, &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("linework_groups_around", linework_groups_around, &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("_shown_guides", shown_guides, &["self"], "list", ""),
    ("_view_style", view_style, &["self"], "", "The style this view shows outside the composer: its own when it\nhas one (one of several views, restyled there), else the scene's."),
    ("pick_face_any", pick_face_any, &["self", "screen_x:float", "screen_y:float"], "", "Front-most face under the cursor across the loose mesh **and** every\ngroup: returns ``(face, group_or_None)``. Same coplanar tiebreak as\n:meth:`pick_face` (the smallest of the overlapping faces wins). The\ngroup half is what lets a tool tell \"this face is inside a container\"\n— drawing takes its plane, Push/Pull refuses it until the group is\nopened. While a group IS open its own faces are the loose part and\ncome back with ``None``; the model around it is in the index for\nsnapping and occlusion only, never as an answer here (the snap\nengine's own reading is :meth:`pick_face_inference`).\n\nMemoised per cursor position and view: a hover asks up to three\ntimes (work plane, acquisition, on-face flag) for the same answer."),
    ("pick_face_inference", pick_face_inference, &["self", "screen_x:float", "screen_y:float"], "", "The face the SNAP ENGINE may read under the cursor as\n``(face, placement_or_None)``: selectable faces as\n:meth:`pick_face_placement`, plus the rest of the model drawn\naround an open group / component — faded, never pickable, but a\nreference the cursor may infer «On face» from, and the plane a\nplanar tool or a moved part lands on.\n\nPicking never uses this: a click outside the open context must not\nselect or edit what it cannot touch (see :meth:`pick_face_any`)."),
    ("pick_face_placement", pick_face_placement, &["self", "screen_x:float", "screen_y:float"], "", "Like :meth:`pick_face_any`, but the group half is the PLACEMENT\nthe face was hit through — the proxy whose composed matrix puts it\nin the world — not the top-level owner a click would select. What\nevery face-PLANE reading needs: the owner of a nested component\ncarries the container's matrix, so a rectangle on the pergola's\npost got the post's plane in the wrong place (Marco, 2026-09-14)."),
    ("_rest_is_hidden", rest_is_hidden, &["self"], "bool", "Whether the model outside the edited group is out of this frame."),
    ("_owner_of", owner_of, &["self", "group"], "", "The object a click on ``group`` must select: a nested placement\nproxy stands for the top-level group that owns it."),
    ("ctrl_tapped", ctrl_tapped, &["self"], "bool", "Whether the Ctrl being released was pressed ALONE — no other key\nin between, not even one a menu shortcut took (Ctrl+Z reaches us\nonly as a ShortcutOverride). Tools that toggle on a Ctrl tap ask\nthis on the release (#183)."),
    ("toggle_projection", toggle_projection, &["self"], "None", ""),
    ("toggle_two_point", toggle_two_point, &["self"], "None", ""),
    ("set_projection", set_projection, &["self", "mode:str"], "None", "Parallel, perspective or two-point (the View toolbar's buttons)."),
];
