"""Grep shell-facing input handlers for calls missing from their event adapters."""
import ast
from pathlib import Path
import re

import ingetrazo_render as events


def test_shell_event_methods():
    root = Path(__file__).resolve().parents[1]
    views = root / "views"
    files = [*views.glob("wgpu*.py"),
             *root.joinpath("tools").glob("*.py"), *root.joinpath("app_shell").glob("*.py")]
    handlers = "\n".join(path.read_text() for path in files)
    shared = (views / "document_interaction.py").read_text()
    lines = shared.splitlines()
    for node in ast.walk(ast.parse(shared)):
        if isinstance(node, ast.FunctionDef) and f"DocumentInteraction.{node.name}(" in handlers:
            handlers += "\n" + "\n".join(lines[node.lineno - 1:node.end_lineno])
    calls = re.compile(r"\b(?:ev|event)\.(\w+)\(")
    adapters = [events.MouseEvent, events.KeyEvent, events.WheelEvent,
                events.ContextMenuEvent, events.NativeGestureEvent]
    assert not set(calls.findall(handlers)).difference(*(dir(adapter) for adapter in adapters))

    # A real Qt method on the wrong event type is a gap too.
    mouse_handlers = ("mousePressEvent", "mouseReleaseEvent", "mouseDoubleClickEvent",
                      "mouseMoveEvent", "_is_triple_click")
    by_handler = {name: events.MouseEvent for name in mouse_handlers}
    by_handler.update(keyPressEvent=events.KeyEvent, keyReleaseEvent=events.KeyEvent,
                      wheelEvent=events.WheelEvent, contextMenuEvent=events.ContextMenuEvent)
    for path in (views / "wgpu_view.py", views / "document_interaction.py"):
        source = path.read_text().splitlines()
        for node in ast.walk(ast.parse("\n".join(source))):
            if isinstance(node, ast.FunctionDef) and node.name in by_handler:
                methods = set(calls.findall("\n".join(source[node.lineno - 1:node.end_lineno])))
                assert methods <= set(dir(by_handler[node.name])), (path, node.name, methods)
