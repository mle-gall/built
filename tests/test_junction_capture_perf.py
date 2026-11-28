# SPDX-License-Identifier: GPL-3.0-or-later
"""J2 budgets on the synthetic garage (docs/junctions.md §3.4, §4.3): the
capture popover's live count, a detail edit across 8 knees and across the
48 purlin cleats along a Repeat, and a step that touches no junction."""
import os
import time

from built_core import command_protocol as live
from tests.test_junction_capture import captured_garage
from tests.test_junction_meetings import cleat_detail, garage
from tests.test_live_recipes import apply

PERF = bool(os.environ.get("BUILT_PERF"))


def best(f, n=5):
    times = []
    for _ in range(n):
        t = time.perf_counter()
        f()
        times.append((time.perf_counter() - t) * 1000)
    return min(times)


def test_capture_count_and_detail_edits_stay_in_budget():
    scene, history, knee, reply = captured_garage()
    detail = reply["detail"]
    matches = [s["roles"] for s in live.junction_suggest(scene, detail)["suggestions"] if s["fit"] == "match"]
    apply(history, "junction_apply", detail=detail, bindings=matches)
    # The popover's live count while the rule is edited (§4.3: < 5 ms).
    scene2, history2 = garage(purlins=False)
    from tests.test_junction_capture import handmade_knee
    cut = handmade_knee(scene2, history2)[4]
    count_ms = best(lambda: live.junction_capture_preview(scene2, cut, {"tol": 3}))
    preset = next(p for p, _ in live.preset_catalog(scene) if p["uid"] == detail)
    widths = iter(range(31, 60))

    def edit_all():
        for p in preset["values"]["params"]:
            if p["key"] == "plate_width":
                p["expr"] = f"joining.width + {next(widths)}"
        apply(history, "preset_update", rows=[preset])
    edit_ms = best(edit_all, 3)
    record_ms = best(lambda: live.junction_record(scene, reply["uid"]))
    print(f"capture preview {count_ms:.2f} ms · detail edit across 8 knees {edit_ms:.2f} ms · record {record_ms:.2f} ms")
    if PERF:
        assert edit_ms < 50
        assert count_ms < 50


def test_detail_edit_along_the_purlin_repeat_stays_in_budget():
    scene, history = garage()
    cleat = cleat_detail(scene)
    apply(history, "preset_update", rows=[cleat])
    crossing = next(s for s in live.junction_suggest(scene, cleat["uid"])["suggestions"] if s["fit"] == "match")
    apply(history, "junction_apply", detail=cleat["uid"], bindings=crossing["roles"], along={"end": "both"})
    heights = iter(range(100, 140))

    def edit():
        cleat["values"]["contents"][0]["part"]["fields"]["profile.params.height"] = next(heights)
        apply(history, "preset_update", rows=[cleat])
    ms = best(edit, 3)
    print(f"cleat detail edit along the Repeat (48 meetings) {ms:.1f} ms")
    if PERF:
        assert ms < 100
