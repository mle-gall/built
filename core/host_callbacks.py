"""Instance-local notifications for native view controllers and plugin subscribers."""


class HostService:
    """A service of one window: its timers run on that window's host queue
    (``core.timers``), the process-wide one when it has none."""

    def __init__(self, parent=None):
        self._host = getattr(parent, "_host", None)

    def deleteLater(self):
        if self._host is not None:
            self._host.timers.cancel_context(self)
        else:
            from core import timers
            timers.cancel_context(self)


class Signal:
    """``name = Signal()`` on a class: each instance gets its own
    ``ChangeCallbacks`` (``connect``, ``disconnect``, ``emit``)."""

    def __init__(self, *_payload_types):
        pass

    def __set_name__(self, owner, name):
        self.name = name

    def __get__(self, owner, _type=None):
        if owner is None:
            return self
        from ingetrazo_render import ChangeCallbacks
        callbacks = ChangeCallbacks()
        vars(owner)[self.name] = callbacks
        return callbacks
