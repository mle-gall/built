"""UI spike (branch ``ui/egui-spike``): the viewport chrome drawn by egui
inside the Metal renderer, in the same frame as the model.

The Metal view draws the tool palette, command cluster and status line.
Both hosts bind the Rust view registry to their document controller.
"""
from __future__ import annotations

import ingetrazo_core
import os
from pathlib import Path

from core.input_codes import Input as Qt
from core.timers import later
from ingetrazo_render import shortcut_text
from core.shortcuts import action_keys

from plugins._widget_language import FONT
from ingetrazo_render import rail_icon as _icon_rgba


#: Drawing and Export have room for previews and a readiness checklist.
WIDE_DOCK = 430.0
#: Drawing and Export reserve room for their columns.
WIDE_LENSES = (1, 2)
#: Window buttons' room at the top left (mac_glass: 14 + three buttons).
LIGHTS_ROOM = 86.0
#: The docked top bar (ui.rs TOP_H): its empty parts move the window.
TOP_H = 40.0
_CTRL = None


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


SEP = "\x1f"
#: Item rows shown per node, definition or item before a "Show more" row;
#: each click on it shows this many more (ui/tree-links: the old cap had
#: no way past it but the filter).
ROW_CAP = 200
#: Between a row's kind and its tooltip (ui.rs splits the kind on it).
TIP = "\x1e"
TIPS = {
    "Reference": "Reference — a linked file. Its contents are read from {path} and are never edited "
                 "in this document: open the source to change them. Eye and lock here are this "
                 "document's own overrides of the file's tags; the file itself is not touched.",
    "Drawing only": "Drawing only — this node has no IFC class: its items are drawn, measured and "
                    "printed, but not exported as building elements. Give it a kind to export them.",
    "Linked files": "The files this document references. Each one shows its own structure, read-only.",
}


#: A file row's state, in words (its dot's tooltip and the inspector).
FILE_STATES = {"resolved": "Up to date", "modified": "Changed on disk — reload to see the changes",
               "missing": "Not found — relink it", "packed": "Packed in this file"}


UNIT_TEXT = {"m2": "m²", "m3": "m³", "m": "m", "und": "u", "kg": "kg"}


ED = "\x1d"


LINK_STATES = {"resolved": "Up to date", "modified": "Changed on disk", "missing": "Missing", "packed": "Packed in this file"}


#: The IFC Status values offered as one click (the rest stay in the BIM panel).
STATUS_CHOICES = ["EXISTING", "NEW", "DEMOLISH", "TEMPORARY"]


def free_area(win):
    """Where the model is seen, in device coordinates (ui.rs ``free_ndc``):
    the window minus the rail, dock and bars showing. None without the chrome."""
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    if view is None or not getattr(view, "_ui_attached", False):
        return None
    return view.renderer.ui_free_ndc()


def _review_camera(win, on: bool) -> None:
    """Review frames the model into the window the panels left (B8); leaving
    it, the camera goes back to where it was if Review's framing is untouched."""
    if on:
        before = win._camera_pose()
        win._on_zoom_extents()
        win.__dict__["_review_back"] = (before, win._camera_pose())
        return
    back = win.__dict__.pop("_review_back", None)
    if back is not None and back[1] == win._camera_pose():
        cam = win.viewport.camera
        x, y, z, cam.distance, cam.yaw, cam.pitch = back[0]
        cam.target = (x, y, z)
        win.viewport.update()


def _run(win, actions, source_view=None) -> None:
    if win is None:
        return
    from ingetrazo_render import chrome_dialogs
    for action in actions:
        kind, _, arg = action.partition(":")
        if kind == "extension":
            win.run_command(action)
            continue
        if kind == "scenephoto":
            from ingetrazo_render import render_scene_photo
            view = chrome_dialogs.view_of(win)
            if view is not None:
                pixels = None
                try:
                    pixels = render_scene_photo(win, arg)
                except (RuntimeError, ValueError, OSError):
                    pass  # The visible row retries a missed frame.
                finally:
                    view.renderer.ui_finish_scene_photo(arg, pixels)
            continue
        if kind == "scenephotomode":
            from ingetrazo_render import save_thumbnail_mode
            save_thumbnail_mode(arg == "1")
            continue
        if kind == "font":
            from ingetrazo_render.font_picker import answer
            answer(win, arg, source_view)
            continue
        if kind == "review":
            _review_camera(win, arg == "1")
            continue
        if kind == "sheet":
            chrome_dialogs.answer(win, arg)
            continue
        if kind == "job":
            verb, _, jid = arg.partition(":")
            job = chrome_dialogs.find_job(win, jid)
            if verb == "cancel" and job is not None:
                job.cancel()
            continue
        if kind == "dlg":
            if chrome_dialogs.current_sheet(win) is None:
                from ingetrazo_render import chrome_forms
                chrome_forms.action(win, arg)
            continue
        if chrome_dialogs.modal(win):
            continue    # a click queued before the sheet came up
        if kind == "plugin":
            from core.plugin_panels import dispatch
            dispatch(win, arg)
            continue
        if kind == "activity":
            row = next((r for r in chrome_dialogs.history(win) if r.id == arg), None)
            if row and row.action:
                row.action()
            continue
        if kind == "linkbanner":
            if arg == "locate":
                from core import links
                scene = win.viewport.scene
                group = next((g for g in links.linked_groups(scene)
                              if links.link_state(scene, links.link_of(g)["path"]) == "missing"), None)
                if group:
                    _link_action(win, chrome_dialogs.view_of(win), group, "linkrelink")
                chrome_dialogs.update_link_banner(win)
            else:
                win._doc._missing_link_banner = ""
            chrome_dialogs._repaint(win)
            continue
        if kind == "lsheet":
            from ingetrazo_render import chrome_sheet
            chrome_sheet.action(win, arg)
            continue
        if kind == "start":
            from ingetrazo_render import wgpu_start
            wgpu_start.action(win, arg)
            continue
        if kind == "sheets":
            from views import wgpu_sheets
            wgpu_sheets.action(win, arg)
            continue
        if (kind, arg) == ("tab", "scene:editcamera"):
            from ingetrazo_render import marked_view
            win.edit_scene_camera(marked_view(win))
            continue
        if kind == "panel" or (kind, arg) in (("act", "camera"), ("tab", "scene:camera")):
            from ingetrazo_render import inspector_action
            inspector_action(win, win._doc.stand_in, arg if kind == "panel" else '["camera.open",null]')
            continue
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
        elif kind == "pal":
            from ingetrazo_render import wgpu_library
            wgpu_library.paint_from_strip(win, arg)
        elif kind == "libsheet":
            from ingetrazo_render import wgpu_library
            wgpu_library.open_library(win, arg or None)
        elif kind == "lib":
            from ingetrazo_render import wgpu_library
            wgpu_library.action(win, getattr(getattr(win, "_doc", None), "stand_in", None), arg)
        elif kind == "dockw":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            keep_dock_width(view, arg)
        elif kind in ("cmdscope", "cmdq", "cmdrun", "cmdshortcut"):
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            if view is not None:
                command_action(view, win, kind, arg)
                view.update()
        elif kind == "proj":
            _project_action(win, arg)
        elif kind == "ptab":
            from views import wgpu_tabs
            wgpu_tabs.action(win, getattr(getattr(win, "_doc", None), "stand_in", None), arg)
        elif kind == "ctx":
            from plugins._context_menu import context_menu_action
            context_menu_action(win, getattr(getattr(win, "_doc", None), "stand_in", None), arg)
        elif kind == "tab":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            if arg == "model":
                win._show_model()
            elif arg == "sheet":
                win._to_sheets()
            elif arg.startswith("visibility:"):
                operation, _, key = arg[len("visibility:"):].partition(":")
                vp = win.viewport
                if operation in ("save", "rename", "delete"):
                    labels = getattr(view, "_ui_visibility_cache", (None, None, []))[2]
                    label = next((r[1] for r in labels if r[0] == f"visibility:apply:{key}"), "")
                    title = {"save": "Save visibility set", "rename": "Rename visibility set",
                             "delete": f"Delete “{label}”?"}[operation]
                    body = "" if operation == "delete" else f"{ED}edit{ED}name{ED}{label}"
                    view._ui_pending = ("visibility_set", (vp.scene, operation, key))
                    view.renderer.ui_set_prompt(title, body, [("yes", operation.capitalize()), ("no", "Cancel")])
                elif _core(vp, "edit_visibility_set", verb=operation, key=key)["ok"]:
                    vp.notify_scene_changed()
                    view.update()
            elif arg.startswith("scenedelete:"):
                from ingetrazo_core import scene_tree
                try:
                    targets = scene_tree.menu_selection(win.viewport.scene, arg.partition(":")[2])
                except ValueError as exc:
                    win.viewport.flash_status(str(exc))
                    continue
                view._ui_pending = ("scene_delete", targets)
                view.renderer.ui_set_prompt(f"Delete {len(targets)} scenes?", "",
                                            [("yes", "Delete"), ("no", "Cancel")])
            elif arg.startswith("scenefolders:"):
                from ingetrazo_core import scene_tree
                vp = win.viewport
                try:
                    scene_tree.edit_menu_folder(vp.history, arg.partition(":")[2],
                                                getattr(win.tray.scenes, "add_scene_in_folder", None))
                    vp.notify_scene_changed()
                    view._ui_tabs = None
                    view.update()
                except ValueError as exc:
                    vp.flash_status(str(exc))
            elif arg in ("scene:add", "scene:update", "scene:up", "scene:down", "scene:delete", "scene:rename", "scene:duplicate"):
                panel = getattr(win.tray, "scenes", None)
                row = getattr(getattr(win, "_scene_tabs", None), "tabs", None)
                marked = row.marked_view() if row is not None else None
                if panel is not None:
                    if arg == "scene:add":
                        panel.add_scene()
                    elif marked is not None and arg == "scene:duplicate":
                        panel.duplicate_view(marked)
                    elif marked is not None and arg == "scene:update":
                        panel.update_view(marked)
                        win.viewport.flash_status(f"Updated “{marked.name}” from the view")
                    elif marked is not None and arg in ("scene:up", "scene:down"):
                        from ingetrazo_core import scene_tree
                        vp = win.viewport
                        if scene_tree.move_among_siblings(vp.history, marked, arg == "scene:up"):
                            vp.notify_scene_changed()
                    elif marked is not None and arg == "scene:rename":
                        view._ui_pending = ("scene_rename", marked)
                        view.renderer.ui_set_prompt("Rename scene", f"{ED}edit{ED}scene{ED}{marked.name}",
                                                    [("yes", "Rename"), ("no", "Cancel")])
                    elif marked is not None and arg == "scene:delete":
                        view._ui_pending = ("scene_delete", [marked])
                        view.renderer.ui_set_prompt(f"Delete “{marked.name}”?", "The scene goes; the model does not change.",
                                                    [("yes", "Delete"), ("no", "Cancel")])
                    view._ui_tabs = None
            elif arg in ("scene:prev", "scene:next"):
                win.step_scene(1 if arg == "scene:next" else -1)
            elif arg.startswith("scene:") and arg[6:].isdecimal():
                from ingetrazo_render import show_listed_scene
                show_listed_scene(win, getattr(view, "_ui_scene_views", []), int(arg[6:]))
        elif kind == "deliver":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            data = getattr(view, "_ui_tree", None) or {}
            if arg in ("unfiled", "problems"):
                from ingetrazo_render import wgpu_library
                wgpu_library.set_mode(view, win, "problems")
            elif arg == "active" and data.get("active"):
                _select(win, _tree_groups(view, data["active"]))
            else:
                from ingetrazo_render import delivery_action
                delivery_action(win, view, arg)
        elif kind == "act" and arg.startswith("form-"):
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            if view is not None:
                _open_form(view, arg[5:])
        elif kind == "form":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            if view is not None:
                _form_apply(win, view, arg)
        elif kind == "act" and arg.isdecimal():
            # The Sheets workspace retains its composer actions.
            from views.wgpu_sheets import workspace_of
            ws = workspace_of(source_view or getattr(win._doc, "stand_in", None))
            if ws is not None and int(arg) < len(ws._actions):
                ws._actions[int(arg)].trigger()
        elif kind == "act":
            from views.view_commands import run
            run(win, arg)
        elif kind == "cmd":
            sel = list(win.viewport.scene.selection)
            run = next((r for k, _, _, r, ok, _ in _commands(win, sel) if k == arg and ok), None)
            if run is not None:
                run()


# ---- Settings forms ---------------------------------------------------------
# A form mirrors an existing panel's controls, so its logic (live apply,
# daylight range, the style library) stays the panel's: (kind, widget
# attribute, label, value text: QLabel attribute or callable, style attr).
FORMS = {
    "style": ("Style", "styles_panel", [
        ("choice", "_combo", "Style"),
        ("choice", "_mode", "Faces"),
        ("caption", None, "Edges"),
        ("check", "_edges", "Edges"),
        ("color", "_edge_c", "Edge colour", None, "edge_color"),
        ("check", "_profiles", "Profiles"),
        ("check", "_back_edges", "Back edges, dashed"),
        ("caption", None, "Faces and background"),
        ("color", "_front_c", "Front colour", None, "front_color"),
        ("color", "_back_c", "Back colour", None, "back_color"),
        ("check", "_sky", "Sky"),
        ("color", "_sky_c", "Sky colour", None, "sky_color"),
        ("color", "_ground_c", "Ground", None, "ground_color"),
        ("color", "_bg_c", "Background", None, "background"),
        ("check", "_fill", "Section fill"),
        ("color", "_fill_c", "Section fill colour", None, "section_fill_color"),
        ("button", "_on_save", "Save style…"),
        ("button", "_on_delete", "Delete style", None, "_del_btn"),
    ]),
    "dimstyle": ("Dimension style", "dimstyle_panel", [
        ("choice", "_units", "Unit"),
        ("slider", "_decimals", "Decimals", lambda p: str(p._decimals.value())),
        ("check", "_show_unit", "Show the unit"),
        ("slider", "_font", "Text size", lambda p: f"{p._font.value()} pt"),
        ("button", None, "Font family…"),
        ("color", "_color_btn", "Colour", None, lambda p: [c / 255 for c in p._style().get("color", [45, 55, 75])]),
        ("choice", "_ends", "Ends"),
        ("slider", "_base_step", "Baseline step", lambda p: f"{p._base_step.value():g} mm", None, 2),
        ("text", None, "Applies to every dimension in the document."),
    ]),
    "shadows": ("Shadows and sun", "shadows_panel", [
        ("check", "_enable", "Show shadows"),
        ("slider", "_doy", "Date", lambda p: p._date.date().toString("d MMMM")),
        ("slider", "_time", "Time", "_time_lbl"),
        ("text", None, lambda p: f"Sunrise {p._sunrise_lbl.text()} · sunset {p._sunset_lbl.text()}"
                                  if p._sunrise_lbl.text() else ""),
        ("slider", "_dark", "Darkness", lambda p: f"{p._dark.value()} %"),
        ("choice", "_tz", "Time zone"),
        ("caption", None, "Site"),
        ("text", None, lambda p: p._site.text()),
        ("button", "_on_north_tool", "Set north on screen"),
        ("button", "_on_add_location", "Add location…"),
    ]),
}


def _open_form(view, name) -> None:
    win = _window(view)
    view._ui_form = name
    view._ui_form_sig = None
    panel = _form_panel(win, name) if win is not None else None
    if panel is not None and hasattr(panel, "refresh"):
        panel.refresh()
    view.update()


def _form_panel(win, name):
    spec = FORMS.get(name)
    return getattr(win, spec[1], None) if spec else None


def _form(view, win) -> None:
    """Send the open form's fields (only when they changed)."""
    name = view.__dict__.get("_ui_form")
    if not name:
        return
    panel = _form_panel(win, name)
    if panel is None:
        view._ui_form = None
        view.renderer.ui_set_form("", [])
        return
    title, _attr, spec = FORMS[name]
    fields = []

    def text_of(t):
        if callable(t):
            return str(t(panel))
        if isinstance(t, str) and t.startswith("_") and hasattr(panel, t):
            return getattr(panel, t).text()
        return str(t or "")

    for i, entry in enumerate(spec):
        kind, attr, label = entry[:3]
        extra = entry[3] if len(entry) > 3 else None
        label = text_of(label) if callable(label) else label
        fid = str(i)
        if kind == "caption":
            fields.append((fid, kind, label, "", 0.0, 0.0, 0.0, []))
            continue
        if kind == "text":
            t = text_of(label)
            if t:
                fields.append((fid, kind, "", t, 0.0, 0.0, 0.0, []))
            continue
        if kind == "button":
            gate = entry[4] if len(entry) > 4 else None
            if gate and not getattr(panel, gate).isEnabled():
                continue
            fields.append((fid, kind, label, "", 0.0, 0.0, 0.0, []))
            continue
        w = getattr(panel, attr, None)
        if w is None or w.isHidden() or not w.isEnabled():
            continue
        if kind == "check":
            fields.append((fid, kind, label, "", float(w.isChecked()), 0.0, 1.0, []))
        elif kind == "slider":
            k = entry[5] if len(entry) > 5 else 1
            fields.append((fid, kind, label, text_of(extra) if extra else str(w.value()),
                           float(w.value()) * k, float(w.minimum()) * k, float(w.maximum()) * k, []))
        elif kind == "choice":
            opts = [w.itemText(j) for j in range(w.count())]
            fields.append((fid, kind, label, w.currentText(), float(w.currentIndex()), 0.0, float(len(opts)), opts))
        elif kind == "color":
            try:
                c = entry[4](panel) if callable(entry[4]) else panel._shown_color(entry[4])
            except Exception:
                c = None
            fields.append((fid, kind, label, _hex(c) if c else "", 0.0, 0.0, 0.0, []))
    sig = (name, tuple(fields))
    if sig != view.__dict__.get("_ui_form_sig"):
        view._ui_form_sig = sig
        view.renderer.ui_set_form(title, fields)


def _form_apply(win, view, arg) -> None:
    if arg == "close":
        view._ui_form = None
        view._ui_form_sig = None
        return
    name = view.__dict__.get("_ui_form")
    panel = _form_panel(win, name)
    if panel is None:
        return
    fid, _, value = arg.partition(":")
    entry = FORMS[name][2][int(fid)]
    kind, attr = entry[0], entry[1]
    if name == "dimstyle" and kind == "button" and attr is None:
        from ingetrazo_render.font_picker import annotation
        annotation(win)
    elif kind == "button":
        getattr(panel, attr)()
    else:
        w = getattr(panel, attr)
        if kind == "check":
            w.setChecked(value == "1")
        elif kind == "slider":
            k = entry[5] if len(entry) > 5 else 1
            v = float(value) / k
            w.setValue(v if k != 1 else int(v))
        elif kind == "choice":
            i = int(value)
            w.setCurrentIndex(i)
            w.activated.emit(i)
        elif kind == "color":
            w.click()
    if hasattr(panel, "refresh") and kind != "slider":
        panel.refresh()
    view._ui_form_sig = None
    view._ui_cluster_sig = None
    view.update()


_KEYS = {Qt.Key_PageUp: "PageUp", Qt.Key_PageDown: "PageDown", Qt.Key_Escape: "Escape",
         Qt.Key_Return: "Enter", Qt.Key_Enter: "Enter",
         Qt.Key_Backspace: "Backspace", Qt.Key_Delete: "Delete", Qt.Key_Tab: "Tab", Qt.Key_Backtab: "Tab", Qt.Key_Space: "Space", Qt.Key_F2: "F2",
         Qt.Key_Up: "ArrowUp", Qt.Key_Down: "ArrowDown", Qt.Key_Left: "ArrowLeft",
         Qt.Key_Right: "ArrowRight", Qt.Key_Home: "Home", Qt.Key_End: "End",
         Qt.Key_A: "A", Qt.Key_C: "C", Qt.Key_V: "V", Qt.Key_X: "X", Qt.Key_Y: "Y", Qt.Key_Z: "Z",
         Qt.Key_F: "F"}


def _opens_commands(view, event) -> bool:
    window = view.window()
    action = vars(window).get("_registry_actions", {}).get("command_search")
    if action is None:
        return False
    combo = event.key() | event.modifiers().value
    return any(parsed[2] == combo for key in action_keys(action) if (parsed := shortcut_text(key)))


def _all_commands(win):
    from ingetrazo_render.command_metadata import commands
    return [(cmd.name, cmd.path, cmd.keys, cmd.action) for cmd in commands(win)]


def command_action(view, win, verb, arg):
    from ingetrazo_render import command_metadata
    if verb == "cmdscope":
        view._ui_command_scope = arg
        view._ui_command_owner = None
    elif verb == "cmdq":
        _command_query(view, win, arg)
    elif verb in ("cmdrun", "cmdshortcut"):
        hits = getattr(view, "_ui_cmd_hits", ())
        i = int(arg)
        if 0 <= i < len(hits):
            action = hits[i]
            if verb == "cmdrun":
                command_metadata.run(action)
            elif action.isVisible():
                owner = win if hasattr(win, "open_preferences") else win.parentWidget()
                if owner is not None:
                    owner.open_preferences(shortcut_of=action)


def _command_query(view, win, query) -> None:
    from views import wgpu_sheets
    from ingetrazo_render import command_metadata
    workspace = wgpu_sheets.workspace_of(view)
    from ingetrazo_render import sheet_command_scope
    owner = sheet_command_scope(win) if workspace is not None else win
    if not query or getattr(view, "_ui_command_owner", None) is not owner:
        view._ui_command_pool = command_metadata.commands(owner, getattr(view, "_ui_command_scope", ""))
        view._ui_command_owner = owner
    live = [c for c in view._ui_command_pool if c.action.isVisible()]
    hits = command_metadata.search(owner, query, pool=live)
    view._ui_cmd_hits = [c.action for c in hits]
    view.renderer.ui_set_command_results([(c.name, c.path, c.keys, c.action.isEnabled()) for c in hits])
    detail = getattr(view.renderer, "ui_set_command_details", None)
    if detail:
        detail([(c.tip, bool(getattr(c.action, "isChecked", lambda: False)()),
                 hasattr(c.action, "isChecked")) for c in hits])


def _problem_key(view, event) -> int:
    if view.__dict__.get("_ui_lens") != 2 or not (event.modifiers() & Qt.ControlModifier):
        return 0
    return {Qt.Key_BracketRight: 1, Qt.Key_BracketLeft: -1}.get(event.key(), 0)


def _reveal_shortcut(event):
    return event.key() == Qt.Key_J and _mods(event) == 12


def claims_key(view, event) -> bool:
    """ShortcutOverride: does the chrome own this key (a focused field, or
    Escape while its popup is open)? Then no window shortcut fires."""
    if not getattr(view, "_ui_attached", False):
        return False
    from ingetrazo_render import chrome_sheet
    large = chrome_sheet.claims_key(view, event)
    if large is not None:
        # The large sheet owns every key but the window's ⌘ shortcuts.
        return large or _opens_commands(view, event)
    if event.key() == Qt.Key_Escape:
        return True  # Reserve it for the chrome and viewport Escape router.
    if _reveal_shortcut(event) or _problem_key(view, event):
        return True
    if _opens_commands(view, event):
        return True
    name = _KEYS.get(event.key(), "")
    if view.renderer.ui_claims_key(name, _mods(event)):
        return True
    from views import wgpu_sheets
    return wgpu_sheets.claims_key(view, event)


def key(view, event, pressed: bool) -> bool:
    """Offer a key to the chrome first. True: it was the chrome's."""
    if not getattr(view, "_ui_attached", False):
        return False
    if _opens_commands(view, event):
        if pressed:
            view.renderer.ui_open_commands()
            view.update()
        return True
    from ingetrazo_render import chrome_sheet
    if chrome_sheet.claims_key(view, event) is False:
        return False
    from views import wgpu_sheets
    sheets = wgpu_sheets.shown(view)
    if not sheets and _reveal_shortcut(event):
        if pressed:
            _tree_action(view.window(), view, "revealselection" + SEP)
        return True
    step = 0 if sheets else _problem_key(view, event)
    if step:
        if pressed:
            view.renderer.ui_step_issue(step)
            view.update()
        return True
    name = _KEYS.get(event.key(), "")
    if not view.renderer.ui_claims_key(name, _mods(event)):
        # The Sheets workspace: the page's keys go to the composer.
        return wgpu_sheets.key(view, event, pressed) if sheets else False
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
    from ingetrazo_render import chrome_dialogs
    if chrome_dialogs.modal(_window(view)):
        return True
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
        return True
    from views import wgpu_sheets
    return wgpu_sheets.wheel(view, event)


def _held(view, kind, event) -> bool:
    """Exclusive work holds the window: only its chip (Cancel) and the
    window's own drag take the pointer; the model and the rest of the
    chrome see nothing until it ends."""
    from ingetrazo_render import chrome_dialogs
    win = _window(view)
    if win is None or not chrome_dialogs.exclusive(win) or chrome_dialogs.current_sheet(win) is not None:
        return False
    if event is None:
        return False
    p = event.position()
    rect = view.renderer.ui_area_rect("jobs")
    if rect is not None:
        x, y, w, h = rect
        if x <= p.x() <= x + w and y <= p.y() <= y + h:
            return False        # the chip's own: Cancel
    if kind == "press" and event.button() == Qt.LeftButton \
            and view.renderer.ui_top_empty(float(p.x()), float(p.y())):
        return False            # the window still moves
    view.renderer.ui_pointer("leave", -1.0, -1.0, 0, 0)
    return True


def pointer(view, kind: str, event=None) -> bool:
    """Offer a pointer event to the chrome first. True: it was the chrome's
    (or, in the Sheets workspace, the page's)."""
    if not getattr(view, "_ui_attached", False):
        return False
    from views import wgpu_sheets
    if _held(view, kind, event):
        return True
    if event is None:
        taken = view.renderer.ui_pointer(kind, -1.0, -1.0, 0, 0)
    elif kind == "press" and event.button() == Qt.LeftButton \
            and view.renderer.ui_top_empty(float(event.position().x()), float(event.position().y())):
        # The empty top strip is the window's title bar: drag moves it.
        view.window().start_system_move()
        return True
    else:
        p = event.position()
        taken = view.renderer.ui_pointer(kind, float(p.x()), float(p.y()),
                                         _BUTTONS.get(event.button(), 1), _mods(event))
    if taken or kind != "move" or getattr(view, "_ui_was_over", False):
        view.update()  # hover highlight in, or out
    if kind in ("move", "leave"):
        was = getattr(view, "_ui_was_over", False)
        shape = view.renderer.ui_cursor_shape() if taken and hasattr(view.renderer, "ui_cursor_shape") else ""
        if shape == "resize_h":
            view.setCursor(Qt.SizeHorCursor)
        elif taken and (not was or getattr(view, "_ui_cursor_shape", "")):
            view.setCursor(Qt.ArrowCursor)
        elif was and not taken and not wgpu_sheets.shown(view):
            view.tools._apply_tool_cursor()
        view._ui_was_over = taken
        view._ui_cursor_shape = shape
    if wgpu_sheets.shown(view):
        # The Sheets workspace: what the chrome leaves is the page's.
        if taken:
            wgpu_sheets.chrome_took(view)
            return True
        return wgpu_sheets.pointer(view, kind, event)
    return taken


# Transitional: the functions already native (chrome_*.rs) go on the module.
import ingetrazo_render
ingetrazo_render.register_view_functions(globals())
