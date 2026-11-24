"""Model clipboard bytes shared by the native and Classic pasteboards."""
from __future__ import annotations

import io
import json
import tempfile
import zipfile
from pathlib import Path

from ingetrazo_core import Vec3 as QVector3D

_FORMAT = 1


def _xyz(p: QVector3D) -> list:
    return [p.x(), p.y(), p.z()]


def encode(clip: dict) -> bytes:
    """``viewport.clipboard`` → bytes for the system clipboard."""
    from core.scene import Scene
    from formats import igz

    scene = Scene()
    for loop, holes, *rest in clip.get("faces", ()):
        try:
            face = scene.mesh.add_face(loop, holes or None)
        except ValueError:
            continue
        attrs = rest[0] if rest else None
        if attrs:
            face.attrs = dict(attrs)
    scene.groups.extend(clip.get("groups", ()))
    registry = clip.get("registry") or clip.get("materials")
    if registry:
        from core.mesh import face_attrs
        from core.group import iter_placements
        def include(attrs):
            name = attrs.get("mat")
            if name in registry:
                scene.materials[name] = registry[name]
            back = attrs.get("back")
            if isinstance(back, dict):
                include(back)
        seen = set()
        for root in scene.groups:
            for group, _ in iter_placements(root):
                include(group.material or {})
                if id(group.mesh) not in seen:
                    seen.add(id(group.mesh))
                    for attrs in face_attrs(group.mesh):
                        include(attrs)
        for face in scene.mesh.faces:
            include(face.attrs)
    # The copy's tags and folders ride in the document itself: the .igz
    # writer keeps a scene's tag list and folders.
    from core.layers import add_missing
    add_missing(scene, clip.get("layers"))
    head = {
        "format": _FORMAT,
        "ref": _xyz(clip["ref"]),
        "edges": [[_xyz(a), _xyz(b), bool(soft), curve]
                  for a, b, soft, curve in clip.get("edges", ())],
        # Apart from ``edges``, so a reader that predates it still reads
        # four fields per edge.
        "edge_layers": list(clip.get("edge_layers") or ()),
    }
    with tempfile.TemporaryDirectory(prefix="ingetrazo-clip-") as tmp:
        path = Path(tmp) / "scene.igz"
        igz.save_scene(scene, path)
        doc = path.read_bytes()
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_STORED) as z:
        z.writestr("clip.json", json.dumps(head))
        z.writestr("scene.igz", doc)
    return out.getvalue()


def decode(data: bytes) -> dict | None:
    """Bytes from the system clipboard → a ``viewport.clipboard`` dict, or
    ``None`` when they are not a clipboard this version can read."""
    from core.scene import Scene
    from formats import igz

    try:
        from core import zip_limits
        with zipfile.ZipFile(io.BytesIO(bytes(data))) as z:
            zip_limits.check(z, "the clipboard")
            head = json.loads(z.read("clip.json"))
            doc = z.read("scene.igz")
    except (zipfile.BadZipFile, KeyError, ValueError):
        return None
    if head.get("format") != _FORMAT:
        return None
    scene = Scene()
    with tempfile.TemporaryDirectory(prefix="ingetrazo-clip-") as tmp:
        path = Path(tmp) / "scene.igz"
        path.write_bytes(doc)
        igz.load_into(scene, path)
    faces = [([QVector3D(v) for v in f.vertices],
              [[QVector3D(v) for v in h] for h in f.holes],
              dict(f.attrs) if f.attrs else {})
             for f in scene.mesh.faces]
    edges = [(QVector3D(*a), QVector3D(*b), soft, curve)
             for a, b, soft, curve in head.get("edges", ())]
    # Pasted copies of a classic group share one definition until edited,
    # as with a local copy (``Viewport.copy_selection``).
    from ingetrazo_core import Matrix4 as QMatrix4x4
    for g in scene.groups:
        if getattr(g, "xform", None) is None:
            g.xform = QMatrix4x4()
            g.component = False       # a group still (issue #90)
    from core.layers import template_of
    return {"faces": faces, "edges": edges, "groups": list(scene.groups),
            "edge_layers": list(head.get("edge_layers") or ()),
            "layers": template_of(scene), "materials": dict(scene.materials),
            "ref": QVector3D(*head["ref"])}
