"""Adapter from the native inspector to the still-Python controllers."""

import json

from core import bim_registry, deliveries, links, structure
from core.group import Group
from core.protocol import commands
from core.protocol.records import filing_rule, occurrence
from georef.geopath import GeoPath


def _send(view, key, value):
    view.renderer.ui_panel_set(key, json.dumps(value, allow_nan=False))


def _marked(win):
    row = getattr(getattr(win, "_scene_tabs", None), "tabs", None)
    return row.marked_view() if row is not None else None


def _assembly(scene):
    sel = list(scene.selection)
    if len(sel) == 1 and isinstance(sel[0], Group):
        return sel[0]
    return getattr(scene, "edit_group", None)


def _selection(view, win):
    scene = win.viewport.scene
    sel = list(scene.selection)
    focus = getattr(view, "_ui_insp_node", None)
    focused = focus and focus[1] == (len(sel), id(sel[0]) if sel else 0, id(sel[-1]) if sel else 0)
    sig = (
        id(scene),
        scene.version,
        tuple(map(id, sel)),
        focus[0] if focused else None,
        id(scene.edit_group),
    )
    delivery_key = getattr(view, "_delivery_key", None)
    sig += (delivery_key,)
    if sig == getattr(view, "_panel_selection_sig", None):
        return
    view._panel_selection_sig = sig
    groups = [g for g in sel if isinstance(g, Group) and not focused]
    records = []
    schema = "IFC4"
    if groups:
        registry = bim_registry.get(scene)
        delivery = deliveries.records(scene).get(delivery_key, {})
        if delivery.get("format") == "ifc":
            profile = delivery.get("options", {}).get("profile")
            schema = registry.get("export_profiles", {}).get(profile, {}).get("schema", schema)
        records = [occurrence(g, registry, bool(links.linked_ancestor(g))) for g in groups]
    g = _assembly(scene) if not focused else None
    parts = None
    if g is not None:
        parts = dict(
            uid=g.uid,
            name=g.name or "Assembly",
            editable=not (links.linked_ancestor(g) or g.locked),
            mode=(g.exploded or {}).get("mode", "outward"),
            factor=(g.exploded or {}).get("factor", 0),
            children=[dict(uid=c.uid, name=c.name or "Part", hidden=c.hidden) for c in g.children],
        )
    rule = None
    node = focus[0] if focused else structure.node_of(groups[0]) if len(groups) == 1 else None
    if node:
        registry = bim_registry.get(scene)
        source = structure._nodes(registry).get(node)
        if source and "source" not in source:
            rule = dict(node=node, name=source["name"], recipe=source.get("recipe", {}))
    view._panel_schema = schema
    _send(
        view,
        "selection",
        dict(
            schema=schema,
            selection_count=len(sel),
            rule=rule,
            records=records,
            parts=parts,
            path=len(sel) == 1 and isinstance(sel[0], GeoPath),
        ),
    )


def sync(view, win):
    vp = win.viewport
    # The native renderer follows the tab. Resend this document's bindings
    # whenever it reacquires the surface, including a new empty document.
    if getattr(view.renderer, "ui_panel_set", None) is None:
        return
    if getattr(win, "_panel_view", None) is not view:
        win._panel_view = view
        view._panel_selection_sig = view._panel_live = None
        _send(view, "terrain", None)
        binding = getattr(view, "_profile_binding", None)
        if binding:
            binding.dirty = True
    _selection(view, win)
    b = None
    tool = vp.active_tool
    if type(tool).__name__ == "FilletTool":
        from core import bevel_profiles, units

        b = dict(
            affect=tool.affect,
            measure=tool.measure,
            amount=units.fmt_len_fine(tool.radius),
            segments=tool.segments,
            shape=(
                tool.shape
                if isinstance(tool.shape, str)
                else "custom" if tool.shape else "chamfer" if tool.profile == 0.25 else "round"
            ),
            corners=tool.corners,
            points=bevel_profiles.controls(tool.shape, tool.segments),
        )
    c = None
    if getattr(view, "_camera_inspector", False):
        cam = vp.camera
        marked = _marked(win)
        eye, target = cam.eye(), cam.target
        c = dict(
            cam.lens_dict(),
            projection=cam.projection_mode(),
            focal=cam.focal_mm,
            fov=cam.fov_deg,
            sensor=cam.sensor_size(),
            eye=[eye.x(), eye.y(), eye.z()],
            target=[target.x(), target.y(), target.z()],
            saved=marked is not None,
            scene=marked.name if marked else "Current view",
        )
    if (b, c) != getattr(view, "_panel_live", None):
        _send(view, "bevel", b)
        _send(view, "camera", c)
        view._panel_live = b, c
    terrain = getattr(view, "_profile_binding", None)
    if terrain:
        terrain.sync(view, vp)


def _camera(win, view, key, v):
    from core.camera_point import CameraPoint
    from core.camera import FOV_RANGE

    vp, cam = win.viewport, win.viewport.camera
    if key in ("open", "close"):
        view._camera_inspector = key == "open"
    elif key == "update":
        marked = _marked(win)
        if marked and marked.name == v:
            win.tray.scenes.update_view(marked)
    elif key == "reset":
        cam.reset_lens()
    elif key == "projection":
        cam.set_projection(v)
    elif key in ("focal", "fov"):
        cam.fov_deg = min(
            FOV_RANGE[1], max(FOV_RANGE[0], cam.fov_for_focal(v) if key == "focal" else v)
        )
    elif key.startswith("sensor"):
        size = list(cam.sensor_size())
        size[int(key[-1])] = v
        cam.set_sensor(size)
    elif key.startswith(("eye", "target")):
        eye, target = cam.eye(), cam.target
        pts = [[eye.x(), eye.y(), eye.z()], [target.x(), target.y(), target.z()]]
        pts[int(key.startswith("target"))][int(key[-1])] = v
        cam.place(*(CameraPoint(*p) for p in pts))
    elif key in ("frame_aspect", "shift_x", "shift_y", "roll_deg", "clip_near", "clip_far"):
        cam.apply_lens({**cam.lens_dict(), key: v or None})
    vp.update()


def _bevel(view, key, value):
    host = view.tools
    tool = host.tool
    if type(tool).__name__ != "FilletTool":
        return
    if key == "apply":
        tool.apply_from_panel(host)
    elif key == "amount":
        from core.units import parse_length_field

        n = parse_length_field(value)
        if isinstance(n, (int, float)) and n > 0:
            tool.set_size(host, n)
    elif key == "segments":
        tool.on_segments_value(host, value)
    elif key in ("affect", "measure", "corners"):
        getattr(tool, "set_" + key)(host, value)
    elif key == "shape" and value in ("round", "chamfer"):
        tool.on_profile_value(host, 0.5 if value == "round" else 0.25)
    elif key == "shape" and value == "custom":
        from core.bevel_profiles import controls

        tool.set_shape(host, controls(tool.shape, tool.segments))
    elif key in ("shape", "curve"):
        tool.set_shape(host, value)


def action(win, view, payload):
    key, v = json.loads(payload)
    surface, verb = key.split(".")
    vp = win.viewport
    try:
        if surface == "record":
            placements, path, value, *kind = v
            if not set(placements).issubset({getattr(g, "uid", None) for g in vp.scene.selection}):
                return
            commands.run(
                vp.history,
                "patch_record_field",
                placements=placements,
                path=path,
                value=value,
                property_kind=kind[0] if kind else None,
                schema=getattr(view, "_panel_schema", "IFC4"),
            )
            vp.notify_scene_changed()
        elif surface == "rule":
            node, expected, recipe = v
            if verb == "preview":
                try:
                    result = filing_rule(vp.scene, node, recipe, expected=expected)
                except (KeyError, TypeError, ValueError) as exc:
                    result = {"error": str(exc)}
                _send(view, "rulePreview", dict(result, request=v))
            elif verb == "apply":
                commands.run(
                    vp.history, "set_filing_rule", node=node, expected=expected, recipe=recipe
                )
                vp.notify_scene_changed()
        elif surface == "parts":
            g = _assembly(vp.scene)
            uid = v if verb == "split" else v[0]
            if g is None or g.uid != uid or links.linked_ancestor(g) or g.locked:
                return
            if vp.scene.edit_group is g:
                vp.end_group_edit()
                vp.scene.select([g])
            if verb == "visible":
                child = next((c for c in g.children if c.uid == v[1]), None)
                if child and not child.locked and not links.linked_ancestor(child):
                    commands.run(vp.history, "unhide" if v[2] else "hide", placements=[child.uid])
            elif verb == "split":
                commands.run(vp.history, "split_into_pieces", placement=uid)
            elif verb == "explode":
                commands.run(vp.history, "explode_view", placement=uid, factor=v[1], mode=v[2])
            vp.notify_scene_changed()
        elif surface == "bevel":
            _bevel(view, verb, v)
        elif surface == "camera":
            _camera(win, view, verb, v)
        elif surface == "terrain":
            from views.terrain_binding import action as terrain_action

            terrain_action(view, vp, verb)
    except (KeyError, ValueError, OSError) as exc:
        vp.flash_status(str(exc))
    view.update()
