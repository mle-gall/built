// SPDX-License-Identifier: GPL-3.0-or-later
//! The chrome's input (`views.wgpu_ui`): keys, the wheel and the pointer
//! offered to egui before the tools; and the module itself, built here.
use super::chrome_frame::{render, truthy, ui};
use super::*;

fn input<'py>(py: Python<'py>, name: &str) -> Reply<'py> {
    py.import("built_core")?.getattr("Input")?.getattr(name)
}
fn is_input(value: &Obj<'_>, name: &str) -> PyResult<bool> {
    value.eq(input(value.py(), name)?)
}
fn sheets<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("built_render.sheet_workspace")
}
fn mods_of<'py>(py: Python<'py>, event: &Obj<'py>) -> Reply<'py> {
    ui(py)?.call_method1("_mods", (event,))
}
/// The chrome's name of the event's key, "" for one it does not take.
pub(super) fn key_name<'py>(py: Python<'py>, event: &Obj<'py>) -> Reply<'py> {
    ui(py)?.getattr("_KEYS")?.call_method1("get", (event.call_method0("key")?, ""))
}
pub(super) fn pointer_button<'py>(button: &Obj<'py>) -> Reply<'py> {
    ui(button.py())?.getattr("_BUTTONS")?.call_method1("get", (button, 1))
}
fn point(event: &Obj<'_>) -> PyResult<(f64, f64)> {
    let position = event.call_method0("position")?;
    let float = |value: Obj<'_>| -> PyResult<f64> { position.py().get_type::<pyo3::types::PyFloat>().call1((value,))?.extract() };
    Ok((float(position.call_method0("x")?)?, float(position.call_method0("y")?)?))
}

pub(super) fn opens_commands<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    let module = ui(py)?;
    let window = view.call_method0("window")?;
    let actions = py.import("builtins")?.call_method1("vars", (window,))?.call_method1("get", ("_registry_actions", PyDict::new(py)))?;
    let action = actions.call_method1("get", ("command_search",))?;
    if action.is_none() {
        return false.into_bound_py_any(py);
    }
    let combo = event.call_method0("key")?.bitor(event.call_method0("modifiers")?.getattr("value")?)?;
    for key in module.call_method1("action_keys", (action,))?.try_iter()? {
        let parsed = module.call_method1("shortcut_text", (key?,))?;
        if parsed.is_truthy()? && parsed.get_item(2)?.eq(&combo)? {
            return true.into_bound_py_any(py);
        }
    }
    false.into_bound_py_any(py)
}

pub(super) fn problem_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    // The keys step through the export checks while the dock shows them.
    let export = super::chrome_tree::wants_readiness(&view)?;
    if !export || !event.call_method0("modifiers")?.bitand(input(py, "ControlModifier")?)?.is_truthy()? {
        return 0.into_bound_py_any(py);
    }
    let key = event.call_method0("key")?;
    let step = if is_input(&key, "Key_BracketRight")? {
        1
    } else if is_input(&key, "Key_BracketLeft")? {
        -1
    } else {
        0
    };
    step.into_bound_py_any(py)
}

pub(super) fn reveal_shortcut<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let event = a.get_item(0)?;
    (is_input(&event.call_method0("key")?, "Key_J")? && mods_of(py, &event)?.eq(12)?).into_bound_py_any(py)
}

/// ShortcutOverride: does the chrome own this key (a focused field, or
/// Escape while its popup is open)? Then no window shortcut fires.
pub(super) fn claims_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    if !truthy(&view, "_ui_attached")? {
        return false.into_bound_py_any(py);
    }
    let opens = || direct(py, opens_commands, (&view, &event));
    let large = render(py)?.getattr("chrome_sheet")?.call_method1("claims_key", (&view, &event))?;
    if !large.is_none() {
        // The large sheet owns every key but the window's ⌘ shortcuts.
        return if large.is_truthy()? { Ok(large) } else { opens() };
    }
    if is_input(&event.call_method0("key")?, "Key_Escape")? {
        return true.into_bound_py_any(py); // Reserve it for the chrome and viewport Escape router.
    }
    if direct(py, reveal_shortcut, (&event,))?.is_truthy()?
        || direct(py, problem_key, (&view, &event))?.is_truthy()?
        || opens()?.is_truthy()?
        || view.getattr("renderer")?.call_method1("ui_claims_key", (key_name(py, &event)?, mods_of(py, &event)?))?.is_truthy()?
    {
        return true.into_bound_py_any(py);
    }
    sheets(py)?.call_method1("claims_key", (&view, &event))
}

/// Offer a key to the chrome first. True: it was the chrome's.
fn key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    let pressed = a.get_item(2)?.is_truthy()?;
    let (yes, no) = (true.into_bound_py_any(py)?, false.into_bound_py_any(py)?);
    if !truthy(&view, "_ui_attached")? {
        return Ok(no);
    }
    let module = ui(py)?;
    let renderer = view.getattr("renderer")?;
    if direct(py, opens_commands, (&view, &event))?.is_truthy()? {
        if pressed {
            renderer.call_method0("ui_open_commands")?;
            view.call_method0("update")?;
        }
        return Ok(yes);
    }
    if render(py)?.getattr("chrome_sheet")?.call_method1("claims_key", (&view, &event))?.is(&no) {
        return Ok(no);
    }
    let on_sheets = sheets(py)?.call_method1("shown", (&view,))?.is_truthy()?;
    if !on_sheets && direct(py, reveal_shortcut, (&event,))?.is_truthy()? {
        if pressed {
            let separator: String = module.getattr("SEP")?.extract()?;
            module.call_method1("_tree_action", (view.call_method0("window")?, &view, format!("revealselection{separator}")))?;
        }
        return Ok(yes);
    }
    let step = if on_sheets { 0.into_bound_py_any(py)? } else { direct(py, problem_key, (&view, &event))? };
    if step.is_truthy()? {
        if pressed {
            renderer.call_method1("ui_step_issue", (step,))?;
            view.call_method0("update")?;
        }
        return Ok(yes);
    }
    let name = key_name(py, &event)?;
    if !renderer.call_method1("ui_claims_key", (&name, mods_of(py, &event)?))?.is_truthy()? {
        // The Sheets workspace: the page's keys go to the composer.
        return if on_sheets { sheets(py)?.call_method1("key", (&view, &event, pressed)) } else { Ok(no) };
    }
    renderer.call_method1("ui_key", (&name, pressed, mods_of(py, &event)?))?;
    let text = event.call_method0("text")?;
    if pressed && text.is_truthy()? && text.call_method0("isprintable")?.is_truthy()? {
        renderer.call_method1("ui_text", (text,))?;
    }
    view.call_method0("update")?;
    Ok(yes)
}

pub(super) fn mods<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let held = a.get_item(0)?.call_method0("modifiers")?;
    let mut bits = 0;
    for (name, bit) in [("AltModifier", 1), ("MetaModifier", 2), ("ShiftModifier", 4), ("ControlModifier", 8)] {
        if held.bitand(input(py, name)?)?.is_truthy()? {
            bits |= bit;
        }
    }
    bits.into_bound_py_any(py)
}

fn wheel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    if !truthy(&view, "_ui_attached")? {
        return false.into_bound_py_any(py);
    }
    let window = ui(py)?.call_method1("_window", (&view,))?;
    if render(py)?.getattr("chrome_dialogs")?.call_method1("modal", (window,))?.is_truthy()? {
        return true.into_bound_py_any(py);
    }
    let (x, y) = point(&event)?;
    let pixels = event.call_method0("pixelDelta")?;
    let amount = |delta: &Obj<'py>| -> PyResult<(f64, f64)> { Ok((delta.call_method0("x")?.extract()?, delta.call_method0("y")?.extract()?)) };
    let (dx, dy) = if pixels.call_method0("isNull")?.is_truthy()? {
        let (ax, ay) = amount(&event.call_method0("angleDelta")?)?;
        (ax / 8.0 * 3., ay / 8.0 * 3.)
    } else {
        amount(&pixels)?
    };
    if view.getattr("renderer")?.call_method1("ui_wheel", (x, y, dx, dy))?.is_truthy()? {
        view.call_method0("update")?;
        return true.into_bound_py_any(py);
    }
    sheets(py)?.call_method1("wheel", (&view, &event))
}

/// Exclusive work holds the window: only its chip (Cancel) and the window's
/// own drag take the pointer; the model and the rest of the chrome see
/// nothing until it ends.
pub(super) fn held<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, kind, event) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let dialogs = render(py)?.getattr("chrome_dialogs")?;
    let win = ui(py)?.call_method1("_window", (&view,))?;
    let free = win.is_none()
        || !dialogs.call_method1("exclusive", (&win,))?.is_truthy()?
        || !dialogs.call_method1("current_sheet", (&win,))?.is_none()
        || event.is_none();
    if free {
        return false.into_bound_py_any(py);
    }
    let renderer = view.getattr("renderer")?;
    let (px, py_) = point(&event)?;
    let rect = renderer.call_method1("ui_area_rect", ("jobs",))?;
    if !rect.is_none() {
        let (x, y, w, h): (f64, f64, f64, f64) = rect.extract()?;
        if x <= px && px <= x + w && y <= py_ && py_ <= y + h {
            return false.into_bound_py_any(py); // the chip's own: Cancel
        }
    }
    if kind.eq("press")? && is_input(&event.call_method0("button")?, "LeftButton")? && renderer.call_method1("ui_top_empty", (px, py_))?.is_truthy()? {
        return false.into_bound_py_any(py); // the window still moves
    }
    renderer.call_method1("ui_pointer", ("leave", -1.0, -1.0, 0, 0))?;
    true.into_bound_py_any(py)
}

/// Offer a pointer event to the chrome first. True: it was the chrome's
/// (or, in the Sheets workspace, the page's).
fn pointer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, kind, event) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    if !truthy(&view, "_ui_attached")? {
        return false.into_bound_py_any(py);
    }
    let pages = sheets(py)?;
    if direct(py, held, (&view, &kind, &event))?.is_truthy()? {
        return true.into_bound_py_any(py);
    }
    let renderer = view.getattr("renderer")?;
    let taken = if event.is_none() {
        renderer.call_method1("ui_pointer", (&kind, -1.0, -1.0, 0, 0))?
    } else {
        let (x, y) = point(&event)?;
        let button = event.call_method0("button")?;
        if kind.eq("press")? && is_input(&button, "LeftButton")? && renderer.call_method1("ui_top_empty", (x, y))?.is_truthy()? {
            // The empty top strip is the window's title bar: drag moves it.
            view.call_method0("window")?.call_method0("start_system_move")?;
            return true.into_bound_py_any(py);
        }
        renderer.call_method1("ui_pointer", (&kind, x, y, pointer_button(&button)?, mods_of(py, &event)?))?
    };
    let over = taken.is_truthy()?;
    let was = truthy(&view, "_ui_was_over")?;
    let moving = kind.eq("move")?;
    if over || !moving || was {
        view.call_method0("update")?; // hover highlight in, or out
    }
    if moving || kind.eq("leave")? {
        let shape = if over && renderer.hasattr("ui_cursor_shape")? { renderer.call_method0("ui_cursor_shape")? } else { "".into_bound_py_any(py)? };
        if shape.eq("resize_h")? {
            view.call_method1("setCursor", (input(py, "SizeHorCursor")?,))?;
        } else if over && (!was || truthy(&view, "_ui_cursor_shape")?) {
            view.call_method1("setCursor", (input(py, "ArrowCursor")?,))?;
        } else if was && !over && !pages.call_method1("shown", (&view,))?.is_truthy()? {
            view.getattr("tools")?.call_method0("_apply_tool_cursor")?;
        }
        view.setattr("_ui_was_over", &taken)?;
        view.setattr("_ui_cursor_shape", shape)?;
    }
    if pages.call_method1("shown", (&view,))?.is_truthy()? {
        // The Sheets workspace: what the chrome leaves is the page's.
        if over {
            pages.call_method1("chrome_took", (&view,))?;
            return true.into_bound_py_any(py);
        }
        return pages.call_method1("pointer", (&view, &kind, &event));
    }
    Ok(taken)
}

#[rustfmt::skip]
const FUNCTIONS: &[Method] = &[
    ("key", Mode::Function, key, &["view", "event", "pressed"], "", ""),
    ("_mods", Mode::Function, mods, &["event"], "", ""),
    ("wheel", Mode::Function, wheel, &["view", "event"], "", ""),
    ("pointer", Mode::Function, pointer, &["view", "kind", "event=None"], "", ""),
];

/// Plain-language kinds for the IFC classes a node promises (provisional:
/// the kinds catalogue in docs/project-structure.md replaces this).
const KINDS: &[(&str, &str)] = &[
    ("IfcSlab", "Floor / slab"), ("IfcWall", "Wall"), ("IfcWallStandardCase", "Wall"), ("IfcBeam", "Beam"), ("IfcColumn", "Column"),
    ("IfcMember", "Member"), ("IfcPlate", "Plate"), ("IfcRailing", "Railing"), ("IfcStair", "Stair"), ("IfcStairFlight", "Stair flight"),
    ("IfcRamp", "Ramp"), ("IfcRoof", "Roof"), ("IfcDoor", "Door"), ("IfcWindow", "Window"), ("IfcCovering", "Covering"),
    ("IfcFooting", "Footing"), ("IfcPile", "Pile"), ("IfcCurtainWall", "Curtain wall"), ("IfcFurnishingElement", "Furniture"),
    ("IfcFurniture", "Furniture"), ("IfcSpace", "Space"), ("IfcBuildingElementProxy", "Generic object"),
    ("IfcDistributionElement", "Services"), ("IfcFlowSegment", "Pipe / duct"), ("IfcFlowFitting", "Fitting"),
    ("IfcFlowTerminal", "Terminal"), ("IfcMechanicalFastener", "Fastener"), ("IfcDiscreteAccessory", "Accessory"),
    ("IfcBuildingStorey", "Level"), ("IfcBuilding", "Building"), ("IfcSite", "Site"),
];
const TIPS: &[(&str, &str)] = &[
    ("Reference", "Reference — a linked file. Its contents are read from {path} and are never edited in this document: open the source to change them. Eye and lock here are this document's own overrides of the file's tags; the file itself is not touched."),
    ("Drawing only", "Drawing only — this node has no IFC class: its items are drawn, measured and printed, but not exported as building elements. Give it a kind to export them."),
    ("Linked files", "The files this document references. Each one shows its own structure, read-only."),
];
/// A file row's state, in words (its dot's tooltip and the inspector).
const FILE_STATES: &[(&str, &str)] = &[
    ("resolved", "Up to date"), ("modified", "Changed on disk — reload to see the changes"), ("missing", "Not found — relink it"),
    ("packed", "Packed in this file"),
];
const LINK_STATES: &[(&str, &str)] = &[("resolved", "Up to date"), ("modified", "Changed on disk"), ("missing", "Missing"), ("packed", "Packed in this file")];
const UNIT_TEXT: &[(&str, &str)] = &[("m2", "m²"), ("m3", "m³"), ("m", "m"), ("und", "u"), ("kg", "kg")];
const KEYS: &[(&str, &str)] = &[
    ("Key_PageUp", "PageUp"), ("Key_PageDown", "PageDown"), ("Key_Escape", "Escape"), ("Key_Return", "Enter"), ("Key_Enter", "Enter"),
    ("Key_Backspace", "Backspace"), ("Key_Delete", "Delete"), ("Key_Tab", "Tab"), ("Key_Backtab", "Tab"), ("Key_Space", "Space"),
    ("Key_F2", "F2"), ("Key_Up", "ArrowUp"), ("Key_Down", "ArrowDown"), ("Key_Left", "ArrowLeft"), ("Key_Right", "ArrowRight"),
    ("Key_Home", "Home"), ("Key_End", "End"), ("Key_A", "A"), ("Key_C", "C"), ("Key_V", "V"), ("Key_X", "X"), ("Key_Y", "Y"),
    ("Key_Z", "Z"), ("Key_F", "F"),
];

fn table<'py>(py: Python<'py>, rows: &[(&str, &str)]) -> PyResult<Bound<'py, PyDict>> {
    let table = PyDict::new(py);
    for (key, value) in rows {
        table.set_item(key, value)?;
    }
    Ok(table)
}

/// Build `views.wgpu_ui`: its tables and the chrome's functions.
pub(super) fn install_module(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add(
        "__doc__",
        "The viewport chrome drawn by egui inside the Metal renderer, in the same frame\nas the model.\n\nThe Metal view draws the tool palette, command cluster and status line.\nBoth hosts bind the Rust view registry to their document controller.\n",
    )?;
    module.add("built_core", py.import("built_core")?)?;
    module.add("os", py.import("os")?)?;
    modules::import_names(module, "pathlib", &[("Path", "Path")])?;
    modules::import_names(module, "built_core", &[("Input", "Qt")])?;
    modules::import_names(module, "built_core.timers", &[("later", "later")])?;
    modules::import_names(module, "built_render", &[("shortcut_text", "shortcut_text"), ("rail_icon", "_icon_rgba")])?;
    modules::import_names(module, "built_render.keyboard_bindings", &[("action_keys", "action_keys")])?;
    modules::import_names(module, "plugins._widget_language", &[("FONT", "FONT")])?;
    // Drawing and Export have room for previews and a readiness checklist,
    // and reserve room for their columns.
    // Window buttons' room at the top left (mac_glass: 14 + three buttons).
    module.add("LIGHTS_ROOM", crate::ui::tokens::LIGHTS_ROOM)?;
    // The docked top bar (ui.rs TOP_H): its empty parts move the window.
    module.add("TOP_H", 40.0)?;
    module.add("_CTRL", py.None())?;
    let kinds = PyDict::new(py);
    kinds.set_item(py.None(), "Drawing only")?;
    kinds.update(table(py, KINDS)?.as_mapping())?;
    module.add("KINDS", kinds)?;
    module.add("SEP", "\x1f")?;
    // Item rows shown per node, definition or item before a "Show more"
    // row; each click on it shows this many more (ui/tree-links: the old
    // cap had no way past it but the filter).
    module.add("ROW_CAP", 200)?;
    // Between a row's kind and its tooltip (ui.rs splits the kind on it).
    module.add("TIP", "\x1e")?;
    module.add("TIPS", table(py, TIPS)?)?;
    module.add("FILE_STATES", table(py, FILE_STATES)?)?;
    module.add("UNIT_TEXT", table(py, UNIT_TEXT)?)?;
    module.add("ED", "\x1d")?;
    module.add("LINK_STATES", table(py, LINK_STATES)?)?;
    // The IFC Status values offered as one click (the rest stay in the BIM panel).
    module.add("STATUS_CHOICES", PyList::new(py, ["EXISTING", "NEW", "DEMOLISH", "TEMPORARY"])?)?;
    let keys = PyDict::new(py);
    for (key, name) in KEYS {
        keys.set_item(input(py, key)?, name)?;
    }
    module.add("_KEYS", keys)?;
    let buttons = PyDict::new(py);
    for (button, number) in [("LeftButton", 1), ("RightButton", 2), ("MiddleButton", 3)] {
        buttons.set_item(input(py, button)?, number)?;
    }
    module.add("_BUTTONS", buttons)?;
    let namespace = module.dict();
    for functions in [chrome_frame::FUNCTIONS, chrome_tree::FUNCTIONS, chrome_inspector::FUNCTIONS, chrome_actions::FUNCTIONS, FUNCTIONS] {
        install(namespace.as_any(), &namespace, functions)?;
    }
    chrome_tree_action::register(&namespace)
}
