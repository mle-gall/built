//! Local extension state for the Extensions sheet.

use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}
fn localized(map: &Value, lang: &str) -> String {
    let lang = lang.to_ascii_lowercase();
    let lang = lang.split(['-', '_']).next().unwrap_or("en");
    map[lang]
        .as_str()
        .or(map["en"].as_str())
        .or_else(|| map.as_object()?.values().find_map(Value::as_str))
        .unwrap_or_default()
        .into()
}

fn translated(tr: &Bound<'_, PyAny>, source: &str, fields: &[(&str, &str)]) -> PyResult<String> {
    let args = PyDict::new(tr.py());
    for (key, value) in fields {
        args.set_item(key, value)?;
    }
    tr.call((source,), Some(&args))?.extract()
}

#[derive(Default)]
struct Package {
    id: String,
    name: String,
    summary: String,
    bundled_info: Value,
    managed: Value,
    enabled: bool,
    shipped: bool,
    manual: bool,
    running: bool,
    failure: String,
    path: String,

    restart: bool,
    build: String,
}
impl Package {
    fn installed(&self) -> bool {
        self.shipped || self.manual || self.managed.is_object()
    }
    fn bundled(&self) -> bool {
        self.bundled_info["bundled"].is_string()
    }
    fn origin(&self) -> &str {
        if self.shipped || (self.bundled() && !self.manual) {
            "Included with Built"
        } else if self.manual {
            "Installed by hand"
        } else if self.bundled_info.is_object() {
            "Included with Built"
        } else {
            "Installed locally"
        }
    }
    fn version(&self) -> &str {
        if self.shipped {
            &self.build
        } else {
            self.managed["version"]
                .as_str()
                .or(self.bundled_info["version"].as_str())
                .unwrap_or("Unspecified")
        }
    }
    fn verbs(&self) -> Vec<(&str, &str)> {
        let mut verbs = Vec::new();
        if self.installed() {
            verbs.push(("toggle", if self.enabled { "Disable" } else { "Enable" }));
            if self.manual && !self.shipped && self.bundled() {
                verbs.push(("replace", "Replace with the included version"));
            }
            if !self.shipped {
                verbs.push(("remove", "Uninstall"));
            }
        } else if self.bundled() {
            verbs.push(("install", "Install"));
        }
        if self.manual
            || (!self.bundled_info.is_object() && self.installed())
            || !self.failure.is_empty()
        {
            verbs.push(("folder", "Open extension folder"));
        }
        if !self.failure.is_empty() {
            verbs.push(("logs", "Open logs folder"));
        }
        verbs
    }
}

#[pyclass]
pub struct ExtensionBrowser {
    packages: BTreeMap<String, Package>,
    revision: u64,

    query: String,

    safe: bool,

    #[pyo3(get, set)]
    focus: String,
    #[pyo3(get, set)]
    notice: String,
}
#[pymethods]
impl ExtensionBrowser {
    #[new]
    fn new() -> Self {
        Self {
            packages: Default::default(),
            revision: 0,

            query: String::new(),

            safe: false,

            focus: String::new(),
            notice: String::new(),
        }
    }

    fn receive(&mut self, kind: &str, args: Vec<String>) {
        let Some(value) = args.first() else {
            return;
        };
        match kind {
            "focus" | "open" => self.focus = value.clone(),
            "search" => self.query = value.clone(),
            _ => (),
        }
    }
    fn load(&mut self, snapshot: &str) -> PyResult<()> {
        let d: Value =
            serde_json::from_str(snapshot).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let mut ids = BTreeSet::new();
        for name in ["bundled_info", "managed", "hands", "shipped", "errors"] {
            if let Some(map) = d[name].as_object() {
                ids.extend(map.keys());
            }
        }
        let contains =
            |key: &str, id: &str| d[key].as_array().is_some_and(|v| v.iter().any(|x| x == id));

        self.safe = d["safe"] == true;
        self.packages.clear();

        for id in ids {
            let bundled_info = &d["bundled_info"][id];
            let managed = &d["managed"][id];
            let manual = d["hands"][id].is_string();
            let shipped = d["shipped"][id].is_string();
            let state = &d["states"][id];
            let enabled = state["enabled"] != false && !contains("disabled", id);
            let running = contains("running", id);
            let failure = text(&d["errors"][id]["error"]).to_owned();
            let name = localized(&bundled_info["name"], text(&d["lang"]));
            self.packages.insert(
                id.clone(),
                Package {
                    id: id.clone(),
                    name: if name.is_empty() { id.clone() } else { name },
                    summary: localized(&bundled_info["summary"], text(&d["lang"])),
                    enabled,
                    manual,
                    shipped,
                    running,
                    failure,
                    path: d["errors"][id]["path"]
                        .as_str()
                        .or(d["hands"][id].as_str())
                        .unwrap_or_default()
                        .into(),

                    restart: {
                        let started = &d["startup"][id];
                        state != started
                            && (state["enabled"] != false || started["enabled"] == true)
                    },
                    build: text(&d["build"]).into(),
                    bundled_info: bundled_info.clone(),
                    managed: managed.clone(),
                },
            );
        }
        self.revision += 1;
        Ok(())
    }
    #[getter]
    fn revision(&self) -> u64 {
        self.revision
    }
    fn sheet(&mut self, tr: &Bound<'_, PyAny>) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let mut rows = Vec::new();

        let mut total = 0;
        for p in self.packages.values() {
            total += 1;
            let hay = format!("{} {} {} {}", p.id, p.name, p.summary, p.origin()).to_lowercase();
            if self
                .query
                .to_lowercase()
                .split_whitespace()
                .all(|word| hay.contains(word))
            {
                rows.push(p);
            }
        }
        rows.sort_by_key(|p| p.name.to_lowercase());
        let selected = rows
            .iter()
            .copied()
            .find(|p| p.id == self.focus)
            .or(rows.first().copied());
        self.focus = selected.map_or_else(String::new, |p| p.id.clone());
        let mut items = Vec::new();
        for p in &rows {
            let state = if !p.installed() { "Not installed" } else if self.safe { "Safe mode" }
                else if p.enabled { "Enabled" } else { "Disabled" };
            items.push(json!({"id":p.id, "label":p.name, "sub":p.summary, "right":t(state)?,
                "toggle":(p.installed() && !self.safe).then_some(p.enabled), "tip":p.summary}));

        }
        let detail = if let Some(p) = selected {
            let next = if !p.installed() {
                "Not installed"
            } else if self.safe {
                "Safe mode"
            } else if p.enabled {
                "Enabled"
            } else {
                "Disabled"
            };
            let status = if !p.failure.is_empty() {
                "Failed to load at startup"
            } else if p.running {
                "Running"
            } else {
                "Not running"
            };
            let mut facts = vec![
                json!([t("Source")?, t(p.origin())?, ""]),
                json!([t("Version")?, p.version(), ""]),
            ];
            for (key, label) in [
                ("author", "Author"),
                ("license", "License"),
                ("minimum_version", "Tested with Built"),
            ] {
                if let Some(v) = p.bundled_info[key].as_str().filter(|v| !v.is_empty()) {
                    facts.push(json!([t(label)?, v, ""]));
                }
            }
            if !p.path.is_empty() {
                facts.push(json!([t("Path")?, p.path, ""]));
            }

            facts.extend([
                json!([t("This session")?, t(status)?, ""]),
                json!([t("Next launch")?, t(next)?, ""]),
            ]);
            let mut blocks = vec![
                json!({"kind":"text","text":p.summary}),
                json!({"kind":"facts","rows":facts}),
            ];
            if p.restart {
                blocks.push(json!({"kind":"text","text":t("Restart required to apply changes")?,"tone":"warn"}));
            }
            if !p.failure.is_empty() {
                blocks.push(json!({"kind":"text","text":p.failure,"tone":"error"}));
            }
            let mut buttons = Vec::new();
            for (id, label) in p.verbs() {
                buttons.push(json!({"id":id,"label":t(label)?}));
            }
            blocks.push(json!({"kind":"buttons","align":"fill","buttons":buttons}));
            Some(json!({"title":p.name,"width":320,"blocks":blocks}))
        } else {
            None
        };
        Ok(json!({"id":"extensions","title":t("Extension Manager")?,"width":1100,"search":{"placeholder":t("Search extensions")?},
            "focus":self.focus,"force_focus":true,"detail":detail,"verbs":[{"id":"local_file","label":t("Install local extension")?}],
            "content":{"title":t("Local extensions")?,"sub":translated(tr, "{count} of {total} extensions", &[("count",&rows.len().to_string()),("total",&total.to_string())])?,"empty":t("No matching extensions")?,"blocks":if rows.is_empty(){vec![]}else{vec![json!({"kind":"rows","id":"extensions","items":items})]}},
            "footer":{"links":[{"id":"safe","label":t(if self.safe{"Leave safe mode"}else{"Safe mode"})?},{"id":"reset","label":t("Reset overrides…")?}],"hint":self.notice}}).to_string())
    }

    fn copies(
        &self,
        ident: &str,
        paths: Vec<String>,
        tr: &Bound<'_, PyAny>,
    ) -> PyResult<(String, String)> {
        let name = self.packages.get(ident).map_or(ident, |p| p.name.as_str());
        Ok((translated(tr,"Replace copies",&[])?,
            translated(tr,"«{name}» seems to be installed by hand already, in: {folders}.\n\nIf you continue, that copy is set aside (renamed with a dot in front, not deleted) so the extension does not load twice.",
            &[("name",name),("folders",&paths.join(", "))])?))
    }

    fn command(
        &self,
        verb: &str,
        tr: &Bound<'_, PyAny>,
    ) -> PyResult<Option<(String, String, Vec<(String, String)>)>> {
        let Some(p) = self.packages.get(&self.focus) else {
            return Ok(None);
        };
        if !p.verbs().iter().any(|(id, _)| *id == verb) {
            return Ok(None);
        }
        let mut questions = Vec::new();
        let mut ask = |title, prompt| -> PyResult<()> {
            questions.push((
                translated(tr, title, &[])?,
                translated(
                    tr,
                    prompt,
                    &[
                        ("name", &p.name),
                        ("author", text(&p.bundled_info["author"])),
                    ],
                )?,
            ));
            Ok(())
        };
        let op = match verb {
            "replace" => {
                ask(
                    "Replace extension",
                    "Set aside the hand-installed copy of {name} and install its replacement?",
                )?;
                "replace"
            }
            "remove" if p.manual => {
                ask(
                    "Uninstall extension",
                    "Set aside {name} in its current folder?",
                )?;
                "set_aside"
            }
            "remove" => {
                ask(
                    "Uninstall extension",
                    "Uninstall «{name}»? Its files will be removed.",
                )?;
                "uninstall"
            }
            "toggle" if p.managed.is_object() && !p.shipped => {
                if p.enabled {
                    "disable"
                } else {
                    "enable"
                }
            }
            "toggle" => {
                if p.enabled {
                    "override_off"
                } else {
                    "override_on"
                }
            }
            _ => verb,
        };
        Ok(Some((op.into(), p.id.clone(), questions)))
    }
}
