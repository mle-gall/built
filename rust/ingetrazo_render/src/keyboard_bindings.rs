// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent keyboard bindings for the modelling, sheet and extension commands.
use crate::{commands::shortcut_domains, shortcut_text::shortcut_alternatives};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::collections::{HashMap, HashSet};
type Obj<'p> = Bound<'p, PyAny>;
pub(crate) const DEFAULTS: &str = "ingetrazo_default_shortcuts";
pub(crate) const GROUP: &str = "shortcuts";
const FORMAT: &str = "ingetrazo-shortcuts";

#[pyfunction]
fn action_label(text: &str) -> String {
    text.replace("&&", "\0")
        .replace('&', "")
        .replace('\0', "&")
        .trim_end_matches('…')
        .trim_end_matches('.')
        .trim()
        .to_owned()
}
#[pyfunction]
fn action_key(action: &Obj) -> PyResult<String> {
    let name = action.call_method0("objectName")?;
    if name.is_truthy()? {
        return name.extract();
    }
    let source: String = action
        .py()
        .import("ingetrazo_core.i18n")?
        .call_method1("source_of", (action.call_method0("text")?,))?
        .extract()?;
    Ok(format!("text:{}", action_label(&source)))
}
#[pyfunction]
fn collect_actions<'p>(window: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    if let Some(actions) = window
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_command_actions")?
    {
        for action in actions.call_method0("values")?.try_iter()? {
            let action = action?;
            if action.call_method0("isSeparator")?.is_truthy()?
                || !action.call_method0("menu")?.is_none()
            {
                continue;
            }
            let label = action_label(&action.call_method0("text")?.extract::<String>()?);
            if !label.is_empty() && seen.insert(action_key(&action)?) {
                rows.push((label.to_lowercase(), action));
            }
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(rows.into_iter().map(|(_, action)| action).collect())
}
#[pyfunction]
fn keyboard_scopes<'p>(window: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let vars = window.getattr("__dict__")?;
    let mut scopes = vec![window.clone()];
    if vars.contains("_documents")? && vars.contains("_command_store")? {
        scopes.push(crate::sheet_search::sheet_command_scope(window)?);
    }
    Ok(scopes)
}
#[pyfunction]
pub(crate) fn portable_keys(seqs: &Obj) -> PyResult<Vec<String>> {
    seqs.try_iter()?
        .map(|s| {
            let s = s?;
            s.extract()
                .or_else(|_| s.call_method0("toString")?.extract())
        })
        .collect()
}
#[pyfunction]
fn action_keys(action: &Obj) -> PyResult<Vec<String>> {
    if let Some(method) = action.getattr_opt("portable_keys")? {
        method.call0()?.extract()
    } else {
        portable_keys(&action.call_method0("shortcuts")?)
    }
}
fn defaults(action: &Obj) -> PyResult<Option<String>> {
    action.call_method1("property", (DEFAULTS,))?.extract()
}
#[pyfunction]
fn default_keys(action: &Obj) -> PyResult<Vec<String>> {
    Ok(shortcut_alternatives(
        &defaults(action)?.unwrap_or_default(),
        false,
    ))
}
#[pyfunction]
fn remember_defaults(window: &Obj) -> PyResult<()> {
    for act in collect_actions(window)? {
        if defaults(&act)?.is_none() {
            act.call_method1("setProperty", (DEFAULTS, action_keys(&act)?.join("; ")))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn settings_key(key: &str) -> String {
    key.replace('%', "%25")
        .replace('/', "%2F")
        .replace('\\', "%5C")
}
fn read_key(key: &str) -> String {
    key.replace("%5C", "\\")
        .replace("%2F", "/")
        .replace("%25", "%")
}
fn settings(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("ingetrazo_core.settings")?.call_method0("settings")
}
fn text(value: &Obj) -> PyResult<String> {
    Ok(if value.is_truthy()? {
        value.str()?.to_string()
    } else {
        String::new()
    })
}
fn stored(py: Python<'_>) -> PyResult<HashMap<String, String>> {
    let st = settings(py)?;
    st.call_method1("beginGroup", (GROUP,))?;
    let read = || -> PyResult<_> {
        let mut values = HashMap::new();
        for key in st.call_method0("allKeys")?.extract::<Vec<String>>()? {
            values.insert(read_key(&key), text(&st.call_method1("value", (key,))?)?);
        }
        Ok(values)
    };
    let result = read();
    st.call_method0("endGroup")?;
    result
}
#[pyfunction]
fn apply_user_shortcuts(window: &Obj) -> PyResult<usize> {
    let values = stored(window.py())?;
    let mut count = 0;
    for act in collect_actions(window)? {
        let mut key = action_key(&act)?;
        if !values.contains_key(&key) {
            let Some(legacy) = act
                .call_method1("property", ("legacyShortcutKey",))?
                .extract::<Option<String>>()?
            else {
                continue;
            };
            key = legacy;
        }
        if let Some(keys) = values.get(&key) {
            act.call_method1("setShortcuts", (shortcut_alternatives(keys, true),))?;
            count += 1;
        }
    }
    Ok(count)
}
fn persist(action: &Obj, keys: &[String]) -> PyResult<()> {
    let py = action.py();
    let st = settings(py)?;
    let key = action_key(action)?;
    let legacy = action
        .call_method1("property", ("legacyShortcutKey",))?
        .extract::<Option<String>>()?;
    if let Some(old) = legacy.filter(|old| !old.is_empty() && old != &key) {
        st.call_method1("remove", (format!("{GROUP}/{}", settings_key(&old)),))?;
    }
    let encoded = format!("{GROUP}/{}", settings_key(&key));
    let raw = format!("{GROUP}/{key}");
    if raw != encoded {
        st.call_method1("remove", (raw,))?;
    }
    let text = keys
        .iter()
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("; ");
    if text == defaults(action)?.unwrap_or_default() {
        st.call_method1("remove", (encoded,))?;
    } else {
        st.call_method1("setValue", (encoded, text))?;
    }
    st.call_method0("sync")?;
    for host in py
        .import("ingetrazo_render.shell_windows")?
        .call_method0("hosts")?
        .try_iter()?
    {
        let window = host?.getattr("window")?;
        let owner = if key.starts_with("sheet:") {
            window
                .getattr("__dict__")?
                .cast::<PyDict>()?
                .get_item("_sheet_command_scope")?
                .filter(|v| !v.is_none())
        } else {
            Some(window)
        };
        if let Some(owner) = owner {
            if let Some(actions) = owner
                .getattr("__dict__")?
                .cast::<PyDict>()?
                .get_item("_registry_actions")?
            {
                if let Some(peer) = actions
                    .cast::<PyDict>()?
                    .get_item(&key)?
                    .filter(|a| !a.is(action))
                {
                    peer.call_method1("setShortcuts", (keys,))?;
                }
            }
        }
    }
    Ok(())
}
#[pyfunction]
fn save_shortcut(action: &Obj, seqs: &Obj) -> PyResult<()> {
    persist(action, &portable_keys(seqs)?)
}
fn actions<'p>(window: &Obj<'p>) -> PyResult<Vec<(String, Obj<'p>)>> {
    let mut out = Vec::new();
    for scope in keyboard_scopes(window)? {
        for act in collect_actions(&scope)? {
            out.push((action_key(&act)?, act));
        }
    }
    Ok(out)
}
#[pyfunction]
fn export_shortcuts<'p>(window: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(window.py());
    let shortcuts = PyDict::new(window.py());
    for (id, act) in actions(window)? {
        shortcuts.set_item(id, action_keys(&act)?.join("; "))?;
    }
    out.set_item("format", FORMAT)?;
    out.set_item("version", 1)?;
    out.set_item("shortcuts", shortcuts)?;
    Ok(out)
}
type Clashes<'p> = Vec<(String, Obj<'p>, Obj<'p>)>;
#[pyfunction]
fn import_shortcuts<'p>(window: &Obj<'p>, data: &Obj<'p>) -> PyResult<(usize, usize, Clashes<'p>)> {
    let valid = if let Ok(d) = data.cast::<PyDict>() {
        d.get_item("format")?
            .is_some_and(|v| v.extract::<String>().is_ok_and(|s| s == FORMAT))
            && d.get_item("shortcuts")?
                .is_some_and(|v| v.is_instance_of::<PyDict>())
    } else {
        false
    };
    if !valid {
        return Err(PyValueError::new_err(
            window
                .py()
                .import("ingetrazo_core.i18n")?
                .call_method1("tr", ("This is not an IngeTrazo shortcuts file.",))?
                .extract::<String>()?,
        ));
    }
    let wanted = PyDict::new(window.py());
    for (k, v) in data.get_item("shortcuts")?.cast::<PyDict>()? {
        wanted.set_item(k.str()?, text(&v)?)?;
    }
    let actions = actions(window)?;
    let by_key: HashMap<_, _> = actions.iter().map(|(k, a)| (k.as_str(), a)).collect();
    let mut taken: HashMap<String, Vec<(u8, Obj)>> = HashMap::new();
    let mut clashes = Vec::new();
    let mut applied = 0;
    for (key, text) in &wanted {
        let key = key.extract::<String>()?;
        let Some(&act) = by_key.get(key.as_str()) else {
            continue;
        };
        let domain = shortcut_domains(&key);
        let mut keys = Vec::new();
        for seq in shortcut_alternatives(&text.extract::<String>()?, true) {
            let holders = taken.entry(seq.clone()).or_default();
            if let Some((_, owner)) = holders.iter().find(|(mask, _)| mask & domain != 0) {
                clashes.push((seq, owner.clone(), act.clone()));
            } else {
                holders.push((domain, act.clone()));
                keys.push(seq);
            }
        }
        act.call_method1("setShortcuts", (&keys,))?;
        persist(act, &keys)?;
        applied += 1;
    }
    for (id, act) in actions {
        if wanted.contains(&id)? {
            continue;
        }
        let domain = shortcut_domains(&id);
        let mut keys = action_keys(&act)?;
        let before = keys.len();
        keys.retain(|seq| {
            !taken
                .get(seq)
                .is_some_and(|owners| owners.iter().any(|(mask, _)| mask & domain != 0))
        });
        if keys.len() != before {
            act.call_method1("setShortcuts", (&keys,))?;
            persist(&act, &keys)?;
        }
    }
    Ok((applied, wanted.len() - applied, clashes))
}
pub(crate) fn register(root: &Bound<PyModule>) -> PyResult<()> {
    let py = root.py();
    let name = "ingetrazo_render.keyboard_bindings";
    let m = PyModule::new(py, name)?;
    m.add("EXPORT_FORMAT", FORMAT)?;
    for f in [
        wrap_pyfunction!(action_label, &m)?,
        wrap_pyfunction!(action_key, &m)?,
        wrap_pyfunction!(collect_actions, &m)?,
        wrap_pyfunction!(keyboard_scopes, &m)?,
        wrap_pyfunction!(action_keys, &m)?,
        wrap_pyfunction!(remember_defaults, &m)?,
        wrap_pyfunction!(default_keys, &m)?,
        wrap_pyfunction!(settings_key, &m)?,
        wrap_pyfunction!(apply_user_shortcuts, &m)?,
        wrap_pyfunction!(save_shortcut, &m)?,
        wrap_pyfunction!(export_shortcuts, &m)?,
        wrap_pyfunction!(import_shortcuts, &m)?,
    ] {
        m.add_function(f)?;
    }
    py.import("sys")?.getattr("modules")?.set_item(name, &m)?;
    root.add("keyboard_bindings", m)
}
