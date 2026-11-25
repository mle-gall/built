# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The tag (layer) edits moved to Rust (``rust/ingetrazo_core/src/model/tag_edits.rs``)
and must answer as ``core/layers.py``'s Python did at ``9cf53080``: the folder
edits (add, move, rename, remove, file tags), the labels and paths, the
display state (visibility, lock, isolate, the active tag, scenes hiding a
tag), the templates (``template_of``, ``template_for``, ``matched_template``,
``template_name_map``, ``apply_template``, ``add_missing``, ``remove_added``),
the document's line types brought in, and a tag's archive (``to_dict`` /
``from_dict``).

``tests/fixtures/tag_edits_golden.json.gz`` holds, for seeded documents and
seeded runs of those operations, every answer and the document's tag state
after each step. Uids are random, so they are compared by order of first
appearance. Regenerate only on purpose, against the reference
implementation: ``python tests/test_tag_edits_parity.py --write``."""
from __future__ import annotations

import gzip

import ingetrazo_core
import json
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "tag_edits_golden.json.gz"

NAMES = ["Walls", "Steel", "Beams", "Doors", "Glass", "Notes", "Survey", "Roof",
         "Beams", "Walls 1", "Layer 0", "Ü"]
FOLDERS = ["Structure", "Steel", "Building A", "Building B", "Annex", "Structure 1"]


def _scene(rng):
    from core.layers import Layer
    from ingetrazo_core.saved_views import SavedView
    from core.scene import Scene
    s = Scene()
    for k in range(rng.randint(2, 9)):
        name = rng.choice(NAMES)
        if name == "Layer 0":
            continue
        kw = {}
        if rng.random() < 0.3:
            kw["key"] = f"{name}#{k}"
        if rng.random() < 0.5:
            kw["color"] = tuple(round(rng.random(), 3) for _ in range(3))
        if rng.random() < 0.3:
            kw["linetype"] = rng.choice(("dashed", "dot", "lin:mine", "nonsense"))
        if rng.random() < 0.3:
            kw["lineweight"] = rng.choice((0.25, 0.5, 1.0))
        if rng.random() < 0.2:
            kw["opacity"] = 0.5
        if rng.random() < 0.2:
            kw["hatch"] = rng.choice(("brick", "concrete", "insulation"))
            kw["hatch_angle"] = rng.choice((0.0, 45.0))
        kw["visible"] = rng.random() < 0.85
        kw["locked"] = rng.random() < 0.15
        if s.layer_by_key(kw.get("key", name)) is not None:
            continue
        s.layers.append(Layer(name, **kw))
    for f in rng.sample(FOLDERS, rng.randint(0, 4)):
        s.layer_folders = list(s.layer_folders) + [f]
    folders = list(s.layer_folders)
    for ly in list(s.layers)[1:]:
        if folders and rng.random() < 0.5:
            ly.folder = rng.choice(folders)
    if len(folders) > 1 and rng.random() < 0.6:
        s.folder_parents = {folders[1]: folders[0]}
    if folders and rng.random() < 0.4:
        s.layer_folder_names = {folders[0]: "Label " + folders[0]}
    if folders and rng.random() < 0.4:
        s.folder_visibility = {folders[-1]: False}
    if rng.random() < 0.4:
        s.line_types = {"mine": {"label": "Mine", "pattern": [5.0, -2.0]}}
    if rng.random() < 0.3:
        s.linetype_colors = {"dashed": (1.0, 0.0, 0.0)}
    if rng.random() < 0.3:
        s.active_layer = rng.choice([ly.key for ly in s.layers])
    for i in range(rng.randint(0, 2)):
        s.saved_views.append(SavedView(f"S{i}", hidden_layers=[s.layers[-1].key]))
    return s


class _Uids:
    def __init__(self):
        self.seen = {}

    def __call__(self, x):
        if isinstance(x, dict):
            return {self(k): self(v) for k, v in x.items()}
        if isinstance(x, (list, tuple)):
            return [self(v) for v in x]
        if isinstance(x, str) and len(x) >= 16 and x.isalnum() and any(c.isdigit() for c in x):
            return "uid%d" % self.seen.setdefault(x, len(self.seen))
        return x


def _state(s, uids):
    return uids({"layers": [ly.to_dict() for ly in s.layers],
                 "folders": list(s.layer_folders), "parents": dict(s.folder_parents),
                 "names": dict(s.layer_folder_names), "uids": dict(s.layer_folder_uids),
                 "fvis": dict(s.folder_visibility), "active": s.active_layer,
                 "line_types": dict(s.line_types),
                 "ltc": {k: list(v) for k, v in s.linetype_colors.items()},
                 "views": [list(v.hidden_layers) for v in s.saved_views]})


def _pick(rng, s):
    keys = [ly.key for ly in s.layers] + [ly.name for ly in s.layers] + ["missing", None]
    return rng.choice(keys)


def run(L, seed):
    rng = random.Random(seed)
    s, other = _scene(rng), _scene(rng)
    uids = _Uids()
    _state(other, uids)
    out = [_state(s, uids)]

    def step(name, value):
        out.append([name, uids(value), _state(s, uids)])

    for _ in range(14):
        folders = L.folders_of(s) + ["nowhere", None]
        op = rng.randrange(22)
        try:
            if op == 0:
                step("add", ingetrazo_core.tag_edits.add_folder(s, rng.choice(FOLDERS), rng.choice(folders)))
            elif op == 1:
                step("move", ingetrazo_core.tag_edits.move_folder(s, rng.choice(folders[:-1] or ["x"]), rng.choice(folders)))
            elif op == 2:
                step("rename", ingetrazo_core.tag_edits.rename_folder(s, rng.choice(folders[:-1] or ["x"]),
                                               rng.choice(("New", "", "  ", "Steel"))))
            elif op == 3 and len(folders) > 2:
                step("remove", ingetrazo_core.tag_edits.remove_folder(s, rng.choice(folders[:-2])))
            elif op == 4:
                step("file", ingetrazo_core.tag_edits.move_to_folder(s, [_pick(rng, s), _pick(rng, s)], rng.choice(folders)))
            elif op == 5:
                step("vis", ingetrazo_core.tag_edits.set_visible(s, [_pick(rng, s)], rng.random() < 0.5))
            elif op == 6:
                step("lock", ingetrazo_core.tag_edits.set_locked(s, [_pick(rng, s)], rng.random() < 0.5))
            elif op == 7:
                step("isolate", ingetrazo_core.tag_edits.isolate(s, [_pick(rng, s)]))
            elif op == 8:
                step("isolated", [ingetrazo_core.tag_edits.is_isolated(s, ly.key) for ly in s.layers])
            elif op == 9:
                step("active", ingetrazo_core.tag_edits.set_active(s, _pick(rng, s)))
            elif op == 10:
                st = ingetrazo_core.tag_edits.layer_state(s)
                ingetrazo_core.tag_edits.set_visible(s, [ly.key for ly in s.layers], False)
                ingetrazo_core.tag_edits.apply_layer_state(s, st)
                step("state", st)
            elif op == 11:
                step("scenes", ingetrazo_core.tag_edits.hide_in_scenes(s, [_pick(rng, s)], rng.random() < 0.7))
            elif op == 12:
                step("template_of", ingetrazo_core.tag_edits.template_of(s))
            elif op == 13:
                step("template_for", ingetrazo_core.tag_edits.template_for(s, [_pick(rng, s), _pick(rng, s)]))
            elif op == 14:
                t = ingetrazo_core.tag_edits.template_of(other)
                step("matched", ingetrazo_core.tag_edits.matched_template(s, t))
                step("name_map", ingetrazo_core.tag_edits.template_name_map(s, t))
            elif op == 15:
                step("apply", ingetrazo_core.tag_edits.apply_template(s, ingetrazo_core.tag_edits.template_of(other)))
            elif op == 16:
                added = ingetrazo_core.tag_edits.add_missing(s, ingetrazo_core.tag_edits.template_for(other, [ly.key for ly in other.layers]))
                step("add_missing", added)
                if rng.random() < 0.6:
                    step("remove_added", ingetrazo_core.tag_edits.remove_added(s, *added))
            elif op == 17:
                step("line_types", ingetrazo_core.tag_edits.load_line_types(s, rng.choice((
                    None, {"a": {"pattern": [1, -1]}, "": {}, "b": {"label": 3, "pattern": "x"},
                           "c" * 70: {"label": "L" * 200, "pattern": [2, -1, 0, -1],
                                      "color": [1, 0, 0]}}))))
            elif op == 18:
                step("labels", [ingetrazo_core.tag_edits.path_label(s, _pick(rng, s)), ingetrazo_core.tag_edits.layer_label(s, _pick(rng, s)),
                                ingetrazo_core.tag_edits.export_labels(s), [ingetrazo_core.tag_edits.folder_label(s, f) for f in folders],
                                [ingetrazo_core.tag_edits.folder_visible(s, f) for f in folders],
                                [ingetrazo_core.tag_edits.effective_visible(s, ly) for ly in s.layers],
                                L.active_layer(s), ingetrazo_core.tag_edits.unique_name(folders[:-1], "Folder")])
            elif op == 19:
                step("in_folder", [L.layers_in_folder(s, f) for f in folders[:-1]])
            elif op == 20:
                raw = rng.choice((ingetrazo_core.tag_edits.template_of(other)["layers"] or [{}]))
                bad = rng.choice(({}, {"name": 3}, {"name": "X", "color": [2, "a", 0]},
                                  {"name": "Y", "lineweight": "x", "opacity": None,
                                   "hatch_on_paper": 1, "printable": 0, "key": ""}))
                step("archive", [L.Layer.from_dict(raw).to_dict(),
                                 L.Layer.from_dict(bad).to_dict()])
            elif op == 21:
                step("apply_bad", ingetrazo_core.tag_edits.apply_template(s, rng.choice(({}, {"ingetrazo_layers": 1},
                                                                  []))))
        except ValueError as e:
            step("ValueError", str(e))
        except TypeError:
            step("TypeError", None)
    return out


def corpus(L):
    return [run(L, seed) for seed in range(60)]


def test_the_tag_edits_answer_as_the_python_did():
    from core import layers
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(corpus(layers)))
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        for j, (a, b) in enumerate(zip(g, w)):
            assert a == b, f"case {i} step {j}: {b[0] if j else 'start'}"
        assert len(g) == len(w)


def test_the_corpus_reaches_every_operation():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    ops = {s[0] for case in want for s in case[1:]}
    assert {"add", "move", "rename", "remove", "file", "apply", "add_missing",
            "remove_added", "template_for", "matched", "line_types", "archive",
            "ValueError"} <= ops


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import layers
    data = corpus(layers)
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN} ({len(data)} cases)")
