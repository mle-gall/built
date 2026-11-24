"""Bind a Rust window command to its handler and the shared shortcut editor."""
from ingetrazo_render import shortcut_text


class ShellAction:
    def __init__(self, window, label, *, run=None, toggled=None, checkable=False, listed=True):
        self.window = window
        self.state = window._command_store.add(label, checkable)
        self.run = run
        self.toggled = toggled
        self.changed = None
        if listed:
            window._command_actions[self.state.id] = self

    def __getattr__(self, name):
        return getattr(self.state, name)

    def parent(self):
        return self.window

    def toolTip(self):
        return self.text()

    def statusTip(self):
        return ""

    def menu(self):
        return None

    def isSeparator(self):
        return False

    def shortcut(self):
        from PySide6.QtGui import QKeySequence
        keys = self.state.portable_keys()
        return QKeySequence(keys[0]) if keys else QKeySequence()

    def shortcuts(self):
        from PySide6.QtGui import QKeySequence
        return [QKeySequence(key) for key in self.state.portable_keys()]

    def setShortcuts(self, sequences):
        from views.shortcuts import portable_keys
        keys = [(parsed[0], parsed[2]) for text in portable_keys(sequences)
                if (parsed := shortcut_text(text))]
        if self.state.set_keys(keys) and self.changed is not None:
            self.changed()

    def setEnabled(self, enabled):
        if self.state.set_enabled(enabled) and self.changed is not None:
            self.changed()

    def setChecked(self, checked):
        if self.state.set_checked(checked) and self.toggled is not None:
            self.toggled(checked)

    def trigger(self):
        if self.state.isEnabled():
            self.setChecked(not self.state.isChecked())
            if self.run is not None:
                self.run()

    def remove(self):
        self.window._command_store.remove(self.state.id)
        self.window._command_actions.pop(self.state.id, None)
        self.changed = self.run = self.toggled = None
