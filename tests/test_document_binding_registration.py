"""The two hosts and extensions can call the controller through the same names."""
import inspect
import weakref
from types import MethodType, SimpleNamespace

import pytest
import built_core
import built_render

from views.document_interaction import DocumentInteraction as Controller


def test_host_kernel_binding_accepts_another_surface_and_defaults(monkeypatch):
    owner = SimpleNamespace()
    calls = []
    monkeypatch.setattr(built_core, "document_pick_index",
                        lambda *args: calls.append(args) or "index")
    assert Controller._pick_index(self=owner) == "index"
    assert Controller._pick_index(owner, near=("px", 4, 5)) == "index"
    assert calls == [(owner, None), (owner, ("px", 4, 5))]
    with pytest.raises(TypeError):
        Controller._pick_index(owner, None, near=None)
    with pytest.raises(TypeError):
        Controller._pick_index(owner, surprise=True)


def test_binding_descriptors_allow_subclasses_instances_and_weak_callbacks(monkeypatch):
    class Surface(Controller):
        def _pick_index(self, near=None):
            return super()._pick_index(near)

    view = object.__new__(Surface)
    monkeypatch.setattr(built_core, "document_pick_index", lambda receiver, near: near)
    assert view._pick_index(near="override") == "override"
    original = Controller._pick_index.__get__(view, Surface)
    callback = weakref.WeakMethod(original)
    assert callback()("weak") == "weak"
    view._pick_index = lambda: "instance"
    assert view._pick_index() == "instance"


def test_class_and_static_helpers_keep_their_receiver(monkeypatch):
    class Surface(Controller):
        pass

    view = object.__new__(Surface)
    marker = object()
    monkeypatch.setattr(built_render, "tool_busy_for_delete", lambda cls, tool: (cls, tool))
    monkeypatch.setattr(built_render, "tool_busy", lambda tool: tool)
    assert view._tool_busy(marker) is Surface._tool_busy(marker) is marker
    assert Surface._tool_busy_for_delete(marker) == (Surface, marker)
    assert view._tool_busy_for_delete(marker) == (Surface, marker)


def test_plugin_pick_uses_public_coordinates_and_bound_signature(monkeypatch):
    class Surface(Controller):
        pass

    view = object.__new__(Surface)
    method = view.pick_text_label
    assert isinstance(method, MethodType)
    assert method.__name__ == "pick_text_label"
    assert method.__qualname__ == "DocumentInteraction.pick_text_label"
    assert str(inspect.signature(method)) == "(screen_x: 'float', screen_y: 'float', rect_only: 'bool' = False)"
    monkeypatch.setattr(built_render, "pick_text_label", lambda *a: a)
    assert method(screen_y=2, screen_x=1) == (view, 1, 2, False)
    assert Controller.pick_text_label(view, 1, 2, rect_only=True) == (view, 1, 2, True)


def test_plugin_clipboard_option_stays_keyword_only(monkeypatch):
    view = SimpleNamespace()
    monkeypatch.setattr(built_render, "copy_selection", lambda *args: args)
    assert Controller.copy_selection(view) == (view, True)
    assert Controller.copy_selection(view, publish=False) == (view, False)
    with pytest.raises(TypeError):
        Controller.copy_selection(view, False)


def test_host_registers_defaults_and_independent_inference_fields():
    from views import document_interaction as module

    assert type(Controller.CENTER_HINT_PX) is float
    assert Controller._SHADOW_MAP_SIZE == Controller._shadow_size == 2048
    assert Controller._SNAP_LABELS["intersection_edge"] == "Intersect with Edge"
    assert isinstance(Controller._ENCOURAGING_KINDS, frozenset)
    assert module._SHARED_BAKES[0] == "_group_chunks"
    assert module._SnapEdge is built_render.SnapEdge
    assert vars(Controller)["_bind_bake_store"] is module._bind_bake_store
    assert module._AXIS_DIRS["z"] == (0., 0., 1.)
    view = object.__new__(Controller)
    view.inference = SimpleNamespace()
    view.last_snap = object()
    assert view.inference.last_snap is view.last_snap


def test_tool_pick_adapters_respect_override_keywords_and_face_context(monkeypatch):
    sink = []

    def pick(x, y, **opts):
        return x, y, opts

    view = SimpleNamespace(_pick_edge=pick, linework_hidden=sink.append)
    assert Controller.pick_edge(view, 2, 3) == (2, 3, {"visible_only": False})
    assert Controller.pick_linework_edge(view, 2, 3)[2] == {
        "visible_only": True, "hidden": sink.append, "with_distance": True,
    }
    def hit(receiver, x, y, kind):
        assert receiver is view
        return x, "owner", kind, "mesh"

    monkeypatch.setattr(Controller, "_face_any", hit)
    assert Controller.pick_face_any(view, 1, 2) == (1, "owner")
    assert Controller.pick_face_placement(view, None, 2) == (None, None)
    assert Controller.pick_face_inference(view, 1, 2) == (1, "infer")


def test_plugin_projection_and_edit_adapters_preserve_order():
    calls = []
    view = SimpleNamespace(
        camera=SimpleNamespace(set_projection=calls.append),
        _sync_projection=lambda: calls.append("sync"),
        update=lambda: calls.append("update"),
        scene=SimpleNamespace(bump_view=lambda: calls.append("bump")),
        notify_scene_changed=lambda: calls.append("notify"),
        _project_px=lambda data: (data.shape, data.dtype.name),
    )
    Controller.set_projection(view, "parallel")
    Controller.refresh_after_edit(view)
    assert calls == ["parallel", "sync", "update", "bump", "notify"]
    assert Controller.world_to_pixels(view, [1, 2, 3]) == ((1, 3), "float64")


def test_python_can_pickle_references_to_exported_callables():
    import copy
    import pickle

    from views.document_interaction import _bind_bake_store

    for method in (Controller.pick_edge, Controller.pick_text_label, _bind_bake_store):
        assert pickle.loads(pickle.dumps(method)) is method
        assert copy.deepcopy(method) is method
