# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Line types moved to Rust (``rust/ingetrazo_core/src/linetypes.rs``) and
must answer exactly as the Python ones did.

``tests/fixtures/linetype_golden.json.gz`` holds what the retired Python
implementation (``core/layers.py`` at ``6f3a2576``) answered for every case
of :func:`corpus`, in order. Regenerate it only on purpose, with
``python tests/test_linetype_parity.py --write`` against the reference
implementation."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "linetype_golden.json.gz"

LIN_FILES = (
    "",
    "*DASHED,Dashed __ __ __\nA,.5,-.25\n",
    "*DASHED2,Dashed (.5x) _ _ _\nA,.25,-.125\n*CENTER,Center ____ _ ____\n"
    "A,1.25,-.25,.25,-.25\n",
    ";; comment\n; another\n*BORDER,Border __ __ . __\nA,12.7,-6.35,12.7,-6.35,0,-6.35\n",
    "*GAS_LINE,Gas line ----GAS----GAS----\nA,12.7,-5.08,[\"GAS\",STANDARD,S=2.54,"
    "R=0.0,X=-2.54,Y=-1.27],-6.35\n",
    "*HOT_WATER,Hot water ---- HW ---- HW\nA,.5,-.2,[\"HW\",STANDARD,S=.1,R=0.0,"
    "X=-0.1,Y=-.05],-.2\n",
    "*FENCE,Fence ---0---0---\nA,.25,-.1,[CIRC1,ltypeshp.shx,x=-.1,s=.1],-.1,1\n"
    "*OK,ok\nA,5,-2\n",
    "*TWOTEXT,two\nA,5,[\"A\",STANDARD],-2,[\"B\",STANDARD],-2\n",
    "*LEADTEXT,lead\nA,[\"X\",STANDARD,S=3],-4,5\n",
    "*BAD,bad\nA,5,abc,-2\n*EMPTY,\nA,0,0\n*ALLGAP,gap\nA,-1,-2\n",
    "*CRLF,crlf\r\nA,6,-3\r\n*FF,form\x0cA,4,-1\n",
    "*  SPACED NAME  ,  label, with, commas  \n  a , 3 , -1.5 ,0, -1.5  \n",
    "*NOA,no a\n3,-1\n*TRAIL,trail\nA,3,-1 ;; trailing comment\n",
    "A,5,-5\n*ORPHAN\n*LATE,late\nA,1e1,-2.5e0\n",
    "*BIG,big\nA,2000,-1\n*NAN,nan\nA,nan,-1\n*INF,inf\nA,inf,-1\n",
    "*UNDERSCORE,u\nA,1_0,-2\n*SIZE,s\nA,5,-1,[\"T\",STANDARD,S=x],-1\n",
    "*" + "N" * 80 + "," + "L" * 150 + "\nA,4,-2\n",
    "*DUP,first\nA,1,-1\n*OTHER,o\nA,2,-2\n*DUP,second\nA,3,-3\n",
    "*TXT,t\nA,10,[\"VERY LONG TEXT THAT GOES ON AND ON\",STANDARD,S=50],-3\n",
    "*TXT2,t2\nA,10,-1,[\"éñ x\",STANDARD,S=0.01],-1\n",
    "*TXT3,t3\nA,10,-1,[\"\",STANDARD],-1\n*TXT4,t4\nA,10,-1,[\"Q\"],-1\n",
    "*LOWS,s\nA,10,-1,[\"Q\",STANDARD,s=4],-1\n",
)

PATTERNS = (
    (), (3.0, 1.5), (0.5, 1.0), (6.0, 1.5, 0.5, 1.5), (12.0, 1.5, 2.0, 1.5),
    (0.0, 1.0), (5.0,), (2.0, 0.0, 3.0), (0.1, 0.05), (1e-4, 1e-4),
)


def _rng():
    return random.Random(20261124)


def _pieces(rng):
    out = []
    for _ in range(rng.randint(0, 9)):
        r = rng.random()
        if r < 0.15:
            out.append(0.0)
        elif r < 0.6:
            out.append(round(rng.uniform(0.1, 12), rng.choice((0, 1, 2, 5))))
        else:
            out.append(-round(rng.uniform(0.1, 12), rng.choice((0, 1, 2, 5))))
    return out


def corpus():
    rng = _rng()
    out = [("LINETYPES", ()), ("DXF_LINETYPES", ())]
    raws = [None, "1,2", [], [1], [1, -1], [3, -1.5, 0, -1.5], ["2", "-1"],
            [1, "x"], [None, -1], [math.inf, -1], [1001, -1], [0.01, -0.01],
            [0.03, -0.03], (2, -2), [0.00004, -1], [1.23456789, -9.87654321],
            [True, -1], list(range(-20, 20)), [1, -1] * 20]
    raws += [_pieces(rng) for _ in range(60)]
    for raw in raws:
        out.append(("clean_pattern", (raw,)))
    for _ in range(80):
        p = _pieces(rng)
        out.append(("drawn_dashes", (p,)))
        out.append(("file_dashes", (p,)))
    for p in (_pieces(rng) for _ in range(40)):
        if p:
            for at in (0, len(p) - 1, rng.randrange(len(p))):
                out.append(("text_offset", (p, at)))
    entries = [None, {}, {"text": 5}, {"text": "  GAS "}, {"text": "\tWäter\x00\n"},
               {"text": "G", "text_size": "x"}, {"text": "G", "text_size": None},
               {"text": "G", "text_size": math.nan}, {"text": "G", "text_size": 0.1},
               {"text": "G", "text_size": 99}, {"text": "G", "text_size": "3.14159"},
               {"text": "G", "text_at": 0}, {"text": "G", "text_at": 1.9},
               {"text": "G", "text_at": "1"}, {"text": "G", "text_at": None},
               {"text": "G", "text_at": -1}, {"text": "G", "text_at": 9},
               {"text": "X" * 40, "text_size": 2.123456}, {"text": "   "},
               {"text": "​zero­width "}]
    for e in entries:
        for p in ([5, -2], [5, -2, 1, -3], [], None, [5, 2], [-1, 3]):
            out.append(("clean_line_text", (e, p)))
    for p in ([5, -2], [5, 2], [], [3, -1, 0, -1]):
        for text in ("", "  ", "GAS", " AGUA ", "x" * 30):
            for size in (2.5, 0.2, 7.123):
                out.append(("with_text", (p, text, size)))
    for text in ("", "A", "GAS", "éé"):
        for size in (2.5, 0.0, 3.3):
            out.append(("text_width", (text, size)))
    for _ in range(120):
        p0 = (rng.uniform(-100, 100), rng.uniform(-100, 100))
        p1 = rng.choice(((rng.uniform(-100, 100), rng.uniform(-100, 100)), p0,
                         (p0[0], p0[1] + rng.uniform(-50, 50)),
                         (p0[0] + rng.uniform(-50, 50), p0[1])))
        spec = rng.choice((None, ("GAS", 2.5, rng.uniform(0, 10), rng.uniform(5, 30)),
                           ("W", rng.uniform(0.5, 5), 0.0, rng.uniform(0, 8)),
                           ("LONG TEXT", 5.0, 3.0, 0.0)))
        out.append(("text_marks", (p0, p1, spec)))
    for _ in range(40):
        n = rng.randint(0, 3)
        segs = [[[rng.uniform(-0.6, 0.6) for _ in range(3)] for _ in range(2)]
                for _ in range(n)]
        if n and rng.random() < 0.2:
            segs[0][1] = list(segs[0][0])
        pattern = rng.choice(PATTERNS)
        unit = rng.choice((0.05, 0.2, 1.0, 0.0))
        out.append(("dash_segments", (segs, pattern, unit)))
    for text in LIN_FILES:
        out.append(("parse_lin", (text,)))
    out.append(("to_lin", ([],)))
    out.append(("to_lin", ([("dashed", "Dashed", [3.0, -1.5]),
                            ("empty", "Empty", []),
                            ("w/ odd *name*", "With, commas", [0.1234567, -1e-5, 123456789.0, -0.5]),
                            ("___", "underscores", [1, -1]),
                            ("gas", "Gas", [12.7, -11.43],
                             {"text": "GAS", "text_size": 2.54, "text_at": 1}),
                            ("q", 'Quote "x"', [5, -4, 2, -6],
                             {"text": 'say "hi"', "text_size": 2.5, "text_at": 3}),
                            ("none", "None spec", [5, -2], None)],)))
    for text in LIN_FILES:
        out.append(("lin_round_trip", (text,)))
    return out


def answer(layers, name, args):
    try:
        if name in ("LINETYPES", "DXF_LINETYPES"):
            return getattr(layers, name)
        if name == "dash_segments":
            dashes, gaps = layers.dash_segments(*args)
            return [str(dashes.dtype), list(dashes.shape), dashes.tolist(),
                    str(gaps.dtype), list(gaps.shape), gaps.tolist()]
        if name == "lin_round_trip":
            found, _ = layers.parse_lin(args[0])
            rows = []
            for n, e in found.items():
                spec = layers.clean_line_text(e, e["pattern"])
                rows.append((n, e["label"], e["pattern"], spec))
            return layers.to_lin(rows)
        return getattr(layers, name)(*args)
    except Exception as exc:  # noqa: BLE001 — the error is part of the answer
        return ["error", type(exc).__name__]


def _jsonable(x):
    if isinstance(x, float) and not math.isfinite(x):
        return repr(x)
    if isinstance(x, (list, tuple)):
        return [_jsonable(i) for i in x]
    if isinstance(x, dict):
        return {str(k): _jsonable(v) for k, v in x.items()}
    return x


def test_line_types_answer_as_the_python_ones_did():
    from core import layers
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    cases = corpus()
    assert len(cases) == len(want)
    wrong = []
    for (name, args), expected in zip(cases, want):
        got = _jsonable(answer(layers, name, args))
        if got != expected:
            wrong.append(f"{name}{_jsonable(list(args))!r}:\n  {got!r}\n  != {expected!r}")
    assert not wrong, f"{len(wrong)} of {len(cases)} differ:\n" + "\n".join(wrong[:30])


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import layers as _layers
    rows = [_jsonable(answer(_layers, n, a)) for n, a in corpus()]
    GOLDEN.write_bytes(gzip.compress(json.dumps(
        rows, ensure_ascii=False, separators=(",", ":")).encode(), mtime=0))
    print(len(rows), "cases ->", GOLDEN)
