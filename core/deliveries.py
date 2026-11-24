# SPDX-License-Identifier: GPL-3.0-or-later
"""Document-owned repeatable exports. Writers receive a scoped model view."""
import json
from copy import copy, deepcopy
from datetime import datetime, timezone
from pathlib import Path
from tempfile import TemporaryDirectory
from uuid import uuid4

from ingetrazo_core import delivery_error, delivery_formats

FORMATS = tuple(delivery_formats())


def records(scene):
    return scene.plugin_data.get("deliveries", {})


def validate(record):
    """Raise ValueError with what makes ``record`` no delivery
    (rust/ingetrazo_core/src/delivery_form.rs)."""
    error = delivery_error(json.dumps(record, default=str))
    if error:
        raise ValueError(error)


def _store(history, key, values):
    from core.history import SetPluginDataCommand
    from core.protocol.commands import _answer
    before = history.revision
    history.execute(SetPluginDataCommand("deliveries", values))
    return dict(_answer(history, before), key=key)


def create_delivery(history, delivery):
    key = uuid4().hex
    record = {"scope": "whole", "options": {}, **delivery, "last_run": None}
    validate(record)
    return _store(history, key, {**records(history.scene), key: record})


def edit_delivery(history, key, changes):
    values = dict(records(history.scene))
    values[key] = {**values[key], **changes}
    validate(values[key])
    return _store(history, key, values)


def delete_delivery(history, key):
    values = dict(records(history.scene))
    del values[key]
    return _store(history, key, values)


def _mesh_part(mesh, keep):
    from core.mesh import Mesh
    if keep is None:
        return mesh
    out = Mesh()
    for f in mesh.faces:
        if keep(f):
            out.add_face(f.vertices, f.holes).attrs.update(f.attrs)
    for e in mesh.edges:
        if keep(e):
            edge = out.add_edge(e.v0.position, e.v1.position)
            for field in ("soft", "flat", "layer", "hidden", "curve"):
                setattr(edge, field, getattr(e, field))
    return out


def scoped(scene, record):
    from core import bim_registry, doc_model
    from core.links import _clone_shell
    from core.link_structure import Tree
    from core.mesh import Mesh
    scope = record["scope"]
    wanted = None
    if scope == "node":
        tree = Tree(scene, bim_registry.get(scene))
        if record["node"] not in tree.nodes:
            raise ValueError("The delivery's project node is missing")
        wanted = set(tree.members(record["node"]))
    elif scope == "selection":
        wanted = set(scene.selection)
        if not wanted:
            raise ValueError("Select geometry before running this delivery")
    visible = scope == "visible"
    meshes = {}

    def branch(g, inside):
        if visible and not scene.entity_visible(g):
            return None
        inside = inside or wanted is None or g in wanted
        children = []
        for child in g.children or ():
            inherit = inside and (scope != "node" or id(child) not in tree.member)
            projected = branch(child, inherit)
            if projected is not None:
                children.append(projected)
        edit_selection = scope == "selection" and g is scene.edit_group
        if not inside and not children and not edit_selection:
            return None
        item = _clone_shell(g)
        if edit_selection:
            item.mesh = _mesh_part(g.mesh, wanted.__contains__)
        elif not inside:
            item.mesh = Mesh()
        elif visible:
            if id(g.mesh) not in meshes:
                meshes[id(g.mesh)] = _mesh_part(g.mesh, scene.entity_visible)
            item.mesh = meshes[id(g.mesh)]
        item.children = children
        if not inside:
            item.ifc = None
        return item

    out = copy(scene)
    # The tables kept on the scene's root (tag folders, active tag,
    # plugin data, materials, kept components) come with the copy.
    from core.scene import ROOT_TABLES
    tables = {name: copy(getattr(scene, name)) for name in ROOT_TABLES}
    tables["plugin_data"] = deepcopy(scene.plugin_data)
    materials = scene.materials.copy()
    kept = list(getattr(scene, "kept_components", []) or [])
    out._root = doc_model.new_scene_root()
    for name, value in tables.items():
        setattr(out, name, value)
    out.materials = materials
    if kept:
        # A kept placement has one parent. Give the projection its own
        # placement handles while preserving the geometry definitions.
        from core.purge import iter_groups
        geometry = {id(g.mesh): g.mesh for g in iter_groups(kept)}
        out.kept_components = deepcopy(kept, geometry)
    if record.get('format') == 'ifc' or 'format' not in record:
        out._repeated_source_uids = scene.repeated_uids()
    out.edit_group = None
    out.__dict__.pop("_groups_owner", None)
    out.layers = [copy(layer) for layer in scene.layers]
    out.groups = [item for g in scene.groups if (item := branch(g, False)) is not None]
    keep = scene.entity_visible if visible else None
    if wanted is not None:
        keep = wanted.__contains__
    out.mesh = _mesh_part(scene.loose_mesh, keep)
    if scope != "scenes":
        out.entity_visible = lambda entity: True  # membership is already resolved
    return out


def _chosen(items, names):
    if names:
        missing = set(names) - {x.name for x in items}
        if missing:
            raise ValueError(f"Missing scene or sheet: {', '.join(sorted(missing))}")
        items = [x for x in items if x.name in names]
    if not items:
        raise ValueError("No scenes or sheets in this delivery")
    return items


def _write_view(model, record, path, camera, view, svg_picture, screen_mm):
    from core.camera import OrbitCamera
    opts = record["options"]
    cam = copy(camera) if camera is not None else OrbitCamera()
    if view:
        # Saved views can also change sections, survey layers
        # and shadows. Isolate those mutable presentation objects.
        model.section_planes = deepcopy(list(model.section_planes))
        model.shadows = deepcopy(model.shadows)
        for attr in ("tile_layer", "terrain", "photo_mesh"):
            obj = getattr(model, attr, None)
            if obj is not None:
                setattr(model, attr, copy(obj))
        view.apply(model, cam)
    from contextlib import nullcontext
    from views.svg_export import framed
    frame = opts.get("frame")
    with framed(cam, frame) if frame else nullcontext():
        return _write_drawing(model, path, cam, view.name if view else "VISTA", record, svg_picture, screen_mm)


def _write_drawing(model, path, cam, label, record, svg_picture, screen_mm, geometry=None):
    from core.hlr import hlr_drawing, hlr_perspective
    fmt, opts = record["format"], record["options"]
    solids, texts = [], []
    if cam.perspective:
        groups = [(label, hlr_perspective(model, cam, geometry=geometry))]
    else:
        from core.layers import export_labels, line_styles
        from formats.dxf_views import dxf_cut_layers, dxf_groups_by_layer
        drawing = hlr_drawing(model, cam, layers=True, geometry=geometry)
        drawing.styles = line_styles(model, True)
        drawing.labels = export_labels(model)
        if fmt == "dxf" and len(model.layers) == 1:
            drawing.labels[model.layers[0].key] = label
        groups = dxf_groups_by_layer(drawing, label, False, texts=texts)
        if fmt == "dxf":
            cuts, solids = dxf_cut_layers(model, cam, geometry=geometry)
            groups.extend(cuts)
    if fmt == "svg":
        from core.vista_caras import view_faces
        faces = opts.get("faces")
        fills = view_faces(model, cam, **faces) if faces is not None else None
        image = None
        if opts.get("picture"):
            if svg_picture is None or record["scope"] != "visible":
                raise ValueError("A viewport picture requires the visible scope and an active viewport")
            image = svg_picture(opts.get("axes", True), opts.get("frame"))
        from formats.svg_out import svg_text
        from core.vista_caras import view_rect
        paper = opts.get("paper")
        if paper == "screen":
            if screen_mm is None:
                raise ValueError("Screen size requires viewport dimensions")
            paper = screen_mm
        crop = view_rect(cam) if opts.get("frame") or opts.get("paper") == "screen" else None
        text, _ = svg_text(groups, scale_n=opts.get("scale", 100), faces=fills, image=image,
                           paper=paper, window=crop, line_colours=(faces or {}).get("materials", True))
        if path is None:
            return text
        path.write_text(text, encoding="utf-8")
    else:
        from formats.dxf_out import save_dxf_layers
        save_dxf_layers(path, groups, solids, texts)


def preview_svg(scene, record, *, camera, svg_picture=None, screen_mm=None):
    """Render the first output page with the delivery writer, entirely in memory."""
    view = _chosen(scene.saved_views, record["options"].get("names"))[0] if record["scope"] == "scenes" else None
    return _write_view(scoped(scene, record), record, None, camera, view, svg_picture, screen_mm)


def write_steps(scene, record, *, camera=None, pdf_writer=None, svg_picture=None, screen_mm=None, project_directory=None, view_geometry=None):
    """Yield progress between output files. Publish only after every writer succeeds."""
    validate(record)
    fmt, opts = record["format"], record["options"]
    if project_directory is None and getattr(scene, "_project_host_path", None):
        project_directory = Path(scene._project_host_path).parent
    target = Path(record["destination"]).expanduser()
    if not target.is_absolute():
        if project_directory is None:
            raise ValueError("Choose an absolute destination for an unsaved document")
        target = Path(project_directory) / target
    saved = (_chosen(scene.saved_views, opts.get("names")) if record["scope"] == "scenes" else [None])
    with TemporaryDirectory(dir=target.parent) as tmp:
        for i, view in enumerate(saved):
            yield i / len(saved), view.name if view else "Writing"
            path = Path(tmp, target.name if len(saved) == 1 else f"{target.stem}-{i+1:02}{target.suffix}")
            if fmt == "pdf":
                sheets = _chosen(scene.compositions, opts.get("names"))
                if pdf_writer is None:
                    raise ValueError("Sheet export requires a composer")
                failed = pdf_writer(str(path), sheets)
                if failed:
                    raise ValueError("Frames failed on sheets: " + ", ".join(failed))
                continue
            if fmt == "dxf" and record["scope"] == "visible" and view_geometry is not None:
                geometry = view_geometry(tags=not camera.perspective)
                _write_drawing(scene, path, camera, "VISTA", record, None, None, geometry)
                continue
            model = scoped(scene, record)
            if fmt == "ifc":
                from formats.ifc import save_ifc
                save_ifc(model, path, profile=opts.get("profile"), project_directory=project_directory)
            elif fmt == "glb":
                from formats.gltf import save_glb
                save_glb(model, path, embed_textures=opts.get("textures", True))
            elif fmt == "obj":
                from formats.obj import save_obj
                save_obj(model, path)
            elif fmt == "dae":
                from formats.dae import save_dae
                save_dae(model, path)
            elif fmt == "stl":
                from formats.stl import save_stl
                save_stl(model, path, opts.get("unit", "mm"))
            else:
                _write_view(model, record, path, camera, view, svg_picture, screen_mm)
        outputs = list(Path(tmp).iterdir())
        if not outputs:
            raise ValueError("The exporter produced no files")
        # Sidecars move with the primary result. Restore the previous files
        # if committing one member of the set fails.
        backup = Path(tmp, ".previous")
        backup.mkdir()
        installed = []
        try:
            for part in outputs:
                dest = target.parent / part.name
                if dest.is_dir():
                    raise IsADirectoryError(dest)
                if dest.exists() or dest.is_symlink():
                    dest.replace(backup / part.name)
                part.replace(dest)
                installed.append(dest)
        except OSError:
            for dest in installed:
                dest.unlink()
            for old in backup.iterdir():
                old.replace(target.parent / old.name)
            raise


def finish(history, key):
    return edit_delivery(history, key, {"last_run": datetime.now(timezone.utc).isoformat(timespec="seconds")})


def run_delivery(history, key, **context):
    record = records(history.scene)[key]
    # An attached UI supplies its job runner. Headless clients drive the
    # same generator synchronously and can provide a sheet composer.
    runner = getattr(history, "delivery_runner", None)
    if runner is not None and not context:
        job_id = runner(key, record)
        return {"ok": True, "step": None, "error": None, "rejected": None, "job": job_id}
    for _ in write_steps(history.scene, record, **context):
        pass
    return finish(history, key)
