"""Scale… — a modal with the X, Y, Z scale of one group or component,
edited in place the way Blender's N panel does."""
from __future__ import annotations

from PySide6.QtCore import QEvent
from PySide6.QtGui import QKeySequence, QShortcut
from PySide6.QtWidgets import (QDialog, QFormLayout, QLineEdit, QPushButton,
                               QVBoxLayout)

from core.group_scale import group_scale
from core.i18n import tr


def _fmt(value: float) -> str:
    return f"{value:.6g}"


class _Field(QLineEdit):
    """A line edit that leaves Ctrl+Z / Ctrl+Y to the dialog's shortcuts
    (the model's history) instead of undoing its own text."""

    def event(self, e) -> bool:
        if e.type() == QEvent.ShortcutOverride and (
                e.matches(QKeySequence.Undo) or e.matches(QKeySequence.Redo)):
            e.ignore()
            return True
        return super().event(e)


def _set_scale(viewport, group, factors):
    from core.protocol.commands import run
    try:
        run(viewport.history, "set_group_scale", placement=group.uid, factors=factors)
    except KeyError:
        # Undo can remove this placement while its editor stays open.
        return
    viewport.update()


class ScaleDialog(QDialog):
    def __init__(self, window, group) -> None:
        super().__init__(window)
        self._window = window
        self._group = group
        self.setWindowTitle(tr("Scale"))
        self.setModal(True)
        form = QFormLayout()
        self.fields: list[QLineEdit] = []
        for axis in "XYZ":
            edit = _Field()
            edit.editingFinished.connect(self._apply)
            form.addRow(axis, edit)
            self.fields.append(edit)
        self.reset_button = QPushButton(tr("Reset scale"))
        self.reset_button.clicked.connect(self._reset)
        self.close_button = QPushButton(tr("Close"))
        self.close_button.clicked.connect(self.accept)
        # Enter belongs to the fields, not to a default button.
        for b in (self.reset_button, self.close_button):
            b.setAutoDefault(False)
            b.setDefault(False)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(self.reset_button)
        layout.addWidget(self.close_button)
        for seq, step in ((QKeySequence.Undo, "undo"),
                          (QKeySequence.Redo, "redo"),
                          (QKeySequence("Ctrl+Shift+Z"), "redo")):
            QShortcut(seq, self, lambda s=step: self._history(s))
        self._refresh()

    def _history(self, step: str) -> None:
        vp = self._window.viewport
        getattr(vp.history, step)()
        vp.update()
        self._refresh()

    def _refresh(self) -> None:
        for edit, value in zip(self.fields, group_scale(self._group)):
            edit.setText(_fmt(value))

    def _typed(self):
        try:
            return tuple(float(e.text().replace(",", ".").strip())
                         for e in self.fields)
        except ValueError:
            return None

    def _set(self, target) -> None:
        if target:
            _set_scale(self._window.viewport, self._group, target)
        self._refresh()      # invalid or unchanged input snaps back

    def _apply(self) -> None:
        now = group_scale(self._group)
        typed = self._typed()
        # A field left as shown keeps the exact value, not its rounded text.
        if typed is not None:
            typed = tuple(n if _fmt(n) == _fmt(t) else t
                          for t, n in zip(typed, now))
        self._set(typed)

    def _reset(self) -> None:
        self._set((1.0, 1.0, 1.0))


def ask_scale(window, group, done=None) -> None:
    from views.chrome_dialogs import in_shell
    if not in_shell():
        ScaleDialog(window, group).exec()
        if done:
            done()
        return
    from views.chrome_forms import Field, Form, show
    def refresh(form):
        for axis, value in zip("xyz", group_scale(group)):
            form.set(axis, _fmt(value))

    def apply(form, factors):
        _set_scale(window.viewport, group, factors)
        refresh(form)

    def change(form, field, value):
        values = list(group_scale(group))
        try:
            values["xyz".index(field)] = float(value.replace(",", "."))
        except ValueError:
            refresh(form)
            return
        import math
        if all(map(math.isfinite, values)):
            apply(form, values)
        else:
            refresh(form)

    def action(form, key):
        if key == "reset":
            apply(form, (1., 1., 1.))
        else:
            getattr(window.viewport.history, key)()
            refresh(form)
            window.viewport.update()

    fields = [Field(key, "edit", key.upper(), _fmt(value), commit_on_blur=True)
              for key, value in zip("xyz", group_scale(group))]
    fields.append(Field("reset", "button", tr("Reset scale")))
    fields.extend([Field("undo", "button", tr("Undo")), Field("redo", "button", tr("Redo"))])
    show(window, Form(tr("Scale"), fields, buttons=[("ok", tr("Close"))], default=None, cancel="ok",
                      on_change=change, on_button=action, on_done=lambda *_: done() if done else None))
