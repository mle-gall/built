"""Observable results captured from the Python core modules before their port."""
from datetime import datetime, timezone
import json
from pathlib import Path

from ingetrazo_core import Vec3


def bulk_results():
    from core import autosave, axes, geometry, guide, materials, structure, style, sun, triangulate
    from core.textlabel import TextLabel

    ring = [Vec3(x, y, 0) for x, y in [(0, 0), (4, 0), (4, 3), (2, 1), (0, 3)]]
    face = geometry.Face(ring)
    node = {"structure": {"nodes": {
        "a": {"parent": None, "name": "Envelope", "order": 0,
              "recipe": {"systems": ["A"], "psets": {"P": {"a": 1}}}},
        "b": {"parent": "a", "name": "Wall", "order": 0,
              "recipe": {"systems": ["A", "B"], "psets": {"P": {"b": 2}}}},
    }}}
    when = datetime(2026, 6, 21, 12, tzinfo=timezone.utc)
    return {
        "plane_axes": [v.toTuple() for v in triangulate.plane_axes(Vec3(2, 3, 5).normalized())],
        "geometry": {"normal": face.normal().toTuple(), "area": face.area(),
                     "centroid": face.centroid().toTuple(),
                     "indices": [[ring.index(v) for v in t] for t in face.triangulate()],
                     "edge_length": geometry.Edge(ring[0], ring[2]).length()},
        "label": TextLabel(ring[0], ring[2], "Wall", layer="Notes").to_dict(),
        "guides": [g.to_dict() for g in [guide.Guide(ring[1], direction=Vec3(3, 4, 0)),
                                        guide.GuideCircle(ring[2], Vec3(0, 0, 1), 2)]],
        "structure": structure.resolve(node, "b"),
        "styles": [preset.to_dict() for preset in style.BUILTIN_STYLES],
        "solar_position": sun.solar_position(48.86, 2.35, when),
        "daylight": [sun.daylight_minutes(lat, 2.35, 12, 21) for lat in (0, 49, 89)],
        "shadows": sun.ShadowSettings().to_dict(),
        "frame_matrix": axes.frame_matrix(Vec3(3, 4, 5), Vec3(0, 1, 0),
                                          Vec3(-1, 0, 0), Vec3(0, 0, 1)).data(),
        "material": materials.Material.from_dict({"name": "Oak", "color": [0.4, 0.2, 0.1]}).to_dict(),
        "recovery_slots": [autosave.slot_for(None, n).name for n in (0, 1, 3)],
    }


def service_results(monkeypatch, tmp_path):
    import sys
    from types import SimpleNamespace
    from core import paths, settings, timers, version

    calls = []

    class Queue:
        def timer(self, callback, interval, repeat, owner):
            calls.append(["timer", callback(), interval, repeat, owner is window])
            return "handle"

        def later(self, owner, delay, callback):
            calls.append(["later", callback(), delay, owner is window])

    window = SimpleNamespace(_host=SimpleNamespace(timers=Queue()))
    handle = timers.timer(window, lambda: "tick", 15, True)
    later = timers.later(window, 35, lambda: "idle")

    monkeypatch.setattr(sys, "_MEIPASS", str(tmp_path), raising=False)
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path))
    # The embedding host may already have selected a store in the suite.
    with monkeypatch.context() as mp:
        mp.setattr(settings, "_store", None)
        first = settings.settings()
        first.beginGroup("parity")
        first.setValue("count", 17)
        second = settings.settings()
        root_value = second.value("count", "missing")
        second.beginGroup("parity")
        grouped_value = second.value("count")
    return {"timer": [calls, handle, later], "profile": [root_value, grouped_value],
            "bundle": [paths.is_frozen(), paths.app_root() == tmp_path],
            "agent_contains_version": version.USER_AGENT.startswith("IngeTrazo/" + version.__version__ + " ")}


def test_bulk_services_python_golden(monkeypatch, tmp_path):
    fixture = Path(__file__).with_name("fixtures") / "commands_services_python.json"
    assert service_results(monkeypatch, tmp_path) == json.loads(fixture.read_text())


def test_bulk_core_python_golden():
    fixture = Path(__file__).with_name("fixtures") / "commands_bulk_python.json"
    assert json.loads(json.dumps(bulk_results())) == json.loads(fixture.read_text())
