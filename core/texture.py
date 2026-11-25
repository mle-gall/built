# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Image textures, mapped the .skp way for interchange compatibility.

A .skp material is a colour plus an optional texture image with a
**real-world tile size** (the model-unit width/height one repeat of the image
covers). The default mapping is a **planar projection**: a face's UVs come from
its world position projected onto the face plane, divided by the tile size. The
projection basis depends only on the face normal — the .skp format's own, see
:func:`projection_basis` — so coplanar faces share it and the texture tiles
**seamlessly** across a flat surface, and a face painted here shows the image
where the .skp file places it.

A textured face carries ``attrs["texture"] = {"path", "sw", "sh"}``. Colour and
texture are independent (a face can have either or both).

``path`` is always a real file on disk (the renderer, and every exporter, load
the image from there). Images that came in *inside* something — extracted from
an imported ``.skp``, unpacked from an ``.igz`` container — live in the app's
own **texture cache** (:func:`texture_cache_root`), never next to the user's
files.
"""
from __future__ import annotations

from dataclasses import dataclass
import re
from pathlib import Path

import ingetrazo_core as _NATIVE
from ingetrazo_core import Vec3 as QVector3D



def _data_root() -> Path:
    """The platform's per-user data folder, as the native shell names it
    (``platform_paths.rs: local_data_directory``) when it is not loaded
    (scripts, tests): ``$BUILT_SETTINGS_DIR/data`` for a profile of its own,
    else ``~/Library/Application Support`` on macOS, ``%LOCALAPPDATA%`` on
    Windows and ``$XDG_DATA_HOME`` or ``~/.local/share`` elsewhere."""
    import os
    import sys
    profile = os.environ.get("BUILT_SETTINGS_DIR")
    if profile:
        return Path(profile) / "data"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support"
    if os.name == "nt":
        return Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
    xdg = os.environ.get("XDG_DATA_HOME")
    return Path(xdg) if xdg and os.path.isabs(xdg) else Path.home() / ".local" / "share"


def texture_cache_root() -> Path:
    """Root of the app's texture cache: ``<user data>/IngeTrazo/textures``
    (``~/Library/Application Support/…`` on macOS, ``~/.local/share/…`` on
    Linux, ``%LOCALAPPDATA%\\…`` on Windows), the native shell's data folder.
    ``$INGETRAZO_TEXTURE_CACHE`` overrides it (tests, and users who want the
    images somewhere else).

    Two subfolders, one per source: ``skp/`` (images extracted from an imported
    ``.skp``) and ``embedded/`` (images unpacked from an ``.igz`` container)."""
    import os
    override = os.environ.get("INGETRAZO_TEXTURE_CACHE")
    if override:
        return Path(override)
    from app_shell.native import _shell
    if _shell is not None:
        return Path(_shell.local_data_directory(os.environ.get("BUILT_SETTINGS_DIR"))) / "IngeTrazo" / "textures"
    return _data_root() / "IngeTrazo" / "textures"


def texture_cache_stats() -> tuple[int, int]:
    """``(file count, total bytes)`` currently held in the texture cache."""
    root = texture_cache_root()
    count = total = 0
    if root.is_dir():
        for f in root.rglob("*"):
            try:
                if f.is_file():
                    count += 1
                    total += f.stat().st_size
            except OSError:
                continue
    return count, total


def clear_texture_cache() -> int:
    """Delete the whole texture cache. Returns the number of files removed.
    Documents saved as ``.igz`` containers re-extract their images the next
    time they are opened; faces whose image came from a ``.skp`` import lose
    their texture until the ``.skp`` is imported again — the caller is expected
    to confirm first."""
    import shutil
    root = texture_cache_root()
    count = texture_cache_stats()[0]
    if root.is_dir():
        shutil.rmtree(root, ignore_errors=True)
    return count


_DIGEST_PREFIX = re.compile(r"^(?:[0-9a-f]{16}-)+")
NAME_LIMIT = 64     # characters kept of an image name inside the cache


def image_has_cutout(path, cache={}) -> bool:
    """Whether the image carries REAL transparency (some pixels see-through)
    — the signature of a photo sprite (a person, a tree, a raschel mesh)
    versus an opaque photo panel (a sign or mural). What makes a textured
    material *translucent* in the .skp format's sense. Cached per path; an
    unreadable image reads as opaque."""
    cached = cache.get(path)
    if cached is not None:
        return cached
    from ingetrazo_render import texture_cutout
    ok = texture_cutout(str(path))
    cache[path] = ok
    return ok


def texture_file_name(name: str) -> str:
    """Filesystem-safe base name for an image called ``name``: characters
    outside the safe set dropped, any content-hash prefix the name already
    carried (from a cached file or an archive member) stripped, and the whole
    clipped to :data:`NAME_LIMIT`. Stripping is what keeps a document from
    stacking ``<hash>-<hash>-…-sumari.png`` one level deeper on every save —
    0.3.10 did exactly that until the cache path outgrew Windows' 260-character
    limit and the file refused to open; clipping keeps any name inside it."""
    safe = "".join(c for c in Path(name).name
                   if c.isalnum() or c in " ._-").strip(" .")
    safe = _DIGEST_PREFIX.sub("", safe).strip(" .")
    if len(safe) > NAME_LIMIT:
        stem, dot, ext = safe.rpartition(".")
        if dot and 0 < len(ext) <= 5 and ext.isalnum():
            safe = stem[:NAME_LIMIT - len(ext) - 1].rstrip(" .") + "." + ext
        else:
            safe = safe[:NAME_LIMIT].rstrip(" .")
    return safe or "texture.png"


_IMAGE_MAGIC = (
    b"\x89PNG\r\n\x1a\n",            # PNG
    b"\xff\xd8\xff",                   # JPEG
    b"GIF87a", b"GIF89a",
    b"BM",                              # BMP
    b"II*\x00", b"MM\x00*",             # TIFF
)


def is_image_bytes(data: bytes) -> bool:
    """True when ``data`` starts like an image file (PNG, JPEG, GIF, BMP,
    TIFF, WebP). TGA has no signature: it is accepted only through
    :func:`is_image_file` by extension plus a TGA header check."""
    head = bytes(data[:16])
    if head.startswith(_IMAGE_MAGIC):
        return True
    return head[:4] == b"RIFF" and head[8:12] == b"WEBP"


def is_image_file(path) -> bool:
    """True when ``path`` is a readable image. A document names its texture
    files by path, and saving or exporting copies those files out: without
    this check a crafted document pointing at ``~/.ssh/id_ed25519`` would
    get that key embedded in the next save."""
    p = Path(path)
    try:
        with open(p, "rb") as fh:
            head = fh.read(16)
    except OSError:
        return False
    if is_image_bytes(head):
        return True
    if p.suffix.lower() == ".tga":
        from ingetrazo_render import readable_tga
        return readable_tga(p)
    return False


def cache_image(data: bytes, name: str, subdir: str) -> Path:
    """Store ``data`` in the texture cache under ``subdir`` and return its
    path. **Content-addressed**: the file name carries a hash of the bytes, so
    the same image shared by several documents is written (and uploaded to the
    GPU) once, and a rewrite of the same file is a no-op. ``name`` goes through
    :func:`texture_file_name`, so feeding a cached file's own name back in
    lands on the same file instead of a longer one."""
    import hashlib
    digest = hashlib.sha1(data).hexdigest()[:16]
    safe = texture_file_name(name)
    out = texture_cache_root() / subdir / f"{digest}-{safe}"
    try:
        out.parent.mkdir(parents=True, exist_ok=True)
        if not out.exists() or out.stat().st_size != len(data):
            out.write_bytes(data)
        return out
    except OSError:
        import tempfile
        tmp = Path(tempfile.mkdtemp(prefix="ingetrazo-tex-")) / safe
        tmp.write_bytes(data)
        return tmp


@dataclass
class Texture:
    path: str          # image file
    sw: float = 1.0    # real-world width of one tile (metres)
    sh: float = 1.0    # real-world height of one tile (metres)

    def as_dict(self) -> dict:
        return {"path": self.path, "sw": self.sw, "sh": self.sh}

    @staticmethod
    def from_dict(d: dict) -> "Texture":
        return Texture(d["path"], float(d.get("sw", 1.0)), float(d.get("sh", 1.0)))


#: |Z × n| below which the .skp projection maps a face with the world axes
#: (X, ±Y) instead of the cross product — measured, see
#: :func:`projection_basis`.
VERTICAL_TOLERANCE = 1e-3


# ---- The maths, in Rust (``rust/ingetrazo_core/src/model/texture_maths.rs``)
#
# ``projection_basis(normal)``: the .skp format's in-plane axes for a face
# normal ``(nx, ny, nz)``, the basis its default texture projection AND its
# per-face texture matrices are expressed in: ``xr = normalize(Z × n)``,
# ``yr = n × xr``; for a vertical normal ``(X, Y)`` looking up and ``(−X, Y)``
# looking down, while ``|Z × n| < VERTICAL_TOLERANCE``. THE one recipe for the
# whole app (renderer, exporters, paste preview, the .skp importer), calibrated
# against the .skp reference for every orientation (2026-09-04): with the
# world-X projection a texture on a wall facing +Y or −X came out upside-down,
# and a horizontal face with float noise in its normal turned 90°.
#
# ``projection_axes(normal, rot=0.0)``: that basis as ``QVector3D`` axes, turned
# in-plane by ``rot`` degrees (the .skp texture rotation). ``planar_uv(normal,
# positions, sw, sh, rot=0.0)``: .skp-style planar ``(u, v)`` of world points,
# scaled by the tile size (``sw``/``sh`` near 0 count as 1).
projection_basis = _NATIVE.texture_projection_basis
projection_axes = _NATIVE.texture_projection_axes
planar_uv = _NATIVE.texture_planar_uv


def face_uvs(face, tex: dict):
    """Planar UVs for ``face``'s outer-loop vertices from a texture attrs dict."""
    return planar_uv(face.normal(), list(face.vertices),
                     float(tex.get("sw", 1.0)), float(tex.get("sh", 1.0)),
                     float(tex.get("rot", 0.0)))


# ``fit_uv_affine(points, uvs)``: the world→UV affine map ``[gu.xyz, u0,
# gv.xyz, v0]`` fitted from a polygon's vertices and their explicit UVs (a
# COLLADA/OBJ import), from the edge pair with the largest cross product;
# ``None`` when degenerate. Any UV assignment on a planar polygon is affine
# over its plane, so coplanar triangles of one original face merge and still
# texture correctly. ``affine_uv(uvw, positions)`` evaluates it.
#
# ``face_uv_axes(tex, normal)``: the affine map a textured face renders with,
# ``(gu, cu, gv, cv)`` with ``u = gu·p + cu``: the fitted ``uvw`` it carries,
# or the .skp planar projection. ONE definition of where the texture sits on
# a face, for the renderer and the exporters alike.
#
# ``placement_of(tex, normal)``: ``(sw, sh, rot)``, the tile size and in-plane
# turn a fitted map shows on the face of ``normal`` (what the eyedropper hands
# to a face on another plane; shear and mirror dropped), or ``None``.
fit_uv_affine = _NATIVE.texture_fit_uv_affine
affine_uv = _NATIVE.texture_affine_uv
face_uv_axes = _NATIVE.texture_face_uv_axes
placement_of = _NATIVE.texture_placement_of


#: ``flattened_texture(tex, normal=None)``: ``tex`` without its per-face map,
#: keeping the look it had on the plane of ``normal`` as a planar projection
#: (``sw``/``sh``/``rot`` from :func:`placement_of`).
flattened_texture = _NATIVE.texture_flattened


#: The map ``uv(p) = g·p + c`` carried across a hinge: ``uv(R⁻¹ p)`` for the
#: rotation about the shared edge that lays the previous face onto the new
#: one, so the image continues across the edge without a cut.
_rotated_map = _NATIVE.texture_rotated_map


def continuous_maps(mesh, faces, seed, tex: dict) -> dict:
    """Per-face texture dicts that make ``tex`` run CONTINUOUSLY over a
    curved surface — ``faces`` joined by soft edges — the classic way to
    paint a cylinder or a rounded corner: the image is laid on ``seed``
    (its planar projection, or the map it already carries) and walked to
    each neighbour across their shared soft edge, turned about that edge
    into the neighbour's plane, so bricks wrap around the bend instead of
    restarting on every facet (Marco's rounded corner, 2026-09-15).

    Returns ``{face.key: tex dict with uvw}`` for the faces reached from
    ``seed``; faces of the set not connected to it are left out (the
    caller paints them the ordinary way)."""
    faces = list(faces)
    if seed is None or not faces:
        return {}
    ids = {f.key for f in faces}
    gu, cu, gv, cv = face_uv_axes(tex, seed.normal())
    out = {seed.key: (gu, cu, gv, cv)}
    stack = [seed]
    while stack:
        f = stack.pop()
        g_u, c_u, g_v, c_v = out[f.key]
        n1 = f.normal()
        if n1.lengthSquared() < 1e-18:
            continue
        n1 = n1.normalized()
        for lp in (f.loop, *f.hole_loops):
            n = len(lp)
            for i in range(n):
                e = mesh.find_edge(lp[i], lp[(i + 1) % n])
                if e is None or not e.soft:
                    continue
                for g in e.faces:
                    if g == f or g.key not in ids or g.key in out:
                        continue
                    n2 = g.normal()
                    d = e.b - e.a
                    if n2.lengthSquared() < 1e-18 or d.lengthSquared() < 1e-18:
                        continue
                    n2 = n2.normalized()
                    d = d.normalized()
                    cos_t = QVector3D.dotProduct(n1, n2)
                    sin_t = QVector3D.dotProduct(d, QVector3D.crossProduct(n1, n2))
                    if cos_t < -0.999:
                        continue                # folded back: no hinge
                    out[g.key] = _rotated_map(g_u, c_u, g_v, c_v, e.a, d,
                                              cos_t, sin_t)
                    stack.append(g)
    result = {}
    base = {k: v for k, v in tex.items() if k not in ("uvw", "rot")}
    for f in faces:
        m = out.get(f.key)
        if m is None:
            continue
        gu, cu, gv, cv = m
        result[f.key] = {**base, "uvw": [gu.x(), gu.y(), gu.z(), float(cu),
                                         gv.x(), gv.y(), gv.z(), float(cv)]}
    return result


#: ``uv_reference_points(points, normal=None)``: three points on the face's
#: plane forming a right-angled triad sized to the face, for pinning an affine
#: UV map (NOT face vertices: a long thin face's corners are nearly collinear,
#: and a map fitted through that sliver came back rotated), or ``None``.
uv_reference_points = _NATIVE.texture_uv_reference_points


# ---- Colourize (tinted .skp materials) -----------------------------------
#
# The .skp format lets a TEXTURED material also carry a colour, and re-tints the
# image toward it. Two modes, and they are genuinely different pictures:
# ``SHIFT`` moves every pixel by the delta between the image average and the
# target (the stone keeps its veining, moved in tone), ``TINT`` replaces hue
# and saturation outright and keeps only the per-pixel lightness.
#
# This lived in ``formats.skp_openskp`` when the only caller was the .skp
# import. It is engine, not import: the material editor tints from here too,
# and the importer now calls in.

COLORIZE_SHIFT = 0
COLORIZE_TINT = 1


#: Vectorized ``colorsys.rgb_to_hls`` over an (..., 3) array in 0..1, and
#: ``hls_to_rgb(hue, lum, sat)`` back to (..., 3).
rgb_to_hls = _NATIVE.texture_rgb_to_hls
hls_to_rgb = _NATIVE.texture_hls_to_rgb


def colorize_image(data, target_rgb, ctype=COLORIZE_SHIFT):
    """Re-tint a shared texture the way the .skp format defines a
    colourized material copy ("[Name]1", ``type="2"``).

    ``ctype`` 0 ("shift") moves every pixel's hue/lightness/saturation by
    the delta between the image average and the material colour; ``1``
    ("tint") replaces hue/saturation outright, keeping the per-pixel
    lightness variation. Alpha is preserved (the chain-link cutout must
    survive). Returns PNG bytes, or ``data`` unchanged on any failure."""
    try:
        from ingetrazo_render import SurveyRaster
        img = SurveyRaster.decode(bytes(data))
        if img is None:
            return data
        rgba = img.rgba()
        out = _NATIVE.texture_colorize_pixels(rgba, _NATIVE.texture_mean_visible(rgba),
                                              tuple(target_rgb[:3]), int(ctype))
        return SurveyRaster(img.width, img.height, out).material_png(bytes(data))
    except Exception:
        return data


def tinted_texture(tex: dict, color, ctype: int = COLORIZE_SHIFT) -> dict:
    """A copy of the ``attrs["texture"]`` dict *tex*, re-tinted toward
    ``color`` (RGB floats 0–1).

    The tint is BAKED into the image the entry points at, and the untinted
    source is remembered under ``"base"``. That split is what makes the
    feature both cheap and non-destructive:

    - ``"path"`` keeps meaning "the image to draw", so the renderer, the six
      exporters, the compositor and the takeoffs need no changes at all —
      ``tex["path"]`` is read in 37 places across 13 files.
    - re-tinting always starts from ``"base"``, never from the last result,
      so sliding through colours cannot degrade the picture, and removing the
      colour restores the original exactly (:func:`untinted_texture`).

    The cache is content-addressed, so the same base tinted the same way is
    written and uploaded once, however many materials ask for it. Returns
    *tex* unchanged if the source image cannot be read.
    """
    base = tex.get("base") or tex.get("path")
    if not base:
        return dict(tex)
    try:
        data = Path(base).read_bytes()
    except OSError:
        return dict(tex)
    rgb255 = tuple(int(round(max(0.0, min(1.0, c)) * 255.0))
                   for c in tuple(color)[:3])
    out = colorize_image(data, rgb255, ctype)
    if out is data:                       # colorize failed: leave it alone
        return dict(tex)
    try:
        path = cache_image(out, Path(base).name, "tinted")
    except OSError:
        return dict(tex)
    entry = dict(tex)
    entry["path"] = str(path)
    entry["base"] = str(base)
    entry["tint"] = [float(c) for c in tuple(color)[:3]]
    entry["tint_mode"] = int(ctype)
    return entry


def untinted_texture(tex: dict) -> dict:
    """The entry with its colour removed: back to the original image."""
    base = tex.get("base")
    entry = {k: v for k, v in tex.items()
             if k not in ("base", "tint", "tint_mode")}
    if base:
        entry["path"] = base
    return entry
