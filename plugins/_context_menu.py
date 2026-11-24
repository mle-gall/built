"""Extension actions in the renderer-owned viewport menu."""
from uuid import uuid4

import ingetrazo_render as native
from core.extensions import note_failure


def _present(win, view, menu):
    if view is None or menu is None:
        return menu
    view._plugin_context = None
    providers = getattr(win, "_ext_context_menus", ())
    renderer = getattr(view, "renderer", None)
    if not providers or not hasattr(renderer, "ui_set_context_menu"):
        return menu
    actions = native.ExtensionMenus()
    for provider in providers:
        owner = win._ext_context_owner.get(provider, "extension")
        try:
            provider(actions.root(owner), menu.sel)
        except (Exception, SystemExit):
            actions.discard(owner)
            note_failure(f"Context menu failed for {owner}")
    commands = [row for row in actions.rows() if row[2]]
    if commands:
        rows = [(row.label, [(v.id, v.label, v.key, v.enabled, v.checked) for v in row.verbs])
                for row in menu.rows]
        token = "plugin-" + uuid4().hex + ":"
        for key, path, label, shortcut, checked, enabled in commands:
            name = " ▸ ".join([*path, label])
            rows.append(("", [(token + key, name, shortcut, enabled, checked)]))
        scene = win.viewport.scene
        view._plugin_context = (token, menu, actions, scene, scene.version)
        renderer.ui_set_context_menu(*menu.at, menu.title, menu.subtitle, rows)
        view.update()
    return menu


def context_menu_open_at(view, x, y):
    menu = native.context_menu_open_at(view, x, y)
    return _present(view.source_viewport.window(), view, menu) if menu is not None else None


def context_menu_show(win, locked_image=None, locked_group=None):
    menu = native.context_menu_show(win, locked_image, locked_group)
    return _present(win, win.view, menu)


def context_menu_action(win, view, key):
    context = vars(view).pop("_plugin_context", None) if view is not None else None
    if not key.startswith("plugin-"):
        return native.context_menu_action(win, view, key)
    if context is None:
        return False
    token, menu, actions, scene, revision = context
    if (not key.startswith(token) or view._ctx_menu is not menu
            or scene is not win.viewport.scene or scene.version != revision):
        return False
    view._ctx_menu = None
    return actions.trigger(key[len(token):])
