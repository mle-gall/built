# SPDX-License-Identifier: GPL-3.0-or-later
"""AI Assistant contributions to the native plugin host."""
from core.i18n import tr
from plugins._assistant_panel import AssistantPanel, create_panel
from plugins._assistant_prompt import (
    MAX_ROUNDS, MAX_FAILED_ROUNDS, MAX_TOKENS, TOKENS_BY_PROVIDER,
    SYSTEM_PROMPT, system_prompt, _asks_to_build, key_placeholder, env_key_note,
)

AsistentePanel = AsistenteDialog = AssistantPanel


def setup(app):
    from core.plugin_panels import open_panel
    app.add_menu_action(tr("AI Assistant"), lambda: open_panel(app.window, app.key),
                        "Ctrl+Shift+A", tr("Open the AI assistant"))
