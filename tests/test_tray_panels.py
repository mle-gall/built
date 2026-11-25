"""AI commands open the renderer-owned controls for the current document."""
from types import SimpleNamespace
from unittest.mock import Mock

from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_the_assistant_and_bridge_publish_native_panels(native_host):
    from views.extension_api import ExtensionApp
    from plugins import ai_assistant, ai_bridge
    for plugin in (ai_assistant, ai_bridge):
        panel = plugin.create_panel(ExtensionApp(native_host.window, plugin.__name__), lambda: None)
        assert panel.slot == "dock" and panel.sections()


def test_assistant_shortcut_reopens_its_panel(native_host, monkeypatch):
    from plugins import ai_assistant
    from core import plugin_panels
    app = SimpleNamespace(window=native_host.window, key="ai_assistant", add_menu_action=Mock())
    monkeypatch.setattr(plugin_panels, "open_panel", opened := Mock())
    ai_assistant.setup(app)
    title, summon, shortcut, tip = app.add_menu_action.call_args.args
    assert title == "AI Assistant" and shortcut == "Ctrl+Shift+A"
    summon()
    opened.assert_called_once_with(app.window, app.key)


def test_the_bridge_control_starts_and_stops(native_host, monkeypatch):
    from plugins.ai_bridge import BridgeSection
    from plugins._mcp_endpoint import connect_private
    section = BridgeSection(native_host.viewport)
    assert not section.running
    section.event("toggle", "invoke", None)
    try:
        assert section.running and "--mcp" in str(section.sections())
        with connect_private() as sock:
            assert sock.getpeername().endswith("bridge.sock")
    finally:
        section.stop()
    assert not section.running


def test_the_prompt_keeps_several_lines_before_send(native_host):
    from plugins.ai_assistant import create_panel
    from views.extension_api import ExtensionApp
    panel = create_panel(ExtensionApp(native_host.window, "ai_assistant"), lambda: None)
    text = "una casa\ncon techo a dos aguas"
    panel.event("prompt", "commit", text)
    assert panel.prompt == text and not panel.busy
