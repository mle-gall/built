"""Occurrence query and atomic, field-sized patches for the native inspector."""

from copy import deepcopy

from core import bim_registry, record, structure
from core.history import CompoundCommand, SetRecordCommand


def _recipe(group, registry):
    node = structure.node_of(group)
    return structure.resolve(registry, node) if node in structure._nodes(registry) else {}


def occurrence(group, registry, linked=False):
    """Return only export fields and their owners. Filing materialises its defaults."""
    own = record.normalize(group.ifc or {})
    recipe = record.normalize(_recipe(group, registry))
    effective = record.apply_type(own, registry)
    fields = []

    def add(path, value, owner, editable, kind="text"):
        fields.append(
            dict(
                path=path,
                value=value,
                owner="Linked source" if linked else owner,
                editable=editable and not linked and not group.locked,
                kind=kind,
            )
        )

    for key in ("class", "predefined_type", "type", "object_type", "name", "description", "tag"):
        filed = bool(recipe.get("class")) and key in (
            "class",
            "predefined_type",
            "type",
            "object_type",
        )
        owner = (
            "Type"
            if key in effective and key not in own
            else "Filing node" if filed else "This object"
        )
        add([key], effective.get(key) or "", owner, not filed)
    for name, props in (effective.get("psets") or {}).items():
        if not isinstance(props, dict):
            continue
        for key, entry in props.items():
            default = recipe.get("psets", {}).get(name, {}).get(key)
            owner = (
                "Type"
                if key not in own.get("psets", {}).get(name, {})
                else "Filing node" if entry == default else "This object"
            )
            add(["psets", name, key], entry.get("value"), owner, True, entry.get("kind"))
    filed_codes = {c["system"] for c in recipe.get("classifications", [])}
    for c in effective.get("classifications") or []:
        if isinstance(c, dict):
            system, code = c.get("system", ""), c.get("code", "")
            add(
                ["classifications", system, code],
                c.get("name", ""),
                (
                    "Filing node"
                    if system in filed_codes
                    else "This object" if "classifications" in own else "Type"
                ),
                system not in filed_codes,
            )
    if effective.get("guid"):
        add(["guid"], effective["guid"], "This object", False)
    return dict(
        uid=group.uid,
        fields=fields,
        ifc_class=effective.get("class"),
        editable=not (linked or group.locked),
    )


def patch_record_field(history, placements, path, value, schema="IFC4", property_kind=None):
    """Validate all proposed effective records before creating one history step."""
    from core.protocol.commands import _answer

    groups = structure._targets(history.scene, placements)
    registry = bim_registry.get(history.scene)
    commands = []
    for g in groups:
        row = next((r for r in occurrence(g, registry)["fields"] if r["path"] == path), None)
        if (
            row is None
            and len(path) == 3
            and path[0] == "psets"
            and property_kind in record.KINDS
            and all(isinstance(k, str) and k.strip() for k in path)
        ):
            row = {"editable": True, "kind": property_kind}
        if row is None or not row["editable"] or getattr(g, "locked", False):
            raise ValueError("Field is read-only or absent on " + g.uid)
        old = g.ifc or {}
        if path[0] == "psets":
            _, pset, prop = path
            psets = deepcopy(old.get("psets", {}))
            entry = record.typed(psets.get(pset, {}).get(prop, {}))
            if value is None:
                default = _recipe(g, registry).get("psets", {}).get(pset, {}).get(prop)
                if default is not None:
                    psets.setdefault(pset, {})[prop] = record.typed(default)
                else:
                    psets.get(pset, {}).pop(prop, None)
                    if not psets.get(pset):
                        psets.pop(pset, None)
            else:
                psets.setdefault(pset, {})[prop] = {**entry, "kind": row["kind"], "value": value}
            patch = {"psets": psets}
        elif path[0] == "classifications":
            codes = deepcopy(record.apply_type(old, registry)["classifications"])
            for c in codes:
                if [c.get("system"), c.get("code")] == path[1:]:
                    c["name"] = value
            patch = {"classifications": codes}
        else:
            patch = {path[0]: value}
            if path[0] in ("type", "predefined_type") and value == "":
                patch[path[0]] = None
        errors = record.validate(record.apply_type({**old, **patch}, registry), registry, schema)
        if errors:
            raise ValueError("; ".join(errors))
        commands.append(SetRecordCommand([g], patch))
    before = history.revision
    history.execute(CompoundCommand(commands) if len(commands) > 1 else commands[0])
    return _answer(history, before)


def filing_rule(scene, key, recipe, *, commit_history=None, expected=None):
    """Plan the registry write and all descendant updates against one snapshot."""
    from core.history import SetRegistryCommand

    registry = bim_registry.get(scene)
    node = structure._nodes(registry).get(key)
    if node is None or "source" in node:
        raise ValueError("Select an editable filing node")
    if not isinstance(recipe, dict):
        raise ValueError("The filing rule must be an object")
    if expected is not None and node.get("recipe", {}) != expected:
        raise ValueError("The filing rule changed. Reopen the preview.")
    node["recipe"] = deepcopy(recipe)
    update = SetRegistryCommand(registry)
    sync, report = structure._resync(scene, registry, [key])
    if commit_history is not None:
        commit_history.execute(CompoundCommand([update, *sync.commands]))
    return {"changed": len(report), "changes": report}


def set_filing_rule(history, node, recipe, expected):
    from core.protocol.commands import _answer

    before = history.revision
    filing_rule(history.scene, node, recipe, commit_history=history, expected=expected)
    return _answer(history, before)
