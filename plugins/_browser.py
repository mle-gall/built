"""Add extension-provided type cells to the native Project browser's rows."""
from ingetrazo_core import project_sidebar as native_project_sidebar


def project_sidebar(scene, tree, rows, *options):
    from ingetrazo_render.shell_windows import hosts
    from core import outliner

    rows, path = native_project_sidebar(scene, tree, rows, *options)
    providers = next((vars(host.window).get("_outliner_kinds", ())
                      for host in hosts() if host.viewport.scene is scene), ())
    if not providers:
        return rows, path

    # The Rust tree already qualifies repeated UIDs by their occurrence path.
    # Keep the lookup with that tree across selection and disclosure changes.
    stamp = scene.content_version, tuple(map(id, providers))
    if getattr(tree, "_plugin_type_stamp", None) != stamp:
        types = {}
        for group, _, _ in outliner.walk(scene.groups):
            result = outliner.custom_kind(group, providers)
            if result:
                tip = "\n".join(f"{key}: {value}" for key, value in result["props"])
                types[tree.row_id(group)] = result["type"] + "\x1e" + tip
        tree._plugin_type_cells = types
        tree._plugin_type_stamp = stamp

    for text, _ in rows:
        kind, _, address = text[0].partition(":")
        if kind in ("i", "c", "g") and address in tree._plugin_type_cells:
            text[2] = tree._plugin_type_cells[address]
    return rows, path
