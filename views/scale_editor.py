"""Native scale form and the shared group scale command."""
from core.group_scale import group_scale
from core.i18n import tr


def _fmt(value: float) -> str:
    return f"{value:.6g}"


def _set_scale(viewport, group, factors):
    from core.protocol.commands import run
    try:
        run(viewport.history, "set_group_scale", placement=group.uid, factors=factors)
    except KeyError:
        # Undo can remove this placement while its editor stays open.
        return
    viewport.update()


def ask_scale(window, group, done=None) -> None:
    from views.chrome_dialogs import in_shell
    if not in_shell():
        from views.scale_dialog import ScaleDialog
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
