"""Drive a Settings unit row through the same typed field as the chrome."""
from views import settings_sheet


def units_row(window, key):
    env = settings_sheet.Env(window)
    return next(row for _, _, rows in settings_sheet._units(env) for row in rows
                if row.id == "u_" + key)


def edit_unit(window, key, raw):
    env = settings_sheet.Env(window)
    row = units_row(window, key)
    field = row.field(env)
    form = settings_sheet.cf.Form("", [field], on_change=lambda f, fid, v: row.put(env, v))
    form.receive(row.id, str(raw))
