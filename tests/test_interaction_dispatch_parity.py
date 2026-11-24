"""Document interaction policies shared by the controller and viewport facades."""
from types import SimpleNamespace

from views.document_interaction import DocumentInteraction as Interaction


def test_extension_pick_notifies_before_checking_document_version():
    calls = []
    view = SimpleNamespace(
        scene=SimpleNamespace(version=1),
        extension_pick=None,
        update=lambda: calls.append("redraw"),
    )

    def selected(item):
        calls.append(item)
        if item is None:
            view.scene.version += 1

    entry = {"pick": lambda *_: 0, "on_select": selected, "delete": calls.append}
    view._ext_pickables = [{"pick": lambda *_: None}, entry]
    view.clear_extension_pick = lambda: Interaction.clear_extension_pick(view)
    hit = Interaction.pick_extension_item(view, 6, 3)
    assert hit == (entry, 0)
    Interaction.set_extension_pick(view, hit)
    assert Interaction.delete_extension_pick(view)
    assert view.extension_pick is None
    assert calls == [0, "redraw", None]
