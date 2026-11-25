"""Replay the plugin command contract captured before the native History port."""
import json
from pathlib import Path

from core.history import Command, CompoundCommand, History, SetPluginDataCommand
from core.scene import Scene


def history_trace():
    history = History(Scene())
    events = []
    unsubscribe = history.add_listener(
        after=lambda cmd, phase, ran: events.append([type(cmd).__name__, phase, ran]))
    states = []

    def capture():
        states.append({
            "undo": [type(cmd).__name__ for cmd in history.undo_stack],
            "redo": [type(cmd).__name__ for cmd in history.redo_stack],
            "data": dict(history.scene.plugin_data),
            "changes": history.changes_since(),
        })

    class PluginCommand(SetPluginDataCommand):
        pass

    grouped = CompoundCommand([PluginCommand("a", 1), PluginCommand("b", 2)])
    history.execute(grouped)
    capture()
    history.undo()
    capture()
    history.redo()
    capture()
    unsubscribe()
    history.undo()
    history.execute(PluginCommand("c", 3))
    capture()
    return {"states": states, "events": events, "plugin_command": isinstance(grouped, Command)}


def test_history_matches_python_plugin_contract():
    fixture = Path(__file__).with_name("fixtures") / "history_plugin_python.json"
    assert history_trace() == json.loads(fixture.read_text())
