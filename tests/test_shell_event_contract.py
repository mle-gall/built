"""Grep shell-facing input handlers for calls missing from their event adapters."""
import ast
from pathlib import Path
import re

import ingetrazo_render as events


def test_shell_event_methods():
    root = Path(__file__).resolve().parents[1]
    views = root / "views"
    files = [*views.glob("wgpu*.py"),
             *root.joinpath("tools").glob("*.py")]
    handlers = "\n".join(path.read_text() for path in files)
    calls = re.compile(r"\b(?:ev|event)\.(\w+)\(")
    adapters = [events.MouseEvent, events.KeyEvent, events.WheelEvent,
                events.ContextMenuEvent, events.NativeGestureEvent]
    assert not set(calls.findall(handlers)).difference(*(dir(adapter) for adapter in adapters))

    native_keys = root / "rust/ingetrazo_render/src/document_input/keys.rs"
    key_calls = set(re.findall(r'\bev\s*\.call_method0\("([^"]+)"', native_keys.read_text()))
    assert {"key", "modifiers"} <= key_calls <= set(dir(events.KeyEvent))

    # Check each pointer and keyboard handler against its own event type.
    mouse_handlers = ("mousePressEvent", "mouseReleaseEvent", "mouseDoubleClickEvent",
                      "mouseMoveEvent", "_is_triple_click")
    by_handler = {name: events.MouseEvent for name in mouse_handlers}
    by_handler.update(keyPressEvent=events.KeyEvent, keyReleaseEvent=events.KeyEvent,
                      wheelEvent=events.WheelEvent, contextMenuEvent=events.ContextMenuEvent)
    for path in files:
        source = path.read_text().splitlines()
        for node in ast.walk(ast.parse("\n".join(source))):
            if isinstance(node, ast.FunctionDef) and node.name in by_handler:
                methods = set(calls.findall("\n".join(source[node.lineno - 1:node.end_lineno])))
                assert methods <= set(dir(by_handler[node.name])), (path, node.name, methods)
