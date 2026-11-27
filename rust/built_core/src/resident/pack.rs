// SPDX-License-Identifier: GPL-3.0-or-later
//! The pack: the document's placements read once, each distinct mesh and
//! baked paint made a prototype, each placement a resident row.
//!
//! The tree comes from the document in one call (`doc_render_rows`, or
//! `doc_tree` while an edit context or the navigator filters it); what a
//! placement is painted with is still resolved by the document's own
//! material rules, once per distinct tag and paint.
use super::assemble::assemble;
use super::packed_scene::{PackedScene, OBJECT_HIDDEN, VISIBILITY_FILTERED};
use numpy::{PyArray1, PyArrayMethods, PyReadonlyArray1};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
    IntoPyObjectExt,
};

type Obj<'py> = Bound<'py, PyAny>;
type Reply<'py> = PyResult<Obj<'py>>;
pub type World = [[f64; 4]; 4];

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
/// `getattr(o, name, None)`.
fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}
fn namespace<'py>(py: Python<'py>, scene: &Obj<'py>) -> Reply<'py> {
    let kw = PyDict::new(py);
    kw.set_item("scene", scene)?;
    py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&kw))
}
fn paint_of<'py>(py: Python<'py>, vp: &Obj<'py>, group: &Obj<'py>) -> Reply<'py> {
    py.import("views.document_interaction")?.call_method1("_paint_of", (vp, group))
}

/// `max(1, min(255, round(opacity * 255)))`.
fn alpha(attrs: &Obj<'_>) -> PyResult<u8> {
    let opacity: f64 = match attrs.call_method1("get", ("opacity",))? {
        value if value.is_none() => 1.0,
        value => value.call_method0("__float__")?.extract()?,
    };
    Ok((opacity * 255.).round_ties_even().clamp(1., 255.) as u8)
}

/// The paint a prototype is baked with, or None while a plain container
/// colour can stay an instance tint (shared geometry). Anything else keeps
/// the whole wrapper, `base` and `fx_self` included: dropping those lost
/// inherited tag fade and opacity.
#[pyfunction]
pub fn baked_paint<'py>(paint: &Obj<'py>) -> Reply<'py> {
    let py = paint.py();
    if !paint.is_truthy()? || paint.call_method1("get", ("front_only",))?.is_truthy()? {
        return Ok(none(py));
    }
    for key in ["texture", "pbr", "by_layer", "layer_fx", "opacity"] {
        if paint.contains(key)? {
            let base = paint.call_method1("get", ("base",))?;
            if base.is_truthy()? && base.call_method1("get", ("front_only",))?.is_truthy()? {
                let copy = py.get_type::<PyDict>().call1((paint,))?;
                copy.set_item("base", py.None())?;
                return Ok(copy);
            }
            return Ok(paint.clone());
        }
    }
    Ok(none(py))
}

/// Whether a placement is drawn and the tint word of its paint, memoised
/// over tags and paints while the edit context and the navigator's filters
/// keep their say.
#[pyclass(module = "built_core", unsendable)]
pub struct PlacementState {
    scene: Py<PyAny>,
    vp: Py<PyAny>,
    visibility: Py<PyDict>,
    paints: Py<PyDict>,
}

impl PlacementState {
    pub fn new(py: Python<'_>, scene: &Obj<'_>) -> PyResult<Self> {
        Ok(Self {
            scene: scene.clone().unbind(),
            vp: namespace(py, scene)?.unbind(),
            visibility: PyDict::new(py).unbind(),
            paints: PyDict::new(py).unbind(),
        })
    }

    pub fn resolve(&self, py: Python<'_>, group: &Obj<'_>) -> PyResult<(bool, u32)> {
        let scene = self.scene.bind(py);
        let truthy = |name: &str| -> PyResult<bool> { scene.call_method1(name, (group,))?.is_truthy() };
        let through = attr(scene, "_navigator_through")?;
        let visible = if group.is_none() {
            true
        } else if through.is_truthy()? && through.contains(scene.call_method1("navigator_key", (group,))?)? {
            // Holds an isolated part: drawn through (its own geometry is
            // filtered in the walk), so its parts are not hidden.
            !(truthy("entity_hidden")? || truthy("similar_hidden")?)
                && scene.call_method1("_layer_state", (group,))?.get_item(0)?.is_truthy()?
        } else if scene.getattr("_edit_stack")?.is_truthy()? || !attr(scene, "_navigator_allowed")?.is_none() {
            truthy("entity_visible")?
        } else {
            let key = (group.getattr("layer")?, group.getattr("hidden")?.is_truthy()?);
            let memo = self.visibility.bind(py);
            match memo.get_item(&key)? {
                Some(seen) => seen.is_truthy()?,
                None => {
                    let seen = truthy("entity_visible")?;
                    memo.set_item(key, seen)?;
                    seen
                }
            }
        };
        let materials = py.import("built_core.materials")?;
        let material = py.import("built_core.group")?.call_method1("effective_material", (group,))?;
        let layer = py.import("built_core")?.getattr("tag_styles")?.call_method1("object_layer", (group,))?;
        let key = (layer, materials.call_method1("material_sig", (material,))?);
        let memo = self.paints.bind(py);
        if let Some(word) = memo.get_item(&key)? {
            return Ok((visible, word.extract()?));
        }
        let mut paint = paint_of(py, self.vp.bind(py), group)?;
        if !paint.is_truthy()? {
            paint = PyDict::new(py).into_any();
        }
        // A style-generated Front tint is not material paint. Keep that
        // distinction resident so changing style needs no geometry rebuild.
        let mut resolved = materials.call_method1("effective_attrs", (PyDict::new(py), &paint))?;
        if !resolved.is_truthy()? {
            resolved = PyDict::new(py).into_any();
        }
        let mut rgb = resolved.call_method1("get", ("color",))?;
        if !rgb.is_truthy()? {
            rgb = py.import("built_core.style")?.getattr("DEFAULT_FACE_COLOR")?;
        }
        let mut base = paint.call_method1("get", ("base", &paint))?;
        if !base.is_truthy()? {
            base = PyDict::new(py).into_any();
        }
        let unpainted = !resolved.is_truthy()?
            || (base.call_method1("get", ("front_only",))?.is_truthy()? && !paint.call_method1("get", ("fx_self",))?.is_truthy()?);
        let mut bytes = [0u8; 4];
        for (axis, byte) in bytes.iter_mut().take(3).enumerate() {
            let value: f64 = rgb.get_item(axis)?.extract()?;
            *byte = (value * 255.).clamp(0., 255.) as u8;
        }
        bytes[3] = if unpainted { 0 } else { alpha(&resolved)? };
        let word = u32::from_le_bytes(bytes);
        memo.set_item(key, word)?;
        Ok((visible, word))
    }
}

#[pymethods]
impl PlacementState {
    #[new]
    fn py_new(scene: &Obj<'_>) -> PyResult<Self> {
        Self::new(scene.py(), scene)
    }
    fn __call__(&self, py: Python<'_>, group: &Obj<'_>) -> PyResult<(bool, u32)> {
        self.resolve(py, group)
    }
}

/// World matrices of a `doc_tree` walk, as `model_placements` composes them
/// (`local @ parent`, f64, through the same product; undefined while the
/// chain has no matrix). Parents come before their children.
pub fn compose(local: &[f64], has: &[bool], parent: &[i64]) -> PyResult<(Vec<World>, Vec<bool>)> {
    let n = parent.len();
    if local.len() != n * 16 || has.len() != n {
        return Err(PyValueError::new_err("one matrix and one flag per placement"));
    }
    let mut world: Vec<World> = Vec::with_capacity(n);
    let mut defined = Vec::with_capacity(n);
    for i in 0..n {
        let own: World = std::array::from_fn(|r| std::array::from_fn(|c| local[i * 16 + r * 4 + c]));
        let up = usize::try_from(parent[i]).ok();
        if up.is_some_and(|p| p >= i) {
            return Err(PyValueError::new_err("a placement comes after its parent"));
        }
        let (matrix, known) = match up {
            None => (own, has[i]),
            Some(p) => match (has[i], defined[p]) {
                (true, true) => {
                    // The model's own product, in NumPy's order: a plain
                    // sum agrees with it only where zeros line up.
                    let flat = |m: &World| -> [f64; 16] { std::array::from_fn(|at| m[at / 4][at % 4]) };
                    let product = crate::model::placement_walks::numpy_matmul4(&flat(&own), &flat(&world[p]));
                    (std::array::from_fn(|r| std::array::from_fn(|c| product[r * 4 + c])), true)
                }
                (true, false) => (own, true),
                (false, true) => (world[p], true),
                (false, false) => (own, false),
            },
        };
        world.push(matrix);
        defined.push(known);
    }
    Ok((world, defined))
}

fn matrix<'py>(py: Python<'py>, world: &World) -> Reply<'py> {
    Ok(PyArray1::from_vec(py, world.concat()).reshape([4, 4])?.into_any())
}

/// `(world, defined)` of a tree walk, as arrays (the document's readers).
#[pyfunction]
fn doc_worlds<'py>(py: Python<'py>, local: &Obj<'py>, has: PyReadonlyArray1<'py, bool>, parent: PyReadonlyArray1<'py, i64>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let np = py.import("numpy")?;
    let flat: PyReadonlyArray1<'_, f64> = np.call_method1("ascontiguousarray", (local, np.getattr("float64")?))?.call_method1("reshape", (-1,))?.extract()?;
    let (world, defined) = compose(flat.as_slice()?, has.as_slice()?, parent.as_slice()?)?;
    let n = world.len();
    let world: Vec<f64> = world.iter().flat_map(|w| w.concat()).collect();
    Ok((PyArray1::from_vec(py, world).reshape([n, 4, 4])?.into_any(), PyArray1::from_vec(py, defined).into_any()))
}

/// One placement of the walk: the group, its world matrix, what its paint
/// resolves from, its row flags and, from the native walk, its mesh,
/// billboard mark and uid.
pub struct Record<'py> {
    pub group: Obj<'py>,
    pub world: Option<World>,
    pub paint_group: Obj<'py>,
    pub flags: u32,
    pub columns: Option<(Obj<'py>, Obj<'py>, Obj<'py>)>,
}

impl<'py> Record<'py> {
    fn tuple(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let world = match &self.world {
            Some(world) => matrix(py, world)?,
            None => none(py),
        };
        let columns = match &self.columns {
            Some((mesh, billboard, uid)) => PyTuple::new(py, [mesh, billboard, uid])?.into_any(),
            None => none(py),
        };
        PyTuple::new(py, [self.group.clone(), world, self.paint_group.clone(), self.flags.into_bound_py_any(py)?, columns])
    }
}

fn floats(array: &Obj<'_>) -> PyResult<Vec<f64>> {
    let np = array.py().import("numpy")?;
    let flat: PyReadonlyArray1<'_, f64> = np.call_method1("ascontiguousarray", (array, np.getattr("float64")?))?.call_method1("reshape", (-1,))?.extract()?;
    Ok(flat.as_slice()?.to_vec())
}
fn integers(array: &Obj<'_>) -> PyResult<Vec<i64>> {
    let np = array.py().import("numpy")?;
    let flat: PyReadonlyArray1<'_, i64> = np.call_method1("ascontiguousarray", (array, np.getattr("int64")?))?.extract()?;
    Ok(flat.as_slice()?.to_vec())
}
fn bools(array: &Obj<'_>) -> PyResult<Vec<bool>> {
    let np = array.py().import("numpy")?;
    let flat: PyReadonlyArray1<'_, bool> = np.call_method1("ascontiguousarray", (array, np.getattr("bool_")?))?.extract()?;
    Ok(flat.as_slice()?.to_vec())
}

/// The native walk (`doc_render_rows`): what each nested placement inherits
/// (tag, paint, Hide, the tag's visibility) resolved in the document, the
/// paint resolved here once per distinct tag and paint, not per placement.
pub fn native_records<'py>(py: Python<'py>, scene: &Obj<'py>, roots: &Obj<'py>) -> PyResult<Vec<Record<'py>>> {
    let layers = py.import("built_core.layers")?;
    let effective = py.import("built_core")?.getattr("tag_edits")?.getattr("effective_visible")?;
    let invisible = PySet::empty(py)?;
    let blocked = PySet::empty(py)?;
    for layer in scene.getattr("layers")?.try_iter()? {
        let layer = layer?;
        if !effective.call1((scene, &layer))?.is_truthy()? {
            invisible.add(layer.getattr("key")?)?;
            blocked.add(layer.getattr("key")?)?;
        } else if layer.getattr("locked")?.is_truthy()? {
            blocked.add(layer.getattr("key")?)?;
        }
    }
    let cols = py.import("built_core.doc_model")?.getattr("NATIVE")?.call_method1(
        "doc_render_rows",
        (roots, invisible, blocked, layers.getattr("LINK_UNTAGGED_SUFFIX")?, layers.getattr("DEFAULT_LAYER")?),
    )?;
    let column = |name: &str| cols.get_item(name);
    let parent = integers(&column("parent")?)?;
    let (world, defined) = compose(&floats(&column("local")?)?, &bools(&column("has")?)?, &parent)?;
    let (names, materials) = (column("names")?, column("materials")?);
    let proxy = py.import("built_core.group")?.getattr("PlacementProxy")?;
    let stand_ins = PyDict::new(py);
    let stand_in = |layer: i64, material: i64| -> Reply<'py> {
        if let Some(seen) = stand_ins.get_item((layer, material))? {
            return Ok(seen);
        }
        let kw = PyDict::new(py);
        kw.set_item("name", "Paint")?;
        kw.set_item("uid", "paint")?;
        let made = proxy.call((py.None(),), Some(&kw))?;
        made.setattr("layer", if layer >= 0 { names.get_item(layer)? } else { none(py) })?;
        made.setattr("material", if material >= 0 { materials.get_item(material)? } else { none(py) })?;
        made.setattr("owner", py.None())?;
        stand_ins.set_item((layer, material), &made)?;
        Ok(made)
    };
    let flags = integers(&column("flags")?)?;
    let paint_layers = integers(&column("paint_layer")?)?;
    let paint_materials = integers(&column("material")?)?;
    let (handles, meshes, billboards, uids) = (column("handles")?, column("mesh")?, column("billboard")?, column("uid")?);
    let mut out = Vec::with_capacity(parent.len());
    for i in 0..parent.len() {
        let group = handles.get_item(i)?;
        let mesh = match meshes.get_item(i)? {
            mesh if mesh.is_none() => group.getattr("mesh")?,
            mesh => mesh,
        };
        let uid = match uids.get_item(i)? {
            uid if uid.is_none() => group.getattr("uid")?,
            uid => uid,
        };
        out.push(Record {
            world: defined[i].then(|| world[i]),
            paint_group: stand_in(paint_layers[i], paint_materials[i])?,
            flags: flags[i] as u32,
            columns: Some((mesh, billboards.get_item(i)?, uid)),
            group,
        });
    }
    Ok(out)
}

/// The walk under an edit context or the navigator's filters: the tree from
/// the document in one call (`doc_tree`), the editor's proxies deciding
/// what is hidden and filtered.
pub fn proxy_records<'py>(py: Python<'py>, scene: &Obj<'py>, roots: &Obj<'py>, vp: &Obj<'py>, state: &PlacementState) -> PyResult<Vec<Record<'py>>> {
    let tree = py.import("built_core.doc_model")?.getattr("NATIVE")?.call_method1("doc_tree", (roots,))?;
    let handles = tree.get_item(0)?;
    let parent = integers(&tree.get_item(1)?)?;
    let (world, defined) = compose(&floats(&tree.get_item(2)?)?, &bools(&tree.get_item(3)?)?, &parent)?;
    let own_hidden = bools(&tree.get_item(4)?)?;
    let proxies = py.import("views.document_interaction")?.getattr("DocumentInteraction")?.call_method1(
        "_bind_placement_rows",
        (vp, &handles, PyList::new(py, &parent)?, PyList::empty(py)),
    )?;
    // This adapter is local to one walk. Release each temporary editor proxy
    // with its row instead of tearing down the whole table under the GIL.
    vp.getattr("_placement_proxies")?.call_method0("clear")?;
    let allowed = attr(scene, "_navigator_allowed")?;
    let through = attr(scene, "_navigator_through")?;
    let n = parent.len();
    let (mut hidden_of, mut object_hidden_of, mut filtered_of) = (vec![false; n], vec![false; n], vec![false; n]);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let group = handles.get_item(i)?;
        let proxy = proxies.get_item(i)?;
        proxies.set_item(i, py.None())?;
        let mut hidden = !state.resolve(py, &proxy)?.0;
        let mut object_hidden = own_hidden[i];
        let key = || scene.call_method1("navigator_key", (&group,));
        let outside = !allowed.is_none() && !allowed.contains(key()?)?;
        let mut filtered = !scene.call_method1("_layer_state", (&proxy,))?.get_item(0)?.is_truthy()?
            || scene.call_method1("similar_hidden", (&proxy,))?.is_truthy()?
            || (outside && !(through.is_truthy()? && through.contains(key()?)?));
        if let Ok(p) = usize::try_from(parent[i]) {
            hidden |= hidden_of[p];
            object_hidden |= object_hidden_of[p];
            filtered |= filtered_of[p];
        }
        (hidden_of[i], object_hidden_of[i], filtered_of[i]) = (hidden, object_hidden, filtered);
        let flags = u32::from(hidden)
            | if object_hidden { OBJECT_HIDDEN } else { 0 }
            | if filtered || outside { VISIBILITY_FILTERED } else { 0 };
        out.push(Record { group, world: defined[i].then(|| world[i]), paint_group: proxy, flags, columns: None });
    }
    Ok(out)
}

/// The placements under `roots`, parents first: the native walk, or the
/// editor's while an edit context or the navigator filters the tree.
pub fn placement_records<'py>(py: Python<'py>, scene: &Obj<'py>, roots: &Obj<'py>, vp: &Obj<'py>, state: &PlacementState) -> PyResult<Vec<Record<'py>>> {
    if !scene.getattr("_edit_stack")?.is_truthy()?
        && attr(scene, "_navigator_allowed")?.is_none()
        && !attr(scene, "_navigator_through")?.is_truthy()?
        && scene.getattr("edit_group")?.is_none()
    {
        return native_records(py, scene, roots);
    }
    proxy_records(py, scene, roots, vp, state)
}

fn tuples<'py>(py: Python<'py>, records: &[Record<'py>]) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    for record in records {
        out.append(record.tuple(py)?)?;
    }
    Ok(out)
}

/// `(group, world, paint group, flags, columns)` per placement under `roots`.
#[pyfunction(name = "placement_records")]
fn py_placement_records<'py>(py: Python<'py>, scene: &Obj<'py>, roots: &Obj<'py>, vp: &Obj<'py>, state: PyRef<'py, PlacementState>) -> PyResult<Bound<'py, PyList>> {
    tuples(py, &placement_records(py, scene, roots, vp, &state)?)
}
#[pyfunction(name = "native_records")]
fn py_native_records<'py>(py: Python<'py>, scene: &Obj<'py>, roots: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    tuples(py, &native_records(py, scene, roots)?)
}
#[pyfunction(name = "proxy_records")]
fn py_proxy_records<'py>(py: Python<'py>, scene: &Obj<'py>, roots: &Obj<'py>, vp: &Obj<'py>, state: PyRef<'py, PlacementState>) -> PyResult<Bound<'py, PyList>> {
    // `(group, world, proxy, flags)`: this walk reads no columns.
    let out = PyList::empty(py);
    for record in proxy_records(py, scene, roots, vp, &state)? {
        out.append(record.tuple(py)?.get_slice(0, 4))?;
    }
    Ok(out)
}

/// Where an opening document is looked at from: its saved home, else the
/// middle of its box.
#[pyfunction]
fn open_focus<'py>(py: Python<'py>, scene: &Obj<'py>) -> Reply<'py> {
    let home = scene.getattr("camera_home")?;
    if home.is_instance_of::<PyDict>() {
        let target = home.call_method1("get", ("target",))?;
        if !target.is_none() {
            return Ok(target);
        }
    }
    let (low, high) = {
        let bounds = scene.call_method0("bounds")?;
        (bounds.get_item(0)?, bounds.get_item(1)?)
    };
    if low.is_none() {
        return (0., 0., 0.).into_bound_py_any(py);
    }
    let low: [f64; 3] = low.call_method0("toTuple")?.extract()?;
    let high: [f64; 3] = high.call_method0("toTuple")?.extract()?;
    PyTuple::new(py, (0..3).map(|k| (low[k] + high[k]) / 2.))?.into_bound_py_any(py)
}

/// A PBR material as a texture key: its images (base, normal, ORM, AO,
/// roughness, metallic), whether the normal map is flipped, and its factors
/// (metallic, roughness, normal scale, AO strength). Logical material IDs
/// survive incremental prototype remapping; the image layers are assigned
/// separately, so a shared normal or ORM map takes one layer.
#[pyfunction]
pub fn pbr_entry<'py>(py: Python<'py>, base: &Obj<'py>, recipe: &Obj<'py>) -> Reply<'py> {
    let path = |key: &str| -> Reply<'py> {
        let map = recipe.call_method1("get", (key,))?;
        if map.is_truthy()? { map.call_method1("get", ("path",)) } else { Ok(none(py)) }
    };
    let images = PyTuple::new(py, [base.clone(), path("normal_map")?, path("orm_map")?, path("ao_map")?, path("roughness_map")?, path("metallic_map")?, path("emission_map")?])?;
    let factor = |key: &str, default: f64| -> PyResult<f64> {
        recipe.call_method1("get", (key, default))?.call_method0("__float__")?.extract()
    };
    let factors = (factor("metallic", 0.)?, factor("roughness", 1.)?, factor("normal_scale", 1.)?, factor("ao_strength", 1.)?);
    let normal = recipe.call_method1("get", ("normal_map",))?;
    let flip = normal.is_truthy()? && normal.call_method1("get", ("flip_y",))?.is_truthy()?;
    let emission: [f32; 3] = recipe.call_method1("get", ("emission", (0., 0., 0.)))?.extract()?;
    let strength = factor("emission_strength", 0.)? as f32;
    let radiance = PyTuple::new(py, emission.map(|v| v * strength))?;
    // An explicitly empty legacy recipe still selects the default PBR surface.
    let surface = recipe.len()? == 0 || crate::gltf_write::has_surface(recipe)?;
    py.import("built_core")?.getattr("PbrEntry")?.call1((images, flip, factors, radiance, surface))
}

pub(super) fn invisible_tags<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let visible = py.import("built_core")?.getattr("tag_edits")?.getattr("effective_visible")?;
    let hidden = PySet::empty(py)?;
    for layer in scene.getattr("layers")?.try_iter()? {
        let layer = layer?;
        if !visible.call1((scene, &layer))?.is_truthy()? {
            hidden.add(layer.getattr("key")?)?;
        }
    }
    Ok(hidden)
}

/// The resident scene of `scene`.
///
/// `baked_paints` (id(group) -> paint) bakes those groups with that paint
/// instead of the one they resolve to: the incremental bake of a changed
/// prototype variant. `progress(fraction, text)` is called as the
/// placements are read (the bake that follows reports at 0.9). `roots`
/// packs off-document clipboard roots with the model's bake, without
/// walking the document.
#[pyfunction]
#[pyo3(signature = (scene, view_tint = None, *, face_overrides = None, texture_images = true, baked_paints = None,
    progress = None, previous_origin = None, previous_pack = None, focus = None, roots = None, hidden_tags = None))]
#[allow(clippy::too_many_arguments)]
pub fn pack_scene<'py>(
    py: Python<'py>,
    scene: &Obj<'py>,
    view_tint: Option<Obj<'py>>,
    face_overrides: Option<Obj<'py>>,
    texture_images: bool,
    baked_paints: Option<Obj<'py>>,
    progress: Option<Obj<'py>>,
    previous_origin: Option<Obj<'py>>,
    previous_pack: Option<Obj<'py>>,
    focus: Option<Obj<'py>>,
    roots: Option<Obj<'py>>,
    hidden_tags: Option<Bound<'py, PySet>>,
) -> PyResult<Bound<'py, PackedScene>> {
    let hidden_tags = match hidden_tags { Some(tags) => tags, None => invisible_tags(py, scene)? };
    let element_tags = PySet::empty(py)?;
    let packing = "Packing geometry for the GPU…";
    let (view_tint, face_overrides, baked_paints, progress, previous_pack, focus, roots) = (
        view_tint.filter(|v| v.is_truthy().unwrap_or(false)),
        face_overrides.filter(|v| !v.is_none()),
        baked_paints.filter(|v| !v.is_none()),
        progress.filter(|v| !v.is_none()),
        previous_pack.filter(|v| !v.is_none()),
        focus.filter(|v| !v.is_none()),
        roots.filter(|v| !v.is_none()),
    );
    let previous_origin = match previous_origin {
        Some(origin) => origin,
        None => (0., 0., 0.).into_bound_py_any(py)?,
    };
    let state = PlacementState::new(py, scene)?;
    let vp = namespace(py, scene)?;
    let texture_ids = PyDict::new(py);
    let meshes = PyList::empty(py);
    let lookup = PyDict::new(py);
    let prototype_paints = PyDict::new(py);
    let coordinates = PyList::empty(py);
    let wide_variants = PySet::empty(py)?;
    let mesh_codes = PyDict::new(py);
    let (uids, groups) = (PyList::empty(py), PyList::empty(py));
    let (mut codes, mut worlds, mut flags, mut paints) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut billboard_codes: Vec<usize> = Vec::new();

    // Where each mesh's prototype was anchored by the pack this one replaces.
    let old_anchors = PyDict::new(py);
    if let Some(previous) = &previous_pack {
        let origins = previous.getattr("prototype_origins")?;
        let known = origins.len()?;
        let old = previous.getattr("prototype_lookup")?;
        if old.is_truthy()? {
            for item in old.call_method0("items")?.try_iter()? {
                let item = item?;
                let code: i64 = item.get_item(1)?.extract()?;
                if code >= 0 && (code as usize) < known {
                    old_anchors.set_item(item.get_item(0)?.get_item(0)?, origins.get_item(code)?)?;
                }
            }
        }
    }
    // Off-document clipboard roots share the model bake without changing the
    // document tree or walking its unrelated resident placements.
    let records = match &roots {
        Some(roots) => native_records(py, scene, roots)?,
        None => {
            let mut records = vec![Record { group: none(py), world: None, paint_group: none(py), flags: 0, columns: None }];
            records.extend(placement_records(py, scene, &scene.getattr("groups")?, &vp, &state)?);
            records
        }
    };
    let mut total = 1usize;
    if let Some(progress) = &progress {
        let count = py.import("built_core")?.call_method1("purge_iter_groups", (scene.getattr("groups")?,))?.try_iter()?.count();
        total = count.max(1);
        progress.call1((0.0, packing))?;
    }
    // The paint each paint group resolves to, once per paint group: the
    // native walk shares one between every placement of a tag and paint.
    let resolved = PyDict::new(py);
    let edit_sources = PyDict::new(py);
    let builtins = py.import("builtins")?;
    let id = |object: &Obj<'py>| object.as_ptr() as usize;
    for (index, record) in records.iter().enumerate() {
        if let Some(progress) = &progress {
            if index % 64 == 63 {
                progress.call1((0.9 * index as f64 / total as f64, packing))?;
            }
        }
        let group = &record.group;
        let (mesh, billboard, uid) = match &record.columns {
            Some((mesh, billboard, uid)) => (mesh.clone(), billboard.clone(), uid.clone()),
            None if group.is_none() => (scene.getattr("loose_mesh")?, false.into_bound_py_any(py)?, "loose".into_bound_py_any(py)?),
            None => (
                group.getattr("mesh")?,
                match group.getattr_opt("billboard")? {
                    Some(billboard) => billboard,
                    None => false.into_bound_py_any(py)?,
                },
                match group.getattr_opt("uid")? {
                    Some(uid) => uid,
                    None => "loose".into_bound_py_any(py)?,
                },
            ),
        };
        let paint_key = id(&record.paint_group);
        let got = match resolved.get_item(paint_key)? {
            Some(got) => got,
            None => {
                let mut attrs = paint_of(py, &vp, &record.paint_group)?;
                if attrs.is_truthy()? && attrs.call_method1("get", ("front_only",))?.is_truthy()? {
                    attrs = none(py); // style tint stays dynamic
                }
                let got = (baked_paint(&attrs)?, state.resolve(py, &record.paint_group)?.1).into_bound_py_any(py)?;
                resolved.set_item(paint_key, &got)?;
                got
            }
        };
        // Texture mapping, opacity and tag effects create paint variants.
        // Opaque plain container RGB stays in the instance packed RGBA word.
        let baked = match &baked_paints {
            Some(paints) if paints.contains(id(group))? => paints.get_item(id(group))?,
            _ => got.get_item(0)?,
        };
        let own = if group.is_truthy()? { id(group).into_bound_py_any(py)? } else { none(py) };
        let variant = (id(&mesh), builtins.call_method1("repr", (&baked,))?, &billboard).into_bound_py_any(py)?;
        let apart = |variant: &Obj<'py>| -> Reply<'py> {
            (variant.get_item(0)?, (variant.get_item(1)?, &own), variant.get_item(2)?).into_bound_py_any(py)
        };
        let mut key = if wide_variants.contains(&variant)? { apart(&variant)? } else { variant.clone() };
        if !lookup.contains(&key)? {
            let anchor = match old_anchors.get_item(id(&mesh))? {
                Some(anchor) => anchor,
                None => (0., 0., 0.).into_bound_py_any(py)?,
            };
            let world = match &record.world {
                Some(world) => matrix(py, world)?,
                None => none(py),
            };
            let arrays = py.import("built_core")?.call_method1(
                "source_arrays",
                (&mesh, &baked, &texture_ids, face_overrides.as_ref(), &billboard, &coordinates, anchor, focus.as_ref(), world, &hidden_tags, &element_tags),
            )?;
            let last = coordinates.get_item(coordinates.len() - 1)?;
            if last.get_item(2)?.is_truthy()? && focus.is_some() {
                wide_variants.add(&variant)?;
                key = apart(&variant)?;
            }
            if arrays.get_item(0)?.len()? != 0 {
                if face_overrides.is_none() && arrays.get_item(3)?.len()? >= 32768 {
                    // A deferred stream may weld close coordinates on first
                    // touch. Only seed spans from the same materialized store.
                    if let Some(store) = mesh.getattr("__dict__")?.cast_into::<PyDict>()?.get_item("_store")? {
                        if store.is_exact_instance_of::<crate::store::py::PyMeshStore>() {
                            let st = store.cast::<crate::store::py::PyMeshStore>()?.borrow();
                            edit_sources.set_item(meshes.len(), (st.inner.lineage, st.inner.revision))?;
                        }
                    }
                }
                lookup.set_item(&key, meshes.len())?;
                prototype_paints.set_item(meshes.len(), &baked)?;
                meshes.append(arrays)?;
            } else {
                lookup.set_item(&key, -1)?;
                coordinates.call_method0("pop")?;
            }
        }
        let code: i64 = lookup.get_item(&key)?.expect("just set").extract()?;
        mesh_codes.call_method1("setdefault", (id(&mesh), (&mesh, code)))?;
        if code < 0 {
            continue;
        }
        let sprite = billboard.is_truthy()?;
        if sprite && !billboard_codes.contains(&(code as usize)) {
            billboard_codes.push(code as usize);
        }
        codes.push(code as usize);
        worlds.push(record.world.unwrap_or_else(|| std::array::from_fn(|r| std::array::from_fn(|c| f64::from(u8::from(r == c))))));
        flags.push(record.flags | if sprite { 8 } else { 0 });
        paints.push(got.get_item(1)?.extract::<u32>()?);
        uids.append(uid)?;
        groups.append(group)?;
    }
    drop(records);
    let occurrence_keys = PyDict::new(py);
    if roots.is_none() {
        let repeated = scene.call_method0("repeated_uids")?;
        let mut linked = false;
        for uid in uids.iter() {
            linked |= uid.call_method1("startswith", ("@[",))?.is_truthy()?;
        }
        if repeated.is_truthy()? || linked {
            let row_id = py.import("built_core.link_structure")?.getattr("row_id")?;
            for (i, group) in groups.iter().enumerate() {
                if group.is_none() {
                    continue;
                }
                let uid = group.getattr("uid")?;
                if repeated.contains(&uid)? || uid.call_method1("startswith", ("@[",))?.is_truthy()? {
                    let key = row_id.call1((scene.call_method1("occurrence_path", (&group,))?,))?;
                    occurrence_keys.set_item(&group, &key)?;
                    uids.set_item(i, key)?;
                }
            }
        }
    }
    if let Some(progress) = &progress {
        progress.call1((0.9, packing))?;
    }
    let mut tints = vec![None; codes.len()];
    if let Some(view_tint) = &view_tint {
        for (row, uid) in uids.iter().enumerate() {
            let tint = view_tint.call_method1("get", (uid,))?;
            if !tint.is_none() {
                tints[row] = Some(tint.extract::<Vec<f64>>()?);
            }
        }
    }
    let wide = !wide_variants.is_empty();
    let with_maps = !texture_ids.is_empty() || !billboard_codes.is_empty();
    let hypot: String = py.import("core._native")?.call_method0("qt_hypot")?.extract()?;
    let built = assemble(py, &meshes, codes, worlds, flags, paints, tints, &coordinates, billboard_codes, with_maps, &previous_origin, wide, &hypot)?;
    // Drop the per-prototype copies before decoding textures. Deferred
    // source arrays remain owned by their meshes.
    drop(meshes);
    let textures = if texture_images {
        py.import("built_render")?.call_method1("material_texture_array", (&texture_ids, 8192, 256 * 1024 * 1024))?
    } else {
        none(py)
    };
    let field = |name: &str| -> PyResult<Py<PyAny>> {
        Ok(built.get_item(name)?.ok_or_else(|| PyValueError::new_err("an assembled field is missing"))?.unbind())
    };
    let reveal = |name: &str| -> PyResult<bool> { attr(scene, name)?.is_truthy() };
    let bake_focus = match (&focus, wide) {
        (Some(focus), true) => py.get_type::<PyTuple>().call1((focus,))?.unbind(),
        _ => py.None(),
    };
    let packed = Bound::new(
        py,
        PackedScene::assembled(
            py,
            &field,
            uids.into_any().unbind(),
            groups.into_any().unbind(),
            textures.unbind(),
            py.get_type::<PyTuple>().call1((&texture_ids,))?.unbind(),
            (reveal("show_hidden_geometry")?, reveal("show_hidden_objects")?).into_py_any(py)?,
            lookup.into_any().unbind(),
            prototype_paints.into_any().unbind(),
            bake_focus,
            occurrence_keys.into_any().unbind(),
            mesh_codes.into_any().unbind(),
        )?,
    )?;
    packed.setattr("_element_tags", element_tags)?;
    if !edit_sources.is_empty() { packed.setattr("_edit_sources", edit_sources)?; }
    Ok(packed)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let entry = py.import("collections")?.call_method1("namedtuple", ("PbrEntry", ("images", "flip_y", "factors", "radiance", "surface")))?;
    m.add("PbrEntry", entry)?;
    m.add_class::<PlacementState>()?;
    m.add_function(wrap_pyfunction!(baked_paint, m)?)?;
    m.add_function(wrap_pyfunction!(doc_worlds, m)?)?;
    m.add_function(wrap_pyfunction!(py_placement_records, m)?)?;
    m.add_function(wrap_pyfunction!(py_native_records, m)?)?;
    m.add_function(wrap_pyfunction!(py_proxy_records, m)?)?;
    m.add_function(wrap_pyfunction!(open_focus, m)?)?;
    m.add_function(wrap_pyfunction!(pbr_entry, m)?)?;
    m.add_function(wrap_pyfunction!(pack_scene, m)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translation(x: f64) -> [f64; 16] {
        let mut m = [0.; 16];
        (m[0], m[5], m[10], m[15], m[12]) = (1., 1., 1., 1., x);
        m
    }

    #[test]
    fn a_child_composes_with_its_parent_and_inherits_without_a_matrix() {
        let local: Vec<f64> = [translation(1.), translation(10.), translation(100.), translation(0.)].concat();
        let (world, defined) = compose(&local, &[true, true, false, false], &[-1, 0, 1, -1]).unwrap();
        assert_eq!(defined, [true, true, true, false]);
        assert_eq!(world[1][3][0], 11.);
        assert_eq!(world[2], world[1]);
    }

    #[test]
    fn a_child_before_its_parent_is_refused() {
        let local: Vec<f64> = [translation(1.), translation(2.)].concat();
        assert!(compose(&local, &[true, true], &[1, -1]).is_err());
    }
}
