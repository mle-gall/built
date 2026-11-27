"""Measure a committed profile edit on 200 individually notched, anchored purlins."""
import json
from pathlib import Path
from statistics import median
from tempfile import TemporaryDirectory
from time import perf_counter

from built_core.history import History
from built_core.protocol import commands, queries
from built_core.scene import Scene
from formats import igz
from repeat_storyboard import build


def measure(folder, samples=15):
    build(folder)
    scene = Scene()
    igz.load_into(scene, folder / "portal.built")
    history = History(scene)

    def apply(verb, **params):
        result = commands.run(history, verb, **params)
        if not result["ok"]:
            raise ValueError(result)

    repeat = next(r for r in queries.live_rows(scene) if r["uid"] == f"{210:016x}")
    apply("create_sweeps", sweeps=[{
        "profile": {"library": "basic/timber", "params": {"width": 120, "height": 55}},
        "path": {"points": [[-.2, .3, 3.39], [3.65, .3, 4.82]]}, "attachment": "centre",
    }])
    apply("live_set", row={"kind": "cut", "uid": f"{399:016x}", "owner": repeat["source"],
                           "operation": "subtract", "cutter": {"body": scene.groups[-1].uid}})
    apply("live_set", row=dict(repeat, rule={"divide": 200}))
    assembly = next(g for g in scene.groups if g.uid == repeat["owner"])
    source = next(r for r in queries.live_rows(scene)
                  if r["owner"] == repeat["source"] and r["kind"] == "sweep")
    elapsed = []
    for i in range(samples):
        source["profile"]["params"]["width"] = 91 + i % 5
        start = perf_counter()
        apply("live_set", row=source)
        elapsed.append(round((perf_counter() - start) * 1000, 2))
    marker = assembly.ext["built.live"]
    assert marker["copies"] == marker["distinct"] == 200
    assert all(r["state"] == "ok" for r in queries.live_rows(scene))
    assert all(len(c.mesh.faces) > 6 and not c.ext.get("built.repeat_state") for c in assembly.children)
    return {"copies": marker["copies"], "distinct": marker["distinct"], "samples_ms": elapsed,
            "median_ms": median(elapsed), "max_ms": max(elapsed)}


if __name__ == "__main__":
    with TemporaryDirectory() as folder:
        print(json.dumps(measure(Path(folder)), indent=2))
