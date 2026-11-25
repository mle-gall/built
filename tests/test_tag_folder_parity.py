# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The tag-folder tree's queries moved to Rust
(``rust/built_core/src/tag_folders.rs``: a folder's parent, its
subfolders, the Layers tab's order) and must answer as ``core/layers.py``'s
Python did at ``74b54c81``.

``tests/fixtures/tag_folder_golden.json.gz`` holds those answers on seeded
trees (parents missing, looping, pointing at themselves, folders named only
by a tag, tags in an empty-named folder, duplicate tag keys). Regenerate
only on purpose, against the reference implementation:
``python tests/test_tag_folder_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "tag_folder_golden.json.gz"


class _Tag:
    def __init__(self, key, folder):
        self.key, self.name, self.folder = key, key, folder


class _Scene:
    def __init__(self, layers, folders, parents):
        self.layers, self.layer_folders, self.folder_parents = layers, folders, parents
        self.layer_folder_uids = {}

    def layer(self, ref):
        return next((ly for ly in self.layers if ly.key == ref), None)


def scenes():
    rng = random.Random(20261124)
    out = []
    for k in range(80):
        n = rng.randint(0, 14)
        folders = [f"F{i}" for i in range(n)]
        rng.shuffle(folders)
        parents = {}
        for f in folders:
            r = rng.random()
            if r < 0.5 and folders:
                parents[f] = rng.choice(folders)          # loops and self-parents too
            elif r < 0.6:
                parents[f] = "Gone"
            elif r < 0.65:
                parents[f] = None
        listed = [f for f in folders if rng.random() < 0.8]
        if rng.random() < 0.2:
            listed.append("")
        layers = [_Tag("Layer 0", None)]
        for t in range(rng.randint(0, 25)):
            folder = rng.choice([None] + folders + (["", "Tagonly"] if rng.random() < 0.2 else []))
            layers.append(_Tag(rng.choice([f"T{t}", f"T{t}", f"T{t // 2}"]), folder))
        if rng.random() < 0.1:
            layers = layers[1:]
        out.append(_Scene(layers, listed, parents))
    return out


def answer(layers, scene):
    folders = layers.folders_of(scene)
    return {"folders": folders,
            "parents": [layers.folder_parent(scene, f) for f in folders + ["Gone", "Tagonly"]],
            "sub": [layers.subfolders(scene, f) for f in [None] + folders],
            "deep": [layers.subfolders(scene, f, deep=True) for f in [None] + folders],
            "in": [layers.layers_in_folder(scene, f) for f in folders],
            "order": layers.ordered_names(scene)}


def test_the_folder_tree_answers_as_the_python_one_did():
    from core import layers
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = [answer(layers, s) for s in scenes()]
    assert len(got) == len(want)
    wrong = [f"scene {i} {k}" for i, (g, w) in enumerate(zip(got, want)) for k in w if g[k] != w[k]]
    assert not wrong, f"{len(wrong)} answers differ: {wrong[:20]}"


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import layers as _layers
    rows = [answer(_layers, s) for s in scenes()]
    GOLDEN.write_bytes(gzip.compress(json.dumps(rows, separators=(",", ":")).encode(),
                                     mtime=0))
    print(len(rows), "scenes ->", GOLDEN, GOLDEN.stat().st_size)
