# SPDX-License-Identifier: GPL-3.0-or-later
"""Paint History commands shared by the bucket and protocol."""
from core.history import (
    CompoundCommand,
    SetFaceBackCommand,
    SetFaceColorCommand,
    SetFaceMaterialTagCommand,
    SetFaceOpacityCommand,
    SetFaceTextureCommand,
    SetGroupMaterialCommand,
)


def _face_plane(face) -> tuple:
    """``(unit normal, offset)`` of the face's plane."""
    from ingetrazo_core import Vec3 as QVector3D
    n = face.normal()
    p = face.vertices[0] if face.vertices else QVector3D()
    return (n, QVector3D.dotProduct(n, p))


def _same_plane(face, plane, tol: float = 1e-4) -> bool:
    from ingetrazo_core import Vec3 as QVector3D
    n, d = plane
    fn = face.normal()
    if abs(abs(QVector3D.dotProduct(fn, n)) - 1.0) > 1e-3:
        return False
    p = face.vertices[0] if face.vertices else QVector3D()
    return abs(QVector3D.dotProduct(n, p) - d) <= tol


def _surface_commands(mesh, faces, seed, tex, plane) -> list | None:
    """The commands that paint a curved SURFACE (faces joined by soft
    edges) with ``tex`` running continuously from ``seed`` — see
    ``core.texture.continuous_maps``. ``None`` when ``faces`` is not such
    a surface (a single face, or a plain selection), so the caller falls
    back to the per-face rule."""
    if mesh is None or seed is None or len(faces) < 2:
        return None
    from core.texture import continuous_maps
    look = tex
    if tex.get("uvw") and (plane is None or not _same_plane(seed, plane)):
        from core.texture import flattened_texture
        look = flattened_texture(tex, plane[0] if plane else None)
    maps = continuous_maps(mesh, faces, seed, look)
    if len(maps) < 2:
        return None
    cmds = [SetFaceTextureCommand([f], maps[f.key]) for f in faces if f.key in maps]
    rest = [f for f in faces if f.key not in maps]
    if rest:
        cmds.extend(_texture_commands(rest, tex, plane))
    return cmds


def _texture_commands(faces, tex, plane) -> list:
    """Apply ``tex`` the way the classic eyedropper does.

    An explicit ``uvw`` is where the image sits IN THE WORLD; it only means
    the same thing on the plane it was fitted for. Faces on that plane keep
    it, so a pattern continues across a seam; every other face takes the
    material without it and projects the image on its own plane with the
    LOOK the sample had — its tile size and turn, read off the map
    (``core.texture.flattened_texture``) — so a texture scaled and rotated
    with the pins carries to the next wall (Marco, 2026-09-15)."""
    if not tex.get("uvw") or plane is None:
        return [SetFaceTextureCommand(faces, tex)]
    same = [f for f in faces if _same_plane(f, plane)]
    other = [f for f in faces if not _same_plane(f, plane)]
    cmds = []
    if same:
        cmds.append(SetFaceTextureCommand(same, tex))
    if other:
        from core.texture import flattened_texture
        cmds.append(SetFaceTextureCommand(other, flattened_texture(tex, plane[0])))
    return cmds


def paint_stamp(color, texture, opacity, material, plane=None, face=None):
    """The brush as container or back-side attributes."""
    if texture is None:
        stamp = {"color": list(color)}
    else:
        tex = dict(texture)
        if face is None:
            tex.pop("uvw", None)
        elif tex.get("uvw") and (plane is None or not _same_plane(face, plane)):
            from core.texture import flattened_texture
            tex = flattened_texture(tex, plane[0] if plane else None)
        stamp = {"texture": tex}
    if opacity is not None:
        stamp["opacity"] = float(opacity)
    if material is not None:
        stamp["mat"] = material.name
        if material.pbr is not None:
            stamp["pbr"] = material.pbr
            if texture:
                stamp["color"] = list(material.color or (1., 1., 1.))
    return stamp


def build_paint(mesh, faces, seed, group, back, default, color, texture, opacity, material, plane):
    """Translate one complete stroke to the existing undoable commands."""
    name = material.name if material is not None else None
    register = SetFaceMaterialTagCommand([], name, material)
    if group is not None:
        stamp = None if default else paint_stamp(color, texture, opacity, material)
        return CompoundCommand([register, SetGroupMaterialCommand(group, stamp)])
    if back:
        if default:
            return SetFaceBackCommand(faces, None)
        backs = [paint_stamp(color, texture, opacity, material, plane, face) for face in faces]
        return CompoundCommand([register, SetFaceBackCommand(faces, backs)])
    if default:
        return CompoundCommand([SetFaceColorCommand(faces, None), SetFaceTextureCommand(faces, None),
                                SetFaceOpacityCommand(faces, None), SetFaceMaterialTagCommand(faces, None, None)])
    if texture is None:
        cmds = [SetFaceColorCommand(faces, color), SetFaceTextureCommand(faces, None)]
    else:
        cmds = _surface_commands(mesh, faces, seed, texture, plane)
        if cmds is None:
            cmds = _texture_commands(faces, texture, plane)
        if material is not None and material.pbr is not None:
            cmds.append(SetFaceColorCommand(faces, material.color or (1., 1., 1.)))
    return CompoundCommand(cmds + [SetFaceOpacityCommand(faces, opacity),
                                   SetFaceMaterialTagCommand(faces, name, material)])
