"""The two hosts and extensions can call the controller through the same names."""
import inspect
import weakref
from types import MethodType, SimpleNamespace

import pytest
import ingetrazo_core
import ingetrazo_render

from views.document_interaction import DocumentInteraction as Controller


def test_host_kernel_binding_accepts_another_surface_and_defaults(monkeypatch):
    owner = SimpleNamespace()
    calls = []
    monkeypatch.setattr(ingetrazo_core, "document_pick_index",
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
    monkeypatch.setattr(ingetrazo_core, "document_pick_index", lambda receiver, near: near)
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
    monkeypatch.setattr(ingetrazo_render, "tool_busy_for_delete", lambda cls, tool: (cls, tool))
    monkeypatch.setattr(ingetrazo_render, "tool_busy", lambda tool: tool)
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
    monkeypatch.setattr(ingetrazo_render, "pick_text_label", lambda *a: a)
    assert method(screen_y=2, screen_x=1) == (view, 1, 2, False)
    assert Controller.pick_text_label(view, 1, 2, rect_only=True) == (view, 1, 2, True)


def test_plugin_clipboard_option_stays_keyword_only(monkeypatch):
    view = SimpleNamespace()
    monkeypatch.setattr(ingetrazo_render, "copy_selection", lambda *args: args)
    assert Controller.copy_selection(view) == (view, True)
    assert Controller.copy_selection(view, publish=False) == (view, False)
    with pytest.raises(TypeError):
        Controller.copy_selection(view, False)
