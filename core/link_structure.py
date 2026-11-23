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
  host's ``plugin_data["links"][path]["structure"]``, refreshed on every
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

from collections import Counter, defaultdict
import json

from core import links, structure

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


def _children_of(table):
    kids = defaultdict(list)
    for k, n in table.items():
        kids[n.get("parent")].append(k)
    return kids


def _own(table):
    """A file's own nodes: neither mounted from another file nor the
    «Linked files» folder that only holds such mounts."""
    own = {k: n for k, n in table.items()
           if isinstance(n, dict) and "source" not in n and not str(k).startswith("link:")}
    kids = _children_of(table)
    for k in [k for k, n in own.items() if n.get("name") == LINKED_NAME]:
        if not any(c in own for c in kids.get(k, ())):
            own.pop(k)
    return own


def _subtable(table, stem):
    """The structure a file mounted for its link ``stem``, in that link's
    own key space (``link:<stem>/x`` → ``x``)."""
    pre, top = f"link:{stem}/", f"link:{stem}"
    out = {}
    for k, n in table.items():
        if isinstance(k, str) and k.startswith(pre) and isinstance(n, dict):
            p = n.get("parent")
            p = None if p in (None, top) else p[len(pre):] if isinstance(p, str) and p.startswith(pre) else None
            node = {f: v for f, v in n.items() if f not in ("source", "source_node", "mount")}
            node["parent"] = p
            if "recipe" not in node and "source_recipe" in n:
                node["recipe"] = n["source_recipe"]
            out[k[len(pre):]] = node
    return out


def walk(scene):
    """Every placement once: ``(group, chain, inner, file_root)`` where
    ``chain`` is the ``(stem, path)`` links above it, ``inner`` the same
    with the group itself when it is a link, and ``file_root`` whether it
    is a top placement of the file it is in (the host or a linked file)."""
    seen = set()
    stack = [(g, (), True) for g in reversed(getattr(scene, "groups", None) or ())]
    while stack:
        g, chain, root = stack.pop()
        if id(g) in seen:
            continue
        seen.add(id(g))
        link = links.link_of(g)
        inner = chain + ((links.stem(link["path"]), links._norm(link["path"])),) if link else chain
        yield g, chain, inner, root
        kids = getattr(g, "children", None) or ()
        stack.extend((c, inner, bool(link)) for c in reversed(kids))


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
        self.scene = scene
        self.nodes, self.direct = {}, defaultdict(list)
        self.member, self.links = {}, defaultdict(list)
        self.unfiled, self.tables, self._mk = [], {}, {}
        #: Keys standing for another row: a merged single top node → its
        #: file row; a second chain to a file already shown → its row.
        self.alias = {}
        self._host(registry)
        placements = list(walk(scene))
        counts = Counter(g.uid for g, *_ in placements)
        self._qualified = {uid for uid, count in counts.items() if count > 1}

        files = {}
        for g, chain, inner, root in placements:
            if inner != chain:
                files.setdefault(tuple(s for s, _p in inner), inner)
        seen = list(files)                        # depth first: a file, then what it links
        shown = {}                                # normalised path → canonical stems
        for stems, inner in sorted(files.items(), key=lambda kv: (len(kv[0]), seen.index(kv[0]))):
            path = inner[-1][1]
            first = shown.get(path)
            if first is None:
                shown[path] = stems
                self._file(stems, path)
            else:
                self._same_file(stems, first)
        # Files after the host's roots, each right after the file placing it.
        rank = {stems: n for n, stems in enumerate(seen)}
        files_in_order = sorted((k for k, n in self.nodes.items() if n["kind"] == "link"),
                                key=lambda k: rank.get(self.nodes[k]["stems"], 0))
        for n, k in enumerate(files_in_order):
            self.nodes[k]["order"] = self._host_roots + 1 + n
        recipes = {k: n["recipe"] for k, n in self.nodes.items()}
        for g, chain, inner, root in placements:
            key = self._membership(g, chain, inner, root)
            if inner != chain:
                # A link placement is its file's row itself, not one of
                # the items counted or listed under it.
                self.member[id(g)] = key
                continue
            if not chain:
                if structure._is_unsorted(g, None, root, key if key in recipes else None, recipes):
                    self.unfiled.append(g)
            if key is not None:
                self.member[id(g)] = key
                self.direct[key].append(g)
        self.children = {None: []}
        for k in self.nodes:
            self.children.setdefault(k, [])
        for k, n in self.nodes.items():
            self.children.setdefault(n["parent"], []).append(k)
        for sib in self.children.values():
            sib.sort(key=lambda k: (self.nodes[k]["order"], self.nodes[k]["name"].casefold()))

    def address(self, group):
        """Keep legacy row UIDs when they identify a single occurrence."""
        if group.uid in self._qualified or group.uid.startswith("@["):
            return self.scene.occurrence_path(group)
        return group.uid

    def row_id(self, group):
        return row_id(self.address(group))

    # -- the host ----------------------------------------------------------
    def _host(self, registry):
        table = structure._nodes(registry)
        own = _own(table)
        for k, n in own.items():
            parent = n.get("parent") if n.get("parent") in own else None
            self.nodes[k] = dict(name=n["name"], parent=parent, order=n.get("order", 0), kind="host",
                                 path=None, stems=(), recipe=structure.resolve(registry, k), tag=None, folder=None)
        self._host_roots = max((n["order"] for n in self.nodes.values() if n["parent"] is None), default=-1)

    # -- linked files ------------------------------------------------------
    def resolve(self, key):
        """The row a display key stands for (aliases followed)."""
        seen = 0
        while key in self.alias and seen < 64:
            key, seen = self.alias[key], seen + 1
        return key

    def _same_file(self, stems, first):
        """A second chain to a file already shown: its placements and parts
        count on the first chain's row, and its folder joins the row's."""
        key, canon = mount_key(stems), mount_key(first)
        self.alias[key] = canon
        self.tables[stems] = self.tables.get(first, {})
        base = links.SEP.join(stems)
        if base not in self.nodes[canon]["folders"]:
            self.nodes[canon]["folders"].append(base)

    def _file(self, stems, path):
        key = mount_key(stems)
        base = links.SEP.join(stems)              # the file's tag folder in the host
        self.nodes[key] = dict(name=stems[-1], parent=None, order=len(self.nodes),
                               kind="link", path=path, stems=stems, recipe={}, tag=None, folder=base,
                               via=stems[-2] if len(stems) > 1 else None, folders=[base], merged=None)
        if len(stems) == 1:
            entry = links.registry(self.scene).get(path)
            table = (entry or {}).get("structure") or {}
        else:
            table = _subtable(self.tables.get(stems[:-1], {}), stems[-1])
        self.tables[stems] = table
        own = _own(table)
        pre = key + "/"
        tops = [k for k, n in own.items() if n.get("parent") not in own]
        single = tops[0] if len(tops) == 1 else None
        for k, n in own.items():
            recipe = dict(n.get("recipe") or n.get("source_recipe") or {})
            tag = recipe.get("tag")
            host_tag = base + links.SEP + tag if tag else None
            if host_tag is not None and self.scene.layer_by_key(host_tag) is None:
                host_tag = None
            if k == single:
                # One top node: it is the file («Ride» in Ride.igz). Merge
                # it into the row; its children and items hang from the row.
                self.alias[pre + k] = key
                self.nodes[key].update(merged=n.get("name") or k, recipe=recipe, tag=host_tag)
                continue
            parent = n.get("parent")
            parent = key if parent not in own or parent == single else pre + parent
            self.nodes[pre + k] = dict(name=n.get("name") or k, parent=parent, order=n.get("order", 0), kind="node",
                                       path=path, stems=stems, recipe=recipe, tag=host_tag, folder=None)
        if not own:
            self._tags(stems, path, key, base)

    def _tags(self, stems, path, key, base):
        """A file without a structure: its tag folders and tags as mirrored."""
        from core.layers import folder_label
        scene = self.scene
        parents = getattr(scene, "folder_parents", {}) or {}
        nested = base + links.SEP + LINKED_NAME
        folders, todo = {}, [base]
        while todo:
            f = todo.pop()
            for c in [c for c, p in parents.items() if p == f]:
                if c == nested or c in folders:
                    continue
                folders[c] = f
                todo.append(c)
        pre = key + "/"
        mine = [ly for ly in scene.layers
                if ly.key.startswith(base + links.SEP) and (ly.folder == base or ly.folder in folders)]
        tops = [f for f, parent in folders.items() if parent == base]
        # The file's «Untagged» tag (its loose geometry) does not count as a
        # second top: one top folder is still the file itself.
        untagged = base + links.SEP + "Untagged"
        single = tops[0] if len(tops) == 1 and not any(ly.folder == base and ly.key != untagged
                                                        for ly in mine) else None

        def under(f):
            return key if f in (base, single) else pre + "tf:" + f
        for n, (f, parent) in enumerate(folders.items()):
            label = folder_label(scene, f) or f
            label = label[len(base) + len(links.SEP):] if label.startswith(base + links.SEP) else label
            if f == single:
                self.alias[pre + "tf:" + f] = key
                self.nodes[key]["merged"] = label
                continue
            self.nodes[pre + "tf:" + f] = dict(name=label, parent=under(parent),
                                               order=n, kind="tagfolder", path=path, stems=stems, recipe={},
                                               tag=None, folder=f)
        for n, ly in enumerate(scene.layers):
            if not ly.key.startswith(base + links.SEP) or not (ly.folder == base or ly.folder in folders):
                continue
            parent = under(ly.folder)
            self.nodes[pre + "tag:" + ly.key] = dict(name=ly.name or ly.key, parent=parent, order=1000 + n,
                                                    kind="tag", path=path, stems=stems,
                                                    recipe={"tag": ly.key}, tag=ly.key, folder=None)

    # -- membership --------------------------------------------------------
    def _membership(self, g, chain, inner, root):
        if inner != chain:                         # a link placement: its file's row
            key = self._mk.get(inner)
            if key is None:
                key = self._mk[inner] = self.resolve(mount_key(tuple(s for s, _p in inner)))
            self.links[key].append(g)
            return key
        own = structure.node_of(g)
        if not chain:
            return own if own in self.nodes else None
        mk = self._mk.get(chain)
        if mk is None:
            mk = self._mk[chain] = self.resolve(mount_key(tuple(s for s, _p in chain)))
        if own is not None:
            k = self.resolve(mk + "/" + own)
            if k in self.nodes:
                return k
        layer = getattr(g, "layer", None)
        if root and layer and mk + "/tag:" + layer in self.nodes:
            return mk + "/tag:" + layer
        return mk if root else None

    # -- reads -------------------------------------------------------------
    def order(self):
        """``(key, depth)`` in display order."""
        out, stack = [], [(k, 0) for k in reversed(self.children[None])]
        while stack:
            k, d = stack.pop()
            out.append((k, d))
            stack.extend((c, d + 1) for c in reversed(self.children.get(k, ())))
        return out

    def subtree(self, key):
        out, stack = [], [key]
        while stack:
            k = stack.pop()
            out.append(k)
            stack.extend(self.children.get(k, ()))
        return out

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
    """A linked file's state for its row (one of ``STATES``). A file the
    host has an entry for reads ``links.link_state`` (as Problems and the
    activity banner do). A nested file has no entry of its own, and
    ``link_state`` would add one: it is missing when its file is, else as
    current as the file placing it was when read."""
    from pathlib import Path
    if not path:
        return "missing"
    entry = links.registry(scene).get(links._norm(path))
    if isinstance(entry, dict) and (not nested or "loaded_mtime_ns" in entry or entry.get("packed")):
        return links.link_state(scene, path)
    if not nested:
        return links.link_state(scene, path)
    return "resolved" if Path(path).is_file() else "missing"
