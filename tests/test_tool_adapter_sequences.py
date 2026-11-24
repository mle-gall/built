"""Seeded event sequences for the plugin-facing adapters and actual Push/Pull meshes."""
import gzip
import json
import random
from pathlib import Path

import pytest

from tests import tool_parity as parity

GOLDEN = Path(__file__).parent / "fixtures" / "tool_adapter_sequences.json.gz"


def push_sequence(seed):
    from tests import test_pushpull_ux as push

    rng = random.Random(seed)
    scene = push.Scene()
    push._cube(scene, push.History(scene), size=rng.randint(2, 8), height=3)
    face = push._top(scene, 3)
    view = push._DragViewport(scene, pick=face)
    tool = push.PushPullTool()
    tool.hovered_face = face
    ctx = lambda x: push._px(view, x, 100)
    steps = []

    def capture():
        # Include face orientation, wall seams and coincident-face multiplicity.
        def ring(face):
            pts = [tuple(round(n, 8) for n in v.toTuple()) for v in face.vertices]
            return min(pts[i:] + pts[:i] for i in range(len(pts)))
        steps.append([tool.busy(), round(tool.extrusion, 8), tool._keep_mode,
                      len(view.history.undo_stack), sorted(ring(f) for f in scene.faces)])

    tool.on_click(ctx(100))
    capture()
    for _ in range(4):
        tool.on_hover(ctx(rng.randrange(30, 280)))
        capture()
    if seed % 3 == 0:
        tool.on_cancel(view)
    elif seed % 3 == 1:
        tool.on_release(view)
    else:
        tool.on_value(view, rng.choice([-.5, 1.25, 2.0]))
    capture()
    tool.on_value(view, 0.75)
    capture()
    view.history.undo()
    capture()
    view.history.redo()
    capture()
    return steps


def adapter_sequences():
    from tests import remaining_tool_parity as rest
    from tests import test_texture_position_parity as texture

    scenarios = {}
    for tag in ("line", "select", "rect", "rot", "arc", "circle", "move", "scale",
                "rotate", "walkthrough", "offset", "paint", "protractor", "fillet",
                "tape", "paste", "flip", "change_axes", "followme", "freehand"):
        cases = getattr(parity, tag.upper() + "_SCENARIOS")
        run = getattr(parity, "run_" + tag)
        for name, spec in cases.items():
            prefix, steps = (), spec
            if tag == "walkthrough":
                prefix, steps = (spec[0],), spec[1]
            elif tag == "arc":
                prefix = (name,)
            scenarios[tag + "/" + name] = run, prefix, steps
    for tag, cases in rest.CASES.items():
        for name, steps in cases.items():
            scenarios[tag + "/" + name] = rest.run, (tag,), steps
    for name, steps in texture.SCENARIOS.items():
        scenarios["texture/" + name] = texture.replay, (), steps
    return scenarios


def replay_adapter(name):
    run, prefix, steps = adapter_sequences()[name]
    rng = random.Random(name)
    # Park twice on some positions; occasionally abandon a gesture before its last step.
    steps = list(steps)
    limit = rng.randint(1, len(steps))
    events = []
    for event in steps[:limit]:
        op = event if isinstance(event, str) else event[0]
        events.extend([event] * (rng.randint(1, 4) if op == "hover" else 1))
    return json.loads(json.dumps(run(*prefix, events)))


@pytest.fixture(scope="module")
def recorded():
    with gzip.open(GOLDEN, "rt") as stream:
        return json.load(stream)


@pytest.mark.parametrize("seed", range(12))
def test_push_adapter_state_and_topology(seed, recorded, monkeypatch):
    from tools.pushpull import PushPullTool
    monkeypatch.setattr(PushPullTool, "last_distance", None)
    assert json.loads(json.dumps(push_sequence(seed))) == recorded["push"][seed]


@pytest.mark.parametrize("name", sorted(adapter_sequences()))
def test_adapter_event_prefix(name, recorded):
    assert replay_adapter(name) == recorded["adapters"][name]


def test_plugin_can_connect_and_disconnect_a_bound_native_tool_slot():
    from PySide6.QtCore import QObject, Signal
    from tools.rust_tools import SelectTool

    class Events(QObject):
        hover = Signal(object, int, int, int)

    class RecordingSelect(SelectTool):
        def _px(self, viewport, kind, x, y, modifiers=0, t=0.0):
            calls.append((viewport, kind, x, y, modifiers))

    tool, events, calls = RecordingSelect(), Events(), []
    events.hover.connect(tool.hover_at)
    events.hover.emit(events, 7, 11, 3)
    events.hover.disconnect(tool.hover_at)
    events.hover.emit(events, 8, 12, 4)
    assert calls == [(events, "hover", 7, 11, 3)]
