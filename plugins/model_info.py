# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Ahsan Mehmood (OpenSKP) — IngeTrazo plugin contribution.
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Model Info plugin — model statistics in a dialog (Extensions menu).

A read-only, tabbed breakdown of the open document: geometry counts,
bounding box, materials in use, layers, and BIM objects. The numbers agree
with the rest of the application because they come from the same sources:

- geometry reads ``scene.loose_mesh`` (NOT ``scene.mesh``, which is swapped
  while a group is open for editing — reading it would double-count);
- materials mirror the Paint tray's idiom: ``attrs["color"]`` (floats 0–1)
  and ``attrs["texture"]`` on each face — IngeTrazo has no material
  registry to query;
- BIM objects come from :func:`core.bim.collect_objects`, the same call
  behind the BIM tray and the IFC export, so a six-face wall with one tag
  is ONE ``IfcWall`` here too — with its area and (when the face set is
  watertight) its volume;
- lengths are formatted by the dimension style (unit + precision), so the
  dialog speaks the same units as the rest of the document.
"""
from __future__ import annotations

from pathlib import Path

from core import bim
from core.i18n import tr
from tools.base import Tool
from core.units import fmt_area, fmt_volume


# ---------------------------------------------------------------------------
# Statistics collection (pure data, no GUI — see tests/test_model_info_plugin)
# ---------------------------------------------------------------------------

def _fmt_len(metres: float, style: dict) -> str:
    """A length in the document's dimension style (Viewport's formatter)."""
    from views.document_controller import DocumentController
    return DocumentController._format_dim_value(metres, style)


def _collect_stats(scene) -> dict:
    """Walk the document and return a statistics dictionary (raw numbers;
    formatting happens at display time with the dimension style)."""
    loose = scene.loose_mesh          # the real loose mesh, group open or not
    groups = scene.groups

    loose_verts = len(loose.vertices)
    loose_edges = len(loose.edges)
    loose_faces = len(loose.faces)

    group_count = len(groups)
    instance_count = sum(1 for g in groups if g.is_component())

    # Instances share their prototype mesh; counting it once per instance is
    # deliberate — these are render/entity counts, the way modellers usually
    # report them, not a dedup of prototypes.
    from core.group import iter_placements
    placed = [(pg, m) for g in groups for pg, m in iter_placements(g)]

    group_verts = sum(len(g.mesh.vertices) for g, _m in placed)
    group_edges = sum(len(g.mesh.edges) for g, _m in placed)
    group_faces = sum(len(g.mesh.faces) for g, _m in placed)

    all_faces = list(loose.faces) + [f for g, _m in placed for f in g.mesh.faces]
    tri_count = sum(len(f.loop) - 2 for f in all_faces if len(f.loop) >= 3)

    # --- Bounding box (world space: instance prototypes through their xform)
    positions = [v.position for v in loose.vertices]
    for g, m in placed:
        if m is not None:
            positions.extend(m.map(v.position) for v in g.mesh.vertices)
        else:
            positions.extend(v.position for v in g.mesh.vertices)
    if positions:
        xs = [p.x() for p in positions]
        ys = [p.y() for p in positions]
        zs = [p.z() for p in positions]
        bbox = {"min_x": min(xs), "max_x": max(xs),
                "min_y": min(ys), "max_y": max(ys),
                "min_z": min(zs), "max_z": max(zs),
                "width": max(xs) - min(xs),
                "depth": max(ys) - min(ys),
                "height": max(zs) - min(zs)}
    else:
        bbox = {k: 0.0 for k in ("min_x", "max_x", "min_y", "max_y",
                                 "min_z", "max_z", "width", "depth", "height")}

    # --- Materials in use — the Paint tray's idiom (attrs, floats 0–1) -----
    colors: dict[tuple, dict] = {}
    textures: dict[str, dict] = {}
    for f in all_faces:
        tex = f.attrs.get("texture")
        if tex and tex.get("path"):
            entry = textures.setdefault(
                tex["path"], {"name": Path(tex["path"]).stem,
                              "faces": 0, "area": 0.0})
        else:
            col = f.attrs.get("color")
            if col is None:
                continue
            entry = colors.setdefault(
                tuple(col), {"rgb": tuple(col), "faces": 0, "area": 0.0})
        # Registry identity (attrs["mat"], core.materials): the entry shows
        # ITS name — "Concreto visto: 84 m²", a finishes takeoff line —
        # instead of an anonymous rgb() or a texture-file stem.
        if f.attrs.get("mat") and "mat" not in entry:
            entry["mat"] = f.attrs["mat"]
        entry["faces"] += 1
        entry["area"] += bim.face_net_area(f)   # a slab's opening is not paint

    # --- BIM objects — the same call behind the BIM tray and IFC export ----
    objects = [{"class": o["class"], "name": o["name"],
                "area": o["area"], "volume": o["volume"]}
               for o in bim.collect_objects(scene)]
    by_class: dict[str, int] = {}
    for o in objects:
        by_class[o["class"]] = by_class.get(o["class"], 0) + 1

    return {
        "loose_verts": loose_verts, "loose_edges": loose_edges,
        "loose_faces": loose_faces,
        "group_count": group_count,
        "instances": instance_count,
        "classic_groups": group_count - instance_count,
        "group_verts": group_verts, "group_edges": group_edges,
        "group_faces": group_faces,
        "total_verts": loose_verts + group_verts,
        "total_edges": loose_edges + group_edges,
        "total_faces": loose_faces + group_faces,
        "tri_count": tri_count,
        "bbox": bbox,
        "colors": list(colors.values()),
        "textures": list(textures.values()),
        "layers": [{"name": ly.name, "key": ly.key, "visible": ly.visible,
                    "locked": ly.locked} for ly in scene.layers],
        "bim_objects": objects,
        "bim_by_class": by_class,
        "dims_count": len(scene.dimensions),
        "text_count": len(scene.text_labels),
        "guides_count": len(scene.guides),
        "saved_views_count": len(scene.saved_views),
        "geo_paths_count": len(scene.geo_paths),
        "style": dict(scene.dimension_style),
    }


def _stats_to_text(stats: dict) -> str:
    """Plain-text summary (the Copy to Clipboard payload)."""
    s, style = stats, stats["style"]
    lines = [
        "=== Built — " + tr("Model Info") + " ===",
        "",
        "--- " + tr("Geometry") + " ---",
        f"{tr('Vertices')}:  {s['total_verts']:,}",
        f"{tr('Edges')}:     {s['total_edges']:,}",
        f"{tr('Faces')}:     {s['total_faces']:,}",
        f"{tr('Triangles (est.)')}: {s['tri_count']:,}",
        f"{tr('Groups (classic)')}: {s['classic_groups']:,}  /  "
        f"{tr('Component instances')}: {s['instances']:,}",
        "",
        "--- " + tr("Bounding box") + " ---",
        f"X: {_fmt_len(s['bbox']['width'], style)}   "
        f"Y: {_fmt_len(s['bbox']['depth'], style)}   "
        f"Z: {_fmt_len(s['bbox']['height'], style)}",
        "",
        "--- " + tr("Materials in use") + " ---",
    ]
    for m in s["textures"]:
        label = m.get("mat") or m["name"]
        lines.append(f"  {label}: {m['faces']} {tr('faces')}, "
                     f"{fmt_area(m['area'])}")
    for m in s["colors"]:
        r, g, b = (int(round(c * 255)) for c in m["rgb"][:3])
        label = m.get("mat") or f"rgb({r},{g},{b})"
        lines.append(f"  {label}: {m['faces']} {tr('faces')}, "
                     f"{fmt_area(m['area'])}")
    if not s["textures"] and not s["colors"]:
        lines.append("  " + tr("(none)"))

    lines += ["", f"--- {tr('Layers')} ({len(s['layers'])}) ---"]
    for ly in s["layers"]:
        vis = "✓" if ly["visible"] else "✗"
        lock = " 🔒" if ly["locked"] else ""
        lines.append(f"  [{vis}] {ly['name']}{lock}")

    if s["bim_objects"]:
        lines += ["", f"--- {tr('BIM objects')} ({len(s['bim_objects'])}) ---"]
        for o in s["bim_objects"]:
            vol = (fmt_volume(o['volume']) if o["volume"] is not None
                   else "—")
            lines.append(f"  {o['class']}  {o['name']}: "
                         f"{fmt_area(o['area'])}, {vol}")

    lines += [
        "",
        "--- " + tr("Annotations") + " ---",
        f"{tr('Dimensions')}: {s['dims_count']}   "
        f"{tr('Text labels')}: {s['text_count']}   "
        f"{tr('Guides')}: {s['guides_count']}",
        f"{tr('Saved views')}: {s['saved_views_count']}   "
        f"{tr('Geo paths')}: {s['geo_paths_count']}",
    ]
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Native statistics panel
# ---------------------------------------------------------------------------

class ModelInfoPanel:
    id, title, slot = "statistics", tr("Model Info"), "utility"

    def __init__(self, app, invalidate):
        self.app = app
        self.invalidate = invalidate

    def sections(self):
        from core.plugin_panels import field, item, section

        s = _collect_stats(self.app.scene)
        style = s["style"]

        def table(key, columns, data):
            return item("table", key, columns=list(map(tr, columns)),
                        rows=[dict(key=str(i), cells=list(row)) for i, row in enumerate(data)])

        rows = [
            (tr("TOTALS"), ""),
            (tr("Vertices"), f"{s['total_verts']:,}"),
            (tr("Edges"), f"{s['total_edges']:,}"),
            (tr("Faces"), f"{s['total_faces']:,}"),
            (tr("Triangles (est.)"), f"{s['tri_count']:,}"),
            (tr("LOOSE GEOMETRY"), ""),
            (tr("Vertices"), f"{s['loose_verts']:,}"),
            (tr("Edges"), f"{s['loose_edges']:,}"),
            (tr("Faces"), f"{s['loose_faces']:,}"),
            (tr("GROUPS / COMPONENTS"), ""),
            (tr("Groups (classic)"), f"{s['classic_groups']:,}"),
            (tr("Component instances"), f"{s['instances']:,}"),
            (tr("Group vertices"), f"{s['group_verts']:,}"),
            (tr("Group faces"), f"{s['group_faces']:,}"),
            (tr("BOUNDING BOX"), ""),
            (tr("Width (X)"), _fmt_len(s["bbox"]["width"], style)),
            (tr("Depth (Y)"), _fmt_len(s["bbox"]["depth"], style)),
            (tr("Height (Z)"), _fmt_len(s["bbox"]["height"], style)),
            (tr("ANNOTATIONS"), ""),
            (tr("Dimensions"), str(s["dims_count"])),
            (tr("Text labels"), str(s["text_count"])),
            (tr("Guides"), str(s["guides_count"])),
            (tr("Saved views"), str(s["saved_views_count"])),
            (tr("Geo paths"), str(s["geo_paths_count"])),
        ]
        materials = []
        swatches = []
        for m in s["textures"]:
            materials.append((m.get("mat") or m["name"], str(m["faces"]), fmt_area(m["area"])))
        for i, m in enumerate(s["colors"]):
            rgb = [round(c * 255) for c in m["rgb"][:3]]
            label = m.get("mat") or "rgb(%d,%d,%d)" % tuple(rgb)
            materials.append((label, str(m["faces"]), fmt_area(m["area"])))
            swatches.append(field(f"swatch{i}", label, "colour", rgb, enabled=False))
        layers = [(ly["name"], "✓" if ly["visible"] else "✗", "✓" if ly["locked"] else "—")
                  for ly in s["layers"]]
        objects = [(o["class"], o["name"], fmt_area(o["area"]),
                    fmt_volume(o["volume"]) if o["volume"] is not None else tr("(not watertight)"))
                   for o in s["bim_objects"]]
        return [
            section("geometry", tr("Geometry"), table("counts", ["Property", "Value"], rows),
                    item("button", "copy", tr("Copy to Clipboard"))),
            section("materials", tr("Materials in use"),
                    table("takeoff", ["Material", "Faces", "Area"], materials), *swatches),
            section("layers", tr("Layers"), table("layer_list", ["Name", "Visible", "Locked"], layers)),
            section("bim", tr("BIM objects"), table("objects", ["Class", "Name", "Area", "Volume"], objects)),
        ]

    def event(self, control, event, value):
        if (control, event) == ("copy", "invoke"):
            from app_shell.native import pasteboard_write
            text = _stats_to_text(_collect_stats(self.app.scene))
            if not pasteboard_write([("public.utf8-plain-text", text.encode())]):
                raise RuntimeError(tr("Could not copy to clipboard"))


create_panel = ModelInfoPanel


class ModelInfoTool(Tool):
    """Extensions-menu tool that opens the Model Info dialog."""
    name = "Model Info"
    shortcut = None          # menu entry only; no key to fight over
    description = (
        "Show the model's statistics: faces, edges, groups, materials "
        "and more.")
    uses_snap = False

    def on_activate(self, viewport) -> None:
        from core.plugin_panels import open_panel
        open_panel(viewport.window(), "model_info")

    def on_deactivate(self, viewport) -> None:
        pass
