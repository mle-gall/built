// SPDX-License-Identifier: GPL-3.0-or-later
//! The chrome's inspector (`views.wgpu_ui`): what is selected or picked in
//! the tree, in plain words; the linked files' rows and verbs; the IFC
//! status chips; and the end of a frame.
use super::chrome_frame::{attr, id, own, render, truthy, ui};
use super::*;
use pyo3::exceptions::{PyException, PyKeyError, PyOSError, PyValueError};
use pyo3::types::PyCFunction;

/// Past this many entries the inspector reads the selection by kind.
const HUGE_SELECTION: usize = 50_000;

fn empty<'py>(py: Python<'py>) -> Bound<'py, PyList> {
    PyList::empty(py)
}
fn list<'py>(values: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    Ok(values.py().get_type::<PyList>().call1((values,))?.cast_into()?)
}
fn text(value: &Obj<'_>) -> PyResult<String> {
    value.str()?.extract()
}
/// `str(value or fallback)`.
fn text_or(value: Obj<'_>, fallback: &str) -> PyResult<String> {
    if value.is_truthy()? { text(&value) } else { Ok(fallback.to_owned()) }
}
/// `mapping.get(key)`.
fn get<'py>(mapping: &Obj<'py>, key: impl IntoPyObject<'py>) -> Reply<'py> {
    mapping.call_method1("get", (key,))
}
/// `value or {}`.
fn or_dict<'py>(value: Obj<'py>) -> Reply<'py> {
    Ok(if value.is_truthy()? { value } else { PyDict::new(value.py()).into_any() })
}
/// The chips "All existing", "All new"… or, for one item, "Existing"….
fn status_chips<'py>(py: Python<'py>, prefix: &str, all: bool) -> PyResult<Bound<'py, PyList>> {
    let chips = empty(py);
    for value in ui(py)?.getattr("STATUS_CHOICES")?.try_iter()? {
        let value: String = value?.extract()?;
        let label = if all { format!("All {}", value.to_lowercase()) } else { text(&value.as_str().into_bound_py_any(py)?.call_method0("capitalize")?)? };
        chips.append((format!("{prefix}{value}"), label))?;
    }
    Ok(chips)
}
/// Items with an IFC class and no Status.
fn missing_status<'py>(py: Python<'py>, groups: &Obj<'py>) -> PyResult<usize> {
    let structure = py.import("built_core.structure")?;
    let mut missing = 0;
    for group in groups.try_iter()? {
        let record = or_dict(group?.getattr("ifc")?)?;
        if get(&record, "class")?.is_truthy()? && !structure.call_method1("_has_status", (&record,))?.is_truthy()? {
            missing += 1;
        }
    }
    Ok(missing)
}
/// "Junction · 8 uses of its detail", "Junction · custom", "Bolt group in
/// a junction": what a junction assembly is, for the inspector's header.
fn junction_subtitle(record: &Obj<'_>) -> PyResult<String> {
    let parent = record.get_item("parent")?;
    if !parent.is_none() {
        let name = text_or(parent.get_item("name")?, "a junction")?;
        return Ok(format!("Junction · part of {name}"));
    }
    let detail = record.get_item("detail")?;
    if detail.is_none() || !detail.is_truthy()? {
        return Ok("Junction · custom".into());
    }
    let uses: usize = record.get_item("uses")?.extract().unwrap_or(1);
    Ok(if uses <= 1 { "Junction · the only use of its detail".into() } else { format!("Junction · {uses} uses of its detail") })
}
fn show<'py>(view: &Obj<'py>, title: impl IntoPyObject<'py>, sections: impl IntoPyObject<'py>) -> PyResult<()> {
    view.getattr("renderer")?.call_method1("ui_set_inspector", (title, sections)).map(drop)
}
/// `renderer.ui_set_inspector(*given)`.
fn show_given<'py>(view: &Obj<'py>, given: &Obj<'py>) -> PyResult<()> {
    view.getattr("renderer")?.getattr("ui_set_inspector")?.call1(positional(given)?).map(drop)
}

/// What is in front of you, in plain words (council 2026-11-22): a node
/// picked in the tree, one item, several, or loose geometry. Fields edit in
/// place; IFC stays under Export details.
fn inspector<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let module = ui(py)?;
    let glue = render(py)?;
    let core = py.import("built_core")?;
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let state = view.getattr("__dict__")?;
    let focus = attr(&view, "_ui_insp_node")?;
    let sel_sig = direct(py, super::chrome_frame::sel_key, (scene.getattr("selection")?,))?;
    let node_focus = if focus.is_truthy()? && focus.get_item(1)?.eq(&sel_sig)? { focus.get_item(0)? } else { none(py) };
    let filename = text_or(attr(&win, "_current_path")?, "")?;
    let tool = module.call_method1("_active_tool_id", (&win,))?;
    let brush = if tool.eq("paint")? {
        let paint = py.import("tools.paint")?.getattr("PaintTool")?;
        let texture = or_dict(paint.getattr("current_texture")?)?;
        (paint.getattr("current_is_default")?, paint.getattr("current_color")?, get(&texture, "path")?, get(&texture, "sw")?, get(&texture, "sh")?, get(&texture, "rot")?)
            .into_bound_py_any(py)?
    } else {
        none(py)
    };
    let sig = PyTuple::new(
        py,
        [
            id(&scene).into_bound_py_any(py)?,
            scene.getattr("content_version")?,
            sel_sig,
            node_focus.clone(),
            (!attr(&view, "_ui_tree_sig")?.is_none()).into_bound_py_any(py)?,
            get(&state, "_ui_mat_focus")?,
            get(&state, "_ui_mode")?,
            // (The tree's columns are not here: they never change the inspector.)
            filename.as_str().into_bound_py_any(py)?,
            direct(py, super::chrome_tree::current_material_name, ())?,
            tool,
            brush,
            scene.getattr("show_section_planes")?,
            scene.getattr("show_section_cuts")?,
            scene.getattr("display_style")?.getattr("section_fill")?,
            get(&state, "_ui_sheet_revision")?,
        ],
    )?;
    if sig.as_any().eq(attr(&view, "_ui_insp_sig")?)? {
        return Ok(none(py));
    }
    let selection = scene.getattr("selection")?;
    let sel = list(&selection)?;
    // A huge selection (a select-all of a big model) is read by kind: its
    // placements and other objects one by one, its mesh elements by count.
    let by_kind = sel.len() >= HUGE_SELECTION && selection.hasattr("values_of")?;
    let of_kind = |kind: &str| -> PyResult<Bound<'py, PyList>> { Ok(selection.call_method1("values_of", (kind,))?.cast_into()?) };
    module.getattr("_measure")?.call((&view, &win), Some(&fields!(py; "auto" => true)))?;
    let takeoff = own(&scene, "_inspector_takeoff")?;
    view.setattr("_ui_insp_sig", sig)?;
    let counter = py.import("collections")?.getattr("Counter")?;
    let structure = py.import("built_core.structure")?;
    let links = py.import("built_core.links")?;
    let group_type = py.import("built_core.group")?.getattr("Group")?;
    let groups = empty(py);
    for entity in if by_kind { of_kind("placement")? } else { sel.clone() }.iter() {
        if entity.is_instance(&group_type)? {
            groups.append(entity)?;
        }
    }
    let focused = node_focus.is_truthy()?;
    let focus_name = if focused { text(&node_focus)? } else { String::new() };
    let renderer = view.getattr("renderer")?;
    let registry = py.import("core.bim_registry")?.call_method1("get", (&scene,))?;
    let nodes = structure.call_method1("_nodes", (&registry,))?;
    let data = or_dict(attr(&view, "_ui_tree")?)?;
    let library = glue.getattr("wgpu_library")?;
    if !library.call_method1("inspector_key", (&view, &win))?.is_none() {
        let got = library.call_method1("inspector", (&view, &win))?;
        if !got.is_none() {
            show_given(&view, &got)?;
            return Ok(none(py));
        }
    }
    let kinds = module.getattr("KINDS")?;
    let identity = || core.call_method1("selection_identity", (&scene, &data, &filename, &kinds));
    let section_key = if focused {
        focus_name.clone()
    } else if sel.len() == 1 && sel.get_item(0)?.is_instance(&py.import("built_core.section")?.getattr("SectionPlane")?)? {
        format!("section:{}", text(&sel.get_item(0)?.getattr("uid")?)?)
    } else {
        String::new()
    };
    if focus_name == "solid:active" {
        let edited = scene.getattr("edit_group")?;
        let title = if edited.is_truthy()? { edited.getattr("name")? } else { "Loose geometry".into_bound_py_any(py)? };
        show(&view, title, core.call_method1("solid_inspector_sections", (&scene, py.None(), true))?)?;
        return Ok(none(py));
    }
    if ["sheet-page:", "sheet-frame:", "saved-scene:"].iter().any(|prefix| focus_name.starts_with(prefix)) {
        let sheet = view.getattr("_ui_sheet_catalog")?.call_method1("inspector", (&node_focus,))?;
        if !sheet.is_none() {
            show_given(&view, &sheet)?;
            return Ok(none(py));
        }
    }
    if section_key.starts_with("section:") || section_key.starts_with("sections:") {
        let section = core.call_method1("section_inspector", (&scene, &section_key))?;
        if !section.is_none() {
            if !focused && !sel.is_empty() {
                let fields = identity()?;
                show(&view, fields.get_item(0)?.get_item(1)?, section.get_item(1)?)?;
                renderer.call_method1("ui_set_identity", (fields,))?;
            } else {
                show_given(&view, &section)?;
            }
            return Ok(none(py));
        }
    }
    let tree_nodes = or_dict(get(&data, "nodes")?)?;
    if !node_focus.is_none() && tree_nodes.contains(&node_focus)? && get(&data.get_item("nodes")?.get_item(&node_focus)?, "kind")?.ne("host")? {
        show_given(&view, &module.call_method1("_linked_inspector", (&view, &win, &data, &node_focus))?)?;
        return Ok(none(py));
    }
    let edit = |key: String, value: Obj<'py>| module.call_method1("_edit", (key, value));
    let qty_rows = |per_unit: Obj<'py>| direct(py, super::chrome_tree::qty_rows, (per_unit,));
    if !node_focus.is_none() && nodes.contains(&node_focus)? {
        let node = nodes.get_item(&node_focus)?;
        let recipe = structure.call_method1("resolve", (&registry, &node_focus))?;
        let own_recipe = node.call_method1("get", ("recipe", PyDict::new(py)))?;
        let members = direct(py, super::chrome_tree::tree_groups, (&view, &node_focus))?;
        let missing = missing_status(py, &members)?;
        let kind = if node.contains("source")? { "Reference".into_bound_py_any(py)? } else { direct(py, super::chrome_tree::kind_of, (get(&recipe, "class")?,))? };
        let rows = empty(py);
        rows.append(("Name", edit(format!("nodename:{focus_name}"), node.get_item("name")?)?))?;
        rows.append(("Kind", kind))?;
        rows.append(("Items", members.len()?.to_string()))?;
        if missing != 0 {
            rows.append(("Missing Status", missing.to_string()))?;
        }
        let sections = empty(py);
        sections.append(("Node", rows))?;
        sections.append((
            "Actions",
            [
                (format!("kindmenu:{focus_name}"), "Change kind…"),
                (format!("active:{focus_name}"), "Draw into"),
                (format!("isolate:{focus_name}"), "Isolate"),
                (format!("cutlist:{focus_name}"), "Cut list…"),
            ],
        ))?;
        if missing != 0 {
            sections.append(("Actions", status_chips(py, &format!("nodestatus:{focus_name}:"), true)?))?;
        }
        let export = empty(py);
        export.append(("IFC class", text_or(get(&recipe, "class")?, "none (drawing only)")?))?;
        export.append(("Predefined type", edit(format!("predef:{focus_name}"), text_or(get(&own_recipe, "predefined_type")?, "")?.into_bound_py_any(py)?)?))?;
        export.append(("Object type", edit(format!("objtype:{focus_name}"), text_or(get(&own_recipe, "object_type")?, "")?.into_bound_py_any(py)?)?))?;
        export.append(("Tag", text_or(get(&recipe, "tag")?, "—")?))?;
        for classification in recipe.call_method1("get", ("classifications", empty(py)))?.get_item(pyo3::types::PySlice::new(py, 0, 4, 1))?.try_iter()? {
            let classification = classification?;
            let field = |name: &str, fallback: &str| -> PyResult<String> { text(&classification.call_method1("get", (name, fallback))?) };
            let label = format!("{} {}", field("code", "")?, field("name", "")?);
            export.append((classification.call_method1("get", ("system", "Classification"))?, label.as_str().into_bound_py_any(py)?.call_method0("strip")?))?;
        }
        if get(&recipe, "systems")?.is_truthy()? {
            export.append(("Systems", ", ".into_bound_py_any(py)?.call_method1("join", (recipe.get_item("systems")?,))?))?;
        }
        match &takeoff {
            Some(takeoff) => {
                let total = PyDict::new(py);
                for group in members.try_iter()? {
                    for entry in takeoff.call_method1("quantities", (group?,))?.call_method0("items")?.try_iter()? {
                        let [unit, value] = unpack::<2>(&entry?)?;
                        let so_far = total.as_any().call_method1("get", (&unit, 0.0))?;
                        total.set_item(unit, so_far.add(value)?)?;
                    }
                }
                let rows = qty_rows(total.into_any())?;
                if rows.is_truthy()? {
                    sections.append(("Quantities", rows))?;
                } else {
                    sections.append(("Quantities", [("Measured", "nothing to measure")]))?;
                }
            }
            None => sections.append(("Actions", [("measure", "Measure quantities")]))?,
        }
        sections.append(("Export details", export))?;
        show(&view, node.get_item("name")?, sections)?;
        return Ok(none(py));
    }
    if sel.is_empty() {
        show(&view, "", empty(py))?;
        return Ok(none(py));
    }
    let identity = identity()?;
    let is_curve = groups.is_empty() && complete_curve(&scene, &sel)?;
    let tag_row = glue.call_method1("inspector_tag_row", (&scene,))?;
    let has_tag_picker = tag_row.is_truthy()?;
    let facts = |offer_filing: bool| -> PyResult<Bound<'py, PyList>> {
        let facts = empty(py);
        for fact in identity.try_iter()? {
            let fact = fact?;
            let label = fact.get_item(0)?;
            if label.eq("Tag")? && has_tag_picker { continue; }
            if is_curve && label.eq("Kind")? {
                facts.append((label, "Curve", fact.get_item(2)?, fact.get_item(3)?))?;
            } else if label.eq("Filed in")? && offer_filing {
                facts.append(("Filing", "Not filed · File to…", "insp:file", py.None()))?;
            } else {
                facts.append(fact)?;
            }
        }
        Ok(facts)
    };
    let sel_kind = glue.call_method1("selection_kind", (&scene, if by_kind { selection.clone() } else { sel.clone().into_any() }))?;
    let offered = |verb: &str| -> PyResult<bool> { glue.call_method1("verbs_offered", (&sel_kind, verb))?.is_truthy() };
    if sel.len() == 1 && !groups.is_empty() {
        let group = groups.get_item(0)?;
        let record = or_dict(group.getattr("ifc")?)?;
        let mut key = data.call_method1("get", ("node_of", PyDict::new(py)))?.call_method1("get", (id(&group),))?;
        if !key.is_truthy()? {
            key = structure.call_method1("node_of", (&group,))?;
        }
        let filed = nodes.contains(&key)?;
        let recipe = data.call_method1("get", ("recipes", PyDict::new(py)))?.call_method1("get", (&key, PyDict::new(py)))?;
        let name = text_or(group.getattr("name")?, "")?;
        let name_cell = if glue.call_method1("verbs_name_editable", (&sel_kind,))?.is_truthy()? {
            edit(format!("groupname:{}", text(&group.getattr("uid")?)?), name.as_str().into_bound_py_any(py)?)?
        } else {
            name.as_str().into_bound_py_any(py)?
        };
        let rows = empty(py);
        rows.append(("Name", name_cell))?;
        rows.call_method1("extend", (&tag_row,))?;
        if glue.call_method1("verbs_read_only", (&sel_kind,))?.is_truthy()? {
            rows.append(("Edits", "Read-only here"))?;
        }
        let definition = if group.call_method0("is_component")?.is_truthy()? {
            super::chrome_tree::component_definition(&view, &win, &group)?
        } else { None };
        let link = links.call_method1("link_of", (&group,))?;
        let part = sel_kind.eq("linked_part")?;
        let needs = if link.is_truthy()? || part {
            ("Record", "Kept in the source file".to_owned())
        } else if !filed {
            ("Filing", "Not filed".to_owned())
        } else if !record.is_truthy()? {
            ("Record", "Comes with a kind".to_owned())
        } else {
            ("Status", text_or(direct(py, status_text, (&record,))?, "Missing")?)
        };
        // A junction's assembly is a component of its own, but what the
        // user selected is the junction: say so, and offer no copy verbs.
        let record_of = |g: &Obj<'py>| -> PyResult<Option<Obj<'py>>> {
            Ok(match g.getattr("ext")? {
                ext if ext.is_truthy()? && ext.contains("built.live")? => py
                    .import("built_core.command_protocol")?
                    .call_method1("junction_record", (&scene, g.getattr("uid")?))
                    .ok(),
                _ => None,
            })
        };
        let junction = record_of(&group)?;
        // A part a junction placed (its plate, a bolt): it belongs to that
        // junction, it is not a component to copy or make unique.
        let part_of = match &junction {
            Some(_) => None,
            None => match group.getattr("_parent")? {
                parent if !parent.is_none() && parent.hasattr("ext")? => record_of(&parent)?,
                _ => None,
            },
        };
        let sections = empty(py);
        if definition.is_some() && junction.is_none() && part_of.is_none() && offered("unique")? {
            sections.append(("Actions", [("definition", "Open definition in Library")]))?;
            sections.append(("Component actions", [("copies", "Select copies"), ("unique", "Make unique")]))?;
        }
        sections.append(("Item", rows))?;
        let offer_filing = !filed && offered("fileto")?;
        if !offer_filing { sections.append(("What it needs", [needs]))?; }
        if get(&record, "class")?.is_truthy()? && offered("status")? {
            sections.append(("Actions", status_chips(py, "status:", false)?))?;
        }
        if part {
            sections.append(("Actions", [("linkfile", "Select its linked file"), ("linkopen", "Edit source")]))?;
        }
        let children = attr(&group, "children")?;
        if children.is_truthy()? {
            let names = empty(py);
            for child in children.try_iter()? {
                names.append(text_or(child?.getattr("name")?, "Part")?)?;
            }
            let parts = counter.call1((names,))?;
            let rows = empty(py);
            for entry in parts.call_method1("most_common", (8,))?.try_iter()? {
                let [name, count] = unpack::<2>(&entry?)?;
                rows.append((name, format!("×{}", text(&count)?)))?;
            }
            if parts.len()? > 8 {
                rows.append(("…", format!("{} more kinds", parts.len()? - 8)))?;
            }
            sections.append(("Parts", rows))?;
            sections.append(("Actions", [("cutlist", "Cut list…")]))?;
        }
        // The same under every column preset: the quick check first and, when
        // it finds a fault, each fault with its own repair.
        let mut solid = core.call_method1("solid_inspector_sections", (&scene, &group, false))?;
        if py.get_type::<PyDict>().call1((&solid,))?.contains("Actions")? {
            solid = core.call_method1("solid_inspector_sections", (&scene, &group, true))?;
        }
        if solid.is_truthy()? {
            if offered("repair")? {
                sections.call_method1("extend", (solid,))?;
            } else {
                for section in solid.try_iter()? {
                    let section = section?;
                    if section.get_item(0)?.ne("Actions")? {
                        sections.append(section)?;
                    }
                }
            }
        }
        if link.is_truthy()? {
            let state = direct(py, link_state, (&scene, &link))?;
            let nested = link.call_method1("get", ("include_nested", true))?.is_truthy()?;
            let source = text(&link.call_method1("get", ("path", ""))?)?;
            let rows = empty(py);
            rows.append(("Source", source.rsplit('/').next().unwrap_or("")))?;
            rows.append(("Status", module.getattr("LINK_STATES")?.call_method1("get", (&state, &state))?))?;
            rows.call_method1("extend", (core.call_method1("link_subset_rows", (&group,))?,))?;
            rows.append(("Nested links", if nested { "included" } else { "source only" }))?;
            sections.append(("Linked file", rows))?;
            sections.append((
                "Actions",
                [
                    ("linkopen", "Edit source"),
                    ("linkreload", if state.ne("modified")? { "Reload" } else { "Reload changes" }),
                    ("linkunlink", "Unlink"),
                    ("linkfinder", "Show in Finder"),
                ],
            ))?;
            sections.append((
                "Actions",
                [
                    ("linkrelink", "Relink…"),
                    ("linkpack", if state.eq("packed")? { "Unpack" } else { "Pack into this file" }),
                    ("linknested", if nested { "Source only" } else { "Include nested links" }),
                ],
            ))?;
            if module.getattr("built_core")?.call_method1("link_linked_ancestor", (group.getattr("owner")?,))?.is_none() {
                sections.append(("Actions", [("linksubset", "Edit linked subset…")]))?;
            }
        }
        match takeoff.filter(|takeoff| takeoff.is_truthy().unwrap_or(false)) {
            Some(takeoff) => {
                let quantities = takeoff.call_method1("quantities", (&group,))?;
                if quantities.is_truthy()? {
                    sections.append(("Quantities", qty_rows(quantities)?))?;
                }
                let actions = takeoff.call_method1("face_actions", (&group,))?;
                if actions.is_truthy()? {
                    sections.append(("Actions", actions))?;
                }
            }
            None if group.getattr("ifc")?.is_truthy()? => sections.append(("Actions", [("measure", "Measure quantities")]))?,
            None => {}
        }
        let either = |field: &str, fallback: &str| -> PyResult<String> {
            let own = get(&record, field)?;
            if own.is_truthy()? { text(&own) } else { text_or(get(&recipe, field)?, fallback) }
        };
        sections.append((
            "Export details",
            [
                ("IFC class", either("class", "Not set")?),
                ("Predefined type", either("predefined_type", "—")?),
            ],
        ))?;
        if let Some((_, definition)) = definition {
            let n = definition.get_item(2)?.len()?;
            let subtitle = match (&junction, &part_of) {
                (Some(j), _) => junction_subtitle(j)?,
                (None, Some(j)) => {
                    let detail = j.get_item("detail")?;
                    let name = if detail.is_truthy()? { text_or(detail.get_item("name")?, "a junction")? } else { "a custom junction".into() };
                    format!("Part of {name}")
                }
                _ => format!("Component · {n} {}", if n == 1 { "copy" } else { "copies" }),
            };
            renderer.call_method1("ui_set_inspector", (definition.get_item(0)?, sections, subtitle))?;
        } else {
            show(&view, identity.get_item(0)?.get_item(1)?, sections)?;
        }
        renderer.call_method1("ui_set_identity", (facts(offer_filing)?,))?;
        return Ok(none(py));
    }
    let by_node = counter.call0()?;
    let mut unfiled = 0;
    for group in groups.iter() {
        let key = structure.call_method1("node_of", (group,))?;
        if nodes.contains(&key)? {
            let name = nodes.get_item(&key)?.get_item("name")?;
            by_node.set_item(&name, by_node.get_item(&name)?.add(1)?)?;
        } else {
            unfiled += 1;
        }
    }
    let rows = empty(py);
    let counted = |rows: &Bound<'py, PyList>, tally: Obj<'py>| -> PyResult<()> {
        for entry in tally.try_iter()? {
            let [name, count] = unpack::<2>(&entry?)?;
            rows.append((name, text(&count)?))?;
        }
        Ok(())
    };
    counted(&rows, by_node.call_method1("most_common", (8,))?)?;
    if unfiled != 0 {
        rows.append(("Not filed", unfiled.to_string()))?;
    }
    let others = sel.len() - groups.len();
    if others != 0 && by_kind {
        // The counts the walk below gives, faces and edges first.
        let snapshot = selection.call_method0("snapshot")?;
        let names = empty(py);
        for (slot, name) in [(2, "Faces"), (3, "Edges")] {
            let n: usize = snapshot.get_item(slot)?.extract()?;
            if n != 0 {
                names.append((name, n))?;
            }
        }
        let rest = counter.call0()?;
        for kind in ["placement", "vertex", "other"] {
            for entity in of_kind(kind)?.iter() {
                if entity.is_instance(&group_type)? {
                    continue;
                }
                let name = entity.get_type().name()?;
                rest.set_item(&name, rest.get_item(&name)?.add(1)?)?;
            }
        }
        for entry in rest.call_method0("items")?.try_iter()? {
            names.append(entry?)?;
        }
        let tally = counter.call1((py.import("builtins")?.getattr("dict")?.call1((names,))?,))?;
        counted(&rows, tally.call_method0("most_common")?)?;
    } else if others != 0 {
        let mesh = py.import("built_core.mesh")?;
        let (edge, face) = (mesh.getattr("Edge")?, mesh.getattr("Face")?);
        let names = empty(py);
        for entity in sel.iter() {
            if entity.is_instance(&group_type)? {
                continue;
            }
            if entity.is_instance(&face)? {
                names.append("Faces")?;
            } else if entity.is_instance(&edge)? {
                names.append("Edges")?;
            } else {
                names.append(entity.get_type().name()?)?;
            }
        }
        counted(&rows, counter.call1((names,))?.call_method0("most_common")?)?;
    }
    rows.call_method1("extend", (&tag_row,))?;
    let sections = empty(py);
    sections.append((if groups.is_empty() { "Geometry" } else { "By node" }, &rows))?;
    for entity in if by_kind { of_kind("other")? } else { sel.clone() }.iter() {
        if entity.get_type().name()?.to_str()? == "Dimension" {
            sections.append(("Actions", [("form:dimstyle", "Dimension style…")]))?;
            break;
        }
    }
    let shape = if is_curve { regular_curve(&scene, &sel)? } else { none(py) };
    if !shape.is_none() {
        sections.insert(0, glue.call_method1("inspector_curve", (&scene, shape))?)?;
    } else if others != 0 {
        rows.call_method1("extend", (glue.call_method1("inspector_totals", (&scene, &sel))?,))?;
    }
    let missing = missing_status(py, groups.as_any())?;
    if missing != 0 && offered("status")? {
        sections.append(("Status", [("Missing on", format!("{missing} items"))]))?;
        sections.append(("Actions", status_chips(py, "status:", true)?))?;
    }
    if !groups.is_empty() {
        sections.append(("Export details", empty(py)))?;
    }
    let title = if is_curve { "Curve".to_owned() } else if sel.len() == 1 {
        text(&identity.get_item(0)?.get_item(1)?)?
    } else {
        format!("{} selected", sel.len())
    };
    show(&view, title, sections)?;
    renderer.call_method1("ui_set_identity", (facts(false)?,))?;
    Ok(none(py))
}

/// A linked row in the inspector: what it is (a reference to another file,
/// read-only here), where that file is, its state and visibility, and what
/// can be done with it.
fn linked_inspector<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, data, key) = (a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let module = ui(py)?;
    let path_type = py.import("pathlib")?.getattr("Path")?;
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let edits = module.getattr("built_core")?.getattr("tag_edits")?;
    let tree = data.get_item("tree")?;
    let nodes = tree.getattr("nodes")?;
    let node = nodes.get_item(&key)?;
    let name = text(&key)?;
    let count = text(&data.call_method1("get", ("counts", PyDict::new(py)))?.call_method1("get", (&key, 0))?)?;
    let kind: String = node.get_item("kind")?.extract()?;
    let children = tree.getattr("children")?.call_method1("get", (&key, PyTuple::empty(py)))?;
    let (rows, acts) = (empty(py), empty(py));
    if kind == "linked" {
        let files = empty(py);
        for child in children.try_iter()? {
            let child = child?;
            let file = nodes.get_item(&child)?;
            if file.get_item("kind")?.eq("link")? && files.len() < 12 {
                files.append((file.get_item("name")?, format!("{} items", text(&data.get_item("counts")?.call_method1("get", (&child, 0))?)?)))?;
            }
        }
        let mut linked = 0;
        for child in children.try_iter()? {
            linked += nodes.get_item(child?)?.get_item("kind")?.eq("link")? as usize;
        }
        rows.append(("What", "Files this document references"))?;
        rows.append(("Files", linked.to_string()))?;
        rows.append(("Items", &count))?;
        rows.call_method1("extend", (files,))?;
        acts.append((format!("lnk:eye:{name}"), "Show / hide all"))?;
        acts.append((format!("isolate:{name}"), "Isolate"))?;
        return (node.get_item("name")?, [("Linked files", rows), ("Actions", acts)]).into_bound_py_any(py);
    }
    let path = text_or(node.get_item("path")?, "")?;
    let file = path_type.call1((&path,))?;
    let file_name = file.getattr("name")?;
    let stems = node.get_item("stems")?;
    if kind == "link" {
        let copies = tree.getattr("links")?.call_method1("get", (&key, empty(py)))?;
        let placed = copies.len()?;
        let top = stems.len()? == 1;
        let folders = match get(&node, "folders")? {
            folders if folders.is_truthy()? => folders,
            _ => PyList::new(py, [node.get_item("folder")?])?.into_any(),
        };
        let mut visible = true;
        for folder in folders.try_iter()? {
            if !edits.call_method1("folder_visible", (&scene, folder?))?.is_truthy()? {
                visible = false;
                break;
            }
        }
        let entry = or_dict(scene.getattr("link_registry")?.call_method1("get", (&path,))?)?;
        let overrides = or_dict(get(&entry, "overrides")?)?.len()?;
        let parent = file.getattr("parent")?;
        let folder_parts = parent.getattr("parts")?;
        let place = if folder_parts.len()? <= 3 {
            text(&parent)?
        } else {
            let last = folder_parts.get_item(pyo3::types::PySlice::new(py, -2, isize::MAX, 1))?;
            format!("…/{}", text(&"/".into_bound_py_any(py)?.call_method1("join", (last,))?)?)
        };
        let state = py.import("built_core.link_structure")?.call_method("file_state", (&scene, &path), Some(&fields!(py; "nested" => !top)))?;
        let missing = state.eq("missing")?;
        let visibility = format!(
            "{}{}",
            if visible { "Shown" } else { "Hidden" },
            if overrides != 0 { format!(" · {overrides} tag override{}", if overrides != 1 { "s" } else { "" }) } else { String::new() }
        );
        rows.append(("What", "Linked file (read-only)"))?;
        rows.append(("File", &file_name))?;
        rows.append(("Folder", place))?;
        rows.append(("Status", module.getattr("LINK_STATES")?.call_method1("get", (&state, &state))?))?;
        rows.append(("Placed", format!("{placed} {}", if placed == 1 { "copy" } else { "copies" })))?;
        rows.append(("Items", &count))?;
        rows.append(("Visibility", visibility))?;
        if !top {
            let via = " › ".into_bound_py_any(py)?.call_method1("join", (stems.get_item(pyo3::types::PySlice::new(py, 0, -1, 1))?,))?;
            rows.insert(2, ("Via", via))?;
        }
        if get(&node, "merged")?.is_truthy()? {
            rows.append(("Top node", node.get_item("merged")?))?;
        }
        // The file verbs first (the verb source, verbs.rs), then what any row offers.
        for verb in render(py)?.call_method1("verbs_file", (!top, &state))?.try_iter()? {
            let [verb, label] = unpack::<2>(&verb?)?;
            acts.append((format!("lnk:{}:{name}", text(&verb)?), label))?;
        }
        if placed == 1 {
            let copy = copies.get_item(0)?;
            rows.call_method1("extend", (py.import("built_core")?.call_method1("link_subset_rows", (&copy,))?,))?;
            if top && !missing {
                acts.append((format!("lnk:linksubset:{name}"), "Edit linked subset…"))?;
                let nested = py.import("built_core.links")?.call_method1("link_of", (copy,))?.call_method1("get", ("include_nested", true))?.is_truthy()?;
                acts.append((format!("lnk:linknested:{name}"), if nested { "Source only" } else { "Include nested links" }))?;
            }
        }
        if top && !missing {
            acts.append((format!("lnk:linkpack:{name}"), if state.eq("packed")? { "Unpack" } else { "Pack into this file" }))?;
        }
        if overrides != 0 && top {
            acts.append((format!("lnk:linkresettags:{name}"), "Show tags as the source has them"))?;
        }
        if !missing {
            acts.append((format!("lnk:contents:{name}"), "Select contents"))?;
        }
        if placed == 0 {
            acts.append((format!("isolate:{name}"), "Isolate"))?;
            acts.append((format!("lnk:eye:{name}"), if visible { "Hide" } else { "Show" }))?;
        }
        let title = if file_name.is_truthy()? { file_name } else { node.get_item("name")? };
        return (title, [("Linked file", rows), ("Actions", acts)]).into_bound_py_any(py);
    }
    let tags = empty(py);
    let layers = data.call_method1("get", ("node_layers", PyDict::new(py)))?;
    for below in tree.call_method1("subtree", (&key,))?.try_iter()? {
        tags.call_method1("extend", (layers.call_method1("get", (below?, PyTuple::empty(py)))?,))?;
    }
    let tag = node.get_item("tag")?;
    if tag.is_truthy()? {
        tags.append(&tag)?;
    }
    let layer = if tags.is_empty() { none(py) } else { scene.call_method1("layer", (tags.get_item(0)?,))? };
    let visible = layer.is_none() || edits.call_method1("effective_visible", (&scene, &layer))?.is_truthy()?;
    let class = get(&node.get_item("recipe")?, "class")?;
    let what = if kind == "tag" {
        "Drawing only".into_bound_py_any(py)?
    } else if kind == "tagfolder" {
        "Tag folder".into_bound_py_any(py)?
    } else if class.is_truthy()? || !children.is_truthy()? {
        direct(py, super::chrome_tree::kind_of, (&class,))?
    } else {
        "Group of nodes".into_bound_py_any(py)?
    };
    rows.append(("Name", node.get_item("name")?))?;
    rows.append(("Kind", what))?;
    rows.append(("From", format!("{} — read-only here", text(&file_name)?)))?;
    rows.append(("Items", &count))?;
    rows.append(("Visibility", if visible { "Shown" } else { "Hidden" }))?;
    acts.append((format!("lnk:eye:{name}"), if visible { "Hide" } else { "Show" }))?;
    acts.append((format!("isolate:{name}"), "Isolate"))?;
    acts.append((format!("lnk:contents:{name}"), "Select contents"))?;
    let tagged = if tag.is_truthy()? { scene.call_method1("layer", (&tag,))? } else { none(py) };
    let export = [
        ("IFC class", text_or(class, "none (drawing only)")?),
        ("Tag", if tagged.is_truthy()? { text(&tagged.getattr("name")?)? } else { "—".to_owned() }),
    ];
    (node.get_item("name")?, (("Node", rows), ("Actions", acts), ("Export details", export))).into_bound_py_any(py).and_then(|shown| {
        // `[..]`, as the inspector takes its sections.
        let [title, sections] = unpack::<2>(&shown)?;
        (title, list(&sections)?).into_bound_py_any(py)
    })
}

pub(super) fn link_state<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py().import("built_core.links")?.call_method1("link_state", (a.get_item(0)?, a.get_item(1)?.get_item("path")?))
}

fn core<'py>(viewport: &Obj<'py>, command: &str, args: Bound<'py, PyDict>) -> Reply<'py> {
    super::chrome_tree_action::core_command(viewport, command.into_bound_py_any(viewport.py())?.as_any(), Some(&args))
}
fn flash(viewport: &Obj<'_>, message: &Obj<'_>) -> PyResult<()> {
    viewport.call_method1("flash_status", (message.str()?,)).map(drop)
}
/// Whether `error` is one a link verb tells in the status line.
fn link_error(py: Python<'_>, error: &PyErr, key_error: bool) -> PyResult<bool> {
    Ok(error.is_instance_of::<PyValueError>(py)
        || error.is_instance_of::<PyOSError>(py)
        || (key_error && error.is_instance_of::<PyKeyError>(py))
        || error.is_instance(py, &py.import("built_core.links")?.getattr("LinkError")?))
}

/// One link verb; `false` when it is done without the common ending.
fn link_verb<'py>(win: &Obj<'py>, view: &Obj<'py>, group: &Obj<'py>, key: &str, record: &Obj<'py>) -> PyResult<bool> {
    let py = win.py();
    let module = ui(py)?;
    let links = py.import("built_core.links")?;
    let viewport = win.getattr("viewport")?;
    let scene = viewport.getattr("scene")?;
    let path = || record.get_item("path");
    match key {
        "linkfinder" => {
            py.import("built_core.desktop_open")?.call_method1("reveal", (path()?,))?;
            return Ok(false);
        }
        "linkunlink" => {
            core(&viewport, "unlink_placements", fields!(py; "placements" => PyList::new(py, [scene.call_method1("occurrence_path", (group,))?])?))?;
            viewport.call_method0("notify_scene_changed")?;
            view.setattr("_ui_insp_sig", py.None())?;
            view.setattr("_ui_tree_sig", py.None())?;
            return Ok(false);
        }
        "linksubset" => {
            let address = scene.call_method1("occurrence_path", (group,))?.unbind();
            // The form keeps the answer's callback until it closes: it
            // holds the window and the document weakly.
            let weak = py.import("weakref")?.getattr("ref")?;
            let (held_win, held_scene) = (weak.call1((win,))?.unbind(), weak.call1((&scene,))?.unbind());
            let accept = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
                let py = args.py();
                let text = args.get_item(0)?;
                let (win, scene) = (held_win.bind(py).call0()?, held_scene.bind(py).call0()?);
                if text.is_none() || win.is_none() || scene.is_none() || !win.getattr("viewport")?.getattr("scene")?.is(&scene) {
                    return Ok(());
                }
                let viewport = win.getattr("viewport")?;
                let edited = (|| -> PyResult<()> {
                    let result = core(&viewport, "edit_link_subset", fields!(py; "placement" => address.bind(py), "text" => text))?;
                    if !result.is_truthy()? {
                        return Ok(());
                    }
                    if result.get_item("ok")?.is_truthy()? {
                        win.call_method1("_relink_done", ("Linked subset updated",))?;
                    } else {
                        let error = result.get_item("error")?;
                        let reason = if error.is_truthy()? { error } else { result.get_item("rejected")? };
                        if reason.is_truthy()? {
                            flash(&viewport, &reason)?;
                        }
                    }
                    Ok(())
                })();
                match edited {
                    Err(error) if link_error(py, &error, true)? => flash(&viewport, error.value(py)),
                    other => other,
                }
            })?;
            let shown = py.import("json")?.call_method("dumps", (get(record, "select")?,), Some(&fields!(py; "indent" => 2)))?;
            render(py)?.getattr("chrome_forms")?.call_method(
                "ask_text",
                (win, "Edit linked subset", "Full document (null), {\"uids\":[…]} or {\"filter\":{…}}", shown),
                Some(&fields!(py; "done" => accept, "multiline" => true)),
            )?;
            return Ok(false);
        }
        "linkreload" => {
            links.call_method1("reload", (&scene, path()?))?;
        }
        "linkopen" => {
            // A ⛓ tab right of this project (views/wgpu_tabs.py).
            let source = py.import("pathlib")?.getattr("Path")?.call1((path()?,))?;
            py.import("views.wgpu_tabs")?.call_method1("open_link_source", (win, source, group))?;
            return Ok(false);
        }
        "linkpack" => {
            let core_module = module.getattr("built_core")?;
            let packed = !core_module.call_method1("link_is_packed", (&scene, path()?))?.is_truthy()?;
            if !packed {
                links.call_method1("reload", (&scene, path()?))?;
            }
            core_module.call_method1("link_set_packed", (&scene, path()?, packed))?;
        }
        "linknested" => {
            win.call_method1("_set_link_nested", (group, !record.call_method1("get", ("include_nested", true))?.is_truthy()?))?;
        }
        "linkrelink" => {
            let source = py.import("pathlib")?.getattr("Path")?.call1((path()?,))?;
            let filter = py.import("built_core.formats.built")?.getattr("DOCUMENT_FILTER")?;
            let chosen = render(py)?.getattr("shell_files")?.getattr("ShellFileDialog")?.call_method1(
                "getOpenFileName",
                (win, format!("Locate {}", source.getattr("name")?.str()?), source.getattr("parent")?.str()?, filter),
            )?;
            let chosen = chosen.get_item(0)?;
            if !chosen.is_truthy()? {
                return Ok(false);
            }
            let (records, previous) = (empty(py), empty(py));
            for copy in links.call_method1("linked_groups", (&scene, &source))?.try_iter()? {
                let record = links.call_method1("link_of", (copy?,))?;
                previous.append(py.get_type::<PyDict>().call1((&record,))?)?;
                records.append(record)?;
            }
            for record in records.iter() {
                record.call_method1("update", (crate::host_context::core_module(py)?.call_method1("link_record", (&chosen, win.getattr("_current_path")?))?,))?;
            }
            if let Err(error) = links.call_method1("reload", (&scene, &chosen)) {
                if error.is_instance_of::<PyException>(py) {
                    for (record, original) in records.iter().zip(previous.iter()) {
                        record.call_method0("clear")?;
                        record.call_method1("update", (original,))?;
                    }
                }
                return Err(error);
            }
        }
        _ => {}
    }
    win.call_method1("_relink_done", ("Linked files updated",))?;
    Ok(true)
}

/// The Linked files panel's verbs on the selected placement.
fn link_action<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, view, group) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let key: String = a.get_item(3)?.extract()?;
    let record = py.import("built_core.links")?.call_method1("link_of", (&group,))?;
    if !record.is_truthy()? {
        return Ok(none(py));
    }
    match link_verb(&win, &view, &group, &key, &record) {
        Ok(false) => return Ok(none(py)),
        Ok(true) => {}
        Err(error) if link_error(py, &error, false)? => flash(&win.getattr("viewport")?, error.value(py))?,
        Err(error) => return Err(error),
    }
    for cache in ["_ui_rules_cache", "_ui_insp_sig", "_ui_tree_sig"] {
        view.setattr(cache, py.None())?;
    }
    Ok(none(py))
}

/// A complete curve group can be an arbitrary polyline. Radius and
/// resegmentation are available only for a regular circular shape.
fn complete_curve(scene: &Obj<'_>, sel: &Bound<'_, PyList>) -> PyResult<bool> {
    if sel.len() < 2 {
        return Ok(false);
    }
    let first = sel.get_item(0)?;
    let Some(tag) = first.getattr_opt("curve")? else {
        return Ok(false);
    };
    if tag.is_none() {
        return Ok(false);
    }
    let mesh = scene.getattr("mesh")?;
    for e in sel.iter().skip(1) {
        let Some(other) = e.getattr_opt("curve")? else {
            return Ok(false);
        };
        if !tag.eq(other)? {
            return Ok(false);
        }
    }
    Ok(mesh.call_method1("curve_edges", (first,))?.len()? == sel.len())
}

fn regular_curve<'py>(scene: &Obj<'py>, sel: &Bound<'py, PyList>) -> Reply<'py> {
    let py = scene.py();
    if sel.len() > 4096 {
        return Ok(none(py));
    }
    py.import("built_core")?
        .getattr("curve_segments")?
        .call_method1("read_curve", (scene.getattr("mesh")?, sel.get_item(0)?))
}

pub(super) fn curve_of<'py>(a: &Args<'py>) -> Reply<'py> {
    let scene = a.get_item(0)?;
    let sel = list(&a.get_item(1)?)?;
    if complete_curve(&scene, &sel)? {
        regular_curve(&scene, &sel)
    } else {
        Ok(none(a.py()))
    }
}

pub(super) fn status_text<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    for entry in or_dict(get(&a.get_item(0)?, "psets")?)?.call_method0("items")?.try_iter()? {
        let [name, props] = unpack::<2>(&entry?)?;
        if name.call_method1("endswith", ("Common",))?.is_truthy()? {
            let mut value = get(&props, "Status")?;
            if value.is_instance_of::<PyDict>() {
                value = get(&value, "value")?;
            }
            if value.is_truthy()? {
                return value.str()?.call_method0("capitalize");
            }
        }
    }
    "".into_bound_py_any(py)
}

/// Pset_<Class>Common.Status on every selected item that has a record: one
/// undo step.
pub(super) fn set_status<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, groups, value) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let history = py.import("built_core.history")?;
    let commands = empty(py);
    for group in groups.try_iter()? {
        let group = group?;
        let record = or_dict(group.getattr("ifc")?)?;
        let class = get(&record, "class")?;
        if !class.is_truthy()? {
            continue;
        }
        let psets = py.import("copy")?.call_method1("deepcopy", (or_dict(get(&record, "psets")?)?,))?;
        let mut name = None;
        for known in psets.try_iter()? {
            let known = known?;
            if known.call_method1("endswith", ("Common",))?.is_truthy()? {
                name = Some(known);
                break;
            }
        }
        let name = match name {
            Some(name) => name,
            None => py.import("core.ifc_tables")?.call_method1("common_pset", (class,))?,
        };
        psets.call_method1("setdefault", (name, PyDict::new(py)))?.set_item("Status", &value)?;
        commands.append(history.getattr("SetRecordCommand")?.call1((PyList::new(py, [group])?, fields!(py; "psets" => psets)))?)?;
    }
    let count = commands.len();
    if count != 0 {
        let viewport = win.getattr("viewport")?;
        render(py)?.getattr("tree_navigation")?.call_method1("execute", (&viewport, history.getattr("CompoundCommand")?.call1((commands,))?))?;
        let told = format!("Status {} on {count} item{}", value.call_method0("capitalize")?.str()?, if count != 1 { "s" } else { "" });
        viewport.call_method1("flash_status", (told,))?;
    }
    Ok(none(py))
}

/// The tree takes the next pick (or typed name + Enter).
pub(super) fn file_to<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = attr(&attr(&a.get_item(0)?, "_doc")?, "stand_in")?;
    if !view.is_none() {
        let count = a.get_item(1)?;
        let label = format!("{} item{}", count.str()?, if count.ne(1)? { "s" } else { "" });
        view.getattr("renderer")?.call_method1("ui_start_filing", (label,))?;
        view.call_method0("update")?;
    }
    Ok(none(py))
}

fn after_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    if !truthy(&view, "_ui_attached")? {
        return Ok(none(py));
    }
    let module = ui(py)?;
    view.setattr("_ui_frames", view.getattr_opt("_ui_frames")?.unwrap_or(0.into_bound_py_any(py)?).add(1)?)?;
    let renderer = view.getattr("renderer")?;
    let actions = renderer.call_method0("ui_take_actions")?;
    if actions.is_truthy()? {
        let win = module.call_method1("_window", (&view,))?;
        // Never switch tools from inside a paint.
        let (held_win, held_actions, held_view) = (win.unbind(), actions.unbind(), view.clone().unbind());
        let run = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
            let py = args.py();
            ui(py)?.call_method1("_run", (held_win.bind(py), held_actions.bind(py), held_view.bind(py))).map(drop)
        })?;
        module.call_method1("later", (&view, 0, run))?;
    }
    let mut delay = renderer.call_method0("ui_repaint_after_ms")?;
    if own(&view, "_ui_poll_jobs")?.is_some_and(|poll| poll.is_truthy().unwrap_or(false)) {
        delay = if delay.is_none() { 400.into_bound_py_any(py)? } else { py.import("builtins")?.call_method1("min", (delay, 400))? };
    }
    if !delay.is_none() {
        let wait = py.import("builtins")?.call_method1("max", (0, delay.call_method0("__int__")?))?;
        view.getattr("_ui_timer")?.call_method1("start", (wait,))?;
    }
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_inspector", Mode::Function, inspector, &["view", "win"], "", ""),
    ("_linked_inspector", Mode::Function, linked_inspector, &["view", "win", "data", "key"], "", ""),
    ("_link_action", Mode::Function, link_action, &["win", "view", "g", "key"], "", ""),
    ("after_frame", Mode::Function, after_frame, &["view"], "", ""),
];
