"""Bundled Python panel implementations loaded by the native window."""
import os

from built_core import extensions


def discover_plugins(*, native_panels):
    modules = {**native_panels, "model_info": "plugins.model_info",
               "solid_inspector": "plugins.solid_inspector",
               "ai_assistant": "plugins._assistant_panel"}
    if os.name == "posix":
        modules["ai_bridge"] = "plugins.ai_bridge"
    plugins, errors = extensions.discover_plugins(native_panels=modules)
    # Model tools precede the scripting console in the native panel chooser.
    return sorted(plugins, key=lambda p: p.stem == "python_console"), errors
