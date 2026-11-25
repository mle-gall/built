# SPDX-License-Identifier: GPL-3.0-or-later
"""Cyclic garbage is freed on the main thread only.

A collection started on the snap-index worker freed Qt objects the tests
had let go of, off the main thread, and the wgpu batch crashed (SIGSEGV /
SIGBUS) one run in three or four."""
import gc
import threading

from built_core import install_main_thread_collector


def test_a_collection_on_a_worker_frees_nothing_the_main_thread_then_does():
    guard = install_main_thread_collector()
    assert guard in gc.callbacks
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
    guard = install_main_thread_collector()
    assert guard is install_main_thread_collector()
    assert gc.callbacks.count(guard) == 1
