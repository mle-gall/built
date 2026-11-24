"""Exercise the same Settings fields and action callbacks as the native sheet."""
from pathlib import Path

import pytest

from PySide6.QtCore import QSettings

from ingetrazo_render.chrome_sheet import Ctx
from views.settings_sheet import Env, SettingsSheet


@pytest.fixture
def settings_file(private_settings):
    # Qt with no organisation set cannot persist even an INI preference.
    from PySide6.QtCore import QCoreApplication
    previous = QCoreApplication.organizationName()
    if not previous:
        QCoreApplication.setOrganizationName("IngeTrazo")
    try:
        yield Path(QSettings().fileName())
    finally:
        QCoreApplication.setOrganizationName(previous)


class Controls:
    def __init__(self, window, section):
        self.ctx = Ctx(window, None)
        self.sheet = SettingsSheet(section)
        self.env = Env(window)

    def select(self, section):
        self.sheet.goto(section)
        return self

    def wire(self):
        return self.sheet.build(self.ctx).wire()

    def row(self, key):
        for _, _, rows in self.sheet.groups(self.env, self.sheet.section):
            for row in rows:
                if row.id == key:
                    return row
        raise KeyError(key)

    def value(self, key):
        return self.row(key).value(self.env)

    def options(self, key):
        return dict(self.row(key).opts(self.env))

    def enabled(self, key):
        return self.row(key).on("enabled", self.env)

    def visible(self, key):
        return self.row(key).on("visible", self.env)

    def set(self, key, value):
        page = self.sheet.build(self.ctx)
        form = next(f for f in page.forms() if f.field(key) is not None)
        row = self.sheet._rows[key]
        assert row.on("enabled", self.env)
        if row.kind == "choice":
            value = list(dict(row.opts(self.env))).index(value)
        elif row.kind == "check":
            value = int(value)
        form.receive(key, str(value))

    def restore(self):
        self.sheet.restore_defaults(self.ctx.win)
