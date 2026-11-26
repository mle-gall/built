"""The Project dock keeps one width, the user's, under every column preset."""
from types import SimpleNamespace

from built_core import NativeSettings

from views.wgpu_ui import apply_dock_width, dock_width, keep_dock_width


def test_first_run_uses_the_default_width_under_every_preset(tmp_path):
    widths = []
    view = SimpleNamespace(renderer=SimpleNamespace(ui_set_dock_width=widths.append))
    store = NativeSettings(str(tmp_path))
    for view._ui_lens in (0, 1, 2):
        apply_dock_width(view, store)
    assert widths == [300., 300., 300.]


def test_the_grip_width_is_one_value_and_returns_after_reopening_settings(tmp_path):
    view = SimpleNamespace(_ui_lens=0)
    store = NativeSettings(str(tmp_path))
    keep_dock_width(view, "335", store)
    # Dragged again with the Export columns showing: the same one width.
    view._ui_lens = 2
    keep_dock_width(view, "520", store)
    store.sync()

    widths = []
    view.renderer = SimpleNamespace(ui_set_dock_width=widths.append)
    reopened = NativeSettings(str(tmp_path))
    for view._ui_lens in (2, 0, 1):
        apply_dock_width(view, reopened)
    assert widths == [520., 520., 520.]
    assert [dock_width(lens, reopened) for lens in (0, 1, 2)] == [520., 520., 520.]
