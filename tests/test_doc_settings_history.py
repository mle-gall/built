from types import SimpleNamespace

import pytest

from core.history import History
from core.protocol.commands import run
from core.scene import Scene
from views.wgpu_ui import _sync_units


@pytest.mark.parametrize("command,args,fields", [
    ("set_model_units", {"units": {"length": "mm", "precision": 3}}, {"units", "dimension_style"}),
    ("set_dimension_style", {"key": "ends", "value": "tick"}, {"dimension_style"}),
    ("set_annotation_font", {"family": "Helvetica", "size": 14, "bold": False, "italic": True},
     {"dimension_style"}),
])
def test_document_settings_history(command, args, fields):
    scene = Scene()
    history = History(scene)
    initial = dict(scene.units), dict(scene.dimension_style)
    scene.worksession_read_only = True
    refused = run(history, command, **args)
    assert refused["rejected"] and refused["step"] is None
    assert (scene.units, scene.dimension_style) == initial
    scene.worksession_read_only = False
    edit = run(history, command, **args)
    assert edit["ok"] and set(edit["step"].scene_fields) == fields
    changed = dict(scene.units), dict(scene.dimension_style)
    if command == "set_model_units":
        assert scene.units == args["units"]
        assert (scene.dimension_style["units"], scene.dimension_style["decimals"]) == ("mm", 3)
    elif command == "set_dimension_style":
        assert scene.dimension_style["ends"] == "tick"
    else:
        assert [scene.dimension_style[k] for k in ("font_family", "font_size", "font_bold", "font_italic")] == [
            "Helvetica", 14, False, True]
    assert history.undo() and (scene.units, scene.dimension_style) == initial
    assert history.redo() and (scene.units, scene.dimension_style) == changed


def test_renderer_suffix_tracks_published_steps():
    scene = Scene()
    history = History(scene)
    labels = []
    view = SimpleNamespace(renderer=SimpleNamespace(ui_set_units=labels.append))
    _sync_units(view, history)
    run(history, "set_model_units", units={"length": "ft", "precision": 2})
    _sync_units(view, history)
    _sync_units(view, history)
    history.undo()
    _sync_units(view, history)
    history.redo()
    _sync_units(view, history)
    assert labels == ["m", "'", "m", "'"]
    # Opening a file in the same tab replaces the settings and clears History.
    scene.units = {"length": "cm", "precision": 2}
    history.clear()
    _sync_units(view, history)
    assert labels[-1] == "cm"
