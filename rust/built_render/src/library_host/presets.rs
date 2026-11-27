// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::exceptions::PyValueError;
use serde_json::{json, Value};

fn rows(ctx: &Bound<'_, PyAny>) -> PyResult<Vec<(Value, String)>> {
    let py = ctx.py();
    let scene = ctx
        .getattr_opt("scene")?
        .filter(|s| s.hasattr("_root").unwrap_or(false));
    let catalog = py
        .import("built_core.command_protocol")?
        .call_method1("preset_catalog", (scene,))?;
    let mut items: Vec<(Value, String)> = serde_json::from_str(&dumps(py, &catalog)?)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    for (_, preset) in saved_presets()? {
        if !items.iter().any(|(row, _)| row["uid"] == preset["uid"]) {
            items.push((preset, "mine".into()));
        }
    }
    Ok(items)
}
fn saved_presets() -> PyResult<Vec<(PathBuf, Value)>> {
    let mut saved = Vec::new();
    if let Ok(files) = std::fs::read_dir(local_dir()?) {
        for file in files.flatten().map(|entry| entry.path()) {
            if file.extension().is_some_and(|ext| ext == "builtpreset") {
                if let Some(preset) = std::fs::read(&file)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .filter(|row| row["uid"].is_string() && row["name"].is_string())
                {
                    saved.push((file, preset));
                }
            }
        }
    }
    Ok(saved)
}
fn saved_path(key: &str) -> PyResult<PathBuf> {
    saved_presets()?
        .into_iter()
        .find(|(_, row)| row["uid"] == key)
        .map(|(path, _)| path)
        .ok_or_else(|| PyValueError::new_err("Saved preset is missing"))
}
fn local_dir() -> PyResult<PathBuf> {
    Ok(ProfilesProvider::saved_directory()?.with_file_name("presets"))
}
fn update(ctx: &Bound<'_, PyAny>, rows: Value) -> PyResult<()> {
    let py = ctx.py();
    let args = PyDict::new(py);
    args.set_item(
        "rows",
        json(py)?.call_method1("loads", (rows.to_string(),))?,
    )?;
    let reply = core(ctx, "preset_update", &args)?;
    if !reply.get_item("ok")?.is_truthy()? {
        return flash(
            ctx,
            &reply
                .call_method1("get", ("error", "Preset update failed"))?
                .str()?
                .to_string(),
        );
    }
    notify(&ctx.getattr("vp")?)
}
fn uid(py: Python<'_>) -> PyResult<String> {
    let hex: String = py
        .import("uuid")?
        .call_method0("uuid4")?
        .getattr("hex")?
        .extract()?;
    Ok(hex[..16].to_owned())
}
fn shown(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
fn edit(ctx: &Bound<'_, PyAny>, preset: Value, title: &str) -> PyResult<()> {
    let py = ctx.py();
    let forms = py.import("built_render.chrome_forms")?;
    let field = forms.getattr("Field")?;
    let fields = PyList::empty(py);
    let capture = title == "Save as preset";
    let personal = title == "Edit saved preset";
    for (key, label) in [
        ("name", "Name"),
        ("pack", "Pack"),
        ("naming", "Naming pattern"),
        ("node", "Filing node"),
    ] {
        if !capture || matches!(key, "name" | "pack") {
            fields.append(field.call1((key, "edit", label, shown(&preset[key])))?)?;
        }
    }
    if !capture {
        for section in ["values", "record"] {
            for (key, v) in preset[section].as_object().into_iter().flatten() {
                fields.append(field.call1((
                    format!("{section}.{key}"),
                    "edit",
                    key,
                    shown(v),
                ))?)?;
                fields.append(field.call1((
                    format!("lock.{key}"),
                    "check",
                    format!("Fix {key}"),
                    preset["locks"][key] == "fixed",
                ))?)?;
            }
        }
    }
    let summary = format!(
        "{} · {} · {}",
        preset["values"]["profile"]["library"]
            .as_str()
            .unwrap_or("Drawn profile"),
        preset["record"]["material"].as_str().unwrap_or(""),
        preset["record"]["class"].as_str().unwrap_or("")
    );
    if capture {
        fields.append(field.call(
            ("destination", "choice", "Save to", 0),
            Some(&[("options", ["This project", "My presets", "Shared folder"])].into_py_dict(py)?),
        )?)?;
        let shared = func(py, |py, args| {
            obj(
                py,
                args.get_item(0)?
                    .call_method1("value", ("destination",))?
                    .eq(2)?,
            )
        })?;
        fields.append(field.call(
            ("folder", "edit", "Shared folder", ""),
            Some(&[("visible", shared)].into_py_dict(py)?),
        )?)?;
        fields.append(field.call1((
            "record_defaults",
            "check",
            "Include record values",
            true,
        ))?)?;
    }
    let c = ctx.clone().unbind();
    let done = func(py, move |py, args| {
        if args.get_item(0)?.eq("ok")? {
            let vals = args.get_item(1)?;
            let mut p = preset.clone();
            for key in ["name", "pack", "naming", "node"] {
                if vals.contains(key)? {
                    p[key] = json!(vals.get_item(key)?.extract::<String>()?);
                }
            }
            if !capture {
                for section in ["values", "record"] {
                    for (key, old) in preset[section].as_object().into_iter().flatten() {
                        let raw: String = vals.get_item(format!("{section}.{key}"))?.extract()?;
                        p[section][key] = if old.is_string() {
                            json!(raw)
                        } else {
                            serde_json::from_str(&raw)
                                .map_err(|e| PyValueError::new_err(e.to_string()))?
                        };
                        p["locks"][key] =
                            json!(if vals.get_item(format!("lock.{key}"))?.is_truthy()? {
                                "fixed"
                            } else {
                                "free"
                            });
                    }
                }
            }
            if capture && !vals.get_item("record_defaults")?.is_truthy()? {
                if let Some(record) = p["record"].as_object_mut() {
                    record.retain(|k, _| {
                        matches!(
                            k.as_str(),
                            "class" | "material" | "predefined_type" | "object_type"
                        )
                    });
                }
            }
            let destination = if capture {
                vals.get_item("destination")?.extract::<usize>()?
            } else {
                0
            };
            if personal {
                let file = saved_path(&shown(&p["uid"]))?;
                let temporary = file.with_extension(format!("{}.tmp", uid(py)?));
                p["rev"] = json!(p["rev"].as_u64().unwrap_or(0).saturating_add(1));
                let result = (|| -> std::io::Result<()> {
                    use std::io::Write;
                    std::fs::File::create_new(&temporary)?.write_all(p.to_string().as_bytes())?;
                    std::fs::rename(&temporary, file)
                })();
                if result.is_err() {
                    let _ = std::fs::remove_file(temporary);
                }
                result?;
                flash(c.bind(py), "Preset saved")?;
            } else if destination == 0 {
                update(c.bind(py), json!([p]))?;
            } else {
                let folder = if destination == 1 {
                    let dir = local_dir()?;
                    std::fs::create_dir_all(&dir)?;
                    dir
                } else {
                    vals.get_item("folder")?.extract::<PathBuf>()?
                };
                let file = folder.join(format!("{}.builtpreset", shown(&p["uid"])));
                use std::io::Write;
                std::fs::File::create_new(file)?.write_all(p.to_string().as_bytes())?;
                flash(c.bind(py), "Preset saved")?;
            }
        }
        Ok(none(py))
    })?;
    let kw = PyDict::new(py);
    if capture {
        kw.set_item("note", summary)?;
    }
    kw.set_item("on_done", done)?;
    kw.set_item("buttons", [("cancel", "Cancel"), ("ok", "Save")])?;
    let form = forms.getattr("Form")?.call((title, fields), Some(&kw))?;
    if capture {
        form.setattr("placement", "cursor")?;
    }
    forms.call_method1("show", (ctx.getattr("win")?, form))?;
    Ok(())
}
fn transfer(ctx: &Bound<'_, PyAny>, pack: Option<String>) -> PyResult<()> {
    let py = ctx.py();
    let title = if pack.is_some() {
        "Export preset pack"
    } else {
        "Import preset pack"
    };
    let c = ctx.clone().unbind();
    let done = func(py, move |py, args| {
        let path = args.get_item(0)?;
        if path.is_none() {
            return Ok(none(py));
        }
        let mut path: PathBuf = path.extract()?;
        if path.is_dir() {
            path.push("pack.json");
        }
        let ctx = c.bind(py);
        if let Some(pack) = &pack {
            let rows: Vec<_> = rows(ctx)?
                .into_iter()
                .filter(|(p, _)| p["pack"] == pack.as_str())
                .map(|(p, _)| p)
                .collect();
            let encoded =
                serde_json::to_vec_pretty(&json!({"format":"built.presets","v":1,"rows":rows}))
                    .map_err(|e| PyValueError::new_err(e.to_string()))?;
            use std::io::Write;
            std::fs::File::create_new(path)?.write_all(&encoded)?;
        } else {
            let data: Value = serde_json::from_slice(&std::fs::read(path)?)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            if data["v"] == 1 && data["format"] == "built.presets" {
                update(ctx, data["rows"].clone())?;
            } else if data["uid"].is_string() {
                update(ctx, json!([data]))?;
            } else {
                return Err(PyValueError::new_err("Unknown preset pack format"));
            }
        }
        Ok(none(py))
    })?;
    py.import("built_render.chrome_forms")?
        .getattr("ask_text")?
        .call(
            (ctx.getattr("win")?, title, "Pack file or folder"),
            Some(&[("done", done)].into_py_dict(py)?),
        )?;
    Ok(())
}
#[pyclass(module="built_render.wgpu_library",extends=LibraryProvider,dict)]
pub(super) struct PresetsProvider;
#[pymethods]
impl PresetsProvider {
    #[new]
    pub(super) fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(Self)
    }
    #[classattr]
    fn id() -> &'static str {
        "presets"
    }
    #[classattr]
    fn label() -> &'static str {
        "Presets"
    }
    fn items<'p>(&self, py: Python<'p>, ctx: &Bound<'p, PyAny>) -> PyResult<Bound<'p, PyAny>> {
        let mut items = Vec::new();
        for (p, source) in rows(ctx)? {
            let mut verbs = vec!["edit", "duplicate", "export"];
            if p["primitive"] == "sweep" {
                verbs.insert(0, "apply");
            }
            if source == "model" || source == "mine" {
                verbs.push("delete");
            }
            let mut details = Vec::new();
            for section in ["values", "record"] {
                for (key, v) in p[section].as_object().into_iter().flatten() {
                    details.push((
                        format!(
                            "{} · {}",
                            match key.as_str() {
                                "class" => "IFC class",
                                "predefined_type" => "Predefined type",
                                "object_type" => "Object type",
                                "material" => "Material",
                                "profile" => "Profile",
                                "mirror" => "Mirror",
                                "rotation" => "Rotation",
                                "attachment" => "Attachment",
                                _ => key,
                            },
                            if p["locks"][key] == "fixed" {
                                "Fixed"
                            } else {
                                "Free"
                            }
                        ),
                        shown(if key == "profile" && v["library"].is_string() {
                            &v["library"]
                        } else {
                            v
                        }),
                        p["locks"][key].as_str().unwrap_or("free"),
                    ));
                }
            }
            items.push(json!({"key":p["uid"],"name":p["name"],"category":p["pack"],"source":source,"preset":p,"verbs":verbs,"defaults":details,
                "thumb":p["values"]["profile"]["library"].as_str().map(|k|format!("profile:{k}"))}));
        }
        json(py)?.call_method1("loads", (json!(items).to_string(),))
    }
    fn type_action(&self, ctx: &Bound<'_, PyAny>, action: &str) -> PyResult<()> {
        if action == "import" {
            transfer(ctx, None)
        } else {
            Ok(())
        }
    }
    #[pyo3(signature=(ctx,verb,item,_arg=""))]
    fn run(
        &self,
        ctx: &Bound<'_, PyAny>,
        verb: &str,
        item: &Bound<'_, PyAny>,
        _arg: &str,
    ) -> PyResult<()> {
        let py = ctx.py();
        let data = item.get_item("preset")?;
        let mut p: Value = serde_json::from_str(&dumps(py, &data)?)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let personal = item.get_item("source")?.eq("mine")?;
        match verb {
            "apply" if p["primitive"] == "sweep" => {
                let win = ctx.getattr("win")?;
                win.call_method1("_activate_tool", ("sweep",))?;
                win.getattr("_tools")?
                    .get_item("sweep")?
                    .getattr("_machine")?
                    .call_method1("set", ("preset", data))?;
            }
            "edit" => edit(
                ctx,
                p,
                if personal {
                    "Edit saved preset"
                } else {
                    "Edit project preset"
                },
            )?,
            "duplicate" => {
                p["uid"] = json!(uid(py)?);
                p["name"] = json!(format!("{} copy", shown(&p["name"])));
                update(ctx, json!([p]))?;
            }
            "export" => transfer(ctx, Some(shown(&p["pack"])))?,
            "delete" => ask(
                &ctx.getattr("view")?,
                (
                    if personal { "delete_saved" } else { "delete" },
                    "presets",
                    shown(&p["uid"]),
                ),
                if personal {
                    "Delete saved preset?"
                } else {
                    "Delete project preset?"
                },
                &shown(&p["name"]),
                "Delete",
            )?,
            _ => {}
        }
        Ok(())
    }
    fn confirmed(
        &self,
        ctx: &Bound<'_, PyAny>,
        what: &str,
        key: &str,
        _text: &str,
    ) -> PyResult<()> {
        if what == "delete_saved" {
            std::fs::remove_file(saved_path(key)?)?;
        } else if what == "delete" {
            let reply = core(
                ctx,
                "preset_delete",
                &[("uid", key)].into_py_dict(ctx.py())?,
            )?;
            if !reply.get_item("ok")?.is_truthy()? {
                return flash(
                    ctx,
                    &reply
                        .call_method1("get", ("error", "Preset is in use"))?
                        .str()?
                        .to_string(),
                );
            }
            notify(&ctx.getattr("vp")?)?;
        }
        Ok(())
    }
}

pub(crate) fn capture_preset(win: &Bound<'_, PyAny>, group: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let ctx = ctx_of(py, win, &win.getattr("view")?)?;
    let api = py.import("built_core.command_protocol")?;
    let scene = ctx.getattr("scene")?;
    let row = api.call_method1("live_sweep", (&scene, group.getattr("uid")?))?;
    if row.is_none() {
        return flash(&ctx, "This element has no reusable recipe");
    }
    let row: Value = serde_json::from_str(&dumps(py, &row)?)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let mut record: Value = serde_json::from_str(&dumps(py, &group.getattr("ifc")?)?)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    if let Some(obj) = record.as_object_mut() {
        for k in [
            "name",
            "uid",
            "global_id",
            "guid",
            "openings",
            "preset_note",
        ] {
            obj.remove(k);
        }
    }
    let mut values = json!({});
    for key in [
        "profile",
        "attachment",
        "rotation",
        "mirror",
        "corner",
        "segments",
    ] {
        values[key] = row[key].clone();
    }
    if let Some(sketch) = row["profile"]["sketch"].as_str() {
        let sketches = api.call_method1("live_rows", (&scene,))?;
        let sketches: Vec<Value> = serde_json::from_str(&dumps(py, &sketches)?)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let drawing = sketches
            .into_iter()
            .find(|r| r["uid"] == sketch)
            .ok_or_else(|| PyValueError::new_err("Missing sketch"))?;
        values["profile"] = json!({"drawing":drawing});
    }
    edit(
        &ctx,
        json!({"uid":uid(py)?,"name":group.getattr("name")?.str()?.to_string(),"pack":"This project","node":"","primitive":"sweep","values":values,"record":record,"locks":{"profile":"fixed"},"naming":"","rev":1}),
        "Save as preset",
    )
}
