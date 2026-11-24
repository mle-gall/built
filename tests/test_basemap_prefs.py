# SPDX-License-Identifier: GPL-3.0-or-later
"""Named XYZ sources in Settings → This model → Location."""
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from tests.preferences_chrome import Controls
from ingetrazo_render.chrome_forms import current

URL = "https://tiles.example.com/drone/{z}/{x}/{y}.png"


def controls(host):
    return Controls(host.window, "location")


def add(c, name, url):
    c.row("basemap_add").press(c.env)
    form = current(c.env.win)
    form.receive("name", name)
    form.receive("url", url)
    form.press("ok")
    return form


def test_named_source_appears_forever_in_the_menu(native_question_host):
    c = controls(native_question_host)
    assert add(c, "Dron Yanque", URL).closed
    fresh = controls(native_question_host)
    assert fresh.value("basemap_source") == "custom-dron-yanque"
    assert fresh.options("basemap_source")["custom-dron-yanque"] == "Dron Yanque"
    import json
    assert json.loads(fresh.env.st.value("basemap/custom_sources")) == [{"name": "Dron Yanque", "url": URL}]


def test_several_sources_coexist_and_upsert_by_name(native_question_host):
    c = controls(native_question_host)
    add(c, "Dron Yanque", URL)
    add(c, "IGN Perú", "https://ign.example/{z}/{x}/{y}")
    newer = "https://v2.example/{z}/{x}/{y}"
    add(c, "Dron Yanque", newer)
    import json
    assert {e["name"]: e["url"] for e in json.loads(c.env.st.value("basemap/custom_sources"))} == {
        "Dron Yanque": newer, "IGN Perú": "https://ign.example/{z}/{x}/{y}"}
    assert {k for k in c.options("basemap_source") if k.startswith("custom-")} == {"custom-dron-yanque", "custom-ign-per"}


def test_url_without_placeholders_is_rejected(native_question_host):
    c = controls(native_question_host)
    form = add(c, "Mala", "https://example.com/tile.png")
    assert not form.closed and "{z}" in form.error
    assert not any(k.startswith("custom-") for k in c.options("basemap_source"))
    form.press("cancel")


def test_remove_forgets_the_source(native_question_host):
    c = controls(native_question_host)
    add(c, "Dron Yanque", URL)
    assert c.enabled("basemap_remove")
    c.row("basemap_remove").press(c.env)
    c = controls(native_question_host)
    assert "custom-dron-yanque" not in c.options("basemap_source")
    assert c.value("basemap_source") == "esri_imagery" and not c.enabled("basemap_remove")


def test_legacy_single_url_pref_migrates_to_a_named_source(native_question_host):
    c = controls(native_question_host)
    c.env.st.setValue("basemap/custom_url", URL)
    assert "XYZ personalizado" in c.options("basemap_source").values()
    assert c.env.st.value("basemap/custom_url", "") == ""
    assert URL in c.env.st.value("basemap/custom_sources")
