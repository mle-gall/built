# SPDX-License-Identifier: GPL-3.0-or-later
"""Run inside Blender by test_blender_link_e2e.py:

    blender -b --factory-startup --python blender_link_driver.py -- ADDON_PARENT PAIRING EXPECTED REPORT

Links with the add-on, waits for the model, reads it back, changes a
placement, a material and adds a light, sends them and waits for Built's
answers. Background Blender has no event loop, so the driver ticks the
controller itself. Writes a JSON report.
"""
import json
import os
import sys
import time

import bpy

argv = sys.argv[sys.argv.index("--") + 1:]
addon_parent, pairing, expected, report_path = argv[0], argv[1], int(argv[2]), argv[3]
sys.path.insert(0, addon_parent)
for o in list(bpy.data.objects):                 # the factory cube, camera and light
    bpy.data.objects.remove(o, do_unlink=True)

import built_link  # noqa: E402

built_link.register()
from built_link import ui  # noqa: E402

report = {"steps": []}


def pump(until, timeout=30.0):
    c = ui.controller()
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        c.tick(budget=0.2)
        if until(c):
            return True
        time.sleep(0.02)
    return False


try:
    ui.link(pairing)
    c = ui.controller()
    report["linked"] = pump(lambda c: c.state == "linked")
    report["model"] = pump(lambda c: c.progress is None and len(c.host.roots()) >= expected and c.last_sync)
    roots = c.host.roots()
    report["roots"] = {uid: {"name": o.name, "fp": o.get("built_fp"), "matrix": [list(r) for r in o.matrix_world],
                             "children": len(o.children_recursive), "type": o.type}
                       for uid, o in roots.items()}
    report["materials"] = {m.get("built_material"): c.host._material(m)
                           for m in bpy.data.materials if m.get("built_material")}
    report["lights"] = [o.get("built_light") for o in bpy.context.scene.objects if o.type == "LIGHT"]
    report["cameras"] = [o.get("built_camera") for o in bpy.context.scene.objects if o.type == "CAMERA"]
    report["document"] = bpy.context.scene.get("built_document")
    report["changes_before_edit"] = len(c.changes()[0])

    if os.environ.get("DRIVER_MODE") == "newmaterial":
        # A Principled material with a base colour image (never saved: the
        # add-on packs it) and a roughness value, on the Leg placement.
        image = bpy.data.images.new("checker", 8, 8)
        image.pixels = [v for i in range(64) for v in ((0.9, 0.2, 0.1, 1.0) if (i + i // 8) % 2 else (0.1, 0.2, 0.9, 1.0))]
        mat = bpy.data.materials.new("Checker")
        mat.use_nodes = True
        tree = mat.node_tree
        bsdf = next(n for n in tree.nodes if n.type == "BSDF_PRINCIPLED")
        tex = tree.nodes.new("ShaderNodeTexImage")
        tex.image = image
        tree.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = 0.3
        leg = next(o for o in roots.values() if o.name.startswith("Leg"))
        leg.data.materials.clear()
        leg.data.materials.append(mat)
        bpy.context.view_layer.update()
        report["new_changes"] = [i["key"] for i in c.changes()[0]]
        report["sent"] = c.send()
        report["answered"] = pump(lambda c: not c.sync.sent, timeout=60)
        report["adopted"] = mat.get("built_material")
        report["changes_after"] = [i["key"] for i in c.changes()[0]]
        c.unlink()
        raise SystemExit
    seat = next(o for o in roots.values() if o.name.startswith("Seat"))
    seat.location.x += 0.5
    oak = next(m for m in bpy.data.materials if m.get("built_material") == "Oak")
    bsdf = next(n for n in oak.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    bsdf.inputs["Roughness"].default_value = 0.3
    data = bpy.data.lights.new("Studio key", type="POINT")
    data.energy = 700.0
    key = bpy.data.objects.new("Studio key", data)
    bpy.context.scene.collection.objects.link(key)
    key.location = (0.0, -2.0, 3.0)
    bpy.context.view_layer.update()
    report["sent"] = c.send()
    report["answered"] = pump(lambda c: not c.sync.sent, timeout=60)
    bpy.context.view_layer.update()
    report["changes_after"] = [i["key"] for i in c.changes()[0]]
    report["seat_x"] = seat.matrix_world.translation.x
    c.unlink()
except SystemExit:
    pass
except Exception as exc:  # report, do not hang Blender
    import traceback
    report["error"] = traceback.format_exc()
    print(exc)
with open(report_path, "w", encoding="utf-8") as fh:
    json.dump(report, fh, indent=1, default=str)
