// SPDX-License-Identifier: GPL-3.0-or-later
//! Hand-offs of a resident frame keyed to scene edits: hidden geometry and
//! the selected faces' dots (`views.wgpu_parity`), and the document's
//! display-only imagery (`views.wgpu_references`) (render-glue lane; from
//! the Python modules of those names).
use super::*;
use pyo3::basic::CompareOp;
use pyo3::exceptions::PyValueError;
use pyo3::types::PySlice;

fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    optional(o, name)
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
fn empty<'py>(py: Python<'py>) -> Bound<'py, PyList> {
    PyList::empty(py)
}
fn int(value: Obj<'_>) -> PyResult<isize> {
    value.call_method0("__int__")?.extract()
}
/// `np.any(array)`.
fn any(array: Obj<'_>) -> PyResult<bool> {
    np(array.py())?.call_method1("any", (array,))?.is_truthy()
}
/// `(alpha > 0) & (alpha < 255)`: some of it is see-through.
fn partly(alpha: &Obj<'_>) -> PyResult<bool> {
    any(alpha.rich_compare(0, CompareOp::Gt)?.bitand(alpha.rich_compare(255, CompareOp::Lt)?)?)
}

// ---- views.wgpu_parity ------------------------------------------------------

fn parity<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_parity")
}
fn bytes_of<'py>(array: impl IntoPyObject<'py>, py: Python<'py>) -> Reply<'py> {
    let numpy = np(py)?;
    numpy.call_method1("ascontiguousarray", (array,))?.call_method1("view", (numpy.getattr("uint8")?,))?.call_method1("reshape", (-1,))
}

fn raw<'py>(a: &Args<'py>) -> Reply<'py> {
    bytes_of(a.get_item(0)?, a.py())
}

fn has_transparency<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let packed = a.get_item(0)?;
    let numpy = np(py)?;
    let yes = || true.into_bound_py_any(py);
    if partly(&packed.getattr("vertices")?.get_item("color")?.get_item((all(py), 3))?)? {
        return yes();
    }
    // Back-side material RGBA records follow the prototype headers.
    let materials = if attr(&packed, "material_updates")?.is_truthy()? { packed.call_method0("material_stream")? } else { packed.getattr("materials")? };
    let headers = materials.get_item(PySlice::new(py, 0, packed.getattr("commands")?.len()? as isize * 16, 1))?.call_method1("view", ("<u4",))?.call_method1("reshape", (-1, 4))?;
    for header in headers.try_iter()? {
        let header = header?;
        if header.get_item(2)?.is_truthy()? {
            let (first, count) = (int(header.get_item(1)?)?, int(header.get_item(2)?)?);
            let records = materials.get_item(PySlice::new(py, first * 16, first * 16 + count * 32, 1))?.call_method1("reshape", (-1, 32))?;
            if partly(&records.get_item((all(py), 19))?)? {
                return yes();
            }
        }
    }
    let textures = packed.getattr("textures")?;
    if textures.is_truthy()? {
        let alpha = numpy.call_method1("frombuffer", (textures.get_item(3)?.get_item(0)?, numpy.getattr("uint8")?))?.get_item(PySlice::new(py, 3, isize::MAX, 4))?;
        if any(alpha.rich_compare(255, CompareOp::Lt)?)? {
            return yes();
        }
    }
    false.into_bound_py_any(py)
}

fn hidden_geometry<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let (renderer, packed) = (view.getattr("renderer")?, view.getattr("packed")?);
    if renderer.is_none() || packed.is_none() || !renderer.hasattr("set_hidden_geometry")? || attr(&packed, "hidden_commands")?.is_none() {
        return Ok(none(py));
    }
    let [geometry, objects_enabled] = unpack::<2>(&packed.getattr("reveal_hidden")?)?;
    let objects_enabled = objects_enabled.is_truthy()?;
    if !geometry.is_truthy()? && !objects_enabled {
        return Ok(none(py)); // set_scene cleared ghosts; feature-off packs do no expansion
    }
    let module = parity(py)?;
    let flags = packed.getattr("instances")?.get_item("info")?.get_item((all(py), 2))?;
    let objects = if objects_enabled {
        let hidden = flags.bitand(module.getattr("OBJECT_HIDDEN")?)?.rich_compare(0, CompareOp::Ne)?;
        let unfiltered = flags.bitand(module.getattr("VISIBILITY_FILTERED")?.bitor(128)?)?.rich_compare(0, CompareOp::Eq)?;
        np(py)?.call_method1("flatnonzero", (hidden.bitand(unfiltered)?,))?.call_method0("tolist")?
    } else {
        empty(py).into_any()
    };
    let part = |name: &str| bytes_of(packed.getattr(name)?, py);
    renderer.call_method1("set_hidden_geometry", (part("vertices")?, part("indices")?, part("hidden_commands")?, part("hidden_edge_commands")?, objects))?;
    Ok(none(py))
}

/// Resolve editable faces against the pack's stable visible-face order.
///
/// Only the active mesh is materialized. Face.triangulate and the resident
/// bake share core's triangulation kernel, including holes and degenerate
/// faces. Cache lives on the pack, so replacing geometry invalidates it
/// automatically.
fn face_ranges<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, faces) = (a.get_item(0)?, a.get_item(1)?);
    let ranges = empty(py);
    if !faces.is_truthy()? {
        return Ok(ranges.into_any());
    }
    let packed = view.getattr("packed")?;
    let scene = view.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let mut cache = attr(&packed, "_face_ranges")?;
    if cache.is_none() {
        cache = PyDict::new(py).into_any();
        packed.setattr("_face_ranges", &cache)?;
    }
    let key = id(&mesh);
    if !cache.contains(key)? {
        let definition = mesh.getattr_opt("_instance_definition")?.unwrap_or_else(|| mesh.clone());
        let info = packed.getattr("instances")?.get_item("info")?;
        let codes = PySet::empty(py)?;
        for (row, group) in packed.getattr("groups")?.try_iter()?.enumerate() {
            let group = group?;
            let own = if group.is_none() { scene.getattr("loose_mesh")? } else { group.getattr("mesh")? };
            if own.is(&definition) {
                codes.add(int(info.get_item((row, 0))?)?)?;
            }
        }
        let mapping = PyDict::new(py);
        let mut first = 0usize;
        for face in mesh.getattr("faces")?.try_iter()? {
            let face = face?;
            if face.getattr("attrs")?.call_method1("get", ("hidden",))?.is_truthy()? {
                continue;
            }
            let count = face.call_method0("triangulate")?.len()?;
            mapping.set_item(face.getattr("key")?, (first, count))?;
            first += count;
        }
        let codes = py.import("builtins")?.call_method1("sorted", (codes,))?;
        // Refuse an invalid range rather than suppress unrelated geometry.
        let commands = packed.getattr("commands")?;
        for code in codes.try_iter()? {
            if int(commands.get_item(code?)?.get_item("count")?)? != (first * 3) as isize {
                return Err(PyValueError::new_err("editable face triangulation differs from resident pack"));
            }
        }
        cache.set_item(key, (codes, mapping))?;
    }
    let [codes, mapping] = unpack::<2>(&cache.get_item(key)?)?;
    for face in faces.try_iter()? {
        let key = face?.getattr("key")?;
        if mapping.contains(&key)? {
            let [first, count] = unpack::<2>(&mapping.get_item(&key)?)?;
            for code in codes.try_iter()? {
                ranges.append((code?, &first, &count))?;
            }
        }
    }
    Ok(ranges.into_any())
}

/// The dot colour of `face` as seen from the view's camera, GL's orange
/// when the viewport does not choose.
fn dot_colour<'py>(view: &Obj<'py>, face: &Obj<'py>, normal: Obj<'py>, origin: Obj<'py>) -> Reply<'py> {
    let py = view.py();
    let choose = attr(&view.getattr("source_viewport")?, "_selection_dot_colors")?;
    if !choose.is_truthy()? {
        return (0.95, 0.45, 0.16).into_bound_py_any(py);
    }
    let [front, back] = unpack::<2>(&choose.call1((face,))?)?;
    let toward = view.getattr("camera")?.call_method0("eye")?.sub(origin)?;
    let facing = py.import("ingetrazo_core")?.getattr("Vec3")?.call_method1("dotProduct", (normal, toward))?.gt(0)?;
    Ok(if facing { front } else { back })
}

fn face_marks<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, faces) = (a.get_item(0)?, a.get_item(1)?);
    let numpy = np(py)?;
    let module = parity(py)?;
    let prepare = py.import("ingetrazo_render")?.getattr("prepare_face_marks")?;
    let packed = view.getattr("packed")?;
    let scene = view.getattr("scene")?;
    let zero = (0., 0., 0.).into_bound_py_any(py)?;
    let origins = if faces.is_truthy()? && scene.getattr("mesh")?.is(&attr(&scene, "loose_mesh")?) {
        packed.getattr_opt("prototype_origins")?.unwrap_or_else(|| PyTuple::empty(py).into_any())
    } else {
        PyTuple::empty(py).into_any()
    };
    let known = origins.len()?;
    let parts = empty(py);
    let three = PySlice::new(py, 0, 3, 1);
    for face in faces.try_iter()? {
        let face = face?;
        for range in module.call_method1("face_ranges", (&view, PyList::new(py, [&face])?))?.try_iter()? {
            let [code, first, count] = unpack::<3>(&range?)?;
            let (first, count) = (int(first)?, int(count)?);
            let command = packed.getattr("commands")?.get_item(&code)?;
            let start = int(command.get_item("first")?)?;
            let indices = packed.getattr("indices")?.get_item(PySlice::new(py, start + first * 3, start + (first + count) * 3, 1))?;
            let rows = indices.call_method1("astype", (numpy.getattr("int64")?,))?.add(int(command.get_item("base")?)?)?;
            let vertices = packed.getattr("vertices")?.get_item(rows)?.call_method0("copy")?;
            let colour = dot_colour(&view, &face, face.call_method0("normal")?, face.getattr("vertices")?.get_item(0)?)?;
            // Incremental prototypes already use the active coordinate frame.
            let anchored = (int(code.clone())? as usize) < known;
            let (origin, frame) = if anchored { (origins.get_item(&code)?, packed.getattr("origin")?) } else { (zero.clone(), zero.clone()) };
            prepare.call1((bytes_of(&vertices, py)?, colour.get_item(&three)?, origin, frame))?;
            parts.append(vertices)?;
        }
    }
    let marks = if parts.is_empty() { numpy.call_method1("empty", (0, module.getattr("VERTEX")?))? } else { numpy.call_method1("concatenate", (parts,))? };
    view.setattr("_selected_mark_count", marks.len()?)?;
    bytes_of(marks, py)
}

/// The dots GL puts on the face under the cursor of Push/Pull, Paint,
/// Offset, Follow Me... (`_paint_hover`): a selected face's dots, in the
/// colour picked against the side you see. The Metal view outlined the face
/// in cyan instead, which reads as a selected edge loop.
///
/// Built from the face's own triangulation through `placement`'s matrix (a
/// face inside a component is stored in its part's coordinates), not from
/// the pack, so it works on any placement and costs one face per hover
/// change.
fn hover_face_marks<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, face) = (a.get_item(0)?, a.get_item(1)?);
    let numpy = np(py)?;
    let vertex = parity(py)?.getattr("VERTEX")?;
    let matrix = attr(&a.get_item(2)?, "xform")?;
    let placed = |point: Obj<'py>| if matrix.is_none() { Ok(point) } else { matrix.call_method1("map", (point,)) };
    let mut points = Vec::new();
    for triangle in face.call_method0("triangulate")?.try_iter()? {
        for point in triangle?.try_iter()? {
            points.push(placed(point?)?);
        }
    }
    if points.is_empty() {
        return bytes_of(numpy.call_method1("empty", (0, vertex))?, py);
    }
    let (mut normal, mut origin) = (face.call_method0("normal")?, face.getattr("vertices")?.get_item(0)?);
    if !matrix.is_none() {
        (normal, origin) = (matrix.call_method1("mapVector", (normal,))?, matrix.call_method1("map", (origin,))?);
    }
    let colour = dot_colour(&view, &face, normal, origin)?;
    let mut rgba = [0u8, 0, 0, 255];
    for (byte, level) in rgba.iter_mut().zip(colour.get_item(PySlice::new(py, 0, 3, 1))?.try_iter()?) {
        *byte = (level?.extract::<f64>()? * 255.).clamp(0., 255.) as u8;
    }
    let rows = empty(py);
    for point in points {
        rows.append((point.call_method0("toTuple")?, (0, 0, 1), (rgba[0], rgba[1], rgba[2], rgba[3])))?;
    }
    bytes_of(numpy.call_method1("array", (rows, vertex))?, py)
}

pub(super) fn install_parity(module: &Bound<PyModule>) -> PyResult<()> {
    module.add("__doc__", "Resident parity handoffs. Work is keyed to scene edits, never screen pixels.")?;
    modules::import_names(
        module,
        "ingetrazo_core",
        &[("pack_tool_preview", "preview_mesh"), ("VERTEX", "VERTEX"), ("OBJECT_HIDDEN", "OBJECT_HIDDEN"), ("VISIBILITY_FILTERED", "VISIBILITY_FILTERED")],
    )?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("raw", Mode::Function, raw, &["array"], "", ""),
            ("has_transparency", Mode::Function, has_transparency, &["packed"], "", ""),
            ("hidden_geometry", Mode::Function, hidden_geometry, &["view"], "", ""),
            ("face_ranges", Mode::Function, face_ranges, &["view", "faces"], "", ""),
            ("face_marks", Mode::Function, face_marks, &["view", "faces"], "", ""),
            ("hover_face_marks", Mode::Function, hover_face_marks, &["view", "face", "placement=None"], "", ""),
        ],
    )
}

// ---- views.wgpu_references --------------------------------------------------

fn references<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_references")
}
fn visible_flag(o: &Obj<'_>) -> PyResult<bool> {
    Ok(!o.is_none() && o.getattr("visible")?.is_truthy()?)
}

fn vertices<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let numpy = np(py)?;
    let (mut positions, mut uvs) = (numpy.call_method1("asarray", (a.get_item(0)?,))?, numpy.call_method1("asarray", (a.get_item(1)?,))?);
    let triangles = a.get_item(3)?;
    if !triangles.is_none() {
        let indices = numpy.call_method1("asarray", (triangles,))?.call_method0("ravel")?;
        (positions, uvs) = (positions.get_item(&indices)?, uvs.get_item(&indices)?);
    }
    numpy.call_method1("column_stack", ((positions.sub(a.get_item(2)?)?, uvs),))?.call_method1("astype", (numpy.getattr("float32")?,))?.call_method0("ravel")
}

fn texture<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let numpy = np(py)?;
    let (fallback, limit) = (a.get_item(1)?, a.get_item(2)?);
    let image = py.import("ingetrazo_core")?.call_method1("as_raster", (a.get_item(0)?,))?;
    let bytes = numpy.getattr("uint8")?;
    if image.is_none() {
        let colour = py.get_type::<PyTuple>().call1((fallback,))?.add((255,))?;
        return (1, 1, numpy.call_method1("array", (colour, bytes))?).into_bound_py_any(py);
    }
    let rgba = image.call_method0("rgba")?;
    let (width, height) = (image.getattr("width")?, image.getattr("height")?);
    if py.import("builtins")?.call_method1("max", (&width, &height))?.le(&limit)? {
        return (width, height, numpy.call_method1("frombuffer", (rgba, bytes))?).into_bound_py_any(py);
    }
    references(py)?.call_method1("fit_reference_rgba", (width, height, rgba, limit))
}

/// What one sync gathers: the layers showing and the uploads they need.
struct Layers<'py> {
    py: Python<'py>,
    module: Bound<'py, PyModule>,
    renderer: Obj<'py>,
    old: Obj<'py>,
    layers: Bound<'py, PyDict>,
    updates: Bound<'py, PyList>,
}

impl<'py> Layers<'py> {
    /// Note the layer and say whether it differs from what was sent.
    fn changed(&self, key: &str, signature: impl IntoPyObject<'py>, image: Option<&Obj<'py>>) -> PyResult<bool> {
        let stamp = match image {
            Some(image) if image.hasattr("cacheKey")? => image.call_method0("cacheKey")?,
            Some(image) => image.clone(),
            None => none(self.py),
        };
        let entry = (signature, stamp).into_bound_py_any(self.py)?;
        self.layers.set_item(key, &entry)?;
        entry.ne(self.old.call_method1("get", (key,))?)
    }
    fn limit(&self) -> Reply<'py> {
        self.renderer.call_method0("max_texture_size")
    }
    fn upload(&self, key: &str, geometry: Obj<'py>, image: &Obj<'py>, fallback: (u8, u8, u8), opacity: Obj<'py>, depth: bool) -> PyResult<()> {
        let [width, height, pixels] = unpack::<3>(&self.module.call_method1("_texture", (image, fallback, self.limit()?))?)?;
        self.updates.append((key, geometry, width, height, pixels, opacity, (0., 0., 0., 0.), depth))
    }
    fn geometry(&self, positions: impl IntoPyObject<'py>, uvs: impl IntoPyObject<'py>, origin: &Obj<'py>, triangles: Option<Obj<'py>>) -> Reply<'py> {
        self.module.call_method1("_vertices", (positions, uvs, origin, triangles))
    }
}

fn tuples<'py>(py: Python<'py>, points: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = empty(py);
    for point in points.try_iter()? {
        out.append(point?.call_method0("toTuple")?)?;
    }
    Ok(out)
}

/// Keep per-layer GPU allocations across frames, including peer/capture
/// cameras.
fn references_sync<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, style, given) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let source = view.getattr("source_viewport")?;
    let frame = !given.is_none();
    let scene = if frame {
        given
    } else if source.is_none() {
        view.getattr("scene")?
    } else {
        source.getattr("scene")?
    };
    let (terrain, tiles, survey) = (attr(&scene, "terrain")?, attr(&scene, "tile_layer")?, attr(&scene, "photo_mesh")?);
    let images = scene.getattr_opt("image_planes")?.unwrap_or_else(|| PyTuple::empty(py).into_any());
    let shared = view.getattr("_shared")?;
    let cached = attr(&shared, "_reference_cache")?;
    if !(terrain.is_truthy()? || tiles.is_truthy()? || survey.is_truthy()? || images.is_truthy()? || cached.is_truthy()?) {
        return Ok(none(py));
    }
    let renderer = py.import("ingetrazo_render")?.getattr("shell_views")?.call_method1("raw_renderer", (view.getattr("renderer")?,))?;
    let origin = py.get_type::<PyTuple>().call1((view.getattr("packed")?.getattr("origin")?,))?;
    let scope = (&renderer, &scene, &origin).into_bound_py_any(py)?;
    let old = if cached.is_truthy()? && cached.get_item(0)?.eq(&scope)? { cached.get_item(1)? } else { PyDict::new(py).into_any() };
    let sync = Layers { py, module: references(py)?, renderer, old, layers: PyDict::new(py), updates: empty(py) };
    let revision = source.getattr_opt("_reference_revision")?.unwrap_or(0.into_bound_py_any(py)?);
    let one = || 1.0.into_bound_py_any(py);
    let visible = |entity: &Obj<'py>| -> PyResult<bool> {
        let off = attr(&scene, "hidden_reference_tags")?;
        if off.is_none() {
            return scene.call_method1("entity_visible", (entity,))?.is_truthy();
        }
        let layer = py.import("core.layers")?.call_method1("layer_of", (entity,))?;
        Ok(!off.contains(layer)? && !scene.call_method1("entity_hidden", (entity,))?.is_truthy()?)
    };
    let datum = attr(&scene, "georef")?;
    if visible_flag(&terrain)? {
        let image = terrain.getattr("texture_image")?;
        let triangles = terrain.getattr("triangles")?;
        if triangles.len()? != 0 && sync.changed("terrain", (id(&terrain), &revision), Some(&image))? {
            let geometry = sync.geometry(tuples(py, &terrain.getattr("vertices")?)?, terrain.getattr("uvs")?, &origin, Some(triangles))?;
            sync.upload("terrain", geometry, &image, (140, 153, 133), one()?, true)?;
        }
    } else if visible_flag(&tiles)? && !datum.is_none() {
        let patches = empty(py);
        for patch in tiles.getattr("patches")?.try_iter()? {
            patches.append(py.get_type::<PyTuple>().call1((patch?,))?)?;
        }
        let zoom = tiles.getattr("zoom")?;
        let described = py.get_type::<PyTuple>().call1((datum.call_method0("to_dict")?.call_method0("items")?,))?;
        let coverage = (id(&tiles), &zoom, patches.to_tuple(), described).into_bound_py_any(py)?;
        if attr(&shared, "_tile_coverage")?.ne(&coverage)? {
            shared.setattr("_reference_tiles", tiles.call_method1("flat_tiles", (&datum,))?)?;
            shared.setattr("_tile_coverage", &coverage)?;
        }
        let held = tiles.getattr("images")?;
        let provider = tiles.getattr("source")?;
        let opacity = tiles.getattr("opacity")?;
        for tile in shared.getattr("_reference_tiles")?.try_iter()? {
            let [x, y] = unpack::<2>(&tile?)?;
            let address = (&x, &y, &zoom).into_bound_py_any(py)?;
            let mut image = held.call_method1("get", (&address,))?;
            if image.is_none() {
                image = source.call_method1("_disk_cached_tile", (&provider, &x, &y, &zoom))?;
                if image.is_none() {
                    source.call_method0("_schedule_tile_requests")?;
                    continue;
                }
                held.set_item(&address, &image)?;
            }
            let key = format!("tile/{}/{}/{}/{}", provider.getattr("id")?.str()?, zoom.str()?, x.str()?, y.str()?);
            if sync.changed(&key, (&coverage, &opacity), Some(&image))? {
                let (positions, uvs) = (empty(py), empty(py));
                for corner in tiles.call_method1("quad_local", (&datum, &x, &y))?.try_iter()? {
                    let [position, uv] = unpack::<2>(&corner?)?;
                    positions.append(position.call_method0("toTuple")?)?;
                    uvs.append(uv)?;
                }
                let geometry = sync.geometry(positions, uvs.to_tuple(), &origin, None)?;
                sync.upload(&key, geometry, &image, (255, 255, 255), opacity.clone(), false)?;
            }
        }
    }
    if visible_flag(&survey)? && visible(&survey)? {
        let held = survey.getattr("images")?;
        for (index, material) in survey.getattr("materials")?.try_iter()?.enumerate() {
            let material = material?;
            let key = format!("photo/{index}");
            let image = held.call_method1("get", (index,))?;
            let (start, count) = (material.getattr("start")?, material.getattr("count")?);
            if count.is_truthy()? && sync.changed(&key, (id(&survey), &revision, &start, &count), Some(&image))? {
                let triangles = survey.getattr("triangles")?.get_item(PySlice::new(py, int(start.clone())?, int(start)? + int(count)?, 1))?;
                let geometry = sync.geometry(survey.getattr("vertices")?, survey.getattr("uvs")?, &origin, Some(triangles))?;
                sync.upload(&key, geometry, &image, (158, 153, 143), one()?, true)?;
            }
        }
    }
    if images.is_truthy()? {
        let amount = if !source.is_none() && !frame { source.call_method1("_edit_rest_fade", (py.None(),))? } else { 0.0.into_bound_py_any(py)? };
        let fade = py.get_type::<PyTuple>().call1((style.call_method1("get", ("fade_color", (1., 1., 1.)))?,))?.add((&amount,))?;
        for (index, plane) in images.try_iter()?.enumerate() {
            let plane = plane?;
            if amount.is_none() || !visible(&plane)? {
                continue;
            }
            let corners = tuples(py, &plane.call_method0("corners")?)?.to_tuple();
            let key = format!("image/{index}");
            let opacity = plane.getattr("opacity")?;
            if sync.changed(&key, (plane.getattr("path")?, &revision, &corners, &opacity, &fade), None)? {
                let texture = sync.module.call_method1("reference_texture", (plane.getattr("path")?, sync.limit()?))?;
                if texture.is_none() {
                    sync.layers.del_item(&key)?;
                    continue;
                }
                let geometry = sync.geometry(&corners, [(0, 1), (1, 1), (1, 0), (0, 0)], &origin, Some(PyList::new(py, [0, 1, 2, 0, 2, 3])?.into_any()))?;
                let [width, height, pixels] = unpack::<3>(&texture)?;
                sync.updates.append((key, geometry, width, height, pixels, &opacity, &fade, opacity.ge(0.999)?))?;
            }
        }
    }
    let names = sync.layers.keys();
    let reordered = cached.is_truthy()? && py.get_type::<PyList>().call1((cached.get_item(1)?,))?.ne(&names)?;
    if !sync.updates.is_empty() || reordered {
        sync.renderer.call_method1("set_reference_layers", (names, &sync.updates))?;
    }
    if sync.layers.is_empty() {
        shared.setattr("_reference_cache", py.None())?;
    } else {
        shared.setattr("_reference_cache", (scope, &sync.layers))?;
    }
    Ok(none(py))
}

pub(super) fn install_references(module: &Bound<PyModule>) -> PyResult<()> {
    module.add("__doc__", "Upload the document's display-only imagery into the shared wgpu scene.")?;
    modules::import_names(module, "ingetrazo_render", &[("fit_reference_rgba", "fit_reference_rgba"), ("reference_texture", "reference_texture")])?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("_vertices", Mode::Function, vertices, &["positions", "uvs", "origin", "triangles=None"], "", ""),
            ("_texture", Mode::Function, texture, &["image", "fallback", "limit"], "", ""),
            ("sync", Mode::Function, references_sync, &["view", "style", "*", "scene=None"], "", ""),
        ],
    )
}
