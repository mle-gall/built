// SPDX-License-Identifier: GPL-3.0-or-later
//! Preserve parse-local metadata across the optional reader provider's callbacks.
use pyo3::{
    exceptions::{PyAttributeError, PyException},
    prelude::*,
    types::{PyBool, PyDict, PyFrozenSet, PyInt, PyIterator, PyList, PySet, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
fn facade(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.reader_compat")
}
fn tables(py: Python<'_>) -> PyResult<Obj<'_>> {
    facade(py)?.getattr("_parse_tables")
}
fn core(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("openskp._core")
}
fn native(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core")
}
fn attr<'p>(obj: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    match obj.getattr(name) {
        Ok(v) => Ok(v),
        Err(e) if e.is_instance_of::<PyAttributeError>(obj.py()) => {
            Ok(obj.py().None().into_bound(obj.py()))
        }
        Err(e) => Err(e),
    }
}
fn mapping<'p>(v: Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    if v.is_none() {
        Ok(PyDict::new(v.py()))
    } else if v.is_instance_of::<PyDict>() {
        Ok(v.cast_into()?)
    } else {
        Ok(v.py().get_type::<PyDict>().call1((v,))?.cast_into()?)
    }
}
fn ensure_groups<'p>(state: &Obj<'p>) -> PyResult<Obj<'p>> {
    let v = attr(state, "groups")?;
    if !v.is_none() {
        return Ok(v);
    }
    let v = PySet::empty(state.py())?;
    state.setattr("groups", &v)?;
    Ok(v.into_any())
}
#[pyfunction]
fn group_definitions(py: Python<'_>) -> PyResult<Obj<'_>> {
    let groups = attr(&tables(py)?, "groups")?;
    if groups.is_none() {
        return Ok(PySet::empty(py)?.into_any());
    }
    py.get_type::<PySet>().call1((groups,))
}
#[pyfunction]
fn reset_group_definitions(py: Python<'_>) -> PyResult<()> {
    let t = tables(py)?;
    t.setattr("groups", PySet::empty(py)?)?;
    t.setattr("kinds", false)?;
    t.setattr("vff", py.None())
}
#[pyfunction]
fn kinds_known(py: Python<'_>, legacy: bool) -> PyResult<bool> {
    if !legacy {
        return attr(&tables(py)?, "kinds")?.is_truthy();
    }
    let Ok(module) = py.import("openskp.legacy") else {
        return Ok(false);
    };
    let Some(reader) = mapping(attr(&module, "_READERS")?)?.get_item("CGroup")? else {
        return Ok(false);
    };
    attr(&reader, "_notes_groups")?.is_truthy()
}
#[pyfunction]
fn hidden_tags<'p>(names: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = names.py();
    let names = py
        .get_type::<PySet>()
        .call1((names,))?
        .cast_into::<PySet>()?;
    let out = names.call_method0("copy")?.cast_into::<PySet>()?;
    for (id, name) in mapping(attr(&tables(py)?, "layers")?)?.iter() {
        if names.contains(name)? {
            out.add(id)?;
        }
    }
    py.get_type::<PyFrozenSet>().call1((out,))
}
#[pyfunction]
fn tag_folder_records(py: Python<'_>) -> PyResult<Bound<'_, PyList>> {
    let state = tables(py)?;
    let layers = mapping(attr(&state, "layers")?)?;
    let records = PyList::empty(py);
    let folders = attr(&state, "folders")?;
    if !folders.is_truthy()? {
        return Ok(records);
    }
    for f in folders.try_iter()? {
        let f = f?;
        let r = PyDict::new(py);
        r.set_item("key", f.get_item("key")?.str()?)?;
        let name = f.get_item("name")?;
        if name.is_truthy()? {
            r.set_item("name", name)?;
        } else {
            r.set_item("name", "Folder")?;
        }
        let parent = f.get_item("parent")?;
        if parent.is_none() {
            r.set_item("parent", parent)?;
        } else {
            r.set_item("parent", parent.str()?)?;
        }
        r.set_item("hidden", f.get_item("hidden")?.is_truthy()?)?;
        let tags = PyList::empty(py);
        for id in f.get_item("tags")?.try_iter()? {
            if let Some(n) = layers.get_item(id?)? {
                tags.append(n)?;
            }
        }
        r.set_item("tags", tags)?;
        records.append(r)?;
    }
    Ok(records)
}
#[pyfunction]
fn resolve_face_layers(model: &Obj<'_>) -> PyResult<usize> {
    let py = model.py();
    let t = tables(py)?;
    let layers = mapping(attr(&t, "layers")?)?;
    let pids = mapping(attr(&t, "pids")?)?;
    let definitions = mapping(attr(model, "definitions")?)?;
    let with_root = definitions.copy()?;
    let root = attr(model, "root")?;
    if !root.is_none() {
        with_root.set_item("ROOT", root)?;
    }
    for (key, d) in with_root.iter() {
        if let Some(ids) = pids.get_item(key)? {
            let instances = attr(&d, "instances")?;
            if !instances.is_none() {
                for (inst, id) in instances.try_iter()?.zip(ids.try_iter()?) {
                    inst?.setattr("persistent_id", id?)?;
                }
            }
        }
        if d.hasattr("is_group")? {
            let is_group = d.getattr("is_group")?;
            t.setattr("kinds", true)?;
            if is_group.is_truthy()? {
                ensure_groups(&t)?.call_method1("add", (d.getattr("id")?,))?;
            }
        }
    }
    let mut count = 0;
    let empty = pyo3::types::PyString::intern(py, "");
    for (_, d) in definitions.iter() {
        for (_, face) in mapping(attr(&d, "faces")?)?.iter() {
            let layer = attr(&face, "layer")?;
            if layer.is_instance_of::<PyInt>() && !layer.is_instance_of::<PyBool>() {
                let name = layers
                    .get_item(layer)?
                    .unwrap_or_else(|| empty.clone().into_any());
                let default = matches!(
                    name.extract::<&str>(),
                    Ok("Layer0" | "Layer 0" | "Untagged")
                );
                face.setattr("layer", if default { empty.as_any() } else { &name })?;
                count += 1;
            }
        }
    }
    Ok(count)
}
#[pyfunction]
fn attach_pbr(model: &Obj<'_>) -> PyResult<usize> {
    let py = model.py();
    let recipes = mapping(attr(&tables(py)?, "pbr")?)?;
    let materials = attr(model, "materials")?;
    let mut count = 0;
    if materials.is_truthy()? {
        for mat in materials.try_iter()? {
            let mat = mat?;
            if !attr(&mat, "pbr")?.is_none() {
                continue;
            }
            let recipe = recipes
                .get_item(attr(&mat, "name")?)?
                .unwrap_or_else(|| py.None().into_bound(py));
            match mat.setattr("pbr", &recipe) {
                Ok(()) => count += usize::from(!recipe.is_none()),
                Err(e) if e.is_instance_of::<PyAttributeError>(py) => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(count)
}
#[pyfunction]
fn resolve_vff_annotations(model: &Obj<'_>) -> PyResult<usize> {
    let py = model.py();
    let t = tables(py)?;
    let inputs = attr(&t, "vff")?;
    t.setattr("vff", py.None())?;
    if !inputs.is_truthy()? {
        return Ok(0);
    }
    native(py)?
        .call_method1(
            "skp_resolve_annotations",
            (
                model,
                inputs.get_item(0)?,
                inputs.get_item(1)?,
                inputs.get_item(2)?,
            ),
        )?
        .extract()
}
#[pyfunction]
fn _parse_pages<'p>(node: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = node.py();
    if node.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    native(py)?.call_method1("skp_pages", (node.get_item("payload")?,))
}
#[pyfunction]
fn _find_node<'p>(node: &Obj<'p>, tag: &str) -> PyResult<Obj<'p>> {
    let mut todo = vec![node.clone()];
    while let Some(node) = todo.pop() {
        if node.getattr("tag")?.eq(tag)? {
            return Ok(node);
        }
        let kids = attr(&node, "children")?;
        if kids.is_truthy()? {
            let start = todo.len();
            for kid in kids.try_iter()? {
                todo.push(kid?);
            }
            todo[start..].reverse();
        }
    }
    Ok(node.py().None().into_bound(node.py()))
}

fn entity_id<'p>(children: &Obj<'p>, dictionary: bool) -> PyResult<Obj<'p>> {
    let py = children.py();
    let kw = PyDict::new(py);
    kw.set_item("children", children)?;
    if dictionary {
        core(py)?.call_method1("extract_entity_id", (kw,))
    } else {
        let ns = py
            .import("types")?
            .getattr("SimpleNamespace")?
            .call((), Some(&kw))?;
        core(py)?.call_method1("_entity_id", (ns,))
    }
}
fn definition_kind(elements: &Obj<'_>) -> PyResult<()> {
    let py = elements.py();
    for el in elements.try_iter()? {
        let el = el?;
        let dictionary = el.is_instance_of::<PyDict>();
        let field = |k: &str| {
            if dictionary {
                el.call_method1("get", (k,))
            } else {
                el.getattr(k)
            }
        };
        if !field("tag")?.eq("8315")? {
            continue;
        }
        let payload = field("payload")?;
        let kind = core(py)?.call_method1("parse_var_int", (&payload, 0, payload.len()?))?;
        let id = entity_id(elements, dictionary)?;
        if !id.is_none() {
            let t = tables(py)?;
            t.setattr("kinds", true)?;
            if kind.eq(1)? {
                ensure_groups(&t)?.call_method1("add", (id,))?;
            }
        }
        break;
    }
    Ok(())
}
fn remember_layers(parsed: &Obj<'_>) -> PyResult<()> {
    let py = parsed.py();
    let t = tables(py)?;
    let layers = mapping(parsed.call_method1("get", ("layer_id_to_name",))?)?;
    t.setattr("layers", layers.copy()?)?;
    let pids = PyDict::new(py);
    t.setattr("pids", &pids)?;
    for (key, defn) in mapping(parsed.call_method1("get", ("defs_dict",))?)?.iter() {
        let ids = PyList::empty(py);
        for inst in defn.get_item("builder")?.getattr("instances")?.try_iter()? {
            let inst = inst?;
            let id = inst.call_method1("get", ("persistent_id",))?;
            if id.is_truthy()? {
                ids.append(id)?;
            } else {
                ids.append(entity_id(
                    &inst.call_method1("get", ("children", PyList::empty(py)))?,
                    false,
                )?)?;
            }
        }
        pids.set_item(key, ids)?;
    }
    Ok(())
}
#[pyclass]
struct FolderRecords {
    pending: Option<(Py<PyAny>, Py<PyTuple>, Option<Py<PyDict>>)>,
    iter: Option<Py<PyIterator>>,
}
#[pymethods]
impl FolderRecords {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'p>(&mut self, py: Python<'p>) -> PyResult<Option<Obj<'p>>> {
        if let Some((source, args, kwargs)) = self.pending.take() {
            let records = source
                .bind(py)
                .call(args.bind(py), kwargs.as_ref().map(|kw| kw.bind(py)))?;
            self.iter = Some(records.try_iter()?.unbind());
        }
        let Some(item) = self
            .iter
            .as_ref()
            .and_then(|it| it.bind(py).clone().next())
            .transpose()?
        else {
            return Ok(None);
        };
        let el = item.get_item(-1)?;
        if attr(&el, "tag")?.eq("F801")? {
            let node = _find_node(&el, "9B3A")?;
            let payload = attr(&node, "payload")?;
            if payload.is_truthy()? {
                let recipes = (|| -> PyResult<_> {
                    let bytes = py.get_type::<pyo3::types::PyBytes>().call1((payload,))?;
                    native(py)?.call_method1("skp_tag_folders", (bytes,))
                })();
                let recipes = match recipes {
                    Ok(v) => v,
                    Err(e) if e.is_instance_of::<PyException>(py) => PyList::empty(py).into_any(),
                    Err(e) => return Err(e),
                };
                tables(py)?.setattr("folders", recipes)?;
            }
        }
        Ok(Some(item))
    }
}
#[derive(Clone, Copy)]
enum Event {
    Group,
    Kind,
    Layers,
    Folders,
    ClearFolders,
    Annotation,
    Material,
    ClearMaterials,
}
#[pyclass(dict)]
struct Hook {
    original: Py<PyAny>,
    event: Event,
}
#[pymethods]
impl Hook {
    #[pyo3(signature=(*args,**kwargs))]
    fn __call__<'p>(
        &self,
        py: Python<'p>,
        args: &Bound<'p, PyTuple>,
        kwargs: Option<&Bound<'p, PyDict>>,
    ) -> PyResult<Obj<'p>> {
        let arg = |i: usize, name: &str| -> PyResult<Obj<'p>> {
            if i < args.len() {
                return args.get_item(i);
            }
            if let Some(v) = kwargs
                .and_then(|kw| kw.get_item(name).transpose())
                .transpose()?
            {
                return Ok(v);
            }
            Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "missing reader argument {name}"
            )))
        };
        match self.event {
            Event::Folders => {
                return Ok(Bound::new(
                    py,
                    FolderRecords {
                        pending: Some((
                            self.original.clone_ref(py),
                            args.clone().unbind(),
                            kwargs.map(|kw| kw.clone().unbind()),
                        )),
                        iter: None,
                    },
                )?
                .into_any())
            }
            Event::Kind => definition_kind(&arg(0, "elements")?)?,
            Event::ClearFolders => tables(py)?.setattr("folders", PyList::empty(py))?,
            Event::ClearMaterials => tables(py)?.setattr("pbr", PyDict::new(py))?,
            Event::Annotation => tables(py)?.setattr(
                "vff",
                (
                    arg(0, "model_dat")?,
                    arg(1, "id2pos")?,
                    arg(2, "inst_world")?,
                ),
            )?,
            _ => {}
        }
        let rec = self.original.bind(py).call(args, kwargs)?;
        match self.event {
            Event::Group => {
                if let Ok(dict) = rec.cast::<PyDict>() {
                    if let Some(id) = dict.get_item("def")?.filter(|id| !id.is_none()) {
                        ensure_groups(&tables(py)?)?.call_method1("add", (id,))?;
                    }
                }
            }
            Event::Layers => remember_layers(&rec)?,
            Event::Material => {
                let recipe = native(py)?.getattr("skp_read_pbr")?.call(args, kwargs);
                match recipe {
                    Ok(recipe) if !recipe.is_none() => {
                        let t = tables(py)?;
                        let table = mapping(attr(&t, "pbr")?)?;
                        t.setattr("pbr", &table)?;
                        table.set_item(
                            arg(2, "mat_elem")?.call_method1("get", ("name", "unknown"))?,
                            recipe,
                        )?;
                    }
                    Err(e) if !e.is_instance_of::<PyException>(py) => return Err(e),
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(rec)
    }
}
fn hook<'p>(original: Obj<'p>, event: Event, marker: &str, flag: bool) -> PyResult<Obj<'p>> {
    let obj = Bound::new(
        original.py(),
        Hook {
            original: original.unbind(),
            event,
        },
    )?;
    obj.setattr(marker, flag)?;
    Ok(obj.into_any())
}
fn install(module: &Obj<'_>, name: &str, event: Event, marker: &str) -> PyResult<bool> {
    let original = attr(module, name)?;
    if original.is_none() || attr(&original, marker)?.is_truthy()? {
        return Ok(false);
    }
    module.setattr(name, hook(original, event, marker, true)?)?;
    Ok(true)
}
#[pyfunction]
fn _note_legacy_groups(module: &Obj<'_>) -> PyResult<bool> {
    let readers = mapping(attr(module, "_READERS")?)?;
    let Some(orig) = readers.get_item("CGroup")? else {
        return Ok(false);
    };
    if orig.is_none() || attr(&orig, "_notes_groups")?.is_truthy()? {
        return Ok(false);
    }
    readers.set_item("CGroup", hook(orig, Event::Group, "_notes_groups", true)?)?;
    Ok(true)
}
#[pyfunction]
fn _note_definition_kinds(py: Python<'_>) -> PyResult<bool> {
    let Ok(c) = core(py) else {
        return Ok(false);
    };
    if !c.hasattr("extract_entity_id")? {
        return Ok(false);
    }
    install(
        &c,
        "_extract_geometry_from_nodes",
        Event::Kind,
        "_notes_kinds",
    )
}
#[pyfunction]
fn _keep_layer_table(py: Python<'_>) -> PyResult<bool> {
    let Ok(c) = core(py) else {
        return Ok(false);
    };
    install(&c, "full_parse", Event::Layers, "_keeps_layers")
}
fn with_parse(c: &Obj<'_>, name: &str, event: Event, marker: &str, clear: Event) -> PyResult<bool> {
    let parse = attr(c, "full_parse")?;
    if parse.is_none() || !install(c, name, event, marker)? {
        return Ok(false);
    }
    let layers = attr(&parse, "_keeps_layers")?.is_truthy()?;
    c.setattr("full_parse", hook(parse, clear, "_keeps_layers", layers)?)?;
    Ok(true)
}
#[pyfunction]
fn _keep_tag_folders(py: Python<'_>) -> PyResult<bool> {
    let Ok(c) = core(py) else {
        return Ok(false);
    };
    with_parse(
        &c,
        "iter_top_level_lazy",
        Event::Folders,
        "_keeps_folders",
        Event::ClearFolders,
    )
}
#[pyfunction]
fn _keep_vff_annotation_inputs(py: Python<'_>) -> PyResult<bool> {
    let Ok(c) = core(py) else {
        return Ok(false);
    };
    install(&c, "_parse_dimensions", Event::Annotation, "_keeps_vff")
}
#[pyfunction]
fn _keep_pbr(py: Python<'_>) -> PyResult<bool> {
    let (Ok(c), Ok(model)) = (core(py), py.import("openskp.model")) else {
        return Ok(false);
    };
    let material = attr(&model, "Material")?;
    if material.is_none() || mapping(attr(&material, "__dataclass_fields__")?)?.contains("pbr")? {
        return Ok(false);
    }
    with_parse(
        &c,
        "_extract_texture",
        Event::Material,
        "_keeps_pbr",
        Event::ClearMaterials,
    )
}
#[pyfunction]
fn apply(py: Python<'_>) -> PyResult<bool> {
    let api = facade(py)?;
    if api.getattr("_applied")?.is_truthy()? {
        return Ok(false);
    }
    let Ok(legacy) = py.import("openskp.legacy") else {
        return Ok(false);
    };
    let c = core(py)?;
    let n = native(py)?;
    let cursor = n.getattr("SkpArchiveCursor")?;
    if !legacy.getattr("_R")?.is(&cursor) {
        n.call_method1("skp_mfc_readers", (&legacy,))?;
    }
    legacy.setattr("_R", cursor)?;
    let functools = py.import("functools")?;
    legacy.getattr("_Archive")?.setattr(
        "read_object",
        functools.call_method1("partialmethod", (n.getattr("skp_mfc_object")?,))?,
    )?;
    if c.hasattr("_GeometryRecordNode")? {
        c.setattr(
            "_parse_tlv_geometry",
            functools.call_method1(
                "partial",
                (
                    n.getattr("skp_geometry_nodes")?,
                    c.getattr("_TlvNode")?,
                    c.getattr("_GeometryRecordNode")?,
                    c.call_method0("_tag_strings")?,
                ),
            )?,
        )?;
    }
    c.setattr("_parse_pages", api.getattr("_parse_pages")?)?;
    api.setattr("_applied", true)?;
    let installed = [
        _keep_layer_table(py)?,
        _keep_tag_folders(py)?,
        _keep_vff_annotation_inputs(py)?,
        _note_legacy_groups(&legacy)?,
        _note_definition_kinds(py)?,
        _keep_pbr(py)?,
    ];
    Ok(installed.contains(&true))
}
pub fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "ingetrazo_core.reader_compat")?;
    m.add("_applied", false)?;
    m.add("_parse_tables", py.import("threading")?.call_method0("local")?)?;
    macro_rules! add {($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,&m)?)?;)*};}
    add!(
        apply,
        reset_group_definitions,
        kinds_known,
        group_definitions,
        hidden_tags,
        tag_folder_records,
        resolve_face_layers,
        attach_pbr,
        resolve_vff_annotations,
        _parse_pages,
        _find_node,
        _note_definition_kinds,
        _note_legacy_groups,
        _keep_pbr,
        _keep_layer_table,
        _keep_tag_folders,
        _keep_vff_annotation_inputs
    );
    py.import("sys")?.getattr("modules")?.set_item(m.name()?, &m)?;
    parent.add("reader_compat", m)
}
