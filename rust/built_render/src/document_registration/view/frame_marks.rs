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
    let tracker = attr(&view, "_instance_updates")?;
    if let Some(lookup) = tracker.getattr_opt("face_ranges")? {
        let found = lookup.call1((&mesh, &faces))?;
        if !found.is_none() { return Ok(found); }
    }
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

/// GL's selection orange, the dot colour when the viewport does not choose.
const ORANGE: [f64; 3] = [0.95, 0.45, 0.16];

fn rgb_of(colour: &Obj<'_>) -> PyResult<[f64; 3]> {
    Ok([colour.get_item(0)?.extract()?, colour.get_item(1)?.extract()?, colour.get_item(2)?.extract()?])
}

/// The dot colours of `face`'s front and back (GL's
/// `_selection_dot_colors`), orange on both when the viewport does not
/// choose. Which side the camera sees is the GPU's to decide
/// (`preview.wgsl`): a face mark carries both, so a camera move never
/// rebuilds the marks.
fn dot_colours(view: &Obj<'_>, face: &Obj<'_>) -> PyResult<[[f64; 3]; 2]> {
    let choose = attr(&view.getattr("source_viewport")?, "_selection_dot_colors")?;
    if !choose.is_truthy()? {
        return Ok([ORANGE, ORANGE]);
    }
    let [front, back] = unpack::<2>(&choose.call1((face,))?)?;
    Ok([rgb_of(&front)?, rgb_of(&back)?])
}

/// One 28-byte face-mark vertex: the corner, the back's dot colour where a
/// normal would be (`preview.wgsl`), the front's as its RGBA.
fn mark_vertex(out: &mut Vec<u8>, p: [f32; 3], [front, back]: &[[f64; 3]; 2]) {
    for v in p {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in back {
        out.extend_from_slice(&(*v as f32).to_le_bytes());
    }
    out.extend(front.iter().map(|c| (c * 255.).clamp(0., 255.) as u8));
    out.push(255);
}

fn as_bytes<'py>(py: Python<'py>, bytes: Vec<u8>) -> Reply<'py> {
    bytes_of(numpy::PyArray1::from_vec(py, bytes), py)
}

/// The view's coordinate frame: the corners of its marks are relative to it.
fn frame_of(view: &Obj<'_>) -> PyResult<[f64; 3]> {
    let origin = attr(&view.getattr("packed")?, "origin")?;
    if origin.is_none() {
        return Ok([0.; 3]);
    }
    Ok([origin.get_item(0)?.extract()?, origin.get_item(1)?.extract()?, origin.get_item(2)?.extract()?])
}

/// The selected faces' dots in one native pass (`MeshStore.mark_triangles`):
/// every face's own triangles (the triangulation the resident bake and
/// `Face.triangulate` share), in the active mesh's frame, so the dots sit
/// on a face inside an open group where the group is, not at its
/// definition's origin. The dot colours are worked out once per
/// attribute-table row, not per face: a select-all of a 200k-face mesh was
/// seconds of per-face calls. Hidden faces are not drawn, so they get none.
///
/// `faces` are the handles (for a face the active mesh's store does not
/// know, measured one by one), `keys` their keys when the caller already
/// has them (`ElementSet.element_keys`).
fn marks_of<'py>(view: &Obj<'py>, faces: &Obj<'py>, keys: Option<Obj<'py>>) -> Reply<'py> {
    let py = view.py();
    let mesh = view.getattr("scene")?.getattr("mesh")?;
    let store = attr(&mesh, "_store")?;
    let table = attr(&mesh, "_attr_table")?;
    let hypot = py.import("core._native")?.call_method0("qt_hypot")?;
    let mut out = Vec::new();
    let mut known = std::collections::HashSet::new();
    if !store.is_none() && store.hasattr("mark_triangles")? && !table.is_none() && hypot.is_truthy()? {
        let keys = match keys {
            Some(keys) => keys,
            None => {
                let list = empty(py);
                for face in faces.try_iter()? {
                    list.append(face?.getattr("key")?)?;
                }
                list.into_any()
            }
        };
        let [corners, rows, counts, missing] = unpack::<4>(&store.call_method1("mark_triangles", (&keys, &hypot, frame_of(view)?))?)?;
        let corners: numpy::PyReadonlyArray2<'_, f32> = corners.extract()?;
        let corners = corners.as_array();
        let rows: Vec<i64> = rows.extract()?;
        let counts: Vec<u32> = counts.extract()?;
        let missing: Vec<u128> = missing.extract()?;
        // A face's attributes are its own once touched (`_attr_own`), else
        // its table row's: the colours (or None: hidden) are worked out
        // once per row, and once per distinct paint of the own ones.
        let own = attr(&mesh, "_attr_own")?.cast_into::<PyDict>().ok().filter(|own| !own.is_empty());
        let found: Vec<u128> = if own.is_some() {
            let gone: std::collections::HashSet<u128> = missing.iter().copied().collect();
            keys.try_iter()?.map(|k| k?.extract::<u128>()).filter(|k| k.as_ref().map_or(true, |k| !gone.contains(k))).collect::<PyResult<_>>()?
        } else {
            Vec::new()
        };
        let mut by_row: std::collections::HashMap<i64, Option<[[f64; 3]; 2]>> = std::collections::HashMap::new();
        let mut by_paint: std::collections::HashMap<String, Option<[[f64; 3]; 2]>> = std::collections::HashMap::new();
        let namespace = py.import("types")?.getattr("SimpleNamespace")?;
        let colours_of = |attrs: &Obj<'py>| -> PyResult<Option<[[f64; 3]; 2]>> {
            if attrs.call_method1("get", ("hidden",))?.is_truthy()? {
                return Ok(None);
            }
            Ok(Some(dot_colours(view, &namespace.call((), Some(&fields!(py; "attrs" => attrs)))?)?))
        };
        out.reserve(corners.nrows() * 28);
        let mut at = 0usize;
        for (i, (row, count)) in rows.iter().zip(&counts).enumerate() {
            let n = *count as usize * 3;
            // An own dict left empty (attributes read, never set) is the
            // unpainted face's.
            let own_attrs = match &own {
                Some(own) => own.get_item(found[i])?.filter(|d| !d.is_none()),
                None => None,
            };
            let unpainted = own_attrs.as_ref().is_some_and(|d| d.len().is_ok_and(|n| n == 0));
            let mine = own_attrs.filter(|_| !unpainted);
            let colour = match mine {
                Some(attrs) => {
                    let mut paint = String::new();
                    for name in ["color", "mat", "texture", "opacity", "pbr", "back", "hidden"] {
                        paint.push_str(&attrs.call_method1("get", (name,))?.repr()?.to_string_lossy());
                        paint.push('\u{1f}');
                    }
                    if !by_paint.contains_key(&paint) {
                        by_paint.insert(paint.clone(), colours_of(&attrs)?);
                    }
                    by_paint[&paint]
                }
                None => {
                    let row = &if unpainted { -1 } else { *row };
                    if !by_row.contains_key(row) {
                        let attrs = if *row >= 0 { table.get_item(*row)? } else { PyDict::new(py).into_any() };
                        by_row.insert(*row, colours_of(&attrs)?);
                    }
                    by_row[row]
                }
            };
            if let Some(colour) = &colour {
                for k in at..at + n {
                    mark_vertex(&mut out, [corners[[k, 0]], corners[[k, 1]], corners[[k, 2]]], colour);
                }
            }
            at += n;
        }
        if missing.is_empty() {
            view.setattr("_selected_mark_count", out.len() / 28)?;
            return as_bytes(py, out);
        }
        known = keys.try_iter()?.map(|k| k?.extract::<u128>()).collect::<PyResult<_>>()?;
        for key in missing {
            known.remove(&key);
        }
    }
    // Faces the store does not know (another mesh's, a stand-in's): one by one.
    let origin = frame_of(view)?;
    for face in faces.try_iter()? {
        let face = face?;
        if !known.is_empty() && known.contains(&face.getattr("key")?.extract::<u128>()?) {
            continue;
        }
        if attr(&face, "attrs")?.is_truthy()? && face.getattr("attrs")?.call_method1("get", ("hidden",))?.is_truthy()? {
            continue;
        }
        let colour = dot_colours(view, &face)?;
        for triangle in face.call_method0("triangulate")?.try_iter()? {
            for point in triangle?.try_iter()? {
                let [x, y, z]: [f64; 3] = point?.call_method0("toTuple")?.extract()?;
                mark_vertex(&mut out, [(x - origin[0]) as f32, (y - origin[1]) as f32, (z - origin[2]) as f32], &colour);
            }
        }
    }
    view.setattr("_selected_mark_count", out.len() / 28)?;
    as_bytes(py, out)
}

fn face_marks<'py>(a: &Args<'py>) -> Reply<'py> {
    let (view, faces) = (a.get_item(0)?, a.get_item(1)?);
    let keys = a.get_item(2)?;
    marks_of(&view, &faces, (!keys.is_none()).then_some(keys))
}

/// The selected loose edges as native preview line vertices in GL's
/// selection orange, `(pos, (0, 0, 1), rgba)` per end, in one native pass
/// (`MeshStore.mark_segments`) for the edges the active mesh knows, one by
/// one for any other.
fn edge_marks<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, edges, keys) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let numpy = np(py)?;
    let vertex = parity(py)?.getattr("VERTEX")?;
    let store = attr(&view.getattr("scene")?.getattr("mesh")?, "_store")?;
    let mut parts = Vec::new();
    let mut known = std::collections::HashSet::new();
    if !store.is_none() && store.hasattr("mark_segments")? {
        let keys = if keys.is_none() {
            let list = empty(py);
            for edge in edges.try_iter()? {
                list.append(edge?.getattr("key")?)?;
            }
            list.into_any()
        } else {
            keys
        };
        let [ends, missing] = unpack::<2>(&store.call_method1("mark_segments", (&keys, (0., 0., 0.)))?)?;
        let lines = numpy.call_method1("zeros", (ends.len()?, &vertex))?;
        lines.set_item("pos", &ends)?;
        lines.set_item("normal", (0, 0, 1))?;
        lines.set_item("color", (242, 114, 40, 255))?;
        parts.push(lines);
        let missing: Vec<u128> = missing.extract()?;
        if !missing.is_empty() {
            known = keys.try_iter()?.map(|k| k?.extract::<u128>()).collect::<PyResult<_>>()?;
            for key in missing {
                known.remove(&key);
            }
        } else {
            return Ok(parts.pop().expect("one part"));
        }
    }
    let rows = empty(py);
    for edge in edges.try_iter()? {
        let edge = edge?;
        if !known.is_empty() && known.contains(&edge.getattr("key")?.extract::<u128>()?) {
            continue;
        }
        for end in ["a", "b"] {
            rows.append((edge.getattr(end)?.call_method0("toTuple")?, (0, 0, 1), (242, 114, 40, 255)))?;
        }
    }
    parts.push(numpy.call_method1("array", (rows, &vertex))?);
    numpy.call_method1("concatenate", (PyList::new(py, parts)?,))
}

/// The dots GL puts on the face under the cursor of Push/Pull, Paint,
/// Offset, Follow Me... (`_paint_hover`): a selected face's dots, in the
/// colours of its two sides. The Metal view outlined the face in cyan
/// instead, which reads as a selected edge loop.
///
/// Built from the face's own triangulation through `placement`'s matrix (a
/// face inside a component is stored in its part's coordinates), not from
/// the pack, so it works on any placement and costs one face per hover
/// change.
fn hover_face_marks<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, face) = (a.get_item(0)?, a.get_item(1)?);
    let matrix = attr(&a.get_item(2)?, "xform")?;
    let placed = |point: Obj<'py>| if matrix.is_none() { Ok(point) } else { matrix.call_method1("map", (point,)) };
    let mut out = Vec::new();
    let colour = dot_colours(&view, &face)?;
    for triangle in face.call_method0("triangulate")?.try_iter()? {
        for point in triangle?.try_iter()? {
            let [x, y, z]: [f64; 3] = placed(point?)?.call_method0("toTuple")?.extract()?;
            mark_vertex(&mut out, [x as f32, y as f32, z as f32], &colour);
        }
    }
    as_bytes(py, out)
}

pub(super) fn install_parity(module: &Bound<PyModule>) -> PyResult<()> {
    module.add("__doc__", "Resident parity handoffs. Work is keyed to scene edits, never screen pixels.")?;
    modules::import_names(
        module,
        "built_core",
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
            ("face_marks", Mode::Function, face_marks, &["view", "faces", "keys=None"], "", ""),
            ("edge_marks", Mode::Function, edge_marks, &["view", "edges", "keys=None"], "", ""),
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

pub(super) fn vertices<'py>(a: &Args<'py>) -> Reply<'py> {
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

pub(super) fn texture<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let numpy = np(py)?;
    let (fallback, limit) = (a.get_item(1)?, a.get_item(2)?);
    let image = py.import("built_core")?.call_method1("as_raster", (a.get_item(0)?,))?;
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
        let [width, height, pixels] = unpack::<3>(&direct(self.py, texture, (image, fallback, self.limit()?))?)?;
        self.updates.append((key, geometry, width, height, pixels, opacity, (0., 0., 0., 0.), depth))
    }
    fn geometry(&self, positions: impl IntoPyObject<'py>, uvs: impl IntoPyObject<'py>, origin: &Obj<'py>, triangles: Option<Obj<'py>>) -> Reply<'py> {
        direct(self.py, vertices, (positions, uvs, origin, triangles))
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
    let renderer = py.import("built_render")?.getattr("shell_views")?.call_method1("raw_renderer", (view.getattr("renderer")?,))?;
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
        let layer = py.import("built_core.layers")?.call_method1("layer_of", (entity,))?;
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
    modules::import_names(module, "built_render", &[("fit_reference_rgba", "fit_reference_rgba"), ("reference_texture", "reference_texture")])?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("sync", Mode::Function, references_sync, &["view", "style", "*", "scene=None"], "", ""),
        ],
    )
}
