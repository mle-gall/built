// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use regex::Regex;
use std::sync::LazyLock;

const NAMES: &[(&str, &[&str])] = &[
    ("proyecto", &["project", "projekt"]),
    ("autor", &["author", "verfasser"]),
    ("lamina", &["sheet", "plannr"]),
    ("escala", &["scale", "massstab"]),
    ("escena", &["scene", "szene"]),
    ("fecha", &["date", "datum"]),
    ("archivo", &["file", "datei"]),
    ("nombre", &["name", "planname", "sheet_name"]),
    ("hoja", &["page", "blatt"]),
    ("total", &["total", "gesamt"]),
];
static TOKENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{([A-Za-z_]+)\}").unwrap());
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let names = PyDict::new(py);
    for (key, aliases) in NAMES {
        names.set_item(key, PyTuple::new(py, *aliases)?)?;
    }
    m.add("FIELD_NAMES", names)?;
    m.add("_FIELD_CTX", PyDict::new(py))?;
    for f in [
        wrap_pyfunction!(set_field_context, m)?,
        wrap_pyfunction!(field_values, m)?,
        wrap_pyfunction!(local_field_names, m)?,
        wrap_pyfunction!(expand_fields, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
#[pyfunction(signature=(comp=None,scene=None,path=None,index=None,total=None))]
fn set_field_context(
    py: Python<'_>,
    comp: Option<&Obj>,
    scene: Option<&Obj>,
    path: Option<&Obj>,
    index: Option<&Obj>,
    total: Option<&Obj>,
) -> PyResult<()> {
    let ctx = api(py)?.getattr("_FIELD_CTX")?.cast_into::<PyDict>()?;
    ctx.clear();
    for (key, value) in [
        ("comp", comp),
        ("scene", scene),
        ("path", path),
        ("index", index),
        ("total", total),
    ] {
        ctx.set_item(key, value)?;
    }
    Ok(())
}
#[pyfunction(signature=(frame_uid=""))]
fn field_values<'p>(py: Python<'p>, frame_uid: &str) -> PyResult<Bound<'p, PyDict>> {
    let ctx = api(py)?.getattr("_FIELD_CTX")?;
    let get = |k: &str| ctx.call_method1("get", (k,));
    let comp = get("comp")?;
    let mut main = None;
    let mut area = f64::NEG_INFINITY;
    let mut bound = None;
    let mut first_scene = String::new();
    let name = if comp.is_none() {
        String::new()
    } else {
        string(&comp, "name")?
    };
    if !comp.is_none() {
        for f in comp.getattr("frames")?.try_iter()? {
            let f = f?;
            if first_scene.is_empty() {
                if let Some(n) = string(&f, "view_key")?.strip_prefix("scene:") {
                    first_scene = n.into();
                }
            }
            if bound.is_none() && !frame_uid.is_empty() && string(&f, "uid")? == frame_uid {
                bound = Some(f.clone());
            }
            let size = num(&f, "w_mm")? * num(&f, "h_mm")?;
            if size > area {
                main = Some(f);
                area = size;
            }
        }
    }
    let block = comp.getattr("cajetin").ok().filter(|c| !c.is_none());
    let row = |key: &str| -> PyResult<String> {
        let Some(c) = block.as_ref() else {
            return Ok(String::new());
        };
        let u = py.import("unicodedata")?;
        let aliases = NAMES.iter().find(|(n, _)| *n == key).unwrap().1;
        for r in c.getattr("campos")?.try_iter()? {
            let r = r?;
            let label = r.get_item(0)?.str()?.to_string().trim().to_lowercase();
            let folded = u
                .call_method1("normalize", ("NFKD", label))?
                .extract::<String>()?;
            let label: String = folded
                .chars()
                .filter(|ch| !('\u{300}'..='\u{36f}').contains(ch))
                .collect();
            if label == key || aliases.contains(&label.as_str()) {
                let value = r.get_item(1)?;
                return Ok(if value.is_truthy()? {
                    value.str()?.to_string()
                } else {
                    String::new()
                });
            }
        }
        string(c, key)
    };
    let mut scale = String::new();
    if let Some(f) = bound.or(main) {
        scale = model::format_scale(num(&f, "scale_n")?);
        if let Some(s) = string(&f, "view_key")?.strip_prefix("scene:") {
            first_scene = s.into();
        }
    }
    let out = PyDict::new(py);
    let date = py
        .import("datetime")?
        .getattr("date")?
        .call_method0("today")?
        .call_method1("strftime", ("%d/%m/%Y",))?;
    let path = get("path")?;
    let path = if path.is_truthy()? {
        py.import("pathlib")?
            .call_method1("Path", (path,))?
            .getattr("stem")?
            .extract()?
    } else {
        String::new()
    };
    let title = row("lamina")?;
    let idx = get("index")?;
    let total = get("total")?;
    for (k, v) in [
        ("proyecto", row("proyecto")?),
        ("autor", row("autor")?),
        (
            "lamina",
            if title.is_empty() {
                name.clone()
            } else {
                title
            },
        ),
        ("nombre", name),
        ("escala", scale),
        ("escena", first_scene),
        ("fecha", date.extract()?),
        ("archivo", path),
        (
            "hoja",
            if idx.is_none() {
                String::new()
            } else {
                idx.add(1)?.str()?.to_string()
            },
        ),
        (
            "total",
            if total.is_truthy()? {
                total.str()?.to_string()
            } else {
                String::new()
            },
        ),
    ] {
        out.set_item(k, v)?;
    }
    for (key, aliases) in NAMES {
        let value = out.get_item(key)?.unwrap();
        for name in *aliases {
            out.set_item(name, &value)?;
        }
    }
    Ok(out)
}
#[pyfunction]
fn local_field_names(py: Python<'_>, text: &str) -> PyResult<String> {
    let lang: String = py
        .import("core.i18n")?
        .call_method0("current_language")?
        .extract()?;
    let col = match lang.as_str() {
        "en" => 0,
        "de" => 1,
        _ => return Ok(text.into()),
    };
    Ok(TOKENS
        .replace_all(text, |caps: &regex::Captures| {
            NAMES
                .iter()
                .find(|(k, _)| *k == &caps[1])
                .map_or_else(|| caps[0].into(), |(_, a)| format!("{{{}}}", a[col]))
        })
        .into_owned())
}
#[pyfunction(signature=(text,frame_uid=""))]
fn expand_fields(py: Python<'_>, text: Option<&str>, frame_uid: &str) -> PyResult<String> {
    let text = text.unwrap_or("");
    if !text.contains('{') {
        return Ok(text.into());
    }
    let values =
        field_values(py, frame_uid)?.extract::<std::collections::HashMap<String, String>>()?;
    Ok(TOKENS
        .replace_all(text, |caps: &regex::Captures| {
            values
                .get(&caps[1].to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| caps[0].into())
        })
        .into_owned())
}
