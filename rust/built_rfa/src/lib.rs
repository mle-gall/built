//! .rfa family reader for Built.
//!
//! Alongside its parametric forms, a family file stores the display meshes for
//! nested family types and placed elements. rvt-rs decodes those meshes (`native_saved_scene`). This
//! module turns them into one JSON document per file, schema
//! `built.rfa/1`: the family's types (from `PartAtom`), each composed of
//! meshes with transforms, plus materials, graphics styles, the preview
//! image and warnings. `formats/rfa.py` builds the scene from it.
//!
//! Per type, the geometry is chosen as follows. A type whose `PartAtom`
//! parameters name a nested type (`Nested Family : Type`) swaps every placed
//! instance of that nested family for that type's saved mesh, under the
//! instance's own transform; the instance's own saved mesh is kept when it
//! already shows that type (it then also carries the instance's own
//! visibility choices). A type with no such link gets the family as saved.

use anyhow::{Context, Result};
use base64::Engine;
use rvt::research::native_saved_scene;
use rvt::research::{native_document::Options, native_form_geometry, native_parameters};
use rvt::RevitFile;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "built.rfa/1";
const FOOT: f64 = 0.3048;
/// Built-in category of the saved-graphics style .rfa families use for
/// clearance and access zones (seen on Schneider's Prisma P families).
const ZONE_CATEGORIES: &[i64] = &[-2009512];

/// Classes of the main document's records this reader keeps decoded.
const KEPT: &[&str] = &[
    "Family",
    "FamilySymbol",
    "FamSymSurrogate",
    "FamilySurrogate",
    "FamilyInstance",
    "MaterialElem",
    "GStyleElem",
    "CategoryElem",
    "Text3dElem",
];

/// Classes of the elements placed in a family whose saved mesh is part of
/// the family's body. Others (filled regions, symbols, annotations) are 2D or
/// definitions.
const PLACED: &[&str] = &[
    "FamilyInstance",
    "ExtrusionElem",
    "BlendElem",
    "SweepElem",
    "SweptBlendElem",
    "RevolutionElem",
    "FreeFormElement",
    "GenericFormElem",
    "Text3dElem",
];

fn id_of(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::Object(m) => m.get("m_id").or_else(|| m.get("m_id64")).and_then(id_of),
        _ => None,
    }
}

struct Rec {
    class: String,
    objects: Vec<(String, Value)>,
}

impl Rec {
    fn root(&self) -> &Value {
        &self.objects[0].1
    }
    fn obj(&self, class: &str) -> Option<&Value> {
        self.objects
            .iter()
            .find(|(c, _)| c == class)
            .map(|(_, v)| v)
    }
    fn name(&self) -> Option<String> {
        self.objects
            .iter()
            .find_map(|(_, v)| v.get("m_name").and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }
}

type Mat4 = [[f64; 4]; 4];
const IDENTITY: Mat4 = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

/// An instance's transform from its `InstanceInfo.m_Trf`, rows as saved
/// (rvt-rs's `read_transform`), translation in metres.
fn instance_transform(rec: &Rec) -> Option<Mat4> {
    let trf = rec.obj("InstanceInfo")?.get("m_Trf")?;
    let rows = trf.get("m_3x3")?.as_array()?;
    let or = trf.get("m_or")?.as_array()?;
    let mut m = IDENTITY;
    for i in 0..3 {
        let row = rows.get(i)?.as_array()?;
        for j in 0..3 {
            m[i][j] = row.get(j)?.as_f64()?;
        }
        m[i][3] = or.get(i)?.as_f64()? * FOOT;
    }
    Some(m)
}

fn flat(m: &Mat4) -> Value {
    json!(m.iter().flatten().copied().collect::<Vec<f64>>())
}

/// `0xBBGGRR` colour order in .rfa files, to sRGB bytes.
fn rgb(color: i64) -> [u8; 3] {
    [
        (color & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        ((color >> 16) & 0xff) as u8,
    ]
}

struct Part {
    solid: usize,
    material: Option<i64>,
    style: Option<i64>,
    hidden: bool,
    positions: Vec<f64>,
    indices: Vec<u32>,
}

pub fn read_family_json(path: &str) -> Result<String> {
    Ok(serde_json::to_string(&read_family(path)?)?)
}

pub fn read_family(path: &str) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    let sha = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    drop(bytes);
    let mut rf = RevitFile::open_with_limits(path, rvt::cli::open_limits(None, None, true))
        .with_context(|| format!("opening {path} as an .rfa file"))?;
    let version = rf.basic_file_info().ok().map(|b| b.version);
    let mut warnings: Vec<String> = Vec::new();

    // Family identity and types, from PartAtom.
    let atom = rf
        .part_atom()
        .context("the file has no PartAtom: not a .rfa family")?;
    let omniclass = atom.omniclass.clone();
    let category = atom
        .categories
        .iter()
        .map(|c| c.term.clone())
        .find(|t| Some(t) != omniclass.as_ref());
    let family_name = atom
        .entry_title
        .clone()
        .or_else(|| {
            std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let atom_types = atom.family_types().unwrap_or_default();

    let preview = rf
        .preview_png()
        .ok()
        .map(|png| base64::engine::general_purpose::STANDARD.encode(png));

    let mut out = Map::new();
    out.insert("schema".into(), json!(SCHEMA));
    out.insert(
        "source".into(),
        json!({
            "file_name": std::path::Path::new(path).file_name().map(|s| s.to_string_lossy()),
            "sha256": sha,
            "revit_version": version,
        }),
    );
    out.insert(
        "family".into(),
        json!({
            "name": family_name,
            "category": category,
            "omniclass": omniclass,
            "updated": atom.updated,
        }),
    );
    out.insert("preview_png_base64".into(), json!(preview));

    let geometry = decode_geometry(&mut rf);
    let (records, classes, scene) = match geometry {
        Ok(v) => v,
        Err(error) => {
            warnings.push(format!(
                "No geometry read from this file ({error:#}); each type is a box of its Width, Depth and Height."
            ));
            let types: Vec<Value> = atom_types
                .iter()
                .map(|t| type_json(t, vec![], "none"))
                .collect();
            out.insert("materials".into(), json!({}));
            out.insert("styles".into(), json!({}));
            out.insert("geometries".into(), json!({}));
            out.insert("types".into(), json!(types));
            out.insert("warnings".into(), json!(warnings));
            return Ok(Value::Object(out));
        }
    };

    // Saved meshes by owner, grouped into parts.
    let mut meshes: BTreeMap<u64, Vec<Part>> = BTreeMap::new();
    for element in &scene.elements {
        let mut parts: BTreeMap<(Option<u64>, usize, Option<i64>, Option<i64>, bool), Part> =
            BTreeMap::new();
        for p in &element.meshes.primitives {
            if p.triangles.is_empty() || p.filtered {
                continue;
            }
            let material = p
                .material_id
                .or(Some(p.render_style_id))
                .filter(|m| *m >= 0);
            let key = (
                p.source_owner_id,
                p.object_index,
                material,
                p.style_id,
                p.hidden,
            );
            let part = parts.entry(key).or_insert_with(|| Part {
                solid: p.object_index,
                material,
                style: p.style_id,
                hidden: p.hidden,
                positions: vec![],
                indices: vec![],
            });
            let base = (part.positions.len() / 3) as u32;
            for v in &p.vertices {
                part.positions.extend(v.iter().map(|c| c * FOOT));
            }
            for t in &p.triangles {
                part.indices.extend(t.iter().map(|i| base + i));
            }
        }
        if !parts.is_empty() {
            let mut parts: Vec<Part> = parts.into_values().collect();
            // A 3D text is a note (a supplier's contact line, a label): kept,
            // hidden.
            if classes.get(&element.id).map(String::as_str) == Some("Text3dElem") {
                for p in &mut parts {
                    p.hidden = true;
                }
            }
            meshes.insert(element.id, parts);
        }
    }

    // Nested families: symbol -> (family id, name), family id -> name.
    let mut symbol_family: BTreeMap<i64, i64> = BTreeMap::new();
    let mut symbol_name: BTreeMap<i64, String> = BTreeMap::new();
    let mut family_name_of: BTreeMap<i64, String> = BTreeMap::new();
    let surrogate_family: BTreeMap<i64, i64> = records
        .iter()
        .filter(|(_, r)| r.class == "FamilySurrogate")
        .filter_map(|(id, r)| Some((*id as i64, r.root().get("m_elemId").and_then(id_of)?)))
        .collect();
    for (id, r) in &records {
        match r.class.as_str() {
            "FamSymSurrogate" => {
                let Some(symbol) = r.root().get("m_elemId").and_then(id_of) else {
                    continue;
                };
                if let Some(name) = r.name() {
                    symbol_name.insert(symbol, name);
                }
                if let Some(family) = r
                    .root()
                    .get("m_famSurrogateId")
                    .and_then(id_of)
                    .and_then(|s| surrogate_family.get(&s))
                {
                    symbol_family.insert(symbol, *family);
                }
            }
            "Family" => {
                if let Some(name) = r.name() {
                    family_name_of.insert(*id as i64, name);
                }
            }
            _ => {}
        }
    }

    // What is placed in the family, as saved: every owner of a mesh that is
    // not a type of a nested family.
    let placed: Vec<u64> = meshes
        .keys()
        .copied()
        .filter(|id| {
            classes
                .get(id)
                .is_some_and(|c| PLACED.contains(&c.as_str()))
        })
        .collect();
    if placed.is_empty() {
        warnings.push("The family as saved shows no 3D geometry.".into());
    }

    let mut used: BTreeSet<u64> = BTreeSet::new();
    let mut types = Vec::new();
    let mut warned_nested = false;
    for t in &atom_types {
        // A type parameter naming a nested type: "Family : Type".
        let link = t.parameters.iter().find_map(|p| {
            let (fam, sym) = p.value.split_once(" : ")?;
            let fam = fam.trim();
            let sym = sym.trim();
            symbol_name
                .iter()
                .find(|(s, n)| {
                    n.as_str() == sym
                        && symbol_family
                            .get(s)
                            .and_then(|f| family_name_of.get(f))
                            .is_none_or(|f| f == fam)
                })
                .map(|(s, _)| (*s, symbol_family.get(s).copied()))
        });
        let mut components = Vec::new();
        let mut source = "as_saved";
        for owner in &placed {
            let rec = records.get(owner);
            let instance = rec.filter(|r| r.class == "FamilyInstance");
            if let (Some((symbol, family)), Some(inst)) = (link, instance) {
                let master = inst.root().get("m_masterSymbolId").and_then(id_of);
                let inst_family = master.and_then(|m| symbol_family.get(&m).copied());
                if family.is_some() && inst_family == family {
                    if master == Some(symbol) {
                        source = "saved_instance";
                    } else if meshes.contains_key(&(symbol as u64)) {
                        let transform = instance_transform(inst).unwrap_or(IDENTITY);
                        components.push(
                            json!({"geometry": symbol.to_string(), "transform": flat(&transform)}),
                        );
                        used.insert(symbol as u64);
                        source = "nested_type";
                        if !warned_nested {
                            warned_nested = true;
                            warnings.push(format!(
                                "Type \"{}\": drawn from nested type {}'s own defaults; settings applied to the placed instance (plinth height, door hand) may differ.",
                                t.name,
                                symbol_name.get(&symbol).cloned().unwrap_or_default()
                            ));
                        }
                        continue;
                    } else {
                        warnings.push(format!(
                            "Type \"{}\": no saved mesh for nested type {}; shown as saved.",
                            t.name,
                            symbol_name.get(&symbol).cloned().unwrap_or_default()
                        ));
                    }
                }
            }
            components.push(json!({"geometry": owner.to_string(), "transform": flat(&IDENTITY)}));
            used.insert(*owner);
        }
        if components.is_empty() {
            source = "none";
        } else if source == "as_saved" && atom_types.len() > 1 {
            warnings.push(format!(
                "Type \"{}\": shown as the family was saved (its own geometry is not linked to a nested type).",
                t.name
            ));
        }
        types.push(type_json(t, components, source));
    }
    if atom_types.is_empty() && !placed.is_empty() {
        let components: Vec<Value> = placed
            .iter()
            .map(|o| {
                used.insert(*o);
                json!({"geometry": o.to_string(), "transform": flat(&IDENTITY)})
            })
            .collect();
        types.push(json!({"name": family_name, "parameters": [], "components": components, "geometry_source": "as_saved"}));
    }

    // Styles, materials and geometries actually used.
    let mut style_ids = BTreeSet::new();
    let mut material_ids = BTreeSet::new();
    for id in &used {
        for p in &meshes[id] {
            style_ids.extend(p.style);
            material_ids.extend(p.material);
        }
    }
    let subcategory_of_style: BTreeMap<i64, String> = records
        .values()
        .filter(|r| r.class == "CategoryElem")
        .filter_map(|r| {
            let name = r.obj("Category")?.get("m_name")?.as_str()?.to_owned();
            let ids = r
                .root()
                .get("m_gstyleIds")?
                .as_array()?
                .iter()
                .filter_map(id_of)
                .collect::<Vec<_>>();
            Some(ids.into_iter().map(move |g| (g, name.clone())))
        })
        .flatten()
        .filter(|(_, n)| !n.is_empty())
        .collect();
    let mut styles = Map::new();
    let mut zone_styles = BTreeSet::new();
    for s in &style_ids {
        let category_id = records
            .get(&(*s as u64))
            .and_then(|r| r.root().get("m_categoryId"))
            .and_then(id_of);
        let sub = subcategory_of_style.get(s).cloned();
        let zone = category_id.is_some_and(|c| ZONE_CATEGORIES.contains(&c))
            || sub.as_deref().is_some_and(|n| {
                let n = n.to_lowercase();
                [
                    "clearance",
                    "zone",
                    "access",
                    "maintenance",
                    "dégagement",
                    "degagement",
                ]
                .iter()
                .any(|k| n.contains(k))
            });
        if zone {
            zone_styles.insert(*s);
        }
        styles.insert(
            s.to_string(),
            json!({"category": category, "category_id": category_id, "subcategory": sub, "zone": zone}),
        );
    }
    let mut materials = Map::new();
    for m in &material_ids {
        let Some(rec) = records.get(&(*m as u64)) else {
            continue;
        };
        let mat = rec.obj("Material");
        let name = mat
            .and_then(|v| v.get("m_name"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| rec.name())
            .unwrap_or_else(|| format!("Material {m}"));
        let color = mat.and_then(|v| v.get("m_color")).and_then(Value::as_i64);
        let transparency = mat
            .and_then(|v| v.get("m_transparency"))
            .and_then(Value::as_f64);
        materials.insert(
            m.to_string(),
            json!({"name": name, "rgb": color.map(rgb), "transparency": transparency}),
        );
    }
    let mut geometries = Map::new();
    for id in &used {
        let label = symbol_name
            .get(&(*id as i64))
            .map(|s| {
                match symbol_family
                    .get(&(*id as i64))
                    .and_then(|f| family_name_of.get(f))
                {
                    Some(f) => format!("{f} : {s}"),
                    None => s.clone(),
                }
            })
            .or_else(|| {
                let r = records.get(id)?;
                let text = text_in(&Value::Array(
                    r.objects.iter().map(|(_, v)| v.clone()).collect(),
                ));
                if r.class == "FamilyInstance" {
                    let master = r.root().get("m_masterSymbolId").and_then(id_of)?;
                    let sym = symbol_name.get(&master)?;
                    return Some(
                        match symbol_family
                            .get(&master)
                            .and_then(|f| family_name_of.get(f))
                        {
                            Some(f) => format!("{f} : {sym} (as placed)"),
                            None => format!("{sym} (as placed)"),
                        },
                    );
                }
                Some(match (r.class.as_str(), text) {
                    ("Text3dElem", Some(t)) => format!("Model text: {t}"),
                    _ => format!("{} {id}", r.class),
                })
            })
            .unwrap_or_else(|| format!("element {id}"));
        let parts: Vec<Value> = meshes[id]
            .iter()
            .map(|p| {
                json!({
                    "solid": p.solid,
                    "material": p.material.map(|m| m.to_string()),
                    "style": p.style.map(|s| s.to_string()),
                    // A zone's visibility is the "Clearance zones" tag's.
                    "hidden": p.hidden && !p.style.is_some_and(|s| zone_styles.contains(&s)),
                    "zone": p.style.is_some_and(|s| zone_styles.contains(&s)),
                    "positions": p.positions,
                    "indices": p.indices,
                })
            })
            .collect();
        geometries.insert(id.to_string(), json!({"label": label, "parts": parts}));
    }
    out.insert("materials".into(), Value::Object(materials));
    out.insert("styles".into(), Value::Object(styles));
    out.insert("geometries".into(), Value::Object(geometries));
    out.insert("types".into(), json!(types));
    out.insert("warnings".into(), json!(warnings));
    Ok(Value::Object(out))
}

/// The longest text of a 3D text's record (`m_text`/`m_str…`, any depth):
/// its words rather than its font name.
fn text_in(v: &Value) -> Option<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    let k = k.to_lowercase();
                    match v {
                        Value::String(s) if k.contains("text") || k.starts_with("m_str") => {
                            out.push(s.clone())
                        }
                        _ => walk(v, out),
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(v, &mut found);
    found
        .into_iter()
        .max_by_key(|s| s.chars().count())
        .filter(|s| s.len() > 1)
}

fn type_json(t: &rvt::part_atom::FamilyType, components: Vec<Value>, source: &str) -> Value {
    let parameters: Vec<Value> = t
        .parameters
        .iter()
        .map(|p| {
            json!({
                "name": p.display_name.clone().unwrap_or_else(|| p.name.clone()),
                "value": p.value,
                "units": p.units,
                "kind": p.type_of_parameter,
            })
        })
        .collect();
    json!({"name": t.name, "parameters": parameters, "components": components, "geometry_source": source})
}

/// The main document's kept records and the decoded saved graphics.
fn decode_geometry(
    rf: &mut RevitFile,
) -> Result<(
    BTreeMap<u64, Rec>,
    BTreeMap<u64, String>,
    native_saved_scene::SavedScene,
)> {
    let doc = native_form_geometry::open(rf)?;
    let mut records = BTreeMap::new();
    let mut classes = BTreeMap::new();
    native_form_geometry::walk(rf, &doc, &[102], |e| {
        if let Some(class) = e.class {
            classes.insert(e.id, class.to_owned());
        }
        let Some(class) = e.class.filter(|c| KEPT.contains(c)) else {
            return Ok(());
        };
        if let Ok(graph) = native_parameters::decode_graph(e.body, &doc.registry) {
            let objects = graph
                .objects
                .iter()
                .map(|o| (o.class_name.clone(), o.fields.to_json()))
                .collect::<Vec<_>>();
            if !objects.is_empty() {
                records.insert(
                    e.id,
                    Rec {
                        class: class.to_owned(),
                        objects,
                    },
                );
            }
        }
        Ok(())
    })?;
    let mut options = Options::default();
    options.releases = 2011..=2027;
    // A 4-column switchboard's symbol graphics hold ~10^5 values, rvt-rs's
    // default budget; families are small files, so allow more.
    options.max_graph_values = options.max_graph_values.max(5_000_000);
    options.max_graph_objects = options.max_graph_objects.max(1_000_000);
    let scene = native_saved_scene::extract_including_hidden(rf, &options)?;
    Ok((records, classes, scene))
}

#[pyo3::pymodule]
mod built_rfa {
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;

    /// Read a .rfa family file; returns the `built.rfa/1` JSON text.
    #[pyfunction]
    fn read_family(py: Python<'_>, path: String) -> PyResult<String> {
        py.detach(|| super::read_family_json(&path))
            .map_err(|e| PyValueError::new_err(format!("{e:#}")))
    }

    #[pymodule_export]
    const SCHEMA: &str = super::SCHEMA;
}
