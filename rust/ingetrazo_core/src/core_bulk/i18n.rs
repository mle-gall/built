// SPDX-License-Identifier: GPL-3.0-or-later
//! File-backed UI catalogues. Each interpreter starts in English.
use super::{paths, Module, Obj};
use pyo3::{
    exceptions::{PyIndexError, PyKeyError, PyValueError},
    prelude::*,
    types::{PyDict, PyFrozenSet, PyString},
};
use std::{collections::HashMap, fs, path::PathBuf};

const LANGUAGE_NAMES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Español"),
    ("pt-BR", "Português (Brasil)"),
    ("zh-CN", "简体中文"),
    ("zh-TW", "正體中文"),
    ("it", "Italiano"),
    ("id", "Bahasa Indonesia"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("pl", "Polski"),
    ("ru", "Русский"),
    ("vi", "Tiếng Việt"),
    ("lo", "ລາວ"),
    ("th", "ไทย"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("uk", "Українська"),
    ("ar", "العربية"),
    ("bg", "Български"),
];
const RTL: [&str; 4] = ["ar", "he", "fa", "ur"];
const SYSTEM: &str = "system";

#[pyclass(frozen)]
struct Catalogue {
    language: String,
    translations: HashMap<String, String>,
    sources: HashMap<String, String>,
}

impl Catalogue {
    fn empty(language: &str) -> Self {
        Self {
            language: language.to_owned(),
            translations: HashMap::new(),
            sources: HashMap::new(),
        }
    }

    fn load(py: Python<'_>, language: &str) -> PyResult<Self> {
        let mut catalogue = Self::empty(language);
        if language == "en" {
            return Ok(catalogue);
        }
        let path = directory(py)?.join(format!("{language}.json"));
        let Ok(text) = fs::read_to_string(path) else {
            return Ok(catalogue);
        };
        // The JSON decoder retains insertion order; the last source of a
        // duplicate translation wins. Plugins may supply non-string values.
        let data = match py.import("json")?.call_method1("loads", (text,)) {
            Err(err) if err.is_instance_of::<PyValueError>(py) => return Ok(catalogue),
            result => result?,
        };
        for (source, value) in data.cast::<PyDict>()?.iter() {
            let source = source.extract::<String>()?;
            let value = value.str()?.extract::<String>()?;
            catalogue.sources.insert(value.clone(), source.clone());
            catalogue.translations.insert(source, value);
        }
        Ok(catalogue)
    }
}

fn directory(py: Python<'_>) -> PyResult<PathBuf> {
    Ok(paths::app_root(py)?.extract::<PathBuf>()?.join("i18n"))
}

/// Return the sorted language codes with installed JSON catalogues.
#[pyfunction]
fn available_languages(py: Python<'_>) -> PyResult<Vec<String>> {
    let dir = directory(py)?;
    if !dir.is_dir() {
        return Ok(vec!["en".into()]);
    }
    let mut codes = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(code) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.strip_suffix(".json"))
            {
                codes.push(code.to_owned());
            }
        }
    }
    codes.sort();
    Ok(codes)
}

/// Choose the catalogue for the first matching system locale, or English.
#[pyfunction]
fn language_for_locales(locales: &Obj<'_>) -> PyResult<String> {
    let codes = available_languages(locales.py())?;
    for name in locales.try_iter()? {
        let tag = name?
            .str()?
            .extract::<String>()?
            .replace('_', "-")
            .to_lowercase();
        let mut parts = tag.split('-');
        let base = parts.next().unwrap_or_default();
        let traditional = base == "zh" && parts.any(|s| matches!(s, "hant" | "tw" | "hk" | "mo"));
        let found = codes
            .iter()
            .find(|s| s.eq_ignore_ascii_case(&tag))
            .or_else(|| {
                codes
                    .iter()
                    .find(|s| traditional && s.eq_ignore_ascii_case("zh-TW"))
            })
            .or_else(|| {
                codes
                    .iter()
                    .find(|s| s.split('-').next().unwrap().eq_ignore_ascii_case(base))
            });
        if let Some(code) = found {
            return Ok(code.clone());
        }
    }
    Ok("en".into())
}

/// Resolve a saved language, including the system-language choice.
#[pyfunction(signature=(choice))]
fn resolve_language(py: Python<'_>, choice: Option<&str>) -> PyResult<String> {
    if let Some(code) = choice.filter(|s| !s.is_empty() && *s != SYSTEM) {
        return Ok(code.to_owned());
    }
    if py
        .import("ingetrazo_render.shell_services")?
        .call_method0("available")?
        .is_truthy()?
    {
        return language_for_locales(
            &py.import("ingetrazo_render")?
                .call_method0("preferred_languages")?,
        );
    }
    let language = py
        .import("locale")?
        .call_method0("getlocale")?
        .get_item(0)?;
    if language.is_none() {
        return Ok("en".into());
    }
    language_for_locales(pyo3::types::PyTuple::new(py, [language])?.as_any())
}

/// Return whether the base language is written from right to left.
#[pyfunction(signature=(lang))]
fn is_rtl(lang: Option<&str>) -> bool {
    let base = lang
        .unwrap_or_default()
        .split(['-', '_'])
        .next()
        .unwrap_or_default();
    RTL.iter().any(|s| s.eq_ignore_ascii_case(base))
}

/// Activate a catalogue. English and unreadable catalogues use the source strings.
#[pyfunction(pass_module, signature=(lang))]
fn set_language(m: &Module<'_>, lang: Option<&str>) -> PyResult<()> {
    let language = lang.filter(|s| !s.is_empty()).unwrap_or("en");
    let catalogue = Catalogue::load(m.py(), language)?;
    m.setattr("_active", Py::new(m.py(), catalogue)?)
}

fn active<'py>(m: &Module<'py>) -> PyResult<Bound<'py, Catalogue>> {
    Ok(m.getattr("_active")?.cast_into()?)
}

/// Return the active language code.
#[pyfunction(pass_module)]
fn current_language(m: &Module<'_>) -> PyResult<String> {
    Ok(active(m)?.get().language.clone())
}

/// Return the English source for a translated label, for example a shortcut key.
#[pyfunction(pass_module)]
fn source_of(m: &Module<'_>, text: &str) -> PyResult<String> {
    Ok(active(m)?
        .get()
        .sources
        .get(text)
        .map_or(text, String::as_str)
        .to_owned())
}

/// Translate a source string, then apply Python string formatting if keywords are supplied.
#[pyfunction(pass_module, signature=(text, /, **kwargs))]
fn tr<'py>(
    m: &Module<'py>,
    text: &Bound<'py, PyString>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<Obj<'py>> {
    let py = m.py();
    let catalogue = active(m)?;
    let out = match catalogue.get().translations.get(text.to_cow()?.as_ref()) {
        Some(translated) => PyString::new(py, translated),
        None => text.clone(),
    };
    let Some(kwargs) = kwargs.filter(|d| !d.is_empty()) else {
        return Ok(out.into_any());
    };
    match out.call_method("format", (), Some(kwargs)) {
        Err(err)
            if err.is_instance_of::<PyKeyError>(py)
                || err.is_instance_of::<PyIndexError>(py)
                || err.is_instance_of::<PyValueError>(py) =>
        {
            text.call_method("format", (), Some(kwargs))
        }
        result => result,
    }
}

pub(super) fn install(m: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    let names = PyDict::new(py);
    for &(code, name) in LANGUAGE_NAMES {
        names.set_item(code, name)?;
    }
    m.add("LANGUAGE_NAMES", names)?;
    m.add("RTL_LANGUAGES", PyFrozenSet::new(py, RTL)?)?;
    m.add("HIDDEN_LANGUAGES", PyFrozenSet::empty(py)?)?;
    m.add("SYSTEM", SYSTEM)?;
    m.add("_active", Py::new(py, Catalogue::empty("en"))?)?;
    for f in [
        wrap_pyfunction!(available_languages, m)?,
        wrap_pyfunction!(current_language, m)?,
        wrap_pyfunction!(is_rtl, m)?,
        wrap_pyfunction!(language_for_locales, m)?,
        wrap_pyfunction!(resolve_language, m)?,
        wrap_pyfunction!(set_language, m)?,
        wrap_pyfunction!(source_of, m)?,
        wrap_pyfunction!(tr, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
