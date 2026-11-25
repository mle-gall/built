"""Host behavior shared by scripted viewports, Qt and device connections."""
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
