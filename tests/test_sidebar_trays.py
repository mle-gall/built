"""The Project dock restores the widths of its Filing and wide lenses."""
from types import SimpleNamespace

from ingetrazo_core import NativeSettings

from views.wgpu_ui import WIDE_DOCK, apply_dock_width, keep_dock_width


def test_first_run_uses_the_default_width_for_each_lens(tmp_path):
    widths = []
    view = SimpleNamespace(renderer=SimpleNamespace(ui_set_dock_width=widths.append))
    store = NativeSettings(str(tmp_path))
    for view._ui_lens in (0, 1, 2):
        apply_dock_width(view, store)
    assert widths == [300., WIDE_DOCK, WIDE_DOCK]


def test_the_grip_widths_return_after_reopening_settings(tmp_path):
    view = SimpleNamespace(_ui_lens=0)
    store = NativeSettings(str(tmp_path))
    keep_dock_width(view, "335", store)
    view._ui_lens = 2
    keep_dock_width(view, "520", store)
    store.sync()

    widths = []
    view.renderer = SimpleNamespace(ui_set_dock_width=widths.append)
    reopened = NativeSettings(str(tmp_path))
    for view._ui_lens in (2, 0, 1):
        apply_dock_width(view, reopened)
    assert widths == [520., 335., 520.]
