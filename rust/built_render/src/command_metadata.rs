//! The F3 index. Cache translated rows only while their action, keys and
//! menu path still match, and rank them with the document search engine.
use super::{commands::command_description, shortcut_text::shortcut_text};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
const SEP: &str = " ▸ ";
const RECENT: &str = "command_search/recent";

fn trailing(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_end().strip_suffix(')')?;
    let (label, keys) = text.rsplit_once('(')?;
    (!keys.is_empty() && !keys.contains(')')).then_some((label.trim_end(), keys.trim()))
}
#[pyfunction]
fn _keys_in(text: &str) -> Option<String> {
    let (_, keys) = trailing(text)?;
    let portable = shortcut_text(keys)?.0;
    let compact = |s: &str| s.replace(' ', "").to_lowercase();
    (compact(&portable) == compact(keys)).then_some(portable)
}
fn label(s: &str) -> String {
    s.replace("&&", "\0")
        .replace('&', "")
        .replace('\0', "&")
        .trim()
        .into()
}
fn plain(s: &str) -> String {
    label(s)
        .trim_end_matches('…')
        .trim_end_matches('.')
        .trim()
        .into()
}
fn description(py: Python, action: &Obj, name: &str) -> PyResult<String> {
    let status = action.call_method0("statusTip")?;
    let mut tip: String = if status.is_truthy()? {
        status
    } else {
        action.call_method0("toolTip")?
    }
    .extract()?;
    tip = tip.trim().into();
    let i18n = py.import("built_core.i18n")?;
    if _keys_in(
        &i18n
            .call_method1("source_of", (&tip,))?
            .extract::<String>()?,
    )
    .is_some()
        || _keys_in(&tip).is_some()
    {
        if let Some((bare, _)) = trailing(&tip) {
            tip = bare.to_owned();
        }
    }
    let fold = py.import("built_core")?.getattr("command_fold")?;
    let bare = plain(name);
    let folded: String = fold.call1((&bare,))?.extract()?;
    if fold
        .call1((&tip,))?
        .extract::<String>()?
        .starts_with(&folded)
    {
        let mut rest = &tip[tip
            .char_indices()
            .nth(bare.chars().count())
            .map_or(tip.len(), |(i, _)| i)..];
        rest = rest.trim_start();
        if let Some(keys) = rest.strip_prefix('(') {
            if let Some((inside, after)) = keys.split_once(')') {
                if !inside.contains('(') {
                    rest = after.trim_start();
                }
            }
        }
        if rest.is_empty() {
            tip.clear();
        } else if let Some(body) = rest.strip_prefix(['—', '–', ':', '-']) {
            let mut chars = body.trim_start().chars();
            tip = chars
                .next()
                .map(|first| format!("{}{}", first.to_uppercase(), chars.as_str()))
                .unwrap_or_default();
        }
    }
    if fold.call1((plain(&tip),))?.eq(&folded)? {
        tip.clear();
    }
    Ok(tip)
}
#[pyclass(get_all, module = "built_render.command_metadata")]
struct Command {
    action: Py<PyAny>,
    key: String,
    name: String,
    path: String,
    keys: String,
    tip: String,
    items: Py<PyAny>,
}
fn action_key(action: &Obj) -> PyResult<String> {
    action
        .py()
        .import("built_render.keyboard_bindings")?
        .call_method1("action_key", (action,))?
        .extract()
}
fn binding(action: &Obj) -> PyResult<String> {
    let keys = action
        .py()
        .import("built_render.keyboard_bindings")?
        .call_method1("action_keys", (action,))?;
    Ok(keys
        .try_iter()?
        .next()
        .transpose()?
        .map(|s| s.extract())
        .transpose()?
        .unwrap_or_default())
}
#[pymethods]
impl Command {
    #[new]
    fn new(action: &Obj, path: &str) -> PyResult<Self> {
        let py = action.py();
        let i18n = py.import("built_core.i18n")?;
        let source = i18n.getattr("source_of")?;
        let text = action.call_method0("text")?;
        let mut english = plain(&source.call1((&text,))?.extract::<String>()?);
        let mut name = label(&text.extract::<String>()?);
        let mut keys = binding(action)?;
        if let Some(in_text) = _keys_in(&english) {
            english = trailing(&english).unwrap().0.into();
            if let Some((short, _)) = trailing(&name) {
                name = short.into();
            }
            if keys.is_empty() {
                keys = in_text;
            }
        }
        let keys = shortcut_text(&keys).map(|s| s.1).unwrap_or_default();
        let key = action_key(action)?;
        let mut tip = description(py, action, &name)?;
        if tip.is_empty() {
            if let Some(text) = command_description(&english, &key) {
                tip = i18n.call_method1("tr", (text,))?.extract()?;
            }
        }
        let trail: Vec<_> = path.split(SEP).filter(|s| !s.is_empty()).collect();
        let item = py.import("built_core")?.getattr("SearchItem")?;
        let mut translated = trail.clone();
        translated.push(&name);
        let mut originals = trail
            .iter()
            .map(|p| source.call1((p,))?.extract::<String>())
            .collect::<PyResult<Vec<_>>>()?;
        originals.push(english);
        let mut aliases = vec![item.call1((translated,))?, item.call1((originals,))?];
        if let Some(id) = key.strip_prefix("tool:") {
            aliases.push(item.call1((vec![id],))?);
        }
        let items = PyList::new(py, aliases)?
            .into_any()
            .unbind();
        Ok(Self {
            action: action.clone().unbind(),
            key,
            name,
            path: path.into(),
            keys,
            tip,
            items,
        })
    }
    fn __traverse__(
        &self,
        visit: pyo3::class::gc::PyVisit,
    ) -> Result<(), pyo3::class::gc::PyTraverseError> {
        visit.call(&self.action)?;
        visit.call(&self.items)
    }
}
fn recent(py: Python) -> PyResult<Vec<String>> {
    let value = py
        .import("built_core.settings")?
        .call_method0("settings")?
        .call_method1("value", (RECENT,))?;
    if !value.is_truthy()? {
        return Ok(Vec::new());
    }
    if let Ok(one) = value.extract::<String>() {
        return Ok(vec![one]);
    }
    value
        .try_iter()?
        .map(|s| Ok(s?.str()?.to_string()))
        .collect()
}
#[pyfunction]
fn run(action: &Obj) -> PyResult<()> {
    if action.call_method0("isVisible")?.is_truthy()?
        && action.call_method0("isEnabled")?.is_truthy()?
    {
        let py = action.py();
        let key = action_key(action)?;
        let mut order = recent(py)?;
        order.retain(|s| s != &key);
        order.insert(0, key);
        order.truncate(100);
        py.import("built_core.settings")?
            .call_method0("settings")?
            .call_method1("setValue", (RECENT, order))?;
        action.call_method0("trigger")?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn menu_paths<'p>(window: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let vars = window.getattr("__dict__")?;
    match vars.cast::<PyDict>()?.get_item("_command_paths")? {
        Some(paths) => paths.cast_into::<PyDict>()?.copy(),
        None => Ok(PyDict::new(window.py())),
    }
}
#[pyfunction(signature=(window,scope=""))]
fn commands<'p>(window: &Obj<'p>, scope: &str) -> PyResult<Vec<Obj<'p>>> {
    let py = window.py();
    if let Some(refresh) = window.get_type().getattr_opt("refresh_commands")? {
        refresh.call1((window,))?;
    }
    let paths = menu_paths(window)?;
    let previous = window
        .getattr_opt("_command_cache")?
        .filter(|v| !v.is_none())
        .map(|d| d.cast_into::<PyDict>())
        .transpose()?;
    let fresh = PyDict::new(py);
    let mut pool = Vec::new();
    for a in py
        .import("built_render.keyboard_bindings")?
        .call_method1("collect_actions", (window,))?
        .try_iter()?
    {
        let a = a?;
        let id = a.call_method0("objectName")?;
        if id.eq("command_search")?
            || !a.call_method0("isVisible")?.is_truthy()?
            || !a.call_method0("parent")?.is(window)
        {
            continue;
        }
        let path = paths
            .get_item(action_key(&a)?)?
            .map(|p| p.extract::<String>())
            .transpose()?
            .unwrap_or_default();
        let sig = (
            id,
            a.call_method0("text")?,
            a.call_method0("toolTip")?,
            a.call_method0("statusTip")?,
            binding(&a)?,
            &path,
        )
            .into_pyobject(py)?;
        let old = previous
            .as_ref()
            .map(|d| d.get_item(&a))
            .transpose()?
            .flatten();
        let cmd = match old {
            Some(row) if row.get_item(0)?.eq(&sig)? => row.get_item(1)?,
            _ => Bound::new(py, Command::new(&a, &path)?)?.into_any(),
        };
        fresh.set_item(&a, (&sig, &cmd))?;
        if scope.is_empty()
            || path == scope
            || path.strip_prefix(scope).is_some_and(|s| s.starts_with(SEP))
        {
            pool.push(cmd);
        }
    }
    window.setattr("_command_cache", fresh)?;
    Ok(pool)
}
#[pyfunction(signature=(window,query,pool=None))]
fn search<'p>(window: &Obj<'p>, query: &str, pool: Option<Vec<Obj<'p>>>) -> PyResult<Vec<Obj<'p>>> {
    let py = window.py();
    let pool = match pool {
        Some(p) => p,
        None => commands(window, "")?,
    };
    let short = query.trim().chars().count() <= 1;
    let order = if short { recent(py)? } else { vec![] };
    let entries = PyList::empty(py);
    for row in &pool {
        let key: String = row.getattr("key")?.extract()?;
        let rank = order
            .iter()
            .position(|k| k == &key)
            .map_or(-1, |i| (order.len() - i) as i64);
        entries.append((row.getattr("items")?, rank))?;
    }
    let folded = py
        .import("built_core")?
        .call_method1("command_fold", (query,))?;
    let ranked: Vec<usize> = py
        .import("built_core")?
        .call_method1("rank_commands", (folded, short, entries))?
        .extract()?;
    if query.trim().is_empty() && !order.is_empty() {
        let mut recent_rows = Vec::new();
        for index in &ranked {
            let row = &pool[*index];
            if order.contains(&row.getattr("key")?.extract::<String>()?) {
                recent_rows.push(row.clone());
            }
        }
        if !recent_rows.is_empty() {
            return Ok(recent_rows);
        }
    }
    Ok(ranked.into_iter().map(|i| pool[i].clone()).collect())
}
pub(super) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "built_render.command_metadata")?;
    m.add_class::<Command>()?;
    for f in [
        wrap_pyfunction!(run, &m)?,
        wrap_pyfunction!(commands, &m)?,
        wrap_pyfunction!(search, &m)?,
        wrap_pyfunction!(_keys_in, &m)?,
    ] {
        m.add_function(f)?;
    }
    py.import("sys")?
        .getattr("modules")?
        .set_item(m.name()?, &m)?;
    parent.add("command_metadata", m)
}
