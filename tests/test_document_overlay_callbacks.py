"""Extension overlays may change their registration during a paint pass."""
from types import SimpleNamespace

from views.document_interaction import DocumentInteraction


def test_a_self_removing_overlay_finishes_the_current_pass(caplog):
    calls = []
    painter = SimpleNamespace(save=lambda: calls.append("save"),
                              restore=lambda: calls.append("restore"))
    view = SimpleNamespace(_ext_overlays=[])

    def next_frame(viewport, context):
        calls.append("next")

    def failing(viewport, context):
        viewport._ext_overlays.remove(failing)
        viewport._ext_overlays.append(next_frame)
        raise ValueError("drawing failed")

    def survivor(viewport, context):
        assert viewport is view and context is painter
        calls.append("survivor")

    view._ext_overlays.extend([failing, survivor])
    DocumentInteraction._draw_extension_overlays(view, painter)
    assert calls == ["save", "restore", "save", "survivor", "restore"]
    assert view._ext_overlays == [survivor, next_frame]
    assert len(caplog.records) == 1
    assert isinstance(caplog.records[0].exc_info[1], ValueError)
    calls.clear()
    DocumentInteraction._draw_extension_overlays(view, painter)
    assert calls == ["save", "survivor", "restore", "save", "next", "restore"]
