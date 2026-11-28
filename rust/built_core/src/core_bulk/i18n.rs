// SPDX-License-Identifier: GPL-3.0-or-later
//! File-backed UI catalogues. Each interpreter starts in English.
use super::{paths, Module, Obj};
use pyo3::{
    exceptions::{PyIndexError, PyKeyError, PyValueError},
    prelude::*,
    types::{PyDict, PyFrozenSet},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

type Coverage = BTreeMap<String, (usize, usize)>;

fn source_strings() -> &'static Vec<String> {
    static SOURCES: OnceLock<Vec<String>> = OnceLock::new();
    SOURCES.get_or_init(|| serde_json::from_str(include_str!("translation_sources.json")).unwrap())
}

fn translated_count(sources: &[String], catalogue: &Value) -> usize {
    sources
        .iter()
        .filter(|key| {
            catalogue
                .get(key.as_str())
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        })
        .count()
}

fn qualifies((translated, total): (usize, usize)) -> bool {
    total > 0 && translated * 100 >= total * 98
}

/// Installed catalogues are immutable for a running application. Keep just the
/// counts, so drawing or searching Settings never decodes the catalogues again.
#[pyfunction]
fn language_coverage(py: Python<'_>) -> PyResult<Coverage> {
    static CACHE: OnceLock<Mutex<Option<(PathBuf, Coverage)>>> = OnceLock::new();
    let dir = directory(py)?;
    let mut cache = CACHE.get_or_init(Mutex::default).lock().unwrap();
    if let Some((root, counts)) = cache.as_ref() {
        if root == &dir {
            return Ok(counts.clone());
        }
    }
    let sources = source_strings();
    let total = sources.len();
    let mut counts = BTreeMap::from([("en".to_owned(), (total, total))]);
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(code) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if code == "en" {
                continue;
            }
            let data = fs::read(path.as_path())
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(Value::Null);
            counts.insert(code.to_owned(), (translated_count(sources, &data), total));
        }
    }
    *cache = Some((dir, counts.clone()));
    Ok(counts)
}

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
    translations: Py<PyDict>,
    sources: Py<PyDict>,
}

impl Catalogue {
    fn empty(py: Python<'_>, language: &str) -> Self {
        Self {
            language: language.to_owned(),
            translations: PyDict::new(py).unbind(),
            sources: PyDict::new(py).unbind(),
        }
    }

    fn load(py: Python<'_>, language: &str) -> PyResult<Self> {
        let catalogue = Self::empty(py, language);
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
            let value = value.str()?;
            catalogue.sources.bind(py).set_item(&value, &source)?;
            catalogue.translations.bind(py).set_item(source, value)?;
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
    Ok(language_coverage(py)?.into_keys().collect())
}

/// Choose the catalogue for the first matching system locale, or English.
#[pyfunction]
fn language_for_locales(locales: &Obj<'_>) -> PyResult<String> {
    let counts = language_coverage(locales.py())?;
    let codes: Vec<_> = counts.keys().collect();
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
            return Ok(if qualifies(counts[*code]) {
                (*code).clone()
            } else {
                "en".into()
            });
        }
    }
    Ok("en".into())
}

/// Resolve a saved language, including the system-language choice.
#[pyfunction(signature=(choice))]
fn resolve_language(py: Python<'_>, choice: Option<&str>) -> PyResult<String> {
    if let Some(code) = choice.filter(|s| !s.is_empty() && *s != SYSTEM) {
        return Ok(
            if language_coverage(py)?
                .get(code)
                .copied()
                .is_some_and(qualifies)
            {
                code.to_owned()
            } else {
                "en".into()
            },
        );
    }
    if py
        .import("built_render.shell_services")?
        .call_method0("available")?
        .is_truthy()?
    {
        return language_for_locales(
            &py.import("built_render")?
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

/// A snapshot for native widgets that paint without a Python interpreter.
#[pyfunction(pass_module)]
fn translations<'py>(m: &Module<'py>) -> PyResult<Bound<'py, PyDict>> {
    active(m)?.get().translations.bind(m.py()).copy()
}

/// Return the active language code.
#[pyfunction(pass_module)]
fn current_language(m: &Module<'_>) -> PyResult<String> {
    Ok(active(m)?.get().language.clone())
}

/// Return the English source for a translated label, for example a shortcut key.
#[pyfunction(pass_module)]
fn source_of<'py>(m: &Module<'py>, text: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(active(m)?
        .get()
        .sources
        .bind(m.py())
        .get_item(text)?
        .unwrap_or_else(|| text.clone()))
}

/// Optional catalogue variants for positive integer counts in languages whose
/// noun changes beyond the singular/plural distinction in English. Existing
/// catalogues and count-neutral messages keep their ordinary translation.
fn counted_variant<'py>(
    catalogue: &Catalogue,
    text: &Obj<'py>,
    kwargs: &Bound<'py, PyDict>,
) -> PyResult<Option<Obj<'py>>> {
    let lang = catalogue.language.as_str();
    if !matches!(lang, "ru" | "uk" | "pl" | "ar") {
        return Ok(None);
    }
    let Some(n) = kwargs.get_item("n")?.and_then(|v| v.extract::<u64>().ok()) else {
        return Ok(None);
    };
    let Ok(source) = text.extract::<&str>() else {
        return Ok(None);
    };
    if !source.contains("{n}") {
        return Ok(None);
    }
    let translations = catalogue.translations.bind(text.py());
    let last = n % 10;
    let hundred = n % 100;
    let form = match lang {
        "ar" if n == 2 => return translations.get_item(source.replace("{n}", "2")),
        "ar" if (3..=10).contains(&hundred) => "few",
        "ar" if (11..=99).contains(&hundred) => "many",
        "ar" => "other",
        "ru" | "uk" if last == 1 && hundred != 11 => "one",
        "pl" if n == 1 => "one",
        _ if (2..=4).contains(&last) && !(12..=14).contains(&hundred) => "few",
        _ => "many",
    };
    translations.get_item(format!("{source} [{form}]"))
}

/// Translate a source string, then apply Python string formatting if keywords are supplied.
#[pyfunction(pass_module, signature=(text, /, **kwargs))]
fn tr<'py>(
    m: &Module<'py>,
    text: &Obj<'py>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<Obj<'py>> {
    let py = m.py();
    let catalogue = active(m)?;
    // Plugins pass optional labels as None; preserve ordinary Python key
    // equality and return the original object when there is no translation.
    let out = catalogue
        .get()
        .translations
        .bind(py)
        .get_item(text)?
        .unwrap_or_else(|| text.clone());
    let Some(kwargs) = kwargs.filter(|d| !d.is_empty()) else {
        return Ok(out);
    };
    let out = counted_variant(catalogue.get(), text, kwargs)?.unwrap_or(out);
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
    m.add("_active", Py::new(py, Catalogue::empty(py, "en"))?)?;
    for f in [
        wrap_pyfunction!(available_languages, m)?,
        wrap_pyfunction!(current_language, m)?,
        wrap_pyfunction!(translations, m)?,
        wrap_pyfunction!(is_rtl, m)?,
        wrap_pyfunction!(language_for_locales, m)?,
        wrap_pyfunction!(language_coverage, m)?,
        wrap_pyfunction!(resolve_language, m)?,
        wrap_pyfunction!(set_language, m)?,
        wrap_pyfunction!(source_of, m)?,
        wrap_pyfunction!(tr, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_counts_live_nonblank_entries_and_compares_exactly() {
        let keys = ["Air", "OK", "Water", "Glass"].map(str::to_owned);
        let data = serde_json::json!({"Air": "Air", "OK": "Oui", "Water": " ", "Old": "Ancien"});
        assert_eq!(translated_count(&keys, &data), 2);
        assert!(qualifies((49, 50)));
        assert!(!qualifies((979, 1000)));
    }
}
