// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for base.
use super::*;

fn do_nothing<'py>(a: &Args<'py>) -> Reply<'py> {
    Ok(none(a.py()))
}
fn decline<'py>(a: &Args<'py>) -> Reply<'py> {
    false.into_bound_py_any(a.py())
}
fn empty_hint<'py>(a: &Args<'py>) -> Reply<'py> {
    "".into_bound_py_any(a.py())
}
fn empty_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    Ok(PyList::empty(a.py()).into_any())
}
fn repeat_click<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.call_method1("on_click", (a.get_item(1)?,))?;
    Ok(none(a.py()))
}
fn ctrl_tapped<'py>(a: &Args<'py>) -> Reply<'py> {
    if let Some(tapped) = a
        .get_item(0)?
        .getattr_opt("ctrl_tapped")?
        .filter(|f| f.is_callable())
    {
        tapped.call0()?.is_truthy()?.into_bound_py_any(a.py())
    } else {
        true.into_bound_py_any(a.py())
    }
}
fn refresh_hint<'py>(a: &Args<'py>) -> Reply<'py> {
    if let Some(hint) = a
        .get_item(0)?
        .getattr_opt("refresh_status_hint")?
        .filter(|f| f.is_callable())
    {
        hint.call0()?;
    }
    Ok(none(a.py()))
}

pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();

    let keys = PyDict::new(py);
    let names = PyDict::new(py);
    let qt = module.getattr("Qt")?;
    let int = py.import("builtins")?.getattr("int")?;
    for (key, axis, plane) in [
        ("Key_Right", "x", "YZ"),
        ("Key_Left", "y", "XZ"),
        ("Key_Up", "z", "XY"),
    ] {
        keys.set_item(int.call1((qt.getattr(key)?,))?, axis)?;
        names.set_item(axis, plane)?;
    }
    module.add("PLANE_LOCK_KEYS", keys)?;
    module.add("PLANE_LOCK_NAMES", names)?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("__doc__","The Line tool's axis magnet, for the other drawing tools.\n\n@pacaeiro, issue #52: «The Draw commands in the group of ARCS and\nSHAPES should also use the magnetic Snaps, like LINE. Except\n    Freehand» — and #51 for Text, #50 for Dimension. Same two numbers as\nLine (issue #31): within 3° of an axis from ``start_point`` the point\nlands ON the axis instead of merely lighting the cue, and the axis the\nwork plane cannot hold is found in pixels. Alt still switches it off.\n\nA shape that lives in ONE plane (``PlaneLock``) still gets both, but\nthe screen detector's answer is confined to the axes the plane HOLDS\n(``axis_in_plane`` in the snap engine): it widens the capture to the\nscreen's 9 px for those two axes, never pulling the point off the plane.\n\nOnly a click that is a *direction from the start point* wants this:\nan arc's bulge, a rotated rectangle's height or a dimension's placement\nare measured from somewhere else, and a magnet aimed at the first point\nwould bend them. Those tools say when by overriding :meth:`magnet_on`;\nthe viewport reads the two properties afresh on every hover.\n\nRectangle stays out on purpose: its second click is the opposite\ncorner, and a corner pulled onto the axis through the first is a\nrectangle of zero height — every thin rectangle near an axis would\ncollapse. Its plane lock and the Shift/arrow locks are its magnets.")?;
        attrs.set_item("_MAGNET_DEG", 3.0)?;
        attrs.set_item("_MAGNET_PX", 9.0)?;
        let cls = define(module, "AxisMagnet", &[], &attrs)?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("__doc__","Arrow keys BEFORE the first click lock a planar tool (circle,\npolygon, rectangle, the arcs) to a drawing plane: Right = the plane\nnormal to X (YZ), Left = normal to Y (XZ), Up = normal to Z (XY); the\nsame key again frees it. The classic plane lock (Marco, 2026-09-08:\n«quiero dibujar un círculo en el plano ZX… apretando las teclas de\ndesplazamiento»).\n    Before the first click the key only chooses the plane, and that click\nanchors it at the point; after it the plane stops\nfollowing the cursor and the key holds the shape on the plane it is\ndrawing on — the same key frees it (handing the plane back to the\ncursor), a different one switches it. The lock is spent by the shape\n(or Esc), the classic way. The arrows went back to being the\nviewport's linear axis lock after the first click until Marco asked\nfor them to answer the whole way through («também deve funcionar»),\nexcept where a later click reads that lock —\n``plane_lock_after_the_first_point``.\n\nThe DOWN arrow is the classic magenta reference lock: over an\nedge the plane goes PERPENDICULAR to that edge, over a face PARALLEL\nto it; Down again frees it. It is the native way to start a pipe's\ncircle on an inclined axis line, then Follow Me it (issue #10,\n@pacaeiro: «I draw lines to represent the axis of the tubes … then\nhave to rotate [the circle] to align the normal of the circle's plane\nto the line»).\n\nShift is the Down arrow HELD: for as long as the key is down the plane\nthe cursor is detecting stays where it is, and letting go hands it\nback — nothing is locked afterwards, it is the way to look at the\nplane before committing to it (Marco: «antes de clicar no 1º ponto o\nShift funciona como se clicasse na arrow down temporariamente… marca o\nplano que está a ser considerado, por exemplo uma face num cubo;\ndepois do 1º click é igual»). It is additive: the pointer goes on\nmarking midpoints, endpoints and alignments, and the inference lock\nthe viewport takes off the cursor still happens (``Viewport.\n_capture_shift_lock``). A plane an arrow or Down is already holding is\nnot Shift's to release. See ``mark_plane``.")?;
        attrs.set_item("plane_lock_after_the_first_point", true)?;
        attrs.set_item("OFF_PLANE_TOL", 0.001)?;
        attrs.set_item("PREVIEW_PX", 22)?;
        attrs.set_item(
            "plane_color",
            py.import("builtins")?
                .getattr("staticmethod")?
                .call1((module.getattr("cursor_plane_color")?,))?,
        )?;
        attrs.set_item(
            "world_per_pixel",
            py.import("builtins")?
                .getattr("staticmethod")?
                .call1((module.getattr("cursor_pixel_scale")?,))?,
        )?;
        attrs.set_item(
            "plane_preview_scale",
            py.import("builtins")?
                .getattr("staticmethod")?
                .call1((module.getattr("cursor_pixel_scale")?,))?,
        )?;
        attrs.set_item(
            "hovered_reference",
            py.import("builtins")?
                .getattr("staticmethod")?
                .call1((module.getattr("cursor_plane_reference")?,))?,
        )?;
        let annotations = PyDict::new(py);
        annotations.set_item("plane_lock_after_the_first_point", "bool")?;
        attrs.set_item("__annotations__", annotations)?;
        let cls = define(module, "PlaneLock", &[], &attrs)?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item(
            "__doc__",
            "Bundle of data a tool needs to react to a viewport event.",
        )?;
        let annotations = PyDict::new(py);
        annotations.set_item("viewport", "object")?;
        annotations.set_item("world", "QVector3D")?;
        annotations.set_item("screen", "QPointF")?;
        annotations.set_item("modifiers", "Qt.KeyboardModifiers")?;
        annotations.set_item("snap", "SnapResult")?;
        attrs.set_item("__annotations__", annotations)?;
        let cls = define(module, "ToolContext", &[], &attrs)?;
        module.getattr("dataclass")?.call1((&cls,))?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("name", "Unnamed")?;
        attrs.set_item("icon", py.None())?;
        attrs.set_item("shortcut", py.None())?;
        attrs.set_item("shortcut_alt", py.None())?;
        attrs.set_item("description", py.None())?;
        attrs.set_item("menu_order", 0)?;
        attrs.set_item("uses_snap", true)?;
        attrs.set_item("allow_inferences", true)?;
        attrs.set_item("allowed_snap_kinds", py.None())?;
        attrs.set_item("box_select", false)?;
        attrs.set_item("wireframe_color", py.None())?;
        attrs.set_item("wireframe_depth_tested", false)?;
        attrs.set_item("vcb_label", py.None())?;
        attrs.set_item("snap_kind", py.None())?;
        let annotations = PyDict::new(py);
        annotations.set_item("name", "str")?;
        annotations.set_item("icon", "str | None")?;
        annotations.set_item("shortcut", "str | None")?;
        annotations.set_item("shortcut_alt", "str | None")?;
        annotations.set_item("description", "str | None")?;
        annotations.set_item("menu_order", "int")?;
        annotations.set_item("uses_snap", "bool")?;
        annotations.set_item("allow_inferences", "bool")?;
        annotations.set_item("allowed_snap_kinds", "frozenset | None")?;
        annotations.set_item("box_select", "bool")?;
        annotations.set_item(
            "wireframe_color",
            "tuple[float, float, float, float] | None",
        )?;
        annotations.set_item("wireframe_depth_tested", "bool")?;
        annotations.set_item("vcb_label", "str | None")?;
        annotations.set_item("snap_kind", "str | None")?;
        attrs.set_item("__annotations__", annotations)?;
        let cls = define(module, "Tool", &[module.getattr("ABC")?], &attrs)?;
        install_methods(module,&cls,&[("on_activate",Mode::Method, do_nothing, &["self", "viewport"], "None", "Called when the user selects this tool."),
("on_deactivate",Mode::Method, do_nothing, &["self", "viewport"], "None", "Called when the user switches to another tool."),
("on_click",Mode::Method, do_nothing, &["self", "ctx:ToolContext"], "None", "Left click at ``ctx.world`` (already snapped)."),
("on_hover",Mode::Method, do_nothing, &["self", "ctx:ToolContext"], "None", "Mouse moved to ``ctx.world`` without a button pressed."),
("on_double_click",Mode::Method, repeat_click, &["self", "ctx:ToolContext"], "None", "Left double-click. Qt swallows the second press, so tools that act\non it (Push/Pull repeating its last distance) override this; the\ndefault is to treat it as a plain click so other tools keep their\nclick-click rhythm even when the user clicks fast."),
("on_triple_click",Mode::Method, repeat_click, &["self", "ctx:'ToolContext'"], "None", "Third click in place (select all connected). Defaults to\na plain click so unaware tools keep their rhythm."),
("on_box_select",Mode::Method, do_nothing, &["self", "viewport", "rect", "crossing:bool", "additive:bool=False", "mode:str | None=None"], "None", "Rubber-band box released. ``rect`` is ``(x0, y0, x1, y1)`` in screen\npixels (normalized so x0<=x1, y0<=y1). ``crossing`` is True for a\nright-to-left drag (select anything the box touches) and False for a\nleft-to-right drag (select only what's fully enclosed). ``mode`` is how\nthe catch joins the selection — \"replace\" / \"add\" / \"toggle\" /\n\"remove\", from the keyboard modifiers (see\n``tools.rust_tools.selection_mode``); ``additive`` is the older two-state\nspelling, kept for tools that only know it. Only tools with\n``box_select = True`` receive this."),
("drag_plane",Mode::Method, do_nothing, &["self", "viewport"], "", "``(point, normal)`` of the plane the cursor should be read on\nwhile this tool is mid-operation, or ``None`` to leave the choice\nto the viewport (captured face, start point, ground).\n\nThe viewport turns every pixel into a world point by hitting a\nplane, and hands the tool NOTHING — no hover, no click — when the\nray misses it. A tool whose operation runs along a LINE (Push/Pull\nalong its normal) only ever needs the pixel, and the ground stops\nanswering the moment the cursor crosses the horizon: Rafael pulled\na box up with the camera at ground level, the box froze at the\nhorizon, and the click on its top never arrived (2026-09-16). Such a\ntool answers with a plane that contains its line and faces the\ncamera, which the ray always hits."),
("on_cancel",Mode::Method, do_nothing, &["self", "viewport"], "None", "Esc pressed — abandon any in-progress operation."),
("on_release",Mode::Method, do_nothing, &["self", "viewport"], "None", "Left button released. Tools that act on press-drag-release strokes\n(the Eraser) override this; the default is a no-op, so click-driven\ntools are unaffected."),
("on_key",Mode::Method, decline, &["self", "viewport", "key:int", "modifiers:Qt.KeyboardModifiers"], "bool", "Tool gets first shot at the key. Return True to consume it."),
("on_value",Mode::Method, decline, &["self", "viewport", "value:float"], "bool", "User typed a numeric length and pressed Enter (VCB style).\n\nTools that accept a numeric value (line length, rectangle dimensions,\ncircle radius, ...) override this and return True on success. The\nviewport handles digit buffering and dispatch; tools only need to\nconsume the value."),
("status_clause",Mode::Method, empty_hint, &["self"], "str", "A short clause this tool adds to the status bar, or \"\".\n\nThe classic tool keeps modifiers ON SCREEN the whole time the tool is\nactive — «Ctrl = Líneas guía del ciclo/Puntos guía/Medida» sits\nthere next to the instruction — instead of flashing them once when\nyou press the key. A flash tells you what just happened; this tells\nyou what you can do, and which way it is set right now (Marco,\n2026-09-17)."),
("ctrl_tapped",Mode::Static, ctrl_tapped, &["viewport"], "bool", "Whether the Ctrl being RELEASED was a tap of its own.\n\nA modifier toggled on the PRESS fires on every Ctrl+<key> chord too:\nCtrl+Z reaches the viewport only as a ShortcutOverride, so the mode\nswitched silently while undoing (issue #183, Alejandro Limón). Toggle\non the RELEASE and ask this first — the viewport knows whether some\nother key came in between. Stand-ins without the tracker (tests)\ncount as a tap."),
("refresh_hint",Mode::Static, refresh_hint, &["viewport"], "None", "Redraw the status bar, so ``status_clause`` shows the new mode."),
("rubber_band_lines",Mode::Method, empty_preview, &["self"], "", "Return ``[(a, b), ...]`` line segments to draw as the live preview.\n\n``LineTool`` returns a single segment; ``RectangleTool`` returns four.\nThe viewport renders whatever the active tool returns; tools that\ndon't preview anything default to an empty list."),
("guide_preview_lines",Mode::Method, empty_preview, &["self"], "", "``[(a, b), ...]`` — the guide LINES this tool is about to leave,\ndrawn dashed like a real guide while the cursor moves (#89,\n@pacaeiro). Long segments are fine: the viewport clips them."),
("inference_guide_lines",Mode::Method, empty_preview, &["self"], "", "``[(a, b, kind), ...]`` — reference segments drawn dashed in the\nsnap-kind colour while the tool is inferring (Push/Pull's «level with\nthat point» line from the geometry it locked onto to the moving cap).\nTools that infer nothing default to an empty list."),
("preview_faces",Mode::Method, empty_preview, &["self"], "", "Return ``Face`` objects to render shaded as a live solid preview.\n\nPush/Pull uses this so the extruded box appears filled while you drag,\nthe way users expect the solid forming — not just its wireframe. The\nviewport triangulates and draws them depth-tested every frame; tools\nthat have no solid preview default to an empty list.")])?;
        for name in ["on_activate", "on_deactivate"] {
            cls.getattr(name)?.setattr("__isabstractmethod__", true)?;
        }
        py.import("abc")?
            .call_method1("update_abstractmethods", (&cls,))?;
    }
    register_tool_functions(&module.dict())?;
    Ok(())
}
