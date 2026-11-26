"""IFC vocabulary and validated edits for the native record inspector."""
from copy import deepcopy

from core import bim_registry, record
from built_core import structure
from built_core.history import CompoundCommand, SetRecordCommand
from core.ifc_tables import TABLES
from built_core.protocol.records import occurrence


def vocabulary(registry):
    favourites = list(dict.fromkeys(entry["class"] for group in registry.get("tag_sets", {}).values()
                                    if group.get("default") for entry in group.get("classes", ())))
    return {
        "favourites": favourites,
        "classes": {k: {p: v[p] for p in ("domain", "schemas", "predefined_types")} for k, v in TABLES.items()},
        "systems": registry.get("systems", {}),
        "classifications": registry.get("classification_systems", {}),
        "types": sorted(registry.get("types", {})),
    }


def edit(history, paths, action, values, schema):
    """Validate every occurrence before recording a single undo step."""
    scene = history.scene
    if getattr(scene, "worksession_read_only", False):
        raise ValueError("Read-only document")
    targets = structure._targets(scene, paths)
    registry = bim_registry.get(scene)
    commands = []
    for group in targets:
        old = group.ifc or {}
        view = occurrence(group, registry)
        fields = {tuple(f["path"]): f for f in view["fields"]}
        node = structure.node_of(group)
        rule = structure.resolve(registry, node) if node in structure._nodes(registry) else {}
        if scene.entity_locked(group) or scene._layer_state(group)[1] or not view["editable"]:
            raise ValueError("Read-only record")
        if action == "untag":
            if rule.get("class"):
                raise ValueError("This class comes from a filing node")
            if old:
                commands.append(SetRecordCommand([group], None))
            continue
        effective = record.apply_type(old, registry)
        if schema not in ("IFC4", "IFC4X3_ADD2"):
            raise ValueError("Unsupported IFC schema")
        if action == "fields":
            patch = deepcopy(values)
            for key in patch:
                if key not in ("class", "name", "object_type", "predefined_type", "type", "tag", "description", "quantities"):
                    raise ValueError("Unknown record field")
                if key != "quantities" and not fields[key,]["editable"]:
                    raise ValueError("This field comes from a filing node")
            if "class" in patch and rule.get("classes_allowed") is not None and patch["class"] not in rule["classes_allowed"]:
                raise ValueError("Class is outside the filing rule allowlist")
            if "class" in patch and effective.get("predefined_type"):
                valid_types = TABLES.get(patch["class"], {}).get("predefined_types", {}).get(schema, ())
                if effective["predefined_type"] not in valid_types:
                    patch.setdefault("predefined_type", None)
        elif action == "quantity":
            key, value = values
            if not isinstance(key, str) or not key.strip():
                raise ValueError("Enter a quantity name")
            quantities = deepcopy(effective.get("quantities", {}))
            entry = quantities.setdefault(key, {})
            entry.update(value=value, source="manual")
            patch = {"quantities": quantities}
        elif action == "system":
            key, member = values
            if key not in registry.get("systems", {}) or type(member) is not bool:
                raise ValueError("Unknown system membership")
            systems = list(effective.get("systems") or [])
            if member == (key in systems):
                continue
            patch = {"systems": systems + [key] if member else [s for s in systems if s != key]}
        elif action == "classification":
            system, code, name, member = values
            if system not in registry.get("classification_systems", {}) or not isinstance(code, str) or not code.strip():
                raise ValueError("Choose a classification system and code")
            if any(c["system"] == system for c in rule.get("classifications", [])):
                raise ValueError("This classification comes from a filing node")
            codes = effective.get("classifications") or []
            rows = [c for c in codes if (c["system"], c["code"]) != (system, code)]
            if member:
                rows.append(dict(system=system, code=code, name=name))
            patch = {"classifications": rows}
        else:
            raise ValueError("Unknown record action")
        if not effective.get("class") and not patch.get("class"):
            raise ValueError("Choose an IFC class first")
        errors = record.validate(record.apply_type({**old, **patch}, registry), registry, schema)
        if errors:
            raise ValueError("\n".join(errors))
        if any(old.get(key) != value for key, value in patch.items()):
            commands.append(SetRecordCommand([group], patch))
    if commands:
        history.execute(commands[0] if len(commands) == 1 else CompoundCommand(commands))
    return len(commands)
