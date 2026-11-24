"""Schedule on the owner's window. Widget hosts keep their own event loop."""
def timer(owner, callback, interval=0, repeat=False):
    host = getattr(owner, "_host", None)
    if (queue := getattr(host, "timers", None)) is not None:
        return queue.timer(callback, interval, repeat, owner)
    from PySide6.QtCore import QTimer
    handle = QTimer(owner, singleShot=not repeat, interval=interval)
    handle.timeout.connect(callback)
    return handle


def later(owner, delay, callback):
    host = getattr(owner, "_host", None)
    if (queue := getattr(host, "timers", None)) is not None:
        queue.later(owner, delay, callback)
    else:
        from PySide6.QtCore import QTimer
        QTimer.singleShot(delay, owner, callback)
