//! Public protocol modules, installed by the native core package finder.
use pyo3::{
    exceptions::{PyAttributeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyModule, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
type Module<'p> = Bound<'p, PyModule>;
mod feed;
pub(crate) mod history;
mod material;
mod records;
mod wrappers;
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn dict(py: Python<'_>) -> Obj<'_> {
    PyDict::new(py).into_any()
}
fn list(py: Python<'_>) -> Obj<'_> {
    PyList::empty(py).into_any()
}
fn get<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    o.call_method1("get", (k,))
}
fn otherwise<'p>(v: Obj<'p>, default: Obj<'p>) -> PyResult<Obj<'p>> {
    Ok(if v.is_truthy()? { v } else { default })
}
fn copied<'p>(x: &Obj<'p>) -> PyResult<Obj<'p>> {
    x.py().import("copy")?.call_method1("deepcopy", (x,))
}
fn exports(m: &Module<'_>, source: &Obj<'_>, names: &str) -> PyResult<()> {
    for name in names.split_whitespace() {
        m.add(name, source.getattr(name)?)?
    }
    Ok(())
}
fn class<'p>(
    m: &Module<'p>,
    name: &str,
    bases: Vec<Obj<'p>>,
    ns: &Bound<'p, PyDict>,
) -> PyResult<Obj<'p>> {
    let py = m.py();
    ns.set_item("__module__", m.name()?)?;
    let c = py
        .import("builtins")?
        .call_method1("type", (name, PyTuple::new(py, bases)?, ns))?;
    m.add(name, &c)?;
    Ok(c)
}
fn method(ns: &Bound<'_, PyDict>, name: &str, f: Obj<'_>) -> PyResult<()> {
    ns.set_item(
        name,
        ns.py()
            .import("functools")?
            .call_method1("partialmethod", (f,))?,
    )
}
fn prop(ns: &Bound<'_, PyDict>, name: &str, f: Obj<'_>) -> PyResult<()> {
    ns.set_item(
        name,
        ns.py().import("builtins")?.call_method1("property", (f,))?,
    )
}
macro_rules! meth {
    ($ns:expr,$m:expr,$name:literal,$f:ident) => {
        method(&$ns, $name, wrap_pyfunction!($f, $m)?.into_any())?
    };
}
use meth;
const KEYS: &str =
    "mesh_key known_mesh resolve_mesh kind_of store_of element elements to_json_key from_json_key";
const QUERIES: &str =
    "mesh_info faces edges vertices selection select feed solid_report triangles face_set_volume resolve";
#[pyfunction(name = "__getattr__")]
fn child<'p>(py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
    if matches!(name, "commands" | "records" | "queries" | "keys" | "feed") {
        Ok(py.import(format!("built_core.protocol.{name}"))?.into_any())
    } else {
        Err(PyAttributeError::new_err(name.to_owned()))
    }
}
#[pyfunction(signature=(history,name,/,**args))]
pub(super) fn run<'p>(
    history: &Obj<'p>,
    name: &Obj<'p>,
    args: Option<&Bound<'p, PyDict>>,
) -> PyResult<Obj<'p>> {
    let py = history.py();
    let cmds = py.import("built_core.protocol.commands")?.getattr("COMMANDS")?;
    let fun = match cmds.get_item(name) {
        Ok(v) => v,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) => {
            let keys = py.import("builtins")?.call_method1("sorted", (cmds,))?;
            return Err(PyValueError::new_err(format!(
                "unknown command {}; one of {}",
                name.repr()?,
                keys
            )));
        }
        Err(e) => return Err(e),
    };
    // Elements may be named by persistent id (`f:…`) as well as by key.
    let args = match args {
        Some(a) => {
            let mesh = history.getattr("scene")?.getattr("mesh")?;
            let out = PyDict::new(py);
            for (k, v) in a.iter() {
                let element = matches!(
                    k.extract::<String>()?.as_str(),
                    "face" | "faces" | "edge" | "edges" | "vertex" | "vertices" | "keys" | "anchor"
                );
                out.set_item(&k, if element { crate::protocol::keys::translate_ids(&mesh, &v)? } else { v })?;
            }
            Some(out)
        }
        None => None,
    };
    fun.call((history,), args.as_ref())
}
#[pyclass]
pub(super) struct Headless {
    #[pyo3(get)]
    scene: Py<PyAny>,
    #[pyo3(get)]
    history: Py<PyAny>,
}
impl Headless {
    pub(super) fn from_history(history: Obj<'_>) -> PyResult<Self> {
        Ok(Self {
            scene: history.getattr("scene")?.unbind(),
            history: history.unbind(),
        })
    }
}
#[pymethods]
impl Headless {
    fn update(&self) {}
    fn set_hover(&self, _entity: &Obj<'_>) {}
    fn set_suppressed_faces(&self, _faces: &Obj<'_>) {}
    #[pyo3(signature=(_text,_msec=2500))]
    fn flash_status(&self, _text: &Obj<'_>, _msec: i64) {}
}
pub(crate) fn install(m: &Module<'_>, full: &str) -> PyResult<()> {
    let py = m.py();
    let core = py.import("built_core")?;
    let protocol = core.getattr("command_protocol")?;
    match full {
        "built_core.protocol" => {
            let keys = py.import("built_core.protocol.keys")?;
            let feed = py.import("built_core.protocol.feed")?;
            let names =
                "KINDS element elements from_json_key known_mesh mesh_key resolve_mesh to_json_key";
            exports(m, keys.as_any(), names)?;
            exports(m, feed.as_any(), "ChangeFeed MeshChange Step")?;
            m.add(
                "__all__",
                [
                    "KINDS",
                    "ChangeFeed",
                    "MeshChange",
                    "Step",
                    "element",
                    "elements",
                    "from_json_key",
                    "known_mesh",
                    "mesh_key",
                    "resolve_mesh",
                    "to_json_key",
                ]
                .to_vec(),
            )?;
            m.add_function(wrap_pyfunction!(child, m)?)?;
        }
        "built_core.protocol.keys" => {
            exports(m, &protocol, KEYS)?;
            m.add("KINDS", ("vertex", "edge", "face"))?;
        }
        "built_core.protocol.queries" => exports(m, &protocol, QUERIES)?,
        "built_core.protocol.feed" => feed::install(m, &core)?,
        "built_core.protocol.records" => records::install(m)?,
        "built_core.protocol.commands" => {
            let cmds = PyDict::new(py);
            let records = py.import("built_core.protocol.records")?;
            let deliveries = core.getattr("deliveries")?;
            for &name in VERBS {
                let source = match name {
                    "patch_record_field" | "set_filing_rule" => records.as_any(),
                    "create_delivery" | "edit_delivery" | "delete_delivery" | "run_delivery" => {
                        &deliveries
                    }
                    _ => &protocol,
                };
                let f = source.getattr(name)?;
                cmds.set_item(name, &f)?;
                m.add(name, f)?;
            }
            exports(
                m,
                core.as_any(),
                "EditTagCommand LinkSubsetCommand SetLocationCommand SetTagStateCommand",
            )?;
            m.add("COMMANDS", cmds)?;
            m.add("run", protocol.getattr("run")?)?;
        }
        "built_core.material_commands" => material::install(m)?,
        _ => wrappers::install(m, full, &core)?,
    }
    Ok(())
}
const VERBS: &[&str] = &[
    "set_location",
    "set_model_units",
    "set_annotation_font",
    "set_dimension_style",
    "patch_record_field",
    "set_filing_rule",
    "create_delivery",
    "edit_delivery",
    "delete_delivery",
    "run_delivery",
    "follow_me",
    "intersect_faces",
    "add_edge",
    "add_edges",
    "add_exact_geometry",
    "materialize_exact_geometry",
    "bind_exact_plane",
    "recover_exact_geometry",
    "make_faceted",
    "add_face",
    "add_guide",
    "change_guide",
    "add_guide_circle",
    "add_arc",
    "move_placements",
    "copy_placements",
    "move_selection",
    "scale_selection",
    "place_group",
    "set_opening_position",
    "erase_stroke",
    "add_geo_path",
    "move_geo_path_node",
    "add_text_label",
    "add_dimension",
    "place_section_plane",
    "set_north",
    "add_image_plane",
    "set_group_scale",
    "set_group_axes",
    "flip_selection",
    "paste_clipboard",
    "bevel_selection",
    "rotate_selection",
    "offset_outline",
    "paint_material",
    "divide_edges",
    "join_edges",
    "move_vertices",
    "erase",
    "reverse_faces",
    "push_pull",
    "orient_faces",
    "rebuild_cap",
    "solid_operation",
    "explode_group",
    "split_into_pieces",
    "explode_view",
    "purge_unused",
    "purge_debris",
    "undo",
    "redo",
    "hide",
    "unhide",
    "lock",
    "rename_placement",
    "rename_placements",
    "paint_placements",
    "make_unique",
    "edit_assembly_part",
    "edit_visibility_set",
    "edit_saved_scene",
    "edit_link_subset",
    "reset_link_tags",
    "unlink_placements",
    "edit_section_plane",
    "edit_loose_geometry",
    "file_placements",
    "promote_tag",
    "edit_structure_node",
    "edit_tag",
    "set_tag",
    "set_tag_state",
    "resegment_curve",
    "repair_solid",
    "rename_material",
    "edit_material",
    "duplicate_material",
    "delete_material",
    "delete_placements",
    "rename_component",
    "create_material",
    "set_material_base",
    "set_material_pbr",
    "name_unnamed_colours",
];
