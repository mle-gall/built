"""A 3-member Built project with one missing member, opened in a real
native window (`built --script`): each member through the asynchronous open
and its capsule, the missing one a row and a problem, no question asked."""
import json
import os
from pathlib import Path

import pytest

from scripts.isolated_launch import run
from tests.test_builtproject_shell import three_member_project

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "rust/built_app/target/release/built"


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_project_open_with_a_missing_member(tmp_path):
    project = three_member_project(tmp_path / "job", legacy=True)
    steps = tmp_path / "commands.txt"
    steps.write_text(f"""open {project}
wait-open
wait 600
wait-open
wait 400
state
quit
""")
    (tmp_path / "home").mkdir()
    result = run([str(BIN), "--root", str(ROOT), "--new-instance", "--size", "1280x800",
                  "--script", str(steps)],
                 env=dict(os.environ, BUILT_ANSWERS="escape"),
                 home=tmp_path / "home", settings=tmp_path / "profile",
                 capture_output=True, text=True, timeout=120)
    log = result.stdout + result.stderr
    assert result.returncode == 0, log
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert states and states[-1]["errors"] == 0, log
    state = states[-1]
    assert state["documents"] == ["a.igz", "b.igz"], log
    assert state["read_only"] == [False, True], log
    files = {Path(f["path"]).name: f for f in state["project"]["files"]}
    assert files["gone.igz"]["state"] == "missing" and files["a.igz"]["state"] == "open", log
    assert [Path(p["path"]).name for p in state["project"]["problems"]] == ["gone.igz"], log
    assert not state["form_open"] and state["large_sheet"] in ("", "closed"), log
    # Converted in place; the session went to the isolated profile.
    assert not project.exists() and project.with_suffix(".builtproject").is_file()
    assert list((tmp_path / "profile" / "projects").glob("*.json")), log
