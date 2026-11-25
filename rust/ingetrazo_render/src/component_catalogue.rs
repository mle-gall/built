//! The online scope of Library's Components tab. The document host supplies
//! the catalogue and files. Browsing, selection and preview demand live here.
pub(crate) mod session;
use crate::extension_previews::{decode, read};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

fn field<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or_default()
}
fn name<'a>(row: &'a Value, lang: &str) -> &'a str {
    let english = field(row, "nombre_en");
    if !lang.starts_with("es") && !english.is_empty() {
        return english;
    }
    row["nombre"]
        .as_str()
        .or_else(|| row["id"].as_str())
        .unwrap_or("?")
}
fn category(raw: &str) -> &str {
    match raw {
        "Cocina" => "Kitchen",
        "Cuarto de Baño" => "Bathroom",
        "Dormitorio" => "Bedroom",
        "Escaleras" => "Staircases",
        "Exterior" => "Exterior",
        "Iluminación" => "Lights",
        "Oficina" => "Office",
        "Personajes" => "People",
        "Puertas y Ventanas" => "Doors and windows",
        "Salón" => "Living room",
        "Varios" => "Miscellaneous",
        "Vehículos" => "Vehicles",
        other => other,
    }
}

fn credit(row: &Value, lang: &str, tr: &Bound<'_, PyAny>) -> PyResult<String> {
    let kwargs = PyDict::new(tr.py());
    let license = ["licencia_nombre", "licencia"]
        .iter()
        .map(|key| field(row, key))
        .find(|s| !s.is_empty())
        .unwrap_or("—");
    kwargs.set_item("licence", tr.call1((license,))?)?;
    let author = field(row, "autor");
    kwargs.set_item(
        "author",
        if author.is_empty() {
            tr.call1(("unknown author",))?.extract::<String>()?
        } else {
            author.into()
        },
    )?;
    kwargs.set_item("name", name(row, lang))?;
    tr.call(("{name} · {licence} · by {author}",), Some(&kwargs))?
        .extract()
}

#[pyclass]
#[derive(Default)]
pub struct ComponentCatalogue {
    entries: BTreeMap<String, Value>,
    category: String,
    focus: String,
    prefer_online: bool,
    shown: BTreeSet<String>,
    visible: Vec<String>,
    decoded: BTreeSet<String>,
    ready: bool,
    inserting: Option<String>,
    failed: Option<String>,
}
#[pymethods]
impl ComponentCatalogue {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    #[staticmethod]
    fn attribution(entry: &Bound<'_, PyDict>) -> PyResult<BTreeMap<String, String>> {
        let mut record = BTreeMap::new();
        for key in [
            "id",
            "nombre",
            "nombre_en",
            "autor",
            "licencia",
            "licencia_nombre",
        ] {
            if let Some(value) = entry
                .get_item(key)?
                .and_then(|v| v.extract::<String>().ok())
            {
                record.insert(key.into(), value);
            }
        }
        Ok(record)
    }

    #[staticmethod]
    fn credit_line(data: &str, lang: &str, tr: &Bound<'_, PyAny>) -> PyResult<String> {
        let row = serde_json::from_str(data).map_err(|e| PyValueError::new_err(e.to_string()))?;
        credit(&row, lang, tr)
    }

    fn load(&mut self, data: &str) -> PyResult<()> {
        let rows: Vec<Value> =
            serde_json::from_str(data).map_err(|e| PyValueError::new_err(e.to_string()))?;
        self.entries.clear();
        for mut row in rows {
            let id = field(&row, "id").to_owned();
            if id.is_empty()
                || id.len() > 128
                || !id.as_bytes()[0].is_ascii_alphanumeric()
                || id.contains("..")
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            {
                continue;
            }
            // Models are flat archives. Use the importer's OBJ discovery if
            // an old catalogue names a path outside the model folder.
            if field(&row, "obj").contains(['/', '\\', ':']) || field(&row, "obj") == ".." {
                row["obj"] = json!("");
            }
            self.entries.insert(id, row);
        }
        if !self
            .entries
            .values()
            .any(|r| field(r, "categoria") == self.category)
        {
            self.category.clear();
        }
        self.ready = true;
        self.failed = None;
        Ok(())
    }

    fn select_category(&mut self, key: &str) {
        self.category = key.strip_prefix("online-cat:").unwrap_or("").into();
        self.focus.clear();
    }

    fn select(&mut self, key: &str) {
        self.prefer_online = key.starts_with("online:");
        if let Some(id) = key
            .strip_prefix("online:")
            .filter(|id| self.shown.contains(*id))
        {
            self.focus = id.into();
        }
    }

    fn insert(&mut self, key: Option<&str>) -> Option<String> {
        if self.inserting.is_some() {
            return None;
        }
        let id = match key {
            Some(key) => key.strip_prefix("online:")?,
            None => &self.focus,
        };
        if !self.shown.contains(id) {
            return None;
        }
        let entry = self.entries.get(id)?;
        self.inserting = Some(id.into());
        self.failed = None;
        Some(entry.to_string())
    }

    fn download_finished(&mut self, ok: bool) {
        let id = self.inserting.take();
        self.failed = if ok { None } else { id };
    }

    fn visible(&mut self, icons: Vec<String>) -> Vec<String> {
        self.visible = icons
            .iter()
            .filter_map(|icon| icon.strip_prefix("online:"))
            .filter(|id| self.shown.contains(*id))
            .map(String::from)
            .collect();
        self.visible
            .iter()
            .filter(|id| !self.decoded.contains(*id))
            .cloned()
            .collect()
    }

    fn previews(&mut self, py: Python<'_>, dir: &str) -> Vec<(String, u32, u32, Py<PyBytes>)> {
        let mut photos = vec![];
        for id in &self.visible {
            if self.decoded.contains(id) {
                continue;
            }
            let path = Path::new(dir).join("miniaturas").join(format!("{id}.png"));
            let Some(data) = read(&path) else { continue };
            self.decoded.insert(id.clone());
            if let Some(image) = decode(&data) {
                let pixels = image.thumbnail(192, 192).to_rgba8();
                photos.push((
                    format!("online:{id}"),
                    pixels.width(),
                    pixels.height(),
                    PyBytes::new(py, &pixels).unbind(),
                ));
            }
        }
        photos
    }

    fn sheet(
        &mut self,
        base: &str,
        lang: &str,
        query: &str,
        tr: &Bound<'_, PyAny>,
    ) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let mut s: Value =
            serde_json::from_str(base).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let side = s["side"]
            .as_array_mut()
            .ok_or_else(|| PyValueError::new_err("missing Library scopes"))?;
        let mut labels = HashMap::new();
        for row in self.entries.values() {
            let raw = field(row, "categoria");
            if !raw.is_empty() && !labels.contains_key(raw) {
                labels.insert(raw, t(category(raw))?);
            }
        }
        let query = query.trim().to_lowercase();
        let searching = !query.is_empty();
        let mut facets: Vec<_> = labels.iter().collect();
        facets.sort_by(|a, b| a.1.cmp(b.1).then(a.0.cmp(b.0)));
        for (raw, label) in facets {
            side.push(json!({"id":format!("online-cat:{raw}"),"label":label,"depth":1,"on":!searching && **raw==self.category}));
        }
        if let Some(root) = side.iter_mut().find(|row| row["id"] == "s:online") {
            root["on"] = json!(!searching && self.category.is_empty());
        }
        let mut rows: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, r)| {
                (searching || self.category.is_empty() || field(r, "categoria") == self.category)
                    && (query.is_empty()
                        || ["nombre", "nombre_en", "id"]
                            .iter()
                            .any(|k| field(r, k).to_lowercase().contains(&query)))
            })
            .collect();
        rows.sort_by_cached_key(|(_, r)| name(r, lang).to_lowercase());
        self.shown = rows.iter().map(|(id, _)| (*id).clone()).collect();
        if !self.shown.contains(&self.focus) {
            self.focus = rows
                .first()
                .map(|(id, _)| (*id).clone())
                .unwrap_or_default();
        }
        let tiles:Vec<_> = rows.iter().map(|(id,r)| {
            let key = format!("online:{id}");
            json!({"id":key,"label":name(r,lang),"icon":key,"sub":labels.get(field(r,"categoria")).cloned().unwrap_or_default()})
        }).collect();
        let title = labels
            .get(self.category.as_str())
            .cloned()
            .unwrap_or(t("Component library")?);
        let count = t("{n} of {total} models")?
            .replace("{n}", &rows.len().to_string())
            .replace("{total}", &self.entries.len().to_string());
        let empty = if !self.ready {
            "Loading…"
        } else if self.entries.is_empty() {
            "The library could not be reached, and nothing is cached yet."
        } else {
            "Nothing matches."
        };
        s["search"]["placeholder"] = json!(t("Search…")?);
        s["menus"] = json!([]);
        s["verbs"] = json!([{"id":"refresh_catalogue","label":t("Refresh")?,"enabled": self.ready && self.inserting.is_none()}]);
        let mut blocks = if searching {
            s["content"]["blocks"]
                .take()
                .as_array()
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let local_count: usize = blocks
            .iter()
            .filter_map(|b| b["items"].as_array())
            .map(Vec::len)
            .sum();
        if !tiles.is_empty() {
            if searching {
                blocks.push(json!({"kind":"caption","text":t("Component library")?,"right":rows.len().to_string()}));
            }
            blocks.push(json!({"kind":"tiles","tone":"components","size":96,"items":tiles}));
        }
        if searching {
            s["content"]["blocks"] = json!(blocks);
            s["content"]["sub"] =
                json!(t("{n} found")?.replace("{n}", &(local_count + rows.len()).to_string()));
            if local_count > 0 && (!self.prefer_online || self.shown.is_empty()) {
                return Ok(s.to_string());
            }
        } else {
            s["content"] = json!({"title":title,"sub":count,"empty":t(empty)?,"blocks":blocks});
        }
        if let Some(row) = self.entries.get(&self.focus) {
            let key = format!("online:{}", self.focus);
            let credit = credit(row, lang, tr)?;
            let mut details = vec![
                json!({"kind":"heading","text":name(row,lang)}),
                json!({"kind":"picture","icon":key}),
                json!({"kind":"text","text":credit}),
            ];
            if let Some(id) = &self.failed {
                if let Some(failed) = self.entries.get(id) {
                    details.push(json!({"kind":"text","tone":"error","text":t("Could not download “{name}”. Check your connection.")?.replace("{name}",name(failed,lang))}));
                }
            }
            details.push(json!({"kind":"buttons","align":"fill","buttons":[{"id":"insert_online","label":t(if self.inserting.is_some() {"Downloading…"} else {"Insert"})?,"role":1,"enabled":self.inserting.is_none(),"key":"↩"}]}));
            s["detail"] = json!({"width":300,"blocks":details});
            s["focus"] = json!(key);
        }
        Ok(s.to_string())
    }
}
