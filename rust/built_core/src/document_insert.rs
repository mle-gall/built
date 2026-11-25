// SPDX-License-Identifier: GPL-3.0-or-later
//! Import a document as one component, retaining its materials and tags.
use pyo3::{prelude::*, types::PyDict};
use std::{collections::HashSet, sync::LazyLock};
type Obj<'p> = Bound<'p, PyAny>;
#[pyfunction]
pub(crate) fn is_scale_figure(group: &Obj) -> PyResult<bool> {
    let py = group.py();
    let figure = py.import("built_core.scale_figure")?;
    if figure.call_method1("is_marked", (group,))?.is_truthy()? {
        return Ok(true);
    }
    if !group
        .getattr_opt("billboard")?
        .is_some_and(|v| v.is(pyo3::types::PyBool::new(py, true)))
    {
        return Ok(false);
    }
    if group.getattr("name")?.eq("Sumari")? {
        return Ok(true);
    }
    let suffixes = figure.getattr("LEGACY_IMAGES")?.extract::<Vec<String>>()?;
    for face in group.getattr("mesh")?.getattr("faces")?.try_iter()? {
        let attrs = face?.getattr("attrs")?;
        if !attrs.is_truthy()? {
            continue;
        }
        let texture = attrs.call_method1("get", ("texture",))?;
        if texture.is_truthy()? {
            let path = texture
                .call_method1("get", ("path", ""))?
                .str()?
                .to_string();
            let file = path.rsplit(['/', '\\']).next().unwrap_or("");
            if suffixes.iter().any(|s| file.ends_with(s)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[pyfunction]
fn component_from_scene<'p>(src: &Obj<'p>, name: &str) -> PyResult<Option<Obj<'p>>> {
    let py = src.py();
    let mut groups = Vec::new();
    for g in src.getattr("groups")?.try_iter()? {
        let g = g?;
        if !is_scale_figure(&g)? {
            groups.push(g);
        }
    }
    let mesh = src.getattr("mesh")?;
    if !mesh.getattr("faces")?.is_truthy()? && !mesh.getattr("edges")?.is_truthy()? {
        if groups.is_empty() {
            return Ok(None);
        }
        if groups.len() == 1 {
            let group = groups.pop().unwrap();
            static AUTO: LazyLock<regex::Regex> =
                LazyLock::new(|| regex::Regex::new(r"^Group \d+\n?$").unwrap());
            if group
                .getattr("name")?
                .extract::<Option<String>>()?
                .is_some_and(|s| AUTO.is_match(&s))
            {
                group.setattr("name", name)?;
            }
            return Ok(Some(group));
        }
    }
    let args = PyDict::new(py);
    args.set_item("name", name)?;
    let group = py
        .import("built_core.group")?
        .call_method("Group", (mesh,), Some(&args))?;
    if !groups.is_empty() {
        group.call_method1("adopt", (groups,))?;
    }
    Ok(Some(group))
}
fn placements<'p>(comp: &Obj<'p>) -> PyResult<impl Iterator<Item = PyResult<Obj<'p>>>> {
    Ok(comp
        .py()
        .import("built_core.group")?
        .call_method1("iter_placements", (comp,))?
        .try_iter()?
        .map(|row| row?.get_item(0)))
}
#[pyfunction]
#[pyo3(signature=(src_materials,dst_materials,comp,*,reused=None))]
fn merge_materials<'p>(
    src_materials: &Obj<'p>,
    dst_materials: &Obj<'p>,
    comp: &Obj<'p>,
    reused: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyDict>> {
    let py = comp.py();
    let renamed = PyDict::new(py);
    let register = py.import("built_core.materials")?.getattr("register")?;
    for row in src_materials.call_method0("items")?.try_iter()? {
        let row = row?;
        let key = row.get_item(0)?;
        let name = register.call1((dst_materials, row.get_item(1)?))?;
        if !key.eq(&name)? {
            renamed.set_item(key, name)?;
        }
    }
    if !renamed.is_empty() {
        let core = py.import("built_core")?;
        let mut seen = HashSet::new();
        for group in placements(comp)? {
            let group = group?;
            let paint = group.getattr("material")?;
            if paint.is_truthy()? {
                group.setattr(
                    "material",
                    core.call_method1("link_renamed_paint", (paint, &renamed))?,
                )?;
            }
            let mesh = group.getattr("mesh")?;
            if seen.insert(mesh.as_ptr())
                && !reused
                    .map(|r| r.contains(&mesh))
                    .transpose()?
                    .unwrap_or(false)
            {
                core.call_method1("link_restamp_materials", (mesh, &renamed))?;
            }
        }
    }
    Ok(renamed)
}
struct Tags {
    seen: HashSet<String>,
    ordered: Vec<String>,
    default: String,
}
impl Tags {
    fn add(&mut self, tag: Option<String>) {
        if let Some(s) = tag {
            if !s.is_empty() && s != self.default && self.seen.insert(s.clone()) {
                self.ordered.push(s);
            }
        }
    }
    fn component(&mut self, comp: &Obj) -> PyResult<()> {
        let layer = comp.py().import("built_core.layers")?.getattr("layer_of")?;
        for group in placements(comp)? {
            let group = group?;
            self.add(layer.call1((&group,))?.extract()?);
            let mesh = group.getattr("mesh")?;
            for field in ["faces", "edges"] {
                for ent in mesh.getattr(field)?.try_iter()? {
                    self.add(layer.call1((ent?,))?.extract()?);
                }
            }
        }
        Ok(())
    }
    fn new(py: Python) -> PyResult<Self> {
        Ok(Self {
            seen: HashSet::new(),
            ordered: vec![],
            default: py
                .import("built_core.layers")?
                .getattr("DEFAULT_LAYER")?
                .extract()?,
        })
    }
}
#[pyfunction]
fn used_layers(face_attrs: &Obj, edge_layers: &Obj, groups: &Obj) -> PyResult<Vec<String>> {
    let mut tags = Tags::new(groups.py())?;
    for attrs in face_attrs.try_iter()? {
        let attrs = attrs?;
        if attrs.is_truthy()? {
            tags.add(attrs.call_method1("get", ("layer",))?.extract()?);
        }
    }
    for tag in edge_layers.try_iter()? {
        tags.add(tag?.extract()?);
    }
    for comp in groups.try_iter()? {
        tags.component(&comp?)?;
    }
    Ok(tags.ordered)
}
#[pyfunction]
fn ensure_layers(dst_scene: &Obj, comp: &Obj) -> PyResult<Vec<String>> {
    let mut tags = Tags::new(comp.py())?;
    let layers = dst_scene.getattr("layers")?;
    for layer in layers.try_iter()? {
        tags.seen.insert(layer?.getattr("key")?.extract()?);
    }
    tags.component(comp)?;
    let cls = comp.py().import("built_core.layers")?.getattr("Layer")?;
    for name in &tags.ordered {
        layers.call_method1("append", (cls.call1((name,))?,))?;
    }
    Ok(tags.ordered)
}
#[pyfunction]
fn import_document_as_component<'p>(
    dst_scene: &Obj<'p>,
    src_scene: &Obj<'p>,
    name: &str,
) -> PyResult<Option<Obj<'p>>> {
    let Some(comp) = component_from_scene(src_scene, name)? else {
        return Ok(None);
    };
    let py = comp.py();
    let uid = py.import("built_core.group")?.getattr("new_uid")?;
    for group in placements(&comp)? {
        let group = group?;
        group.setattr("uid", uid.call0()?)?;
        let ifc = group.getattr("ifc")?;
        if ifc.is_truthy()? {
            let attrs = ifc.cast::<PyDict>()?.copy()?;
            attrs.call_method1("pop", ("guid", py.None()))?;
            group.setattr("ifc", attrs)?;
        }
    }
    if let Some(src) = src_scene.getattr_opt("materials")? {
        if src.is_truthy()? {
            merge_materials(&src, &dst_scene.getattr("materials")?, &comp, None)?;
        }
    }
    ensure_layers(dst_scene, &comp)?;
    Ok(Some(comp))
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let name = "built_core.document_insert";
    let sub = PyModule::new(m.py(), name)?;
    for f in [
        wrap_pyfunction!(is_scale_figure, &sub)?,
        wrap_pyfunction!(component_from_scene, &sub)?,
        wrap_pyfunction!(merge_materials, &sub)?,
        wrap_pyfunction!(used_layers, &sub)?,
        wrap_pyfunction!(ensure_layers, &sub)?,
        wrap_pyfunction!(import_document_as_component, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(name, &sub)?;
    m.add("document_insert", sub)
}
