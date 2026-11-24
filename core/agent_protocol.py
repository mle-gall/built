# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The core protocol's feed and read queries for the agent bridge
(docs/core-protocol.md §5): an agent names faces, edges and vertices by
``(mesh key, element key)`` and follows what each step changed."""
from core.protocol import queries as Q


class ProtocolTools:
    def _tool_query_feed(self, since_revision=0):
        return Q.feed(self._viewport.history, since_revision)

    def _tool_query_mesh(self, mesh=None):
        return Q.mesh_info(self._viewport.scene, mesh)

    def _tool_query_faces(self, keys, mesh=None):
        return Q.faces(self._viewport.scene, keys, mesh)

    def _tool_query_edges(self, keys, mesh=None):
        return Q.edges(self._viewport.scene, keys, mesh)

    def _tool_query_vertices(self, keys, mesh=None):
        return Q.vertices(self._viewport.scene, keys, mesh)

    def _tool_query_selected_elements(self):
        return Q.selection(self._viewport.scene)

    def _tool_select_elements(self, faces=(), edges=(), vertices=(), mode="replace"):
        out = Q.select(self._viewport.scene, faces=faces, edges=edges,
                       vertices=vertices, mode=mode)
        self._viewport.update()
        return out

    def _tool_scale_selection(self, anchor, factors, vertices=(), placements=(), images=(), axes=None):
        from core.protocol.commands import run
        out = run(self._viewport.history, "scale_selection", anchor=anchor, factors=factors,
                  vertices=vertices, placements=placements, images=images, axes=axes)
        self._viewport.update()
        return {**out, "step": out["step"].to_json() if out["step"] else None}
