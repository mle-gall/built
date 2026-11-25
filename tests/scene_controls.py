"""Exercise the native Scene chooser over a synthetic document."""
from contextlib import contextmanager
from unittest.mock import Mock, patch

from built_core import Input as Qt
from views import wgpu_ui


@contextmanager
def chrome(win):
    with patch.object(win.view, "renderer", Mock()) as renderer:
        yield renderer


def rows(win):
    with chrome(win):
        wgpu_ui._tabs(win.view, win)
    return win.view._ui_tabs[1]


def choose(win, index):
    rows(win)
    wgpu_ui._run(win, [f"tab:scene:{index}"])


def prompt(win, command, answer, text=""):
    with chrome(win) as renderer:
        assert win.run_command(command)
        renderer.ui_set_prompt.assert_called_once()
        wgpu_ui._tree_action(win, win.view, wgpu_ui.SEP.join(("prompt", answer, text)))
        return renderer.ui_set_prompt.call_args.args


def interrupt(win, kind):
    host = win._host
    if kind == "esc":
        return host.key(True, Qt.Key_Escape)
    if kind == "wheel":
        return host.wheel(200, 180, 0, 0, 0, 120)
    return host.pointer(kind, 200, 180, button=4, held=4)
