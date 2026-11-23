// SPDX-License-Identifier: GPL-3.0-or-later
//! Release diagnostics, localized credits and About actions.
use pyo3::{exceptions::PyValueError, prelude::*};
use serde_json::{json, Value};
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

type Credit = (String, String, String, bool);

fn credits() -> &'static [Credit] {
    static LIST: OnceLock<Vec<Credit>> = OnceLock::new();
    LIST.get_or_init(|| {
        serde_json::from_str(include_str!("contributors.json"))
            .expect("valid contributor catalogue")
    })
}

pub fn sheet(data: &Value, copied: bool, t: impl Fn(&str) -> PyResult<String>) -> PyResult<Value> {
    let mut people = Vec::with_capacity(credits().len());
    for (i, (name, role, link, starred)) in credits().iter().enumerate() {
        people.push(json!({"name":name,"role":t(role)?,"verb":if link.is_empty() {String::new()} else {format!("person-{i}")},"starred":starred}));
    }
    let thanks = t("Thanks as well to all the authors of the extensions catalogue")?;
    let mut blocks = json!([
        {"kind":"text","text":t("Free 3D modeler for architecture, engineering and 3D design.")?},
        {"kind":"text","text":format!("{} Marco Sumari Tellez · {}",t("Created by")?,t("Civil Engineer — Arequipa, Peru")?)},
        {"kind":"text","text":"Built is based on IngeTrazo and distributed under GPL-3.0-or-later."},
        {"kind":"buttons","align":"left","buttons":[{"id":"license","label":t("Licence")?},{"id":"upstream","label":"IngeTrazo ↗"},{"id":"copy","label":t(if copied {"Copied"} else {"Copy to clipboard"})?}]},
        {"kind":"caption","text":t("System information")?},
        {"kind":"kv","rows":data["facts"]},
        {"kind":"caption","text":t("With contributions from")?},
        {"kind":"credits","credits":people},
        {"kind":"prose","spans":[{"text":thanks,"link":"catalogue"}]},
        {"kind":"text","text":t("And thanks to everyone who tries Built and reports what they find.")?},
        {"kind":"buttons","align":"left","buttons":[{"id":"authors","label":"AUTHORS"}]}
    ]);
    if data["authors"] == true {
        let rows = blocks.as_array_mut().unwrap();
        for paragraph in include_str!("../../../AUTHORS").split("\n\n") {
            let mut lines = paragraph
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.chars().all(|c| c == '-'));
            if let Some(name) = lines.next() {
                rows.push(json!({"kind":"caption","text":name}));
                let text = lines.collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    rows.push(json!({"kind":"text","text":text}));
                }
            }
        }
    }
    Ok(
        json!({"id":"about","title":t("About Built")?,"width":900,"content":{"title":"Built","sub":data["build"],"blocks":blocks}}),
    )
}

#[pyclass]
pub struct AboutReview {
    #[pyo3(get)]
    info: String,
    data: Value,
    last_copy: Option<Instant>,
}
#[pymethods]
impl AboutReview {
    #[new]
    fn new(source: &str) -> PyResult<Self> {
        let source: Value =
            serde_json::from_str(source).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let s = |key: &str| source[key].as_str().unwrap_or("");
        let distribution = if source["flatpak"] == true {
            "Flatpak"
        } else if source["appimage"] == true {
            "AppImage"
        } else if source["packaged"] == true {
            "packaged"
        } else {
            "from source"
        };
        let title = format!("Built {} ({distribution})", s("version"));
        let os = match s("system") {
            "Darwin" => format!("macOS {}", s("os_version")),
            "Windows" => format!("Windows {} ({})", s("release"), s("os_version")),
            sys => {
                let mut os = format!("{sys} {}", s("release"));
                if !s("pretty").is_empty() {
                    os.push_str(", ");
                    os.push_str(s("pretty"));
                }
                os
            }
        };
        let mut facts = vec![
            ("OS", format!("{os} ({})", s("machine"))),
            ("Python", s("python").to_owned()),
            ("Qt", format!("{}, PySide6: {}", s("qt"), s("pyside"))),
        ];
        if !s("gl").is_empty() {
            facts.push(("OpenGL", s("gl").into()));
        }
        if let Some(size) = source["size"].as_array().filter(|a| a.len() == 3) {
            facts.push((
                if source["native"] == true {
                    "Window"
                } else {
                    "Screen"
                },
                format!(
                    "{}x{} @ {}x",
                    size[0],
                    size[1],
                    size[2].as_f64().unwrap_or(1.)
                ),
            ));
        }
        facts.extend([
            ("Language", s("language").into()),
            ("Engine", s("engine").into()),
            ("Renderer", source["renderer"].as_str().unwrap_or("wgpu").into()),
            ("Commit", s("commit").into()),
        ]);
        let mut info = title.clone();
        for (name, value) in &facts {
            info.push_str(&format!("\n{name}: {value}"));
        }
        let data = json!({"build":format!("{title} · {}",s("commit")), "facts":facts.into_iter().map(|(k,v)|json!([k,v,""])).collect::<Vec<_>>()});
        Ok(Self {
            info,
            data,
            last_copy: None,
        })
    }

    fn sheet(&self, tr: &Bound<'_, PyAny>) -> PyResult<String> {
        Ok(sheet(&self.data, self.copied(), |s| {
            tr.call1((s,))?.extract::<String>()
        })?
        .to_string())
    }
    fn copied(&self) -> bool {
        self.last_copy
            .is_some_and(|t| t.elapsed() < Duration::from_millis(1500))
    }
    fn acknowledge(&mut self) {
        self.last_copy = Some(Instant::now());
    }
    fn action(&mut self, verb: &str) -> Option<(&'static str, String)> {
        match verb {
            "copy" => Some(("copy", self.info.clone())),
            "authors" => {
                self.data["authors"] = json!(self.data["authors"] != true);
                None
            }
            _ => Self::link(verb).map(|url| ("open", url.to_owned())),
        }
    }
    #[staticmethod]
    fn link(verb: &str) -> Option<&'static str> {
        match verb {
            "license" => Some("https://www.gnu.org/licenses/gpl-3.0.html"),
            "upstream" => Some("https://github.com/ingelibre/ingetrazo"),
            "catalogue" => Some("https://github.com/ingelibre/ingetrazo-extensions"),
            _ => credits()
                .get(verb.strip_prefix("person-")?.parse::<usize>().ok()?)
                .map(|c| c.2.as_str())
                .filter(|s| !s.is_empty()),
        }
    }
    #[staticmethod]
    fn contributors() -> Vec<Credit> {
        credits().to_owned()
    }
}
