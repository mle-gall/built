# SPDX-License-Identifier: GPL-3.0-or-later
"""The Settings pages and Window → Units reach the shared large sheet.

The three screen-size cases live with the renderer in ui/large_sheet.rs.
"""
from tests.preferences_chrome import Controls
from views import chrome_sheet


def test_the_pages_are_all_there_and_units_still_opens_on_its_page():
    from views.main_window import MainWindow
    win = MainWindow(defer_extensions=True)
    try:
        ui = Controls(win, "general")
        side = ui.wire()["side"]
        assert {row["id"] for row in side if row.get("id")} == {
            "general", "navigation", "display", "ndof", "keyboard", "import",
            "units", "dimensions", "location"}
        win._on_units()
        assert chrome_sheet.current(win).section == "units"
    finally:
        win._saved_version = win.viewport.scene.version
        win.close()
