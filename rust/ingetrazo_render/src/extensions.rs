//! Catalogue and installed-plugin state for the Extensions sheet.
use crate::extension_previews::{PreviewFile, Queue};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict},
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::{path::Path, time::Duration};

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

/// The command can fail before a sheet exists (for example, an unreadable install).
#[pyfunction]
pub fn open_extensions<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let sheets = py.import("views.report_sheets")?;
    let chrome = py.import("views.chrome_sheet")?;
    let opened = sheets.call_method0("Extensions")
        .and_then(|provider| chrome.call_method1("open", (win, provider)));
    match opened {
        Ok(result) => Ok(result),
        Err(error) if error.is_instance_of::<pyo3::exceptions::PyException>(py) => {
            crate::extension_setup::log_failure(py, "the extension manager could not open", &error);
            let tr = py.import("core.i18n")?.getattr("tr")?;
            let detail = translated(&tr, "Could not load extensions: {error}",
                                    &[("error", &error.to_string())])?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("tone", "warning")?;
            let title = tr.call1(("Extension manager",))?;
            py.import("views.chrome_dialogs")?
                .call_method("notify", (win, title, detail), Some(&kwargs))?;
            Ok(py.None().into_bound(py))
        }
        Err(error) => Err(error),
    }
}

const TAGS: &[(&str, &str)] = &[
    ("architecture", "Architecture"),
    ("bim", "BIM"),
    ("structures", "Structures"),
    ("terrain", "Terrain"),
    ("drawing", "Drawing"),
    ("analysis", "Analysis"),
    ("import-export", "Import/export"),
    ("rendering", "Rendering"),
    ("fabrication", "Fabrication"),
    ("productivity", "Productivity"),
    ("education", "Education"),
    ("other", "Other"),
];

#[derive(Default)]
struct Package {
    id: String,
    name: String,
    summary: String,
    catalog: Value,
    managed: Value,
    enabled: bool,
    shipped: bool,
    manual: bool,
    running: bool,
    failure: String,
    path: String,
    old: bool,
    icon: String,
    reviewed: bool,
    compatible: bool,
    restart: bool,
    build: String,
    tags: Vec<String>,
}
impl Package {
    fn installed(&self) -> bool {
        self.shipped || self.manual || self.managed.is_object()
    }
    fn update(&self) -> bool {
        !self.shipped
            && self.managed.is_object()
            && self.catalog.is_object()
            && self.managed["sha256"] != self.catalog["sha256"]
    }
    fn bundled(&self) -> bool {
        self.catalog["bundled"].is_string()
    }
    fn origin(&self) -> &str {
        if self.shipped || (self.bundled() && !self.manual) {
            "Included with Built"
        } else if self.manual {
            "Installed by hand"
        } else if self.catalog.is_object() {
            "Catalog"
        } else {
            "No longer in the catalog"
        }
    }
    fn version(&self) -> &str {
        if self.shipped {
            &self.build
        } else {
            self.managed["version"]
                .as_str()
                .or(self.catalog["version"].as_str())
                .unwrap_or("Unspecified")
        }
    }
    fn verbs(&self) -> Vec<(&str, &str)> {
        let mut verbs = Vec::new();
        if self.installed() {
            if self.enabled || self.compatible {
                verbs.push(("toggle", if self.enabled { "Disable" } else { "Enable" }));
            }
            if self.manual && !self.shipped && self.catalog.is_object() && self.compatible {
                verbs.push((
                    "replace",
                    if self.bundled() {
                        "Replace with the included version"
                    } else {
                        "Replace with the catalog version"
                    },
                ));
            } else if self.update() && self.compatible {
                verbs.push(("install", "Update"));
            }
            if !self.shipped {
                verbs.push(("remove", "Uninstall"));
            }
        } else if self.compatible && self.catalog.is_object() {
            verbs.push(("install", "Install"));
        }
        if text(&self.catalog["repository"]).starts_with("https://") {
            verbs.push(("repository", "Repository ↗"));
        }
        if self.manual
            || (!self.catalog.is_object() && self.installed())
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
    previews: Queue,
    tab: String,
    browsed: bool,
    query: String,
    sort: String,
    tag: String,
    reviewed: bool,
    safe: bool,
    fetched: String,
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
            previews: Default::default(),
            tab: "Installed".into(),
            browsed: false,
            query: String::new(),
            sort: "name".into(),
            tag: String::new(),
            reviewed: false,
            safe: false,
            fetched: String::new(),
            focus: String::new(),
            notice: String::new(),
        }
    }
    #[staticmethod]
    fn tags() -> Vec<(&'static str, &'static str)> {
        TAGS.to_vec()
    }

    fn receive(&mut self, kind: &str, args: Vec<String>) -> bool {
        if kind == "visible" {
            self.previews.visible(&args);
            return false;
        }
        let Some(value) = args.first() else {
            return false;
        };
        match kind {
            "tab" if ["Installed", "Browse", "Updates"].contains(&value.as_str()) => {
                self.tab = value.clone();
                if value == "Browse" && !self.browsed {
                    self.browsed = true;
                    return true;
                }
            }
            "focus" | "open" => self.focus = value.clone(),
            "search" => self.query = value.clone(),
            "headmenu" if args.len() == 2 => match value.as_str() {
                "sort" => self.sort = args[1].clone(),
                "tag" => self.tag = args[1].clone(),
                "trust" => self.reviewed = args[1] == "reviewed",
                _ => (),
            },
            _ => (),
        }
        false
    }
    fn load(&mut self, snapshot: &str, py: Python<'_>) -> PyResult<()> {
        let d: Value =
            serde_json::from_str(snapshot).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let mut ids = BTreeSet::new();
        for name in ["catalog", "managed", "hands", "shipped", "errors"] {
            if let Some(map) = d[name].as_object() {
                ids.extend(map.keys());
            }
        }
        let contains =
            |key: &str, id: &str| d[key].as_array().is_some_and(|v| v.iter().any(|x| x == id));
        self.fetched = text(&d["fetched"]).to_owned();
        self.safe = d["safe"] == true;
        self.packages.clear();
        let mut previews = BTreeMap::new();
        for id in ids {
            let catalog = &d["catalog"][id];
            let icon = PreviewFile::from_entry(id, catalog, Path::new(text(&d["cache"])))
                .map(|file| {
                    let icon = file.icon.clone();
                    previews.insert(icon.clone(), file);
                    icon
                })
                .unwrap_or_default();
            let managed = &d["managed"][id];
            let manual = d["hands"][id].is_string();
            let shipped = d["shipped"][id].is_string();
            let state = &d["states"][id];
            let enabled = state["enabled"] != false && !contains("disabled", id);
            let running = contains("running", id);
            let failure = text(&d["errors"][id]["error"]).to_owned();
            let name = localized(&catalog["name"], text(&d["lang"]));
            self.packages.insert(
                id.clone(),
                Package {
                    id: id.clone(),
                    name: if name.is_empty() { id.clone() } else { name },
                    summary: localized(&catalog["summary"], text(&d["lang"])),
                    reviewed: catalog["bundled"].is_string()
                        || (catalog["sha256"].is_string()
                            && text(&d["reviewed"][id]["sha256"])
                                .eq_ignore_ascii_case(text(&catalog["sha256"]))),
                    compatible: !contains("incompatible", id),
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
                    old: contains("old", id),
                    icon,
                    restart: {
                        let started = &d["startup"][id];
                        state != started
                            && (state["enabled"] != false || started["enabled"] == true)
                    },
                    build: text(&d["build"]).into(),
                    tags: catalog["tags"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect(),
                    catalog: catalog.clone(),
                    managed: managed.clone(),
                },
            );
        }
        self.previews.sync(previews, py);
        Ok(())
    }
    fn jobs(&mut self, now_ms: u64) -> Vec<PreviewFile> {
        self.previews.jobs(Duration::from_millis(now_ms))
    }
    fn preview_done(&mut self, icon: &str, now_ms: u64, pixels: Option<Py<PyBytes>>) {
        self.previews
            .done(icon, pixels, Duration::from_millis(now_ms))
    }
    fn icons(&self, py: Python<'_>) -> Vec<(String, u32, u32, Py<PyBytes>)> {
        self.previews.icons(py)
    }
    #[getter]
    fn revision(&self) -> u64 {
        self.previews.revision
    }
    fn pending(&self) -> bool {
        self.previews.pending()
    }

    fn sheet(&mut self, tr: &Bound<'_, PyAny>) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let mut rows = Vec::new();
        let mut tags = BTreeSet::new();
        let mut recent = false;
        let mut total = 0;
        for p in self.packages.values() {
            if !match self.tab.as_str() {
                "Updates" => p.update(),
                "Browse" => p.catalog.is_object(),
                _ => p.installed(),
            } {
                continue;
            }
            total += 1;
            tags.extend(p.tags.iter().cloned());
            recent |= p.catalog["updated_at"].is_string();
            if !self.tag.is_empty() && !p.tags.contains(&self.tag) {
                continue;
            }
            if self.tab == "Browse" && self.reviewed && !p.reviewed {
                continue;
            }
            let hay = format!(
                "{} {} {} {} {} {}",
                p.id,
                p.name,
                p.summary,
                p.origin(),
                p.tags.join(" "),
                text(&p.catalog["author"])
            )
            .to_lowercase();
            if self
                .query
                .to_lowercase()
                .split_whitespace()
                .all(|word| hay.contains(word))
            {
                rows.push(p);
            }
        }
        rows.sort_by(|a, b| {
            let cmp = match self.sort.as_str() {
                "recent" => text(&b.catalog["updated_at"]).cmp(text(&a.catalog["updated_at"])),
                "reviewed" => b.reviewed.cmp(&a.reviewed),
                "updates" => b.update().cmp(&a.update()),
                _ => std::cmp::Ordering::Equal,
            };
            cmp.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.id.cmp(&b.id))
        });
        let selected = rows
            .iter()
            .copied()
            .find(|p| p.id == self.focus)
            .or(rows.first().copied());
        self.focus = selected.map_or_else(String::new, |p| p.id.clone());
        let mut items = Vec::new();
        for p in &rows {
            items.push(json!({"id":p.id,"label":p.name,"sub":t(p.origin())?,"icon":p.icon,"tip":p.summary}));
        }
        let mut menus = vec![
            json!({"id":"sort","label":t("Sort")?,"on":self.sort,"options":[
            {"id":"name","label":t("Name")?},{"id":"reviewed","label":t("Reviewed first")?},
            {"id":"updates","label":t("Updates first")?},{"id":"recent","label":t("Most recent")?,"enabled":recent}]}),
        ];
        if !tags.is_empty() || !self.tag.is_empty() {
            let mut opts = vec![json!({"id":"","label":t("All")?})];
            for tag in tags {
                opts.push(json!({"id":tag,"label":t(TAGS.iter().find(|(k,_)|*k==tag).map_or(tag.as_str(),|x|x.1))?}));
            }
            menus.push(json!({"id":"tag","label":t("Tags")?,"on":self.tag,"options":opts}));
        }
        if self.tab == "Browse" {
            menus.push(json!({"id":"trust","label":t("Review")?,"on":if self.reviewed{"reviewed"}else{"all"},"options":[
                {"id":"all","label":t("All")?},{"id":"reviewed","label":t("Reviewed")?}]}));
        }
        let mut tabs = Vec::new();
        for id in ["Installed", "Browse", "Updates"] {
            tabs.push(json!({"id":id,"label":t(id)?,"on":self.tab==id}));
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
                if let Some(v) = p.catalog[key].as_str().filter(|v| !v.is_empty()) {
                    facts.push(json!([t(label)?, v, ""]));
                }
            }
            if !p.path.is_empty() {
                facts.push(json!([t("Path")?, p.path, ""]));
            }

            if p.catalog.is_object() {
                facts.push(json!([
                    t("Review")?,
                    t(if p.reviewed {
                        "Reviewed"
                    } else {
                        "Not reviewed by a Built maintainer"
                    })?,
                    ""
                ]));
            }
            facts.extend([
                json!([t("This session")?, t(status)?, ""]),
                json!([t("Next launch")?, t(next)?, ""]),
            ]);
            let mut blocks = vec![
                json!({"kind":"text","text":p.summary}),
                json!({"kind":"facts","rows":facts}),
            ];
            if !p.tags.is_empty() {
                let mut items = Vec::new();
                for tag in &p.tags {
                    let fallback = tag.replace('-', " ");
                    let label = TAGS
                        .iter()
                        .find(|(id, _)| id == tag)
                        .map_or(fallback.as_str(), |pair| pair.1);
                    items.push(json!({"id":tag,"label":t(label)?}));
                }
                blocks.push(json!({"kind":"tags","items":items}));
            }
            if p.restart {
                blocks.push(json!({"kind":"text","text":t("Restart required to apply changes")?,"tone":"warn"}));
            }
            if p.update() {
                blocks.push(json!({"kind":"text","text":translated(tr,"Update available: v{installed} → v{available}", &[("installed",p.version()),("available",text(&p.catalog["version"]))])?}));
            }
            if !p.compatible {
                blocks.push(json!({"kind":"text","text":translated(tr,"Requires Built {version} or later.", &[("version",text(&p.catalog["minimum_version"]))])?,"tone":"warn"}));
            }
            if p.old && p.compatible {
                blocks.push(json!({"kind":"text","text":translated(tr, "Compatibility is uncertain: this extension was tested with Built {version}.",&[("version",text(&p.catalog["minimum_version"]))])?, "tone":"warn"}));
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
        Ok(json!({"id":"extensions","title":t("Extension Manager")?,"width":1100,"menus":menus,"tabs":tabs,"search":{"placeholder":t("Search extensions")?},
            "focus":self.focus,"force_focus":true,"detail":detail,"verbs":[{"id":"refresh","label":t("Refresh catalog")?}],
            "content":{"title":t(&self.tab)?,"sub":translated(tr, "{count} of {total} extensions", &[("count",&rows.len().to_string()),("total",&total.to_string())])?,"empty":t("No matching extensions")?,"blocks":if rows.is_empty(){vec![]}else{vec![json!({"kind":"tiles","tone":"catalog","size":176,"items":items})]}},
            "footer":{"links":[{"id":"safe","label":t(if self.safe{"Leave safe mode"}else{"Safe mode"})?},{"id":"reset","label":t("Reset overrides…")?}],"hint":if self.notice.is_empty(){self.fetched.as_str()}else{self.notice.as_str()}}}).to_string())
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
                    &[("name", &p.name), ("author", text(&p.catalog["author"]))],
                )?,
            ));
            Ok(())
        };
        if matches!(verb, "install" | "replace") && !p.reviewed {
            ask("Install an unreviewed extension?", "«{name}» has not been reviewed by a Built maintainer.\n\nExtensions are Python code: once installed it runs with your permissions and can read or change your files. Install it only if you trust its author ({author}).")?;
        }
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
