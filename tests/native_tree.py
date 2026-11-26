"""Drive the native sidebar wire commands and read the rows sent to egui."""
from types import SimpleNamespace

from views import wgpu_ui


def action(host, verb, key, *args):
    wgpu_ui._tree_action(host.window, host.view, wgpu_ui.SEP.join((verb, key, *args)))


def group_rows(host, group):
    from unittest.mock import patch

    rows = []
    output = SimpleNamespace(ui_set_tree_group_rows=lambda _, entries: rows.extend(entries))
    with patch.object(host.view, "renderer", output):
        action(host, "grouprows", "i:" + group.uid)
    return {label: why for _, label, why in rows}


def inspector(host, selection):
    from unittest.mock import patch

    sections = []
    output = SimpleNamespace(ui_set_identity=lambda value: None,
                             ui_set_inspector=lambda _, entries, subtitle=None: sections.extend(entries))
    host.viewport.scene.select(selection)
    with patch.object(host.view, "renderer", output):
        host.view._ui_insp_sig = None
        wgpu_ui._inspector(host.view, host.window)
    return {title: dict(rows) for title, rows in sections}


def tag_picker(host, selection):
    """Read the selected stable key and the choices provided to the native inspector."""
    import json
    sections = inspector(host, selection)
    for rows in sections.values():
        value = rows.get("Tag", "")
        if value.startswith("\x1dtagpicker\x1d"):
            index, choices = json.loads(value.split("\x1d", 2)[2])
            return choices[index][0], dict(choices)
    return None


class FilingRows:
    """Compare the displayed Filing rows with an independently built pane."""

    def __init__(self, host, expanded=()):
        from tests.test_tree_links import _View
        self.window = host.window
        self.view = _View()
        self.view._ui_exp = {"n:", *expanded}

    def read(self, fresh=False):
        from tests.test_tree_links import _View
        view = self.view
        if fresh:
            view = _View()
            view._ui_exp = self.view._ui_exp.copy()
        wgpu_ui._tree(view, self.window)
        return view.renderer.calls["ui_set_tree"][0]

    @property
    def model(self):
        return self.view._ui_tree_model[1]
