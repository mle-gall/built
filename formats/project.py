# SPDX-License-Identifier: GPL-3.0-or-later
"""The Built project manifest (docs/builtproject-format.md).

The file I/O, the envelope checks, conversion and merging are Rust
(``ingetrazo_core.project_*``); the registry's contents stay Python's
(``core.bim_registry.validate``), and link filters the Navigator grammar's.
"""
import json
from pathlib import Path

SUFFIX = '.builtproject'
LEGACY_SUFFIX = '.igzproj'
SUFFIXES = (SUFFIX, LEGACY_SUFFIX)


def is_project(path):
    return Path(path).suffix.lower() in SUFFIXES


def _check(value):
    from core.links import validate_select
    for edge in value['links']:
        validate_select(edge.get('select'))
    from core.bim_registry import validate
    errors = validate(value['bim_registry'])
    if errors:
        raise ValueError('; '.join(errors))
    return value


def open_path(path):
    """The manifest for ``path``; an ``.igzproj`` is converted in place first
    (or its existing ``.builtproject`` sibling is used)."""
    from ingetrazo_core import project_convert
    manifest, _how = project_convert(Path(path))
    return Path(manifest)


def load(path):
    """The validated manifest (converting an ``.igzproj`` it names)."""
    from ingetrazo_core import project_manifest_load
    return _check(json.loads(project_manifest_load(open_path(path))))


def peek(path):
    """A manifest or an ``.igzproj``, read-only: nothing is converted."""
    from ingetrazo_core import project_manifest_peek
    return json.loads(project_manifest_peek(Path(path)))


def save(path, value):
    from ingetrazo_core import project_manifest_save
    _check(value)
    project_manifest_save(Path(path), json.dumps(value, ensure_ascii=False, allow_nan=False))


def create(path, registry, identity):
    from ingetrazo_core import project_manifest_new
    _check(dict(links=[], bim_registry=registry))
    return json.loads(project_manifest_new(Path(path), json.dumps(registry), json.dumps(identity)))
