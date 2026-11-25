"""Reuse previously installed IGZ geometry while reading a linked definition."""
from hashlib import sha256
import json


def _digest(block, geom, compressed):
    """Hash geometry and element attributes independently of the ZIP member name."""
    if "geom" in block:
        meta = dict(block["geom"])
        member = meta.pop("member")
        h = sha256(compressed[member]) if member in compressed else sha256(geom[member])
        h.update(json.dumps(meta, sort_keys=True).encode())
        return h.digest()
    if "_arrays" in block:
        import numpy as np
        h = sha256()
        for name, value in sorted(block["_arrays"].items()):
            h.update(name.encode())
            if isinstance(value, np.ndarray):
                h.update(str((value.dtype.str, value.shape)).encode())
                h.update(memoryview(np.ascontiguousarray(value)).cast("B") if value.size else b"")
            else:
                h.update(json.dumps(value, sort_keys=True).encode())
        return h.digest()
    return sha256(json.dumps((block.get("edges", []), block.get("faces", [])),
                             sort_keys=True).encode()).digest()


class LinkMeshes:
    def __init__(self, path, registry, previous=None, *, filtered=False):
        from ingetrazo_core import purge_iter_groups as iter_groups
        self.path = str(path)
        self.registry = registry
        self.previous = {}
        self.reused = set()
        self.filtered = filtered
        for group in iter_groups([previous] if previous is not None else []):
            mesh = group.mesh
            stamp = mesh.__dict__.get("_link_geometry")
            if stamp and stamp[1] == mesh.revision:
                self.previous[stamp[0]] = mesh

    def prepare(self, materials, payload):
        # Prototype numbers follow write order. A stable placement identifies
        # each shared definition even when sibling groups change order.
        self.proto_uids = {}
        stack = list(payload.get("groups") or []) + list(payload.get("kept_components") or [])
        while stack:
            group = stack.pop()
            if not isinstance(group, dict):
                continue
            if group.get("xform") is not None and "proto" in group and group.get("uid"):
                code, uid = int(group["proto"]), str(group["uid"])
                self.proto_uids[code] = min(uid, self.proto_uids.get(code, uid))
            stack.extend(group.get("children") or [])
        self.compressed = {}

        from ingetrazo_core.materials import register
        registry = dict(self.registry)
        self.renames = {name: final for name, mat in materials.items()
                        if (final := register(registry, mat)) != name}

    def key(self, block, geom, identity):
        if identity[0] == "proto":
            identity = ("proto", self.proto_uids.get(identity[1], identity[1]))
        names = set()
        if self.renames:
            meta = block.get("geom") or block.get("_arrays", {}).get("meta")
            attrs = meta.get("face_attrs", ()) if meta else block.get("faces", ())
            for row in attrs:
                names.add(row.get("mat"))
                if isinstance(row.get("back"), dict):
                    names.add(row["back"].get("mat"))
        renamed = tuple(sorted((name, self.renames[name]) for name in names if name in self.renames))
        return self.path, renamed, identity, _digest(block, geom, self.compressed)

    def unchanged_members(self, archive, payload):
        """Read compressed bytes once. Only changed members need inflation,
        validation and native mesh construction. zipfile checks the local
        header before exposing the member's data offset."""
        from ingetrazo_core.formats.igz import _mesh_blocks
        protos = {id(raw): i for i, raw in enumerate(payload.get("protos") or [])}
        unchanged, needed = set(), set()
        for block in _mesh_blocks(payload):
            if "geom" not in block:
                # The legacy curve-name migration reads every binary curve.
                if any(isinstance(e.get("curve"), str) for e in block.get("edges", ())):
                    return ()
                continue
            member = block["geom"]["member"]
            if member not in self.compressed:
                info = archive.getinfo(member)
                with archive.open(info):
                    remaining = info.compress_size
                    h = sha256()
                    while remaining:
                        chunk = archive.fp.read(min(remaining, 1 << 20))
                        if not chunk:
                            raise ValueError("truncated geometry member")
                        h.update(chunk)
                        remaining -= len(chunk)
                self.compressed[member] = (h.digest() + str((info.compress_type,
                                            info.file_size, info.CRC)).encode())
            if block is payload:
                identity = ("loose",)
            elif id(block) in protos:
                identity = ("proto", protos[id(block)])
            else:
                identity = ("group", str(block.get("uid", "")))
            if not self.filtered and self.key(block, {}, identity) in self.previous:
                unchanged.add(member)
            else:
                needed.add(member)
        return unchanged - needed

    def read(self, mesh, block, geom, identity):
        from ingetrazo_core.formats.igz import _load_mesh
        key = self.key(block, geom, identity)
        old = self.previous.get(key)
        if old is not None and not self.filtered:
            self.reused.add(old)
            return old
        _load_mesh(mesh, block, geom)
        mesh.__dict__["_link_geometry"] = key, mesh.revision
        return mesh
