// SPDX-License-Identifier: GPL-3.0-or-later
//! Inspector identity for a scene selection. Targets use the Project tree's
//! occurrence addresses, so two placements of one definition stay distinct.
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::BTreeMap;
use std::path::Path;

type Field = (String, String, String, Option<bool>);
const REVEAL: &str = "revealselection\x1f";
const ROOT: &str = "documentroot\x1f";

fn attr(obj: &Bound<'_, PyAny>, key: &str) -> Option<String> {
    obj.getattr(key).ok()?.extract().ok()
}

fn field(label: &str, value: impl Into<String>, action: impl Into<String>) -> Field {
    (label.into(), value.into(), action.into(), None)
}

fn paint(obj: &Bound<'_, PyAny>, field: &str) -> Option<String> {
    obj.getattr(field)
        .ok()?
        .call_method1("get", ("mat",))
        .ok()?
        .extract()
        .ok()
}

#[derive(Default)]
struct Materials {
    counts: BTreeMap<String, usize>,
    faces: usize,
    sampled: bool,
}

impl Materials {
    fn face(&mut self, obj: &Bound<'_, PyAny>, fallback: &Option<String>) {
        let name = paint(obj, "attrs")
            .or_else(|| fallback.clone())
            .unwrap_or_default();
        *self.counts.entry(name).or_default() += 1;
        self.faces += 1;
    }

    /// A mesh not built yet: its faces' materials counted from the loader's
    /// arrays (the attribute row of each face), exactly, without building it.
    fn deferred(&mut self, mesh: &Bound<'_, PyAny>, fallback: Option<String>) -> PyResult<()> {
        let py = mesh.py();
        let source = mesh.getattr("__dict__")?.call_method1("get", ("_deferred_source",))?;
        if source.is_none() {
            if let Some(name) = fallback {
                self.counts.entry(name).or_default();
            }
            return Ok(());
        }
        let rows: Vec<u8> = py
            .import("numpy")?
            .call_method1("asarray", (source.get_item("face_attr")?, "<i8"))?
            .call_method0("tobytes")?
            .extract()?;
        let mut per_row = BTreeMap::<i64, usize>::new();
        for chunk in rows.chunks_exact(8) {
            *per_row.entry(i64::from_le_bytes(chunk.try_into().unwrap())).or_default() += 1;
        }
        if per_row.is_empty() {
            if let Some(name) = fallback {
                self.counts.entry(name).or_default();
            }
            return Ok(());
        }
        let table = source.get_item("meta")?.call_method1("get", ("face_attrs",))?;
        let decode = py.import("built_core")?.getattr("igz_face_attrs_from_json")?;
        for (row, n) in per_row {
            let mut name = None;
            if row >= 0 && !table.is_none() {
                let attrs = decode.call1((table.get_item(row)?,))?;
                if !attrs.is_none() {
                    name = attrs.call_method1("get", ("mat",))?.extract::<Option<String>>()?;
                }
            }
            *self.counts.entry(name.or_else(|| fallback.clone()).unwrap_or_default()).or_default() += n;
            self.faces += n;
        }
        Ok(())
    }

    fn visit(&mut self, obj: &Bound<'_, PyAny>, fallback: Option<String>) -> PyResult<()> {
        if self.sampled {
            return Ok(());
        }
        if let Ok(mesh) = obj.getattr("mesh") {
            // Selecting an imported part must not build its mesh.
            if mesh.get_type().is(&mesh.py().import("built_core.mesh")?.getattr("DeferredMesh")?) {
                return self.deferred(&mesh, fallback);
            }
            let faces = mesh.getattr("faces")?;
            if faces.len()? == 0 {
                if let Some(name) = fallback {
                    self.counts.entry(name).or_default();
                }
                return Ok(());
            }
            for face in faces.try_iter()? {
                if self.faces == 4096 {
                    self.sampled = true;
                    break;
                }
                self.face(&face?, &fallback);
            }
        } else if obj.hasattr("loop")? {
            if self.faces == 4096 {
                self.sampled = true;
            } else {
                self.face(obj, &fallback);
            }
        }
        Ok(())
    }

    /// The materials of the active mesh's selected faces (`keys`), every
    /// one of them, without a handle each: a face's own attributes
    /// (`_attr_own`) or its table row, the row's `mat` read once. False
    /// when the store cannot answer (the caller samples the handles then).
    fn bulk_faces(&mut self, mesh: &Bound<'_, PyAny>, keys: &Bound<'_, PyAny>, fallback: &Option<String>) -> PyResult<bool> {
        let py = mesh.py();
        let (Ok(store), Ok(table)) = (mesh.getattr("_store"), mesh.getattr("_attr_table")) else { return Ok(false) };
        if !store.hasattr("element_tags")? {
            return Ok(false);
        }
        let (rows, _): (Vec<i64>, Vec<String>) = store.call_method1("element_tags", ("face", keys))?.extract()?;
        let keys: Vec<u128> = keys.extract()?;
        if rows.iter().any(|&r| r == -2) {
            return Ok(false);
        }
        let own = mesh.getattr("_attr_own").ok().and_then(|o| o.cast_into::<PyDict>().ok()).filter(|o| !o.is_empty());
        let mat = |attrs: &Bound<'_, PyAny>| -> Option<String> { attrs.call_method1("get", ("mat",)).ok()?.extract().ok() };
        // Counted by table row, then named once per row: a select-all of a
        // big mesh is hundreds of thousands of faces and a handful of rows.
        // Only the few faces with attributes of their own are looked up.
        let own_keys: std::collections::HashSet<u128> = match &own {
            Some(own) => own.keys().iter().filter_map(|k| k.extract::<u128>().ok()).collect(),
            None => Default::default(),
        };
        let mut per_row: BTreeMap<i64, usize> = BTreeMap::new();
        let mut named: BTreeMap<Option<String>, usize> = BTreeMap::new();
        for (key, row) in keys.into_iter().zip(rows) {
            self.faces += 1;
            if own_keys.contains(&key) {
                if let Some(attrs) = own.as_ref().map(|own| own.get_item(key)).transpose()?.flatten().filter(|d| !d.is_none()) {
                    *named.entry(mat(&attrs)).or_default() += 1;
                    continue;
                }
            }
            *per_row.entry(row).or_default() += 1;
        }
        for (row, n) in per_row {
            let name = if row < 0 { None } else { mat(&table.get_item(row)?) };
            *named.entry(name).or_default() += n;
        }
        for (name, n) in named {
            *self.counts.entry(name.or_else(|| fallback.clone()).unwrap_or_default()).or_default() += n;
        }
        let _ = py;
        Ok(true)
    }

    fn field(self) -> Field {
        let kinds = self.counts.len();
        let (name, count) = self
            .counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .unwrap_or_default();
        let mut text = if name.is_empty() {
            "Default".into()
        } else {
            name.clone()
        };
        if count > 0 {
            text.push_str(&format!(
                " · {count} {}",
                if count == 1 { "face" } else { "faces" }
            ));
            if kinds > 1 {
                text.push_str(&format!(" · +{}", kinds - 1));
            }
        }
        if self.sampled {
            text.push_str(" (sample)");
        }
        let action = if name.is_empty() {
            "insp:lib:browse:materials".into()
        } else {
            format!("insp:lib:edit:m:{name}")
        };
        field("Material", text, action)
    }
}

#[pyfunction]
pub(crate) fn selection_identity(
    scene: &Bound<'_, PyAny>,
    data: &Bound<'_, PyDict>,
    filename: &str,
    kinds: &Bound<'_, PyDict>,
) -> PyResult<Vec<Field>> {
    let filename = Path::new(filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    let tree = data.get_item("tree")?;
    let member = data.get_item("node_of")?;
    let root = scene.getattr("_root")?;
    let nodes = data.get_item("nodes")?;
    let mut result: Vec<Field> = vec![];
    let mut materials = Materials::default();
    let mut inside_differs = false;
    // One entry's fields, merged into `result` ("Mixed" where they differ);
    // `own_layer` stands for the entry's own tag when given, `visit` counts
    // its material. The fallback material is returned for its kin.
    let one = |obj: Bound<'_, PyAny>, own_layer: Option<Option<String>>, visit: bool, result: &mut Vec<Field>,
               materials: &mut Materials, inside_differs: &mut bool| -> PyResult<Option<String>> {
        let mut key: Option<String> = member
            .as_ref()
            .and_then(|m| m.get_item(obj.as_ptr() as usize).ok()?.extract().ok());
        let filing_target = key
            .as_ref()
            .map(|k| format!("revealnode\x1fn:{k}"))
            .unwrap_or_default();
        let mut path = Vec::new();
        let mut source = None;
        let mut class = ifc_class(&obj);
        while let (Some(k), Some(table)) = (key.take(), nodes.as_ref()) {
            let node = table.get_item(k)?;
            path.push(node.get_item("name")?.extract::<String>()?);
            if class.is_none() && path.len() == 1 {
                class = node
                    .get_item("recipe")?
                    .call_method1("get", ("class",))?
                    .extract()
                    .ok();
            }
            if node.get_item("kind")?.extract::<String>()? == "link" {
                let file = node.get_item("path")?.extract::<String>()?;
                if let Some(name) = Path::new(&file).file_name().and_then(|s| s.to_str()) {
                    *path.last_mut().unwrap() = name.to_owned();
                    source.get_or_insert_with(|| name.to_owned());
                }
            }
            key = node.get_item("parent")?.extract().ok();
        }
        path.reverse();
        let kind = match class {
            Some(c) => kinds
                .get_item(&c)?
                .and_then(|v| v.extract().ok())
                .unwrap_or_else(|| c.trim_start_matches("Ifc").into()),
            None if obj.hasattr("is_component")? && obj.call_method0("is_component")?.is_truthy()? => "Component".into(),
            None => match obj.get_type().name()?.to_str()? {
                "EdgeView" => "Edge",
                "FaceView" => "Face",
                "VertexView" => "Vertex",
                "SectionPlane" => "Section plane",
                "ConstructionLine" | "GuideLine" | "GuidePoint" => "Guide",
                "TextLabel" => "Text label",
                "ImagePlane" => "Image",
                name => name,
            }.to_owned(),
        };
        let mut parent = obj
            .getattr("_parent")
            .or_else(|_| scene.getattr("edit_group"))?;
        let mut parent_action = ROOT.to_owned();
        let mut inside = Vec::new();
        let mut fallback = paint(&obj, "material");
        while !parent.is_none() && !parent.is(&root) {
            if inside.is_empty() {
                if let Some(t) = &tree {
                    parent_action = format!(
                        "open\x1fi:{}",
                        t.call_method1("row_id", (&parent,))?.extract::<String>()?
                    );
                }
            }
            inside.push(
                attr(&parent, "name")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Group".into()),
            );
            fallback = fallback.or_else(|| paint(&parent, "material"));
            parent = parent.getattr("_parent")?;
        }
        inside.push(filename.into());
        inside.reverse();
        let layer = match own_layer {
            Some(layer) => layer,
            None => attr(&obj, "layer").or_else(|| {
                obj.getattr("attrs")
                    .ok()?
                    .call_method1("get", ("layer",))
                    .ok()?
                    .extract()
                    .ok()
            }),
        };
        let tag = scene.call_method1("layer", (layer,))?;
        let mut tag_field = if tag.is_none() {
            field("Tag", "Untagged", "")
        } else {
            field(
                "Tag",
                if tag.getattr("key")?.eq("Layer 0")? { "Untagged".into() } else { tag.getattr("name")?.extract::<String>()? },
                format!(
                    "eye\x1fpreview-tag:{}",
                    tag.getattr("key")?.extract::<String>()?
                ),
            )
        };
        if !tag.is_none() {
            tag_field.3 = Some(tag.getattr("visible")?.is_truthy()?);
        }
        let linked = source.is_some();
        let file = source.unwrap_or_else(|| filename.into());
        let inside = inside.join(" › ");
        *inside_differs |= inside != file;
        if visit {
            materials.visit(&obj, fallback.clone())?;
        }
        let fields = vec![
            field("Kind", kind, REVEAL),
            field(
                "Filed in",
                if path.is_empty() {
                    "—".into()
                } else {
                    path.join(" › ")
                },
                filing_target,
            ),
            tag_field,
            field(
                "Inside",
                inside,
                if linked {
                    "insp:linkfile".into()
                } else {
                    parent_action
                },
            ),
            field(
                "File",
                file,
                if linked { "insp:linkfile" } else { ROOT },
            ),
        ];
        if result.is_empty() {
            *result = fields;
        } else {
            for (previous, next) in result.iter_mut().zip(fields) {
                if *previous != next {
                    previous.1 = "Mixed".into();
                    previous.2.clear();
                    previous.3 = None;
                }
            }
        }
        Ok(fallback)
    };
    let selection = scene.getattr("selection")?;
    let mesh = scene.getattr("mesh")?;
    let bulk = mesh.getattr("_store").ok().is_some_and(|s| s.hasattr("distinct_tags").unwrap_or(false));
    match bulk {
        // A big selection: placements and the rest one by one; the active
        // mesh's faces and edges once per kind and distinct tag (what else
        // tells them apart is the same for all: kind, place, file), their
        // materials sampled as ever. A select-all of a 600k-element mesh
        // was a second of attribute reads per change.
        true if selection.hasattr("values_of")? && selection.len()? > crate::model::layers_api::BULK_SELECTION => {
            for kind in ["placement", "other", "vertex"] {
                for obj in selection.call_method1("values_of", (kind,))?.try_iter()? {
                    one(obj?, None, true, &mut result, &mut materials, &mut inside_differs)?;
                }
            }
            for kind in ["face", "edge"] {
                let values = selection.call_method1("values_of", (kind,))?;
                if values.len()? == 0 {
                    continue;
                }
                let Some(tags) = crate::model::layers_api::mesh_element_tags(scene, &selection, kind)? else {
                    for obj in values.try_iter()? {
                        one(obj?, None, true, &mut result, &mut materials, &mut inside_differs)?;
                    }
                    continue;
                };
                let first = values.get_item(0)?;
                let mut fallback = None;
                // Faces' materials are counted in bulk below, all of them;
                // edges count none.
                for tag in tags {
                    fallback = one(first.clone(), Some(tag), false, &mut result, &mut materials, &mut inside_differs)?;
                }
                if kind == "face" && !materials.bulk_faces(&mesh, &selection.call_method1("element_keys", ("face",))?, &fallback)? {
                    for obj in values.try_iter()? {
                        if materials.sampled {
                            break;
                        }
                        materials.visit(&obj?, fallback.clone())?;
                    }
                }
            }
        }
        _ => {
            for obj in selection.try_iter()? {
                one(obj?, None, true, &mut result, &mut materials, &mut inside_differs)?;
            }
        }
    }
    if !result.is_empty() {
        result.insert(4, materials.field());
        if !inside_differs { result.retain(|row| row.0 != "Inside"); }
    }
    Ok(result)
}

fn ifc_class(obj: &Bound<'_, PyAny>) -> Option<String> {
    obj.getattr("ifc")
        .ok()?
        .call_method1("get", ("class",))
        .ok()?
        .extract()
        .ok()
}
