// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[rustfmt::skip]
const CORE_INSTANCE: &[Entry] = &[
    ("_snap_context_of", "snap_context_of", &["vp", "place"], "", ""),
    ("_boxes_around", "boxes_around", &["keys", "x0", "y0", "x1", "y1", "px", "py", "owner_of", "scene"], "", ""),
    ("_lazy_edge_may_be_curve", "lazy_edge_may_be_curve", &["mesh", "a", "b", "tol=0.001"], "bool", ""),
    ("_snap_dropped_loose", "snap_dropped_loose", &["vp"], "", ""),
    ("_px_near", "px_near", &["x", "y", "ok", "px", "py", "thr", "pts=None", "seen=None"], "", ""),
    ("_hypot3_f32", "hypot3_f32", &["a", "kind='libstdc++'"], "", ""),
];

#[rustfmt::skip]
const RENDER_INSTANCE: &[Entry] = &[
    ("linework_slack_point", "linework_slack_point", &["camera", "world"], "", ""),
    ("_inst_proto_of", "inst_proto_of", &["mod", "base"], "", ""),
    ("_split_chunk_spans", "split_chunk_spans", &["chunk", "depth=0", "force=False"], "", ""),
    ("_drop_bake_store", "drop_bake_store", &["key", "ref"], "None", ""),
    ("_bind_bake_store", "bind_bake_store", &["vp"], "None", ""),
    ("_proto_wrapper", "proto_wrapper", &["vp", "mesh", "paint=None"], "", ""),
    ("_layer_look_sig", "layer_look_sig", &["scene", "style=None"], "tuple", ""),
    ("_with_front_tint", "with_front_tint", &["paint", "style"], "", ""),
    ("_paint_of", "paint_of", &["vp", "group"], "", ""),
    ("_cache_ver", "cache_ver", &["vp"], "", ""),
    ("_content_ver", "content_ver", &["vp"], "", ""),
    ("_batch_instances", "batch_hlr_instances", &["base", "xfs", "tri_parts", "hard_parts", "soft_parts", "n_parts"], "None", ""),
    ("_axes_vertices", "axes_vertices", &["spacing:float", "pos_len:float=100000.0"], "", ""),
    ("shown_guides", "shown_guides", &["scene"], "list", ""),
    ("_ray_triangle", "scalar_ray_triangle", &["origin:QVector3D", "direction:QVector3D", "v0:QVector3D", "v1:QVector3D", "v2:QVector3D"], "Optional[float]", ""),
    ("_point_to_segment_distance_2d", "point_segment_distance", &["p", "a", "b"], "float", ""),
    ("_face_geometry", "face_geometry", &["f", "memo", "triangulate"], "", ""),
];

pub(super) fn install(r: &Registrar, owner: &Obj) -> PyResult<()> {
    r.bindings.install(owner, &r.core, Mode::Function, CORE_INSTANCE)?;
    r.bindings.install(owner, &r.render, Mode::Function, RENDER_INSTANCE)?;
    Ok(())
}
