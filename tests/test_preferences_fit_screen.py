# SPDX-License-Identifier: GPL-3.0-or-later
"""The Settings pages and Window → Units reach the shared large sheet.

The three screen-size cases live with the renderer in ui/large_sheet.rs.
"""
from tests.preferences_chrome import Controls
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_render import chrome_sheet


def test_the_pages_are_all_there_and_units_still_opens_on_its_page(native_host):
    win = native_host.window
    ui = Controls(win, "general")
    side = ui.wire()["side"]
    assert {row["id"] for row in side if row.get("id")} == {
        "general", "navigation", "display", "ndof", "keyboard", "import", "ai",
        "units", "dimensions", "appearance", "location"}
    win._on_units()
    assert chrome_sheet.current(win).section == "units"
