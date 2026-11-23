//! Detached Library draft. Only Apply can write the project's registry.
use crate::ui::dialog::{Field, FULL, MULTILINE};
use pyo3::{exceptions::PyValueError, prelude::*};
use serde_json::{json, Value};

#[pyclass]
pub struct RegistryEditor {
    scene: Py<PyAny>,
    original: Value,
    draft: String,
    key: String,
    name: String,
    namespace: String,
    #[pyo3(get)]
    error: String,
}

fn read(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    let text: String = value
        .py()
        .import("json")?
        .call_method1("dumps", (value,))?
        .extract()?;
    parse(&text)
}

fn parse(text: &str) -> PyResult<Value> {
    serde_json::from_str(text).map_err(|e| PyValueError::new_err(e.to_string()))
}

fn registry(scene: &Bound<'_, PyAny>) -> PyResult<Value> {
    read(
        &scene
            .py()
            .import("core.bim_registry")?
            .call_method1("get", (scene,))?,
    )
}

fn edit(id: &str, label: &str, value: &str) -> Field {
    (
        id.into(),
        "edit".into(),
        label.into(),
        value.into(),
        0.,
        0.,
        0.,
        vec![],
        0,
        String::new(),
    )
}

impl RegistryEditor {
    fn namespaces(&self) -> Vec<String> {
        parse(&self.draft)
            .ok()
            .and_then(|v| {
                v["classification_systems"]
                    .as_object()
                    .map(|systems| systems.keys().cloned().collect())
            })
            .unwrap_or_default()
    }

    fn rows(&self) -> Vec<Field> {
        let mut document = edit("registry_json", "Registry JSON", &self.draft);
        document.8 = MULTILINE | FULL;
        let mut system = edit("registry_system", "Import into", "");
        system.1 = "choice".into();
        system.7 = self.namespaces();
        system.4 = system
            .7
            .iter()
            .position(|key| key == &self.namespace)
            .unwrap_or(0) as f64;
        vec![
            document,
            edit("registry_key", "Namespace key", &self.key),
            edit("registry_name", "Namespace name", &self.name),
            system,
        ]
    }

    fn perform(&mut self, win: &Bound<'_, PyAny>, verb: &str, path: &str) -> PyResult<()> {
        let py = win.py();
        let vp = win.getattr("viewport")?;
        let scene = vp.getattr("scene")?;
        if !scene.is(self.scene.bind(py)) {
            return Err(PyValueError::new_err(
                "The active model changed. Reopen the registry.",
            ));
        }
        let mut value = parse(&self.draft)?;
        if !value.is_object() {
            return Err(PyValueError::new_err("The registry must be a JSON object."));
        }
        let backend = py.import("core.bim_registry")?;
        let data = || py.import("json")?.call_method1("loads", (&self.draft,));
        match verb {
            "registry_add" => {
                let key = self.key.trim();
                let name = self.name.trim();
                if key.is_empty() || name.is_empty() {
                    return Err(PyValueError::new_err("Enter a namespace key and name."));
                }
                let systems = value
                    .as_object_mut()
                    .unwrap()
                    .entry("classification_systems")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| {
                        PyValueError::new_err("Classification systems must be a JSON object.")
                    })?;
                if systems.contains_key(key) {
                    return Err(PyValueError::new_err("Key already exists"));
                }
                systems.insert(
                    key.into(),
                    json!({"name":name,"edition":"","source":"","description":""}),
                );
                self.namespace = key.into();
                self.key.clear();
                self.name.clear();
            }
            "registry_csv" => {
                if path.is_empty() {
                    return Ok(());
                }
                let systems = self.namespaces();
                let key = systems
                    .iter()
                    .find(|s| **s == self.namespace)
                    .or(systems.first())
                    .ok_or_else(|| {
                        PyValueError::new_err("Add a classification namespace first.")
                    })?;
                let text = std::fs::read_to_string(path)?;
                value = read(&backend.call_method1("import_codes_csv", (data()?, key, text))?)?;
            }
            "registry_apply" => {
                let parsed = data()?;
                let errors: Vec<String> =
                    backend.call_method1("validate", (&parsed,))?.extract()?;
                if !errors.is_empty() {
                    return Err(PyValueError::new_err(errors.join("\n")));
                }
                if registry(&scene)? != self.original {
                    return Err(PyValueError::new_err(
                        "The registry changed. Reopen it before applying.",
                    ));
                }
                if value != self.original {
                    let command = py
                        .import("core.history")?
                        .getattr("SetRegistryCommand")?
                        .call1((parsed,))?;
                    let history = vp.getattr("history")?;
                    history.call_method1("execute", (command,))?;
                    for attr in ["last_error", "last_rejected"] {
                        if let Some(error) = history.getattr(attr)?.extract::<Option<String>>()? {
                            return Err(PyValueError::new_err(error));
                        }
                    }
                    self.original = value.clone();
                }
            }
            _ => return Err(PyValueError::new_err("Unknown registry command")),
        }
        self.draft = serde_json::to_string_pretty(&value).unwrap();
        Ok(())
    }
}

#[pymethods]
impl RegistryEditor {
    #[new]
    fn new(scene: &Bound<'_, PyAny>) -> PyResult<Self> {
        let original = registry(scene)?;
        Ok(Self {
            scene: scene.clone().unbind(),
            draft: serde_json::to_string_pretty(&original).unwrap(),
            original,
            key: String::new(),
            name: String::new(),
            namespace: String::new(),
            error: String::new(),
        })
    }

    fn wire(&self) -> ((), (), (), (), Vec<Field>) {
        ((), (), (), (), self.rows())
    }

    fn field(&self, id: &str) -> Option<Field> {
        self.rows().into_iter().find(|row| row.0 == id)
    }

    fn receive(&mut self, id: &str, text: String) {
        self.error.clear();
        match id {
            "registry_json" => self.draft = text,
            "registry_key" => self.key = text,
            "registry_name" => self.name = text,
            "registry_system" => {
                if let Some(key) = text
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| self.namespaces().get(i).cloned())
                {
                    self.namespace = key;
                }
            }
            _ => (),
        }
    }

    #[pyo3(signature=(win, verb, path=""))]
    fn act(&mut self, win: &Bound<'_, PyAny>, verb: &str, path: &str) -> bool {
        match self.perform(win, verb, path) {
            Ok(()) => {
                self.error.clear();
                true
            }
            Err(error) => {
                self.error = error.value(win.py()).to_string();
                false
            }
        }
    }

    fn build(slf: PyRef<'_, Self>, sheet: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = sheet.py();
        let content = sheet.call_method1("content", ("Project registry",))?;
        let error = slf.error.clone();
        content.call_method1("fields", (slf, "registry"))?;
        if !error.is_empty() {
            content.call_method1("text", (error, "error"))?;
        }
        let verb = py.import("views.chrome_sheet")?.getattr("verb")?;
        let buttons = [
            ("registry_add", "Add namespace"),
            ("registry_csv", "Import codes CSV…"),
        ]
        .map(|args| verb.call1(args))
        .into_iter()
        .collect::<PyResult<Vec<_>>>()?;
        content.call_method1("buttons", (buttons, "left"))?;
        let kw = pyo3::types::PyDict::new(py);
        kw.set_item("primary", true)?;
        sheet.call_method("verb", ("registry_apply", "Apply"), Some(&kw))?;
        Ok(())
    }
}
