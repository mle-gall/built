"""The Rust document against the Python one (docs/rust-document-model.md §7).

``tests/doc_parity_scenario.py`` builds a document (tags, materials,
component instances, a nested assembly, a ``.built`` placement), runs the
slice-1 commands through History, undoes and redoes them, groups two
placements, and saves and reopens ``.igz`` and ``.built``. It runs once with
``INGETRAZO_DOC=0`` and once with the Rust document; the two runs must agree:

- the document after every step;
- the feed's records (the Rust one may name MORE modified placements: it
  reads them from the step's journal instead of from command knowledge);
- the saved files, byte for byte, for the same document; and each run's
  save → open → save gives the same bytes.

Two differences are the point of the move and are asserted as such: undoing
a Move puts back the exact matrix (Python composes ``-delta`` in float32),
and a tag a command made keeps its uid across undo and redo (Python's redo
makes a new one)."""
import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

from core import doc_model

ROOT = Path(__file__).resolve().parent.parent
SCENARIO = ROOT / "tests" / "doc_parity_scenario.py"

pytestmark = pytest.mark.skipif(not doc_model.ENABLED,
                                reason="the Rust document is not built")


def run(tmp_path, doc):
    work = tmp_path / ("rust" if doc else "python")
    work.mkdir()
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen")
    if doc:
        env.pop("INGETRAZO_DOC", None)
    else:
        env["INGETRAZO_DOC"] = "0"
    out = subprocess.run([sys.executable, str(SCENARIO), str(work)], env=env,
                         capture_output=True, text=True, timeout=300, cwd=ROOT)
    assert out.returncode == 0, out.stderr[-4000:]
    return json.loads(out.stdout)


@pytest.fixture(scope="module")
def runs(tmp_path_factory):
    tmp = tmp_path_factory.mktemp("parity")
    return run(tmp, False), run(tmp, True)


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
    assert py["states"][17]["groups"][0]["children"][1]["xform"] != before


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
        assert py["initial"][name]["members"] == rs["initial"][name]["members"], name


def test_save_open_save_is_stable(runs):
    for out in runs:
        for when in ("initial", "final"):
            for name, f in out[when].items():
                assert f["again"] == f["members"], (out["engine"], when, name)


def test_reopened_documents_agree(runs):
    py, rs = runs
    for when in ("initial", "final"):
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
