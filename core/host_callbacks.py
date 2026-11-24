"""Instance-local notifications for native view controllers and plugin subscribers."""
from core.input_types import NATIVE

if not NATIVE:
    from PySide6.QtCore import Signal
else:
    class Signal:
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
