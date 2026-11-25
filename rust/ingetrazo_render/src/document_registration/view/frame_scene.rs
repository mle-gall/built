// SPDX-License-Identifier: GPL-3.0-or-later
//! What a resident frame needs of the scene besides its rows: the axes
//! uniform (`views.wgpu_axes`), the camera-facing figures
//! (`views.wgpu_billboards`), the style and sun payload
//! (`views.wgpu_style`) and the section cut (`views.wgpu_sections`)
//! (render-glue lane; from the Python modules of those names).
use super::*;
use numpy::{ndarray::ArrayView3, PyArray1, PyReadonlyArray3};
use pyo3::exceptions::PyValueError;
use pyo3::basic::CompareOp;
use pyo3::types::PySlice;

fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    optional(o, name)
}
fn truthy(o: &Obj<'_>, name: &str) -> PyResult<bool> {
    attr_flag(o, name, false)
}
fn id(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}
fn np<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("numpy")
}
fn all<'py>(py: Python<'py>) -> Bound<'py, PySlice> {
    PySlice::full(py)
}
fn span<'py>(py: Python<'py>, from: isize, to: isize) -> Bound<'py, PySlice> {
    PySlice::new(py, from, to, 1)
}
fn tuple_of<'py>(value: &Obj<'py>) -> Reply<'py> {
    value.py().get_type::<PyTuple>().call1((value,))
}
fn number(value: Obj<'_>) -> PyResult<f64> {
    value.py().get_type::<pyo3::types::PyFloat>().call1((value,))?.extract()
}

// ---- views.wgpu_axes --------------------------------------------------------

/// Small axes uniform, independent of global core.axes state across
/// documents.
fn axes_state<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (scene, camera, source) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let origin: Vec<f64> = a.get_item(3)?.try_iter()?.map(|value| value?.extract()).collect::<PyResult<_>>()?;
    let frame = scene.getattr("drawing_frame")?;
    let mut visible = scene.getattr("show_axes")?.is_truthy()? || !frame.is_none();
    if !source.is_none() {
        visible = visible && source.getattr("plano_style")?.is_none() && source.getattr("style_override")?.is_none();
    }
    let frame = if frame.is_none() { py.import("ingetrazo_core")?.getattr("Matrix4")?.call0()? } else { frame };
    let mut matrix: Vec<f64> = frame.call_method0("data")?.try_iter()?.map(|value| value?.extract()).collect::<PyResult<_>>()?;
    for (value, shift) in matrix.iter_mut().skip(12).take(3).zip(&origin) {
        *value -= shift;
    }
    let size = (camera.getattr("distance")?.extract::<f64>()? * 0.008).max(1e-4);
    (PyTuple::new(py, matrix)?, size, visible).into_bound_py_any(py)
}

// ---- views.wgpu_billboards --------------------------------------------------

/// Map a sprite's horizontal extent and height to 0..1, on both sides.
fn fitted_maps<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (pos, corners, sizes, maps) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(4)?);
    let int = py.get_type::<pyo3::types::PyInt>();
    let first = if sizes.len()? != 0 { int.call1((sizes.get_item(0)?,))? } else { 0.into_bound_py_any(py)? };
    let numpy = np(py)?;
    let points = numpy.call_method1("asarray", (pos, numpy.getattr("float64")?))?.call_method1("reshape", (-1, 3))?;
    let indices = PyList::empty(py);
    for corner in corners.try_iter()? {
        indices.append(int.call1((corner?,))?)?;
    }
    let mapping = py.import("ingetrazo_render")?.call_method1("sprite_mapping", (points, indices, first))?;
    if !mapping.is_none() {
        maps.set_item(all(py), py.get_type::<PyList>().call1((mapping,))?.mul(2)?)?;
    }
    Ok(maps)
}

fn world_rows<'py>(packed: &Obj<'py>) -> Reply<'py> {
    packed.getattr("instances")?.get_item("world")
}

fn billboards_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (this, packed, candidates) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let numpy = np(py)?;
    this.setattr("packed", &packed)?;
    let info = packed.getattr("instances")?.get_item("info")?;
    let flags = info.get_item((all(py), 2))?;
    let ids = if candidates.is_none() {
        numpy.call_method1("flatnonzero", (flags.bitand(8)?,))?
    } else {
        candidates.get_item(flags.get_item(&candidates)?.bitand(8)?.rich_compare(0, CompareOp::Ne)?)?
    };
    let base = world_rows(&packed)?.get_item(&ids)?.call_method0("copy")?;
    this.setattr("ids", &ids)?;
    this.setattr("base", &base)?;
    this.setattr("key", py.None())?;
    let mut entries = PyList::empty(py).into_any();
    if ids.len()? != 0 {
        let commands = packed.getattr("commands")?;
        let int = py.get_type::<pyo3::types::PyInt>();
        let ranges = PyList::empty(py);
        for code in info.get_item((&ids, 0))?.try_iter()? {
            let command = commands.get_item(code?)?;
            ranges.append((int.call1((command.get_item("base")?,))?, int.call1((command.get_item("count")?,))?))?;
        }
        let contiguous = |array: Obj<'py>| numpy.call_method1("ascontiguousarray", (array,));
        entries = py.import("ingetrazo_render")?.call_method1("billboard_stances", (contiguous(packed.getattr("vertices")?.get_item("pos")?)?, ranges, contiguous(base)?))?;
    }
    this.setattr("entries", entries)?;
    let groups = packed.getattr("groups")?;
    let mesh = PyList::empty(py);
    for row in ids.try_iter()? {
        mesh.append(groups.get_item(row?)?.getattr("billboard")?.eq("mesh")?)?;
    }
    this.setattr("_mesh", mesh)?;
    Ok(none(py))
}

fn billboards_restore<'py>(a: &Args<'py>) -> Reply<'py> {
    let this = a.get_item(0)?;
    world_rows(&this.getattr("packed")?)?.set_item(this.getattr("ids")?, this.getattr("base")?)?;
    Ok(none(a.py()))
}

fn billboards_update<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (this, camera, renderer, inference) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let ids = this.getattr("ids")?;
    if ids.len()? == 0 {
        return Ok(none(py));
    }
    let eye = camera.call_method0("eye")?.call_method0("toTuple")?;
    let target = camera.getattr("target")?.call_method0("toTuple")?;
    let perspective = camera.getattr("perspective")?;
    let key = tuple_of(&eye)?.add(tuple_of(&target)?)?.add((&perspective,))?;
    if key.eq(this.getattr("key")?)? {
        return Ok(none(py));
    }
    this.setattr("key", key)?;
    let base = np(py)?.call_method1("ascontiguousarray", (this.getattr("base")?,))?;
    let matrices = py.import("ingetrazo_render")?.call_method1(
        "billboard_worlds",
        (base, this.getattr("entries")?, this.getattr("_mesh")?, eye, target, perspective.is_truthy()?),
    )?;
    let packed = this.getattr("packed")?;
    world_rows(&packed)?.set_item(&ids, &matrices)?;
    if !renderer.is_none() {
        renderer.call_method1("set_instance_world", (ids.call_method0("tolist")?, matrices.call_method1("reshape", (-1,))?))?;
    }
    if !inference.is_none() {
        inference.call_method1("update", (ids, packed))?;
    }
    Ok(none(py))
}

#[rustfmt::skip]
const BILLBOARDS: &[Method] = &[
    ("__init__", Mode::Method, billboards_init, &["self", "packed", "candidates=None"], "", ""),
    ("restore", Mode::Method, billboards_restore, &["self"], "", ""),
    ("update", Mode::Method, billboards_update, &["self", "camera", "renderer=None", "inference=None"], "", ""),
];

// ---- views.wgpu_style -------------------------------------------------------

/// Return a fresh native payload, respecting the viewport override.
///
/// `camera` is the view's camera: from below the ground the ground's
/// shadows are not drawn, as in GL (#1301). `soft_shadows` is the
/// Preferences level (#1281).
fn style_payload<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (scene, given, horizon, scale, fade, soft, camera) =
        (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?, a.get_item(4)?, a.get_item(5)?, a.get_item(6)?);
    let styles = py.import("core.style")?;
    let mut style = given.clone();
    if !style.is_truthy()? {
        style = attr(&scene, "display_style")?;
    }
    if !style.is_truthy()? {
        style = styles.getattr("Style")?.call0()?;
    }
    let result = style.call_method0("to_dict")?;
    let photo = attr(&scene, "photo_mesh")?;
    let no_sky = (truthy(&attr(&scene, "tile_layer")?, "visible")? && !attr(&scene, "georef")?.is_none())
        || truthy(&attr(&scene, "terrain")?, "visible")?
        || (truthy(&photo, "visible")? && scene.call_method1("entity_visible", (scene.getattr("photo_mesh")?,))?.is_truthy()?);
    if no_sky {
        result.set_item("sky", false)?;
    }
    result.set_item("fade", number(fade)?)?;
    result.set_item("fade_color", py.import("views.document_interaction")?.call_method1("edit_fade_color", (style.getattr("background")?,))?)?;
    result.set_item("back_color", styles.call_method1("effective_back_color", (&style, &scene))?)?;
    let occlusion = py.import("ingetrazo_render")?.call_method1("ao_settings", (&style,))?;
    result.set_item("ambient_occlusion", !occlusion.is_none())?;
    if !occlusion.is_none() {
        result.set_item("ao_distance", occlusion.get_item("radius")?)?;
        result.set_item("ao_intensity", occlusion.get_item("intensity")?)?;
        result.set_item("ao_samples", occlusion.get_item("samples")?)?;
    }
    result.set_item("horizon", number(horizon)?)?;
    result.set_item("profile_width", match style.getattr_opt("profile_width")? {
        Some(width) => number(width)?,
        None => 1.0,
    })?;
    result.set_item("dash_width", 4.0 * number(scale)?.max(1.0))?;
    // R3c-2a: GL keeps dot(normal, world - point) <= 0.
    let active = match scene.getattr_opt("active_section")? {
        Some(active) => active.call0()?,
        None => none(py),
    };
    let cuts = match scene.getattr_opt("show_section_cuts")? {
        Some(shown) => shown.is_truthy()?,
        None => true,
    };
    let mut plane = (0., 0., 0., 0.);
    if !active.is_none() && cuts {
        let axes = |vector: Obj<'py>| -> PyResult<[f64; 3]> {
            Ok([vector.call_method0("x")?.extract()?, vector.call_method0("y")?.extract()?, vector.call_method0("z")?.extract()?])
        };
        let (n, o) = (axes(active.getattr("normal")?)?, axes(active.getattr("point")?)?);
        plane = (-n[0], -n[1], -n[2], n[0] * o[0] + n[1] * o[1] + n[2] * o[2]);
    }
    result.set_item("section_plane", plane)?;
    result.set_item("material_opacity", true)?;
    result.set_item("group_selection_edges", true)?;
    result.set_item("show_hidden_geometry", truthy(&scene, "show_hidden_geometry")?)?;
    result.set_item("show_hidden_objects", truthy(&scene, "show_hidden_objects")?)?;
    let sun = py.import("views.wgpu_style")?.getattr("sun_payload")?.call((&scene,), Some(&fields!(py; "override" => given, "soft_shadows" => soft, "camera" => camera)))?;
    result.call_method1("update", (sun,))?;
    Ok(result)
}

/// Height of the GL shadow catcher (DocumentInteraction._draw_shadow_ground).
const SHADOW_CATCHER_Z: f64 = -0.005;

/// Same sun/date/north convention and GL light fit; no viewport required.
///
/// `soft_shadows` is the Preferences level (#1281, GL's
/// `DocumentInteraction._soft_shadows`); the payload carries its penumbra
/// spread, 0 for the crisp map. A `camera` that sees the ground from below
/// turns the ground's shadows off, like GL's catcher pass (#1301).
fn sun_payload<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (scene, given, soft, camera) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let sun = py.import("core.sun")?;
    let interaction_module = py.import("views.document_interaction")?;
    let interaction = interaction_module.getattr("DocumentInteraction")?;
    let result = fields!(py;
        "shadows" => false, "sun_direction" => (0., 0., 1.), "shadow_darkness" => 0.55, "ground_shadows" => true, "shadow_soft" => 0.0,
    )
    .into_any();
    let shadows = attr(&scene, "shadows")?;
    let name = attr(&given, "name")?;
    let muted = py.import("os")?.getattr("environ")?.call_method1("get", ("INGETRAZO_NO_SHADOWS",))?.is_truthy()?;
    if name.eq("tecnico")? || name.eq("lineas")? || muted || !truthy(&shadows, "enabled")? {
        return Ok(result);
    }
    let datum = attr(&scene, "georef")?;
    let (mut lat, mut lon) = (attr(&datum, "lat")?, attr(&datum, "lon")?);
    if lat.is_none() || lon.is_none() {
        (lat, lon) = (sun.getattr("DEFAULT_LAT")?, sun.getattr("DEFAULT_LON")?);
    }
    let year = py.import("datetime")?.getattr("date")?.call_method0("today")?.getattr("year")?;
    let direction = shadows
        .call_method("when_utc", (&lon,), Some(&fields!(py; "year" => year)))
        .and_then(|when| sun.call_method1("sun_direction", (&lat, &lon, when)));
    let mut direction = match direction {
        Ok(direction) => direction,
        Err(error) if error.is_instance_of::<PyValueError>(py) => return Ok(result),
        Err(error) => return Err(error),
    };
    if direction.is_none() {
        return Ok(result);
    }
    if !datum.is_none() && datum.getattr_opt("north")?.is_some_and(|north| north.is_truthy().unwrap_or(false)) {
        let [x, y] = unpack::<2>(&datum.call_method1("grid_to_local_xy", (direction.get_item(0)?, direction.get_item(1)?))?)?;
        direction = (x, y, direction.get_item(2)?).into_bound_py_any(py)?;
    }
    let [low, high] = unpack::<2>(&scene.call_method0("bounds")?)?;
    if low.is_none() {
        return Ok(result);
    }
    result.set_item("shadows", true)?;
    result.set_item("sun_direction", tuple_of(&direction)?)?;
    result.set_item("shadow_darkness", match shadows.getattr_opt("darkness")? {
        Some(darkness) => number(darkness)?,
        None => 0.55,
    })?;
    result.set_item("shadow_soft", number(interaction_module.getattr("SOFT_SHADOW_SPREAD")?.call_method1("get", (soft, 0.0))?)?)?;
    let light = interaction.call_method1("_light_vp", (&direction, low, high))?.call_method0("data")?;
    result.set_item("shadow_matrix", py.get_type::<PyList>().call1((light,))?)?;
    let reference = py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&fields!(py; "scene" => &scene)))?;
    let photo = attr(&scene, "photo_mesh")?;
    let covered = interaction.call_method1("_base_map_showing", (&reference,))?.is_truthy()?
        || interaction.call_method1("_terrain_showing", (&reference,))?.is_truthy()?
        || (!photo.is_none() && truthy(&photo, "visible")? && scene.call_method1("entity_visible", (&photo,))?.is_truthy()?)
        || (!camera.is_none() && camera.call_method1("sees_from_below", (py.import("views.wgpu_style")?.getattr("SHADOW_CATCHER_Z")?,))?.is_truthy()?);
    result.set_item("ground_shadows", !covered)?;
    Ok(result)
}

// ---- views.wgpu_sections ----------------------------------------------------

/// Resolve saved-frame visibility without baking or changing resident rows.
fn capture_rows<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (packed, scene, tint) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let core = py.import("ingetrazo_core")?;
    let state = core.getattr("PlacementState")?.call1((&scene,))?;
    let viewport = py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&fields!(py; "scene" => &scene)))?;
    let mut visible = std::collections::HashMap::new();
    for record in core.call_method1("placement_records", (&scene, scene.getattr("groups")?, viewport, state))?.try_iter()? {
        let record = record?;
        let flags: u32 = record.get_item(3)?.extract()?;
        visible.insert(id(&record.get_item(0)?), flags & 1 == 0);
    }
    let rows = packed.getattr("instances")?.call_method0("copy")?;
    let info = rows.get_item("info")?;
    let tinted = tint.is_truthy()?;
    for (row, (uid, group)) in packed.getattr("uids")?.try_iter()?.zip(packed.getattr("groups")?.try_iter()?).enumerate() {
        let (uid, group) = (uid?, group?);
        let shown = visible.get(&id(&group)).copied().unwrap_or(group.is_none());
        let figure = truthy(&group, "billboard")?;
        info.set_item((row, 2), (!shown) as u32 | if figure { 8 } else { 0 })?;
        let rgb = if tinted { tint.call_method1("get", (uid,))? } else { none(py) };
        let colour = if rgb.is_none() {
            0u32
        } else {
            let mut bytes = [0u8, 0, 0, 255];
            for (byte, level) in bytes.iter_mut().zip(rgb.get_item(span(py, 0, 3))?.try_iter()?) {
                *byte = (level?.extract::<f64>()? * 255.).clamp(0., 255.) as u8;
            }
            u32::from_le_bytes(bytes)
        };
        info.set_item((row, 3), colour)?;
    }
    Ok(rows)
}

/// The rows whose prototype box comes near the plane, `None` when the
/// arrays are not the pack's own (then every row is asked).
///
/// A first sieve in binary64 with a margin: the rows it keeps are then
/// tested as before, so which rows are cut does not change.
fn rows_near_plane<'py>(worlds: &Obj<'py>, bounds: &Obj<'py>, codes: &[i64], normal: &Obj<'py>, point: &Obj<'py>) -> PyResult<Option<Vec<bool>>> {
    let (Ok(worlds), Ok(bounds)) = (worlds.extract::<PyReadonlyArray3<'_, f32>>(), bounds.extract::<PyReadonlyArray3<'_, f32>>()) else {
        return Ok(None);
    };
    let (worlds, bounds) = (worlds.as_array(), bounds.as_array());
    if worlds.dim().0 != codes.len() || worlds.dim().1 != 4 || worlds.dim().2 != 4 || bounds.dim().1 != 2 || bounds.dim().2 < 3 {
        return Ok(None);
    }
    let (n, o): ([f64; 3], [f64; 3]) = (normal.extract()?, point.extract()?);
    let mut near = vec![true; codes.len()];
    for (row, code) in codes.iter().enumerate() {
        if *code < 0 || *code as usize >= bounds.dim().0 {
            continue;
        }
        let code = *code as usize;
        // Instance matrices are stored column by column: the transpose's
        // linear part is `w[j][i]`, its translation `w[3][i]`.
        let w = |j: usize, i: usize| worlds[[row, j, i]] as f64;
        let (mut distance, mut radius) = (0f64, 0f64);
        let mut scale = 0f64;
        for i in 0..3 {
            let mut centre = w(3, i);
            for j in 0..3 {
                centre += w(j, i) * (bounds[[code, 0, j]] as f64 + bounds[[code, 1, j]] as f64) * 0.5;
            }
            distance += n[i] * (centre - o[i]);
            scale += centre.abs() + o[i].abs();
        }
        for j in 0..3 {
            let along: f64 = (0..3).map(|i| n[i] * w(j, i)).sum();
            radius += along.abs() * (bounds[[code, 1, j]] as f64 - bounds[[code, 0, j]] as f64) * 0.5;
        }
        // Not a number: let the exact test decide.
        if distance.abs() > radius + 1e-4 * (radius + scale + 1.) {
            near[row] = false;
        }
    }
    Ok(Some(near))
}

/// The camera and the plane the cut's lines are widened for.
struct Lens {
    normal: [f64; 3],
    eye: [f64; 3],
    /// `tan(fov / 2)`.
    tangent: f64,
    height: f64,
}

/// The element type of a segment array, with NumPy's arithmetic in it.
trait Scalar: numpy::Element + Copy + std::ops::Add<Output = Self> + std::ops::Sub<Output = Self> + std::ops::Mul<Output = Self> + std::ops::Div<Output = Self> + PartialOrd {
    const HALF: Self;
    /// `1e-9` in this type.
    const TINY: Self;
    fn root(self) -> Self;
    fn wide(self) -> f64;
}
impl Scalar for f32 {
    const HALF: Self = 0.5;
    const TINY: Self = 1e-9;
    fn root(self) -> Self {
        self.sqrt()
    }
    fn wide(self) -> f64 {
        self as f64
    }
}
impl Scalar for f64 {
    const HALF: Self = 0.5;
    const TINY: Self = 1e-9;
    fn root(self) -> Self {
        self.sqrt()
    }
    fn wide(self) -> f64 {
        self
    }
}

fn push_vertex(out: &mut Vec<f32>, point: [f64; 3], rgb: [f64; 3]) {
    out.extend(point.iter().chain(&rgb).map(|value| *value as f32));
}

/// Each segment as a quad facing along the plane's normal, as wide as
/// `half_width` pixels at its middle and lifted off the cut by `lift`
/// widths: two triangles, position then colour per vertex.
fn quads<S: Scalar>(segments: ArrayView3<'_, S>, lens: &Lens, rgb: [f64; 3], half_width: f64, lift: f64, out: &mut Vec<f32>) {
    let n = lens.normal;
    let lifted = n.map(|axis| axis * lift);
    out.reserve(segments.dim().0 * 36);
    for segment in segments.outer_iter() {
        let a: [S; 3] = std::array::from_fn(|k| segment[[0, k]]);
        let b: [S; 3] = std::array::from_fn(|k| segment[[1, k]]);
        let t: [S; 3] = std::array::from_fn(|k| b[k] - a[k]);
        let length = ((t[0] * t[0] + t[1] * t[1]) + t[2] * t[2]).root();
        // NumPy's `length > 1e-9`: false for a NaN.
        if !(length > S::TINY) {
            continue;
        }
        let middle: [f64; 3] = std::array::from_fn(|k| ((a[k] + b[k]) * S::HALF).wide() - lens.eye[k]);
        let depth = ((middle[0] * middle[0] + middle[1] * middle[1]) + middle[2] * middle[2]).sqrt();
        let per_pixel = depth * lens.tangent * 2. / lens.height;
        // np.maximum(…, 1e-6): a NaN stays one.
        let per_pixel = if per_pixel.is_nan() || per_pixel > 1e-6 { per_pixel } else { 1e-6 };
        let width = per_pixel * half_width;
        let d: [f64; 3] = std::array::from_fn(|k| (t[k] / length).wide());
        let cross = [n[1] * d[2] - n[2] * d[1], n[2] * d[0] - n[0] * d[2], n[0] * d[1] - n[1] * d[0]];
        let side = cross.map(|axis| axis * width);
        let offset = lifted.map(|axis| axis * width);
        let (a, b) = (a.map(S::wide), b.map(S::wide));
        let q0: [f64; 3] = std::array::from_fn(|k| a[k] - side[k] + offset[k]);
        let q1: [f64; 3] = std::array::from_fn(|k| b[k] - side[k] + offset[k]);
        let q2: [f64; 3] = std::array::from_fn(|k| b[k] + side[k] + offset[k]);
        let q3: [f64; 3] = std::array::from_fn(|k| a[k] + side[k] + offset[k]);
        for corner in [q0, q1, q2, q0, q2, q3] {
            push_vertex(out, corner, rgb);
        }
    }
}

/// `(N, 2, 3)` segments as quads, in the precision the kernel gave them.
fn line_vertices(segments: &Obj<'_>, lens: &Lens, rgb: [f64; 3], half_width: f64, lift: f64, out: &mut Vec<f32>) -> PyResult<()> {
    if segments.is_none() || segments.len()? == 0 {
        return Ok(());
    }
    if let Ok(single) = segments.extract::<PyReadonlyArray3<'_, f32>>() {
        quads(single.as_array(), lens, rgb, half_width, lift, out);
        return Ok(());
    }
    let np = np(segments.py())?;
    let double: PyReadonlyArray3<'_, f64> = np.call_method1("asarray", (segments, np.getattr("float64")?))?.extract()?;
    quads(double.as_array(), lens, rgb, half_width, lift, out);
    Ok(())
}

/// The fill's triangles, position then colour per vertex.
fn fill_vertices(fill: &Obj<'_>, rgb: [f64; 3], out: &mut Vec<f32>) -> PyResult<()> {
    if fill.is_none() || fill.len()? == 0 {
        return Ok(());
    }
    let np = np(fill.py())?;
    let points: numpy::PyReadonlyArray2<'_, f64> = np.call_method1("asarray", (fill, np.getattr("float64")?))?.call_method1("reshape", (-1, 3))?.extract()?;
    let points = points.as_array();
    out.reserve(points.dim().0 * 6);
    for point in points.outer_iter() {
        push_vertex(out, [point[0], point[1], point[2]], rgb);
    }
    Ok(())
}

/// Cache intersections and triangulation; only line widths depend on
/// camera.
fn geometry<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, scene, style, camera, height, rows) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?, a.get_item(4)?, a.get_item(5)?);
    let numpy = np(py)?;
    let core = py.import("ingetrazo_core")?;
    let packed = view.getattr("packed")?;
    let rows = if rows.is_none() { packed.getattr("instances")? } else { rows };
    let plane = scene.call_method0("active_section")?;
    let empty = numpy.call_method1("empty", (0, numpy.getattr("uint8")?))?;
    let cuts = match scene.getattr_opt("show_section_cuts")? {
        Some(shown) => shown.is_truthy()?,
        None => true,
    };
    if plane.is_none() || !cuts {
        return Ok(empty);
    }
    let (normal, point) = (plane.getattr("normal")?, plane.getattr("point")?);
    let vector = |value: &Obj<'py>| numpy.call_method("array", (value.call_method0("toTuple")?,), Some(&fields!(py; "dtype" => py.get_type::<pyo3::types::PyFloat>())));
    let (n, o) = (vector(&normal)?, vector(&point)?);
    let info = rows.get_item("info")?;
    let flags = info.get_item((all(py), 2))?;
    let key = (id(&packed), scene.getattr("version")?, tuple_of(&n)?, tuple_of(&o)?, flags.call_method0("tobytes")?).into_bound_py_any(py)?;
    let mut cache = attr(&view, "_section_geometry_cache")?;
    if cache.is_none() || cache.get_item(0)?.ne(&key)? {
        let chunks = PyList::empty(py);
        let hidden: Vec<u32> = flags.extract()?;
        let codes: Vec<i64> = info.get_item((all(py), 0))?.extract()?;
        let worlds = rows.get_item("world")?;
        let (bounds, commands, positions) = (packed.getattr("bounds")?, packed.getattr("commands")?, packed.getattr("vertices")?.get_item("pos")?);
        let three = span(py, 0, 3);
        let near = rows_near_plane(&worlds, &bounds, &codes, &n, &o)?;
        for (row, (flags, code)) in hidden.iter().zip(&codes).enumerate() {
            if flags & 1 != 0 || near.as_ref().is_some_and(|near| !near[row]) {
                continue;
            }
            let world = worlds.get_item(row)?.getattr("T")?;
            let linear = world.get_item((&three, &three))?;
            let shift = world.get_item((&three, 3))?;
            let (low, high) = (bounds.get_item((*code, 0, &three))?, bounds.get_item((*code, 1, &three))?);
            let center = linear.matmul(low.add(&high)?.mul(0.5)?)?.add(&shift)?;
            let radius = numpy.call_method1("abs", (n.matmul(&linear)?,))?.matmul(high.sub(&low)?.mul(0.5)?)?;
            if n.matmul(center.sub(&o)?)?.abs()?.gt(radius)? {
                continue;
            }
            let command = commands.get_item(*code)?;
            let start: isize = command.get_item("base")?.call_method0("__int__")?.extract()?;
            let count: isize = command.get_item("count")?.call_method0("__int__")?.extract()?;
            let triangles = positions.get_item(span(py, start, start + count))?.call_method1("reshape", (-1, 3, 3))?.matmul(linear.getattr("T")?)?.add(&shift)?;
            let corner = |at: usize| triangles.get_item((all(py), at));
            let first = corner(0)?;
            let chords = core.call_method1("section_cut_chords", (&first, corner(1)?.sub(&first)?, corner(2)?.sub(&first)?, &normal, &point))?;
            if !chords.is_none() && chords.len()? != 0 {
                chunks.append(chords)?;
            }
        }
        let chords = if chunks.is_empty() { numpy.call_method1("empty", ((0, 2, 3),))? } else { numpy.call_method1("concatenate", (chunks,))? };
        let fill = core.call_method1("section_fill_triangles", (&chords, &normal))?;
        cache = (&key, chords, fill, PyDict::new(py)).into_bound_py_any(py)?;
        view.setattr("_section_geometry_cache", &cache)?;
    }
    let (chords, fill, hatches) = (cache.get_item(1)?, cache.get_item(2)?, cache.get_item(3)?);
    let [pattern, colour, spacing] = unpack::<3>(&core.call_method1("section_effective_fill", (&plane, &style))?)?;
    let eye_tuple = tuple_of(&camera.call_method0("eye")?.call_method0("toTuple")?)?;
    let field_of_view = camera.getattr("fov_deg")?;
    let face_mode = style.getattr("face_mode")?;
    let edge_colour = style.getattr("edge_color")?;
    let render_key = (&key, &pattern, &colour, &spacing, &face_mode, tuple_of(&edge_colour)?, &eye_tuple, &field_of_view, &height).into_bound_py_any(py)?;
    if attr(&view, "_section_render_key")?.eq(&render_key)? {
        return view.getattr("_section_render_data");
    }
    let eye: [f64; 3] = eye_tuple.extract()?;
    let plane_normal: [f64; 3] = n.extract()?;
    let fov: f64 = field_of_view.extract()?;
    let tall: f64 = height.extract()?;
    let lens = Lens { normal: plane_normal, eye, tangent: (fov.to_radians() * 0.5).tan(), height: tall.max(1.) };
    let rgb = |colour: &Obj<'py>| -> PyResult<[f64; 3]> { colour.get_item(span(py, 0, 3))?.extract() };
    let mut vertices: Vec<f32> = Vec::new();
    let solid_faces = face_mode.ne("xray")? && face_mode.ne("wireframe")?;
    if solid_faces && pattern.ne("none")? {
        let colour = rgb(&colour)?;
        let base = if pattern.eq("solid")? { colour } else { colour.map(|channel| channel + (1. - channel) * 0.78) };
        fill_vertices(&fill, base, &mut vertices)?;
        let cross = pattern.eq("cross")?;
        if cross || pattern.eq("hatch")? {
            let hatch_key = (&spacing, cross).into_bound_py_any(py)?;
            if !hatches.contains(&hatch_key)? {
                hatches.set_item(&hatch_key, core.call_method1("section_hatch_segments", (&chords, &normal, &spacing, cross))?)?;
            }
            line_vertices(&hatches.get_item(&hatch_key)?, &lens, colour, 0.5, 0.01, &mut vertices)?;
        }
    }
    line_vertices(&chords, &lens, rgb(&edge_colour)?, 1.25, 0.6, &mut vertices)?;
    let result = if vertices.is_empty() { empty } else { PyArray1::from_vec(py, vertices).into_any().call_method1("view", (numpy.getattr("uint8")?,))? };
    view.setattr("_section_render_key", render_key)?;
    view.setattr("_section_render_data", &result)?;
    Ok(result)
}

fn sections_sync<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let source = view.getattr("source_viewport")?;
    let scene = view.getattr("scene")?;
    let style = if source.is_none() {
        match scene.getattr_opt("display_style")? {
            Some(style) => style,
            None => py.import("core.style")?.getattr("Style")?.call0()?,
        }
    } else {
        source.call_method0("_effective_style")?
    };
    let data = py.import("views.wgpu_sections")?.call_method1("geometry", (&view, scene, style, view.getattr("camera")?, view.call_method0("height")?))?;
    if !data.is(&attr(&view, "_section_uploaded")?) {
        view.getattr("renderer")?.call_method1("set_section_geometry", (&data,))?;
        view.setattr("_section_uploaded", data)?;
    }
    Ok(none(py))
}

// ---- The modules ------------------------------------------------------------

fn functions(module: &Bound<PyModule>, functions: &[Method]) -> PyResult<()> {
    install(module.dict().as_any(), &module.dict(), functions)
}

pub(super) fn install_axes(module: &Bound<PyModule>) -> PyResult<()> {
    module.add("__doc__", "Small axes uniform, independent of global core.axes state across documents.")?;
    functions(module, &[("axes_state", Mode::Function, axes_state, &["scene", "camera", "source=None", "origin=(0.0, 0.0, 0.0)"], "", "")])
}

pub(super) fn install_billboards(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add(
        "__doc__",
        "Camera-facing resident placements; geometry and document transforms stay fixed.\n\nThe measuring and turning are the renderer's (billboards.rs); this keeps\nwhich placements are figures, their unturned matrices and the last camera.",
    )?;
    functions(module, &[("fitted_maps", Mode::Function, fitted_maps, &["pos", "corners", "sizes", "counts", "maps"], "", "")])?;
    let attrs = fields!(py; "__doc__" => "Cache world-space anchors/normals once per scene edit, O(figures) per orbit.");
    let cls = modules::class(module, "Billboards", &[], &attrs)?;
    install(&cls, &module.dict(), BILLBOARDS)
}

pub(super) fn install_style(module: &Bound<PyModule>) -> PyResult<()> {
    module.add(
        "__doc__",
        "Translate display styles without a GPU so both viewports share defaults.\n\nThe horizon is supplied by the reference viewport; camera ownership stays there.\nUnsupported controls remain explicit in the payload for subsequent passes.\n",
    )?;
    modules::import_names(module, "core.style", &[("Style", "Style"), ("effective_back_color", "effective_back_color")])?;
    module.add("SHADOW_CATCHER_Z", SHADOW_CATCHER_Z)?;
    functions(
        module,
        &[
            ("style_payload", Mode::Function, style_payload, &["scene", "override=None", "*", "horizon=2.0", "scale=1.0", "edit_rest_fade=0.0", "soft_shadows=0", "camera=None"], "", ""),
            ("sun_payload", Mode::Function, sun_payload, &["scene", "*", "override=None", "soft_shadows=0", "camera=None"], "", ""),
        ],
    )
}

pub(super) fn install_sections(module: &Bound<PyModule>) -> PyResult<()> {
    module.add("__doc__", "Resident section polygons using GL's closed-ring and hatch algorithms.")?;
    functions(
        module,
        &[
            ("capture_rows", Mode::Function, capture_rows, &["packed", "scene", "tint"], "", ""),
            ("geometry", Mode::Function, geometry, &["view", "scene", "style", "camera", "height", "rows=None"], "", ""),
            ("sync", Mode::Function, sections_sync, &["view"], "", ""),
        ],
    )
}
