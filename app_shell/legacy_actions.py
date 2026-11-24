"""Mirror shortcuts contributed as raw Qt actions into the window command store."""
from weakref import ref

from PySide6.QtCore import QEvent, QObject, Slot
from PySide6.QtGui import QAction
from shiboken6 import isValid


class ActionObserver(QObject):
    def __init__(self, window):
        super().__init__()
        self.window = ref(window)
        self.watched = set()
        self.connections = {}

    @Slot()
    def invalidate(self):
        if (window := self.window()) is not None:
            window._action_tree_changed = True

    def eventFilter(self, obj, event):
        if event.type() in (QEvent.ChildAdded, QEvent.ChildRemoved):
            self.invalidate()
        return False

    def watch(self, children):
        current = set(children)
        for obj in self.watched - current:
            connection = self.connections.pop(obj, None)
            if isValid(obj):
                obj.removeEventFilter(self)
                if connection is not None:
                    QObject.disconnect(connection)
        for obj in current - self.watched:
            obj.installEventFilter(self)
            if isinstance(obj, QAction):
                self.connections[obj] = obj.changed.connect(self.invalidate)
        self.watched = current


def sync(window):
    # ChildAdded can arrive before the QAction constructor finishes. Inspect
    # the completed tree on the next shortcut lookup, outside the Qt event.
    children = window.findChildren(QObject)
    observer = vars(window).get("_action_observer")
    if children and observer is None:
        window._action_observer = observer = ActionObserver(window)
    if observer is not None:
        observer.watch(children)
    bindings = [(seq[0].toCombined(), act) for act in children
                if isinstance(act, QAction) and act.isEnabled()
                for seq in act.shortcuts() if seq.count() == 1]
    window._command_store.replace_plugin_keys(bindings)
    window._action_tree_changed = False


def close(window):
    if (observer := vars(window).pop("_action_observer", None)) is not None:
        observer.watch(())
    window._command_store.replace_plugin_keys([])
