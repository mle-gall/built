"""The Built project format (docs/builtproject-format.md): the manifest,
the in-place .igzproj conversion, the per-user session and model pointers.

Every fixture is synthetic, in the shapes test_igz_project.py and
test_home_worksession.py build: never the owner's files.
"""
import json
import os
from pathlib import Path

import pytest

import ingetrazo_core as native
from core import bim_registry, project
from core.scene import Scene
from formats import igz
from formats import project as storage

NAMESPACE = "ca2c12df23ac48f9a103455660d950e5"


def legacy_value(**extra):
    """An .igzproj as the fork wrote it (version 1, every section)."""
    registry = bim_registry.seed_defaults()
    registry["project"] = {"name": "As-built ride", "phase": "As-built"}
    registry["types"]["queue_wall"] = {"name": "Queue wall", "ifc_class": "IfcWall",
                                       "predefined_type": "STANDARD", "psets": {}, "classifications": []}
    value = {
        "version": 1,
        "bim_registry": registry,
        "ifc_identity": {"namespace": NAMESPACE, "project": "2gB0vM6Bf8mPFPXX3tF4sX",
                         "site": "2gB0vM6Bf8mPFPXX3tF4sY", "building": "2gB0vM6Bf8mPFPXX3tF4sZ",
                         "storey": "2gB0vM6Bf8mPFPXX3tF4sa"},
        "files": [{"path": "host.igz", "role": "host"},
                  {"path": "parts/track.igz", "role": "linked"},
                  {"path": "survey.pdf", "role": "reference"}],
        "links": [{"host": "host.igz", "source": "parts/track.igz",
                   "placement_uid": "0123456789abcdef", "select": {"uids": ["fedcba9876543210"]}},
                  {"host": "host.igz", "source": "parts/track.igz", "placement_uid": "00000000000000aa",
                   "select": {"filter": {"facet": "name", "glob": "Track*"}}, "include_nested": False}],
        "worksession": {"files": [{"path": "parts/track.igz", "camera": {"distance": 12.5}},
                                  {"path": "host.igz", "camera": None}],
                        "active_file": "parts/track.igz"},
    }
    value.update(extra)
    return value


def write_legacy(folder, value=None, name="Job.igzproj", text=None):
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / name
    path.write_text(text if text is not None else json.dumps(value or legacy_value(), indent=2), encoding="utf-8")
    return path


def members(folder):
    """The member models on disk: what a conversion must never rewrite."""
    (folder / "parts").mkdir(parents=True, exist_ok=True)
    out = {}
    for rel in ("host.igz", "parts/track.igz"):
        scene = Scene()
        scene.plugin_data["project"] = {"path": str(folder / "Job.igzproj"),
                                        "rel": os.path.relpath(folder / "Job.igzproj", (folder / rel).parent)}
        igz.save_scene(scene, folder / rel)
        out[rel] = (folder / rel).read_bytes(), (folder / rel).stat().st_mtime_ns
    (folder / "survey.pdf").write_bytes(b"%PDF-1.4\n")
    return out


# ---- the manifest (§1) -------------------------------------------------------
def test_manifest_round_trip_keeps_unknown_fields_and_exact_numbers(tmp_path):
    path = tmp_path / "Job.builtproject"
    value = json.loads(native.project_manifest_new(path, json.dumps(bim_registry.seed_defaults()),
                                                   json.dumps({"namespace": NAMESPACE})))
    assert value["built_project"] == 1 and len(value["id"]) == 32
    text = path.read_text()
    edited = json.loads(text)
    edited["future"] = {"nested": [1, {"deep": True}], "number": 0.1}
    edited["bim_registry"]["custom_section"] = {"kept": "yes"}
    storage.save(path, edited)
    raw = path.read_text().replace('"number": 0.1', '"number": 0.10000000000000000555')
    assert raw != path.read_text()
    path.write_text(raw)
    again = storage.load(path)
    assert again["future"]["nested"] == [1, {"deep": True}]
    assert again["bim_registry"]["custom_section"] == {"kept": "yes"}
    # Rust keeps the number's text; its writes never round it.
    native.project_manifest_save(path, native.project_manifest_load(path))
    assert "0.10000000000000000555" in path.read_text()
    assert path.read_text().endswith("\n")


@pytest.mark.parametrize("change, message", [
    ({"built_project": 2}, "Unsupported Built project version"),
    ({"built_project": None}, "Unsupported"),
    ({"igz_format": 3}, "cannot also be a document"),
    ({"id": "NOT-HEX"}, "id must be"),
    ({"files": [{"path": "a.built", "role": "owner"}]}, "role"),
    ({"files": [{"path": "a.built", "role": "host"}, {"path": "a.built", "role": "linked"}]}, "twice"),
    ({"links": [{"host": "a.built", "source": "b.built"}]}, "placement_uid"),
    ({"links": [{"host": "a", "source": "b", "placement_uid": "c", "select": {"uids": [], "filter": {}}}]}, "exactly"),
    ({"ifc_identity": {"namespace": "nope"}}, "namespace"),
])
def test_manifest_rejects_what_the_spec_rejects(tmp_path, change, message):
    path = tmp_path / "Job.builtproject"
    native.project_manifest_new(path, "{}", json.dumps({"namespace": NAMESPACE}))
    value = json.loads(path.read_text())
    value.update(change)
    path.write_text(json.dumps(value))
    with pytest.raises(ValueError, match=message):
        storage.load(path)


def test_new_manifest_never_replaces_a_file(tmp_path):
    path = tmp_path / "Job.builtproject"
    path.write_text("keep me")
    with pytest.raises(ValueError, match="already exists"):
        native.project_manifest_new(path, "{}", json.dumps({"namespace": NAMESPACE}))
    assert path.read_text() == "keep me"


def test_create_writes_builtproject_with_relative_slash_paths(tmp_path):
    scene = Scene()
    host = tmp_path / "models" / "a.built"
    host.parent.mkdir()
    project.create(scene, tmp_path / "Job.igzproj", host)      # an old name becomes the new suffix
    path = tmp_path / "Job.builtproject"
    assert path.is_file() and not (tmp_path / "Job.igzproj").exists()
    assert storage.load(path)["files"] == [{"path": "models/a.built", "role": "host"}]
    assert scene.plugin_data["project"]["rel"] == "../Job.builtproject"


def test_saving_a_host_keeps_other_hosts_edges(tmp_path):
    path = tmp_path / "Job.builtproject"
    native.project_manifest_new(path, "{}", json.dumps({"namespace": NAMESPACE}))
    edge = {"host": "b.built", "source": "s.built", "placement_uid": "1"}
    native.project_sync_host(path, "b.built", json.dumps([{"path": "b.built", "role": "host"}]), json.dumps([edge]))
    native.project_sync_host(path, "a.built", json.dumps([{"path": "a.built", "role": "host"},
                                                          {"path": "b.built", "role": "linked"}]), "[]")
    value = storage.load(path)
    assert value["links"] == [edge]
    assert value["files"] == [{"path": "b.built", "role": "host"}, {"path": "a.built", "role": "host"}]


# ---- conversion (§5) ---------------------------------------------------------
def test_conversion_is_lossless_and_removes_the_old_file(tmp_path):
    before = members(tmp_path)
    old = write_legacy(tmp_path)
    original = json.loads(old.read_text())
    manifest, how = native.project_convert(old)
    assert how == "converted" and Path(manifest) == tmp_path / "Job.builtproject"
    assert not old.exists()
    value = storage.load(manifest)
    assert value["id"] == NAMESPACE and value["built_project"] == 1 and "version" not in value
    # Every old field, the inert worksession included, with its value.
    assert json.loads(native.project_legacy_json(manifest)) == original
    # Member models are never rewritten.
    for rel, (data, mtime) in before.items():
        assert (tmp_path / rel).read_bytes() == data and (tmp_path / rel).stat().st_mtime_ns == mtime


def test_conversion_ids_without_a_hex_namespace(tmp_path):
    value = legacy_value()
    value["ifc_identity"]["namespace"] = "ca2c12df-23ac-48f9-a103-455660d950e5"
    manifest, _ = native.project_convert(write_legacy(tmp_path, value))
    assert len(storage.load(manifest)["id"]) == 32


def test_existing_manifest_wins_and_the_old_file_stays(tmp_path):
    old = write_legacy(tmp_path)
    newer = tmp_path / "Job.builtproject"
    native.project_manifest_new(newer, "{}", json.dumps({"namespace": NAMESPACE}))
    text = newer.read_text()
    assert native.project_convert(old) == (newer, "already")
    assert old.is_file() and newer.read_text() == text


@pytest.mark.parametrize("text", ["{not json", json.dumps({"version": 2}),
                                  json.dumps({**legacy_value(), "files": "nope"})])
def test_failed_conversion_touches_nothing(tmp_path, text):
    old = write_legacy(tmp_path, text=text)
    with pytest.raises(ValueError):
        native.project_convert(old)
    assert old.read_text() == text
    assert sorted(p.name for p in tmp_path.iterdir()) == ["Job.igzproj"]


def test_unwritable_folder_leaves_the_old_file(tmp_path):
    old = write_legacy(tmp_path)
    original = old.read_text()
    os.chmod(tmp_path, 0o555)
    try:
        with pytest.raises(ValueError):
            native.project_convert(old)
    finally:
        os.chmod(tmp_path, 0o755)
    assert old.read_text() == original and not (tmp_path / "Job.builtproject").exists()


def test_start_page_listing_never_converts(tmp_path):
    old = write_legacy(tmp_path)
    rows = native.project_members(old)
    assert (str((tmp_path / "parts/track.igz").resolve()), "linked") in [tuple(r) for r in rows]
    assert old.is_file() and not (tmp_path / "Job.builtproject").exists()


# ---- pointers (§2) -----------------------------------------------------------
def test_old_pointer_finds_the_converted_sibling_and_moves_on_save(tmp_path):
    before = members(tmp_path)
    write_legacy(tmp_path)
    host = tmp_path / "host.igz"
    scene = Scene()
    igz.load_into(scene, host)          # reading the registry converts the project
    assert (tmp_path / "Job.builtproject").is_file() and not (tmp_path / "Job.igzproj").exists()
    assert host.read_bytes() == before["host.igz"][0]     # opening rewrote nothing
    assert scene.plugin_data["project"]["path"].endswith("Job.igzproj")
    assert project.path_for(scene) == (tmp_path / "Job.builtproject").resolve()
    assert bim_registry.get(scene)["project"]["name"] == "As-built ride"
    igz.save_scene(scene, host)         # the next normal save moves the pointer
    again = Scene()
    igz.load_into(again, host)
    assert again.plugin_data["project"] == {"path": str((tmp_path / "Job.builtproject").resolve()),
                                            "rel": "Job.builtproject"}


def test_pointer_prefers_rel_then_path_and_survives_a_moved_folder(tmp_path):
    first = tmp_path / "first"
    old = write_legacy(first)
    native.project_convert(old)
    moved = tmp_path / "moved"
    first.rename(moved)
    (moved / "models").mkdir()
    pointer = json.dumps({"path": str(first / "Job.igzproj"), "rel": "../Job.igzproj"})
    found = native.project_resolve_pointer(moved / "models" / "a.built", pointer)
    assert Path(found) == (moved / "Job.builtproject").resolve()
    assert native.project_resolve_pointer(tmp_path / "x.built", json.dumps({"path": str(moved / "Job.igzproj")})) \
        == (moved / "Job.builtproject").resolve()
    assert native.project_resolve_pointer(tmp_path / "x.built", json.dumps({"rel": "nowhere.igzproj"})) is None


def test_missing_project_never_recreates_a_shared_file(tmp_path):
    scene = Scene()
    project.create(scene, tmp_path / "Job.builtproject")
    (tmp_path / "Job.builtproject").unlink()
    with pytest.raises((OSError, ValueError)):
        project.write_registry(scene, bim_registry.seed_defaults())
    assert not (tmp_path / "Job.builtproject").exists()


# ---- the per-user session (§3) -------------------------------------------------
def make_project(folder, hosts=("a.built", "b.built"), missing=()):
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / "Job.builtproject"
    native.project_manifest_new(path, "{}", json.dumps({"namespace": NAMESPACE}))
    value = json.loads(path.read_text())
    for name in (*hosts, *missing):
        value["files"].append({"path": name, "role": "host"})
        if name not in missing:
            (folder / name).write_bytes(b"model")
    native.project_manifest_save(path, json.dumps(value))
    return path


def test_two_people_on_one_project_keep_their_own_sessions(tmp_path):
    path = make_project(tmp_path / "shared", hosts=("a.built", "b.built", "c.built"))
    manifest = path.read_bytes()
    alice_dir, bob_dir = tmp_path / "alice", tmp_path / "bob"
    alice = native.ProjectSession(path, alice_dir)
    bob = native.ProjectSession(path, bob_dir)
    assert Path(alice.session_file).parent == alice_dir / "projects"
    assert Path(alice.session_file).name == alice.id + ".json"
    folder = path.parent.resolve()
    for p, *_ in alice.plan():
        alice.loaded(Path(p))
    alice.set_editable(folder / "c.built")
    assert alice.record([(folder / "c.built", json.dumps({"distance": 4}), "Entrance"),
                         (folder / "a.built", None, None)])
    for p, *_ in bob.plan():
        bob.loaded(Path(p))
    assert bob.record([(folder / "b.built", None, None)])
    alice = native.ProjectSession(path, alice_dir)
    bob = native.ProjectSession(path, bob_dir)
    assert [Path(p).name for p, *_ in alice.plan()] == ["c.built", "a.built"]
    assert json.loads(alice.plan()[0][2]) == {"distance": 4} and alice.plan()[0][3] == "Entrance"
    assert Path(alice.editable).name == "c.built" and alice.read_only(folder / "a.built")
    assert [Path(p).name for p, *_ in bob.plan()] == ["b.built"]
    assert Path(bob.editable).name == "b.built"
    assert path.read_bytes() == manifest            # sessions never touch the manifest


def test_session_location_honours_the_settings_override(tmp_path, monkeypatch):
    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path / "profile"))
    assert Path(native.project_session_dir()) == tmp_path / "profile" / "projects"
    assert Path(native.project_session_dir(tmp_path / "x")) == tmp_path / "x" / "projects"
    path = make_project(tmp_path / "job")
    session = native.ProjectSession(path)
    assert Path(session.session_file).is_relative_to(tmp_path / "profile")


def test_a_converted_worksession_seeds_the_first_session(tmp_path):
    members(tmp_path)
    old = write_legacy(tmp_path)
    session = native.ProjectSession(old, tmp_path / "me")
    assert session.conversion == (old.resolve(), "converted")
    plan = session.plan()
    assert [Path(p).name for p, *_ in plan] == ["track.igz", "host.igz"]
    assert json.loads(plan[0][2]) == {"distance": 12.5}
    assert Path(session.editable).name == "track.igz"
    rows = {Path(r["path"]).name: r for r in json.loads(session.query())["files"]}
    assert rows["survey.pdf"]["role"] == "reference" and not rows["survey.pdf"]["tab"]


def test_missing_member_is_a_row_and_a_problem_not_an_error(tmp_path):
    path = make_project(tmp_path, hosts=("a.built", "b.built"), missing=("gone.built",))
    session = native.ProjectSession(path, tmp_path / "me")
    plan = {Path(p).name: exists for p, _r, _c, _s, exists in session.plan()}
    assert plan == {"a.built": True, "b.built": True, "gone.built": False}
    session.loaded(tmp_path / "a.built")
    session.failed(tmp_path / "b.built", "not a Built document")
    assert session.pending() == []
    problems = dict(session.problems())
    assert problems[str((tmp_path / "gone.built").resolve())] == "gone.built is missing"
    assert "not a Built document" in problems[str((tmp_path / "b.built").resolve())]
    rows = session.tree_rows("", set())
    assert rows[0][0][0] == "project-files:all" and rows[0][1][2] == 2
    icons = {r[0][1]: r[0][4] for r in rows[1:]}
    assert icons == {"a.built": "file:resolved", "b.built": "file:missing", "gone.built": "file:missing"}
    assert session.tree_rows("", {"project-files:all"})[1:] == []     # folded
    # A planned tab that never came in stays in the session for next time.
    session.record([(tmp_path / "a.built", None, None)])
    assert [Path(p).name for p, *_ in native.ProjectSession(path, tmp_path / "me").plan()] == \
        ["a.built", "b.built", "gone.built"]


def test_editable_file_must_be_a_document_of_the_project(tmp_path):
    path = make_project(tmp_path)
    (tmp_path / "notes.pdf").write_bytes(b"%PDF")
    session = native.ProjectSession(path, tmp_path / "me")
    with pytest.raises(ValueError):
        session.set_editable(tmp_path / "notes.pdf")
    with pytest.raises(ValueError):
        session.set_editable(tmp_path / "absent.built")
    session.set_editable(tmp_path / "b.built")
    assert not session.read_only(tmp_path / "b.built") and session.read_only(tmp_path / "a.built")


def test_classic_worksession_helpers_use_the_session_file(tmp_path):
    path = make_project(tmp_path)
    a, b = tmp_path / "a.built", tmp_path / "b.built"
    project.save_worksession(path, [dict(path=b, camera={"distance": 7}), dict(path=a)], a)
    state = project.worksession(path)
    assert [Path(r["path"]).name for r in state["files"]] == ["b.built", "a.built"]
    assert state["files"][0]["camera"] == {"distance": 7}
    assert Path(state["active_file"]).name == "a.built"
    assert "worksession" not in storage.load(path)
    session_dir = Path(os.environ["BUILT_SETTINGS_DIR"]) / "projects"
    assert (session_dir / (storage.load(path)["id"] + ".json")).is_file()
