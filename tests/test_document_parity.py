"""Compare the Rust document with the integration base's recorded Python
states and change feed, and compare every synthetic save byte for byte."""
import json
import os
import re
import subprocess
import sys
import zipfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
SCENARIO = ROOT / "tests" / "doc_parity_scenario.py"


def run(tmp_path):
    work = tmp_path / "rust"
    work.mkdir()
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen")
    out = subprocess.run([sys.executable, str(SCENARIO), str(work)], env=env,
                         capture_output=True, text=True, timeout=300, cwd=ROOT)
    assert out.returncode == 0, out.stderr[-4000:]
    with zipfile.ZipFile(ROOT / "tests/fixtures/document-parity.zip") as archive:
        reference = json.loads(archive.read("python/states.json"))
        current = json.loads(out.stdout)
        assert current["feed"] == json.loads(archive.read("rust/feed.json"))
        for name in archive.namelist():
            if not name.startswith("python/") or name.endswith(".json"):
                continue
            filename = name.split("/", 1)[1]
            # The base engines differed in tag UIDs after redo and in the
            # placement frame used to serialize the nested anchor scenario.
            native = "rust/" + filename
            expected = native if native in archive.namelist() else name
            assert (work / filename).read_bytes() == archive.read(expected), filename
    return reference, current


@pytest.fixture(scope="module")
def runs(tmp_path_factory):
    tmp = tmp_path_factory.mktemp("parity")
    return run(tmp)


_AUTO = re.compile(r"^uid-\d{5}$")


def norm(x, tol=1e-6):
    """Auto-made uids become one token; floats are rounded to ``tol``."""
    if isinstance(x, dict):
        return {k: norm(v, tol) for k, v in x.items()}
    if isinstance(x, list):
        return [norm(v, tol) for v in x]
    if isinstance(x, str) and _AUTO.match(x):
        return "<auto uid>"
    if isinstance(x, float):
        return round(x / tol) * tol
    return x


def test_engines(runs):
    py, rs = runs
    assert py["engine"] == "Python" and rs["engine"] == "Rust"


def test_affine_instance_edit_agrees(runs):
    py, rs = runs
    assert norm(rs["rotated_edit"]) == norm(py["rotated_edit"])


def test_every_state_agrees(runs):
    py, rs = runs
    assert len(py["states"]) == len(rs["states"])
    for i, (a, b) in enumerate(zip(py["states"], rs["states"])):
        assert norm(a) == norm(b), f"state {i}"


def test_undoing_a_move_is_exact(runs):
    py, rs = runs
    # States: 0 initial, 1..11 the commands, then the undos (12 undoes the
    # last command, ..., 17 undoes the Move of the part, command 6).
    before = rs["states"][5]["groups"][0]["children"][1]["xform"]
    assert rs["states"][17]["groups"][0]["children"][1]["xform"] == before
    assert py["states"][17]["groups"][0]["children"][1]["xform"] == before


def test_feed_records_agree(runs):
    py, rs = runs
    assert len(py["feed"]) == len(rs["feed"])
    for i, (a, b) in enumerate(zip(py["feed"], rs["feed"])):
        a, b = norm(a), norm(b)
        assert (a["phase"], a["command"], a["complete"]) == \
            (b["phase"], b["command"], b["complete"]), i
        for kind in ("created", "removed"):
            assert a["placements"][kind] == b["placements"][kind], (i, kind)
        assert set(a["placements"]["modified"]) <= set(b["placements"]["modified"]), i
        assert a["tags"] == b["tags"], i
        assert a["materials"] == b["materials"], i


def test_the_same_document_saves_the_same_bytes(runs):
    py, rs = runs
    for name in ("doc.igz", "doc.built"):
        assert py["initial"][name]["file_hash"] == rs["initial"][name]["file_hash"], name


def test_save_open_save_is_stable(runs):
    for out in runs:
        for when in ("initial", "final", "shared"):
            for name, f in out[when].items():
                assert f["again"] == f["members"], (out["engine"], when, name)
                assert f["again_hash"] == f["file_hash"], (out["engine"], when, name)


def test_reopened_documents_agree(runs):
    py, rs = runs
    for when in ("initial", "final", "shared"):
        for name in ("doc.igz", "doc.built"):
            assert norm(py[when][name]["reopened"]) == norm(rs[when][name]["reopened"]), (when, name)


def test_a_tag_made_by_a_command_keeps_its_uid(runs):
    py, rs = runs

    def uids(out):
        return [next(ly["uid"] for ly in s["layers"] if ly["name"] == "Brand new tag")
                for s in out["states"]
                if any(ly["name"] == "Brand new tag" for ly in s["layers"])]

    assert len(set(uids(rs))) == 1
    assert len(set(uids(py))) > 1


def test_geometry_history_agrees(runs):
    python, rust = runs
    assert norm(rust["geometry"]) == norm(python["geometry"])


def test_scene_folder_edits_match_archived_replay():
    from built_core import scene_tree
    from built_core.history import History, SceneTreeCommand
    from built_core.saved_views import SavedView
    from built_core.scene import Scene

    scene = Scene()
    scene.scene_folders = ["A", "B", "C", "D", "E", "F", "A"]
    scene.scene_folder_parents = {"B": "A", "C": "A", "D": "B",
                                  "E": "F", "F": "E", "missing": "A"}
    a, b, c = [SavedView("v" + str(i), folder=f)
               for i, f in enumerate(["D", "C", "implicit"])]
    scene.saved_views.extend([a, b, c])
    history = History(scene)

    def snapshot():
        names = scene_tree.folders_of(scene)
        return [names, [scene_tree.folder_parent(scene, f) for f in [*names, "missing"]],
                scene_tree.subfolders(scene, None, deep=True),
                [v.name for v in scene_tree.tree_order(scene)],
                [[v.name, v.folder] for v in scene.saved_views], dict(scene.scene_folder_parents)]

    edit = scene_tree.FolderEdit
    states = [snapshot()]
    for op in (edit.add("A", "B"), edit.rename("B", "\x1cPlan\x1c"),
               edit.move_rows([c, a], ["C"], "Plan", a), edit.delete_rows([b], ["A"])):
        history.execute(SceneTreeCommand(op))
        assert history.last_error is None and history.last_rejected is None
        states.append(snapshot())
        assert history.undo()
        states.append(snapshot())
        assert history.redo()
        states.append(snapshot())
    with zipfile.ZipFile(ROOT / "tests/fixtures/document-parity.zip") as archive:
        assert states == json.loads(archive.read("python/scene-folders.json"))
