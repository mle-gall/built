// SPDX-License-Identifier: GPL-3.0-or-later
//! Publish editable controller defaults and historical viewport module names.
use super::*;
use pyo3::types::PyFrozenSet;

pub(super) fn module(r: &Registrar, ns: &Bound<PyDict>) -> PyResult<()> {
    let py = ns.py();
    let env = py.import("os")?.getattr("environ")?;
    // Perf telemetry (INGETRAZO_PERF=1): every operation slower than 50 ms and a
    // once-per-second frame summary land in ~/ingetrazo-perf.log — the tool for
    // "it feels slow" reports from real sessions, where synthetic benchmarks lie.
    ns.set_item(
        "_PERF",
        env.call_method1("get", ("INGETRAZO_PERF",))?.is_truthy()?,
    )?;
    // Kill-switch for the instanced component pass (P2): set to 1 to draw
    // every instance through the consolidated VBOs like before.
    ns.set_item(
        "_NO_INSTANCING",
        env.call_method1("get", ("INGETRAZO_NO_INSTANCING", ""))?
            .eq("1")?,
    )?;
    // Kill-switch for the default-back tint pass (faces have sides, 2026-09-11):
    // set to 1 to draw every paint on both sides like before — a diagnostic,
    // and the escape hatch should a driver misbehave with front culling.
    ns.set_item(
        "_NO_BACK_TINT",
        env.call_method1("get", ("INGETRAZO_NO_BACK_TINT", ""))?
            .eq("1")?,
    )?;
    // Kill-switch for the snap engine's shortcuts (edge grid, occlusion in
    // batches, centre lookup through the registry): set to 1 to run every hover
    // the way it ran before — one pass over all group edges, one ray per
    // candidate, the rounded-position table. A diagnostic, and the escape hatch
    // should a snap ever land somewhere it did not before.
    ns.set_item(
        "_NO_SNAP_FAST",
        env.call_method1("get", ("INGETRAZO_NO_SNAP_FAST", ""))?
            .eq("1")?,
    )?;
    // Kill-switch for the pick index's span grid: set to 1 to keep one span per
    // chunk however big, as before (a ray that meets the box of a 107 k-triangle
    // group then tests all of its triangles).
    ns.set_item(
        "_NO_SPAN_GRID",
        env.call_method1("get", ("INGETRAZO_NO_SPAN_GRID", ""))?
            .eq("1")?,
    )?;
    // A chunk with more triangles than this is split into grid cells for the
    // ray prefilter (_split_chunk_spans); ~4 096 triangles a cell.
    ns.set_item("_SPAN_GRID_MIN", 8192)?;
    ns.set_item("_SPAN_CELL_TRIS", 4096)?;
    ns.set_item("_active_cut", r.core.getattr("active_cut")?)?;
    ns.set_item("_SnapEdge", r.render.getattr("SnapEdge")?)?;
    ns.set_item("_copy_snap_edge", r.render.getattr("copy_snap_edge")?)?;
    ns.set_item("_group_pseudo_edge", r.core.getattr("group_pseudo_edge")?)?;
    // The depth slack of a line the Select tool may take: a fraction of the
    // eye's distance to it, and never less than a millimetre. A line drawn on
    // a face, or a few millimetres under or over it, z-fights that face: the
    // renderers draw it at one angle and lose it at the next (Metal, 2 mm
    // under a face 9 m away: drawn from 45°, gone from above), and the strict
    // occlusion test made it unclickable — the object behind took the click.
    ns.set_item("LINEWORK_SLACK", 0.0005)?;
    ns.set_item("LINEWORK_SLACK_MIN", 0.001)?;
    // Beyond this many loose edges the snap engine gets a near-cursor subset
    // instead of the whole mesh: compute_snap walks its edge list in Python
    // ~a dozen times per hover, and an exploded medium import (9k faces =
    // ~20k edges) froze every mouse move (user report, piscina.igz).
    ns.set_item("_LOOSE_SNAP_CAP", 3000)?;
    ns.set_item("_PickWarm", r.render.getattr("PickWarm")?)?;
    ns.set_item("_pick_warm_take", r.render.getattr("pick_warm_take")?)?;
    // The snap engine's occlusion rays go through core._native's kernel when
    // the experimental engine is on (tests compare both).
    ns.set_item("_OCCL_NATIVE", true)?;
    ns.set_item("_occlusion_native", r.render.getattr("occlusion_native")?)?;
    // Lazy placements are filtered by core._native's spatial index before
    // being baked for a pick (the experimental engine; tests compare both).
    ns.set_item("_INSTANCE_FILTER", true)?;
    // The engine answers the corner and occlusion queries for the lazy
    // placements without baking them (tests compare both ways).
    ns.set_item("_LAZY_NATIVE", true)?;
    // A big document's pick index starts behind it as soon as it opens (see
    // DocumentInteraction.start_pick_warmup; the switch is for measuring and for tests).
    ns.set_item(
        "_PICK_WARM",
        !env.call_method1("get", ("INGETRAZO_NO_PICK_WARM", ""))?
            .eq("1")?,
    )?;
    // A pointer move over a still view reuses the model as the last frame
    // drew it (Viewport._frame_cache_key); INGETRAZO_FRAME_CACHE=0 turns it
    // off, and every frame draws everything as before.
    ns.set_item(
        "_FRAME_CACHE",
        !env.call_method1("get", ("INGETRAZO_FRAME_CACHE", "1"))?
            .eq("0")?,
    )?;
    // Occlusion answers kept a view (DocumentInteraction._occluded_many);
    // INGETRAZO_OCCLUSION_MEMO=0 asks every point again.
    ns.set_item(
        "_OCCLUSION_MEMO",
        !env.call_method1("get", ("INGETRAZO_OCCLUSION_MEMO", "1"))?
            .eq("0")?,
    )?;
    // The first corner in sight asked of the native engine in one call
    // (DocumentInteraction._first_clear); INGETRAZO_FIRST_CLEAR=0 asks in batches.
    ns.set_item(
        "_FIRST_CLEAR",
        !env.call_method1("get", ("INGETRAZO_FIRST_CLEAR", "1"))?
            .eq("0")?,
    )?;
    // Span lists this long are culled with NumPy (_visible_spans_np).
    ns.set_item("_SPAN_VECTOR_MIN", 64)?;
    ns.set_item(
        "_SHARED_BAKES",
        (
            "_group_chunks",
            "_inst_chunks",
            "_fp_memo",
            "_proto_wrappers",
        ),
    )?;
    ns.set_item("_bake_store_of", r.render.getattr("bake_store_of")?)?;
    ns.set_item(
        "_several_views_shown",
        r.render.getattr("several_views_shown")?,
    )?;
    ns.set_item("_DECODED_BUDGET", 256 * 1024 * 1024)?;
    ns.set_item("_decoded_texture", r.render.getattr("decoded_texture")?)?;
    ns.set_item("_fp_winding", r.render.getattr("fp_winding")?)?;
    ns.set_item("_flags_read", r.render.getattr("flags_read")?)?;
    // flat look.
    ns.set_item("_BY_LAYER_MODES", ("textures", "shaded", "xray"))?;
    // The face modes that draw the per-vertex colour, where the style's Front
    // color reaches the default faces through their bake. Hidden line and
    // Monochrome paint every face flat in it with a uniform instead.
    ns.set_item("_VCOLOR_MODES", ("textures", "shaded", "xray"))?;
    ns.set_item("_front_tint", r.render.getattr("front_tint")?)?;
    ns.set_item("_ghost_edge_points", r.render.getattr("ghost_edge_points")?)?;
    ns.set_item("_plan_curved_edges", r.core.getattr("plan_curved_edges")?)?;
    ns.set_item(
        "_nearest_within",
        r.core.getattr("document_nearest_within")?,
    )?;
    // The cursor's edge and the snap's read off one search in an event
    // (INGETRAZO_EDGES_SHARED=0: two searches, as before).
    ns.set_item(
        "_EDGES_SHARED",
        !env.call_method1("get", ("INGETRAZO_EDGES_SHARED", "1"))?
            .eq("0")?,
    )?;
    // The front face at a pixel without gathering every face the ray meets
    // (INGETRAZO_FRONT_LEAN=0: through _face_hits, as before).
    ns.set_item(
        "_FRONT_LEAN",
        !env.call_method1("get", ("INGETRAZO_FRONT_LEAN", "1"))?
            .eq("0")?,
    )?;
    // The nearest group edges picked in the native engine
    // (INGETRAZO_NEAREST=0: in NumPy, as before).
    ns.set_item(
        "_NEAREST_NATIVE",
        !env.call_method1("get", ("INGETRAZO_NEAREST", "1"))?
            .eq("0")?,
    )?;
    // The depth test of the corners near the cursor in the native engine
    // (INGETRAZO_NEAR_SEEN=0: in NumPy, as before).
    ns.set_item(
        "_NEAR_SEEN_NATIVE",
        !env.call_method1("get", ("INGETRAZO_NEAR_SEEN", "1"))?
            .eq("0")?,
    )?;
    ns.set_item("_unit_rays", r.core.getattr("document_unit_rays")?)?;
    // Barycentric roundoff on a shared triangle edge can reject both faces.
    ns.set_item("_BARY_EPS", 1e-14)?;
    ns.set_item("_occlusion_block", r.core.getattr("occlusion_block")?)?;
    ns.set_item("_placed_maps", r.render.getattr("placed_maps")?)?;
    ns.set_item("_cat_kept", r.render.getattr("cat_kept")?)?;
    ns.set_item("_offset_kept", r.render.getattr("offset_kept")?)?;
    ns.set_item("_moller_trumbore", r.core.getattr("ray_triangles")?)?;
    // How the rest of the model reads while a group is open for editing.
    // ``normal`` draws it as always, ``fade`` washes it out behind the group
    // being edited (the default), ``hide`` leaves it out of the frame.
    ns.set_item("EDIT_REST_MODES", ("normal", "fade", "hide"))?;
    // Default for how far the context is mixed TOWARD grey (0 = untouched, 1 =
    // fully washed). It is a wash, not transparency: the context keeps writing
    // depth, so it still occludes itself and never turns the model into glass.
    // The user-adjustable value lives on :class:`Viewport`; this constant remains
    // the default and the fallback for the small viewport stubs used by tools and
    // tests.
    ns.set_item("EDIT_REST_FADE", 0.75)?;
    ns.set_item("edit_fade_color", r.render.getattr("edit_fade_color")?)?;
    // X-ray: how opaque each translucent face layer is. Layers stack, so a face
    // seen through another reads darker and an opening (no face) stays clear.
    ns.set_item("XRAY_FACE_OPACITY", 0.6)?;
    // X-ray: edges hidden behind a face are washed this far toward the
    // background (0 = like a visible edge, 1 = gone), so the form still reads
    // and you can tell which edges have a face in front of them.
    ns.set_item("XRAY_HIDDEN_EDGE_FADE", 0.6)?;
    // Dot colour of a SELECTED face (the usual selection orange) and the one it
    // switches to on a surface that is itself orange/red, where orange dots
    // would vanish into the paint.
    ns.set_item("SELECTION_DOT_COLOR", (0.95, 0.45, 0.16))?;
    ns.set_item("SELECTION_DOT_ALT_COLOR", (0.1, 0.25, 0.85))?;
    ns.set_item(
        "selection_dot_color",
        r.render.getattr("selection_dot_color")?,
    )?;
    ns.set_item("_box_edges", r.render.getattr("box_edges")?)?;
    ns.set_item(
        "_load_edit_rest_mode",
        r.render.getattr("load_edit_rest_mode")?,
    )?;
    ns.set_item(
        "_load_edit_rest_fade",
        r.render.getattr("load_edit_rest_fade")?,
    )?;
    ns.set_item("_load_hide_similar", r.render.getattr("load_hide_similar")?)?;
    // Soft sun shadows (#1281), Preferences ▸ General: off (0) or a level, and
    // how fast the penumbra widens with the distance from what casts it, in
    // world width per world distance. The real sun is about 0.009; the levels
    // are broader, as a picture of soft daylight asks.
    let dict = PyDict::new(py);
    dict.set_item(1, 0.012)?;
    dict.set_item(2, 0.03)?;
    dict.set_item(3, 0.06)?;
    ns.set_item("SOFT_SHADOW_SPREAD", dict)?;
    ns.set_item("_load_soft_shadows", r.render.getattr("load_soft_shadows")?)?;
    let dict = PyDict::new(py);
    dict.set_item("x", (1.0, 0.0, 0.0))?;
    dict.set_item("y", (0.0, 1.0, 0.0))?;
    dict.set_item("z", (0.0, 0.0, 1.0))?;
    ns.set_item("_AXIS_DIRS", dict)?;
    ns.set_item("_finite_point", r.render.getattr("finite_point")?)?;
    ns.set_item("_curved_faces", r.render.getattr("curved_faces")?)?;
    ns.set_item("_smooth_vcol", r.render.getattr("smooth_vcol")?)?;
    Ok(())
}

pub(super) fn controller(r: &Registrar, cls: &Obj) -> PyResult<()> {
    let py = cls.py();
    // Tooltip text shown next to the snap marker. English source
    // strings; translated at draw time via ``tr`` (see i18n/es.json).
    // How close (px) the cursor must come to an acquired circle centre
    // for its green dot to show — beyond that the reference stays silent.
    // The real radius is a share of the circle's size on screen, capped
    // here (see ``_center_hint_px``).
    cls.setattr("CENTER_HINT_PX", 40.0)?;
    // A circle smaller than this on screen offers no centre at all.
    cls.setattr("CENTER_MIN_RADIUS_PX", 12.0)?;
    // How long the cursor must rest on a point to encourage it (ms).
    cls.setattr("ENCOURAGE_MS", 250)?;
    // Snap kinds whose point can be encouraged (a definite point, not a
    // place along an edge or on a face).
    cls.setattr(
        "_ENCOURAGING_KINDS",
        PyFrozenSet::new(
            py,
            [
                "endpoint",
                "midpoint",
                "arc_midpoint",
                "center",
                "component_origin",
                "origin",
                "intersection",
                "intersection_edge",
                "intersection_face",
                "close",
                "face_center",
            ],
        )?,
    )?;
    let dict = PyDict::new(py);
    dict.set_item("endpoint", "Endpoint")?;
    dict.set_item("midpoint", "Midpoint")?;
    dict.set_item("arc_midpoint", "Arc midpoint")?;
    dict.set_item("quadrant", "Quadrant")?;
    dict.set_item("component_origin", "Component origin")?;
    dict.set_item("on_line", "On line")?;
    dict.set_item("on_circle", "On circle")?;
    dict.set_item("on_axis", "On axis")?;
    dict.set_item("tangent", "Tangent")?;
    dict.set_item("on_edge", "On edge")?;
    dict.set_item("on_face", "On face")?;
    dict.set_item("face_center", "Face center")?;
    dict.set_item("origin", "Origin")?;
    dict.set_item("extension", "Extension")?;
    dict.set_item("intersection", "Intersection")?;
    dict.set_item("intersection_edge", "Intersect with Edge")?;
    dict.set_item("intersection_face", "Intersect with Face")?;
    dict.set_item("from_point", "From point")?;
    dict.set_item("aligned", "In line with point")?;
    dict.set_item("through_point", "Through point")?;
    dict.set_item("perp_face", "Perpendicular to face")?;
    dict.set_item("center", "Center")?;
    dict.set_item("guide_point", "Guide point")?;
    cls.setattr("_SNAP_LABELS", dict)?;
    // This picture's background is alpha 0 (``render_image``'s ``transparent``).
    cls.setattr("_export_transparent", false)?;
    // Off in a view of several (ingetrazo_render.DocumentViews) other than the one being
    // worked in: the model shows there, without the tool in hand's preview.
    cls.setattr("tool_preview", true)?;
    // How many viewports are inside ``paintGL`` right now (a view of
    // several paints the others from its own). What a paint announces must
    // not show or hide a widget beside the view: that resizes it in the
    // middle of its frame, and macOS crashed on the flush after (#1316).
    cls.setattr("painting", 0)?;
    // How many past epochs keep their parts for ``_epoch_only_matrices``.
    cls.setattr("_EPOCH_HISTORY", 4)?;
    // How far past the view the instanced batches reach, as a fraction of
    // its width and height, so an orbit keeps drawing the same ones
    // (_gather_instanced).
    cls.setattr("_INST_VIEW_MARGIN", 0.1)?;
    // ... and while they hold at most this many times what is in view.
    cls.setattr("_INST_SUPERSET_MAX", 2)?;
    // An instance whose bounding sphere spans less than this many
    // framebuffer pixels is not drawn (the camera's frame only).
    cls.setattr("_INST_MIN_PX", 1.0)?;
    // Per-document caches keyed by ``id()`` of the document's groups,
    // meshes and placements — the render/pick chunks and what hangs off
    // them. Nothing but the document boundary makes them all stale at once.
    cls.setattr(
        "_DOCUMENT_CACHES",
        PyTuple::new(
            py,
            [
                "_group_chunks",
                "_inst_chunks",
                "_fp_memo",
                "_proto_wrappers",
                "_proto_draw",
                "_hidden_edge_draws",
                "_faceme_cache",
                "_proto_pts_store",
                "_container_obb",
                "_placement_frames",
                "_pick_live",
                "_pick_foot_keep",
                "_billboard_world",
                "_placement_proxies",
                "_placement_tops",
                "_arc_mid_by_mesh",
                "_billboard_extents",
                "_figure_tokens",
            ],
        )?,
    )?;
    // Max NEW GL textures created per frame. Uploading hundreds at once (a big
    // capture) overwhelms Mesa and reads back as garbage (black/green tears at
    // the far edge); creating a few per frame spreads it — the map fills in over
    // a second and repaints itself until done.
    // P4: 6 uploads (each with mipmap generation, ~5-15 ms on the iGPU)
    // stacked into one frame was the ~100 ms hitch while zooming over the
    // base map. 2 per frame keeps every frame under the 33 ms gate; the
    // deferred-repaint loop below spreads the rest across frames.
    cls.setattr("_TEX_PER_FRAME", 2)?;
    // Sky (top) and ground (bottom) backdrop colours — subtle two-tone.
    cls.setattr("_SKY_RGB", (0.925, 0.935, 0.945))?;
    cls.setattr("_GROUND_RGB", (0.815, 0.82, 0.815))?;
    // NDC span over which each half of the backdrop completes its ramp.
    cls.setattr("_SKY_GRAD_SPAN", 1.4)?;
    cls.setattr("_GROUND_GRAD_SPAN", 0.9)?;
    cls.setattr("_SHADOW_MAP_SIZE", 2048)?;
    // The map an exported picture draws with (``render_image``'s
    // ``sharp_shadows``): drawn once per picture, so its cost does not
    // matter the way the view's does. Capped by the driver's texture size.
    cls.setattr("_SHADOW_EXPORT_SIZE", 8192)?;
    cls.setattr("_shadow_size", cls.getattr("_SHADOW_MAP_SIZE")?)?;
    // The .skp translucency rule: materials under 70 % opacity cast no
    // shadow — the sun passes through glass.
    cls.setattr("_SHADOW_OPACITY_MIN", 0.7)?;
    // How long the view must stay put before the fitted map is drawn (#958).
    cls.setattr("SHADOW_SETTLE_MS", 200)?;
    // A face this big (corners, holes included) is worth handing to the
    // native earcut ahead of ``_tris_of``.
    cls.setattr("NATIVE_TRIS_MIN_CORNERS", 64)?;
    // The sheet's cut pen (mm) a layer's own cut weight is measured
    // against in the model: a layer at 0.50 draws the usual ~2.5 px line.
    cls.setattr("CUT_PEN_MM", 0.5)?;
    // Screen pixels between the occlusion samples of a long annotation
    // line (``_draw_occluded_segment``), and the most samples it takes.
    cls.setattr("_OCCLUSION_STEP_PX", 12.0)?;
    cls.setattr("_OCCLUSION_MAX_SAMPLES", 128)?;
    // Where a line goes behind something, its sample step is cut in this
    // many parts this many times: 1/64 of a step, under a pixel.
    cls.setattr("_OCCLUSION_SPLIT", 8)?;
    cls.setattr("_OCCLUSION_ROUNDS", 2)?;
    // A ray meeting the work plane below this angle gives a point nobody
    // asked for. Measured: at 25 deg of camera pitch — an ordinary
    // architectural view — nothing on screen comes anywhere near it, so this
    // only fires where the plane really is unreadable.
    cls.setattr("GRAZING_PLANE_DEG", 6.0)?;
    // XY stays the ground in shallow views and where its cursor-ray hit is
    // reasonably close compared with the two vertical global planes.
    cls.setattr("GROUND_PREFERENCE_ANGLE_DEG", 5.0)?;
    cls.setattr("GROUND_DEPTH_PREFERENCE_RATIO", 1.25)?;
    cls.setattr("PERSPECTIVE_GROUND_DEPTH_PREFERENCE_RATIO", 10.0)?;
    cls.setattr("PLANE_ALIGNMENT_TIE_MARGIN", 0.05)?;
    // When the camera is at least this tilted off the horizon, the work plane
    // stays horizontal (XY) — that covers top, iso, and most architectural
    // angles. Only at near-horizon views does it switch to a vertical plane,
    // which is the only orientation where cursor-to-ground is ambiguous.
    cls.setattr("HORIZON_PITCH_THRESHOLD_DEG", 15.0)?;
    // How far from the cursor (px) a face crossing snaps, and how far
    // either side of the crossing the search checks that each face shows.
    cls.setattr("_CROSSING_RING_PX", 8.0)?;
    cls.setattr("_CROSSING_SIDE_PX", 3.0)?;
    // More selected faces than this and their centres are neither drawn
    // nor offered: a «select all» would bury the model in dots.
    cls.setattr("FACE_CENTER_SELECTION_MAX", 500)?;
    // Eye depth (m) a segment is cut at when it runs behind the camera.
    cls.setattr("_CLIP_W", 0.001)?;
    cls.setattr("_CHUNK_CACHE_MIN_FACES", 5000)?;
    cls.setattr("_CHUNK_CACHE_KEEP", 120)?;
    cls.setattr(
        "_CHUNK_CACHE_FIELDS",
        PyTuple::new(
            py,
            [
                "edges",
                "vcol",
                "smooth_vcol",
                "by_texture",
                "tcol",
                "ttex",
                "back_vcol",
                "back_tex",
                "back_tcol",
                "back_ttex",
                "fvcol",
                "dback",
                "areas",
                "v0",
                "e1",
                "e2",
                "tri_ent",
                "soft_pts",
                "soft_n0",
                "soft_c0",
                "soft_n1",
                "soft_c1",
                "soft_single",
                "edge_layer",
                "soft_layer",
                "bbox",
                "coordsum",
                "nv",
                "ne",
                "nf",
                "samples",
            ],
        )?,
    )?;
    // Component placements holding more faces than this, all together,
    // switch the pick index to LAZY placements (issue #158): below it the
    // index is exactly what it always was.
    cls.setattr("_PICK_LAZY_MIN_FACES", 1000000)?;
    // How many faces of lazy placements the index keeps baked at once.
    cls.setattr("_PICK_LIVE_MAX_FACES", 600000)?;
    // Lazy placements baked by one query at most (the nearest first) —
    // by all the footprint queries of one input event together.
    cls.setattr("_PICK_NEW_PER_QUERY", 300)?;
    // A snap's footprint skips placements under this many framebuffer
    // pixels across — the ones the frame does not draw.
    cls.setattr("_PICK_MIN_PX", 1.0)?;
    // Screen margin, in pixels, around the cursor for «near the cursor».
    cls.setattr("_PICK_NEAR_PX", 32.0)?;
    // The group-edge distances' contract with their two readers (the
    // hovered-edge pick, within ``pick_threshold_px``; the snap prefilter,
    // the ``GEDGE_ENOUGH`` nearest within 48 px): exact within
    // ``GEDGE_NEAR_PX``, and within ``GEDGE_REACH_PX`` too unless at least
    // ``GEDGE_ENOUGH`` edges already lie inside the near radius — then the
    // nearest ones are all there. Past that a big model says ``inf``.
    cls.setattr("GEDGE_NEAR_PX", 8.0)?;
    cls.setattr("GEDGE_REACH_PX", 64.0)?;
    cls.setattr("GEDGE_ENOUGH", 48)?;
    // The radius of the snap's own search of group edges near the cursor
    // (``_nearby_group_edges``'s default), which narrower ones in the
    // same event are read off (see _nearby_edges_unbaked).
    cls.setattr("GEDGE_SHARED_PX", 48.0)?;
    // From this many group edges up, the distances come through a screen
    // grid (core.edge_grid) instead of one pass over all of them.
    cls.setattr("GEDGE_GRID_MIN", 20000)?;
    // How close (px) the cursor must come to a component's origin or an
    // arc's midpoint for it to enter the snap scene at all.
    cls.setattr("POINT_INFERENCE_PX", 40.0)?;
    // Cap on rays × triangles in one NumPy block of ``_occluded_many``
    // (×3 float64 per array: ~50 MB at most).
    cls.setattr("_OCCL_BLOCK", 2000000)?;
    // -- Snapping inference, delegated to self.inference -----------------
    // The reference state (the hover / acquired edges and points, the centre
    // reference, the axis and reference locks, the last snap) lives in
    // core.inference.InferenceEngine, and so do the rules that used to be the
    // two post-processors of the search (the axis-source cue, the extension
    // hooks). These properties and thin wrappers keep the surface the tools,
    // the extensions, the probe and the tests read and write, so the
    // extraction stays behaviour-neutral.
    cls.setattr(
        "_acquired_point",
        r.render
            .call_method1("InferenceField", ("_acquired_point",))?,
    )?;
    cls.setattr(
        "_encouraged",
        r.render.call_method1("InferenceField", ("_encouraged",))?,
    )?;
    cls.setattr(
        "_dwell_point",
        r.render.call_method1("InferenceField", ("_dwell_point",))?,
    )?;
    cls.setattr(
        "_dwell_timer",
        r.render.call_method1("InferenceField", ("_dwell_timer",))?,
    )?;
    cls.setattr(
        "axis_lock",
        r.render.call_method1("InferenceField", ("axis_lock",))?,
    )?;
    cls.setattr(
        "last_snap",
        r.render.call_method1("InferenceField", ("last_snap",))?,
    )?;
    cls.setattr(
        "reference_edge",
        r.render
            .call_method1("InferenceField", ("reference_edge",))?,
    )?;
    cls.setattr(
        "reference_mode",
        r.render
            .call_method1("InferenceField", ("reference_mode",))?,
    )?;
    cls.setattr(
        "_shift_lock",
        r.render.call_method1("InferenceField", ("_shift_lock",))?,
    )?;
    cls.setattr(
        "_shift_lock_ref",
        r.render
            .call_method1("InferenceField", ("_shift_lock_ref",))?,
    )?;
    cls.setattr(
        "_hover_edge",
        r.render.call_method1("InferenceField", ("_hover_edge",))?,
    )?;
    cls.setattr(
        "_hover_center",
        r.render
            .call_method1("InferenceField", ("_hover_center",))?,
    )?;
    cls.setattr(
        "_center_ref",
        r.render.call_method1("InferenceField", ("_center_ref",))?,
    )?;
    cls.setattr(
        "_acquired_edge",
        r.render
            .call_method1("InferenceField", ("_acquired_edge",))?,
    )?;
    cls.setattr(
        "_acquired_face_normal",
        r.render
            .call_method1("InferenceField", ("_acquired_face_normal",))?,
    )?;
    // Below this many pixels of drag, a left press/release is a click, not a box.
    cls.setattr("BOX_DRAG_THRESHOLD_PX", 4.0)?;
    // The least a notch of the wheel backs out, from a close-up, once the
    // camera stands 2 cm or more from what it looks at — and always in an
    // empty scene, which has no size to go by.
    cls.setattr("_ZOOM_OUT_STEP", 0.05)?;
    // Inferences whose direction can be captured by a Shift lock.
    cls.setattr(
        "_SHIFT_LOCKABLE",
        PyFrozenSet::new(
            py,
            [
                "axis",
                "extension",
                "from_point",
                "reference",
                "axis_inference",
                "perp_face",
                "through_point",
            ],
        )?,
    )?;
    Ok(())
}
