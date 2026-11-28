"""Imported steel profile packs in Library › Profiles: collections named by
family, provisional status and source in the details, the sweep chip."""
from tests import addon_cases
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_render import wgpu_library as L
from tests.test_library_destinations import Renderer, win  # noqa: F401


def test_pack_sizes_list_under_family_names_with_their_status_and_source(win, tmp_path):
    addon_cases.install(win, "steel-profiles-en", tmp_path)
    items = L.ProfilesProvider().items(L.Ctx(win, win.view))
    families = {row["category"] for row in items}
    assert {"I and H sections", "Channels", "Angles", "Round tubes", "Rectangular and square tubes"} <= families
    assert not families & {"i", "channel", "angle", "chs", "rhs", "zed", "round"}
    ipn = next(row for row in items if row["name"] == "IPN 300")
    details = {k: v for k, v, _ in ipn["defaults"]}
    assert details["Status"] == "Provisional" and details["Standard"] == "EN 10365"
    assert details["Source"].startswith("https://") and details["slope"] == "14 %"
    assert details["Mass"].endswith("kg/m") and details["Area"].endswith("cm²")

    # Using a size arms Sweep with the size as its own preset: named once.
    win._activate_tool("sweep")
    machine = win._tools["sweep"]._machine
    machine.set("preset", ipn["profile_preset"])
    chip = win._tools["sweep"]._state("chip")
    assert chip.startswith("IPN 300 · ") and chip.count("IPN 300") == 1, chip
