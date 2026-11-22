# SPDX-License-Identifier: GPL-3.0-or-later
"""The Project tree's nodes, linked files mounted under their link row
(ui/tree-links, 2026-11-22).

The host's registry keeps a *snapshot* of each linked file's structure,
taken when the host's structure was proposed (``structure.bootstrap``).
That snapshot goes stale when a source is reloaded, it puts nested links'
stems at the top level, and it misses the items of a link inside a link.
The tree therefore mounts every linked file from what is current:

* the host's own nodes (registry nodes without ``source``), in their order;
* then one «Linked files» root, holding one row per linked file placed in
  the host (``link:<stem>``);
* under a file's row, that file's own structure as the file has it (the
  host's ``plugin_data["links"][path]["structure"]``, refreshed on every
  reload), then its own «Linked files» (``link:<stem>/linked``) holding
  the files it links, recursively (``link:A/link:C``). A nested file's
  structure is the one its parent file mounted (``link:C/…`` nodes in the
  parent's structure);
* a file without a structure shows its tag folders and tags instead, as
  the host mirrors them («A / C / …»).

Host first, then Linked files: what you work on comes before what you only
reference (the order of Revit's Project Browser and of the Tags panel, where
«Linked files» is one folder among the host's).

Every placement belongs to at most one display node: the host's own by
their ``structure.node``; a linked file's placement to the file's row; a
part of a linked file to the node its own file filed it in, else to its
tag's node, else (a top part of the file only) to the file's row.

Keys are display keys, never written to the registry: ``link:…`` keys are
read-only (``structure.file`` refuses linked structure anyway).
"""
from __future__ import annotations

from collections import defaultdict

from core import links, structure

LINKED = "linked"          # the root «Linked files» row
LINKED_NAME = links.LINKED_FOLDER


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
    "linked", "link", "node", "tag", "tagfolder"), path, stems, recipe,
    tag (host tag key or None), folder (host folder key or None)};
    ``children``: key → ordered child keys (None: the roots);
    ``member``: id(group) → key; ``direct``: key → groups;
    ``links``: link-row key → the link placements it stands for;
    ``unfiled``: the host's own unsorted placements."""

    def __init__(self, scene, registry):
        self.scene = scene
        self.nodes, self.direct = {}, defaultdict(list)
        self.member, self.links = {}, defaultdict(list)
        self.unfiled, self.tables, self._mk = [], {}, {}
        self._host(registry)
        placements = list(walk(scene))
        files = {}
        for g, chain, inner, root in placements:
            if inner != chain:
                files.setdefault(tuple(s for s, _p in inner), inner)
        for stems, inner in sorted(files.items(), key=lambda kv: len(kv[0])):
            self._file(stems, inner[-1][1])
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
    def _linked_folder(self, stems):
        """The «Linked files» row holding ``stems``'s links."""
        key = LINKED if not stems else mount_key(stems) + "/" + LINKED
        if key not in self.nodes:
            parent = None if not stems else mount_key(stems)
            order = self._host_roots + 1 if not stems else 1 << 20
            self.nodes[key] = dict(name=LINKED_NAME, parent=parent, order=order, kind="linked", path=None,
                                   stems=tuple(stems), recipe={}, tag=None, folder=None)
        return key

    def _file(self, stems, path):
        key = mount_key(stems)
        base = links.SEP.join(stems)              # the file's tag folder in the host
        self.nodes[key] = dict(name=stems[-1], parent=self._linked_folder(stems[:-1]), order=len(self.nodes),
                               kind="link", path=path, stems=stems, recipe={}, tag=None, folder=base)
        if len(stems) == 1:
            entry = links.registry(self.scene).get(path)
            table = (entry or {}).get("structure") or {}
        else:
            table = _subtable(self.tables.get(stems[:-1], {}), stems[-1])
        self.tables[stems] = table
        own = _own(table)
        pre = key + "/"
        for k, n in own.items():
            recipe = dict(n.get("recipe") or n.get("source_recipe") or {})
            tag = recipe.get("tag")
            host_tag = base + links.SEP + tag if tag else None
            if host_tag is not None and self.scene.layer_by_key(host_tag) is None:
                host_tag = None
            parent = pre + n["parent"] if n.get("parent") in own else key
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
        for n, (f, parent) in enumerate(folders.items()):
            label = folder_label(scene, f) or f
            label = label[len(base) + len(links.SEP):] if label.startswith(base + links.SEP) else label
            self.nodes[pre + "tf:" + f] = dict(name=label, parent=key if parent == base else pre + "tf:" + parent,
                                               order=n, kind="tagfolder", path=path, stems=stems, recipe={},
                                               tag=None, folder=f)
        for n, ly in enumerate(scene.layers):
            if not ly.key.startswith(base + links.SEP) or not (ly.folder == base or ly.folder in folders):
                continue
            parent = key if ly.folder == base else pre + "tf:" + ly.folder
            self.nodes[pre + "tag:" + ly.key] = dict(name=ly.name or ly.key, parent=parent, order=1000 + n,
                                                    kind="tag", path=path, stems=stems,
                                                    recipe={"tag": ly.key}, tag=ly.key, folder=None)

    # -- membership --------------------------------------------------------
    def _membership(self, g, chain, inner, root):
        if inner != chain:                         # a link placement: its file's row
            key = self._mk.get(inner)
            if key is None:
                key = self._mk[inner] = mount_key(tuple(s for s, _p in inner))
            self.links[key].append(g)
            return key
        own = structure.node_of(g)
        if not chain:
            return own if own in self.nodes else None
        mk = self._mk.get(chain)
        if mk is None:
            mk = self._mk[chain] = mount_key(tuple(s for s, _p in chain))
        if own is not None and mk + "/" + own in self.nodes:
            return mk + "/" + own
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
        if node is None:
            return []
        if node["kind"] == "linked":
            bases = [self.nodes[c]["folder"] for c in self.children.get(key, ()) if self.nodes[c]["kind"] == "link"]
        elif node["kind"] == "link":
            bases = [node["folder"]]
        else:
            return []
        pre = tuple(b + links.SEP for b in bases)
        return [ly.key for ly in self.scene.layers if pre and ly.key.startswith(pre)]
