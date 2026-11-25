"""Host behavior shared by scripted viewports and device connections."""
from types import SimpleNamespace
import struct

from views import ndof_input


def test_spnav_split_packets_and_interleaved_buttons():
    events = []
    owner = SimpleNamespace(
        _emit_button=lambda *args: events.append(("button", args)),
        _emit_raw=lambda values: events.append(("raw", tuple(values))),
        _emit_motion=lambda sample, dt: events.append(("motion", sample, dt)),
    )
    backend = ndof_input.SpnavBackend(owner)
    packet = struct.Struct("=8i").pack
    data = (packet(0, 1, 2, 3, 4, 5, 6, 20) + packet(1, 8, 0, 0, 0, 0, 0, 0)
            + packet(0, 8, 7, 6, 5, 4, 3, 7) + packet(2, 8, 0, 0, 0, 0, 0, 0))
    fragments = iter((data[:5], data[5:117], data[117:]))
    backend.sock = SimpleNamespace(recv=lambda size: next(fragments))
    for _ in range(3):
        backend._read()
    assert events == [
        ("raw", (1, 2, 3, 4, 5, 6)), ("button", (8, True)),
        ("raw", (8, 7, 6, 5, 4, 3)),
        ("motion", ndof_input.from_spacenavd(8, 7, 6, 5, 4, 3), 0.027),
        ("button", (8, False)),
    ]


def test_context_pick_preserves_an_existing_selected_loose_edge():
    from unittest.mock import Mock
    from views.document_interaction import DocumentInteraction
    view = Mock()
    edge = object()
    view.scene.selection = {edge}
    view.pick_edge.return_value = edge
    assert DocumentInteraction._context_pick(view, 4., 2.) is edge
    view.pick_group.assert_not_called()
    view.scene.select.assert_not_called()
    view.update.assert_not_called()


def test_host_methods_accept_a_plugin_keyword_receiver():
    from inspect import signature
    from views.document_controller import DocumentController
    from views.view_host import WidgetShim

    resized = []
    display = SimpleNamespace(resize=lambda *size: resized.append(size))
    view = SimpleNamespace(keys_stand_in=display, _host=None)
    DocumentController.resize(view, 40, 24)
    WidgetShim.update(self=view)
    WidgetShim.setFocus(self=view)
    assert view._focus and resized == [(40, 24)]
    assert str(signature(DocumentController.resize)) == "(self, *size)"
    assert str(signature(WidgetShim._noop)) == "(self, *args, **kwargs)"
    assert str(signature(ndof_input.SpnavBackend._read)) == "(self, *_args) -> 'None'"


def test_view_classes_remain_importable_and_discoverable():
    import importlib
    import pickle
    import pkgutil
    import views

    modules = {item.name for item in pkgutil.iter_modules(views.__path__, "views.")}
    for leaf, name in [
        ("document_controller", "DocumentController"), ("document_tools", "DocumentTools"),
        ("document_interaction", "DocumentInteraction"),
        ("view_navigation", "ViewNavigation"), ("view_host", "WidgetShim"),
        ("ndof_input", "NdofInput"),
    ]:
        module = f"views.{leaf}"
        assert module in modules
        cls = getattr(importlib.import_module(module), name)
        assert cls.__module__ == module
        assert pickle.loads(pickle.dumps(cls)) is cls
    assert "views.view_commands" in modules


def test_coalesced_hover_uses_the_native_shell_event_and_plugin_codes():
    import pickle
    from ingetrazo_core import Input, ScreenPoint, Type
    from ingetrazo_render import MouseEvent
    from views.document_interaction import _HoverEvent

    flags = Input.ShiftModifier | Input.ControlModifier
    event = _HoverEvent(ScreenPoint(-4.25, 17.75), flags)
    assert isinstance(event, MouseEvent)
    assert event.type() is Type.MouseMove
    assert (event.position().x(), event.position().y()) == (-4.25, 17.75)
    assert isinstance(event.modifiers(), Input.KeyboardModifier)
    assert pickle.loads(pickle.dumps(event.modifiers())) == flags


def test_extension_import_paths_resolve_to_the_native_form_and_dialog_modules():
    from importlib import import_module
    from ingetrazo_render import chrome_dialogs, chrome_forms

    assert import_module("views.chrome_forms") is chrome_forms
    assert import_module("views.chrome_dialogs") is chrome_dialogs
