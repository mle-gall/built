"""Regression walk through a real native window at 1280 × 800 logical points."""
import json
import sys
from pathlib import Path

import pytest

from scripts.isolated_launch import run


def test_native_input_ownership(tmp_path):
    root = Path(__file__).resolve().parents[1]
    binary = root / "rust/built_app/target/release/built"
    if sys.platform != "darwin" or not binary.is_file():
        pytest.skip("requires the macOS release shell")
    fixtures = root / "tests/fixtures/shell"
    run([sys.executable, fixtures / "make_kinds.py", tmp_path], check=True)
    script = fixtures / "input-ownership.txt"
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                  "--size", "1280x800", "--script", script, tmp_path / "kinds.built"],
                 text=True, capture_output=True, check=True, timeout=90)
    output = result.stdout + result.stderr
    assert "[built] error" not in output, output
    states = [json.loads(line.removeprefix("[built] state ")) for line in output.splitlines()
              if line.startswith("[built] state ")]
    names = [line.split("# ")[1] for line in script.read_text().splitlines() if line.startswith("state")]
    assert len(states) == len(names) + 1, output  # quit prints the final state as well
    assert all(s["errors"] == 0 for s in states), output
    s = dict(zip(names, states))

    assert s["component"]["edit_group"] == "Door" and s["left_component"]["edit_group"] is None
    assert s["opened"]["edit_group"] == "Box A"
    assert s["inside"]["selection"] == 1
    assert (s["cleared"]["edit_group"], s["cleared"]["selection"]) == ("Box A", 0)
    assert s["closed"]["edit_group"] is None
    assert s["selected"]["selection"] == s["empty_menu"]["selection"] == 1
    assert s["dismissed_context"]["selection"] == 1 and not s["dismissed_context"]["context_menu"]
    assert "Select all" in s["empty_menu"]["context_menu"]
    assert "Open for editing" not in s["empty_menu"]["context_menu"]

    assert s["view_menu"]["input"]["view_menu"]
    assert not s["dismissed"]["input"]["view_menu"] and not s["dismissed"]["tool_busy"]
    assert s["started"]["tool_busy"] and not s["cancelled"]["tool_busy"]
    assert s["review"]["input"]["review"] and not s["model"]["input"]["review"]

    assert s["filtered"]["filter"] == "Oak"
    assert s["cleared_filter"]["filter"] == "" and s["cleared_filter"]["input"]["filter_focus"]
    assert not s["blurred"]["input"]["filter_focus"] and s["blurred"]["dock_lens"] == 2
    assert s["tree"]["dock_mode"] == "" and s["tree"]["filter"] == ""
    assert s["deliveries"]["dock_lens"] == 2 and s["filing"]["dock_lens"] == 0
    assert s["formats"]["input"]["choice"]
    assert s["export"]["large_sheet"] == "export" and not s["export"]["input"]["choice"]
    assert not s["model_again"]["large_sheet"]
