"""Bundled Python panel implementations loaded by the native window."""

from built_core import extensions


def discover_plugins(*, native_panels):
    # The assistant and the MCP bridge run through Tasks and Settings > AI:
    # no panel, and their files are not loaded as plugins.
    modules = {**native_panels, "model_info": "plugins.model_info",
               "solid_inspector": "plugins.solid_inspector",
               "ai_assistant": None, "ai_bridge": None}
    plugins, errors = extensions.discover_plugins(native_panels=modules)
    # Model tools precede the scripting console in the native panel chooser.
    return sorted(plugins, key=lambda p: p.stem == "python_console"), errors
