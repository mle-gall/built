"""Schedule on the owner's window: its host's timer queue (``WindowTimers``,
drained by ``Host.idle``). An owner with no host (headless CPython, a test
with no window) schedules on one process-wide queue, which every host's
``idle`` drains too and a headless caller drains with :func:`run_due`."""
from ingetrazo_render import WindowTimers

_detached = WindowTimers()


def _queue(owner):
    host = getattr(owner, "_host", None)
    queue = getattr(host, "timers", None)
    return queue if queue is not None else _detached


def timer(owner, callback, interval=0, repeat=False):
    return _queue(owner).timer(callback, interval, repeat, owner)


def later(owner, delay, callback):
    _queue(owner).later(owner, delay, callback)


def run_due() -> None:
    """Run the hostless timers that are due."""
    _detached.run_due()


def next_ms():
    """Milliseconds until the next hostless timer, or ``None``."""
    return _detached.next_ms()


def cancel_context(context) -> None:
    """Drop the hostless timers scheduled for ``context``."""
    _detached.cancel_context(context)
