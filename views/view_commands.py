"""Bindings between the Rust view command registry and the document controller."""
from functools import lru_cache

from ingetrazo_render import activate_view_command, command_chrome, run_view_command, view_commands


@lru_cache(maxsize=1)
def entries():
    return view_commands()


def _snapshot(win):
    vp = win._active_view()
    scene = vp.scene
    style = win.active_style()
    active = vp.nav_mode or win._tool_key(vp.active_tool) or ""
    flags = {field for field in ("back_edges", "section_fill") if getattr(style, field, False)}
    flags.update(field for field in ("show_section_planes", "show_section_cuts", "show_hidden_geometry")
                 if getattr(scene, field, False))
    if getattr(style, "face_mode", None) == "xray":
        flags.add("xray")
    if scene.shadows.enabled:
        flags.add("shadows")
    return (active, vp.camera.projection_mode(), getattr(style, "name", ""), flags, bool(scene.selection))


@lru_cache(maxsize=4)
def _labels(language):
    from core.i18n import tr
    labels = {text for group, _, label, _, _ in entries() for text in (group, label)}
    return {text: tr(text) for text in labels | {"Navigation", "Walkthrough", "Display"}}


def sync(view, win):
    from core.i18n import current_language
    snapshot = _snapshot(win)
    language = current_language()
    sig = (snapshot, language, vars(win).get("_view_binding_version", 0))
    if sig != getattr(view, "_ui_cluster_sig", None):
        cluster, menu = command_chrome(snapshot, _labels(language), vars(win).get("_view_bindings", {}))
        view.renderer.ui_set_cluster(cluster, menu)
        view._ui_cluster_sig = sig


def run(win, key):
    bindings = vars(win).get("_view_bindings", {})
    if not bindings.get(key, ("", True))[1]:
        return False
    ran = activate_view_command(win, key, _snapshot(win))
    if ran and key in bindings and "_host" not in vars(win):
        win._sync_section_menu()
        win._sync_style_menu()
    return ran


def bind(win, action, key, *, toggle=False):
    """Connect a classic host action before its shortcut customizations load."""
    from PySide6.QtGui import QKeySequence
    entry = next(row for row in entries() if row[1] == key)
    action.setShortcuts([QKeySequence(shortcut) for shortcut in entry[3]])
    signal = action.toggled if toggle else action.triggered
    signal.connect(lambda on=False: run_view_command(win, key, on))
    bindings = win.__dict__.setdefault("_view_bindings", {})

    def reflect():
        value = (action.shortcut().toString(QKeySequence.NativeText), action.isEnabled())
        if bindings.get(key) != value:
            bindings[key] = value
            win._view_binding_version = vars(win).get("_view_binding_version", 0) + 1

    action.changed.connect(reflect)
    reflect()
