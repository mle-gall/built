"""Keyboard submission of a multiline form in the native chrome."""
import json
import os
from pathlib import Path
import sys

import pytest

from scripts.isolated_launch import run
from tests.shell_isolation import restore_shell_patches, native_question_host  # noqa: F401

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="module")
def answers(tmp_path_factory):
    root = tmp_path_factory.mktemp("multiline")
    profile = root / "profile"
    folder = profile / "data/ingetrazo/plugins"
    folder.mkdir(parents=True)
    (folder / "prompt_probe.py").write_text('''import json
from pathlib import Path
from views.chrome_forms import ask_text

def setup(app):
    def ask():
        def done(value):
            with Path(__file__).with_suffix(".jsonl").open("a") as out:
                out.write(json.dumps(value) + "\\n")
        ask_text(app._window, "Text", "Label text:", "Muro", done, multiline=True)
    app.add_menu_action("Try multiline prompt", ask)
''')
    script = root / "keys.txt"
    script.write_text("wait 350\n" + "".join(
        "menu extension:prompt_probe:0\nwait 160\n" + keys + "\nwait 200\n"
        for keys in ("key enter ctrl", "key return cmd", "key end\nkey enter\nwait 70\ntype b\nwait 70\nkey enter ctrl")) + "quit\n")
    binary = Path(os.environ.get("BUILT_BINARY", ROOT / "rust/built_app/target/release/built"))
    result = run([binary, "--new-instance", "--root", ROOT, "--venv", ROOT / ".venv",
                  "--size", "800x600", "--script", script, ROOT / "tests/fixtures/shell/slab-and-cube.igz"],
                 env=dict(os.environ, BUILT_BACKGROUND="1"), settings=profile, capture_output=True, text=True, timeout=75)
    assert result.returncode == 0, result.stderr
    path = folder / "prompt_probe.jsonl"
    assert path.exists(), result.stderr
    return [json.loads(line) for line in path.read_text().splitlines()]


@pytest.mark.window
@pytest.mark.parametrize("index", [0, 1], ids=["control-enter", "command-return"])
def test_ctrl_enter_accepts_and_keeps_the_text(answers, index):
    assert answers[index] == "Muro"


@pytest.mark.window
def test_enter_alone_adds_a_line_and_does_not_accept(answers):
    assert answers[2:] == ["Muro\nb"]


def test_the_dialog_says_how_to_accept(native_question_host):
    from views import chrome_forms
    win = native_question_host.window
    chrome_forms.ask_text(win, "Text", "Label text:", multiline=True)
    form = chrome_forms.current(win)
    assert form.field("text").label == "Label text:"
    assert next(hint for _, _, hint, role in form.wire()[-1] if role == 1) == (
        "⌘↩" if sys.platform == "darwin" else "Ctrl+Enter")
    form.press("cancel")
