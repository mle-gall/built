"""UI spike (branch ``ui/egui-spike``): the viewport chrome drawn by egui
inside the Metal renderer, in the same frame as the model.

Off unless ``BUILT_UI=1``. Then the Metal view draws a floating tool
palette and a status line itself, and the Qt tool bars are hidden. Qt
still owns the window, the docks and every tool: the palette only shows
what the window says is active and asks it to switch.
"""
from __future__ import annotations

import os

from PySide6.QtCore import Qt, QTimer, QSize
from PySide6.QtGui import QImage, QKeySequence

ENABLED = os.environ.get("BUILT_UI") == "1"

# The tool bars' groups (MainWindow._build_toolbar), as palette sections.
GROUPS = [
    ["select", "eraser", "paint", "tape"],
    ["line", "freehand", "rectangle", "rotated_rect", "circle", "polygon",
     "arc", "arc3", "center_arc", "pie"],
    ["pushpull", "move", "rotate", "scale", "flip", "followme", "offset", "fillet"],
    ["protractor", "dimension", "text", "section"],
]
# Navigation (Maxence 2026-11-22: "where are orbit, look around and all the
# nav tools?"): every one visible in the palette, a group of its own.
# They act on the camera, not the model: a top row of their own (his
# follow-up), between Save and the view toggles.
NAV = [["orbit", "pan", "zoom", "zoom_window", "zoom_extents", "zoom_selection"],
       ["look_around", "walk", "position_camera"]]
FONT = "/System/Library/Fonts/SFNS.ttf"
_ICON_PX = 48


def _icon_rgba(key: str) -> tuple[int, int, bytes]:
    """The tool's own icon as white with its alpha, so egui can tint it."""
    from views.icons import tool_icon
    icon = tool_icon(key)
    if icon.isNull():
        return 0, 0, b""
    image = icon.pixmap(QSize(_ICON_PX, _ICON_PX)).toImage().convertToFormat(
        QImage.Format_RGBA8888)
    w, h = image.width(), image.height()
    data = bytearray(bytes(image.constBits())[: w * h * 4])
    data[0::4] = data[1::4] = data[2::4] = b"\xff" * (w * h)
    return w, h, bytes(data)


def _window(view):
    win = view.window()
    return win if hasattr(win, "_tool_actions") else None


def attach(view) -> None:
    """Called once the view has its renderer."""
    renderer = view.renderer
    if not ENABLED or getattr(view, "_ui_attached", False) or not hasattr(renderer, "ui_enable"):
        return
    win = _window(view)
    if win is None:
        return
    view._ui_timer = QTimer(view, singleShot=True)
    view._ui_timer.timeout.connect(view.update)
    renderer.ui_enable(FONT if os.path.exists(FONT) else None)
    view._ui_tools_sent = False
    view._ui_attached = True
    if view.source_viewport is not None:
        view.source_viewport._chrome_owns_labels = True


def _send_tools(view, win) -> None:
    """Once the window has built its tool actions (after the first view)."""
    tools = getattr(win, "_tools", {})
    groups = []
    for keys in GROUPS:
        row = []
        for key in keys:
            action = win._tool_actions.get(key)
            if key not in tools or action is None:
                continue
            shortcut = action.shortcut().toString(QKeySequence.NativeText)
            w, h, rgba = _icon_rgba(key)
            row.append((key, action.text(), shortcut, w, h, rgba))
        if row:
            groups.append(row)
    view._ui_palette_runs = {}
    view.renderer.ui_set_tools(groups)
    # Full bleed: the palette replaces every tool bar.
    from PySide6.QtWidgets import QToolBar
    for bar in win.findChildren(QToolBar):
        bar.hide()
    # Full bleed: the right column stands in for the docks; they stay
    # reachable from the Window menu until it covers all they do.
    from PySide6.QtWidgets import QDockWidget
    for dock in win.findChildren(QDockWidget):
        # Only the side panels; never a dock that holds the view itself.
        if dock.isAncestorOf(view) or win.dockWidgetArea(dock) not in (
                Qt.RightDockWidgetArea, Qt.LeftDockWidgetArea):
            continue
        dock.hide()
    # The hint and the measurements now live in the chrome: their Qt
    # copies keep their state but take no room.
    status = win.statusBar()
    for w in (getattr(status, "_msg", None), getattr(win, "_vcb_name", None),
              getattr(win, "_vcb_value", None)):
        if w is not None:
            w.setMaximumWidth(0)
    icons = []
    for key in [*(k for row in NAV for k in row), "save", "section_cuts", "view_iso", "view_top", "view_front",
                "view_right", "view_left", "view_back", "view_bottom"]:
        w, h, rgba = _icon_rgba(key)
        icons.append((key, w, h, rgba))
    view.renderer.ui_set_icons(icons)
    view._ui_tools_sent = True


def before_frame(view) -> None:
    """Mirror the window's state into the chrome (cheap: a few dict reads)."""
    if not getattr(view, "_ui_attached", False):
        return
    win = _window(view)
    if win is None:
        return
    if not view._ui_tools_sent and win._tool_actions:
        _send_tools(view, win)
    active = next((k for k, a in getattr(win, "_nav_actions", {}).items() if a.isChecked()), "") \
        or next((k for k, a in win._tool_actions.items() if a.isChecked()), "")
    view.renderer.ui_set_active(active)
    bar = win.statusBar()
    text = (bar.currentMessage() or getattr(bar, "_base", "") or "") if bar is not None else ""
    name, _, hint = text.partition(" — ")
    view.renderer.ui_set_status(name, hint)
    _cursor_value(view, win)
    _selection(view, win, active)
    _cluster(view, win)
    _tree(view, win)
    _inspector(view, win)
    opts = [(i, label, k, state) for i, label, k, state, _ in _options(view)]
    if opts != getattr(view, "_ui_opts", None):
        view._ui_opts = opts
        view.renderer.ui_set_options(opts)


_CTRL = None


def _ctrl() -> str:
    """The native name of the key the tools call Ctrl (⌘ on the Mac)."""
    global _CTRL
    if _CTRL is None:
        _CTRL = QKeySequence(Qt.CTRL | Qt.Key_A).toString(QKeySequence.NativeText)[:-1].strip("+") or "Ctrl"
    return _CTRL


def _options(view):
    """(id, label, key, state, toggle) for the active tool's strip. Each
    toggle does exactly what tapping the key does; locks are read-only."""
    tool = getattr(view.source_viewport, "active_tool", None)
    host = view.tools  # what the tools are handed as their viewport here
    name = type(tool).__name__ if tool is not None else ""
    out = []
    if name == "MoveTool" and hasattr(tool, "_toggle_copy"):
        out.append(("copy", "Copy", _ctrl(), int(bool(tool._copy)),
                    lambda: tool._toggle_copy(host)))
    elif name == "PushPullTool" and hasattr(tool, "_keep_mode"):
        def keep():
            tool._keep_mode = not tool._keep_mode
            tool._keep_base = tool._keep_mode
            tool.refresh_hint(host)
            host.update()
        out.append(("keep", "Keep base", _ctrl(), int(bool(tool._keep_mode)), keep))
    elif name == "RectangleTool" and hasattr(tool, "_set_from_center"):
        out.append(("centre", "From centre", _ctrl(), int(bool(tool._from_center)),
                    lambda: tool._set_from_center(host, not tool._from_center)))
    vp = view.source_viewport
    lock = getattr(vp, "axis_lock", None) if vp is not None else None
    if lock:
        out.append(("axis", f"{lock.upper()} axis locked", "", 2, None))
    if vp is not None and getattr(vp, "_previous_views", None):
        out.append(("showall", "Isolated · Show all", "", 1, lambda: _show_all(vp)))
    mode = getattr(vp, "linear_inference_mode", "all") if vp is not None else "all"
    if mode == "off":
        out.append(("inf", "Inferences off", "", 2, None))
    elif mode == "parallel_perp":
        out.append(("inf", "Parallel / perpendicular only", "", 2, None))
    return out


def _shortcut(action) -> str:
    return action.shortcut().toString(QKeySequence.NativeText)


def _cluster(view, win) -> None:
    """Top-right: Undo, Redo, Save · X-ray, Section cut · View ▾ (council
    2026-11-22). Everything is the window's own QAction, so shortcuts,
    enabled and checked states stay the window's."""
    from PySide6.QtWidgets import QMenu
    actions: list = []

    def item(action, label=None, icon="", checked=None):
        if action is None:
            return None
        actions.append(action)
        state = (-1 if not action.isCheckable() else int(action.isChecked())) if checked is None else int(checked)
        text = (label or action.text()).replace("&", "")
        key = _shortcut(action)
        return (str(len(actions) - 1), text, key, icon, state, action.isEnabled())

    def tipped(it, name):
        # Cluster buttons say what they are in the tooltip.
        if it is None:
            return None
        return (it[0], it[1], f"{name}    {it[2]}".strip(), it[3], it[4], it[5])

    style = win.active_style() if hasattr(win, "active_style") else None
    xray = style is not None and getattr(style, "face_mode", None) == "xray"
    # No Undo/Redo buttons: ⌘Z / ⇧⌘Z (Maxence 2026-11-22).
    doc = [tipped(item(getattr(win, "_act_save_tb", None), icon="save"), "Save")]
    toggles = [tipped(item(getattr(win, "_act_xray_toggle", None), label="X-ray", checked=xray), "X-ray"),
               tipped(item(getattr(win, "_act_show_scuts", None), label="Cut", icon="section_cuts"), "Section cuts")]
    nav = []
    for row in NAV:
        items = []
        for key in row:
            action = (getattr(win, "_nav_actions", {}).get(key) or getattr(win, "_act_" + key, None)
                      or win._tool_actions.get(key))
            it = item(action, icon=key)
            if it is not None:
                items.append(tipped(it, it[1]))
        nav.append(items)
    cluster = [[i for i in doc if i], *[r for r in nav if r], [i for i in toggles if i]]
    menu = []
    views = [item(a, icon=f"view_{k}") for k, a in getattr(win, "_view_actions", {}).items()]
    menu.append(("Views", [i for i in views if i]))
    menu.append(("Projection", [item(a) for a in getattr(win, "_projection_actions", {}).values()]))
    if not hasattr(view, "_ui_style_menu"):
        view._ui_style_menu = next((m for m in win.findChildren(QMenu)
                                    if m.title().replace("&", "") == "Style"), None)
    style_menu = view._ui_style_menu
    if style_menu is not None:
        rows = [item(a) for a in style_menu.actions()
                if not a.isSeparator() and a.text() and a is not getattr(win, "_act_xray_toggle", None)]
        menu.append(("Style", rows))
    menu.append(("Sections", [i for i in (item(getattr(win, "_act_show_splanes", None)),
                                           item(getattr(win, "_act_section_fill", None))) if i]))
    menu = [(t, rows) for t, rows in menu if rows]
    view._ui_actions = actions
    sig = (tuple(map(tuple, cluster)), tuple((t, tuple(r)) for t, r in menu))
    if sig != getattr(view, "_ui_cluster_sig", None):
        view._ui_cluster_sig = sig
        view.renderer.ui_set_cluster(cluster, menu)


def _cursor_value(view, win) -> None:
    """The measurements box, mirrored: fixed at the bottom right, echoed
    at the cursor while typing (council 2026-11-22). The Qt labels stay
    "visible" for the window's own logic but take no room."""
    label = getattr(win, "_vcb_name", None)
    value = getattr(win, "_vcb_value", None)
    if label is None or value is None or label.isHidden():
        view.renderer.ui_set_cursor_value("", "", False, 0.0, 0.0)
        return
    buffer = str(getattr(win, "_vcb_buffer", "") or "")
    pos = getattr(view, "_cursor_pos", None)
    x, y = (float(pos.x()), float(pos.y())) if pos is not None else (-1.0, -1.0)
    view.renderer.ui_set_cursor_value(label.text(), buffer or value.text(), bool(buffer), x, y)


def _summary(sel) -> str:
    from core.group import Group
    groups = sum(1 for e in sel if isinstance(e, Group))
    rest = len(sel) - groups
    parts = []
    if groups:
        parts.append(f"{groups} group" + ("s" if groups != 1 else ""))
    if rest:
        parts.append(f"{rest} item" + ("s" if rest != 1 else ""))
    return " · ".join(parts)


def _selection(view, win, active) -> None:
    """The selection bar: with Select active, something selected, no drag."""
    scene = getattr(view, "scene", None)
    sel = list(getattr(scene, "selection", ()) or ())
    dragging = getattr(view, "last", None) is not None
    if active != "select" or not sel or dragging:
        if getattr(view, "_ui_sel_sig", None) is not None:
            view.renderer.ui_set_selection("", [])
            view._ui_sel_sig = None
        return
    sig = (id(scene), len(sel), id(sel[0]), id(sel[-1]))
    if sig == getattr(view, "_ui_sel_sig", None):
        return
    view._ui_sel_sig = sig
    view.renderer.ui_set_selection(_summary(sel), [(k, label, key, run is not None) for k, label, key, run in _commands(win, sel)])


def _commands(win, sel):
    """(id, label, shortcut, run) for the selection bar, in fixed places
    (council 2026-11-22): File to…, Isolate, Hide, Inspect, then Edit for
    a single group. ``run`` None: shown disabled."""
    from core.group import Group
    groups = [e for e in sel if isinstance(e, Group)]
    out = [
        ("file", "File to…", "⌘⇧F", (lambda: _file_to(win, len(groups))) if groups else None),
        ("isolate", "Isolate", "", lambda: _isolate(win, [getattr(e, "uid", None) for e in sel if getattr(e, "uid", None)])),
        ("hide", "Hide", "", win._on_hide),
        ("inspect", "Properties…", "", lambda: _dock(win, ("Properties", "Entity"))),
    ]
    if len(groups) == 1 and len(sel) == 1:
        one = groups[0]
        out.append(("edit", "Edit", "double-click", lambda: win.viewport.begin_group_edit(one)))
    return out


def _show_all(vp) -> None:
    from core.agent_facets import previous_display, set_display
    if getattr(vp, "_previous_views", []):
        previous_display(vp)
    else:
        set_display(vp, remember=False)


def _isolate(win, uids) -> None:
    from core.agent_facets import set_display
    if uids:
        set_display(win.viewport, {"facet": "uid", "in": list(uids)}, isolate=True, exact=False)


# ---- Right column: project tree and inspector -------------------------------

#: Plain-language kinds for the IFC classes a node promises (provisional:
#: the kinds catalogue in docs/project-structure.md replaces this).
KINDS = {
    None: "Drawing only", "IfcSlab": "Floor / slab", "IfcWall": "Wall", "IfcWallStandardCase": "Wall",
    "IfcBeam": "Beam", "IfcColumn": "Column", "IfcMember": "Member", "IfcPlate": "Plate",
    "IfcRailing": "Railing", "IfcStair": "Stair", "IfcStairFlight": "Stair flight", "IfcRamp": "Ramp",
    "IfcRoof": "Roof", "IfcDoor": "Door", "IfcWindow": "Window", "IfcCovering": "Covering",
    "IfcFooting": "Footing", "IfcPile": "Pile", "IfcCurtainWall": "Curtain wall",
    "IfcFurnishingElement": "Furniture", "IfcFurniture": "Furniture", "IfcSpace": "Space",
    "IfcBuildingElementProxy": "Generic object", "IfcDistributionElement": "Services",
    "IfcFlowSegment": "Pipe / duct", "IfcFlowFitting": "Fitting", "IfcFlowTerminal": "Terminal",
    "IfcMechanicalFastener": "Fastener", "IfcDiscreteAccessory": "Accessory",
    "IfcBuildingStorey": "Level", "IfcBuilding": "Building", "IfcSite": "Site",
}


def kind_of(ifc_class) -> str:
    if ifc_class in KINDS:
        return KINDS[ifc_class]
    return str(ifc_class).removeprefix("Ifc") if ifc_class else "Drawing only"


def _tree(view, win) -> None:
    """Send the Project Structure tree when the model changed."""
    scene = getattr(win.viewport, "scene", None)
    if scene is None:
        return
    sig = (id(scene), scene.content_version)
    if sig == getattr(view, "_ui_tree_sig", None):
        return
    view._ui_tree_sig = sig
    from collections import Counter, defaultdict
    from core import bim_registry, links, structure
    registry = bim_registry.get(scene)
    nodes = structure._nodes(registry)
    recipes = {k: structure.resolve(registry, k) for k in nodes}
    children = structure.tree(registry)
    direct, counts, problems, unfiled = defaultdict(list), Counter(), Counter(), []
    for g, source, root, key in structure._memberships(scene, registry):
        review = structure._is_unsorted(g, source, root, key, recipes)
        if review:
            unfiled.append(g)
        if key in nodes:
            direct[key].append(g)
            counts[key] += 1
            expects = recipes[key].get("class") is not None
            problems[key] += int(expects and not structure._has_status(g.ifc or {})) + int(review)
    order, depth, stack = [], {}, [(k, 0) for k in reversed(children[None])]
    while stack:
        key, d = stack.pop()
        order.append(key)
        depth[key] = d
        stack.extend((c, d + 1) for c in reversed(children[key]))
    for key in reversed(order):
        parent = nodes[key]["parent"]
        counts[parent] += counts[key]
        problems[parent] += problems[key]
    index = {k: i for i, k in enumerate(order)}
    active = registry.get("structure", {}).get("active")
    rows = []
    for key in order:
        node = nodes[key]
        name = node["name"]
        if "source" in node:
            stem = links.stem(node["source"])
            name = f"{name} ↗" if name == stem else f"{name} ↗ {stem}"
        cls = recipes[key].get("class")
        if "source" in node:
            kind = "Reference"
        elif cls is None and children.get(key):
            kind = ""  # a container: its children say what they are
        else:
            kind = kind_of(cls)
        parent = node["parent"]
        rows.append((key, index.get(parent, -1) if parent else -1, depth[key], name, kind,
                     int(counts[key]), int(problems[key]), key == active))
    view._ui_tree = dict(direct=direct, children=children, unfiled=unfiled, recipes=recipes)
    view.renderer.ui_set_tree(rows, len(unfiled))


def _tree_groups(view, key):
    data = getattr(view, "_ui_tree", None)
    if not data:
        return []
    out, stack = [], [key]
    while stack:
        k = stack.pop()
        out.extend(data["direct"].get(k, ()))
        stack.extend(data["children"].get(k, ()))
    return out


def _select(win, groups) -> None:
    scene = win.viewport.scene
    scene.select(g for g in groups if scene.entity_selectable(g))
    getattr(win.viewport, "notify_scene_changed", win.viewport.update)()


def _tree_action(win, view, action) -> None:
    from core import bim_registry, structure
    from core.group import Group
    from core.history import SetRegistryCommand
    from views.structure_recipe import execute
    verb, _, key = action.partition(":")
    vp = win.viewport
    try:
        if verb == "node":
            _select(win, _tree_groups(view, key))
        elif verb == "isolate":
            _isolate(win, [g.uid for g in _tree_groups(view, key)])
        elif verb in ("file", "filehere"):
            uids = [g.uid for g in vp.scene.selection if isinstance(g, Group)]
            if uids:
                execute(vp, structure.file(vp.scene, uids, key))
                path = " › ".join(structure.path(bim_registry.get(vp.scene), key))
                vp.flash_status(f"Filed {len(uids)} to {path}")
        elif verb == "active":
            registry = bim_registry.get(vp.scene)
            registry.setdefault("structure", {})["active"] = key
            execute(vp, SetRegistryCommand(registry))
        elif verb == "unfiled":
            _select(win, (getattr(view, "_ui_tree", None) or {}).get("unfiled", []))
    except (KeyError, ValueError) as exc:
        vp.flash_status(str(exc))


def _inspector(view, win) -> None:
    """What the selection is, in plain words; IFC under Export details."""
    scene = win.viewport.scene
    sel = list(scene.selection)
    sig = (id(scene), scene.content_version, len(sel), id(sel[0]) if sel else 0, id(sel[-1]) if sel else 0)
    if sig == getattr(view, "_ui_insp_sig", None):
        return
    view._ui_insp_sig = sig
    from collections import Counter
    from core import bim_registry, structure
    from core.group import Group
    if not sel:
        view.renderer.ui_set_inspector("", [])
        return
    registry = bim_registry.get(scene)
    nodes = structure._nodes(registry)
    groups = [g for g in sel if isinstance(g, Group)]
    if len(sel) == 1 and groups:
        g = groups[0]
        rec = g.ifc or {}
        key = structure.node_of(g)
        filed = " › ".join(structure.path(registry, key)) if key in nodes else "Not filed"
        recipe = (getattr(view, "_ui_tree", None) or {}).get("recipes", {}).get(key, {})
        title = g.name or getattr(getattr(g, "definition", None), "name", "") or "Group"
        rows = [("Filed in", filed), ("Kind", kind_of(rec.get("class") or recipe.get("class")) if (rec or recipe) else "—")]
        status = structure._has_status(rec)
        needs = [("Status", "set" if status else "missing")] if rec else [("Record", "none yet")]
        export = [("IFC class", str(rec.get("class") or recipe.get("class") or "generic object")),
                  ("Predefined type", str(rec.get("predefined_type") or recipe.get("predefined_type") or "—")),
                  ("Tag", str(recipe.get("tag") or "—"))]
        view.renderer.ui_set_inspector(title, [("Where", rows), ("What it needs", needs), ("Export details", export)])
        return
    kinds = Counter()
    unfiled = 0
    for g in groups:
        key = structure.node_of(g)
        if key in nodes:
            kinds[" › ".join(structure.path(registry, key)[-1:])] += 1
        else:
            unfiled += 1
    rows = [(name, str(n)) for name, n in kinds.most_common(8)]
    if unfiled:
        rows.append(("Not filed", str(unfiled)))
    others = len(sel) - len(groups)
    if others:
        rows.append(("Loose geometry", str(others)))
    view.renderer.ui_set_inspector(f"{len(sel)} selected", [("By node", rows)])


def _dock(win, names) -> None:
    from PySide6.QtWidgets import QDockWidget
    for dock in win.findChildren(QDockWidget):
        if any(n in dock.windowTitle() for n in names):
            dock.show()
            dock.raise_()
            return


def _file_to(win, n: int) -> None:
    """The tree takes the next pick (or typed name + Enter)."""
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    if view is not None:
        view.renderer.ui_start_filing(f"{n} item" + ("s" if n != 1 else ""))
        view.update()


def after_frame(view) -> None:
    if not getattr(view, "_ui_attached", False):
        return
    renderer = view.renderer
    actions = renderer.ui_take_actions()
    if actions:
        win = _window(view)
        # Never switch tools from inside a paint.
        QTimer.singleShot(0, view, lambda: _run(win, actions))
    delay = renderer.ui_repaint_after_ms()
    if delay is not None:
        view._ui_timer.start(max(0, int(delay)))


def _run(win, actions) -> None:
    if win is None:
        return
    for action in actions:
        kind, _, arg = action.partition(":")
        if kind == "tool":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            run = getattr(view, "_ui_palette_runs", {}).get(arg)
            if run is not None:
                run()
            else:
                win._activate_tool(arg)
        elif kind == "opt":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            run = next((t for i, _, _, _, t in _options(view) if i == arg and t), None) if view else None
            if run is not None:
                run()
                view.update()
        elif kind == "tree":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            _tree_action(win, view, arg)
        elif kind == "act":
            _trigger(win, int(arg))
        elif kind == "cmd":
            sel = list(win.viewport.scene.selection)
            run = next((r for k, _, _, r in _commands(win, sel) if k == arg), None)
            if run is not None:
                run()


def _trigger(win, index: int) -> None:
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    actions = getattr(view, "_ui_actions", [])
    if 0 <= index < len(actions):
        actions[index].trigger()


_KEYS = {Qt.Key_Escape: "Escape", Qt.Key_Return: "Enter", Qt.Key_Enter: "Enter",
         Qt.Key_Backspace: "Backspace", Qt.Key_Delete: "Delete", Qt.Key_Tab: "Tab",
         Qt.Key_Up: "ArrowUp", Qt.Key_Down: "ArrowDown", Qt.Key_Left: "ArrowLeft",
         Qt.Key_Right: "ArrowRight", Qt.Key_Home: "Home", Qt.Key_End: "End",
         Qt.Key_A: "A", Qt.Key_C: "C", Qt.Key_V: "V", Qt.Key_X: "X", Qt.Key_Z: "Z"}


def claims_key(view, event) -> bool:
    """ShortcutOverride: does the chrome own this key (a focused field, or
    Escape while its popup is open)? Then no window shortcut fires."""
    if not getattr(view, "_ui_attached", False):
        return False
    name = _KEYS.get(event.key(), "")
    return view.renderer.ui_claims_key(name)


def key(view, event, pressed: bool) -> bool:
    """Offer a key to the chrome first. True: it was the chrome's."""
    if not getattr(view, "_ui_attached", False):
        return False
    name = _KEYS.get(event.key(), "")
    if not view.renderer.ui_claims_key(name):
        return False
    view.renderer.ui_key(name, pressed, _mods(event))
    text = event.text()
    if pressed and text and text.isprintable():
        view.renderer.ui_text(text)
    view.update()
    return True


def _mods(event) -> int:
    m = event.modifiers()
    return ((1 if m & Qt.AltModifier else 0) | (2 if m & Qt.MetaModifier else 0)
            | (4 if m & Qt.ShiftModifier else 0) | (8 if m & Qt.ControlModifier else 0))


_BUTTONS = {Qt.LeftButton: 1, Qt.RightButton: 2, Qt.MiddleButton: 3}


def wheel(view, event) -> bool:
    if not getattr(view, "_ui_attached", False):
        return False
    p = event.position()
    d = event.pixelDelta()
    if d.isNull():
        a = event.angleDelta()
        dx, dy = a.x() / 8.0 * 3, a.y() / 8.0 * 3
    else:
        dx, dy = float(d.x()), float(d.y())
    taken = view.renderer.ui_wheel(float(p.x()), float(p.y()), dx, dy)
    if taken:
        view.update()
    return taken


def pointer(view, kind: str, event=None) -> bool:
    """Offer a pointer event to the chrome first. True: it was the chrome's."""
    if not getattr(view, "_ui_attached", False):
        return False
    if event is None:
        taken = view.renderer.ui_pointer(kind, -1.0, -1.0, 0, 0)
    else:
        p = event.position()
        taken = view.renderer.ui_pointer(kind, float(p.x()), float(p.y()),
                                         _BUTTONS.get(event.button(), 1), _mods(event))
    if taken or kind != "move" or getattr(view, "_ui_was_over", False):
        view.update()  # hover highlight in, or out
    if kind in ("move", "leave"):
        was = getattr(view, "_ui_was_over", False)
        if taken and not was:
            view.setCursor(Qt.ArrowCursor)
        elif was and not taken:
            view.tools._apply_tool_cursor()
        view._ui_was_over = taken
    return taken
