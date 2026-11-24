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

    def _tool_rotate_selection(self, center, axis, angles, **targets):
        from core.protocol.commands import run
        out = run(self._viewport.history, "rotate_selection", center=center, axis=axis,
                  angles=angles, **targets)
        self._viewport.update()
        return {**out, "step": out["step"].to_json() if out["step"] else None}

    def _tool_offset_outline(self, distance, **source):
        from core.protocol.commands import run
        out = run(self._viewport.history, "offset_outline", distance=distance, **source)
        self._viewport.update()
        return {**out, "step": out["step"].to_json() if out["step"] else None}

    def _tool_paint_material(self, placement=None, mesh=None, faces=(), seed=None, material=None,
                             color=None, texture=None, opacity=None, default=False, back=False, plane=None):
        from core.protocol.commands import run
        out = run(self._viewport.history, "paint_material", placement=placement, mesh=mesh, faces=faces,
                  seed=seed, material=material, color=color, texture=texture, opacity=opacity,
                  default=default, back=back, plane=plane)
        self._viewport.update()
        return {**out, "step": out["step"].to_json() if out["step"] else None}

    def _tool_set_group_axes(self, placement, origin, red, green):
        return self._edit_command("set_group_axes", placement=placement, origin=origin, red=red, green=green)

    def _tool_flip_selection(self, center, axis, copy=False, placements=(), faces=(), edges=()):
        return self._edit_command("flip_selection", center=center, axis=axis, copy=copy,
                                  placements=placements, faces=faces, edges=edges)

    def _tool_paste_clipboard(self, offset):
        return self._edit_command("paste_clipboard", offset=offset,
                                  clipboard=getattr(self._viewport, "clipboard", None))

    def _tool_bevel_selection(self, radius, segments=8, profile=.5, measure="radius", corners="round",
                              shape=None, edges=(), vertices=()):
        return self._edit_command("bevel_selection", radius=radius, segments=segments, profile=profile,
                                  measure=measure, corners=corners, shape=shape, edges=edges, vertices=vertices)

    def _edit_command(self, name, **args):
        from core.protocol.commands import run
        out = run(self._viewport.history, name, **args)
        self._viewport.update()
        return {**out, "step": out["step"].to_json() if out["step"] else None}

    def _tool_add_guide(self, point, direction=None, origin=None):
        return self._edit_command("add_guide", point=point, direction=direction, origin=origin)

    def _tool_change_guide(self, index, direction):
        return self._edit_command("change_guide", index=index, direction=direction)
