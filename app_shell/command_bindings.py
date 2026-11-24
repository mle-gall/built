"""Bind the Rust registry to the window's tool actions and the installed menu."""

from app_shell.command_action import ShellAction

from core.i18n import tr
from ingetrazo_render import command_registry, shortcut_text
from views import shortcuts


def refresh(win):
    from app_shell import native
    get_menu = getattr(native._shell, "menu_commands", None)
    native_items = {b.id: b for b in get_menu() if not b.id.startswith("extension:")} if get_menu else {}
    win._menu_items = native_items
    menu = [(b.id, b.path, b.name, b.keys) for b in native_items.values()]
    menus = vars(win).get("_extension_menus")
    enabled = {}
    if menus is not None:
        menu.extend(menus.commands())
        enabled = {row[0]: row[-1] for row in menus.rows()}
    actions = win._registry_actions
    old_keys = set(actions)
    previous = old_keys.copy()
    paths = {}
    for key, group, label, defaults, tool_id, menu_id in command_registry(win._tool_defaults, menu):
        paths[key] = tr(group)
        if key not in actions:
            act = win._tool_actions.get(tool_id) if tool_id else None
            if act is None:
                act = ShellAction(win, tr(label))
            else:
                act.setProperty("legacyShortcutKey", shortcuts.action_key(act))
            act.run = lambda k=key: _run(win, k)
            act.setObjectName(key)
            actions[key] = act
            act.changed = lambda k=key: _reflect(win, k)
        act = actions[key]
        old = act.property(shortcuts._DEFAULTS)
        sequences = [parsed[0] for s in defaults if (parsed := shortcut_text(s))]
        portable = "; ".join(sequences)
        if old != portable:
            current = "; ".join(act.portable_keys())
            act.setProperty(shortcuts._DEFAULTS, portable)
            if old is None or current == old:
                act.setShortcuts(sequences)
        act.setProperty("nativeMenuId", menu_id)
        if menu_id in enabled:
            act.setText(label)
            act.setEnabled(enabled[menu_id])
        elif menu_id in native_items:
            act.setEnabled(native_items[menu_id].isEnabled())
        previous.discard(key)
    for key in previous:
        dead = actions.pop(key)
        dead.remove()
    win._command_paths = paths
    shortcuts.apply_user_shortcuts(win)
    for key in actions:
        _reflect(win, key)
    if old_keys != actions.keys():
        from views import chrome_sheet
        sheet = chrome_sheet.current(win)
        if getattr(sheet, "id", None) == "settings":
            sheet._actions = sheet._paths = None
            chrome_sheet.refresh(win)


def _run(win, key):
    menu_id = win._registry_actions[key].property("nativeMenuId")
    item = win._menu_items.get(menu_id)
    if key == "command_search":
        win._host.view.renderer.ui_open_commands()
        win._host.view.update()
    elif key.startswith("tool:"):
        win._activate_tool(key[5:])
    elif key.startswith("view:"):
        from views.view_commands import run
        run(win, key[5:])
    elif item is not None:
        item.trigger()
    elif menu_id:
        win.run_command(menu_id)


def _reflect(win, key):
    act = win._registry_actions[key]
    keys = act.portable_keys()
    portable = keys[0] if keys else ""
    menu_id = act.property("nativeMenuId")
    if menu_id and menu_id.startswith("extension:"):
        win._extension_menus.bind_shortcut(menu_id, portable)
    item = win._menu_items.get(menu_id)
    if item is not None:
        factory = shortcut_text(item.keys)
        item.set_shortcut(portable, portable == (factory[0] if factory else ""))
    if key.startswith("view:"):
        bindings = win.__dict__.setdefault("_view_bindings", {})
        value = (shortcut_text(portable)[1] if keys else "", act.isEnabled())
        if bindings.get(key[5:]) != value:
            bindings[key[5:]] = value
            win._view_binding_version = vars(win).get("_view_binding_version", 0) + 1
