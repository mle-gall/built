# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Built contributors.
"""Persistent IFC identities for elements and document-owned relationships."""
import json
import uuid

_CHARS = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$"


def compressed_guid(value):
    n = value.int
    return "".join(_CHARS[(n >> (6 * i)) & 63] for i in range(21, -1, -1))


def stable_guid(namespace, *parts):
    key = json.dumps(parts, ensure_ascii=True, separators=(",", ":"))
    return compressed_guid(uuid.uuid5(uuid.UUID(namespace), key))


def ensure_guid(tag):
    if not tag.get("guid"):
        tag["guid"] = compressed_guid(uuid.uuid4())
    return tag["guid"]


def document_namespace(scene):
    """The namespace IFC identities derive from: the project's or the
    document's stored one, else the document id (docs/persistent-ids.md
    §7.1). Choosing it writes nothing."""
    identity = (getattr(scene, "plugin_data", None) or {}).get("ifc_identity") or {}
    if identity.get("namespace"):
        return identity["namespace"]
    from built_core.element_ids import ensure_doc_id
    return ensure_doc_id(scene)


def occurrence_guid(scene, path, namespace=None):
    """The GlobalId of a placement without a stored GUID: derived from its
    persistent uid path, the same in every export and session."""
    return stable_guid(namespace or document_namespace(scene), "occurrence", *path)


def component_guid(scene, tool_id):
    """A BCF component's IfcGuid from its authoring tool id: a placement
    uid, a uid path (`a/b`), optionally `built:`-prefixed and `#element`
    suffixed. None when no placement has it."""
    text = str(tool_id or "")
    if text.startswith("built:"):
        text = text[len("built:"):]
    text = text.split("#", 1)[0]
    path = [p for p in text.split("/") if p]
    if not path:
        return None
    from built_core.group import iter_placements
    target = path[-1]
    for root in scene.groups:
        for group, _ in iter_placements(root):
            if group.uid == target:
                stored = group.ifc.get("guid") if group.ifc else None
                full = scene.occurrence_path(group) or [group.uid]
                return stored or occurrence_guid(scene, [str(p) for p in full])
    return None


def object_guid(scene, obj, occurrence=None):
    group = obj.get("group")
    if group is not None:
        # A stored GUID wins (imported, typed, or saved by an earlier
        # build); otherwise derive it, without writing to the document.
        stored = group.ifc.get("guid")
        if stored:
            return stored
        path = occurrence or scene.occurrence_path(group) or [group.uid]
        return occurrence_guid(scene, [str(p) for p in path])
    owner = obj.get("owner")
    # Shared definitions cannot store a placement's GUID on their faces.
    if owner is not None:
        from built_core.mesh import DeferredMesh
        if type(owner.mesh) is DeferredMesh:
            # Persist into the deferred document metadata, so both building
            # and saving later see the identity without building faces now.
            src = owner.mesh.__dict__["_deferred_source"]
            table = src["meta"].get("face_attrs") or []
            attributes = [table[i] for i in dict.fromkeys(src["face_attr"]) if i >= 0]
        else:
            attributes = [f.attrs for f in owner.mesh.faces]
        attributes = [a for a in attributes
                      if a.get("ifc", {}).get("id") == obj["tag_id"]]
    else:
        attributes = [f.attrs for f in obj["faces"]]
    guid = next((a["ifc"]["guid"] for a in attributes
                 if a["ifc"].get("guid")), None)
    guid = guid or compressed_guid(uuid.uuid4())
    for attrs in attributes:
        if attrs["ifc"].get("guid") != guid:
            attrs["ifc"] = {**attrs["ifc"], "guid": guid}
            scene.version += 1
    if owner is not None:
        return stable_guid(document_namespace(scene),
                           "faces", occurrence or owner.uid, guid)
    return guid


def copied_face_attrs(scene=None):
    """One remapper per placed copy, keeping faces of one object together."""
    # Copies can be planned before any command enters the scene (arrays).
    # Random integer ids avoid reserving mutable document state while planning.
    next_id = uuid.uuid4().int >> 80
    tags = {}

    def copy_attrs(attrs):
        nonlocal next_id
        out = dict(attrs or {})
        tag = out.get("ifc")
        if tag:
            key = tag.get("id")
            if key not in tags:
                tags[key] = {k: v for k, v in tag.items() if k != "guid"}
                tags[key]["id"] = next_id
                next_id += 1
                ensure_guid(tags[key])
            out["ifc"] = dict(tags[key])
        return out
    return copy_attrs
