# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Opening a document must let the previous one go.

The first release check (0.5.3, 25-09-2026) found the viewport's chunk
caches — keyed by id() of groups and meshes — never emptied: every New /
Open kept the old document's chunks and, through their pick arrays, its
faces. Five reopenings of the Plaza Yanque took 0.5.2 from 610 to 1440 MB."""
from __future__ import annotations

from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
import gc
from pathlib import Path


DOC = (Path(__file__).resolve().parents[1] / "examples"
       / "banca-pergola-yanque.igz")


def _live_faces() -> int:
    gc.collect()
    return sum(1 for o in gc.get_objects() if type(o).__name__ == "Face")


def test_reopening_a_document_keeps_no_chunk_or_face_of_the_old_one(native_window):
    win = native_window
    vp = win.viewport
    counts = []
    for _ in range(3):
        win.open_path(DOC)
        for g in vp._placements():          # what a paint would build
            vp._group_chunk(g)
        counts.append((len(vp._group_chunks), _live_faces()))
    assert counts[0][0] > 0
    assert counts[1] == counts[2]            # nothing piles up
    assert counts[2][0] == counts[0][0]
