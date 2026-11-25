// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for metal_coverage.
use super::*;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
fn registry<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tools = imported(py, "built_render")?.call_method0("window_tool_types")?;
    let result = py.get_type::<PyDict>().call1((tools,))?;
    for entry in imported(py, "tools.solid_host")?
        .getattr("SOLID_TOOLS")?
        .try_iter()?
    {
        let [key, cls] = unpack(&entry?)?;
        result.set_item(key, cls.getattr("__name__")?)?;
    }
    Ok(result)
}
struct CoverageClass {
    module: String,
    bases: Vec<String>,
    hooks: BTreeSet<String>,
}
fn source_classes(
    py: Python,
    root: &Path,
    classes: &mut HashMap<String, CoverageClass>,
) -> PyResult<()> {
    static HOOKS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?:viewport|vp)\.(pick_\w+|\w*preview\w*|_world_to_pixel|_pixel_to_ray)",
        )
        .unwrap()
    });
    let ast = py.import("ast")?;
    for file in std::fs::read_dir(root.join("tools"))? {
        let path = file?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("py") {
            continue;
        }
        let source = std::fs::read_to_string(&path)?;
        let parsed = ast.call_method1("parse", (&source,))?;
        for node in parsed.getattr("body")?.try_iter()? {
            let node = node?;
            if !node.is_instance(&ast.getattr("ClassDef")?)? {
                continue;
            }
            let code = ast
                .call_method1("get_source_segment", (&source, &node))?
                .extract::<String>()?;
            let mut hooks = HOOKS
                .captures_iter(&code)
                .map(|m| m[1].to_string())
                .collect::<BTreeSet<_>>();
            if code.contains("rubber_band_lines") {
                hooks.insert("rubber_band_lines".into());
            }
            if code.contains("scale_box_state") {
                hooks.insert("scale_box_state (screen handles)".into());
            }
            let bases = node
                .getattr("bases")?
                .try_iter()?
                .map(|b| ast.call_method1("unparse", (b?,))?.extract())
                .collect::<PyResult<Vec<_>>>()?;
            let module = format!(
                "tools.{}",
                path.file_stem().unwrap_or_default().to_string_lossy()
            );
            classes.insert(
                node.getattr("name")?.extract()?,
                CoverageClass {
                    module,
                    bases,
                    hooks,
                },
            );
        }
    }
    Ok(())
}
fn table<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let value = a.get_item(0)?;
    let root = if value.is_truthy()? {
        value.extract::<PathBuf>()?
    } else {
        let package: PathBuf = py.import("tools")?.getattr("__file__")?.extract()?;
        package
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("."))
            .to_owned()
    };
    let mut classes = HashMap::new();
    for (module, name, bases) in super::super::namespace::tool_module_classes() {
        classes.insert(
            name.into(),
            CoverageClass {
                module: module.into(),
                bases: bases.split(',').map(str::to_owned).collect(),
                hooks: BTreeSet::new(),
            },
        );
    }
    source_classes(py, &root, &mut classes)?;
    let dispatch = imported(py, "built_render")?.getattr("TOOL_DISPATCH")?;
    let registered = imported(py, "tools.metal_coverage")?
        .call_method1("registry", (&root,))?
        .cast_into::<PyDict>()?;
    let mut rows = vec![
        "| Registry key | Class | Metal status | GL hooks / preview dependencies |".to_string(),
        "| --- | --- | --- | --- |".into(),
    ];
    for (key, cls) in registered.iter() {
        let key = key.extract::<String>()?;
        let cls = cls.extract::<String>()?;
        let status = if dispatch.contains(&cls)? {
            "native"
        } else {
            "inert with note"
        };
        let note = if key == "section" {
            " — after the crate work"
        } else {
            ""
        };
        let mut hooks = BTreeSet::new();
        let mut seen = HashSet::new();
        let mut pending = vec![cls.clone()];
        while let Some(name) = pending.pop() {
            let Some(meta) = classes.get(&name) else {
                continue;
            };
            if !seen.insert(name.clone()) || name == "Tool" {
                continue;
            }
            if name == "RustTool" {
                hooks.insert("Rust state machine".to_string());
                continue;
            }
            hooks.extend(super::super::tool_preview_hooks(
                root.clone(),
                &meta.module,
                &name,
            )?);
            hooks.extend(meta.hooks.iter().cloned());
            pending.extend(meta.bases.iter().cloned());
        }
        let hooks = if hooks.is_empty() {
            "shared viewport / tool base".into()
        } else {
            hooks.into_iter().collect::<Vec<_>>().join(", ")
        };
        rows.push(format!("| {key} | {cls} | {status}{note} | {hooks} |"));
    }
    rows.join("\n").into_bound_py_any(py)
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("ast", py.import("ast")?)?;
    module.add("re", py.import("re")?)?;
    install_methods(
        module,
        module.dict().as_any(),
        &[
            ("registry", Mode::Function, registry, &["root"], "", ""),
            ("table", Mode::Function, table, &["root=None"], "", ""),
        ],
    )?;
    Ok(())
}
