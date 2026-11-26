//! A checked set is valid for one document revision and query. Row ordinals
//! never cross that boundary. Sorting changes only the displayed order.
use super::*;

fn revision<'py>(ctx: &Bound<'py, PyAny>) -> PyResult<(usize, Bound<'py, PyAny>)> {
    let scene = ctx.getattr("scene")?;
    Ok((scene.as_ptr() as usize, scene.getattr("content_version")?))
}

pub(super) fn checked<'py>(
    st: &Bound<'py, PyAny>,
    ctx: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PySet>> {
    let origin = st.call_method1("get", ("replacement_origin",))?;
    let stamp = (
        revision(ctx)?,
        if origin.is_none() {
            st.get_item("filter")?
        } else {
            origin
        },
    );
    if !st.call_method1("get", ("bulk_stamp",))?.eq(&stamp)? {
        st.set_item("replacement_origin", st.py().None())?;
        st.set_item("checked", PySet::empty(st.py())?)?;
        st.set_item("bulk_mode", "")?;
        st.set_item("bulk_stamp", stamp)?;
    }
    Ok(st.get_item("checked")?.cast_into()?)
}

pub(super) fn controls(
    st: &Bound<'_, PyAny>,
    content: &Bound<'_, PyAny>,
    sheet: &Bound<'_, PyAny>,
    out: &Bound<'_, PyAny>,
    ctx: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = st.py();
    let picks = checked(st, ctx)?;
    let matches = out.get_item("matches")?;
    st.set_item("component_matches", &matches)?;
    sheet.setattr("_checked", &picks)?;
    if st.get_item("bulk_mode")?.eq("replace")? {
        content.call_method1("caption", ("Choose a replacement definition",))?;
        return Ok(());
    }
    let n = matches.len()?;
    let mut actions = vec![cs_verb(
        py,
        "bulk-all",
        &format!("Select all {n} matches"),
        &[],
    )?];
    if !picks.is_empty() {
        actions.push(cs_verb(
            py,
            "bulk-clear",
            &format!("Clear {} checked", picks.len()),
            &[],
        )?);
    }
    content.call_method(
        "buttons",
        (actions,),
        Some(&[("align", "left")].into_py_dict(py)?),
    )?;
    if n == 0 {
        content.call_method1("text", ("No matching definitions.",))?;
    }
    Ok(())
}

pub(super) fn detail(st: &Bound<'_, PyAny>, d: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = st.py();
    let picks = st.get_item("checked")?;
    let n = picks.len()?;
    let mode: String = st.get_item("bulk_mode")?.extract()?;
    d.call_method1("heading", (format!("{n} definitions checked"),))?;
    let actions = match mode.as_str() {
        "purge" => {
            d.call_method1("caption", ("Review unused definitions",))?;
            let mut review = Vec::new();
            let mut retained = false;
            for key in picks.try_iter()? {
                let item = st.get_item("items")?.get_item(key?)?;
                let reason = if !item.get_item("source")?.eq("model")? {
                    "Read-only source"
                } else if item.get_item("uses")?.is_truthy()? {
                    "In use"
                } else {
                    retained = true;
                    "Kept in file"
                };
                review.push((item.get_item("name")?.extract::<String>()?, reason));
            }
            review.sort();
            d.call_method1("text", ("0 eligible for deletion",))?;
            d.call_method1("kv", (review,))?;
            if retained {
                vec![
                    ("bulk-release", "Stop keeping reviewed templates…"),
                    ("bulk-back", "Back"),
                ]
            } else {
                vec![("bulk-back", "Back")]
            }
        }
        "replace" => {
            d.call_method1(
                "text",
                ("Focus the replacement definition in the table, then confirm.",),
            )?;
            let key = st.get_item("cursor")?;
            let pick = st.get_item("items")?.call_method1("get", (&key,))?;
            if !pick.is_none() && pick.get_item("source")?.eq("model")? && !picks.contains(key)? {
                d.call_method1(
                    "kv",
                    (vec![(
                        "Replacement",
                        pick.get_item("name")?.extract::<String>()?,
                    )],),
                )?;
                vec![("bulk-apply", "Replace copies"), ("bulk-back", "Cancel")]
            } else {
                vec![("bulk-back", "Cancel")]
            }
        }
        _ => vec![
            ("bulk-select", "Select copies in the model"),
            ("bulk-replace", "Replace with…"),
            ("bulk-purge", "Purge unused…"),
            ("bulk-move", "Move to collection…"),
        ],
    };
    let buttons = actions
        .into_iter()
        .map(|(id, label)| cs_verb(py, id, label, &[]))
        .collect::<PyResult<Vec<_>>>()?;
    d.call_method(
        "buttons",
        (buttons,),
        Some(&[("align", "fill")].into_py_dict(py)?),
    )?;
    Ok(())
}

pub(super) fn act(
    st: &Bound<'_, PyAny>,
    ctx: &Bound<'_, PyAny>,
    action: &str,
    key: &str,
) -> PyResult<()> {
    let py = ctx.py();
    let picks = checked(st, ctx)?;
    let matches = st.call_method1("get", ("component_matches", PyList::empty(py)))?;
    match action {
        "bulk-check" | "bulk-range" if matches.contains(key)? => {
            if action == "bulk-range" {
                let anchor = st.call_method1("get", ("bulk_anchor",))?;
                if matches.contains(&anchor)? {
                    let start: usize = matches.call_method1("index", (anchor,))?.extract()?;
                    let end: usize = matches.call_method1("index", (key,))?.extract()?;
                    for i in start.min(end)..=start.max(end) {
                        picks.add(matches.get_item(i)?)?;
                    }
                    return Ok(());
                }
            }
            if picks.contains(key)? {
                picks.discard(key)?;
            } else {
                picks.add(key)?;
            }
            st.set_item("bulk_anchor", key)?;
        }
        "bulk-all" => {
            picks.call_method1("update", (matches,))?;
        }
        "bulk-clear" => {
            picks.clear();
            st.set_item("bulk_mode", "")?;
        }
        "bulk-back" => {
            if st.get_item("bulk_mode")?.eq("replace")? {
                restore_query(st, ctx)?;
            }
            st.set_item("bulk_mode", "")?;
        }
        "bulk-purge" => st.set_item("bulk_mode", "purge")?,
        "bulk-replace" => {
            st.set_item("replacement_origin", st.get_item("filter")?)?;
            st.set_item("bulk_mode", "replace")?;
            set_query(st, ctx, "source:model")?;
        }
        "bulk-release" if st.get_item("bulk_mode")?.eq("purge")? => {
            let scene = ctx.getattr("scene")?;
            let vp = ctx.getattr("vp")?;
            let rows = provider(py, "components")?.call_method1("_model", (ctx,))?;
            let mut targets = Vec::new();
            let mut references = std::collections::HashSet::new();
            for kept in scene.getattr("kept_components")?.try_iter()? {
                let children = kept?.getattr("children")?;
                if children.is_truthy()? {
                    for child in py
                        .import("built_core")?
                        .call_method1("purge_iter_groups", (children,))?
                        .try_iter()?
                    {
                        references.insert(child?.getattr("mesh")?.as_ptr() as usize);
                    }
                }
            }
            for key in picks.iter() {
                let key: String = key.extract()?;
                if let Some(index) = key
                    .strip_prefix("cm:")
                    .and_then(|k| k.parse::<usize>().ok())
                {
                    let row = rows.get_item(index)?;
                    let kept = row.get_item(3)?;
                    if !kept.is_none()
                        && row.get_item(2)?.len()? == 0
                        && !references.contains(&(kept.getattr("mesh")?.as_ptr() as usize))
                    {
                        targets.push(kept.unbind());
                    }
                }
            }
            if targets.is_empty() {
                return flash(ctx, "All reviewed definitions are referenced.");
            }
            let n = targets.len();
            let stamp = scene.getattr("content_version")?.unbind();
            let (scene, vp) = (scene.unbind(), vp.unbind());
            let done = func(py, move |py, a| {
                let vp = vp.bind(py);
                if a.get_item(0)?.eq("release")?
                    && vp.getattr("scene")?.is(scene.bind(py))
                    && scene
                        .bind(py)
                        .getattr("content_version")?
                        .eq(stamp.bind(py))?
                {
                    let history = py.import("built_core.history")?;
                    let steps = PyList::empty(py);
                    for target in &targets {
                        steps.append(
                            history
                                .call_method1("KeepComponentCommand", (target.bind(py), false))?,
                        )?;
                    }
                    vp.getattr("history")?.call_method1(
                        "execute",
                        (history.call_method1("CompoundCommand", (steps,))?,),
                    )?;
                    vp.call_method0("notify_scene_changed")?;
                }
                Ok(none(py))
            })?;
            py.import("built_render.chrome_dialogs")?
                .getattr("ask_later")?
                .call(
                    (
                        ctx.getattr("win")?,
                        format!("Stop keeping {n} unused definitions?"),
                        "These reviewed templates will leave this file. Undo restores them.",
                        vec![("cancel", "Cancel"), ("release", "Stop keeping")],
                        done,
                    ),
                    Some(
                        &[
                            ("default", "cancel"),
                            ("cancel", "cancel"),
                            ("tone", "warning"),
                        ]
                        .into_py_dict(py)?,
                    ),
                )?;
        }
        "bulk-move" => super::move_to_collection(st, ctx, &picks)?,
        "bulk-select" | "bulk-apply" if !picks.is_empty() => {
            let win = ctx.getattr("win")?;
            let scene = ctx.getattr("scene")?;
            let model = provider(py, "components")?.call_method1("_model", (ctx,))?;
            let groups = PyList::empty(py);
            let links = py.import("built_core")?;
            for key in picks.iter() {
                let key: String = key.extract()?;
                if let Some(i) = key
                    .strip_prefix("cm:")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    for g in model.get_item(i)?.get_item(2)?.try_iter()? {
                        let g = g?;
                        if action == "bulk-select"
                            || (links.call_method1("link_linked_ancestor", (&g,))?.is_none()
                                && !scene.call_method1("entity_locked", (&g,))?.is_truthy()?)
                        {
                            groups.append(g)?;
                        }
                    }
                }
            }
            if action == "bulk-select" {
                crate::component_rows::select_component_instances(&win, groups.extract()?)?;
                cs(py)?.call_method1("close", (&win,))?;
            } else {
                let dest: String = st.get_item("cursor")?.extract()?;
                if !st.get_item("bulk_mode")?.eq("replace")?
                    || picks.contains(&dest)?
                    || !st
                        .get_item("items")?
                        .get_item(&dest)?
                        .get_item("source")?
                        .eq("model")?
                {
                    return Ok(());
                }
                if let Some(i) = dest
                    .strip_prefix("cm:")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    let row = model.get_item(i)?;
                    let stored = row.get_item(3)?;
                    let source = if stored.is_none() {
                        row.get_item(2)?.get_item(0)?
                    } else {
                        stored
                    };
                    restore_query(st, ctx)?;
                    crate::component_replacement::replace_groups(&win, &groups, &source)?;
                    st.set_item("bulk_mode", "")?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn set_query(st: &Bound<'_, PyAny>, ctx: &Bound<'_, PyAny>, query: &str) -> PyResult<()> {
    st.set_item("filter", query)?;
    let provider = _sheet_provider(&ctx.getattr("view")?)?;
    cs(st.py())?.getattr("open")?.call(
        (ctx.getattr("win")?, provider),
        Some(&[("search", query)].into_py_dict(st.py())?),
    )?;
    Ok(())
}

fn restore_query(st: &Bound<'_, PyAny>, ctx: &Bound<'_, PyAny>) -> PyResult<()> {
    if let Some(query) = st
        .call_method1("pop", ("replacement_origin", st.py().None()))?
        .extract::<Option<String>>()?
    {
        set_query(st, ctx, &query)?;
    }
    Ok(())
}
