# SPDX-License-Identifier: GPL-3.0-or-later
"""Settings as a large sheet (ingetrazo_render.settings_sheet): its sections and
their two groups, the search across every section, changes applied at
once (native preferences for Built, the document for This model), the 3D mouse and
keyboard sections, and offscreen command dispatch.

A stand-in renderer records the sheet sent by the offscreen host."""
from __future__ import annotations

import json

import pytest
from ingetrazo_render.shell_files import do_save
from core.settings import settings as preferences
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from core import units
from core.scene import Scene
from formats import igz
from ingetrazo_render import chrome_forms
from ingetrazo_render import chrome_sheet as cs
from ingetrazo_render import settings_sheet as ss

SEP = cs.SEP


@pytest.fixture(autouse=True)
def _throwaway_settings(monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", "0")
    ss._PENDING.clear()
    yield
    ss._PENDING.clear()


class FakeRenderer:
    def __init__(self):
        self.sheets = []
        self.icons = []

    def ui_set_large_sheet(self, text):
        cs.check_wire(text)       # what the chrome reads (ui/large_sheet.rs)
        self.sheets.append(text)

    def ui_set_icons(self, icons):
        self.icons.extend(i[0] for i in icons)


class FakeView:
    def __init__(self):
        self.renderer = FakeRenderer()

    def update(self):
        pass

    def setFocus(self, *_):
        pass


@pytest.fixture
def win(monkeypatch, native_host):
    w = native_host.window
    view = FakeView()
    monkeypatch.setattr(cs, "_view", lambda _w: view)
    w._fake_view = view
    return w


def frame(win) -> dict:
    view = win._fake_view
    cs.before_frame(view, win)
    sent = view.renderer.sheets[-1]
    return json.loads(sent) if sent else {}


def act(win, *parts):
    cs.action(win, SEP.join(("settings",) + tuple(str(p) for p in parts)))
    return frame(win)


def fields(sheet, pane="content") -> dict:
    """{field id: wire row} of every fields block of a pane."""
    out = {}
    for b in (sheet.get(pane) or {}).get("blocks", []):
        if b["kind"] == "fields":
            for row in b["fields"]:
                out[row[0]] = row
    return out


def captions(sheet) -> list:
    return [b["text"] for b in sheet["content"]["blocks"] if b["kind"] == "caption"]


# ---- Sections ---------------------------------------------------------------------
def test_sections_in_two_groups(win):
    win.open_preferences()
    sheet = frame(win)
    assert sheet["id"] == "settings" and sheet["search"]["placeholder"]
    side = sheet["side"]
    labels = [(r.get("kind", ""), r["label"]) for r in side]
    assert labels == [("caption", "Built · this Mac"), ("", "General"), ("", "Navigation"),
                      ("", "Display"), ("", "3D mouse"), ("", "Import"), ("", "Keyboard"), ("", "AI assistant"),
                      ("caption", "This model"), ("", "Units"),
                      ("", "Appearance"), ("", "Dimension style"), ("", "Location & north")]
    assert [r["id"] for r in side if r.get("on")] == ["general"]
    assert "theme" not in fields(sheet) and "autosave" in fields(sheet)
    # No OK/Cancel anywhere: the footer restores defaults, that is all.
    assert [l["id"] for l in sheet["footer"]["links"]] == ["restore"]
    assert "apply at once" in sheet["footer"]["hint"]


def test_side_picks_a_section(win):
    win.open_preferences()
    frame(win)
    sheet = act(win, "side", "navigation")
    assert sheet["content"]["title"] == "Navigation"
    assert captions(sheet) == ["Mouse", "Trackpad", "Tools"]
    got = fields(sheet)
    assert {"invert_wheel", "invert_orbit", "look_sens", "trackpad_slide",
            "cursor_style", "ask_section"} <= set(got)
    assert {"touch_slide", "trackpad_inertia", "pinch_sensitivity"} <= set(got)
    # Field ids are unique in the whole sheet.
    ids = [r[0] for b in sheet["content"]["blocks"] if b["kind"] == "fields" for r in b["fields"]]
    assert len(ids) == len(set(ids))


def test_every_section_builds_with_unique_field_ids(win):
    win.open_preferences()
    frame(win)
    for sid, _label, _scope in ss.SECTIONS:
        sheet = act(win, "side", sid)
        assert sheet["content"]["title"], sid
        ids = [r[0] for b in sheet["content"]["blocks"] if b["kind"] == "fields" for r in b["fields"]]
        assert len(ids) == len(set(ids)), sid
        if sid != "keyboard":
            assert ids, sid


# ---- Search -----------------------------------------------------------------------
def test_search_lists_hits_from_every_section_with_their_names(win):
    win.open_preferences()
    frame(win)
    sheet = act(win, "search", "zoom")
    caps = captions(sheet)
    sections = {c.split(" · ")[0] for c in caps}
    assert "Navigation" in sections          # «Invert wheel zoom», «Zooms the view»
    assert "3D mouse" in sections            # «Pan and zoom only», the axes' Zoom
    assert len(sections) >= 2
    got = fields(sheet)
    assert "invert_wheel" in got and "pinch_sensitivity" in got and "ndof_lock" in got
    # The side says how many rows each section holds, none of them picked.
    counts = {r["id"]: r.get("right", "") for r in sheet["side"] if r.get("kind") != "caption"}
    assert counts["navigation"] and counts["ndof"]
    assert not any(r.get("on") for r in sheet["side"])
    # A hit applies at once, like in its section.
    act(win, "v", "invert_wheel", "1")
    assert preferences().value("nav/invert_wheel") == "1"
    # Picking a section empties the search.
    sheet = act(win, "side", "display")
    assert sheet["content"]["title"] == "Display"
    assert sheet["search"]["text"] == "" and sheet["search"]["force"]


def test_search_finds_this_model_rows_and_commands(win):
    win.open_preferences()
    frame(win)
    sheet = act(win, "search", "decimal")
    sections = {c.split(" · ")[0] for c in captions(sheet)}
    assert {"Units", "Dimension style"} <= sections
    sheet = act(win, "search", "nothing-is-called-this")
    assert fields(sheet) == {}
    assert "No setting matches" in sheet["content"]["empty"]


# ---- Built · this Mac: at once ---------------------------------------------------
def test_a_built_setting_applies_at_once(win):
    win.open_preferences(page="navigation")
    sheet = frame(win)
    assert fields(sheet)["invert_wheel"][4] == 0.0
    sheet = act(win, "v", "invert_wheel", "1")
    assert preferences().value("nav/invert_wheel") == "1"
    assert all(v._invert_wheel for v in win.all_viewports())
    assert fields(sheet)["invert_wheel"][4] == 1.0
    # A choice comes as the index of its option.
    sheet = act(win, "side", "display")
    labels = fields(sheet)["rest_mode"][7]
    act(win, "v", "rest_mode", labels.index("Hide (fastest)"))
    assert all(v.edit_rest_mode == "hide" for v in win.all_viewports())
    # A number typed: undo steps reach the history at once.
    act(win, "side", "general")
    act(win, "v", "undo_steps", "37")
    assert win.viewport.history.max_steps == 37
    from core import general_settings as gs
    assert int(gs.value("undo_steps")) == 37


def test_a_change_for_the_next_start_says_so(win):
    from core.i18n import available_languages, current_language, set_language
    running = current_language()
    others = [c for c in available_languages() if c != running]
    if not others:
        pytest.skip("one language only")
    win.open_preferences()
    sheet = frame(win)
    row = fields(sheet)["language"]
    from core.i18n import LANGUAGE_NAMES
    sheet = act(win, "v", "language", row[7].index(LANGUAGE_NAMES.get(others[0], others[0])))
    assert preferences().value("language") == others[0]
    notes = [b["text"] for b in sheet["content"]["blocks"] if b["kind"] == "text" and b.get("tone") == "warn"]
    assert notes and "Restart Built" in notes[0] and "Language" in notes[0]
    # Back to the running language: nothing waits for a restart any more.
    sheet = act(win, "v", "language", 0)            # «Same as the system»
    set_language(running)


def test_restore_defaults_asks_then_restores(win, monkeypatch):
    shown = []
    monkeypatch.setattr(ss.cf, "show", lambda parent, form: shown.append(form) or "chrome")
    win.open_preferences(page="navigation")
    frame(win)
    act(win, "v", "invert_orbit", "1")
    assert preferences().value("nav/invert_orbit_y") == "1"
    act(win, "verb", "restore")
    assert len(shown) == 1 and shown[0].buttons[-1][0] == "ok"
    shown[0].press("ok")
    assert preferences().value("nav/invert_orbit_y") == "0"
    assert not any(v._invert_orbit_y for v in win.all_viewports())


# ---- This model -------------------------------------------------------------------
def test_units_go_in_the_document_and_are_saved_with_it(win, tmp_path):
    win.open_preferences(page="units")
    sheet = frame(win)
    assert [r["id"] for r in sheet["side"] if r.get("on")] == ["units"]
    assert "Saved in the document" in sheet["content"]["sub"]
    scene = win.viewport.scene
    assert units.model_units_of(scene)["length"] == "m"
    assert not win._is_dirty()
    labels = fields(sheet)["u_length"][7]
    sheet = act(win, "v", "u_length", labels.index(units.unit_label("mm")))
    assert units.model_units_of(scene)["length"] == "mm"
    assert scene.dimension_style["units"] == "mm"      # the style follows, as in the dialog
    assert win._is_dirty()
    act(win, "v", "u_precision", "3")
    assert units.model_units_of(scene)["precision"] == 3
    # Saved with the document, read back from it.
    path = tmp_path / "units.igz"
    assert do_save(win, path)
    fresh = Scene()
    igz.load_into(fresh, path)
    assert units.model_units_of(fresh) == {"length": "mm", "precision": 3}
    # And, by default, new documents start in them too.
    assert units.new_document_units()["length"] == "mm"


def test_dimension_style_and_north_are_the_documents(win, tmp_path):
    from core.sun import scene_north_deg
    win.open_preferences(page="dimensions")
    frame(win)
    scene = win.viewport.scene
    act(win, "v", "dim_font", "12")
    assert scene.dimension_style["font_size"] == 12
    act(win, "v", "dim_color", "#ff0000")
    assert scene.dimension_style["color"] == [255, 0, 0]
    assert win._is_dirty()
    sheet = act(win, "side", "location")
    assert "north" in fields(sheet)
    steps = len(win.viewport.history.undo_stack)
    act(win, "v", "north", "3")
    act(win, "v", "north", "30")              # typed: one undo step, not two
    assert scene_north_deg(scene) == pytest.approx(30.0)
    assert len(win.viewport.history.undo_stack) == steps + 1
    path = tmp_path / "north.igz"
    assert do_save(win, path)
    fresh = Scene()
    igz.load_into(fresh, path)
    assert scene_north_deg(fresh) == pytest.approx(30.0)
    assert fresh.dimension_style["font_size"] == 12


# ---- 3D mouse and keyboard --------------------------------------------------------
def test_the_3d_mouse_section_holds_the_axis_table(win):
    from core.ndof import AXES
    win.open_preferences(page="ndof")
    sheet = frame(win)
    got = fields(sheet)
    assert {"ndof_on", "ndof_speed", "ndof_lock", "ndof_preset"} <= set(got)
    for axis in AXES:
        assert {f"ndof_map_{axis}", f"ndof_inv_{axis}", f"ndof_speed_{axis}"} <= set(got)
    verbs = [v["id"] for b in sheet["content"]["blocks"] if b["kind"] == "buttons" for v in b["buttons"]]
    assert verbs == ["ndof_load", "ndof_save", "ndof_reset"]
    # Editing an axis stores it at once and makes the mapping Custom.
    act(win, "v", "ndof_inv_up", "1")
    from views.ndof_input import load_settings
    assert load_settings().axis_inversion["up"] is True
    sheet = frame(win)
    preset = fields(sheet)["ndof_preset"]
    assert preset[7][int(preset[4])] == "Custom"
    # A preset loads its values.
    sheet = act(win, "v", "ndof_preset", preset[7].index("FreeCAD-style"))
    from core.ndof import FREECAD_MAPPING
    assert load_settings().axis_mapping == FREECAD_MAPPING
    act(win, "verb", "ndof_reset")
    assert load_settings().axis_inversion["up"] is False


def test_keyboard_lists_commands_and_changes_their_keys(win, monkeypatch):
    shown = []
    monkeypatch.setattr(ss.cf, "show", lambda parent, form: shown.append(form) or "chrome")
    from core.shortcuts import action_key, collect_actions
    from ingetrazo_render import sheet_command_scope
    acts = collect_actions(win)
    target = next(a for a in acts if a.text() and not a.shortcuts())
    win.open_preferences(shortcut_of=target)
    sheet = frame(win)
    assert [r["id"] for r in sheet["side"] if r.get("on")] == ["keyboard"]
    rows = next(b for b in sheet["content"]["blocks"] if b["kind"] == "rows")
    expected = {action_key(a) for a in acts + collect_actions(sheet_command_scope(win))}
    assert "sheet:group" in expected
    assert {r["id"] for r in rows["items"]} == {"sc:" + k for k in expected}
    assert len(rows["items"]) == len(expected)
    item = "sc:" + action_key(target)
    assert sheet["focus"] == item and sheet["force_focus"]
    assert sheet["detail"] is not None and "Change keys…" in json.dumps(sheet["detail"], ensure_ascii=False)
    # Enter / double-click: a prompt for the keys, checked before it closes.
    act(win, "open", item)
    form = shown[-1]
    form.receive("keys", "3")                  # a digit types a measure
    form.press("ok")
    assert not form.closed and "cannot be a shortcut" in form.error
    form.receive("keys", "Ctrl+Alt+Shift+F11")
    form.press("ok")
    assert form.closed
    assert target.portable_keys() == ["Ctrl+Alt+Shift+F11"]
    st = preferences()
    st.beginGroup("shortcuts")
    assert any("F11" in str(st.value(k)) for k in st.allKeys())
    st.endGroup()
    # Keys another command holds: asked first, then taken from it.
    other = next(a for a in acts if a is not target and a.text() and not a.shortcuts())
    cs.action(win, SEP.join(("settings", "focus", "sc:" + action_key(other))))
    act(win, "verb", "kb_change")
    form = shown[-1]
    form.receive("keys", "Ctrl+Alt+Shift+F11")
    form.press("ok")
    ask = shown[-1]
    assert ask is not form and "already used by" in ask.fields[0].value
    ask.press("ok")
    assert other.portable_keys() == ["Ctrl+Alt+Shift+F11"]
    assert target.shortcuts() == []


def test_search_lists_commands_too(win):
    win.open_preferences()
    frame(win)
    sheet = act(win, "search", "zoom")
    assert any(c == "Keyboard" for c in captions(sheet))
    rows = [b for b in sheet["content"]["blocks"] if b["kind"] == "rows"]
    assert rows and all("zoom" in (i["label"] + i.get("sub", "") + i.get("right", "")).lower()
                        for i in rows[0]["items"])


def test_offscreen_units_command_opens_settings(win):
    win._on_units()
    assert cs.current(win).section == "units"


def test_the_shell_prefs_command_opens_the_sheet(win):
    from ingetrazo_render import shell_files as files
    assert files.open_settings(win)
    assert isinstance(cs.current(win), ss.SettingsSheet)


def test_add_location_opens_coordinates_first_applies_to_the_document_and_undoes(win, tmp_path, monkeypatch):
    from core.sun import scene_north_deg
    from georef import tile_fetcher
    win.open_preferences(page="location")
    frame(win)
    scene = win.viewport.scene
    act(win, "v", "north", "37")
    opened = act(win, "b", "add_location")
    assert {"site_lat", "site_lon"} <= fields(opened).keys()
    assert "apply at once" not in opened["footer"]["hint"]
    assert not any(b["kind"] == "map" for b in opened["content"]["blocks"])
    invalid = act(win, "v", "site_lat", "91")
    apply = next(v for b in invalid["content"]["blocks"] for v in b.get("buttons", ()) if v["id"] == "site_apply")
    assert not apply["enabled"]
    act(win, "verb", "site_apply")
    assert scene.georef is None
    act(win, "v", "site_lat", "48.85")
    act(win, "v", "site_lon", "2.35")
    calls = []
    monkeypatch.setattr(tile_fetcher.TileFetcher, "request", lambda self, src, x, y, z: calls.append((src.id, x, y, z)))
    mapped = act(win, "verb", "site_map")
    map_block = next(b["map"] for b in mapped["content"]["blocks"] if b["kind"] == "map")
    assert (map_block["lat"], map_block["lon"]) == (48.85, 2.35)
    assert not calls
    act(win, "site_tiles", "4,3,12")
    assert calls == [(map_block["source"], 4, 3, 12)]
    act(win, "site_center", "48.86", "2.36", "14")
    act(win, "verb", "site_apply")
    assert (scene.georef.lat, scene.georef.lon, scene_north_deg(scene)) == (48.86, 2.36, 37)
    path = tmp_path / "located.igz"
    assert do_save(win, path)
    other = Scene()
    igz.load_into(other, path)
    assert (other.georef.lat, other.georef.lon, other.georef.north) == (48.86, 2.36, 37)
    assert win.viewport.history.undo() and scene.georef is None
    assert win.viewport.history.redo() and scene.georef.lat == 48.86
    act(win, "b", "add_location")
    act(win, "v", "site_lat", "-22")
    act(win, "verb", "site_cancel")
    assert scene.georef.lat == 48.86


def test_place_search_approximate_location_and_capture_use_the_open_draft(win, monkeypatch):
    from georef.geocode import Geocoder, IpLocator, Place

    selected = []
    ss.edit_location(win, center=(0, 0), done=lambda *coords: selected.append(coords))
    frame(win)
    act(win, "verb", "site_map")

    def search(geocoder, query):
        assert query == "Station"
        geocoder.resultsReady.emit([Place("Station Square", 46.8, 7.5)])

    monkeypatch.setattr(Geocoder, "search", search)
    monkeypatch.setattr(IpLocator, "locate", lambda locator: locator.located.emit(46.85, 7.52, "Test town"))
    act(win, "v", "site_query", "Station")
    matches = act(win, "verb", "site_search")
    assert any(v["label"] == "Station Square" for b in matches["content"]["blocks"] for v in b.get("buttons", ()))
    assert float(fields(act(win, "verb", "site_place_0"))["site_lat"][3]) == 46.8
    assert float(fields(act(win, "verb", "site_locate"))["site_lon"][3]) == 7.52
    act(win, "v", "site_draw", "1")
    act(win, "site_rect", "46.85", "7.52", "46.86", "7.53")
    act(win, "verb", "site_apply")
    assert len(selected) == 1
    lat, lon, width, length = selected[0]
    assert (lat, lon) == pytest.approx((46.855, 7.525))
    assert 750 < width < 780 and 1100 < length < 1120
    assert win.viewport.scene.georef is None


def test_move_origin_confirmation_and_undo_restore_survey_datum(win, monkeypatch):
    from georef.datum import SceneDatum
    from ingetrazo_render import chrome_dialogs
    scene = win.viewport.scene
    old = SceneDatum(45, 7, alt=1250, north=30)
    scene.georef = old
    ss.edit_location(win)
    frame(win)
    act(win, "v", "site_lat", "46")
    choices = iter(("cancel", "move"))
    monkeypatch.setattr(chrome_dialogs, "ask", lambda *_a, **_kw: next(choices))
    act(win, "verb", "site_apply")
    assert scene.georef is old
    act(win, "verb", "site_apply")
    assert (scene.georef.lat, scene.georef.north, scene.georef.alt) == (46, 30, 0)
    assert win.viewport.history.undo() and scene.georef is old

def test_ai_connection_reads_and_writes_the_plugins_provider_slots(win):
    settings = preferences()
    settings.setValue("ia/proveedor", "groq")
    settings.setValue("ia/api_key", "fixture-groq")
    settings.setValue("ia/modelo", "first")
    settings.setValue("ia/claves/openai", "fixture-openai")
    settings.setValue("ia/modelos/openai", "second")
    ss.show(win, page="ai")
    owner = cs.current(win)
    wire = fields(frame(win))
    assert wire["ia/api_key"][8] & chrome_forms.PASSWORD
    for key, value in [("ia/proveedor", "openai"), ("ia/modelo", "updated"),
                       ("ia/api_key", "fixture-replacement"), ("ia/ollama_url", "http://localhost:3210")]:
        owner.apply(owner._rows[key], value)
        frame(win)
    assert settings.value("ia/claves/groq") == "fixture-groq"
    assert settings.value("ia/modelos/groq") == "first"
    assert settings.value("ia/claves/openai") == settings.value("ia/api_key") == "fixture-replacement"
    assert settings.value("ia/modelos/openai") == settings.value("ia/modelo") == "updated"
    assert settings.value("ia/ollama_url") == "http://localhost:3210"


def test_ambient_occlusion_edits_the_active_style(win):
    win.open_preferences(page="appearance")
    before = fields(frame(win))
    assert before["ambient_occlusion"][4] == 0
    assert before["ao_distance"][8] & chrome_forms.DISABLED
    enabled = fields(act(win, "v", "ambient_occlusion", "1"))
    assert not enabled["ao_distance"][8] & chrome_forms.DISABLED
    act(win, "v", "ao_intensity", "1.5")
    assert win.active_style().ao_intensity == 1.5
    act(win, "v", "ambient_occlusion", "0")
    assert not win.active_style().ambient_occlusion


def test_occlusion_distance_slider_preserves_off_step_file_values(win):
    from core.style import AO_DISTANCE_RANGE
    win.active_style().ao_distance = 0.37
    win.open_preferences(page="appearance")
    frame(win)
    rows = fields(act(win, "v", "ambient_occlusion", "1"))
    distance = rows["ao_distance"]
    assert distance[1] == "slider" and "m" in distance[2]
    act(win, "v", "ao_intensity", "1.2")
    assert win.active_style().ao_distance == 0.37
    for step, want in ((0, AO_DISTANCE_RANGE[0]), (200, AO_DISTANCE_RANGE[1])):
        act(win, "v", "ao_distance", str(step))
        assert win.active_style().ao_distance == want
    act(win, "v", "ao_distance", "100")
    assert 0.3 < win.active_style().ao_distance < 1.5
