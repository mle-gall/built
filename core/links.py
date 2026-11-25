# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Linked files: another .igz placed in this one as a live reference.

A linked file is a component whose definition is not the host's: it is the
source document, loaded from disk. The source stays a document of its own —
open it, edit it, save it, and every host that links it shows the change
(the window watches the files). Inside the host the linked part moves,
turns and scales like any component, but it cannot be opened for editing
there: entering it offers to open the source.

What is stored where
--------------------
* Each placed copy carries ``Group.ext["link"] = {"path": …}`` — the source,
  absolute — and ``"rel"``, the same path relative to the host when it was
  last saved, tried first so a project folder can move as a whole.
* The host's links registry (``scene.link_registry``, a root table of the
  document; the file stores it as ``plugin_data["links"]``) keeps, per
  source, the tag overrides (below) and whether the source is *packed*.
* Unpacked (the default), the host file does not hold the source's
  geometry: the copies are saved as empty placements and filled in again
  from the source when the host is opened (:func:`resolve`). Packed —
  Blender's «Pack external data» — the geometry is saved in the host like
  any component's and the source is no longer read; unpacking links it
  again.

Tags
----
The source's tags come into the host under their own names prefixed by the
file's («Site / Wood»), so they never mix with the host's tags of the same
name, in a folder named after the file inside «Linked files», where the
source's own folders are rebuilt. Each tag shows as the source has it,
unless it was switched in the host: that switch is an *override*, kept by
the host and never written to the source, and the source's later changes to
that tag no longer reach it.

The logic is Rust: ``rust/ingetrazo_core/src/model/links.rs`` (the queries,
the registry, the definition renamed as it is read, the tags mirrored and
released, the copies of a source's tree) and ``model/link_io.rs`` (a source
read into a definition, a link made, reloaded, resolved on open, the
nested-link policy, the overrides cleared). This module keeps the names
plugins use, the constants, :class:`LinkError` and the undoable registry
step.
"""

from __future__ import annotations

import ingetrazo_core as _rs

# The plugin surface (docs/model-port.md).
link_of = _rs.link_of
fill_link = _rs.link_fill
is_linked = _rs.link_is_linked
linked_groups = _rs.link_linked_groups
sources = _rs.link_sources
link_state = _rs.link_state
make_link = _rs.link_make
reload = _rs.link_reload

LINK_KEY = "link"                 # Group.ext key on each placed copy
REGISTRY_KEY = "links"            # the file's plugin_data key for the registry
LINKED_FOLDER = "Linked files"    # the tag folder all links' folders sit in
SEP = " / "                       # between the file's name and a tag's


class LinkError(Exception):
    """A source that cannot be linked: missing, unreadable, or a cycle."""


_SET_REGISTRY = None


def set_registry_command(value):
    """An undoable step that replaces the links registry with ``value``
    (a copy, through JSON both ways, as ``SetPluginDataCommand`` copies).
    The registry is a document value: the step's transaction undoes it."""
    global _SET_REGISTRY
    if _SET_REGISTRY is None:
        import json
        from ingetrazo_core.history import Command

        def copy(v):
            return json.loads(json.dumps(v))

        class SetLinkRegistryCommand(Command):
            doc_undo = True

            def __init__(self, value) -> None:
                self.value = copy(value)
                self._before = None

            def do(self, scene) -> None:
                self._before = copy(scene.link_registry)
                scene.link_registry = copy(self.value)
                scene.version += 1

            def undo(self, scene) -> None:
                scene.link_registry = copy(self._before)
                scene.version += 1

        _SET_REGISTRY = SetLinkRegistryCommand
    return _SET_REGISTRY(value)
