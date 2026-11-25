# SPDX-License-Identifier: GPL-3.0-or-later
"""The Project tree's nodes, linked files mounted under their link row
(ui/tree-links, 2026-11-22).

The host's registry keeps a *snapshot* of each linked file's structure,
taken when the host's structure was proposed (``structure.bootstrap``).
That snapshot goes stale when a source is reloaded, it puts nested links'
stems at the top level, and it misses the items of a link inside a link.
The tree therefore mounts every linked file from what is current:

* the host's own nodes (registry nodes without ``source``), in their order;
* then one root row per linked file (``link:<stem>``), typed as a file
  (tree A + C, docs/ui-decisions.md, 2026-11-23). Nested links are roots
  too, each file once, marked ``via`` the file that places it
  (``link:A/link:C`` via A), right after it: every file sits at one
  predictable place, as in Navisworks' and Solibri's model trees. A file
  placed through two parents (or by the host and by a linked file) is one
  row, the shortest chain's; the other chains' parts are counted there;
* under a file's row, that file's own structure as the file has it (the
  host's ``scene.link_registry[path]["structure"]``, refreshed on every
  reload). A nested file's structure is the one its parent file mounted
  (``link:C/…`` nodes in the parent's structure). When that structure has
  one top node («Ride» in Ride.igz), the node is merged into the file row:
  its children and items hang from the row, so the tree never reads
  "Ride › Ride";
* a file without a structure shows its tag folders and tags instead, as
  the host mirrors them («A / C / …»), merged the same way when there is
  one top folder.

Host first, then the files: what you work on comes before what you only
reference (the order of Revit's Project Browser).

Every placement belongs to at most one display node: the host's own by
their ``structure.node``; a linked file's placement to the file's row; a
part of a linked file to the node its own file filed it in, else to its
tag's node, else (a top part of the file only) to the file's row.

Keys are display keys, never written to the registry: ``link:…`` keys are
read-only (``structure.file`` refuses linked structure anyway).
"""
from __future__ import annotations

import ingetrazo_core
import json

from core import links

LINKED = "linked"          # the former «Linked files» row (keys stay read-only)
LINKED_NAME = links.LINKED_FOLDER
#: A file row's states (dot colours in the dock): found, changed on disk
#: since it was read, not found, or packed into this document.
STATES = ("resolved", "modified", "missing", "packed")


def row_id(address):
    """Encode an occurrence address for native controls that echo a string."""
    return address if isinstance(address, str) else "@" + json.dumps(address, separators=(",", ":"))


def mount_key(stems) -> str:
    return "link:" + "/link:".join(stems)


def is_linked_key(key) -> bool:
    return isinstance(key, str) and (key == LINKED or key.startswith("link:"))


class Tree:
    """``nodes``: display key → {name, parent, order, kind ("host",
    "link", "node", "tag", "tagfolder"), path, stems, recipe,
    tag (host tag key or None), folder (host folder key or None)}; a file
    row ("link") adds ``via`` (the stem of the file placing it, None for
    the host's own links), ``folders`` (every host folder its chains
    mirror it in) and ``merged`` (the name of its single top node, folded
    into the row, or None);
    ``children``: key → ordered child keys (None: the roots);
    ``member``: id(group) → key; ``direct``: key → groups;
    ``links``: link-row key → the link placements it stands for;
    ``unfiled``: the host's own unsorted placements."""

    def __init__(self, scene, registry):
        ingetrazo_core.build_project_tree(self, scene, registry)

    def address(self, group):
        """Keep legacy row UIDs when they identify a single occurrence."""
        if group.uid in self._qualified or group.uid.startswith("@["):
            return self.scene.occurrence_path(group)
        return group.uid

    def row_id(self, group):
        return row_id(self.address(group))

    def resolve(self, key):
        """The row a display key stands for (aliases followed)."""
        seen = 0
        while key in self.alias and seen < 64:
            key, seen = self.alias[key], seen + 1
        return key

    # -- reads -------------------------------------------------------------
    def order(self):
        """``(key, depth)`` in display order."""
        return ingetrazo_core.tree_order(self.children)

    def subtree(self, key):
        return ingetrazo_core.tree_subtree(self.children, key)

    def members(self, key):
        return [g for k in self.subtree(key) for g in self.direct.get(k, ())]

    def file_tags(self, key):
        """Host tag keys of the linked file (or files) a row stands for."""
        node = self.nodes.get(key)
        if node is None or node["kind"] != "link":
            return []
        bases = node["folders"]
        pre = tuple(b + links.SEP for b in bases)
        return [ly.key for ly in self.scene.layers if pre and ly.key.startswith(pre)]


def file_state(scene, path, nested=False) -> str:
    """Read the same source status as Problems and the inspector."""
    return links.link_state(scene, path)
