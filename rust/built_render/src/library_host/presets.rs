// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::exceptions::PyValueError;
use serde_json::{json, Value};

pub(super) fn listed(ctx: &Bound<'_, PyAny>) -> PyResult<Vec<(Value, String)>> {
    rows(ctx)
}
fn rows(ctx: &Bound<'_, PyAny>) -> PyResult<Vec<(Value, String)>> {
    let py = ctx.py();
    let scene = ctx
        .getattr_opt("scene")?
        .filter(|s| s.hasattr("_root").unwrap_or(false));
    let catalog = py
        .import("built_core.command_protocol")?
        .call_method1("preset_catalog", (scene.clone(),))?;
    let mut items: Vec<(Value, String)> = serde_json::from_str(&dumps(py, &catalog)?)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let saved = super::destinations::saved_ids(py, scene.as_ref(), "presets")?;
    for (_, preset) in saved_presets()? {
        // Listed from the Library once migration copied it there.
        let id = preset["uid"].as_str().unwrap_or_default().to_lowercase();
        if saved.contains(&id) || saved.contains(&crate::library_store::stable_id(&format!("preset:{id}"))) {
            continue;
        }
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
fn edit(ctx: &Bound<'_, PyAny>, preset: Value, title: &str, lib: Option<String>) -> PyResult<()> {
    let py = ctx.py();
    let forms = py.import("built_render.chrome_forms")?;
    let field = forms.getattr("Field")?;
    let fields = PyList::empty(py);
    let personal = title == "Edit saved preset";
    for (key, label) in [
        ("name", "Name"),
        ("pack", "Pack"),
        ("naming", "Naming pattern"),
        ("node", "Filing node"),
    ] {
        fields.append(field.call1((key, "edit", label, shown(&preset[key])))?)?;
    }
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
            if let Some(key) = &lib {
                p["rev"] = json!(p["rev"].as_u64().unwrap_or(0).saturating_add(1));
                let name = shown(&p["name"]);
                let place = super::destinations::resave(&c.bind(py).getattr("win")?, key, &name, p)?;
                flash(c.bind(py), &format!("Preset saved in {place}"))?;
            } else if personal {
                // A preset still in the older saved folder.
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
            } else {
                update(c.bind(py), json!([p]))?;
            }
        }
        Ok(none(py))
    })?;
    let kw = PyDict::new(py);
    kw.set_item("on_done", done)?;
    kw.set_item("buttons", [("cancel", "Cancel"), ("ok", "Save")])?;
    let form = forms.getattr("Form")?.call((title, fields), Some(&kw))?;
    forms.call_method1("show", (ctx.getattr("win")?, form))?;
    Ok(())
}
/// A preset's values and record defaults as detail rows.
fn details(p: &Value) -> Vec<(String, String, String)> {
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
                    if p["locks"][key] == "fixed" { "Fixed" } else { "Free" }
                ),
                shown(if key == "profile" && v["library"].is_string() { &v["library"] } else { v }),
                p["locks"][key].as_str().unwrap_or("free").to_owned(),
            ));
        }
    }
    details
}
/// Export pack… for sweep, cut and Repeat presets: every preset of the
/// pack, as a format 1 file (junction details use Share pack…).
fn export_v1(ctx: &Bound<'_, PyAny>, pack: String) -> PyResult<()> {
    let py = ctx.py();
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
        let rows: Vec<_> = rows(ctx)?
            .into_iter()
            .filter(|(p, _)| p["pack"] == pack.as_str())
            .map(|(p, _)| p)
            .collect();
        let encoded = serde_json::to_vec_pretty(&json!({"format":"built.presets","v":1,"rows":rows}))
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        use std::io::Write;
        std::fs::File::create_new(path)?.write_all(&encoded)?;
        Ok(none(py))
    })?;
    py.import("built_render.chrome_forms")?
        .getattr("ask_text")?
        .call(
            (ctx.getattr("win")?, "Export preset pack", "Pack file or folder"),
            Some(&[("done", done)].into_py_dict(py)?),
        )?;
    Ok(())
}
/// A format 1 pack goes into this model in one step, as before.
pub(super) fn import_v1(ctx: &Bound<'_, PyAny>, rows: Value) -> PyResult<()> {
    update(ctx, rows)
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
        // Junction details: their open matches, and Follow (docs/junctions.md §4.4).
        let scene = ctx.getattr_opt("scene")?.filter(|s| s.hasattr("_root").unwrap_or(false));
        let counts: Value = match &scene {
            Some(scene) => serde_json::from_str(&dumps(py, &py.import("built_core.command_protocol")?
                .call_method1("junction_counts", (scene,))?)?).unwrap_or_default(),
            None => Value::Null,
        };
        let uses = super::packs::uses(py, scene.as_ref())?;
        let model = super::packs::model_presets(py, scene.as_ref())?;
        for (p, source) in rows(ctx)? {
            let junction = p["primitive"] == "junction";
            let mut verbs = vec!["edit", "duplicate", "export"];
            if matches!(p["primitive"].as_str(), Some("sweep" | "cut" | "repeat" | "junction")) {
                verbs.insert(0, "apply");
            }
            if junction {
                verbs.retain(|v| *v != "export");
                verbs.push("share");
            }
            if source == "model" || source == "mine" {
                verbs.push("delete");
            }
            let mut details = if junction {
                let used = uses.get(p["uid"].as_str().unwrap_or("")).copied().unwrap_or(0);
                let (rows, _) = super::packs::describe(py, &p, used, None)?;
                rows.into_iter()
                    .filter_map(|r| serde_json::from_value::<(String, String, String)>(r).ok())
                    .collect()
            } else {
                details(&p)
            };
            let open_matches = &counts["details"][p["uid"].as_str().unwrap_or_default()];
            if open_matches.is_object() {
                let open = open_matches["counts"]["match"].as_u64().unwrap_or(0);
                if open > 0 {
                    verbs.insert(0, "review");
                }
                verbs.push(if open_matches["follow"] == true { "unfollow" } else { "follow" });
                let state = if open_matches["follow"] == true {
                    crate::settings_rows::tr(py, "On · {n} new")?.replace("{n}", &open.to_string())
                } else {
                    crate::settings_rows::tr(py, "Off")?
                };
                details.insert(0, (crate::settings_rows::tr(py, "Follow")?, state, "free".into()));
                details.insert(0, (crate::settings_rows::tr(py, "Similar meetings")?, open.to_string(), "free".into()));
            }
            let mut row = json!({"key":p["uid"],"name":p["name"],"category":p["pack"],"source":source,"verbs":verbs,"defaults":details,
                "base":p["primitive"],"thumb":p["values"]["profile"]["library"].as_str().map(|k|format!("profile:{k}"))});
            if junction {
                row["thumb"] = json!(super::packs::junction_thumb(py, &p)?);
                row["uses"] = json!(uses.get(p["uid"].as_str().unwrap_or("")).copied().unwrap_or(0));
            }
            row["preset"] = p;
            items.push(row);
        }
        // Presets saved to a project, My library or a shared library.
        for mut row in super::destinations::items(py, scene.as_ref(), &["presets"])? {
            row["base"] = row["preset"]["primitive"].clone();
            if row["preset"]["primitive"] == "junction" {
                let p = row["preset"].clone();
                let used = uses.get(p["uid"].as_str().unwrap_or("")).copied().unwrap_or(0);
                let offer = super::packs::update_offer(&p, &model);
                let (mut defaults, thumb) = super::packs::describe(py, &p, used, offer)?;
                defaults.extend(row["defaults"].as_array().cloned().unwrap_or_default());
                row["defaults"] = json!(defaults);
                if row["thumb"].is_null() {
                    row["thumb"] = json!(thumb);
                }
                row["uses"] = json!(used);
                if let Some(verbs) = row["verbs"].as_array_mut() {
                    let at = verbs.iter().position(|v| v == "-").unwrap_or(verbs.len());
                    verbs.insert(at, json!("share"));
                    if offer.is_some() {
                        verbs.insert(at, json!("modelupdate"));
                    }
                }
                items.push(row);
                continue;
            }
            let mut rows: Vec<Value> = details(&row["preset"]).into_iter().map(|d| json!(d)).collect();
            rows.extend(row["defaults"].as_array().cloned().unwrap_or_default());
            row["defaults"] = json!(rows);
            row["thumb"] = json!(row["preset"]["values"]["profile"]["library"].as_str().map(|k| format!("profile:{k}")));
            items.push(row);
        }
        json(py)?.call_method1("loads", (json!(items).to_string(),))
    }
    fn type_action(&self, ctx: &Bound<'_, PyAny>, action: &str) -> PyResult<()> {
        if action == "import" {
            super::packs::ask_import(ctx)
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
        let lib: Option<String> = if item.contains("lib")? { Some(item.get_item("key")?.extract()?) } else { None };
        let personal = item.get_item("source")?.eq("mine")? || lib.is_some();
        match verb {
            "apply" if p["primitive"] == "junction" => super::packs::apply(ctx, &p)?,
            "share" => super::packs::share(ctx, &p)?,
            "modelupdate" => super::packs::update_model(ctx, &p)?,
            "apply" if p["primitive"] == "repeat" => {
                let scene = ctx.getattr("scene")?;
                let selection = scene.getattr("selection")?;
                if selection.len()? != 1 {
                    return flash(ctx, "Select a Repeat to apply the preset");
                }
                let g = selection.try_iter()?.next().unwrap()?;
                let api = py.import("built_core.command_protocol")?;
                let Some(owner) = g.getattr_opt("uid")? else {
                    return flash(ctx, "Select a Repeat to apply the preset");
                };
                let row = api.call_method1("live_repeat", (&scene, owner))?;
                if row.is_none() {
                    return flash(ctx, "Select a Repeat to apply the preset");
                }
                let link = PyDict::new(py);
                link.set_item("uid", data.get_item("uid")?)?;
                link.set_item("definition", &data)?;
                row.set_item("repeat_preset", link)?;
                let kw = PyDict::new(py);
                kw.set_item("row", row)?;
                let reply = api.getattr("run")?.call(
                    (ctx.getattr("vp")?.getattr("history")?, "live_set"),
                    Some(&kw),
                )?;
                if !reply.get_item("ok")?.is_truthy()? {
                    return flash(ctx, &reply.get_item("error")?.str()?.to_string());
                }
                notify(&ctx.getattr("vp")?)?;
            }
            "apply" if matches!(p["primitive"].as_str(),Some("sweep"|"cut")) => {
                let win = ctx.getattr("win")?;
                win.call_method1("_activate_tool", (p["primitive"].as_str().unwrap(),))?;
                win.getattr("_tools")?
                    .get_item(p["primitive"].as_str().unwrap())?
                    .getattr("_machine")?
                    .call_method1("set", ("preset", data))?;
            }
            "review" => {
                let vp = ctx.getattr("vp")?;
                crate::junction_review::start(&vp.getattr("scene")?, &vp.getattr("history")?, &shown(&p["uid"]))?;
                notify(&vp)?;
            }
            "follow" | "unfollow" => {
                let api = py.import("built_core.command_protocol")?;
                let kw = PyDict::new(py);
                kw.set_item("detail", shown(&p["uid"]))?;
                kw.set_item("on", verb == "follow")?;
                api.getattr("run")?.call((ctx.getattr("vp")?.getattr("history")?, "junction_follow"), Some(&kw))?;
                notify(&ctx.getattr("vp")?)?;
            }
            "edit" => edit(
                ctx,
                p,
                if personal {
                    "Edit saved preset"
                } else {
                    "Edit project preset"
                },
                lib,
            )?,
            "duplicate" => {
                p["uid"] = json!(uid(py)?);
                p["name"] = json!(format!("{} copy", shown(&p["name"])));
                update(ctx, json!([p]))?;
            }
            "export" if p["primitive"] == "junction" => super::packs::share(ctx, &p)?,
            "export" => export_v1(ctx, shown(&p["pack"]))?,
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
        let repeat = api.call_method1("live_repeat", (&scene, group.getattr("uid")?))?;
        if repeat.is_none() {
            return flash(&ctx, "This element has no reusable recipe");
        }
        let mut values = json!({});
        let spec: Value = serde_json::from_str(&dumps(py, &repeat)?)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        for field in ["rule", "orientation", "adapt", "ends", "measure"] {
            if let Some(value) = spec.get(field) {
                values[field] = value.clone();
            }
        }
        let mut record: Value = serde_json::from_str(&dumps(py, &group.getattr("ifc")?)?)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        if let Some(fields) = record.as_object_mut() {
            fields.retain(|k, _| matches!(k.as_str(), "class" | "predefined_type" | "object_type" | "material"));
        }
        let name = group.getattr("name")?.extract::<String>()?;
        let preset = json!({"uid":uid(py)?,"name":name,"pack":"My presets","node":"","primitive":"repeat","values":values,"record":record,"locks":{},"naming":"","rev":1});
        return super::destinations::save_preset(win, &name, preset);
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
    if let Some(cuts)=row["cut_rows"].as_array().filter(|c|!c.is_empty()) {
        let mut cuts=cuts.clone();
        for cut in &mut cuts {
            if let Some(obj)=cut.as_object_mut(){for key in ["uid","owner","state","inputs"] {obj.remove(key);}}
        }
        values["cut_definitions"]=json!(cuts);
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
    let name = group.getattr("name")?.str()?.to_string();
    let id = uid(py)?;
    let preset = json!({"uid":id,"name":name,"pack":"My presets","node":"","primitive":"sweep","values":values,"record":record,"locks":{"profile":"fixed"},"naming":"","rev":1});
    super::destinations::save_preset(win, &name, preset)
}
