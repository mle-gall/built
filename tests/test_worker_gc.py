# SPDX-License-Identifier: GPL-3.0-or-later
"""Cyclic garbage is freed on the main thread only (core/worker_gc.py).

A collection started on the snap-index worker freed Qt objects the tests
had let go of, off the main thread, and the wgpu batch crashed (SIGSEGV /
SIGBUS) one run in three or four."""
import gc
import threading

from core import worker_gc


def test_a_collection_on_a_worker_frees_nothing_the_main_thread_then_does():
    worker_gc.install()
    assert worker_gc._on_collect in gc.callbacks
    freed = []

    class Cycle:
        def __del__(self):
            freed.append(threading.get_ident())

    a = Cycle()
    a.me = a
    del a
    worker = threading.Thread(target=gc.collect)
    worker.start()
    worker.join()
    assert freed == []
    assert gc.get_freeze_count() > 0
    gc.collect()
    assert freed == [threading.get_ident()]
    assert gc.get_freeze_count() == 0


def test_install_is_idempotent():
    worker_gc.install()
    worker_gc.install()
    assert gc.callbacks.count(worker_gc._on_collect) == 1
