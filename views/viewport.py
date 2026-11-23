# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Legacy OpenGL surface. The shared document controller lives in document_interaction."""
from __future__ import annotations
from views.document_interaction import (
    DEFAULT_BACK_COLOR, DEFAULT_FACE_COLOR, DocumentInteraction, DocumentTools,
    EDIT_BOX_PAD_MAX_PX, EDIT_BOX_PAD_MIN_PX, EDIT_BOX_PAD_RATIO, EDIT_REST_FADE,
    EDIT_REST_MODES, Edge, ElementSet, EraseSelectionCommand, Face, Group, History,
    InferenceEngine, LINEWORK_SLACK, LINEWORK_SLACK_MIN, MONOCHROME_GLASS_RGB, Optional,
    OrbitCamera, Path, PlacementProxy, QColor, QEvent, QFont, QFontMetrics, QImage, QMatrix4x4,
    QObject, QPainter, QPen, QPointF, QPolygonF, QRectF, QTimer, QVector2D, QVector3D,
    QVector4D, Qt, SELECTION_DOT_ALT_COLOR, SELECTION_DOT_COLOR, SOFT_SHADOW_SPREAD, Scene,
    Signal, SnapResult, Tool, ToolContext, ViewNavigation, XRAY_FACE_OPACITY,
    XRAY_HIDDEN_EDGE_FADE, _AXIS_DIRS, _AXIS_VECTORS, _AltWatch, _BAKE_STORES, _BARY_EPS,
    _BY_LAYER_MODES, _DECODED_BUDGET, _DECODED_TEXTURES, _EDGES_SHARED, _FIRST_CLEAR,
    _FRAME_CACHE, _FRONT_LEAN, _HoverEvent, _INSTANCE_FILTER, _InstanceBatch, _LAZY_NATIVE,
    _LEAN_INSTANCED, _LIVE_VIEWS, _LOOSE_SNAP_CAP, _LazyChunk, _MIN_CIRCLE_DASHES,
    _NEAREST_NATIVE, _NEAR_SEEN_NATIVE, _NO_BACK_TINT, _NO_INSTANCING, _NO_SNAP_FAST,
    _NO_SPAN_GRID, _OCCLUSION_MEMO, _OCCL_NATIVE, _PERF, _PICK_WARM, _PickWarm, _QV_HYPOT3,
    _SHARED_BAKES, _SPAN_CELL_TRIS, _SPAN_GRID_MIN, _SPAN_VECTOR_MIN, _Slot, _SnapEdge,
    _VCOLOR_MODES, _ZEROS, _active_cut, _array_bake, _axes_vertices, _bake_store_of,
    _batch_instances, _bind_bake_store, _box_edge_vertices, _box_edges, _boxes_around,
    _cache_ver, _cat_kept, _chunk_uid, _circle_guide_dashes, _content_ver, _copy_snap_edge,
    _curved_faces, _decoded_texture, _drop_bake_store, _effective_layer_visible, _entity_owner,
    _expression_fields, _expression_result_text, _face_geometry, _finite_point, _flags_read,
    _fmt_len, _fp_winding, _front_tint, _ghost_edge_points, _group_pseudo_edge,
    _guide_vertices, _hypot3_f32, _inst_proto_of, _is_chord, _layer_look_sig, _layers,
    _lazy_edge_may_be_curve, _lean_instanced, _load_edit_rest_fade, _load_edit_rest_mode,
    _load_hide_similar, _load_invert_orbit_y, _load_invert_wheel, _load_msaa,
    _load_soft_shadows, _material_sig, _merge_mixed_numbers, _moller_trumbore, _nearest_within,
    _occlusion_block, _occlusion_native, _offset_kept, _paint_of, _parse_length_field,
    _parse_number, _perf_file, _pick_warm_take, _placed_maps, _plan_curved_edges, _plog,
    _point_in_screen_hull, _point_to_segment_distance_2d, _proto_wrapper, _pv_hole, _px_near,
    _qv_hypot3, _ray_aabb, _ray_aabb_span, _ray_triangle, _several_views_shown, _slab_far,
    _slab_near, _smooth_vcol, _snap_context_of, _snap_dropped_loose, _split_chunk_spans,
    _time_mod, _typed_value_text, _unit_rays, _with_front_tint, _zeros, array, copy,
    copy_group, edit_fade_color, effective_attrs, effective_back_color, effective_material,
    element_identity, face_uv_axes, importlib, inspect, is_expression, is_link_untagged,
    itertools, linework_slack_point, math, os, parse_units, plane_axes, qt_hypot, re,
    selection_dot_color, selection_mode, shown_guides, smooths, sys, tr, types, weakref,
    world_mesh
)
from PySide6.QtGui import QSurfaceFormat, QOpenGLFunctions
from views.gl_uniforms import ShaderProgram
from PySide6.QtOpenGL import (
    QOpenGLBuffer,
    QOpenGLFramebufferObject,
    QOpenGLFramebufferObjectFormat,
    QOpenGLShader,
    QOpenGLShaderProgram,
    QOpenGLTexture,
    QOpenGLVertexArrayObject,
)
from PySide6.QtOpenGLWidgets import QOpenGLWidget
# OpenGL constants — kept as literals so we don't depend on PyOpenGL.
GL_FLOAT = 0x1406
GL_LINES = 0x0001


def _shifted_mvp(mvp, dx_ndc: float, dy_ndc: float):
    """``mvp`` nudged by a clip-space offset (sub-pixel line thickening)."""
    from ingetrazo_core import Matrix4 as QMatrix4x4
    m = QMatrix4x4()
    m.translate(dx_ndc, dy_ndc, 0.0)
    return m * mvp
GL_TRIANGLES = 0x0004
GL_COLOR_BUFFER_BIT = 0x00004000
GL_DEPTH_BUFFER_BIT = 0x00000100
GL_DEPTH_TEST = 0x0B71
GL_CLIP_DISTANCE0 = 0x3000
GL_STENCIL_TEST = 0x0B90
GL_STENCIL_BUFFER_BIT = 0x00000400
GL_ALWAYS = 0x0207
GL_EQUAL = 0x0202
GL_KEEP = 0x1E00
GL_REPLACE = 0x1E01
#: Stencil bit the foliage cutouts leave, so the ambient occlusion (#954)
#: skips the leaves: a crown of crossed cards reads as one dark corner.
AO_FOLIAGE_BIT = 0x80
GL_BLEND = 0x0BE2
GL_SRC_ALPHA = 0x0302
GL_ONE_MINUS_SRC_ALPHA = 0x0303
GL_ONE = 1
GL_POLYGON_OFFSET_FILL = 0x8037
GL_CULL_FACE = 0x0B44
GL_CW = 0x0900
GL_CCW = 0x0901
GL_FRONT = 0x0404
GL_BACK = 0x0405
GL_LESS = 0x0201
GL_LEQUAL = 0x0203
GL_GREATER = 0x0204
GL_FALSE = 0
GL_TRUE = 1
GL_FRAMEBUFFER = 0x8D40
GL_READ_FRAMEBUFFER = 0x8CA8
GL_DRAW_FRAMEBUFFER = 0x8CA9
GL_NEAREST = 0x2600
GL_TEXTURE_2D = 0x0DE1
GL_TEXTURE0 = 0x84C0
GL_TEXTURE1 = 0x84C1
GL_MAX_TEXTURE_SIZE = 0x0D33


from core.paths import app_root

SHADER_DIR = app_root() / "resources" / "shaders"
_LEAN_NATIVE_REPLAY = True


class _GlTape:
    """A recorded run of GL calls: each one is made at once (through the
    PySide wrapper, as ever) and kept twice — as the wrapper call, and as
    the raw GL call that wrapper makes, for core._native's GlReplay. A
    replay runs the raw list natively when the engine is there, else the
    wrapper calls."""

    perf = None  # Current paint only; retained tapes never retain frame counters.

    DRAW, VAO, U1I, U1F, U4F, ENABLE, DISABLE, CULL, FRONT, ACTIVE, TEX, \
        STENCIL = range(12)

    def __init__(self, execute=True):
        self.calls: list = []
        self.ops: list = []
        self.vals: list = []
        self.arrays = None
        self.execute = execute
        self.draw_count = 0

    def _op(self, code, ints=(), floats=()):
        self.ops.append((code, *ints, *(0,) * (4 - len(ints))))
        self.vals.append((*floats, *(0.0,) * (4 - len(floats))))

    def call(self, fn, *args):
        self.calls.append((fn, args))
        if self.execute:
            start = _time_mod.perf_counter() if self.perf is not None else 0.0
            fn(*args)
            if self.perf is not None:
                self.perf["submit_ms"] += (_time_mod.perf_counter() - start) * 1000

    def draw(self, fn, mode, first, count, n):
        self.draw_count += 1
        if self.execute and self.perf is not None:
            self.perf["draws"] += 1
        self.call(fn, mode, first, count, n)
        self._op(self.DRAW, (mode, first, count, n))

    def bind_vao(self, vao):
        self.call(vao.bind)
        self._op(self.VAO, (vao.objectId(),))

    def release_vao(self, vao):
        self.call(vao.release)
        self._op(self.VAO, (0,))

    def uniform_i(self, prog, loc, v):
        self.call(prog.setUniformValue, loc, v)
        self._op(self.U1I, (loc, int(v)))

    def uniform_f(self, prog, loc, v):
        self.call(prog.setUniformValue1f, loc, v)
        self._op(self.U1F, (loc,), (float(v),))

    def uniform_4f(self, prog, loc, vec):
        self.call(prog.setUniformValue, loc, vec)
        self._op(self.U4F, (loc,), (vec.x(), vec.y(), vec.z(), vec.w()))

    def gl(self, fn, code, value):
        self.call(fn, value)
        self._op(code, (value,))

    def stencil_func(self, fn, func, ref, mask):
        self.call(fn, func, ref, mask)
        self._op(self.STENCIL, (func, ref, mask))

    def tex_bind(self, tex, unit):
        # QOpenGLTexture::bind(unit): active unit, then bind (not reset).
        self.call(tex.bind, unit)
        self._op(self.ACTIVE, (GL_TEXTURE0 + unit,))
        self._op(self.TEX, (tex.target().value, tex.textureId()))

    def tex_release(self, tex, unit):
        self.call(tex.release, unit)
        self._op(self.ACTIVE, (GL_TEXTURE0 + unit,))
        self._op(self.TEX, (tex.target().value, 0))

    def array_data(self):
        if self.arrays is None:
            import numpy as np
            self.arrays = (np.asarray(self.ops, dtype=np.int64).reshape(-1, 5),
                           np.asarray(self.vals, dtype=np.float32).reshape(-1, 4))
        return self.arrays

    @classmethod
    def submit_native(cls, native, ops, vals, draws):
        start = _time_mod.perf_counter() if cls.perf is not None else 0.0
        native.run(ops, vals)
        if cls.perf is not None:
            cls.perf["draws"] += draws
            cls.perf["submit_ms"] += (_time_mod.perf_counter() - start) * 1000

    def replay(self, native) -> None:
        if native is not None:
            self.submit_native(native, *self.array_data(), self.draw_count)
            return
        start = _time_mod.perf_counter() if self.perf is not None else 0.0
        for fn, args in self.calls:
            fn(*args)
        if self.perf is not None:
            self.perf["draws"] += self.draw_count
            self.perf["submit_ms"] += (_time_mod.perf_counter() - start) * 1000


def _callable_proc(addr: int) -> bool:
    """Whether a context's ``getProcAddress`` answer can be called: some
    Windows drivers' wglGetProcAddress hand back 1, 2, 3 or -1 for an entry
    they do not have, instead of null — none of which GlReplay may jump to
    (the tapes then replay through the PySide wrappers)."""
    return 0x10000 <= addr < 1 << 63


#: The GL entry points GlReplay calls, in _GlTape's opcode order.
_TAPE_FUNCS = (b"glDrawArraysInstanced", b"glBindVertexArray",
               b"glUniform1i", b"glUniform1f", b"glUniform4f", b"glEnable",
               b"glDisable", b"glCullFace", b"glFrontFace",
               b"glActiveTexture", b"glBindTexture", b"glStencilFunc")


def _scene_blend(gl) -> None:
    """The scene's blending: «over» for the colour and for the alpha. With
    the plain SRC_ALPHA, ONE_MINUS_SRC_ALPHA on both, a pane of 30 % left
    alpha 0.79 on an opaque background (0.3 * 0.3 + 0.7) and two of them
    0.64, so every exported picture came out with holes behind the glass."""
    gl.glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA,
                           GL_ONE, GL_ONE_MINUS_SRC_ALPHA)


#: The face modes Colour by layer applies to: the ones that draw the
#: per-vertex colour. Hidden line, monochrome and wireframe keep their own

class Viewport(DocumentInteraction, QOpenGLWidget):
    """Legacy Qt OpenGL drawing surface."""

    #: GL objects the frame creates on first use, outside initializeGL —
    #: dropped (not destroyed: their context is gone) when it re-runs.
    _LAZY_GL = ("_img_vao", "_img_vbo",
                "_pv_edges_vao", "_pv_edges_vbo", "_pv_vcol_vao",
                "_pv_vcol_vbo", "_pv_tex_vao", "_pv_tex_vbo",
                "_ghost_edges_vao", "_ghost_edges_vbo", "_ghost_vcol_vao",
                "_ghost_vcol_vbo", "_ghost_tex_vao", "_ghost_tex_vbo",
                "_depth_fbo", "_scene_fbo", "_shadow_fbo", "_depth_snap",
                "_fine_shadow_fbo",
                "_frame_cache_fbo", "_frame_cache")

    def _set_cutout(self, on) -> None:
        """The foliage cutout switch of the face shader, and with it the
        stencil mark the ambient occlusion skips (#954): the frame arms the
        stencil (test always passing, ``AO_FOLIAGE_BIT`` writable) and only
        the value a fragment leaves changes here."""
        self._program.setUniformValue(self._loc_hard_cutout, 1 if on else 0)
        self._gl.glStencilFunc(GL_ALWAYS, AO_FOLIAGE_BIT if on else 0, 0xFF)

    def _create_dynamic(self):
        vao = QOpenGLVertexArrayObject(self)
        vao.create()
        vao.bind()
        vbo = QOpenGLBuffer(QOpenGLBuffer.VertexBuffer)
        vbo.setUsagePattern(QOpenGLBuffer.DynamicDraw)
        vbo.create()
        vbo.bind()
        vbo.allocate(24)  # 2 vertices × 3 floats × 4 bytes
        self._program.bind()
        self._program.enableAttributeArray(self._loc_pos)
        self._program.setAttributeBuffer(self._loc_pos, GL_FLOAT, 0, 3)
        self._program.release()
        vbo.release()
        vao.release()
        return vao, vbo

    def _create_dynamic_uv(self):
        """A dynamic VAO/VBO interleaving position (3f) + UV (2f) per vertex —
        for textured faces."""
        vao = QOpenGLVertexArrayObject(self)
        vao.create()
        vao.bind()
        vbo = QOpenGLBuffer(QOpenGLBuffer.VertexBuffer)
        vbo.setUsagePattern(QOpenGLBuffer.DynamicDraw)
        vbo.create()
        vbo.bind()
        vbo.allocate(40)  # 2 vertices × 5 floats × 4 bytes
        stride = 5 * 4
        self._program.bind()
        self._program.enableAttributeArray(self._loc_pos)
        self._program.setAttributeBuffer(self._loc_pos, GL_FLOAT, 0, 3, stride)
        self._program.enableAttributeArray(self._loc_uv)
        self._program.setAttributeBuffer(self._loc_uv, GL_FLOAT, 3 * 4, 2, stride)
        self._program.release()
        vbo.release()
        vao.release()
        return vao, vbo

    def _create_dynamic_vcol(self):
        """A dynamic VAO/VBO interleaving position (3f) + RGB (3f) per vertex —
        the batched face pass (one draw call for every colour run)."""
        vao = QOpenGLVertexArrayObject(self)
        vao.create()
        vao.bind()
        vbo = QOpenGLBuffer(QOpenGLBuffer.VertexBuffer)
        vbo.setUsagePattern(QOpenGLBuffer.DynamicDraw)
        vbo.create()
        vbo.bind()
        vbo.allocate(48)  # 2 vertices × 6 floats × 4 bytes
        stride = 6 * 4
        self._program.bind()
        self._program.enableAttributeArray(self._loc_pos)
        self._program.setAttributeBuffer(self._loc_pos, GL_FLOAT, 0, 3, stride)
        self._program.enableAttributeArray(self._loc_vcolor)
        self._program.setAttributeBuffer(self._loc_vcolor, GL_FLOAT, 3 * 4, 3,
                                         stride)
        self._program.release()
        vbo.release()
        vao.release()
        return vao, vbo

    def _set_color(self, r: float, g: float, b: float, a: float) -> None:
        self._program.setUniformValue(self._loc_color, QVector4D(r, g, b, a))
        # Keep the back colour in sync by default so lines and highlights
        # (where gl_FrontFacing is meaningless) render one colour; the face
        # passes override it just before their draws.
        self._program.setUniformValue(self._loc_back_color,
                                      QVector4D(r, g, b, a))

    def _set_back_face_color(self) -> None:
        # The style's Back color wins; else the scene's adopted tint (from
        # an imported .skp's style, so unpainted faces read like they did
        # for the author); else the default blue-grey.
        r, g, b = effective_back_color(
            self._view_style(), self.scene)
        self._program.setUniformValue(self._loc_back_color,
                                      QVector4D(r, g, b, 1.0))

    def forget_uploads(self) -> None:
        """Free what this document has on the GPU, per prototype, for a
        context about to go; its bakes and pick index stay, in plain memory."""
        draws = getattr(self, "_proto_draw", None)
        hidden = getattr(self, "_hidden_edge_draws", None)
        if (draws or hidden) and self.context() is not None:
            self.makeCurrent()                # GPU objects need the context
            try:
                for entry in list((draws or {}).values()) + list(
                        (hidden or {}).values()):
                    self._destroy_proto_draw_entry(entry)
            finally:
                self.doneCurrent()
        if draws:
            draws.clear()
        if hidden:
            hidden.clear()
        self._proto_draw_fast = None
        self._frozen_cache_version = None
        self._slot_layout = None

    def reset_texture_cache(self) -> None:
        """Return the document's cached GL textures to the driver.

        ``_get_texture`` uploads lazily and remembers forever, which is right
        WITHIN a document — but nothing ever emptied it, so a long session of
        opening textured models only ever grew VRAM (the same class of leak
        CLAUDE.md measured at 0.42 GB for one survey's tile atlases).
        File ▸ New / Open call this at the document boundary: the old
        document's pictures go back to the driver, and whatever the new one
        actually shows re-uploads on demand at first paint."""
        self.reset_document_caches()
        cache = getattr(self, "_tex_cache", None)
        if not cache:
            return
        # Same contract as reset_tiles: destroying GL textures needs the
        # context current — this runs from menu handlers, not paintGL.
        self.makeCurrent()
        try:
            for tex in cache.values():
                if tex is not None:
                    tex.destroy()
        finally:
            self.doneCurrent()
        cache.clear()
        self.update()

    def _forget_context_objects(self) -> None:
        """Forget every lazily made GL object (see ``_LAZY_GL``) and the
        per-prototype draw entries (their VAOs and buffers): the next frame
        makes them again in the current context."""
        for name in self._LAZY_GL:
            if getattr(self, name, None) is not None:
                setattr(self, name, None)
        self._frame_cache_prev_mvp = None   # the kept picture is forgotten
        self._fbo_size = (0, 0)
        if getattr(self, "_proto_draw", None):
            self._proto_draw = {}
        self._forget_frame_records()

    def _forget_frame_records(self) -> None:
        """Drop the recorded instanced passes, the native replayer and the
        frame's batches. A tape holds the context's own function tables
        (``extraFunctions()``): kept past the context, PySide handed that
        stale wrapper back for a NEW object made at the same address — a
        later viewport's ``context()`` came back as a QOpenGLExtraFunctions
        and initializeGL failed (1 test run in ~10)."""
        for name in ("_lean_faces_rec", "_lean_edges_rec",
                     "_lean_faces_rec_chunks", "_lean_edges_rec_chunks",
                     "_lean_faces_rec_merged", "_lean_edges_rec_merged",
                     "_lean_faces_rec_fast", "_lean_edges_rec_fast",
                     "_inst_chunks_memo", "_inst_perf_counts",
                     "_gl_replay_memo", "_inst_frame_memo",
                     "_frame_instanced"):
            if getattr(self, name, None) is not None:
                setattr(self, name, None)

    def release_gl_textures(self) -> None:
        """Free every GL texture while the context can still take them back.

        Connected to the context's ``aboutToBeDestroyed`` — the one moment
        Qt guarantees for GL cleanup. Without it every texture alive at exit
        leaked with a "Texture has not been destroyed" warning each (harmless
        at process exit, real if the context is ever torn down and recreated
        mid-session, e.g. on reparenting)."""
        self._forget_frame_records()
        self.makeCurrent()
        try:
            cache = getattr(self, "_tex_cache", None)
            if cache:
                for tex in cache.values():
                    if tex is not None:
                        tex.destroy()
                cache.clear()
            tiles = getattr(self, "_tile_textures", None)
            if tiles:
                for tex in tiles.values():
                    if tex is not None:
                        tex.destroy()
                tiles.clear()
            terrain = getattr(self, "_terrain_texture", None)
            if terrain is not None:
                terrain.destroy()
                self._terrain_texture = None
            self._release_photo_textures_unsafe()
            if getattr(self, "_ao", None) is not None:
                self._ao.release()
                self._ao = None
        finally:
            self.doneCurrent()

    def _tile_texture(self, layer, x, y):
        """GL texture for tile ``(x, y)`` of ``layer``, or ``None`` if not yet
        available (a download is kicked off and the frame repaints on arrival)."""
        z = layer.zoom
        key = (layer.source.id, x, y, z)
        if key in self._tile_textures:
            return self._tile_textures[key]
        img = layer.images.get((x, y, z))
        if img is None:
            # A tile already on disk is read now — a file, no network — so a
            # sheet frame or an export, drawn once, still gets every cached
            # tile. Only a DOWNLOAD waits: QtNetwork must not start (the
            # fetcher builds its network manager on first use) inside
            # paintGL. On Windows/AMD that crashed in ntdll while the GL
            # driver callback was active (#327). The queued callback runs
            # when this paint has returned.
            img = self._disk_cached_tile(layer.source, x, y, z)
            if img is None:
                self._schedule_tile_requests()
                return None
            layer.images[(x, y, z)] = img
        if self._tex_budget <= 0:
            self._tex_deferred = True     # too many this frame — next frame
            return None
        self._tex_budget -= 1
        tex = QOpenGLTexture(img)  # QImage is top-down; our UVs map north→v=0
        tex.setWrapMode(QOpenGLTexture.ClampToEdge)
        tex.setMinificationFilter(QOpenGLTexture.LinearMipMapLinear)
        tex.setMagnificationFilter(QOpenGLTexture.Linear)
        self._tile_textures[key] = tex
        return tex

    def upload_terrain(self, terrain) -> None:
        """Build the terrain VBO (pos+uv) and its mosaic texture from a
        :class:`~georef.terrain.TerrainObject`."""
        if (terrain is None or not terrain.vertices or not terrain.triangles
                or self._program is None):        # no GL 3.3 to draw it
            self._terrain_count = 0
            return
        self.makeCurrent()
        try:
            raw = array("f")
            verts, uvs = terrain.vertices, terrain.uvs
            for (i, j, k) in terrain.triangles:
                for idx in (i, j, k):
                    v = verts[idx]
                    u, w = uvs[idx]
                    raw.extend([v.x(), v.y(), v.z(), u, w])
            data = raw.tobytes()
            self._terrain_vbo.bind()
            self._terrain_vbo.allocate(data, len(data))
            self._terrain_vbo.release()
            self._terrain_count = len(raw) // 5
            if self._terrain_texture is not None:
                self._terrain_texture.destroy()
                self._terrain_texture = None
            img = terrain.texture_image
            if img is not None and not img.isNull():
                self._terrain_texture = QOpenGLTexture(img)
                self._terrain_texture.setWrapMode(QOpenGLTexture.ClampToEdge)
                self._terrain_texture.setMinificationFilter(
                    QOpenGLTexture.LinearMipMapLinear)
                self._terrain_texture.setMagnificationFilter(QOpenGLTexture.Linear)
        finally:
            self.doneCurrent()
        self.update()

    def _render_terrain(self) -> None:
        if not self._terrain_showing() or self._terrain_count == 0:
            return
        self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
        self._gl.glPolygonOffset(1.0, 1.0)
        self._terrain_vao.bind()
        if self._terrain_texture is not None:
            self._program.setUniformValue(self._loc_use_tex, 1)
            self._terrain_texture.bind(0)
            self._gl.glDrawArrays(GL_TRIANGLES, 0, self._terrain_count)
            self._terrain_texture.release(0)
            self._program.setUniformValue(self._loc_use_tex, 0)
        else:
            self._set_color(0.55, 0.60, 0.52, 1.0)
            self._gl.glDrawArrays(GL_TRIANGLES, 0, self._terrain_count)
        self._terrain_vao.release()
        self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

    def _photo_showing(self) -> bool:
        m = getattr(self.scene, "photo_mesh", None)
        if m is None or not getattr(m, "visible", False) or self._photo_count == 0:
            return False
        # Its own switch AND its layer, the way a Group works: an entity can be
        # hidden individually or by the layer it carries.
        return self.scene.entity_visible(m)

    def max_texture_size(self) -> int:
        """The driver's ``GL_MAX_TEXTURE_SIZE`` — the hard ceiling for an atlas.

        Queried rather than assumed: ODM routinely emits 24576px sheets, which
        no current GPU accepts (this machine's Radeon 780M reports 16384), and
        an oversized upload fails outright instead of degrading.
        """
        if self._gl is None:
            return 4096                      # conservative until GL is up
        self.makeCurrent()
        try:
            value = self._gl.glGetIntegerv(GL_MAX_TEXTURE_SIZE)
        finally:
            self.doneCurrent()
        try:
            value = int(value[0] if hasattr(value, "__len__") else value)
        except (TypeError, ValueError):
            return 4096
        return value if value > 0 else 4096

    def upload_photo_mesh(self, mesh, images=None) -> None:
        """Upload a :class:`~georef.photomesh.PhotoMesh` and its atlases.

        ``images`` maps a material index → already-loaded ``QImage`` (the reader
        is slow enough that the caller does it off the GUI thread); anything
        missing simply draws untextured rather than blocking here.
        """
        if mesh is None or mesh.triangle_count == 0 or self._program is None:
            self.release_photo_textures()
            self._photo_count = 0
            self.update()
            return

        import numpy as np

        self.makeCurrent()
        try:
            # Inside makeCurrent: destroying a QOpenGLTexture without a current
            # context leaks it and prints "Texture has not been destroyed".
            self._release_photo_textures_unsafe()
            # Expand indexed triangles to the flat pos+uv layout the shared
            # VAO expects. Vectorised: the terrain's per-vertex Python loop is
            # fine for a 180x180 grid and hopeless for 362k triangles.
            tri = mesh.triangles
            pos = mesh.vertices[tri]                     # (M, 3, 3)
            uv = mesh.uvs[tri]                           # (M, 3, 2)
            data = np.concatenate([pos, uv], axis=2).astype(np.float32).tobytes()
            self._photo_vbo.bind()
            self._photo_vbo.allocate(data, len(data))
            self._photo_vbo.release()
            self._photo_count = int(tri.shape[0]) * 3

            # Materials are contiguous triangle runs, so each becomes one
            # glDrawArrays over its slice of the same buffer.
            images = images or {}
            for index, material in enumerate(mesh.materials):
                texture = None
                image = images.get(index)
                if image is not None and not image.isNull():
                    texture = QOpenGLTexture(image)
                    texture.setWrapMode(QOpenGLTexture.ClampToEdge)
                    texture.setMinificationFilter(QOpenGLTexture.LinearMipMapLinear)
                    texture.setMagnificationFilter(QOpenGLTexture.Linear)
                self._photo_textures.append(texture)
                self._photo_ranges.append(
                    (material.start * 3, material.count * 3, texture))
        finally:
            self.doneCurrent()
        self.update()

    def release_photo_textures(self) -> None:
        """Drop the survey's atlases (closing a document, or importing another).

        Half a gigabyte of VRAM is worth freeing eagerly rather than waiting for
        Qt to notice at teardown — when there is no current context left and the
        textures leak with a warning instead.
        """
        if not self._photo_textures:
            self._photo_ranges = []
            return
        self.makeCurrent()
        try:
            self._release_photo_textures_unsafe()
        finally:
            self.doneCurrent()

    def _release_photo_textures_unsafe(self) -> None:
        """Destroy the atlases. The caller must hold a current GL context."""
        for texture in self._photo_textures:
            if texture is not None:
                texture.destroy()
        self._photo_textures = []
        self._photo_ranges = []

    def _render_photo_mesh(self) -> None:
        if not self._photo_showing():
            return
        # Same polygon offset as the terrain: the survey is the ground, and
        # traced lines drawn on it must win the depth fight.
        self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
        self._gl.glPolygonOffset(1.0, 1.0)
        self._photo_vao.bind()
        for first, count, texture in self._photo_ranges:
            if count <= 0:
                continue
            if texture is not None:
                self._program.setUniformValue(self._loc_use_tex, 1)
                texture.bind(0)
                self._gl.glDrawArrays(GL_TRIANGLES, first, count)
                texture.release(0)
                self._program.setUniformValue(self._loc_use_tex, 0)
            else:
                # Untextured run (atlas missing or still loading): neutral clay
                # so it reads as "there is geometry here", not as a hole.
                self._set_color(0.62, 0.60, 0.56, 1.0)
                self._gl.glDrawArrays(GL_TRIANGLES, first, count)
        self._photo_vao.release()
        self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

    def _ensure_tile_geometry(self, layer, datum):
        """Build the base-map tile quad VBO **once** for the current capture
        patches (not per frame), returning ``[(x, y, vert_start), ...]``. The
        capture is static, so a strip of many tiles still draws fast — each
        frame just binds textures and draws slices; no per-tile re-allocation."""
        key = (id(datum), getattr(datum, "north", 0.0), tuple(layer.patches),
               layer.zoom, layer.source.id)
        cache = getattr(self, "_tile_geom", None)
        if cache is not None and cache[0] == key:
            return cache[1]
        raw = array("f")
        runs = []
        for (x, y) in layer.flat_tiles(datum):
            start = len(raw) // 5
            for pos, (u, v) in layer.quad_local(datum, x, y):
                raw.extend([pos.x(), pos.y(), pos.z(), u, v])
            runs.append((x, y, start))
        self._tile_quad_vbo.bind()
        self._tile_quad_vbo.allocate(raw.tobytes(), len(raw) * 4 or 4)
        self._tile_quad_vbo.release()
        self._tile_geom = (key, runs)
        return runs

    def _export_shadow_size(self) -> int:
        try:
            v = self._gl.glGetIntegerv(GL_MAX_TEXTURE_SIZE)
            cap = int(v[0] if hasattr(v, "__len__") else v)
        except (TypeError, ValueError, IndexError):
            cap = self._SHADOW_MAP_SIZE
        return max(self._SHADOW_MAP_SIZE, min(self._SHADOW_EXPORT_SIZE, cap))

    def _set_soft_shadow_uniforms(self) -> None:
        """Soft sun shadows (#1281) for the map this frame samples: the
        level's spread, and the map's footprint in the world read off the
        light matrix — its sides across, its depth range along the sun —
        so a detail map (#958) or an export's bigger one softens the same.
        Level 0 leaves the crisp map as it was."""
        spread = SOFT_SHADOW_SPREAD.get(getattr(self, "_soft_shadows", 0), 0.0)
        self._program.setUniformValue1f(self._loc_shadow_soft, spread)
        if spread:
            across, along = self._shadow_footprint(self._shadow_vp)
            self._program.setUniformValue(self._loc_shadow_world,
                                          QVector2D(across, along))

    def _draw_groups_preview(self, mvp) -> None:
        """Draw the frozen scratch copies of the moving groups at the
        current preview offset: one translated MVP, zero array churn."""
        if not getattr(self, "_preview_groups", None):
            return
        m = self._groups_preview_transform()
        self._program.setUniformValue(self._loc_mvp, mvp * m)
        n = getattr(self, "_pv_vcol_count", 0)
        if n:
            self._program.setUniformValue(self._loc_use_vcolor, 1)
            self._pv_vcol_vao.bind()
            self._gl.glDrawArrays(GL_TRIANGLES, 0, n)
            self._pv_vcol_vao.release()
            self._program.setUniformValue(self._loc_use_vcolor, 0)
        runs = getattr(self, "_pv_tex_runs", None)
        if runs:
            self._program.setUniformValue(self._loc_use_tex, 1)
            self._pv_tex_vao.bind()
            for key, start, count in runs:
                # Chunk texture runs key by (path, shade) tuples (bare path
                # when unshaded) — passing the tuple to QImage raised inside
                # paintGL and KILLED the whole preview frame ("moving shows
                # nothing until release", flatpak report).
                path, shade = key if isinstance(key, tuple) else (key, 1.0)
                tex = self._get_texture(path)
                if tex is None:
                    continue
                self._program.setUniformValue1f(self._loc_shade, float(shade))
                self._set_cutout(getattr(tex, "_cutout", False))
                tex.bind(0)
                self._gl.glDrawArrays(GL_TRIANGLES, start, count)
                tex.release(0)
            self._program.setUniformValue1f(self._loc_shade, 1.0)
            self._set_cutout(False)
            self._pv_tex_vao.release()
            self._program.setUniformValue(self._loc_use_tex, 0)
        n = getattr(self, "_pv_edges_count", 0)
        if n:
            self._set_color(1.0, 0.45, 0.0, 1.0)   # selection orange: moving
            self._pv_edges_vao.bind()
            self._gl.glDrawArrays(GL_LINES, 0, n)
            self._pv_edges_vao.release()
        self._program.setUniformValue(self._loc_mvp, mvp)

    def _set_section_clip(self, on: bool) -> None:
        """Enable/disable the active section cut around a model-geometry pass
        (``gl_ClipDistance`` in basic.vert). No-op when no cut is active."""
        if getattr(self, "_clip_vec", None) is None:
            return
        if on:
            self._program.setUniformValue(self._loc_clip_plane, self._clip_vec)
            self._program.setUniformValue(self._loc_clip_enable, 1)
            self._gl.glEnable(GL_CLIP_DISTANCE0)
        else:
            self._program.setUniformValue(self._loc_clip_enable, 0)
            self._gl.glDisable(GL_CLIP_DISTANCE0)

    def _mask_model_linetype_gaps(self) -> None:
        """Before the model's edges: the gaps of the dashed layers' edges
        laid in the depth buffer only, a hair toward the eye along each
        point's own sight line (so on the same pixels) — the edges drawn
        next fail the depth test there, and break into dashes."""
        try:
            groups = self._model_linetype_edges()
        except Exception:                    # never cost the frame
            return
        gaps = [g for _rgb, _d, g, _w in groups if len(g)]
        if not gaps:
            return
        import numpy as np
        eye = self.camera.eye()
        e = np.array([eye.x(), eye.y(), eye.z()], dtype=np.float32)
        pts = np.concatenate(gaps).reshape(-1, 3)
        pts = pts + (e - pts) * 0.003
        self._gl.glColorMask(False, False, False, False)
        self._draw_section_arrays(pts.reshape(-1, 2, 3), (0.0, 0.0, 0.0),
                                  GL_LINES)
        self._gl.glColorMask(True, True, True, True)

    def _draw_model_linetype_dashes(self) -> None:
        """After the model's edges: the dashes of a layer whose line type
        has a colour of its own drawn again in that colour — the same
        lines, so the depth test lets them through where the edges show."""
        try:
            groups = self._model_linetype_edges()
        except Exception:
            return
        ink = tuple(self._frame_style.edge_color[:3])
        for rgb, dashes, _gaps, words in groups:
            if rgb is not None and len(dashes):
                self._draw_section_arrays(dashes, rgb, GL_LINES)
            if len(words):
                # A hair toward the eye: on a floor, over its face.
                import numpy as np
                eye = self.camera.eye()
                e = np.array([eye.x(), eye.y(), eye.z()], dtype=np.float32)
                pts = words.reshape(-1, 3)
                pts = pts + (e - pts) * 0.002
                self._draw_section_arrays(pts.reshape(-1, 2, 3),
                                          rgb or ink, GL_LINES)

    def _draw_section_arrays(self, arr, rgb, mode) -> None:
        """One colour of the cut's fill: triangles or lines, world space."""
        raw = arr.reshape(-1).astype("float32").tobytes()
        self._section_vbo.bind()
        self._section_vbo.allocate(raw, len(raw))
        self._section_vbo.release()
        self._set_color(rgb[0], rgb[1], rgb[2], 1.0)
        self._program.setUniformValue(
            self._loc_back_color, QVector4D(rgb[0], rgb[1], rgb[2], 1.0))
        self._section_vao.bind()
        per = 3 if mode == GL_TRIANGLES else 2
        self._gl.glDrawArrays(mode, 0, len(arr) * per)
        self._section_vao.release()

    def _draw_layer_fills(self, fc) -> None:
        """«Layers' hatches»: each layer's cut in its ground, its hatch
        lines over it in its ink — the sheets' patterns, in the model."""
        fills = self._section_layer_fills(fc)
        if not fills:
            return
        self._set_section_clip(False)
        self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
        self._gl.glPolygonOffset(2.0, 2.0)
        for tris, rgb, _lines, _ink in fills:
            self._draw_section_arrays(tris, rgb, GL_TRIANGLES)
        self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
        for _tris, _rgb, lines, ink in fills:
            if lines is not None and len(lines):
                self._draw_section_arrays(lines, ink, GL_LINES)
        self._set_section_clip(True)

    def _annotations_need_depth(self) -> bool:
        if self._effective_style().face_mode in ("xray", "wireframe"):
            return False
        sc = self.scene
        return bool(getattr(sc, "dimensions", None)
                    or getattr(sc, "text_labels", None))

    def _read_scene_depth(self, w: int, h: int):
        """The frame's depth, copied out of the (multisampled) scene FBO
        into a single-sample one and read back: an ``(h, w)`` float array,
        or None when the driver refuses. Leaves the read framebuffer bound
        to that copy; the caller rebinds what it draws into."""
        try:
            import numpy as np
            from PySide6.QtOpenGL import (QOpenGLFramebufferObject,
                                          QOpenGLFramebufferObjectFormat)
            dfbo = getattr(self, "_depth_fbo", None)
            if dfbo is None or (dfbo.width(), dfbo.height()) != (w, h):
                fmt = QOpenGLFramebufferObjectFormat()
                fmt.setAttachment(QOpenGLFramebufferObject.CombinedDepthStencil)
                fmt.setSamples(0)
                dfbo = self._depth_fbo = QOpenGLFramebufferObject(w, h, fmt)
            extra = self.context().extraFunctions()
            self._gl.glBindFramebuffer(GL_READ_FRAMEBUFFER,
                                       self._scene_fbo.handle())
            self._gl.glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dfbo.handle())
            extra.glBlitFramebuffer(0, 0, w, h, 0, 0, w, h,
                                    GL_DEPTH_BUFFER_BIT, GL_NEAREST)
            self._gl.glBindFramebuffer(GL_READ_FRAMEBUFFER, dfbo.handle())
            buf = np.empty((h, w), dtype=np.float32)
            self._gl.glReadPixels(0, 0, w, h, 0x1902, GL_FLOAT, buf)  # DEPTH
        except Exception:  # noqa: BLE001
            return None
        return buf

    def _capture_depth(self, w: int, h: int) -> None:
        """Copy the frame's depth out of the (multisampled) scene FBO into a
        single-sample one and read it back once. The overlay then answers
        «is this point hidden?» with a lookup instead of a ray cast against
        the whole model: a house from a 2018 .skp with 25 dimensions spent
        2.8 s of every frame casting 1350 rays against 284 000 triangles
        (Juan José Noriega's files, 26-09-2026); the read-back is ~8 ms."""
        buf = self._read_scene_depth(w, h)
        if buf is None:             # fall back to rays, never break paint
            self._depth_snap = None
            return
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        inv, ok = mvp.inverted()
        if not ok:
            return
        self._depth_snap = (buf, mvp, inv, w, h, self.camera.eye(),
                            self.camera.forward())
        # The kept model it was read from, if nothing over it wrote depth.
        self._depth_snap_of = getattr(self, "_depth_clean", None)

    def _depth_world_at(self, x: float, y: float):
        """What the last frame shows under widget pixel ``(x, y)``, read from
        its depth: ``("hit", point)``, ``("background", None)`` or ``None``
        when there is no frame to read (the camera moved since, an export,
        no GL). The zoom and the orbit pivot ask this first: finding the
        point with a ray needs the pick index, and on a model of 21 406
        placements building it baked every one — minutes and 8 GB on the
        first wheel notch (issue #158). A read-back is ~8 ms, and it is
        what is on screen, hidden objects and cuts included."""
        painted = getattr(self, "_painted_view", None)
        fbo = getattr(self, "_scene_fbo", None)
        if painted is None or fbo is None or self.context() is None:
            return None
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        if tuple(mvp.data()) != painted[0]:
            return None
        w, h = painted[1], painted[2]
        snap = getattr(self, "_depth_snap", None)
        if snap is None or tuple(snap[1].data()) != painted[0]:
            self.makeCurrent()
            try:
                self._capture_depth(w, h)
            finally:
                self.doneCurrent()
            snap = getattr(self, "_depth_snap", None)
            if snap is None:
                return None
        buf, _mvp, inv = snap[0], snap[1], snap[2]
        if float(buf.min()) >= float(buf.max()):
            return None                  # a blank read-back: this GL gives no depth
        dpr = w / max(1, self.width())
        ix = int(x * dpr)
        iy = int(h - 1 - y * dpr)
        if not (0 <= ix < w and 0 <= iy < h):
            return None
        d = float(buf[iy, ix])
        if d >= 1.0:
            return ("background", None)
        nx = (ix + 0.5) / w * 2.0 - 1.0
        ny = (iy + 0.5) / h * 2.0 - 1.0
        q = inv.map(QVector4D(nx, ny, d * 2.0 - 1.0, 1.0))
        if abs(q.w()) < 1e-12:
            return None
        point = QVector3D(q.x() / q.w(), q.y() / q.w(), q.z() / q.w())
        # Zoomed right in, the near plane is a fraction of a millimetre while
        # the far one is kilometres away: the depth buffer has no precision
        # left, and the read-back unprojects to a point far behind the model
        # — or to infinity. The orbit then turned about that point and threw
        # the camera off until its target was NaN and the model would not
        # save (#485, #383). Such a point is no answer: the caller falls
        # back to the ray.
        if not all(math.isfinite(c) for c in (point.x(), point.y(), point.z())):
            return None
        if (point - self.camera.eye()).length() > 1.01 * self.camera.far_plane():
            return None
        return ("hit", point)

    def reset_document_caches(self, keep_bakes=False):
        draws = dict(getattr(self, "_proto_draw", None) or {})
        draws.update({("hidden", k): v for k, v in
                      getattr(self, "_hidden_edge_draws", {}).items()})
        if draws and self.context() is not None:
            self.makeCurrent()                # GPU objects need the context
            try:
                for entry in draws.values():
                    self._destroy_proto_draw_entry(entry)
            finally:
                self.doneCurrent()
        super().reset_document_caches(keep_bakes)

    def reset_tiles(self):
        if self._tile_textures:
            # Destroying GL textures needs the context current — this runs from
            # the Tray, not paintGL.
            self.makeCurrent()
            try:
                for tex in self._tile_textures.values():
                    if tex is not None:
                        tex.destroy()
            finally:
                self.doneCurrent()
        self._tile_textures.clear()
        self._tile_geom = None       # capture patches / datum may have changed
        super().reset_tiles()

    def begin_groups_preview(self, groups=(), external=False):
        super().begin_groups_preview(groups, external)
        if not self._preview_groups or self._program is None:
            return
        self.makeCurrent()
        try:
            if getattr(self, "_pv_edges_vao", None) is None:
                self._pv_edges_vao, self._pv_edges_vbo = self._create_dynamic()
                self._pv_vcol_vao, self._pv_vcol_vbo = \
                    self._create_dynamic_vcol()
                self._pv_tex_vao, self._pv_tex_vbo = self._create_dynamic_uv()
            edge_parts: list = []
            vcol_parts: list = []
            tex_parts: dict = {}
            for g in groups:
                if getattr(g, "billboard", False):
                    # Face-me figures preview through their OWN pass (the
                    # anchor rides the preview transform): their raw mesh
                    # quad in the scratch VBOs showed a swimming texture.
                    continue
                ch = self._preview_chunk(g)
                if ch["edges"]:
                    edge_parts.append(ch["edges"])
                if ch["vcol"]:
                    vcol_parts.append(ch["vcol"])
                for path, raw in ch["by_texture"].items():
                    tex_parts.setdefault(path, []).append(raw)
            raw = b"".join(edge_parts)
            self._pv_edges_vbo.bind()
            self._pv_edges_vbo.allocate(raw or b"\0" * 24, max(len(raw), 24))
            self._pv_edges_vbo.release()
            self._pv_edges_count = len(raw) // 12
            raw = b"".join(vcol_parts)
            self._pv_vcol_vbo.bind()
            self._pv_vcol_vbo.allocate(raw or b"\0" * 24, max(len(raw), 24))
            self._pv_vcol_vbo.release()
            self._pv_vcol_count = len(raw) // 24
            runs: list = []
            blobs: list = []
            off = 0
            for path, parts in tex_parts.items():
                blob = b"".join(parts)
                runs.append((path, off // 20, len(blob) // 20))
                blobs.append(blob)
                off += len(blob)
            raw = b"".join(blobs)
            self._pv_tex_vbo.bind()
            self._pv_tex_vbo.allocate(raw or b"\0" * 24, max(len(raw), 24))
            self._pv_tex_vbo.release()
            self._pv_tex_runs = runs
        finally:
            self.doneCurrent()

    def __init__(self, parent=None):
        super().__init__(parent)
        self._tile_textures = {}
        self._gl: Optional[QOpenGLFunctions] = None
        self._program: Optional[QOpenGLShaderProgram] = None
        self._loc_mvp = -1
        self._loc_color = -1
        self._loc_pos = -1

        self._axes_vao = None
        self._axes_vbo = None
        self._guides_vbo = None

        self._edges_vao = None
        self._edges_vbo = None
        # Default-back tint: the triangles of every face whose back is
        # unpainted, drawn once more with front culling in the style's back
        # colour (see ``core.materials.back_is_default``).
        self._dback_vao = None
        self._dback_vbo = None
        self._dback_count = 0
        self._dback_spans: list = []
        self._edges_count = 0
        self._selected_vao = None
        self._selected_vbo = None
        self._selected_count = 0
        self._sel_faces_vao = None
        self._sel_faces_vbo = None
        self._sel_faces_count = 0
        # [(front_rgb, back_rgb, first_vertex, count)] — one run per dot
        # colour pair inside the selected-faces buffer.
        self._sel_faces_spans = []
        self._faces_vao = None
        self._faces_vbo = None
        self._faces_count = 0
        # Solid-colour faces share ONE interleaved pos+rgb VBO drawn in a
        # single call (the shaded material colour rides per vertex).
        # Textured faces (pos+uv VBO) grouped by image path: [(path, start, count)].
        self._tex_faces_count = 0
        self._tex_opaque_count = 0
        self._tex_runs: list = []
        self._back_vcol_run: tuple = (0, 0)
        self._back_tex_runs: list = []
        self._back_tcol_runs: list = []
        self._back_ttex_runs: list = []
        self._fvcol_run: tuple = (0, 0)
        self._tcol_runs: list = []
        self._ttex_runs: list = []
        self._tex_cache: dict = {}
        self._edges_version = -1
        #: Per consolidated VBO: the byte parts last uploaded and the buffer's
        #: allocated capacity, so a resync re-sends only what changed.
        self._vbo_parts: dict = {}

        self._hover_faces_vao = None
        self._hover_faces_vbo = None
        self._hover_edges_vao = None
        self._hover_edges_vbo = None

        self._rubber_vao = None
        self._rubber_vbo = None

        # Shaded solid preview (Push/Pull): the forming box's faces, uploaded
        # per paint while the tool drags.
        self._preview_faces_vao = None
        self._preview_faces_vbo = None
        self._preview_tex_vao = None
        self._preview_tex_vbo = None
        self._section_vao = None
        self._section_vbo = None

        # Offscreen FBO with depth attachment. QOpenGLWidget's default target
        # on some Mesa/Wayland stacks has no depth buffer, which silently
        # breaks hidden-line removal. Rendering into our own FBO and blitting
        # color out guarantees a real depth buffer is present.
        self._scene_fbo: Optional[QOpenGLFramebufferObject] = None
        self._tile_quad_vao = None
        self._tile_quad_vbo = None
        # Cached base-map tile geometry (built once per capture, not per frame).
        self._tile_geom = None
        # Per-frame GL-texture-creation budget (spreads big captures over frames).
        self._tex_budget = 0
        self._tex_deferred = False

        self._terrain_vao = None
        self._terrain_vbo = None
        self._terrain_count = 0
        self._terrain_texture = None
        # Photogrammetric survey (Track G, G6). One VBO for the whole mesh and
        # one texture per ODM atlas; ``_photo_ranges`` says which slice of the
        # VBO each atlas covers, so the draw is ~20 calls, not one per triangle.
        self._photo_vao = None
        self._photo_vbo = None
        self._photo_count = 0
        self._photo_textures = []
        self._photo_ranges = []

    def _configure_surface(self):
        # Hidden-line removal needs a real depth buffer. setDefaultFormat() in
        # main.py is best-effort; many platforms ignore it for QOpenGLWidget
        # and hand us a 0-bit depth context. Forcing the format here is the
        # only reliable way.
        #
        # Start from the default format rather than from nothing, though:
        # core.gl_fallback may have just dropped the stencil buffer to get a
        # context AT ALL on this driver, and building a fresh format here
        # would ask for it right back and undo the recovery. Depth and
        # samples we still state outright — those two are ours to decide.
        fmt = QSurfaceFormat(QSurfaceFormat.defaultFormat())
        fmt.setVersion(3, 3)
        fmt.setProfile(QSurfaceFormat.CoreProfile)
        fmt.setDepthBufferSize(24)
        fmt.setSamples(0)
        # NO samples on the widget surface: the scene renders into our own
        # multisampled FBO (_ensure_scene_fbo) and the blit resolves it. A
        # multisampled widget FBO adds a second resolve that interleaves stale
        # frames on Wayland (ghost frames during fast zoom) and cannot smooth
        # the already-resolved pixels we blit into it.
        self.setFormat(fmt)

    # ---- GL lifecycle -------------------------------------------------------
    def initializeGL(self) -> None:
        # initializeGL re-runs when Qt recreates the context — the widget
        # moved to another window, which is what MultiView does to put the
        # viewport in its grid. Everything below is rebuilt; what the frame
        # makes lazily must be forgotten too, or it is bound in a context it
        # does not belong to (segfault in _draw_image_planes, 10-10).
        self._forget_context_objects()
        self._gl = QOpenGLFunctions(self.context())
        self._gl.initializeOpenGLFunctions()
        # Who is drawing: the GPU driver or a software rasteriser. Recorded
        # for Help ▸ About, written to the log folder, and announced to the
        # main window so a CPU-drawn viewport says so before anyone wonders
        # why orbiting crawls (a Windows box without a working driver).
        from core.glinfo import meets_gl_requirement, read_gl_info, write_gl_report
        self.gl_info = read_gl_info(self._gl)
        self.gl_info["supported"] = meets_gl_requirement(self.gl_info["version"])
        from PySide6.QtCore import QCoreApplication
        from PySide6.QtGui import QGuiApplication
        self.gl_info["platform"] = QGuiApplication.platformName()
        _app = QCoreApplication.instance()
        if _app is not None:
            self.gl_info["fallback"] = _app.property("gl_fallback") or ""
        write_gl_report(self.gl_info)
        self.glReady.emit(self.gl_info)
        if not self.gl_info["supported"]:
            # Below GL 3.3 the shaders cannot compile (issue #751). Make no
            # GL object on this context, so none is freed on it at exit
            # either; paintGL only clears, and the main window explains.
            self._program = None              # a previous context's, if any
            self._gl.glClearColor(0.93, 0.94, 0.96, 1.0)
            return
        # GL cleanup has exactly one legal moment: the context's own farewell.
        # (initializeGL re-runs if the context is ever recreated; each context
        # gets its own connection, which is precisely what we want.)
        self.context().aboutToBeDestroyed.connect(self.release_gl_textures)
        self._gl.glClearColor(0.93, 0.94, 0.96, 1.0)
        self._gl.glClearDepthf(1.0)
        self._gl.glEnable(GL_DEPTH_TEST)
        # LEQUAL (instead of the default LESS) lets a fragment win when its
        # depth equals the existing one — important for edges drawn on top of
        # coincident faces, which can rasterize to bit-identical depths.
        self._gl.glDepthFunc(GL_LEQUAL)
        self._gl.glEnable(GL_BLEND)
        _scene_blend(self._gl)

        self._program = self._compile_program()
        self._loc_mvp = self._program.uniformLocation("u_mvp")
        self._loc_color = self._program.uniformLocation("u_color")
        self._loc_back_color = self._program.uniformLocation("u_back_color")
        self._loc_fade = self._program.uniformLocation("u_fade")
        self._loc_fade_color = self._program.uniformLocation("u_fade_color")
        self._loc_pos = self._program.attributeLocation("a_pos")
        self._loc_uv = self._program.attributeLocation("a_uv")
        self._loc_use_tex = self._program.uniformLocation("u_use_texture")
        self._loc_tex = self._program.uniformLocation("u_tex")
        self._loc_vcolor = self._program.attributeLocation("a_color")
        self._loc_use_vcolor = self._program.uniformLocation("u_use_vcolor")
        self._loc_opacity = self._program.uniformLocation("u_opacity")
        self._loc_hard_cutout = self._program.uniformLocation("u_hard_cutout")
        self._loc_shade = self._program.uniformLocation("u_shade")
        # Instanced pass: re-shade the prototype's baked tones for the
        # rotation of each copy (#482), against the bake's own light.
        self._loc_reshade = self._program.uniformLocation("u_reshade")
        self._loc_back_shade = self._program.uniformLocation("u_back_shade")
        self._program.bind()
        self._program.setUniformValue(self._loc_reshade, 0)
        self._program.setUniformValue(
            self._program.uniformLocation("u_light_fixed"), self._LIGHT)
        self._program.release()
        # Per-instance model matrix columns (P2 instanced components).
        self._loc_inst = [self._program.attributeLocation(f"a_inst{i}")
                          for i in range(4)]
        self._loc_clip_plane = self._program.uniformLocation("u_clip_plane")
        self._loc_clip_enable = self._program.uniformLocation("u_clip_enable")
        # Sun shadows (core/sun.py): sampling uniforms on the main program,
        # plus the tiny depth-only program the shadow map renders with.
        self._loc_shadow_enable = self._program.uniformLocation(
            "u_shadow_enable")
        self._loc_shadow_map = self._program.uniformLocation("u_shadow_map")
        self._loc_light_vp = self._program.uniformLocation("u_light_vp")
        self._loc_shadow_dark = self._program.uniformLocation("u_shadow_dark")
        self._loc_shadow_bias = self._program.uniformLocation("u_shadow_bias")
        self._loc_shadow_soft = self._program.uniformLocation("u_shadow_soft")
        self._loc_shadow_world = self._program.uniformLocation(
            "u_shadow_world")
        self._loc_sun_dir = self._program.uniformLocation("u_sun_dir")
        self._loc_shadow_overlay = self._program.uniformLocation(
            "u_shadow_overlay")
        self._loc_stipple = self._program.uniformLocation("u_stipple")
        self._loc_viewport_px = self._program.uniformLocation("u_viewport_px")
        self._loc_dash_px = self._program.uniformLocation("u_dash_px")
        self._depth_program = self._compile_depth_program()
        self._loc_d_mvp = self._depth_program.uniformLocation("u_mvp")
        self._loc_d_clip_plane = self._depth_program.uniformLocation(
            "u_clip_plane")
        self._loc_d_clip_enable = self._depth_program.uniformLocation(
            "u_clip_enable")
        self._loc_d_use_tex = self._depth_program.uniformLocation(
            "u_use_texture")
        self._loc_d_tex = self._depth_program.uniformLocation("u_tex")
        self._shadow_fbo = None
        self._shadow_key = None
        self._shadow_vp = None
        self._fine_shadow_fbo = None
        self._fine_shadow_key = None
        self._shadow_fit = None

        # Axes rebuilt per frame (dash spacing scales with zoom), so dynamic.
        self._axes_vao, self._axes_vbo = self._create_dynamic()
        self._guides_vao, self._guides_vbo = self._create_dynamic()
        self._guides_cache = None
        self._axes_spans: dict = {}

        self._sky_vao, self._sky_vbo = self._create_dynamic()
        self._sky_grad_vao, self._sky_grad_vbo = self._create_dynamic_color()
        self._ground_vao, self._ground_vbo = self._create_dynamic()
        self._shadow_bb_vao, self._shadow_bb_vbo = self._create_dynamic_uv()
        self._edges_vao, self._edges_vbo = self._create_dynamic()
        self._dback_vao, self._dback_vbo = self._create_dynamic()
        self._selected_vao, self._selected_vbo = self._create_dynamic()
        self._sel_faces_vao, self._sel_faces_vbo = self._create_dynamic()
        self._faces_vao, self._faces_vbo = self._create_dynamic_vcol()
        self._tex_faces_vao, self._tex_faces_vbo = self._create_dynamic_uv()
        self._billboard_vao, self._billboard_vbo = self._create_dynamic_uv()
        self._bb_sel_vao, self._bb_sel_vbo = self._create_dynamic()
        self._hover_faces_vao, self._hover_faces_vbo = self._create_dynamic()
        self._hover_edges_vao, self._hover_edges_vbo = self._create_dynamic()
        self._silhouette_vao, self._silhouette_vbo = self._create_dynamic()
        self._rubber_vao, self._rubber_vbo = self._create_dynamic()
        self._preview_faces_vao, self._preview_faces_vbo = self._create_dynamic()
        self._preview_tex_vao, self._preview_tex_vbo = self._create_dynamic_uv()
        self._section_vao, self._section_vbo = self._create_dynamic()
        self._tile_quad_vao, self._tile_quad_vbo = self._create_dynamic_uv()
        self._terrain_vao, self._terrain_vbo = self._create_dynamic_uv()
        self._photo_vao, self._photo_vbo = self._create_dynamic_uv()

    def resizeGL(self, w: int, h: int) -> None:
        # Qt passes framebuffer-pixel sizes here (already scaled by DPR), so
        # this is the authoritative source for FBO and viewport dimensions.
        if self._gl is None or self._program is None:
            return
        self._gl.glViewport(0, 0, w, h)
        self.camera.set_aspect(w, h)
        self._ensure_scene_fbo(w, h)

    def _ensure_scene_fbo(self, w: int, h: int) -> None:
        """Create or resize the offscreen FBO used for depth-tested rendering."""
        size = (max(w, 1), max(h, 1))
        if self._scene_fbo is not None and self._fbo_size == size:
            return
        fmt = QOpenGLFramebufferObjectFormat()
        fmt.setAttachment(QOpenGLFramebufferObject.CombinedDepthStencil)
        # Real MSAA happens HERE, where the scene actually renders; the blit
        # to the widget's single-sample FBO is the resolve (MSAA read → plain
        # draw is the legal direction). The widget surface itself stays
        # single-sample — see __init__. Sample count from Preferences
        # (Graphics); changing it just voids _fbo_size.
        fmt.setSamples(getattr(self, "_msaa", 4))
        self._scene_fbo = QOpenGLFramebufferObject(size[0], size[1], fmt)
        if fmt.samples() and self._scene_fbo.format().samples() == 0:
            # Driver refused multisampling — retry single-sample but KEEP the
            # depth/stencil attachment (the Wayland depth workaround).
            fmt.setSamples(0)
            self._scene_fbo = QOpenGLFramebufferObject(size[0], size[1], fmt)
        self._fbo_size = size

    def render_image(self, width_px: int, height_px: Optional[int] = None,
                     overlays: bool = True,
                     annotations_only: bool = False,
                     sharp_shadows: bool = False,
                     transparent: bool = False,
                     crop_to_frame: bool = False) -> Optional["QImage"]:
        """Render the current view at ``width_px`` wide (height follows the
        viewport's aspect) and return it as a ``QImage`` — the hi-res 2D
        export. Reuses the exact ``paintGL`` pipeline against a temporary
        FBO, then paints the presentation overlays (dimensions, geo paths,
        survey points, guides) on top with a scaled QPainter; interactive
        artifacts (snap marker, hover, rubber band) are naturally absent.

        The sheet composer passes an explicit ``height_px`` (its frames have
        their own aspect — the caller is responsible for pointing the camera
        and setting its aspect to match) and ``overlays=False``: the overlay
        painters are calibrated to the live widget's aspect and would land
        misplaced under a composer camera.

        ``sharp_shadows`` (Export image, sheets): the sun's map is drawn
        for this one picture at ``_SHADOW_EXPORT_SIZE`` instead of the view's
        2048 — a plaza of 100 m gets 1.2 cm per texel instead of 5 cm — and
        freed after it, so the view keeps its own map and its memory.

        ``transparent`` (Export image): no sky, no ground and no background
        colour — alpha 0 wherever the model is not, glass and the shadows on
        the ground as translucent as they are.

        ``crop_to_frame`` (Export image): with a camera frame set (Camera
        Settings ▸ Frame) the picture is what lies inside it, ``width_px``
        wide — the view is rendered that much wider and cut to the frame,
        so the export is exactly the shot framed on screen."""
        if self._gl is None or self._program is None:
            return None
        lw = max(self.width(), 1)
        lh = max(self.height(), 1)
        if crop_to_frame and height_px is None:
            return self._render_framed_image(width_px, overlays, annotations_only,
                                             sharp_shadows, transparent)
        width_px = max(int(width_px), 64)
        if height_px is None:
            height_px = max(int(round(width_px * lh / lw)), 64)
        else:
            height_px = max(int(height_px), 64)

        prev_fbo = self._scene_fbo
        prev_size = self._fbo_size
        view_shadow = None
        self.makeCurrent()
        from PySide6.QtGui import QOpenGLContext
        if QOpenGLContext.currentContext() is None:
            # The widget's context is not up yet (asked for a picture before
            # its first frame): an FBO made with no context current crashes.
            return None
        try:
            self._export_size = (width_px, height_px)
            self._export_transparent = transparent
            if sharp_shadows and self._sun_dir_now() is not None:
                view_shadow = (self._shadow_fbo, self._shadow_key)
                self._shadow_fbo = self._shadow_key = None
                self._shadow_size = self._export_shadow_size()
            try:
                self.paintGL()
                image = self._scene_fbo.toImage()   # resolves MSAA itself
            finally:
                if view_shadow is not None:
                    # Freed here, with the context current; the view's own
                    # map was never touched and is reused as it was.
                    self._shadow_fbo = None
                    self._shadow_fbo, self._shadow_key = view_shadow
                    self._shadow_size = self._SHADOW_MAP_SIZE
                self._export_size = None
                self._export_transparent = False
                # Drop the big export FBO (destroyed here, context current)
                # and put the on-screen one back.
                self._scene_fbo = prev_fbo
                self._fbo_size = prev_size
        finally:
            self.doneCurrent()

        if overlays:
            painter = QPainter(image)
            painter.setRenderHint(QPainter.Antialiasing, True)
            painter.setRenderHint(QPainter.TextAntialiasing, True)
            # Overlay drawing works in logical widget pixels
            # (_world_to_pixel); the scale maps it onto the hi-res image,
            # thickening lines and text proportionally.
            painter.scale(width_px / lw, height_px / lh)
            if not annotations_only:
                # A sheet frame wants the model's annotations (the usual
                # convention for sheets: dimensions and texts) but never the guides or
                # the section-plane frames.
                self._draw_guides(painter)
                self._draw_section_planes(painter)
                self._draw_geo_surfaces(painter)
                self._draw_geo_paths(painter)
                self._draw_geo_points(painter)
            self._draw_dimensions(painter)
            self._draw_text_labels(painter)
            painter.end()
        self.update()                               # repaint the widget
        return image


    def paintGL(self) -> None:
        if getattr(self, "_pick_warm_due", False):
            # After this first frame of a new document, not beside it.
            self._pick_warm_due = False
            from PySide6.QtCore import QTimer
            QTimer.singleShot(0, self, self.start_pick_warmup)
        Viewport.painting += 1
        try:
            if getattr(self, "tool_preview", True):
                Viewport._paint_frame(self)
                return
            tool, self.active_tool = self.active_tool, None
            try:
                Viewport._paint_frame(self)
            finally:
                self.active_tool = tool
        finally:
            Viewport.painting -= 1

    def _paint_frame(self) -> None:
        self._tick = getattr(self, "_tick", 0) + 1     # a new epoch memo
        self._depth_clean = None          # see the read-back at the end
        self._sync_axes()
        self._sync_projection()
        if self._program is None:
            if self._gl is not None:          # a context below GL 3.3
                self._gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
            return
        self._sync_depth_reach()
        if _PERF:
            self._inst_perf = dict(draws=0, submit_ms=0.0, gather_ms=0.0,
                                   prototypes=0, placements=0, frustum=0, pixel=0,
                                   batches=0, pass_ms=0.0, faces_draws=0, edges_draws=0)
            _GlTape.perf = self._inst_perf
        _pt0 = _time_mod.perf_counter() if _PERF else 0.0

        # Render the 3D scene into our own FBO (which has a real depth buffer)
        # then blit the colour to the widget's default framebuffer. Sizes are
        # in framebuffer pixels — using logical (self.width/height) here would
        # blit into a fraction of the widget on HiDPI displays and shift the
        # rendered scene away from the mouse cursor.
        w, h = self._export_size or self._fb_size()
        self._ensure_scene_fbo(w, h)
        default_fbo = self.defaultFramebufferObject()
        self._scene_fbo.bind()
        self._gl.glViewport(0, 0, w, h)

        # Re-establish GL state every frame. QPainter (used for the 2D overlay)
        # leaves GL state in an undefined shape — in particular it tends to
        # disable depth test — so we can't trust state to persist across
        # paintGL calls.
        self._gl.glEnable(GL_DEPTH_TEST)
        self._gl.glDepthFunc(GL_LEQUAL)
        self._gl.glDepthMask(GL_TRUE)
        self._gl.glEnable(GL_BLEND)
        _scene_blend(self._gl)
        self._gl.glDisable(GL_CULL_FACE)

        # Effective display style (Styles): the composer's
        # plano_style override maps onto the same face modes; otherwise the
        # scene's active style drives faces, edges and background.
        style = self._effective_style()
        mode = style.face_mode
        self._frame_style = style

        # Active section cut: ONE world-space plane; the kept
        # side is where dot(n, p - origin) <= 0. Applies to model geometry
        # only — sky, axes, terrain, previews and overlays stay uncut.
        self._clip_vec = None
        sp = (self.scene.active_section()
              if getattr(self.scene, "show_section_cuts", True) else None)
        if sp is not None:
            n, o = sp.normal, sp.point
            self._clip_vec = QVector4D(
                -n.x(), -n.y(), -n.z(), QVector3D.dotProduct(n, o))

        self._gl.glClearDepthf(1.0)
        bg = style.background
        if self._export_transparent:
            # Black, not the background's colour: the picture is
            # premultiplied, and an edge's antialiased pixels would otherwise
            # carry that colour out as a halo.
            self._gl.glClearColor(0.0, 0.0, 0.0, 0.0)
        else:
            self._gl.glClearColor(bg[0], bg[1], bg[2], 1.0)
        self._gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT
                         | GL_STENCIL_BUFFER_BIT)
        # Foliage marking for the ambient occlusion: every opaque fragment
        # that wins the depth test leaves its bit — set by a cutout, cleared
        # by anything else (``_set_cutout``) — so the bit ends up saying
        # whether the surface SEEN there is a leaf. The test always passes:
        # colour and depth come out as ever. Off again at the occlusion.
        self._gl.glEnable(GL_STENCIL_TEST)
        self._gl.glStencilFunc(GL_ALWAYS, 0, 0xFF)
        self._gl.glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE)
        self._gl.glStencilMask(AO_FOLIAGE_BIT)

        self._fit_scene_depth()
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        old_detail = getattr(self, "_nav_fast_feedback", False)
        self._nav_fast_feedback = self._navigation_detail(mvp, mode)
        if old_detail and not self._nav_fast_feedback:
            # The settling frame must derive the current outline, not reuse
            # the expensive-profile throttle's pre-navigation segments.
            self._sil_last = None
            self._sil_cam_t = 0.0
        # Frustum planes for chunk culling — also consumed by the
        # silhouette pass (skip whole chunks that are off screen).
        self._frame_planes = self._frustum_planes(mvp)
        # P0 frame telemetry: coarse per-section CPU times, logged with the
        # cull counters when the frame is slow (INGETRAZO_PERF=1).
        _fseg: dict = {}
        _fprev = _pt0

        def _fmark(name: str) -> None:
            nonlocal _fprev
            if _PERF:
                now = _time_mod.perf_counter()
                _fseg[name] = _fseg.get(name, 0.0) + (now - _fprev) * 1000.0
                _fprev = now

        self._program.bind()
        self._program.setUniformValue(self._loc_mvp, mvp)
        # Instanced-matrix attributes default to IDENTITY for every
        # non-instanced draw (the GL default (0,0,0,1) collapses geometry,
        # and QPainter may clobber generic attribute state — reset per frame).
        li = getattr(self, "_loc_inst", None)
        if li and li[0] >= 0:
            self._gl.glVertexAttrib4f(li[0], 1.0, 0.0, 0.0, 0.0)
            self._gl.glVertexAttrib4f(li[1], 0.0, 1.0, 0.0, 0.0)
            self._gl.glVertexAttrib4f(li[2], 0.0, 0.0, 1.0, 0.0)
            self._gl.glVertexAttrib4f(li[3], 0.0, 0.0, 0.0, 1.0)
        # Solid-colour by default; the textured-face pass flips this on.
        self._program.setUniformValue(self._loc_use_tex, 0)
        self._program.setUniformValue(self._loc_stipple, 0)
        self._program.setUniformValue(self._loc_tex, 0)  # sampler → unit 0
        self._program.setUniformValue(self._loc_use_vcolor, 0)
        self._program.setUniformValue1f(self._loc_opacity, 1.0)
        self._program.setUniformValue1f(self._loc_shade, 1.0)
        self._program.setUniformValue(self._loc_hard_cutout, 0)
        self._program.setUniformValue1f(self._loc_fade, 0.0)

        # Sky / ground backdrop with a horizon anchored to the camera pitch —
        # premium modeller feel. Fixed on zoom (it's the point at infinity),
        # moves only on orbit. Skipped over the base map / terrain (which supply
        # their own ground).
        if (self.plano_style is None and style.sky
                and not self._export_transparent
                and not self._base_map_showing()
                and not self._terrain_showing()
                and not self._photo_showing()):
            self._draw_sky(mvp)
            self._program.setUniformValue(self._loc_mvp, mvp)

        # Base-map tiles (Track G) — the ground image, drawn before the grid so
        # the grid lines read on top of the imagery. Depth-write OFF: it's a
        # backdrop, geometry always draws over it.
        self._render_tiles()

        # 3D draped terrain (Track G, G2 full) — real depth-tested relief that
        # replaces the flat map when enabled.
        self._render_terrain()

        # Photogrammetric survey (Track G, G6) — the user's own flight, drawn
        # after the DEM terrain so the real capture wins where both exist.
        self._render_photo_mesh()

        # Imported reference images (a scanned plan, a facade photo). Last of
        # the reference layers and before any model geometry: they are what
        # the user draws ON TOP of.
        if self.scene.edit_group is not None:
            # One colour for every faded pass (images, loose, grouped,
            # instanced, face-me figures); set before the first of them.
            self._program.setUniformValue(
                self._loc_fade_color, QVector3D(*edit_fade_color(
                    style.background)))
        self._draw_image_planes()
        _fmark("ground")

        # No grid — the infinite axes are the spatial reference.

        # Persistent edges + faces
        self._sync_edges()
        _fmark("sync")

        # Sun shadow map — AFTER the sync, so the consolidated VBOs hold
        # THIS frame's geometry; cached across frames by geometry+sun (it is
        # camera-independent: orbit and zoom reuse it). The pass borrows the
        # FBO/program bindings; this block restores them and binds the map
        # on texture unit 1 for the geometry passes to sample.
        self._frame_shadow = (self._ensure_shadow_map(mvp)
                              if mode in ("textures", "shaded",
                                          "hidden_line", "monochrome")
                              else None)
        if self._frame_shadow is not None:
            self._scene_fbo.bind()
            self._gl.glViewport(0, 0, w, h)
            self._program.bind()
            self._gl.glActiveTexture(GL_TEXTURE1)
            self._gl.glBindTexture(GL_TEXTURE_2D,
                                   self._frame_shadow.texture())
            self._gl.glActiveTexture(GL_TEXTURE0)
            self._program.setUniformValue(self._loc_shadow_map, 1)
            self._program.setUniformValue(self._loc_light_vp,
                                          self._shadow_vp)
            sh = getattr(self.scene, "shadows", None)
            self._program.setUniformValue1f(
                self._loc_shadow_dark,
                float(getattr(sh, "darkness", 0.55)))
            # Small constant margin on the receiver; the heavy lifting is
            # the slope-scaled bias the CASTER writes (depth.frag) — a
            # constant big enough to silence grazing faces peeled every
            # shadow off its base instead.
            self._program.setUniformValue1f(self._loc_shadow_bias, 0.0006)
            self._set_soft_shadow_uniforms()
            sun = getattr(self, "_frame_sun", None) or (0.0, 0.0, 1.0)
            self._program.setUniformValue(
                self._loc_sun_dir, QVector3D(sun[0], sun[1], sun[2]))
        self._program.setUniformValue(self._loc_shadow_enable, 0)
        self._program.setUniformValue(self._loc_shadow_overlay, 0)
        _fmark("shadowmap")

        fc = self._frame_cache_key(mvp, w, h, style, mode)
        settled = self._frame_cache_track(
            fc[0] if fc is not None else tuple(mvp.data()))
        if fc is None:
            self._paint_static(mvp, w, h, style, mode, _fmark,
                               self._hover_entity)
        else:
            # The model as the last frame left it, when nothing it shows
            # changed (a pointer move): see _frame_cache_key.
            if not self._frame_cache_restore(fc, w, h):
                self._paint_static(mvp, w, h, style, mode, _fmark, None)
                # Keep the picture only once the view has settled: while
                # the camera moves, every frame's key is new, so the copy
                # (~0.8 ms of an orbit frame, issue #944) was stored and
                # never restored. The frame after the last camera move
                # stores instead; a change that is not the camera (a
                # selection, an edit) keeps the same MVP and stores at
                # once, as before.
                if settled:
                    self._frame_cache_store(fc, w, h)
            self._paint_hover(mvp, w, h)
        # The view at rest: measure what it shows for the fitted sun map
        # (#958) — read from the depth the model just left, before the
        # rubber band and the overlay draw over it.
        self._settle_shadow_fit(mvp, w, h)

        # Rubber band preview. Loose drawing tools float it on top (depth test
        # off, so it never z-fights with coincident axes). Push/Pull's solid
        # preview keeps depth testing on, so the forming box's back edges are
        # hidden behind its faces — classic hidden-line removal.
        depth_wire = (
            getattr(self.active_tool, "wireframe_depth_tested", False)
            if self.active_tool is not None
            else False
        )
        if not depth_wire:
            self._gl.glDisable(GL_DEPTH_TEST)
        self._draw_rubber_band()
        if not depth_wire:
            self._gl.glEnable(GL_DEPTH_TEST)

        self._program.release()

        if self._export_size is not None:
            # Hi-res export: the scene stays in the FBO for render_image to
            # read back; the widget blit and its QPainter overlay don't apply.
            self._scene_fbo.release()
            _GlTape.perf = None
            return

        # The depth this frame just drew, read ONCE for the overlay's
        # annotations (dimensions, leader texts), which ask «is this point
        # hidden?» for every sample of every line — see _occluded_on_screen.
        # A pointer move over the kept model (_frame_cache_key) leaves the
        # depth as it was when nothing over it writes depth — the face's
        # dots do not, the rubber band floats with no depth test, the axes
        # redrawn over the dots fail GL_LESS where they are — and the
        # read-back already made of it is kept: over the plant of #158 it
        # was ~5 ms of every pointer move, reading the same depth again.
        clean = (fc is not None and not depth_wire
                 and not isinstance(getattr(self, "_hover_entity", None),
                                    Edge))
        kept = (clean and getattr(self, "_depth_snap", None) is not None
                and getattr(self, "_depth_snap_of", None) == fc)
        self._depth_clean = fc if clean else None
        if not kept:
            self._depth_snap = None
            if self._annotations_need_depth():
                self._capture_depth(w, h)
        # What the scene FBO now holds: the depth under the cursor can be
        # read from it while the camera stays here (``_depth_world_at``).
        self._painted_view = (tuple(mvp.data()), w, h)

        # Blit colour from our scene FBO to the widget's default framebuffer.
        # We can't use QOpenGLFramebufferObject.blitFramebuffer(None, src) here
        # because in QOpenGLWidget the "default" framebuffer the widget shows
        # is its own internal FBO (returned by defaultFramebufferObject()),
        # NOT the system framebuffer 0. So we bind the read/draw targets by id
        # and call glBlitFramebuffer directly via the GL3+ extra functions.
        extra = self.context().extraFunctions()
        self._gl.glBindFramebuffer(GL_READ_FRAMEBUFFER, self._scene_fbo.handle())
        self._gl.glBindFramebuffer(GL_DRAW_FRAMEBUFFER, default_fbo)
        extra.glBlitFramebuffer(
            0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST
        )
        self._gl.glBindFramebuffer(GL_FRAMEBUFFER, default_fbo)
        if _PERF:
            _plog("paintGL.gl+blit",
                  (_time_mod.perf_counter() - _pt0) * 1000.0, floor=30.0)
            _ov0 = _time_mod.perf_counter()

        # 2D overlays on top of the OpenGL framebuffer.
        self._draw_overlay()

        if _PERF:
            _plog("paintGL.overlay",
                  (_time_mod.perf_counter() - _ov0) * 1000.0, floor=30.0)
            _dt = (_time_mod.perf_counter() - _pt0) * 1000.0
            _plog("paintGL", _dt)
            # P0 breakdown: per-section CPU ms + cull counters + the
            # input→paint latency of the gesture that triggered this frame.
            if _dt >= 25.0:
                cf, tf, ce = getattr(self, "_cull_stats", (0, 0, 0))
                it = getattr(self, "_input_t", None)
                lat = (f" lat={( _time_mod.monotonic() - it) * 1000.0:.0f}ms"
                       if it is not None else "")
                segs = " ".join(f"{k}={v:.0f}ms" for k, v in _fseg.items())
                _plog("frame", _dt,
                      extra=f"{segs} cull={cf//1000}k/{tf//1000}k"
                            f" tris +{ce//1000}k edges{lat}"
                            + self._inst_perf_summary(), floor=0.0)
            self._input_t = None
            st = getattr(self, "_perf_stat", None) or \
                [_time_mod.perf_counter(), 0, 0.0]
            st[1] += 1
            st[2] += _dt
            now = _time_mod.perf_counter()
            if now - st[0] >= 1.0:
                _plog("frames/s", st[2] / max(st[1], 1),
                      extra=f"{st[1]} paints en {now-st[0]:.1f}s (avg ms); last"
                            + self._inst_perf_summary(),
                      floor=0.0)
                st = [now, 0, 0.0]
            self._perf_stat = st
            _GlTape.perf = None

    def _paint_static(self, mvp, w, h, style, mode, _fmark, hover) -> None:
        """paintGL's passes of the model and what sits in it — faces,
        edges, selection, axes, guides — up to the section cut's edges;
        ``hover`` is the entity under the cursor they highlight (None:
        none, for the frame cache, which draws it after)."""
        # Frustum culling (P1): the consolidated VBOs carry per-chunk draw
        # spans; only spans whose chunk AABB touches the frustum are
        # submitted. Conservative — a missing bbox always draws.
        planes = self._frame_planes
        # Inside a group with the context faded, the buffers hold the
        # surroundings first and the edited group last; the split keeps the
        # two apart so each is drawn with its own opacity.
        # Both splits or neither: a frame that faded the context's edges but
        # not its faces would read as a glitch rather than a mode.
        fading = (self.scene.edit_group is not None
                  and self._edit_rest_mode == "fade"
                  and getattr(self, "_edit_split_e", None) is not None
                  and getattr(self, "_edit_split_f", None) is not None)
        split_f = self._edit_split_f if fading else None
        split_e = self._edit_split_e if fading else None
        spans = getattr(self, "_face_spans", None)
        if spans:
            face_spans, culled_fv = self._visible_spans(spans, planes, split_f)
        else:
            face_spans, culled_fv = [(0, self._faces_count)], 0
        espans = getattr(self, "_edge_spans", None)
        if espans:
            edge_spans, culled_ev = self._visible_spans(espans, planes, split_e)
        else:
            edge_spans, culled_ev = [(0, self._edges_count)], 0
        self._frame_edge_spans = edge_spans
        self._cull_stats = (culled_fv // 3, self._faces_count // 3,
                            culled_ev // 2)

        # Faces — drawn before edges, with polygon offset so coincident
        # boundary edges sit cleanly on top instead of z-fighting.
        self._set_section_clip(True)
        if getattr(self, "_frame_shadow", None) is not None:
            self._draw_shadow_ground(style)
            self._program.setUniformValue(self._loc_shadow_enable, 1)
        if self._faces_count > 0 and mode != "wireframe":
            self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
            self._gl.glPolygonOffset(1.0, 1.0)
            if mode == "hidden_line":
                # Plan style: every face flat in the style's front colour, no
                # rebatch needed — per-vertex colours simply not consulted.
                self._program.setUniformValue(self._loc_use_vcolor, 0)
                fr = style.front_color
                self._set_color(fr[0], fr[1], fr[2], 1.0)
                self._program.setUniformValue(
                    self._loc_back_color, QVector4D(fr[0], fr[1], fr[2], 1.0))
            elif mode == "monochrome":
                # Flat default front colour + the back tint — the classic
                # reversed-face checker (Monochrome style).
                self._program.setUniformValue(self._loc_use_vcolor, 0)
                fr = style.front_color
                self._set_color(fr[0], fr[1], fr[2], 1.0)
                self._set_back_face_color()
            else:
                # Every colour run in ONE draw call: the shaded material
                # colour rides per vertex (a_color). An imported model with
                # hundreds of colour×shade runs paid ~2000 uniform/draw
                # calls per frame.
                self._program.setUniformValue(self._loc_use_vcolor, 1)
                self._set_back_face_color()
            if mode == "xray":
                # X-ray: translucent faces that do not occlude — edges stay
                # fully visible because nothing writes depth.
                self._program.setUniformValue1f(self._loc_opacity,
                                                XRAY_FACE_OPACITY)
                self._gl.glDepthMask(GL_FALSE)
            self._faces_vao.bind()
            if split_f is None:
                for _vs, _vc in face_spans:
                    self._gl.glDrawArrays(GL_TRIANGLES, _vs, _vc)
            else:
                # Context first, washed toward the background but fully
                # opaque and still writing depth (the faded rest of
                # model reads as haze, not as glass); then the group itself
                # at full strength.
                self._program.setUniformValue1f(self._loc_fade,
                                                self.edit_rest_fade)
                for _vs, _vc in face_spans:
                    if _vs < split_f:
                        self._gl.glDrawArrays(GL_TRIANGLES, _vs, _vc)
                self._program.setUniformValue1f(self._loc_fade, 0.0)
                for _vs, _vc in face_spans:
                    if _vs >= split_f:
                        self._gl.glDrawArrays(GL_TRIANGLES, _vs, _vc)
            fstart, fcount = self._fvcol_run
            if fcount:
                # opaque front copies of glass-backed faces: visible only
                # from the front (the translucent back pass owns the rear)
                self._gl.glEnable(GL_CULL_FACE)
                self._gl.glCullFace(GL_BACK)
                self._gl.glDrawArrays(GL_TRIANGLES, fstart, fcount)
                self._gl.glDisable(GL_CULL_FACE)
            self._faces_vao.release()
            if mode == "xray":
                self._program.setUniformValue1f(self._loc_opacity, 1.0)
                self._gl.glDepthMask(GL_TRUE)
            self._program.setUniformValue(self._loc_use_vcolor, 0)
            self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

        # Textured faces — same depth/offset treatment, sampling each face's
        # image. One draw per texture (its GL texture bound to unit 0).
        if self._tex_faces_count > 0 and mode in ("hidden_line", "monochrome"):
            # Plan styles: textured faces draw flat like the rest.
            self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
            self._gl.glPolygonOffset(1.0, 1.0)
            fr = style.front_color
            self._set_color(fr[0], fr[1], fr[2], 1.0)
            self._tex_faces_vao.bind()
            if mode == "monochrome":
                # Not its translucent runs: they draw with the glass below,
                # or a textured pane came out as a white wall.
                self._gl.glDrawArrays(GL_TRIANGLES, 0, self._tex_opaque_count)
                for _key, start, count in self._back_tex_runs:
                    self._gl.glDrawArrays(GL_TRIANGLES, start, count)
            else:
                self._gl.glDrawArrays(GL_TRIANGLES, 0, self._tex_faces_count)
            self._tex_faces_vao.release()
            self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
        elif self._tex_faces_count > 0 and mode == "shaded":
            # "Shaded" style: textured faces draw in their texture's
            # AVERAGE colour (the material colour), keeping the run shading.
            self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
            self._gl.glPolygonOffset(1.0, 1.0)
            self._tex_faces_vao.bind()
            run_parts = getattr(self, "_tex_run_parts", None)
            for ri, ((path, shade), start, count) in enumerate(self._tex_runs):
                if not run_parts:
                    vis, subj = [(start, count)], []
                else:
                    vis, subj = self._tex_run_spans(run_parts[ri], planes,
                                                    fading)
                if not vis and not subj:
                    continue
                r, g, b = self._texture_avg_color(path)
                self._program.setUniformValue1f(self._loc_shade, float(shade))
                self._set_color(r, g, b, 1.0)
                for fade, spans in ((self.edit_rest_fade, vis),
                                    (0.0, subj)) \
                        if fading else ((0.0, vis),):
                    if not spans:
                        continue
                    self._program.setUniformValue1f(self._loc_fade, fade)
                    for _vs, _vc in spans:
                        self._gl.glDrawArrays(GL_TRIANGLES, _vs, _vc)
            self._program.setUniformValue1f(self._loc_fade, 0.0)
            self._tex_faces_vao.release()
            self._program.setUniformValue1f(self._loc_shade, 1.0)
            self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
        elif self._tex_faces_count > 0 and mode in ("textures", "xray"):
            self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
            self._gl.glPolygonOffset(1.0, 1.0)
            self._program.setUniformValue(self._loc_use_tex, 1)
            if mode == "xray":
                self._program.setUniformValue1f(self._loc_opacity,
                                                XRAY_FACE_OPACITY)
                self._gl.glDepthMask(GL_FALSE)
            self._tex_faces_vao.bind()
            run_parts = getattr(self, "_tex_run_parts", None)
            for ri, ((path, shade), start, count) in enumerate(self._tex_runs):
                if not run_parts:
                    vis, subj = [(start, count)], []
                else:
                    vis, subj = self._tex_run_spans(run_parts[ri], planes,
                                                    fading)
                if not vis and not subj:
                    continue
                tex = self._get_texture(path)
                if tex is None:
                    continue
                self._program.setUniformValue1f(self._loc_shade, float(shade))
                self._set_cutout(getattr(tex, "_cutout", False))
                tex.bind(0)
                for fade, spans in ((self.edit_rest_fade, vis),
                                    (0.0, subj)) \
                        if fading else ((0.0, vis),):
                    if not spans:
                        continue
                    self._program.setUniformValue1f(self._loc_fade, fade)
                    for _vs, _vc in spans:
                        self._gl.glDrawArrays(GL_TRIANGLES, _vs, _vc)
                tex.release(0)
            self._program.setUniformValue1f(self._loc_fade, 0.0)
            self._set_cutout(False)
            self._tex_faces_vao.release()
            if mode == "xray":
                self._program.setUniformValue1f(self._loc_opacity, 1.0)
                self._gl.glDepthMask(GL_TRUE)
            self._program.setUniformValue1f(self._loc_shade, 1.0)
            self._program.setUniformValue(self._loc_use_tex, 0)
            self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

        # The default back: a painted face shows its material on
        # the side you painted and the style's blue-grey on the other, so
        # the reverse of a wall reads as the reverse. Every face whose back
        # is unpainted — and whose front is opaque; glass, water and a mesh
        # read the same from both sides — is drawn once more here with
        # FRONT culling in the back colour: only its rear fragments arrive,
        # at the depth the face pass already wrote, and LEQUAL lets this
        # later draw win. Plan styles have their own rule (monochrome tints
        # every back, hidden-line none), X-ray keeps the blend clean.
        if (self._dback_count > 0 and mode in ("textures", "shaded")
                and not _NO_BACK_TINT):
            db_spans = getattr(self, "_dback_spans", None)
            split_db = (getattr(self, "_edit_split_db", None)
                        if fading else None)
            if db_spans:
                db_draw, _c = self._visible_spans(db_spans, planes, split_db)
            else:
                db_draw = [(0, self._dback_count)]
            self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
            self._gl.glPolygonOffset(1.0, 1.0)
            self._gl.glEnable(GL_CULL_FACE)
            self._gl.glCullFace(GL_FRONT)
            self._program.setUniformValue(self._loc_use_vcolor, 0)
            self._program.setUniformValue(self._loc_use_tex, 0)
            self._set_back_face_color()
            self._program.setUniformValue(self._loc_back_shade, 1)
            self._dback_vao.bind()
            if split_db is None:
                for _vs, _vc in db_draw:
                    self._gl.glDrawArrays(GL_TRIANGLES, _vs, _vc)
            else:
                self._program.setUniformValue1f(self._loc_fade,
                                                self.edit_rest_fade)
                for _vs, _vc in db_draw:
                    if _vs < split_db:
                        self._gl.glDrawArrays(GL_TRIANGLES, _vs, _vc)
                self._program.setUniformValue1f(self._loc_fade, 0.0)
                for _vs, _vc in db_draw:
                    if _vs >= split_db:
                        self._gl.glDrawArrays(GL_TRIANGLES, _vs, _vc)
            self._dback_vao.release()
            self._gl.glDisable(GL_CULL_FACE)
            self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

        # Instanced components (P2): eligible instances draw from their
        # prototype's local VBOs with per-instance matrices — the
        # consolidated passes above excluded them in _sync_edges.
        # The instanced default backs shade the same way (the flag only
        # speaks to back-facing fragments in the back colour; set for the
        # whole call, which the recorded native replay sees too).
        if mode in ("textures", "shaded") and not _NO_BACK_TINT:
            self._program.setUniformValue(self._loc_back_shade, 1)
        self._draw_instanced_faces(mode, style)
        self._program.setUniformValue(self._loc_back_shade, 0)
        # Shadows stop at the face passes: edges, overlays, previews and
        # billboards stay unlit (v1 scope).
        if getattr(self, "_frame_shadow", None) is not None:
            self._program.setUniformValue(self._loc_shadow_enable, 0)

        # Back-side material overrides (faces painted DIFFERENTLY per side,
        # two-sided paint from .skp): drawn with front-face culling so they
        # only show from behind. Match the front pass depth exactly: an
        # unbiased back can otherwise punch through a millimetre-thin wall.
        # LEQUAL lets the later back material replace its own front copy.
        if (self._back_vcol_run[1] > 0 or self._back_tex_runs) \
                and mode in ("textures", "shaded", "xray"):
            self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
            self._gl.glPolygonOffset(1.0, 1.0)
            self._gl.glEnable(GL_CULL_FACE)
            self._gl.glCullFace(GL_FRONT)
            bstart, bcount = self._back_vcol_run
            if bcount:
                self._program.setUniformValue(self._loc_use_vcolor, 1)
                self._faces_vao.bind()
                self._gl.glDrawArrays(GL_TRIANGLES, bstart, bcount)
                self._faces_vao.release()
                self._program.setUniformValue(self._loc_use_vcolor, 0)
            if self._back_tex_runs:
                self._program.setUniformValue(self._loc_use_tex, 1)
                self._tex_faces_vao.bind()
                for (path, shade), start, count in self._back_tex_runs:
                    tex = self._get_texture(path)
                    if tex is None:
                        continue
                    self._program.setUniformValue1f(self._loc_shade, float(shade))
                    self._set_cutout(getattr(tex, "_cutout", False))
                    tex.bind(0)
                    self._gl.glDrawArrays(GL_TRIANGLES, start, count)
                    tex.release(0)
                self._set_cutout(False)
                self._tex_faces_vao.release()
                self._program.setUniformValue1f(self._loc_shade, 1.0)
                self._program.setUniformValue(self._loc_use_tex, 0)
            self._gl.glDisable(GL_CULL_FACE)
            self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

        self._draw_section_fill(mode, style)
        # Ambient occlusion (#954): onto the opaque faces only — the glass
        # below, the billboards, highlights and every edge come after it.
        self._apply_ambient_occlusion(w, h, mode, style)

        # Translucent material runs (.skp trans with useTrans): drawn
        # after everything opaque, blended, depth-tested but not depth-
        # written, so glass/mesh screens show what's behind them.
        if (self._tcol_runs or self._ttex_runs
                or self._back_tcol_runs or self._back_ttex_runs) \
                and mode in ("textures", "shaded", "xray", "monochrome"):
            self._gl.glDepthMask(GL_FALSE)
            self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
            self._gl.glPolygonOffset(1.0, 1.0)
            # Monochrome: one flat tone on both sides at each run's opacity,
            # instead of the run's own colour or texture.
            flat = MONOCHROME_GLASS_RGB if mode == "monochrome" else None

            def run_opacity(a):
                self._program.setUniformValue1f(self._loc_opacity, float(a))
                if flat is not None:
                    self._set_color(*flat, float(a))

            if self._tcol_runs or self._back_tcol_runs:
                self._program.setUniformValue(self._loc_use_vcolor,
                                              0 if flat is not None else 1)
                self._faces_vao.bind()
                for a, fc, start, count in self._tcol_runs:
                    run_opacity(a)
                    if fc:
                        # glass-backed front copy: front side only
                        self._gl.glEnable(GL_CULL_FACE)
                        self._gl.glCullFace(GL_BACK)
                    self._gl.glDrawArrays(GL_TRIANGLES, start, count)
                    if fc:
                        self._gl.glDisable(GL_CULL_FACE)
                if self._back_tcol_runs:
                    # translucent BACK overrides: back side only
                    self._gl.glEnable(GL_CULL_FACE)
                    self._gl.glCullFace(GL_FRONT)
                    for a, start, count in self._back_tcol_runs:
                        run_opacity(a)
                        self._gl.glDrawArrays(GL_TRIANGLES, start, count)
                    self._gl.glDisable(GL_CULL_FACE)
                self._faces_vao.release()
                self._program.setUniformValue(self._loc_use_vcolor, 0)
            if self._ttex_runs or self._back_ttex_runs:
                self._program.setUniformValue(self._loc_use_tex,
                                              0 if flat is not None else 1)
                self._tex_faces_vao.bind()
                for (path, shade), a, fc, start, count in self._ttex_runs:
                    tex = None if flat is not None else self._get_texture(path)
                    if tex is None and flat is None:
                        continue
                    run_opacity(a)
                    self._program.setUniformValue1f(
                        self._loc_shade, 1.0 if flat is not None else float(shade))
                    if fc:
                        self._gl.glEnable(GL_CULL_FACE)
                        self._gl.glCullFace(GL_BACK)
                    if tex is not None:
                        tex.bind(0)
                    self._gl.glDrawArrays(GL_TRIANGLES, start, count)
                    if tex is not None:
                        tex.release(0)
                    if fc:
                        self._gl.glDisable(GL_CULL_FACE)
                if self._back_ttex_runs:
                    self._gl.glEnable(GL_CULL_FACE)
                    self._gl.glCullFace(GL_FRONT)
                    for (path, shade), a, start, count in self._back_ttex_runs:
                        tex = (None if flat is not None
                               else self._get_texture(path))
                        if tex is None and flat is None:
                            continue
                        run_opacity(a)
                        self._program.setUniformValue1f(
                            self._loc_shade,
                            1.0 if flat is not None else float(shade))
                        if tex is not None:
                            tex.bind(0)
                        self._gl.glDrawArrays(GL_TRIANGLES, start, count)
                        if tex is not None:
                            tex.release(0)
                    self._gl.glDisable(GL_CULL_FACE)
                self._tex_faces_vao.release()
                self._program.setUniformValue1f(self._loc_shade, 1.0)
                self._program.setUniformValue(self._loc_use_tex, 0)
            self._program.setUniformValue1f(self._loc_opacity, 1.0)
            self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
            self._gl.glDepthMask(GL_TRUE)

        # Face-me billboards (2D people): per-frame textured cutout
        # — skipped on plan sheets and line styles: a coloured cutout person
        # on a technical drawing gives the raster away; scale figures belong
        # to the shaded looks only.
        if self.plano_style is None and mode in ("textures", "shaded", "xray"):
            self._draw_billboards()
            if self._export_size is None:
                # Selection cues never reach an export: the orange box
                # around a selected Sumari came out on the printed sheet
                # (Marco, 2026-09-07).
                self._draw_billboard_outlines()

        # Face highlights (selection + hover) — translucent overlays drawn on
        # top of the cream faces. Same polygon offset as the faces so they sit
        # at matching depth (LEQUAL lets this later draw win); depth-write OFF
        # so the overlay tints without blocking the edges drawn afterwards.
        if (self._sel_faces_count > 0 or hover is not None) \
                and self._export_size is None:      # no selection cues in exports
            self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
            self._gl.glPolygonOffset(1.0, 1.0)
            self._gl.glDepthMask(GL_FALSE)
            if self._sel_faces_count > 0:
                # Selected faces: an opaque DOT pattern, not a tint — the
                # old 35% orange wash over a back face read as just another
                # back face. Each run carries the dot colour for its front
                # and for its back side, picked against that side's paint.
                self._program.setUniformValue(self._loc_stipple, 3)
                self._sel_faces_vao.bind()
                for front, back, start, count in self._sel_faces_spans:
                    self._set_color(*front, 1.0)
                    self._program.setUniformValue(
                        self._loc_back_color, QVector4D(*back, 1.0))
                    self._gl.glDrawArrays(GL_TRIANGLES, start, count)
                self._sel_faces_vao.release()
                self._program.setUniformValue(self._loc_stipple, 0)
            hovered = hover
            if isinstance(hovered, Face) and hovered not in self.scene.selection:
                # The face under the cursor (Select, Push/Pull, Paint,
                # Offset, Follow Me...) wears the same dots as a selected
                # one, so every tool points at a face the same way. An
                # already selected face is skipped: it shows its dots.
                hover_count = self._upload_hover_face(
                    hovered, self._hover_placement)
                if hover_count > 0:
                    front, back = self._selection_dot_colors(hovered)
                    self._set_color(*front, 1.0)
                    self._program.setUniformValue(
                        self._loc_back_color, QVector4D(*back, 1.0))
                    self._program.setUniformValue(self._loc_stipple, 3)
                    self._hover_faces_vao.bind()
                    self._gl.glDrawArrays(GL_TRIANGLES, 0, hover_count)
                    self._hover_faces_vao.release()
                    self._program.setUniformValue(self._loc_stipple, 0)
            self._gl.glDepthMask(GL_TRUE)
            self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

        _fmark("faces")
        # Shaded solid preview (Push/Pull box forming as you drag). Drawn after
        # the persistent faces, depth-tested so it occludes geometry behind it
        # and reads as a real solid; its wireframe goes on top via the rubber
        # band below.
        self._set_section_clip(False)
        self._draw_preview_faces()
        self._draw_groups_preview(mvp)
        self._draw_hidden_ghosts(mvp)

        self._draw_hidden_geometry_edges(mvp, w, h)
        self._draw_scaffold_depth(mvp, w, h)

        self._set_section_clip(True)
        show_edges = ((style.edges or mode == "wireframe")
                      and not getattr(self, "_nav_fast_feedback", False))
        ec = style.edge_color
        # Line weight for sheet renders: a GL line is one pixel whatever
        # the DPI (core-profile Mesa clamps glLineWidth), and one pixel at
        # 300 dpi is a 0.085 mm hairline that vanishes on paper and on the
        # composer's scaled preview (Marco, 2026-09-14: «casi no se ven»).
        # The frame asks for a pen in pixels and the pass is redrawn at
        # sub-pixel offsets to reach it — exports only, never the screen.
        _jit_e = self._line_jitter(self._export_edge_px, w, h)
        _jit_p = self._line_jitter(self._export_profile_px, w, h)
        # X-ray draws every edge, but the ones BEHIND a face come out washed
        # toward the background: the faces' depth (never written by their
        # translucent pass) is laid down here, the edges beyond it drawn
        # first in the faded colour, then the ones in front at full strength.
        # From above, an open box keeps every edge dark and a lidded one
        # greys the edges under the lid — you see whether a face is there.
        xray_edges = mode == "xray" and show_edges
        if xray_edges:
            self._xray_face_depth()
            bg = style.background
            k = XRAY_HIDDEN_EDGE_FADE
            _edge_passes = ((tuple(ec[i] + (bg[i] - ec[i]) * k
                                   for i in range(3)), True),
                            (tuple(ec[:3]), False))
        else:
            _edge_passes = ((tuple(ec[:3]), False),)
        if (show_edges and not xray_edges
                and getattr(style, "model_linetypes", True)):
            self._mask_model_linetype_gaps()
        for (_ecol, _hidden), (_dx, _dy) in (
                (_p, _j) for _p in _edge_passes for _j in _jit_e):
            if xray_edges:
                self._gl.glDepthFunc(GL_GREATER if _hidden else GL_LEQUAL)
                self._gl.glDepthMask(GL_FALSE if _hidden else GL_TRUE)
            if len(_jit_e) > 1:
                self._program.setUniformValue(self._loc_mvp,
                                              _shifted_mvp(mvp, _dx, _dy))
            if self._edges_count > 0 and show_edges:
                self._set_color(*_ecol, 1.0)
                self._edges_vao.bind()
                _espans = getattr(self, "_frame_edge_spans",
                                  ((0, self._edges_count),))
                if split_e is None:
                    for _vs, _vc in _espans:
                        self._gl.glDrawArrays(GL_LINES, _vs, _vc)
                else:
                    # Same two-tier draw as the faces: washed-out
                    # surroundings, then the edited group's own edges at
                    # full strength.
                    self._program.setUniformValue1f(self._loc_fade,
                                                    self.edit_rest_fade)
                    for _vs, _vc in _espans:
                        if _vs < split_e:
                            self._gl.glDrawArrays(GL_LINES, _vs, _vc)
                    self._program.setUniformValue1f(self._loc_fade, 0.0)
                    for _vs, _vc in _espans:
                        if _vs >= split_e:
                            self._gl.glDrawArrays(GL_LINES, _vs, _vc)
                self._edges_vao.release()
            if show_edges:
                self._set_color(*_ecol, 1.0)
                self._draw_instanced_edges()
        if show_edges and getattr(style, "model_linetypes", True):
            self._draw_model_linetype_dashes()
        if xray_edges:
            # Back to X-ray's depth: the faces never occlude, so selected
            # and hovered edges behind them still show.
            self._gl.glDepthFunc(GL_LEQUAL)
            self._gl.glDepthMask(GL_TRUE)
            self._gl.glClear(GL_DEPTH_BUFFER_BIT)
        elif (show_edges and getattr(style, "back_edges", False)
                and mode != "wireframe"):
            self._draw_back_edges(tuple(ec[:3]), w, h)
        if len(_jit_e) > 1:
            self._program.setUniformValue(self._loc_mvp, mvp)

        # Profile (silhouette) edges: soft seams of a curved surface are hidden,
        # except where the surface turns away from the viewer — the cylinder's
        # outline. View-dependent, so rebuilt every frame.
        if (show_edges and style.profiles
                and not getattr(self, "_nav_fast_feedback", False)):
            sil_count = self._upload_silhouette_edges()
            if sil_count > 0:
                self._set_color(ec[0], ec[1], ec[2], 1.0)
                self._silhouette_vao.bind()
                for _dx, _dy in _jit_p:
                    if _dx or _dy:
                        self._program.setUniformValue(
                            self._loc_mvp, _shifted_mvp(mvp, _dx, _dy))
                    self._gl.glDrawArrays(GL_LINES, 0, sil_count)
                self._silhouette_vao.release()
                if len(_jit_p) > 1:
                    self._program.setUniformValue(self._loc_mvp, mvp)
        _fmark("edges")

        self._draw_selected_group_edges(mvp, w, h)

        # Selected edges (drawn on top, highlighted) — never in an export
        if self._selected_count > 0 and self._export_size is None:
            self._set_color(0.95, 0.45, 0.16, 1.0)
            self._selected_vao.bind()
            self._gl.glDrawArrays(GL_LINES, 0, self._selected_count)
            self._selected_vao.release()

        # Hovered edge — light blue, on top of everything else so it reads as
        # the pick candidate even when it overlaps a selected (orange) edge.
        # A curve segment highlights its whole contour (what a click selects).
        if isinstance(hover, Edge):
            hover_count = self._upload_hover_edge(hover)
            self._set_color(0.30, 0.55, 0.95, 1.0)
            self._hover_edges_vao.bind()
            self._gl.glDrawArrays(GL_LINES, 0, hover_count)
            self._hover_edges_vao.release()
        self._set_section_clip(False)
        self._draw_section_cut_edges()   # ON the plane — drawn unclipped

    def _draw_scaffold_depth(self, mvp, w, h):
        """``_draw_scaffold`` under a fitted depth range: the axes run far
        past the model's box, so their depth is clamped rather than their
        lines cut at the fitted near and far planes (no geometry is)."""
        fitted = self.camera._fitted_depth_range() is not None
        if fitted:
            self._gl.glEnable(0x864F)  # GL_DEPTH_CLAMP, core since GL 3.2
        try:
            self._draw_scaffold(mvp, w, h)
        finally:
            if fitted:
                self._gl.glDisable(0x864F)

    def _draw_scaffold(self, mvp, w, h) -> None:
        """The axes, the guides and the dashed box of the group being
        edited: drawn without writing depth, over the faces."""
        # Axes — long solid positive + evenly-dashed negative per axis.
        # Dash spacing scales with the camera distance so the on-screen density
        # stays stable across zoom. Depth-write OFF so the ground axes don't cull
        # geometry sitting on z=0; drawn BEFORE user edges so an edge along an
        # axis wins the LEQUAL depth test. Rubber-band stays on top (drawn last).
        from core import axes as _axes
        if (self.plano_style is None and self.style_override is None
                and (getattr(self.scene, "show_axes", True)
                     or not _axes.is_world())):
            # No axes on a plan sheet / styled composer frame, nor the
            # world's once Camera ▸ Axes is off (an open group keeps its own).
            spacing = max(self.camera.distance * 0.008, 1e-4)
            axes_coords, self._axes_spans = _axes_vertices(spacing)
            data = axes_coords.tobytes()
            self._axes_vbo.bind()
            self._axes_vbo.allocate(data, len(data))
            self._axes_vbo.release()
            self._axes_vao.bind()
            self._gl.glDepthMask(GL_FALSE)
            for name, rgb in (("x", (0.86, 0.22, 0.27)),   # red
                              ("y", (0.16, 0.62, 0.36)),   # green
                              ("z", (0.20, 0.40, 0.78))):  # blue
                start, count = self._axes_spans[name]
                self._set_color(*rgb, 1.0)
                self._gl.glDrawArrays(GL_LINES, start, count)
            self._gl.glDepthMask(GL_TRUE)
            self._axes_vao.release()

        # Guides — dashed, DEPTH-TESTED, so geometry in front hides them
        # (issue #23). They used to be painted in the QPainter overlay,
        # which runs after the render and has no depth at all, so a guide
        # behind a wall still showed through. Same rules as the axes:
        # depth-write off (scaffolding must not cull anything), and none on
        # a plan sheet or a styled composer frame — a printed lámina does
        # not carry the scaffolding, and that is also how it behaved before.
        on_screen = (shown_guides(self.scene)
                     if self.plano_style is None
                     and self.style_override is None else [])
        if on_screen:
            dist = max(float(self.camera.distance), 1e-3)
            eye = self.camera.eye() if self.camera.perspective else None
            # The dashes only change with the camera, the guides or the
            # selection; a hover repaint reuses them and their upload (#816).
            v3 = lambda v: (None if v is None
                            else (v.x(), v.y(), v.z()))  # noqa: E731
            key = (dist, v3(self.camera.target), v3(eye),
                   tuple((id(g), v3(g.point), v3(getattr(g, "direction",
                                                         None)),
                          v3(getattr(g, "origin", None)))
                         for g in on_screen),
                   tuple(g in self.scene.selection
                         for g in on_screen))
            cached = getattr(self, "_guides_cache", None)
            if cached is not None and cached[0] == key:
                g_spans = cached[1]
            else:
                g_coords, g_spans = _guide_vertices(
                    on_screen, dist * 0.03, self.scene.selection,
                    centre=self.camera.target, reach=dist * 15.0,
                    # Parallel: one scale at every depth, nothing to adjust.
                    eye=eye)
                self._guides_cache = (key, g_spans)
                if g_spans:
                    data = g_coords.tobytes()
                    self._guides_vbo.bind()
                    self._guides_vbo.allocate(data, len(data))
                    self._guides_vbo.release()
            if g_spans:
                self._guides_vao.bind()
                self._gl.glDepthMask(GL_FALSE)
                for start, count, selected in g_spans:
                    if selected:
                        self._set_color(0.95, 0.45, 0.16, 1.0)   # selection
                    else:
                        self._set_color(0.27, 0.35, 0.47, 1.0)
                    self._gl.glDrawArrays(GL_LINES, start, count)
                self._gl.glDepthMask(GL_TRUE)
                self._guides_vao.release()

        # The dashed box of the group being edited, depth-tested like the
        # guides (it shares their buffer): the part behind the geometry is
        # hidden instead of showing through it. The dashes live in the
        # shader (u_stipple 4): twelve solid edges at any zoom, not a
        # per-frame tessellation whose count scaled with 1/distance.
        box = (self._edit_group_box_corners()
               if self.plano_style is None and self.style_override is None
               else None)
        if box is not None:
            data = _box_edge_vertices(box).tobytes()
            if data:
                self._guides_cache = None   # the guides' dashes are gone
                self._guides_vbo.bind()
                self._guides_vbo.allocate(data, len(data))
                self._guides_vbo.release()
                self._guides_vao.bind()
                # Dash size in framebuffer pixels for visual parity with
                # the old world-space dashes: their 0.006·distance length
                # spanned 0.003/tan(fov/2) of the viewport height — a
                # screen fraction, independent of zoom (perspective frames
                # the target at 2·distance·tan(fov/2); parallel sizes its
                # height from the same product). w/h are framebuffer
                # pixels, which is what gl_FragCoord measures.
                fov = math.radians(float(self.camera.fov_deg))
                dash = min(20.0, max(3.0, 0.003 * h / math.tan(fov / 2.0)))
                self._program.setUniformValue(self._loc_viewport_px,
                                              QVector2D(float(w), float(h)))
                self._program.setUniformValue1f(self._loc_dash_px, dash)
                self._program.setUniformValue(self._loc_stipple, 4)
                self._gl.glDepthMask(GL_FALSE)
                self._set_color(90 / 255, 110 / 255, 140 / 255, 1.0)
                self._gl.glDrawArrays(GL_LINES, 0, len(data) // 12)
                self._program.setUniformValue(self._loc_stipple, 0)
                self._gl.glDepthMask(GL_TRUE)
                self._guides_vao.release()

    def _frame_cache_key(self, mvp, w, h, style, mode):
        """What the model this frame draws depends on, when a pointer move
        may reuse the last frame's (``_frame_cache_restore``) — or None:
        the switch off, an export, or a frame whose passes the cursor's
        cues cannot simply go over (X-ray and back edges draw edges through
        the faces, a section cut its own edges, a tool's preview, the hidden
        ghosts, the base map, terrain and survey layers). The model over
        the plant of #158 took the GPU 40 ms a frame, every pointer move.

        Everything the static passes read is in it: the view, the scene's
        versions (a selection bumps it), every buffer sent to the GPU since
        (``_gpu_epoch``), the style, the open group, the guides, the
        textures loaded, the shadow map."""
        if not _FRAME_CACHE or self._export_size is not None:
            return None
        sc = self.scene
        if (mode == "xray" or getattr(style, "back_edges", False)
                or self.plano_style is not None
                or getattr(self, "_clip_vec", None) is not None
                or getattr(sc, "show_hidden_objects", False)
                or getattr(sc, "show_hidden_geometry", False)
                or getattr(self, "_preview_groups", None)
                or self._base_map_showing() or self._terrain_showing()
                or self._photo_showing()):
            return None
        tool = self.active_tool
        provider = getattr(tool, "preview_faces", None) if tool else None
        if callable(provider) and provider():
            return None
        v3 = lambda v: (None if v is None  # noqa: E731
                        else (v.x(), v.y(), v.z()))
        try:
            look = repr(sorted(vars(style).items()))
        except TypeError:
            look = repr(style)
        return (tuple(mvp.data()), w, h, getattr(self, "_msaa", 4), mode,
                look, id(self.style_override), id(sc), sc.version,
                _content_ver(self), _cache_ver(self),
                getattr(self, "_gpu_epoch", 0),
                id(sc.edit_group), self._edit_rest_mode,
                self.edit_rest_fade,
                getattr(sc, "show_axes", True),
                tuple((id(g), v3(g.point), v3(getattr(g, "direction", None)),
                       v3(getattr(g, "origin", None)), g in sc.selection)
                      for g in (getattr(sc, "guides", None) or ())),
                len(getattr(self, "_tex_cache", None) or ()),
                id(getattr(self, "_frame_shadow", None)),
                getattr(self, "_shadow_key", None),
                # Soft shadows (#1281): set from Preferences, a still view
                # kept the old level until the camera moved (Windows round
                # 0.6.2).
                getattr(self, "_soft_shadows", 0),
                self._faces_count, self._edges_count, self._sel_faces_count,
                self._selected_count, self.camera.fov_deg,
                self.camera.perspective,
                getattr(self, "_nav_fast_feedback", False))

    def _frame_cache_track(self, mvp_key) -> bool:
        """Note this frame's view (``mvp_key``: the frame cache key's MVP
        element, or the MVP itself when there is no key) and answer
        whether it is the same as the last frame's — the view has
        settled, so a picture kept now can be restored by the frames
        after it. While the camera moves every frame's differs."""
        prev = getattr(self, "_frame_cache_prev_mvp", None)
        self._frame_cache_prev_mvp = mvp_key
        return prev == mvp_key

    def _frame_cache_store(self, key, w, h) -> None:
        """Keep the scene FBO's colour and depth as the static passes left
        them, for the pointer moves after (``_frame_cache_restore``)."""
        fbo = getattr(self, "_frame_cache_fbo", None)
        if fbo is None or not fbo.isValid() \
                or (fbo.width(), fbo.height()) != (w, h) \
                or fbo.format().samples() != self._scene_fbo.format().samples():
            fbo = self._frame_cache_fbo = QOpenGLFramebufferObject(
                w, h, self._scene_fbo.format())
        self._frame_cache_copy(self._scene_fbo, fbo, w, h)
        self._frame_cache = key

    def _frame_cache_restore(self, key, w, h) -> bool:
        """The kept model back into the scene FBO when ``key`` is still
        what this frame would draw; then only the cursor's cues are drawn
        (``_paint_hover``)."""
        fbo = getattr(self, "_frame_cache_fbo", None)
        # A context made again (the widget moved to another window) takes
        # its FBOs with it; read from one, the blit took the window's own.
        if (fbo is None or getattr(self, "_frame_cache", None) != key
                or not fbo.isValid() or (fbo.width(), fbo.height()) != (w, h)):
            return False
        self._frame_cache_copy(fbo, self._scene_fbo, w, h)
        return True

    def _frame_cache_copy(self, src, dst, w, h) -> None:
        extra = self.context().extraFunctions()
        self._gl.glBindFramebuffer(GL_READ_FRAMEBUFFER, src.handle())
        self._gl.glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.handle())
        extra.glBlitFramebuffer(0, 0, w, h, 0, 0, w, h,
                                GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT,
                                GL_NEAREST)
        self._scene_fbo.bind()
        self._gl.glViewport(0, 0, w, h)

    def _paint_hover(self, mvp, w, h) -> None:
        """The cursor's cues over the kept model: the dots of the face under
        it, then — on those dots only — the axes, guides and group box the
        full frame draws after them, so they read as before; the edge
        under it on top. Where an edge lies at the same depth as an axis
        the edge keeps it (``GL_LESS``), as when it was drawn later."""
        hovered = self._hover_entity
        if isinstance(hovered, Face) and hovered not in self.scene.selection:
            hover_count = self._upload_hover_face(hovered,
                                                  self._hover_placement)
            if hover_count > 0:
                self._set_section_clip(True)
                self._gl.glClearStencil(0)
                self._gl.glClear(GL_STENCIL_BUFFER_BIT)
                self._gl.glEnable(GL_STENCIL_TEST)
                self._gl.glStencilMask(0xFF)
                self._gl.glStencilFunc(GL_ALWAYS, 1, 0xFF)
                self._gl.glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE)
                self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
                self._gl.glPolygonOffset(1.0, 1.0)
                self._gl.glDepthMask(GL_FALSE)
                front, back = self._selection_dot_colors(hovered)
                self._set_color(*front, 1.0)
                self._program.setUniformValue(
                    self._loc_back_color, QVector4D(*back, 1.0))
                self._program.setUniformValue(self._loc_stipple, 3)
                self._hover_faces_vao.bind()
                self._gl.glDrawArrays(GL_TRIANGLES, 0, hover_count)
                self._hover_faces_vao.release()
                self._program.setUniformValue(self._loc_stipple, 0)
                self._gl.glDepthMask(GL_TRUE)
                self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
                self._set_section_clip(False)
                self._gl.glStencilFunc(GL_EQUAL, 1, 0xFF)
                self._gl.glStencilMask(0x00)
                self._gl.glDepthFunc(GL_LESS)
                self._draw_scaffold_depth(mvp, w, h)
                self._program.setUniformValue(self._loc_mvp, mvp)
                self._gl.glDepthFunc(GL_LEQUAL)
                self._gl.glStencilMask(0xFF)
                self._gl.glDisable(GL_STENCIL_TEST)
        if isinstance(hovered, Edge):
            self._set_section_clip(True)
            hover_count = self._upload_hover_edge(hovered)
            self._set_color(0.30, 0.55, 0.95, 1.0)
            self._hover_edges_vao.bind()
            self._gl.glDrawArrays(GL_LINES, 0, hover_count)
            self._hover_edges_vao.release()
            self._set_section_clip(False)

    def _apply_ambient_occlusion(self, w, h, mode, style=None) -> None:
        """The ambient occlusion pass (#954, the style's switch)
        over the faces the scene FBO holds. Wireframe and X-ray have no
        opaque faces to darken."""
        # The foliage marking ends with the opaque faces, AO or not.
        self._gl.glDisable(GL_STENCIL_TEST)
        self._gl.glStencilMask(0xFF)
        from views import ambient_occlusion as _ao
        settings = _ao.settings_for(style)
        if settings is None or mode in ("wireframe", "xray"):
            return
        if getattr(self, "_ao", None) is None:
            self._ao = _ao.AmbientOcclusion(self, SHADER_DIR)
        if not self._ao.apply(self._gl, self.context().extraFunctions(),
                              self._scene_fbo, w, h,
                              self.camera.projection_matrix(), settings):
            if self._ao.broken and not getattr(self, "_ao_warned", False):
                self._ao_warned = True
                print(f"[ambient occlusion] off: {self._ao.broken}")
        self._program.bind()

    # ---- Setup helpers ------------------------------------------------------
    def _compile_program(self) -> QOpenGLShaderProgram:
        prog = ShaderProgram(self)
        ok_v = prog.addShaderFromSourceFile(
            QOpenGLShader.Vertex, str(SHADER_DIR / "basic.vert")
        )
        ok_f = prog.addShaderFromSourceFile(
            QOpenGLShader.Fragment, str(SHADER_DIR / "basic.frag")
        )
        if not (ok_v and ok_f and prog.link()):
            raise RuntimeError("shader compile/link failed:\n" + prog.log())
        return prog

    def _compile_depth_program(self) -> QOpenGLShaderProgram:
        prog = ShaderProgram(self)
        ok_v = prog.addShaderFromSourceFile(
            QOpenGLShader.Vertex, str(SHADER_DIR / "depth.vert")
        )
        ok_f = prog.addShaderFromSourceFile(
            QOpenGLShader.Fragment, str(SHADER_DIR / "depth.frag")
        )
        if not (ok_v and ok_f and prog.link()):
            raise RuntimeError(
                "depth shader compile/link failed:\n" + prog.log())
        return prog

    def _upload_static(self, data: array):
        vao = QOpenGLVertexArrayObject(self)
        vao.create()
        vao.bind()
        vbo = QOpenGLBuffer(QOpenGLBuffer.VertexBuffer)
        vbo.create()
        vbo.bind()
        raw = data.tobytes()
        vbo.allocate(raw, len(raw))
        self._program.bind()
        self._program.enableAttributeArray(self._loc_pos)
        self._program.setAttributeBuffer(self._loc_pos, GL_FLOAT, 0, 3)
        self._program.release()
        vbo.release()
        vao.release()
        return vao, vbo, len(data) // 3

    def _create_dynamic_color(self):
        """A dynamic VAO/VBO interleaving position (3f) + RGB (3f) per
        vertex — the sky-gradient backdrop (drawn with ``u_use_vcolor``)."""
        vao = QOpenGLVertexArrayObject(self)
        vao.create()
        vao.bind()
        vbo = QOpenGLBuffer(QOpenGLBuffer.VertexBuffer)
        vbo.setUsagePattern(QOpenGLBuffer.DynamicDraw)
        vbo.create()
        vbo.bind()
        vbo.allocate(48)  # 2 vertices × 6 floats × 4 bytes
        stride = 6 * 4
        self._program.bind()
        self._program.enableAttributeArray(self._loc_pos)
        self._program.setAttributeBuffer(self._loc_pos, GL_FLOAT, 0, 3, stride)
        self._program.enableAttributeArray(self._loc_vcolor)
        self._program.setAttributeBuffer(self._loc_vcolor, GL_FLOAT, 12, 3,
                                         stride)
        self._program.release()
        vbo.release()
        vao.release()
        return vao, vbo

    def _front_face(self, mirrored: bool) -> None:
        """Clockwise front faces for a MIRRORED batch — the mirror flips the
        winding of every prototype triangle, and GL decides front and back
        (``gl_FrontFacing``, the culled back-tint pass) by winding."""
        self._gl.glFrontFace(GL_CW if mirrored else GL_CCW)

    def _inst_perf_summary(self):
        """CPU submission timing and allocated prototype VBO bytes.

        Submission can stall in the driver, but is not a GPU timer query.
        Strict-view rejects differ from submissions because the existing
        guard band deliberately keeps some offscreen placements in tapes.
        """
        st = self._inst_perf
        gpu_bytes = sum(e.get("gpu_static_bytes", 0)
                        + e.get("gpu_matrix_bytes", 64)
                        + (e["mirror"].get("gpu_matrix_bytes", 64)
                           if "mirror" in e else 0)
                        for e in (getattr(self, "_proto_draw", None) or {}).values())
        return (f" inst_proto={st['prototypes']} inst_place={st['placements']}"
                f" inst_frustum={st['frustum']} inst_pixel={st['pixel']}"
                f" inst_batches={st['batches']} inst_draws={st['draws']}"
                f" face_draws={st['faces_draws']} edge_draws={st['edges_draws']}"
                f" gather={st['gather_ms']:.1f}ms"
                f" inst_python={max(0, st['pass_ms'] - st['submit_ms']):.1f}ms"
                f" inst_gl_submit={st['submit_ms']:.1f}ms"
                f" inst_vbo={gpu_bytes / 1048576:.1f}MiB")

    def _perf_instanced_draw(self, extra, *args):
        if not _PERF:
            return extra.glDrawArraysInstanced(*args)
        start = _time_mod.perf_counter()
        extra.glDrawArraysInstanced(*args)
        st = self._inst_perf
        st['draws'] += 1
        st['submit_ms'] += (_time_mod.perf_counter() - start) * 1000

    def _gather_instanced(self):
        start = _time_mod.perf_counter() if _PERF else 0.0
        result = Viewport._gather_instanced_impl(self)
        if _PERF:
            st = self._inst_perf
            st['gather_ms'] += (_time_mod.perf_counter() - start) * 1000
            memo = getattr(self, '_inst_perf_counts', None)
            key = (id(result), self.scene.version, self.edit_rest_fade)
            if memo is None or memo[0] != key or memo[1] is not result:
                counts = (len(result), sum(len(v[2]) for v in result.values()),
                          sum(sum(bool(b) for b, _ in self._instanced_batches(v[2]))
                              for v in result.values()))
                memo = self._inst_perf_counts = (key, result, counts)
            st['prototypes'], st['placements'], st['batches'] = memo[2]
        return result

    def _gather_instanced_impl(self):
        """Visible, eligible instances grouped by prototype mesh — computed
        once per frame (faces pass), reused by the edges pass. Instances are
        frustum-culled by their baked chunk's world AABB.

        WHICH placements are eligible changes only with the scene; only the
        frustum test is per frame. Keeping a component's internal sharing
        multiplied the candidates (89 groups -> 411 placements on Marco's
        pool) and re-deriving eligibility every frame cost 2.5 ms of pure
        Python per zoom notch against 0.4 ms before — the wheel felt heavier.
        The pool is cached per scene version and the cull is one NumPy pass
        over every box at once."""
        import numpy as np
        pv = getattr(self, "_preview_groups", None) or ()
        planes = getattr(self, "_frame_planes", None)
        # The previewed placements are masked out below, not left out of
        # the pool: it then lasts while a Move picks an object up, and when
        # it puts it down only that one's row moves (_inst_pool_moved) —
        # each was a new pool, ~0.9 s on the plant of #158. Nor does the
        # frozen preview key it: what the pool holds is the epoch's.
        key = (self._placements_epoch(), id(self.scene.mesh))
        pool = getattr(self, "_inst_pool", None)
        if pool is not None and pool[0] != key:
            moved = self._inst_pool_moved(pool, key)
            pool = self._inst_pool = (moved if moved is not None
                                      else self._inst_pool_build(key, pool))
        if pool is None or pool[0] != key:
            pool = self._inst_pool = self._inst_pool_build(key)
        _k, groups, lo, hi, codes_a, protos, mats, by_code = pool
        codes_a, protos, by_code = self._inst_draw_codes(pool)
        out: dict = {}
        if not groups:
            self._frame_instanced = out
            return out
        if planes is None:
            keep = np.ones(len(groups), dtype=bool)
        else:
            # Same p-vertex test as ``_aabb_visible``, all boxes at once —
            # a plane at a time on the box columns (the (planes, boxes, 3)
            # array it used to build was most of a frame's gather, #158);
            # the same products and sums, in the same order.
            keep = self._instances_in_frustum(lo, hi, planes)
            below = self._instances_below_a_pixel(pool)
            if _PERF:
                # Strict-view rejection counts; submitted batches may retain
                # the existing wider guard band for stable command tapes.
                self._inst_perf['frustum'] = int((~keep).sum())
                self._inst_perf['pixel'] = int((keep & below).sum())
            keep &= ~below
        if pv:
            keep &= ~self._inst_pool_previewed(pool, pv)
        # One pool per (prototype, container paint): a painted instance
        # (issue #47) draws its own bake of the prototype, not the plain
        # one its siblings share — pooled by mesh alone it kept showing
        # unpainted from outside while its inside (the loose bake) was
        # painted (Marco, 2026-09-21, bench test 21).
        # The same placements in view as last frame (an orbit mostly keeps
        # them): the same batches, so the matrix buffers need no look.
        memos = getattr(self, "_inst_frame_memo", None)
        if memos is None:
            memos = self._inst_frame_memo = {}
        # Dense assemblies retain a wider guard band even at rest, so the
        # first navigation frame can reuse its instance buffers as well.
        dense = len(groups) >= 2000
        margin = (max(self._INST_VIEW_MARGIN, 0.35) if dense
                  else self._INST_VIEW_MARGIN)
        superset = (max(self._INST_SUPERSET_MAX, 3) if dense
                    else self._INST_SUPERSET_MAX)
        last = memos.get(planes is None)
        pvkey = frozenset(pv)
        if last is not None and last[3] != pvkey:
            last = None           # its batches still hold what is picked up
        if (last is not None and last[0] is pool
                and np.array_equal(last[1], keep)):
            self._frame_instanced = last[2]
            return last[2]
        if planes is not None and len(planes) == 6:
            # An orbit brings a few placements in or out of view on almost
            # every frame, and each change re-recorded the whole instanced
            # pass in Python: ~70 ms a frame close to the plant of #158,
            # 3 800 prototypes. So the batches drawn are a SUPERSET of what
            # is in view — the frustum widened, the pixel floor halved —
            # kept while what is in view stays inside them and they hold
            # no more than _INST_SUPERSET_MAX times as many placements. What is drawn beyond
            # the view is clipped; nothing on screen changes.
            if (last is not None and last[0] is pool
                    and not (keep & ~last[1]).any()
                    and int(last[1].sum()) <= superset * int(keep.sum()) + 64):
                self._frame_instanced = last[2]
                return last[2]
            keep = self._instances_in_frustum(
                lo, hi, self._widened_planes(planes, margin))
            keep &= ~self._instances_below_a_pixel(pool, 0.5)
            if pv:
                keep &= ~self._inst_pool_previewed(pool, pv)
        # The kept placements of each pool, in placement order, the pools
        # in the order their first kept placement comes (the face-by-face
        # dict this replaces filled them so).
        kept = by_code[keep[by_code]]
        if not len(kept):
            self._frame_instanced = out
            return out
        kc = codes_a[kept]
        cuts = np.flatnonzero(np.diff(kc)) + 1
        starts = np.concatenate([[0], cuts])
        ends = np.concatenate([cuts, [len(kept)]])
        firsts = kept[starts]
        previous = last[2] if last is not None and last[0] is pool else {}
        for j in np.argsort(firsts, kind="stable").tolist():
            idx = kept[starts[j]:ends[j]]
            code = int(kc[starts[j]])
            mesh, paint, mirrored = protos[code]
            batch_key = (id(mesh), _material_sig(paint), mirrored)
            old = previous.get(batch_key)
            if (old is not None and old[2].mats is mats
                    and np.array_equal(old[2].idx, idx)):
                batch = old[2]
            else:
                batch = _InstanceBatch(groups[i] for i in idx.tolist())
                batch.idx = idx
                batch.mats = mats
                batch.draw_bucket = code // 16
            out[batch_key] = (
                mesh, paint, batch, mirrored)
        memos[planes is None] = (pool, keep, out, pvkey)
        self._frame_instanced = out
        return out

    def _inst_draw_codes(self, pool):
        """Share draw batches between prototypes with identical GPU data.

        Imported assemblies can contain separately named definitions of
        the same part. Keep their editing/picking identities in the pool;
        only the renderer uses a common representative. Compare the actual
        bytes (including normals, backs and texture runs), not just a hash.
        Recompute when the pool changes, never during an ordinary orbit.
        """
        memo = getattr(self, "_inst_draw_codes_memo", None)
        if memo is not None and memo[0] is pool:
            return memo[1]
        import numpy as np
        signatures, representatives, remap = {}, [], []
        for mesh, paint, mirrored in pool[5]:
            base = self._proto_base_chunk(mesh, paint)
            signature = (mirrored,
                         *(base.get(k, b"") or b"" for k in
                           ("vcol", "edges", "dback", "smooth_vcol")),
                         tuple(base["by_texture"].items()))
            code = signatures.get(signature)
            if code is None:
                code = signatures[signature] = len(representatives)
                representatives.append((mesh, paint, mirrored))
            remap.append(code)
        if len(representatives) == len(pool[5]):
            result = (pool[4], pool[5], pool[7])
        else:
            codes = np.asarray(remap, dtype=np.int64)[pool[4]]
            result = (codes, representatives, np.argsort(codes, kind="stable"))
        self._inst_draw_codes_memo = (pool, result)
        return result

    def _inst_pool_build(self, key, old=None):
        """The instanced pool for ``key``: every visible, eligible
        placement with its box, its batch (prototype, container paint,
        mirror) and its matrix. With the ``old`` pool, the rows of the
        objects ``_epoch_kept_tops`` finds unchanged are taken from it, not
        worked out again — the same pool, row for row, as one made afresh
        (a Delete or a Paste beside 21 406 placements, #158)."""
        import numpy as np
        kept = (self._epoch_kept_tops(old[0][0], key[0], ctx=True)
                if old is not None else None)
        row = {}
        if kept:
            _k, o_groups, o_lo, o_hi, o_codes, o_protos, o_mats, _b = old
            row = {id(g): i for i, g in enumerate(o_groups)}
            o_pk = [(id(m), _material_sig(p), mir) for m, p, mir in o_protos]
        placements = self._placements()
        top_of = self._placement_top
        groups: list = []
        boxes: list = []                  # a box, or the old row's index
        codes: list = []
        protos: list = []                 # code → (mesh, paint, mirrored)
        code_of: dict = {}
        for g in placements:
            if kept and top_of(g) in kept:
                # Unchanged: in the old pool exactly when it is in this one.
                i = row.get(id(g))
                if i is None:
                    continue
                pk = o_pk[int(o_codes[i])]
                c = code_of.get(pk)
                if c is None:
                    c = code_of[pk] = len(protos)
                    protos.append(o_protos[int(o_codes[i])])
                groups.append(g)
                boxes.append(i)
                codes.append(c)
                continue
            if not self.scene.entity_visible(g):
                continue
            if not self._instanced_eligible(g):
                continue
            groups.append(g)
            boxes.append(self._placement_bbox(g))
            # Which pool (prototype, container paint, mirror) it draws
            # in, and its matrix: fixed while the placements are, so
            # not worked out again per frame (#158: 21 406 placements).
            paint = _paint_of(self, g)
            mirrored = g.xform.determinant() < 0.0
            pk = (id(g.mesh), _material_sig(paint), mirrored)
            c = code_of.get(pk)
            if c is None:
                c = code_of[pk] = len(protos)
                protos.append((g.mesh, paint, mirrored))
            codes.append(c)
        n = len(groups)
        lo = np.empty((n, 3), dtype=np.float64)
        hi = np.empty((n, 3), dtype=np.float64)
        mats = np.empty((n, 16), dtype=np.float32)
        new_rows = []
        if row:
            src = np.fromiter((b if type(b) is int else -1 for b in boxes),
                              np.int64, count=n)
            old_at = np.flatnonzero(src >= 0)
            lo[old_at] = o_lo[src[old_at]]
            hi[old_at] = o_hi[src[old_at]]
            mats[old_at] = o_mats[src[old_at]]
            new_rows = np.flatnonzero(src < 0).tolist()
        else:
            new_rows = range(n)
        for j in new_rows:
            # Boxless chunks (unknown extents) always draw: give them an
            # infinite box so the vectorised test keeps them.
            b = boxes[j]
            lo[j] = b[0] if b else (-np.inf,) * 3
            hi[j] = b[1] if b else (np.inf,) * 3
            mats[j] = groups[j].xform.data()
        codes_a = np.asarray(codes, dtype=np.int64)
        by_code = np.argsort(codes_a, kind="stable")
        return (key, groups, lo, hi, codes_a, protos, mats, by_code)

    def _inst_pool_previewed(self, pool, pv):
        """Mask of the pool's placements a preview draws instead."""
        import numpy as np
        memo = getattr(self, "_inst_pv_memo", None)
        key = (pool[0], frozenset(pv))
        if memo is None or memo[0] != key or memo[1] is not pool:
            groups = pool[1]
            memo = self._inst_pv_memo = (key, pool, np.fromiter(
                (id(g) in pv for g in groups), bool, count=len(groups)))
        return memo[2]

    def _inst_pool_moved(self, pool, key):
        """The instanced pool ``pool`` brought up to ``key`` when only
        placement matrices changed in between: the rows that moved get
        their matrix and box again. ``None`` when it cannot be (anything
        else changed, or a placement turned into its mirror image, which
        draws in another batch)."""
        import numpy as np
        old = pool[0]
        if old[1:] != key[1:] or not self._epoch_only_matrices(old[0], key[0]):
            return None
        _k, groups, lo, hi, codes_a, protos, mats, by_code = pool
        if not groups:
            return (key,) + tuple(pool[1:])
        now = np.array([g.xform.data() for g in groups],
                       dtype=np.float32).reshape(-1, 16)
        moved = np.flatnonzero((now != mats).any(axis=1))
        lo, hi = lo.copy(), hi.copy()
        for i in moved.tolist():
            g = groups[i]
            if (g.xform.determinant() < 0.0) != protos[int(codes_a[i])][2]:
                return None
            box = self._placement_bbox(g)
            lo[i] = box[0] if box else (-np.inf,) * 3
            hi[i] = box[1] if box else (np.inf,) * 3
        return (key, groups, lo, hi, codes_a, protos, now, by_code)

    def _ensure_proto_draw(self, mesh, paint=None):
        """Static draw entry of one prototype: vcol/edges/texture VBOs from
        the LOCAL base chunk, three VAOs wiring them to the shared
        per-instance matrix buffer (divisor 1), built once per proto rev —
        and per container paint (issue #47)."""
        cache = getattr(self, "_proto_draw", None)
        if cache is None:
            cache = self._proto_draw = {}
        # While the placements (and so every prototype's bake) are as they
        # were, the entry is the one already made: re-deriving the base
        # chunk's fingerprint per prototype, per pass, per frame was a large
        # part of drawing 2 722 prototypes (live profile, #158).
        ckey = (id(mesh), _material_sig(paint))
        epoch = self._placements_epoch()
        fast = getattr(self, "_proto_draw_fast", None)
        if fast is None or fast[0] != epoch:
            fast = self._proto_draw_fast = (epoch, {})
        hit = fast[1].get(ckey)
        if hit is not None and cache.get(ckey) is hit:
            return hit
        entry = self._ensure_proto_draw_slow(mesh, paint, cache)
        fast[1][ckey] = entry
        return entry

    def _ensure_proto_draw_slow(self, mesh, paint, cache):
        base = self._proto_base_chunk(mesh, paint)
        key = (base["uid"], base.get("rev"))
        ckey = (id(mesh), _material_sig(paint))
        entry = cache.get(ckey)
        if entry is not None and entry["key"] == key:
            return entry
        if entry is not None:
            # The prototype changed: its GPU objects go back to the driver.
            # Replacing the entry alone kept 4 VAOs and 5 buffers alive per
            # edit of a component, for the whole session.
            self._destroy_proto_draw_entry(entry)

        def static_vbo(raw):
            vbo = QOpenGLBuffer(QOpenGLBuffer.VertexBuffer)
            vbo.setUsagePattern(QOpenGLBuffer.StaticDraw)
            vbo.create()
            vbo.bind()
            vbo.allocate(raw or b"\0" * 4, max(len(raw), 4))
            vbo.release()
            return vbo

        self._program.bind()
        vcol_raw = base["vcol"]
        edges_raw = base["edges"]
        dback_raw = base.get("dback", b"")
        # textures: pos(3)+uv(2), one run per (path, shade)
        tex_parts = []
        tex_runs = []
        start = 0
        for tkey, raw in base["by_texture"].items():
            tex_parts.append(raw)
            n = len(raw) // 20
            tex_runs.append((tkey, start, n))
            start += n
        static_bytes = sum(max(len(raw), 4) for raw in
                           (vcol_raw, edges_raw, dback_raw, b"".join(tex_parts)))
        if base.get("smooth_vcol"):
            static_bytes += max(len(base["smooth_vcol"]), 4)
        entry = {"key": key, "gpu_static_bytes": static_bytes,
                 "vcol_vbo": static_vbo(vcol_raw),
                 "vcol_count": len(vcol_raw) // 24,
                 "normal_vbo": (static_vbo(base["smooth_vcol"])
                                if base.get("smooth_vcol") else None),
                 "edges_vbo": static_vbo(edges_raw),
                 "edge_count": len(edges_raw) // 12,
                 "dback_vbo": static_vbo(dback_raw),
                 "dback_count": len(dback_raw) // 12,
                 "tex_vbo": static_vbo(b"".join(tex_parts)),
                 "tex_runs": tex_runs}
        entry.update(self._wire_proto_vaos(entry))
        # The main program stays BOUND. This is built mid-frame, from a pass
        # that is drawing with it, and releasing it here left every draw
        # call after this point without a shader: the first frame after a
        # prototype changed showed nothing from here on — the edited group,
        # its instanced neighbours, the axes, the figure. Inside a nested
        # group every edit changes a prototype, so every edit blanked the
        # group for a frame («por un segundo el grupo desaparece, pensé que
        # se había eliminado», Marco, 2026-09-11). With shadows on it never
        # showed: the shadow pass builds the entry first and rebinds its own
        # depth program afterwards.
        cache[ckey] = entry
        return entry

    def _wire_proto_vaos(self, statics) -> dict:
        """A per-instance matrix buffer and the four VAOs that draw the
        prototype's static buffers (``statics``: vcol/edges/dback/tex
        ``_vbo``) with it, divisor 1 — ``mat_sig`` empty. Program bound."""
        extra = self.context().extraFunctions()
        mat_vbo = QOpenGLBuffer(QOpenGLBuffer.VertexBuffer)
        mat_vbo.setUsagePattern(QOpenGLBuffer.DynamicDraw)
        mat_vbo.create()
        mat_vbo.bind()
        mat_vbo.allocate(64)
        mat_vbo.release()

        def wire_matrix():
            # Caller keeps the VAO bound; attach the matrix columns.
            mat_vbo.bind()
            for i, loc in enumerate(self._loc_inst):
                self._program.enableAttributeArray(loc)
                self._program.setAttributeBuffer(loc, GL_FLOAT, i * 16, 4, 64)
                extra.glVertexAttribDivisor(loc, 1)
            mat_vbo.release()

        def vao(vbo, attrs, stride):
            v = QOpenGLVertexArrayObject(self)
            v.create()
            v.bind()
            vbo.bind()
            for loc, offset, size in attrs:
                self._program.enableAttributeArray(loc)
                self._program.setAttributeBuffer(loc, GL_FLOAT, offset, size,
                                                 stride)
            vbo.release()
            if vbo is statics["vcol_vbo"] and statics.get("normal_vbo"):
                statics["normal_vbo"].bind()
                self._program.enableAttributeArray(7)
                self._program.setAttributeBuffer(7, GL_FLOAT, 0, 3, 12)
                statics["normal_vbo"].release()
            wire_matrix()
            v.release()
            return v
        pos = (self._loc_pos, 0, 3)
        return {"mat_vbo": mat_vbo, "mat_sig": None,
                # vcol: pos(3)+rgb(3); edges, default-back tint: pos(3);
                # textures: pos(3)+uv(2)
                "vcol_vao": vao(statics["vcol_vbo"],
                                (pos, (self._loc_vcolor, 12, 3)), 24),
                "edges_vao": vao(statics["edges_vbo"], (pos,), 12),
                "dback_vao": vao(statics["dback_vbo"], (pos,), 12),
                "tex_vao": vao(statics["tex_vbo"],
                               (pos, (self._loc_uv, 12, 2)), 20)}

    def _proto_view(self, entry, mirrored: bool) -> dict:
        """The draw entry a batch of ``mirrored`` placements uses: the
        prototype's own, or its mirror view — the same static buffers with
        a matrix buffer and VAOs of its own, so a prototype placed both
        ways no longer refills one matrix buffer twice per pass, every
        frame (6 203 mirrored copies in #158)."""
        if not mirrored:
            return entry
        view = entry.get("mirror")
        if view is None:
            self._program.bind()
            view = dict(entry)
            view.pop("mirror", None)
            view.update(self._wire_proto_vaos(entry))
            view["mat_batch"] = None
            view["shared"] = True
            entry["mirror"] = view
        return view

    @staticmethod
    def _destroy_proto_draw_entry(entry) -> None:
        """Free one prototype draw entry's VAOs and buffers (GL context
        current) — and its mirror view's own (the static buffers are the
        entry's)."""
        mirror = entry.pop("mirror", None)
        if mirror is not None:
            for k in ("mat_vbo", "vcol_vao", "edges_vao", "dback_vao",
                      "tex_vao"):
                obj = mirror.get(k)
                try:
                    obj.destroy()
                    if k.endswith("_vao"):
                        obj.deleteLater()
                except (RuntimeError, AttributeError):
                    pass
        for k, obj in entry.items():
            if k.endswith("_vao") or k.endswith("_vbo"):
                try:
                    obj.destroy()
                    if k.endswith("_vao"):
                        obj.deleteLater()      # a QObject child of the viewport
                # Already gone with the context, or never made (None).
                except (RuntimeError, AttributeError):
                    pass

    def _update_inst_matrices(self, entry, groups) -> int:
        # The placements' matrices are in the epoch: with it and the ids,
        # nothing has to read sixteen numbers per placement per pass
        # (~20 ms a frame for 21 406 placements, issue #158). A groups
        # preview moves matrices under a frozen version; the epoch is
        # walked every tick then, so it still sees them.
        epoch_of = getattr(self, "_placements_epoch", None)
        if isinstance(groups, _InstanceBatch) and epoch_of is not None:
            # A frame's pool: its pool indices say which placements, and
            # the pool holds their matrices already.
            if entry.get("mat_batch") is groups:
                return len(groups)          # this very batch is in
            # Recorded passes depend on these bindings. One generation
            # check replaces walking thousands of entries on every frame;
            # shadow/preview batches invalidate the records here as well.
            self._inst_matrix_serial = getattr(self, "_inst_matrix_serial", 0) + 1
            import numpy as np
            sig = (epoch_of(), id(groups.mats), len(groups.idx))
            old = entry["mat_sig"]
            if (old is not None and old[0] == sig
                    and np.array_equal(old[1], groups.idx)):
                entry["mat_batch"] = groups
                return len(groups)
            raw = groups.mats[groups.idx].tobytes()
            entry["mat_vbo"].bind()
            entry["mat_vbo"].allocate(raw, len(raw))
            entry["gpu_matrix_bytes"] = len(raw)
            entry["mat_vbo"].release()
            entry["mat_sig"] = (sig, groups.idx)
            entry["mat_batch"] = groups
            return len(groups)
        if epoch_of is not None:
            sig = (epoch_of(), tuple(map(id, groups)))
        else:
            sig = tuple((id(g), tuple(g.xform.data())) for g in groups)
        if entry["mat_sig"] != sig:
            self._inst_matrix_serial = getattr(self, "_inst_matrix_serial", 0) + 1
            import numpy as np
            raw = np.asarray([list(g.xform.data()) for g in groups],
                             dtype=np.float32).tobytes()
            entry["mat_vbo"].bind()
            entry["mat_vbo"].allocate(raw, len(raw))
            entry["gpu_matrix_bytes"] = len(raw)
            entry["mat_vbo"].release()
            entry["mat_sig"] = sig
            entry["mat_batch"] = None
        return len(groups)

    def _instanced_batches(self, groups):
        """``[(placements, fade)]`` for one prototype, split by the edit
        context.

        The instanced path is a SEPARATE draw call, so the fade split the
        chunk buffers carry means nothing to it: while you were inside a
        group, every instanced placement kept drawing at full strength and
        the rest of the model did not dim at all («hago doble clic y lo demás
        no se atenúa», Marco, 2026-09-11 — his plaza had just become a
        container, which made its children instanced). Hiding works the same
        way: `hide` drops the surroundings instead of washing them.
        """
        if self.scene.edit_group is None:
            return [(groups, 0.0)]
        fuera = [g for g in groups if self._draws_in_edit_context(g)]
        dentro = [g for g in groups if not self._draws_in_edit_context(g)]
        if not dentro:
            fuera = groups
        elif not fuera:
            dentro = groups
        if self._edit_rest_mode == "hide":
            return [(dentro, 0.0)]
        if self._edit_rest_mode != "fade":
            return [(groups, 0.0)]
        fade = float(getattr(self, "_edit_rest_fade_strength",
                             EDIT_REST_FADE))
        return [(fuera, fade), (dentro, 0.0)]

    def _inst_draw_chunks(self, by_proto):
        """Stable short runs of draw batches, preserving their exact order.

        A camera crossing one placement's bounds must not re-record every
        visible prototype. Unchanged runs retain their command tapes.
        """
        memo = getattr(self, "_inst_chunks_memo", None)
        if memo is not None and memo[0] is by_proto:
            return memo[1]
        old = {tuple(c): c for c in memo[1]} if memo is not None else {}
        chunks, current, bucket = [], {}, None
        for key, value in by_proto.items():
            next_bucket = getattr(value[2], "draw_bucket", 0)
            if current and (next_bucket != bucket or len(current) >= 16):
                chunks.append(current)
                current = {}
            current[key] = value
            bucket = next_bucket
        if current:
            chunks.append(current)
        for i, chunk in enumerate(chunks):
            prev = old.get(tuple(chunk))
            if prev is not None and all(prev[k][2] is v[2]
                                        for k, v in chunk.items()):
                chunks[i] = prev
        self._inst_chunks_memo = (by_proto, chunks)
        return chunks

    def _draw_instanced_chunked(self, by_proto, slot, draw, state_key):
        cache = getattr(self, slot + "_chunks", None) or {}
        kept = {}
        native = self._gl_replay()
        # Split edit-context batches can share and overwrite one matrix
        # buffer, so those must still draw immediately. Ordinary navigation
        # records changed chunks without drawing, then submits one joined
        # native tape instead of crossing Python/native hundreds of times.
        defer = native is not None and self.scene.edit_group is None
        serial = getattr(self, "_inst_matrix_serial", 0)
        fast = getattr(self, slot + "_fast", None)
        if (defer and fast is not None and fast[0] is by_proto
                and fast[1] == serial and fast[2] == state_key):
            _GlTape.submit_native(native, fast[3][1], fast[3][2], fast[3][3])
            return
        self._defer_inst_tapes = defer
        try:
            for chunk in self._inst_draw_chunks(by_proto):
                old = cache.get(id(chunk))
                setattr(self, slot, old[1] if old is not None and old[0] is chunk else None)
                draw(chunk)
                kept[id(chunk)] = (chunk, getattr(self, slot))
        finally:
            self._defer_inst_tapes = False
        if defer and kept:
            tapes = tuple(rec[3] for _chunk, rec in kept.values())
            merged = getattr(self, slot + "_merged", None)
            if merged is None or merged[0] != tapes:
                import numpy as np
                arrays = [tape.array_data() for tape in tapes]
                merged = (tapes, np.concatenate([a[0] for a in arrays]),
                          np.concatenate([a[1] for a in arrays]),
                          sum(tape.draw_count for tape in tapes))
                setattr(self, slot + "_merged", merged)
            _GlTape.submit_native(native, merged[1], merged[2], merged[3])
            setattr(self, slot + "_fast", (by_proto,
                    getattr(self, "_inst_matrix_serial", 0), state_key, merged))
        else:
            setattr(self, slot + "_fast", None)
        setattr(self, slot + "_chunks", kept)

    def _draw_instanced_faces_lean(self, by_proto, mode, extra) -> None:
        # Submit the preceding passes once, not once per command chunk.
        # Mesa's Intel driver otherwise varied coincident edge pixels
        # between frames when fed thousands of draws back to back (#158).
        self._gl.glFlush()
        self._draw_instanced_chunked(
            by_proto, "_lean_faces_rec",
            lambda chunk: self._draw_instanced_faces_chunk(chunk, mode, extra),
            (mode, tuple(effective_back_color(
                getattr(self.scene, "display_style", None), self.scene)),
             id(getattr(self, "_tex_cache", None)), id(self._program),
             self.edit_rest_fade, _NO_BACK_TINT))

    def _draw_instanced_faces_chunk(self, by_proto, mode, extra) -> None:
        """The instanced faces pass of the shaded and textured styles: the
        very draws of the general loop below, in the same order, but each
        piece of GL state set only when it changes — on a plant of 2 000
        prototypes the per-batch uniform/cull/bind churn WAS the frame
        (#158). The calls are recorded; the next frame with the same
        batches (an orbit mostly keeps them) replays the list as is."""
        prog = self._program
        gl = self._gl
        back_rgb = effective_back_color(
            self._view_style(), self.scene)
        key = (mode, tuple(back_rgb), id(getattr(self, "_tex_cache", None)),
               id(self._program), self.edit_rest_fade)
        rec = getattr(self, "_lean_faces_rec", None)
        if (rec is not None and rec[0] == key and rec[1] is by_proto
                and (rec[2] == getattr(self, "_inst_matrix_serial", 0)
                     or all(e.get("mat_batch") is b for e, b in rec[4]))):
            self._lean_faces_rec = (*rec[:2], getattr(self, "_inst_matrix_serial", 0),
                                    *rec[3:])
            if not getattr(self, "_defer_inst_tapes", False):
                rec[3].replay(self._gl_replay())
            return
        tape = _GlTape(execute=not getattr(self, "_defer_inst_tapes", False))
        pairs = []
        back_vec = QVector4D(back_rgb[0], back_rgb[1], back_rgb[2], 1.0)
        # What the general loop never relies on is unknown on entry (None:
        # the first use sets it); the cull and textures switches are off
        # between passes, as that loop assumes too.
        state = {"fade": None, "vcol": None, "back": False, "cull": False,
                 "front": None, "tex": 0, "shade": None, "cut": None}

        def need(key_, value, apply):
            if state[key_] != value:
                apply(value)
                state[key_] = value

        def set_vcol(v):
            tape.uniform_i(prog, self._loc_use_vcolor, v)

        def set_cull(on):
            if on:
                tape.gl(gl.glEnable, tape.ENABLE, GL_CULL_FACE)
                tape.gl(gl.glCullFace, tape.CULL, GL_FRONT)
            else:
                tape.gl(gl.glDisable, tape.DISABLE, GL_CULL_FACE)

        def set_shade(v):
            tape.uniform_f(prog, self._loc_shade, float(v))

        def set_tex(v):
            tape.uniform_i(prog, self._loc_use_tex, v)

        def set_cut(v):
            tape.uniform_i(prog, self._loc_hard_cutout, v)
            # The foliage mark for the ambient occlusion (see _set_cutout).
            tape.stencil_func(gl.glStencilFunc, GL_ALWAYS,
                              AO_FOLIAGE_BIT if v else 0, 0xFF)

        def set_front(mirrored):
            tape.gl(gl.glFrontFace, tape.FRONT, GL_CW if mirrored else GL_CCW)

        def set_fade(v):
            tape.uniform_f(prog, self._loc_fade, v)

        def set_color(r, g, b):
            # _set_color: the colour, and the back kept in step.
            v = QVector4D(r, g, b, 1.0)
            tape.uniform_4f(prog, self._loc_color, v)
            tape.uniform_4f(prog, self._loc_back_color, v)
        bound = None
        back_tint = not _NO_BACK_TINT
        cacheable = True
        draw = extra.glDrawArraysInstanced
        for mesh, paint, groups, mirrored in by_proto.values():
            entry = self._proto_view(self._ensure_proto_draw(mesh, paint),
                                     mirrored)
            need("front", mirrored, set_front)
            for lote, fade in self._instanced_batches(groups):
                if not lote:
                    continue
                n = self._update_inst_matrices(entry, lote)
                if entry.get("mat_batch") is not lote:
                    cacheable = False          # an edit-context split
                else:
                    pairs.append((entry, lote))
                need("fade", fade, set_fade)
                if entry["vcol_count"]:
                    need("cull", False, set_cull)
                    need("vcol", 1, set_vcol)
                    if not state["back"]:
                        tape.uniform_4f(prog, self._loc_back_color, back_vec)
                        state["back"] = True
                    bound = entry["vcol_vao"]
                    tape.bind_vao(bound)
                    tape.draw(draw, GL_TRIANGLES, 0, entry["vcol_count"], n)
                if entry["tex_runs"]:
                    need("cull", False, set_cull)
                    need("vcol", 0, set_vcol)
                    bound = entry["tex_vao"]
                    tape.bind_vao(bound)
                    if mode == "shaded":
                        for (path, shade), s0, cnt in entry["tex_runs"]:
                            r, g, b = self._texture_avg_color(path)
                            need("shade", float(shade), set_shade)
                            set_color(r, g, b)
                            state["back"] = False
                            tape.draw(draw, GL_TRIANGLES, s0, cnt, n)
                    else:
                        need("tex", 1, set_tex)
                        for (path, shade), s0, cnt in entry["tex_runs"]:
                            tex = self._get_texture(path)
                            if tex is None:
                                continue
                            need("shade", float(shade), set_shade)
                            need("cut", 1 if getattr(tex, "_cutout", False)
                                 else 0, set_cut)
                            tape.tex_bind(tex, 0)
                            tape.draw(draw, GL_TRIANGLES, s0, cnt, n)
                            tape.tex_release(tex, 0)
                        need("cut", 0, set_cut)
                        need("tex", 0, set_tex)
                    need("shade", 1.0, set_shade)
                if entry.get("dback_count") and back_tint:
                    need("cull", True, set_cull)
                    need("vcol", 0, set_vcol)
                    bound = entry["dback_vao"]
                    tape.bind_vao(bound)
                    tape.draw(draw, GL_TRIANGLES, 0, entry["dback_count"], n)
        if bound is not None:
            tape.release_vao(bound)
        need("cull", False, set_cull)
        if state["vcol"] is not None:
            need("vcol", 0, set_vcol)
        if state["shade"] is not None:
            need("shade", 1.0, set_shade)
        if state["front"] is not None:
            need("front", False, set_front)
        serial = getattr(self, "_inst_matrix_serial", 0)
        self._lean_faces_rec = (key, by_proto, serial, tape, pairs) if cacheable else None

    def _draw_instanced_faces(self, mode, style) -> None:
        if not _PERF:
            return Viewport._draw_instanced_faces_impl(self, mode, style)
        before = self._inst_perf['draws']
        start = _time_mod.perf_counter()
        try:
            return Viewport._draw_instanced_faces_impl(self, mode, style)
        finally:
            self._inst_perf['faces_draws'] += self._inst_perf['draws'] - before
            self._inst_perf['pass_ms'] += (_time_mod.perf_counter() - start) * 1000

    def _draw_instanced_faces_impl(self, mode, style) -> None:
        by_proto = self._gather_instanced()
        if not by_proto or mode == "wireframe":
            return
        extra = self.context().extraFunctions()
        self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
        self._gl.glPolygonOffset(1.0, 1.0)
        if mode == "xray":
            self._program.setUniformValue1f(self._loc_opacity,
                                            XRAY_FACE_OPACITY)
            self._gl.glDepthMask(GL_FALSE)
        # The prototype's tones were baked for ITS orientation (#482).
        self._program.setUniformValue(self._loc_reshade, 1)
        if mode in ("shaded", "textures") and _lean_instanced():
            self._draw_instanced_faces_lean(by_proto, mode, extra)
            by_proto = {}
        for mesh, paint, groups, mirrored in by_proto.values():
            entry = self._proto_view(self._ensure_proto_draw(mesh, paint),
                                     mirrored)
            self._front_face(mirrored)
            for lote, fade in self._instanced_batches(groups):
                if not lote:
                    continue
                n = self._update_inst_matrices(entry, lote)
                self._program.setUniformValue1f(self._loc_fade, fade)
                if entry["vcol_count"]:
                    if mode in ("hidden_line", "monochrome"):
                        self._program.setUniformValue(self._loc_use_vcolor, 0)
                        fr = style.front_color
                        self._set_color(fr[0], fr[1], fr[2], 1.0)
                        if mode == "hidden_line":
                            self._program.setUniformValue(
                                self._loc_back_color,
                                QVector4D(fr[0], fr[1], fr[2], 1.0))
                        else:
                            self._set_back_face_color()
                    else:
                        self._program.setUniformValue(self._loc_use_vcolor, 1)
                        self._set_back_face_color()
                    entry["vcol_vao"].bind()
                    Viewport._perf_instanced_draw(self, extra,
                        GL_TRIANGLES, 0, entry["vcol_count"], n)
                    entry["vcol_vao"].release()
                    self._program.setUniformValue(self._loc_use_vcolor, 0)
                if entry["tex_runs"]:
                    if mode in ("hidden_line", "monochrome"):
                        fr = style.front_color
                        self._set_color(fr[0], fr[1], fr[2], 1.0)
                        entry["tex_vao"].bind()
                        for _tk, s0, cnt in entry["tex_runs"]:
                            Viewport._perf_instanced_draw(self, extra, GL_TRIANGLES, s0, cnt, n)
                        entry["tex_vao"].release()
                    elif mode == "shaded":
                        entry["tex_vao"].bind()
                        for (path, shade), s0, cnt in entry["tex_runs"]:
                            r, g, b = self._texture_avg_color(path)
                            self._program.setUniformValue1f(
                                self._loc_shade, float(shade))
                            self._set_color(r, g, b, 1.0)
                            Viewport._perf_instanced_draw(self, extra, GL_TRIANGLES, s0, cnt, n)
                        entry["tex_vao"].release()
                        self._program.setUniformValue1f(self._loc_shade, 1.0)
                    else:            # textures / xray
                        self._program.setUniformValue(self._loc_use_tex, 1)
                        entry["tex_vao"].bind()
                        for (path, shade), s0, cnt in entry["tex_runs"]:
                            tex = self._get_texture(path)
                            if tex is None:
                                continue
                            self._program.setUniformValue1f(
                                self._loc_shade, float(shade))
                            self._set_cutout(getattr(tex, "_cutout", False))
                            tex.bind(0)
                            Viewport._perf_instanced_draw(self, extra, GL_TRIANGLES, s0, cnt, n)
                            tex.release(0)
                        entry["tex_vao"].release()
                        self._set_cutout(False)
                        self._program.setUniformValue1f(self._loc_shade, 1.0)
                        self._program.setUniformValue(self._loc_use_tex, 0)
                if (entry.get("dback_count") and not _NO_BACK_TINT
                        and mode in ("textures", "shaded")):
                    # The default back, as the consolidated pass draws it.
                    self._gl.glEnable(GL_CULL_FACE)
                    self._gl.glCullFace(GL_FRONT)
                    self._program.setUniformValue(self._loc_use_vcolor, 0)
                    entry["dback_vao"].bind()
                    Viewport._perf_instanced_draw(self, extra,
                        GL_TRIANGLES, 0, entry["dback_count"], n)
                    entry["dback_vao"].release()
                    self._gl.glDisable(GL_CULL_FACE)
        self._front_face(False)
        self._program.setUniformValue(self._loc_reshade, 0)
        self._program.setUniformValue1f(self._loc_fade, 0.0)
        if mode == "xray":
            self._program.setUniformValue1f(self._loc_opacity, 1.0)
            self._gl.glDepthMask(GL_TRUE)
        self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

    def _draw_instanced_raw(self) -> None:
        """Geometry-only instanced draw — the section-fill stencil pass."""
        by_proto = getattr(self, "_frame_instanced", None)
        if not by_proto:
            return
        extra = self.context().extraFunctions()
        for mesh, paint, groups, mirrored in by_proto.values():
            entry = self._proto_view(self._ensure_proto_draw(mesh, paint),
                                     mirrored)
            n = self._update_inst_matrices(entry, groups)
            if entry["vcol_count"]:
                entry["vcol_vao"].bind()
                Viewport._perf_instanced_draw(self, extra,
                    GL_TRIANGLES, 0, entry["vcol_count"], n)
                entry["vcol_vao"].release()
            if entry["tex_runs"]:
                entry["tex_vao"].bind()
                for _tk, s0, cnt in entry["tex_runs"]:
                    Viewport._perf_instanced_draw(self, extra, GL_TRIANGLES, s0, cnt, n)
                entry["tex_vao"].release()

    def _draw_back_edges(self, color, w: int, h: int) -> None:
        """Back Edges (K, issue #234): the edges a face hides, dashed, over
        the opaque model — where a bar or a frame member continues behind
        a face in a shop drawing. The faces already wrote their depth
        (pushed back by the polygon offset, so an edge lying ON a face is
        not «behind» it): what fails that test is drawn with GL_GREATER and
        a dash measured along each line, without writing depth."""
        dpr = max(1.0, float(self.devicePixelRatioF()))
        self._program.setUniformValue(self._loc_viewport_px,
                                      QVector2D(float(w), float(h)))
        self._program.setUniformValue1f(self._loc_dash_px, 4.0 * dpr)
        self._program.setUniformValue(self._loc_stipple, 4)
        self._gl.glDepthFunc(GL_GREATER)
        self._gl.glDepthMask(GL_FALSE)
        self._set_color(*color, 1.0)
        if self._edges_count > 0:
            self._edges_vao.bind()
            for _vs, _vc in getattr(self, "_frame_edge_spans",
                                    ((0, self._edges_count),)):
                self._gl.glDrawArrays(GL_LINES, _vs, _vc)
            self._edges_vao.release()
        self._draw_instanced_edges()
        self._gl.glDepthFunc(GL_LEQUAL)
        self._gl.glDepthMask(GL_TRUE)
        self._program.setUniformValue(self._loc_stipple, 0)

    def _xray_face_depth(self) -> None:
        """Write every face's depth and no colour — X-ray's translucent
        face pass leaves the depth buffer without them. Same polygon offset
        as the faces, so an edge lying ON a face still counts as in front."""
        self._gl.glColorMask(False, False, False, False)
        self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
        self._gl.glPolygonOffset(1.0, 1.0)
        if self._faces_count > 0:
            self._faces_vao.bind()
            self._gl.glDrawArrays(GL_TRIANGLES, 0, self._faces_count)
            self._faces_vao.release()
        if self._tex_faces_count > 0:
            self._tex_faces_vao.bind()
            self._gl.glDrawArrays(GL_TRIANGLES, 0, self._tex_faces_count)
            self._tex_faces_vao.release()
        self._draw_instanced_raw()
        self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
        self._gl.glColorMask(True, True, True, True)

    def _draw_instanced_edges_lean(self, by_proto, extra) -> None:
        self._draw_instanced_chunked(
            by_proto, "_lean_edges_rec",
            lambda chunk: self._draw_instanced_edges_chunk(chunk, extra),
            (id(self._program), self.edit_rest_fade))

    def _draw_instanced_edges_chunk(self, by_proto, extra) -> None:
        """:meth:`_draw_instanced_edges`'s draws, the fade set only when it
        changes, recorded and replayed while the batches stay (see
        :meth:`_draw_instanced_faces_lean`)."""
        prog = self._program
        key = (id(self._program), self.edit_rest_fade)
        rec = getattr(self, "_lean_edges_rec", None)
        if (rec is not None and rec[0] == key and rec[1] is by_proto
                and (rec[2] == getattr(self, "_inst_matrix_serial", 0)
                     or all(e.get("mat_batch") is b for e, b in rec[4]))):
            self._lean_edges_rec = (*rec[:2], getattr(self, "_inst_matrix_serial", 0),
                                    *rec[3:])
            if not getattr(self, "_defer_inst_tapes", False):
                rec[3].replay(self._gl_replay())
            return
        tape = _GlTape(execute=not getattr(self, "_defer_inst_tapes", False))
        pairs = []
        cacheable = True
        fade_now = None
        bound = None
        draw = extra.glDrawArraysInstanced
        for mesh, paint, groups, mirrored in by_proto.values():
            entry = self._proto_view(self._ensure_proto_draw(mesh, paint),
                                     mirrored)
            if not entry["edge_count"]:
                continue
            for lote, fade in self._instanced_batches(groups):
                if not lote:
                    continue
                n = self._update_inst_matrices(entry, lote)
                if entry.get("mat_batch") is not lote:
                    cacheable = False
                else:
                    pairs.append((entry, lote))
                if fade != fade_now:
                    tape.uniform_f(prog, self._loc_fade, fade)
                    fade_now = fade
                bound = entry["edges_vao"]
                tape.bind_vao(bound)
                tape.draw(draw, GL_LINES, 0, entry["edge_count"], n)
        if bound is not None:
            tape.release_vao(bound)
        tape.uniform_f(prog, self._loc_fade, 0.0)
        serial = getattr(self, "_inst_matrix_serial", 0)
        self._lean_edges_rec = (key, by_proto, serial, tape, pairs) if cacheable else None

    def _gl_replay(self):
        """core._native's GlReplay for this viewport's GL context (the
        experimental engine), or ``None``: tapes then replay through the
        PySide wrappers."""
        if not _LEAN_NATIVE_REPLAY:
            return None
        ctx = self.context()
        memo = getattr(self, "_gl_replay_memo", None)
        if memo is not None and memo[0] is ctx:
            return memo[1]
        replay = None
        from core import _native
        mod = _native.native
        if mod is not None and hasattr(mod, "GlReplay") and ctx is not None:
            try:
                addrs = [int(ctx.getProcAddress(n) or 0) for n in _TAPE_FUNCS]
                if all(_callable_proc(a) for a in addrs):
                    replay = mod.GlReplay(addrs)
            except Exception:  # noqa: BLE001 — a missing entry: Python
                replay = None
        self._gl_replay_memo = (ctx, replay)
        return replay

    def _draw_instanced_edges(self) -> None:
        if not _PERF:
            return Viewport._draw_instanced_edges_impl(self)
        before = self._inst_perf['draws']
        start = _time_mod.perf_counter()
        try:
            return Viewport._draw_instanced_edges_impl(self)
        finally:
            self._inst_perf['edges_draws'] += self._inst_perf['draws'] - before
            self._inst_perf['pass_ms'] += (_time_mod.perf_counter() - start) * 1000

    def _draw_instanced_edges_impl(self) -> None:
        by_proto = getattr(self, "_frame_instanced", None)
        if not by_proto:
            return
        extra = self.context().extraFunctions()
        if _lean_instanced():
            self._draw_instanced_edges_lean(by_proto, extra)
            return
        for mesh, paint, groups, mirrored in by_proto.values():
            entry = self._proto_view(self._ensure_proto_draw(mesh, paint),
                                     mirrored)
            if not entry["edge_count"]:
                continue
            for lote, fade in self._instanced_batches(groups):
                if not lote:
                    continue
                n = self._update_inst_matrices(entry, lote)
                self._program.setUniformValue1f(self._loc_fade, fade)
                entry["edges_vao"].bind()
                Viewport._perf_instanced_draw(self, extra, GL_LINES, 0,
                                            entry["edge_count"], n)
                entry["edges_vao"].release()
        self._program.setUniformValue1f(self._loc_fade, 0.0)

    def _draw_image_planes(self) -> None:
        """Reference images, as textured quads under the model.

        Depth test on. An opaque image writes depth pushed back by a polygon
        offset, so a line or face drawn exactly ON it still wins the depth
        test (tracing works, no z-fighting) and whatever lies behind it stays
        hidden; a faded image writes none, so the model shows through.

        Shading is pinned to 1.0: a reference scan has to read at its true
        tones, not dimmed by the face lighting.
        """
        images = getattr(self.scene, "image_planes", None)
        if not images:
            return
        import struct
        vao = getattr(self, "_img_vao", None)
        if vao is None:
            self._img_vao, self._img_vbo = self._create_dynamic_uv()
            vao = self._img_vao
        drawn = []
        for im in images:
            if not self.scene.entity_visible(im):
                continue
            tex = self._get_texture(im.path)
            if tex is None:
                continue
            drawn.append((im, tex))
        # An image is never part of a group: inside one it is always the
        # surroundings, and greys (or hides) with the rest of the model —
        # it stayed in full colour on top of a grey model.
        rest = self._edit_rest_fade(None)
        if not drawn or rest is None:
            return
        self._program.setUniformValue1f(self._loc_fade, rest)
        self._program.setUniformValue(self._loc_use_tex, 1)
        self._program.setUniformValue(self._loc_use_vcolor, 0)
        self._program.setUniformValue1f(self._loc_shade, 1.0)
        # The reference layers before us may leave the foliage cutout on; a
        # scan with soft edges would come out with its alpha chopped.
        self._program.setUniformValue(self._loc_hard_cutout, 0)
        self._gl.glDepthMask(GL_FALSE)
        # Blending is the frame's default state (paintGL enables it up
        # front for cutouts, glass and the shadow catcher) — a faded image
        # simply relies on it. This pass used to switch blending OFF after a
        # translucent image and never back on: every later blended pass ran
        # opaque, and the shadow catcher (translucent black, alpha 0 where
        # lit) then WROTE alpha 0 over the ground — a transparent hole the
        # size of the model's shadow square, white on screen. Only on
        # frames that reused the cached shadow map (orbit, zoom): a rebuild
        # re-enables blending on its way out, which hid the bug behind
        # every edit (Marco, 2026-09-14: «puse sombras con el tif y orbité»).
        self._gl.glEnable(GL_BLEND)
        _scene_blend(self._gl)
        for im, tex in drawn:
            c = im.corners()
            # Two triangles, UV (0,0) at ``origin``: _get_texture uploads the
            # image mirrored (bottom-up V), so v grows with the plane's "up"
            # axis and the picture stands upright.
            quad = ((c[0], 0.0, 0.0), (c[1], 1.0, 0.0), (c[2], 1.0, 1.0),
                    (c[0], 0.0, 0.0), (c[2], 1.0, 1.0), (c[3], 0.0, 1.0))
            raw = b"".join(struct.pack("<5f", v.x(), v.y(), v.z(), u, w)
                           for v, u, w in quad)
            opacity = max(0.0, min(1.0, float(getattr(im, "opacity", 1.0))))
            self._program.setUniformValue1f(self._loc_opacity, opacity)
            # An OPAQUE image does write depth — pushed back by a polygon
            # offset larger than the faces' (1, 1), so what is drawn ON it
            # (lines, and faces traced over it) still wins, while what lies
            # BEHIND it stays hidden. Without depth, a prism pushed down
            # from a triangle traced on a photo painted its sides over the
            # photo from above the ground (a user's video, 25-09). A faded
            # image keeps writing none: the model behind must show through.
            opaque = opacity >= 0.999
            self._gl.glDepthMask(GL_TRUE if opaque else GL_FALSE)
            if opaque:
                self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
                self._gl.glPolygonOffset(4.0, 8.0)
            self._img_vbo.bind()
            self._img_vbo.allocate(raw, len(raw))
            self._img_vbo.release()
            vao.bind()
            tex.bind(0)
            self._gl.glDrawArrays(GL_TRIANGLES, 0, 6)
            tex.release(0)
            vao.release()
            if opaque:
                self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
        self._program.setUniformValue1f(self._loc_opacity, 1.0)
        self._program.setUniformValue1f(self._loc_fade, 0.0)
        self._gl.glDepthMask(GL_TRUE)
        self._program.setUniformValue(self._loc_use_tex, 0)

    def _get_texture(self, path: str):
        """GL texture for an image ``path``, cached on the viewport. Returns the
        :class:`QOpenGLTexture` (Repeat wrap, linear+mipmap) or ``None`` if the
        image can't be loaded."""
        cache = getattr(self, "_tex_cache", None)
        if cache is None:
            cache = self._tex_cache = {}
        if path in cache:
            return cache[path]
        decoded = _decoded_texture(path)
        tex = None
        if decoded is not None:
            img, cutout = decoded
            tex = QOpenGLTexture(img)           # already mirrored
            tex.setWrapMode(QOpenGLTexture.Repeat)
            tex.setMinificationFilter(QOpenGLTexture.LinearMipMapLinear)
            tex.setMagnificationFilter(QOpenGLTexture.Linear)
            if cutout:
                tex.setMinificationFilter(QOpenGLTexture.Linear)
                tex._cutout = True
        cache[path] = tex
        return tex

    def _draw_sky(self, mvp) -> None:
        """Sky above the horizon and ground below, each a vertical gradient
        (style tones hazed toward the horizon — see ``_sky_gradient``), with
        a subtle horizon line where they meet. Per-vertex colours; GL
        interpolates per pixel. The class constants ``_SKY_RGB`` /
        ``_GROUND_RGB`` are the historical defaults, kept as fallbacks."""
        style = getattr(self, "_frame_style", None)
        sky_rgb = tuple(getattr(style, "sky_color", None)
                        or self._SKY_RGB)[:3]
        ground_rgb = tuple(getattr(style, "ground_color", None)
                           or self._GROUND_RGB)[:3]
        hy_raw = self._horizon_ndc_y(mvp)
        hy = max(-1.0, min(1.0, hy_raw))
        c_sky0, c_sky1, c_gnd0, c_gnd1, c_line = self._sky_gradient(
            sky_rgb, ground_rgb, hy_raw)
        self._program.setUniformValue(self._loc_mvp, QMatrix4x4())  # identity/NDC
        self._gl.glDisable(GL_DEPTH_TEST)
        self._gl.glDepthMask(GL_FALSE)

        def gquad(y0, y1, c0, c1):
            data = array("f", [-1, y0, 0, *c0, 1, y0, 0, *c0, 1, y1, 0, *c1,
                               -1, y0, 0, *c0, 1, y1, 0, *c1, -1, y1, 0, *c1])
            self._sky_grad_vbo.bind()
            self._sky_grad_vbo.allocate(data.tobytes(), len(data) * 4)
            self._sky_grad_vbo.release()
            self._gl.glDrawArrays(GL_TRIANGLES, 0, 6)

        self._sky_grad_vao.bind()
        self._program.setUniformValue(self._loc_use_vcolor, 1)
        if hy < 1.0:
            gquad(hy, 1.0, c_sky0, c_sky1)
        if hy > -1.0:
            gquad(-1.0, hy, c_gnd1, c_gnd0)
        self._program.setUniformValue(self._loc_use_vcolor, 0)
        self._sky_grad_vao.release()
        # A subtle horizon line where sky meets ground, in a tone
        # derived from the two haze colours so custom skies keep it legible.
        if -1.0 < hy < 1.0:
            line = array("f", [-1.0, hy, 0.0, 1.0, hy, 0.0])
            self._sky_vao.bind()
            self._sky_vbo.bind()
            self._sky_vbo.allocate(line.tobytes(), len(line) * 4)
            self._sky_vbo.release()
            self._set_color(*c_line, 1.0)
            self._gl.glDrawArrays(GL_LINES, 0, 2)
            self._sky_vao.release()
        self._gl.glEnable(GL_DEPTH_TEST)
        self._gl.glDepthMask(GL_TRUE)

    def _ensure_shadow_map(self, mvp=None):
        """Build or reuse the packed-depth shadow map. Camera-independent:
        keyed by geometry version + sun + cut + bounds, so orbit and zoom
        REUSE it — only an edit, the clock or the section rebuilds.

        Once the view has settled (``mvp`` the same as when
        ``_settle_shadow_fit`` measured what it shows), a second map of the
        same size spread over only that returns instead (#958): a plaza of
        100 m seen from a bench goes from 5 cm to millimetres per texel.
        The moment the camera moves the whole-model map is back."""
        d = self._sun_dir_now()
        self._frame_sun = d
        if d is None:
            self._shadow_vp = None
            return None
        lo, hi = self.scene.bounds()
        if lo is None:
            self._shadow_vp = None
            return None
        clip = getattr(self, "_clip_vec", None)
        key = (self.scene.version, id(self.scene.mesh),
               getattr(self, "_preview_epoch", 0),
               tuple(round(v, 4) for v in d),
               None if clip is None else (round(clip.x(), 4),
                                          round(clip.y(), 4),
                                          round(clip.z(), 4),
                                          round(clip.w(), 4)),
               round(lo.x(), 2), round(lo.y(), 2), round(lo.z(), 2),
               round(hi.x(), 2), round(hi.y(), 2), round(hi.z(), 2))
        # The light matrix is a pure function of the sun and the bounds —
        # all of them in the key — so deriving it every frame costs two
        # matrix builds and keeps it in step with the map. It must NOT hang
        # off the cache-hit branch: a pass with the sun OFF in between
        # clears it while leaving the map and its key intact (a sheet
        # exports plans without shadows beside a 3D with them, and the live
        # viewport does the same on any Shadows off→on with no edit between)
        # — the next hit then pushed None into the uniform and the render
        # died with «setUniformValue(15, None)». Marco, 2026-09-17,
        # exporting the sheets of Plaza Yanque to PDF.
        vp = self._light_vp(d, lo, hi)
        fine = self._fine_shadow_map(d, lo, hi, key, mvp)
        if fine is not None:
            return fine
        if self._shadow_fbo is not None and self._shadow_key == key:
            self._shadow_vp = vp
            return self._shadow_fbo
        if self._shadow_fbo is None:
            fmt = QOpenGLFramebufferObjectFormat()
            fmt.setAttachment(QOpenGLFramebufferObject.CombinedDepthStencil)
            size = self._shadow_size
            self._shadow_fbo = QOpenGLFramebufferObject(size, size, fmt)
            while not self._shadow_fbo.isValid() and size > self._SHADOW_MAP_SIZE:
                size //= 2                   # a driver short of memory
                self._shadow_fbo = QOpenGLFramebufferObject(size, size, fmt)
        self._render_shadow_map(vp)
        self._shadow_key = key
        self._shadow_vp = vp
        return self._shadow_fbo

    def _fine_shadow_map(self, d, lo, hi, key, mvp):
        """The fitted map for this frame (#958), built on first use; None
        while the view moves, in an export, or with nothing measured yet."""
        fit = getattr(self, "_shadow_fit", None)
        if (fit is None or mvp is None or self._export_size is not None
                or fit[0] != (tuple(mvp.data()), key)):
            return None
        fkey = (key, fit[1])
        vp = self._light_vp(d, lo, hi, fit[1])
        fbo = getattr(self, "_fine_shadow_fbo", None)
        if fbo is not None and getattr(self, "_fine_shadow_key", None) == fkey:
            self._shadow_vp = vp
            return fbo
        if fbo is None:
            fmt = QOpenGLFramebufferObjectFormat()
            fmt.setAttachment(QOpenGLFramebufferObject.CombinedDepthStencil)
            fbo = self._fine_shadow_fbo = QOpenGLFramebufferObject(
                self._SHADOW_MAP_SIZE, self._SHADOW_MAP_SIZE, fmt)
            if not fbo.isValid():
                self._fine_shadow_fbo = None
                return None
        self._render_shadow_map(vp, fbo)
        self._fine_shadow_key = fkey
        self._shadow_vp = vp
        return fbo

    def _settle_shadow_fit(self, mvp, w, h) -> None:
        """After a frame drawn with the whole-model map: once the view has
        stayed put ``SHADOW_SETTLE_MS`` (no button held), read which
        surfaces it shows from its depth and remember the square they span
        in the sun's view plane; the next frame draws the fitted map over
        it (#958). Any camera move, edit or clock change starts over."""
        if self._export_size is not None or self._frame_shadow is None:
            return
        d = getattr(self, "_frame_sun", None)
        lo, hi = self.scene.bounds()
        if d is None or lo is None:
            return
        state = (tuple(mvp.data()), self._shadow_key)
        fit = getattr(self, "_shadow_fit", None)
        if fit is not None and fit[0] == state:
            return                                  # already fitted
        if getattr(self, "_shadow_settle_state", None) != state:
            self._shadow_settle_state = state
            self._shadow_settle_due = False
            timer = getattr(self, "_shadow_settle_timer", None)
            if timer is None:
                timer = self._shadow_settle_timer = QTimer(self)
                timer.setSingleShot(True)
                timer.timeout.connect(self._shadow_settled)
            timer.start(self.SHADOW_SETTLE_MS)
            return
        from PySide6.QtWidgets import QApplication
        if (not getattr(self, "_shadow_settle_due", False)
                or QApplication.mouseButtons() != Qt.NoButton):
            return
        self._shadow_settle_due = False
        rect = self._visible_light_square(mvp, w, h, d, lo, hi)
        self._scene_fbo.bind()
        self._gl.glViewport(0, 0, w, h)
        if rect is not None:
            self._shadow_fit = (state, rect)
            self.update()

    def _shadow_settled(self) -> None:
        self._shadow_settle_due = True
        self.update()

    def _visible_light_square(self, mvp, w, h, d, lo, hi):
        """The square, in the sun's view plane, that the surfaces this frame
        shows span — from its depth, every 8th pixel — with a margin; None
        when the sky fills the view or it would not be smaller than the
        whole-model map."""
        import numpy as np
        depth = self._read_scene_depth(w, h)
        if depth is None:
            return None
        step = 8
        z = depth[::step, ::step]
        iy, ix = np.nonzero(z < 1.0)
        if ix.size == 0:
            return None
        zz = z[iy, ix].astype(np.float64)
        nx = (ix * step + 0.5) / w * 2.0 - 1.0
        ny = (iy * step + 0.5) / h * 2.0 - 1.0
        inv, ok = mvp.inverted()
        if not ok:
            return None
        view, r = self._light_view(d, lo, hi)
        m = np.array((view * inv).data(), dtype=np.float64).reshape(4, 4).T
        clip = np.stack([nx, ny, zz * 2.0 - 1.0, np.ones_like(nx)])
        p = m @ clip
        px, py = p[0] / p[3], p[1] / p[3]
        x0, x1, y0, y1 = px.min(), px.max(), py.min(), py.max()
        # Margin: the gaps between sampled pixels, and the PCF kernel.
        half = max(x1 - x0, y1 - y0) * 0.5 * 1.08 + 0.05
        if half >= r * 0.9:
            return None                 # the whole model: nothing to gain
        return (round(float(x0 + x1) * 0.5, 4), round(float(y0 + y1) * 0.5, 4),
                round(float(half), 4))

    def _render_shadow_map(self, light_vp, fbo=None) -> None:
        """Depth-from-the-sun pass: every caster — the consolidated face
        VBOs plus ALL instanced components (no camera culling here: an
        off-screen tree still throws its shadow into the frame) — drawn
        with the depth program into the packed-depth FBO. The active
        section cut applies, so severed geometry does not cast. The caller
        restores the scene FBO/program bindings this pass borrows."""
        fbo = self._shadow_fbo if fbo is None else fbo
        size = fbo.width()
        fbo.bind()
        self._gl.glViewport(0, 0, size, size)
        self._gl.glClearColor(1.0, 1.0, 1.0, 1.0)
        self._gl.glClearDepthf(1.0)
        self._gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
        self._gl.glEnable(GL_DEPTH_TEST)
        self._gl.glDepthFunc(GL_LEQUAL)
        self._gl.glDepthMask(GL_TRUE)
        self._gl.glDisable(GL_BLEND)
        prog = self._depth_program
        prog.bind()
        prog.setUniformValue(self._loc_d_mvp, light_vp)
        prog.setUniformValue(self._loc_d_use_tex, 0)
        clip = getattr(self, "_clip_vec", None)
        if clip is not None:
            prog.setUniformValue(self._loc_d_clip_plane, clip)
            prog.setUniformValue(self._loc_d_clip_enable, 1)
            self._gl.glEnable(GL_CLIP_DISTANCE0)
        else:
            prog.setUniformValue(self._loc_d_clip_enable, 0)
        # Identity on the instanced-matrix generic attributes for the
        # non-instanced draws (same reset the main pass does per frame).
        for i, row in enumerate(((1.0, 0.0, 0.0, 0.0), (0.0, 1.0, 0.0, 0.0),
                                 (0.0, 0.0, 1.0, 0.0),
                                 (0.0, 0.0, 0.0, 1.0))):
            self._gl.glVertexAttrib4f(3 + i, *row)
        if self._faces_count > 0 or self._tcol_runs:
            self._faces_vao.bind()
            if self._faces_count > 0:
                self._gl.glDrawArrays(GL_TRIANGLES, 0, self._faces_count)
            # The .skp translucency rule: under 70 % opacity a material
            # casts NO shadow (glass lets the sun through); at or above it,
            # it casts like a solid.
            for a, _fc, start, count in self._tcol_runs:
                if a >= self._SHADOW_OPACITY_MIN:
                    self._gl.glDrawArrays(GL_TRIANGLES, start, count)
            self._faces_vao.release()
        if self._tex_runs or self._ttex_runs:
            # Textured faces cast with their ALPHA CUTOUT: a chain-link
            # fence throws its weave, a leaf card its silhouette (texture
            # unavailable = solid fallback). Translucent runs follow the
            # 70 % rule above.
            self._tex_faces_vao.bind()
            prog.setUniformValue(self._loc_d_tex, 0)

            def _cast_run(path, start, count):
                tex = self._get_texture(path)
                if tex is not None:
                    prog.setUniformValue(self._loc_d_use_tex, 1)
                    tex.bind(0)
                    self._gl.glDrawArrays(GL_TRIANGLES, start, count)
                    tex.release(0)
                else:
                    prog.setUniformValue(self._loc_d_use_tex, 0)
                    self._gl.glDrawArrays(GL_TRIANGLES, start, count)

            for (path, _shade), start, count in self._tex_runs:
                _cast_run(path, start, count)
            for (path, _shade), a, _fc, start, count in self._ttex_runs:
                if a >= self._SHADOW_OPACITY_MIN:
                    _cast_run(path, start, count)
            prog.setUniformValue(self._loc_d_use_tex, 0)
            self._tex_faces_vao.release()
        saved = getattr(self, "_frame_planes", None)
        self._frame_planes = None      # gather WITHOUT camera culling
        try:
            by_proto = self._gather_instanced()
        finally:
            self._frame_planes = saved
        extra = self.context().extraFunctions()
        for mesh, paint, groups, mirrored in by_proto.values():
            entry = self._proto_view(self._ensure_proto_draw(mesh, paint),
                                     mirrored)
            n = self._update_inst_matrices(entry, groups)
            prog.bind()                # entry building binds the main program
            if entry["vcol_count"]:
                entry["vcol_vao"].bind()
                extra.glDrawArraysInstanced(
                    GL_TRIANGLES, 0, entry["vcol_count"], n)
                entry["vcol_vao"].release()
            if entry["tex_runs"]:
                # Alpha-cutout casting for instances too — the hedge's
                # leaves dapple instead of throwing solid slabs.
                entry["tex_vao"].bind()
                prog.setUniformValue(self._loc_d_tex, 0)
                for (path, _shade), s0, cnt in entry["tex_runs"]:
                    tex = self._get_texture(path)
                    if tex is not None:
                        prog.setUniformValue(self._loc_d_use_tex, 1)
                        tex.bind(0)
                        extra.glDrawArraysInstanced(GL_TRIANGLES, s0, cnt, n)
                        tex.release(0)
                    else:
                        prog.setUniformValue(self._loc_d_use_tex, 0)
                        extra.glDrawArraysInstanced(GL_TRIANGLES, s0, cnt, n)
                prog.setUniformValue(self._loc_d_use_tex, 0)
                entry["tex_vao"].release()
        # Billboard people cast their SILHOUETTE, as a quad turned toward
        # the sun — a camera-facing caster would swing its shadow while the
        # camera orbits (face-me "shadows face sun"). Vector-art
        # and mesh face-me stay non-casting (documented deferral).
        sun = getattr(self, "_frame_sun", None)
        bbs = [g for g in self._placements()
               if getattr(g, "billboard", False) is True
               and self.scene.entity_visible(g)]
        if sun is not None and bbs:
            face = QVector3D(sun[0], sun[1], 0.0)
            data = array("f")
            draws = []
            uvs = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
            for g in bbs:
                quad = self._billboard_quad(g, face_dir=face)
                if quad is None or not quad[1]:
                    continue
                corners, path = quad
                start = len(data) // 5
                for idx in (0, 1, 2, 0, 2, 3):
                    c = corners[idx]
                    u, v = uvs[idx]
                    data.extend([c.x(), c.y(), c.z(), u, v])
                draws.append((path, start))
            if draws:
                raw = data.tobytes()
                self._shadow_bb_vbo.bind()
                self._shadow_bb_vbo.allocate(raw, len(raw))
                self._shadow_bb_vbo.release()
                prog.bind()
                prog.setUniformValue(self._loc_d_use_tex, 1)
                prog.setUniformValue(self._loc_d_tex, 0)
                self._shadow_bb_vao.bind()
                for path, start in draws:
                    tex = self._get_texture(path)
                    if tex is None:
                        continue
                    tex.bind(0)
                    self._gl.glDrawArrays(GL_TRIANGLES, start, 6)
                    tex.release(0)
                self._shadow_bb_vao.release()
                prog.setUniformValue(self._loc_d_use_tex, 0)
        if clip is not None:
            self._gl.glDisable(GL_CLIP_DISTANCE0)
        prog.release()
        fbo.release()
        self._gl.glEnable(GL_BLEND)

    def _draw_shadow_ground(self, style) -> None:
        """The on-ground shadow catcher: a world plane just under z=0 that
        draws ONLY the shadow — translucent black where the sun is blocked,
        fully transparent where it is not, so the plane itself has no
        visible colour or edges (on-ground shadows). Skipped
        over the map, terrain and photo mesh, which bring their own ground;
        depth writes OFF so below-ground geometry still draws over it and a
        real floor AT z=0 never z-fights. Not drawn either when the
        camera looks at it from below the ground (#1301): a shadow cast
        on the ground is not seen from underneath."""
        if (self._base_map_showing() or self._terrain_showing()
                or self._photo_showing()):
            return
        z = -0.005
        if self.camera.sees_from_below(z):
            return
        lo, hi = self.scene.bounds()
        if lo is None:
            return
        cx = (lo.x() + hi.x()) / 2.0
        cy = (lo.y() + hi.y()) / 2.0
        r = max(hi.x() - lo.x(), hi.y() - lo.y(), 20.0) * 1.5
        x0, x1, y0, y1 = cx - r, cx + r, cy - r, cy + r
        data = array("f", [x0, y0, z, x1, y0, z, x1, y1, z,
                           x0, y0, z, x1, y1, z, x0, y1, z])
        self._ground_vbo.bind()
        self._ground_vbo.allocate(data.tobytes(), len(data) * 4)
        self._ground_vbo.release()
        self._program.setUniformValue(self._loc_shadow_enable, 1)
        self._program.setUniformValue(self._loc_shadow_overlay, 1)
        self._gl.glDepthMask(GL_FALSE)
        self._ground_vao.bind()
        self._gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        self._ground_vao.release()
        self._gl.glDepthMask(GL_TRUE)
        self._program.setUniformValue(self._loc_shadow_overlay, 0)

    def _render_tiles(self) -> None:
        if self._terrain_showing():
            return  # the 3D terrain replaces the flat map
        layer = getattr(self.scene, "tile_layer", None)
        datum = getattr(self.scene, "georef", None)
        if layer is None or datum is None or not getattr(layer, "visible", False):
            return
        try:
            runs = self._ensure_tile_geometry(layer, datum)
        except Exception:
            return
        if not runs:
            return
        # Budget new texture uploads this frame (see _tile_texture).
        self._tex_budget = self._TEX_PER_FRAME
        self._tex_deferred = False
        self._program.setUniformValue(self._loc_use_tex, 1)
        # A faded map (opacity < 1) blends over the background so the
        # model and its lines read on top of the imagery.
        opacity = max(0.0, min(1.0, float(getattr(layer, "opacity", 1.0))))
        self._program.setUniformValue1f(self._loc_opacity, opacity)
        self._gl.glDepthMask(GL_FALSE)
        self._tile_quad_vao.bind()
        for (x, y, start) in runs:
            tex = self._tile_texture(layer, x, y)
            if tex is None:
                continue
            tex.bind(0)
            self._gl.glDrawArrays(GL_TRIANGLES, start, 6)
            tex.release(0)
        self._tile_quad_vao.release()
        self._gl.glDepthMask(GL_TRUE)
        self._program.setUniformValue1f(self._loc_opacity, 1.0)
        self._program.setUniformValue(self._loc_use_tex, 0)
        if self._tex_deferred:            # more tiles to upload — schedule a frame
            self.update()

    # ---- Hidden things shown (View ▸ Hidden Objects / Geometry) ------------
    def _sync_hidden_ghosts(self) -> bool:
        """Scratch VBOs of the hidden things the view shows — the hidden
        OBJECTS' chunk arrays and the hidden FACES / EDGES of the mesh being
        edited, its soft edges among them — rebuilt only when the
        placements or the mesh change.
        Returns False when there is nothing to draw."""
        sc = self.scene
        objects = bool(getattr(sc, "show_hidden_objects", False))
        geometry = bool(getattr(sc, "show_hidden_geometry", False))
        if not objects and not geometry:
            self._ghost_key = None
            return False
        key = (self._placements_epoch(), _cache_ver(self), id(sc.mesh),
               objects, geometry)
        if getattr(self, "_ghost_key", None) == key:
            return bool(self._ghost_counts[0] or self._ghost_counts[1]
                        or self._ghost_tex_runs)
        if getattr(self, "_ghost_edges_vao", None) is None:
            self._ghost_edges_vao, self._ghost_edges_vbo = self._create_dynamic()
            self._ghost_vcol_vao, self._ghost_vcol_vbo = \
                self._create_dynamic_vcol()
            self._ghost_tex_vao, self._ghost_tex_vbo = self._create_dynamic_uv()
            self._program.bind()
        edge_parts: list = []
        vcol_parts: list = []
        tex_parts: dict = {}
        if objects:
            for g in self._placements():
                if not getattr(g, "hidden", False) \
                        or getattr(g, "billboard", False):
                    continue
                if not sc._layer_state(g)[0]:
                    continue                  # a hidden LAYER stays hidden
                ch = self._group_chunk(g)
                if ch["edges"]:
                    edge_parts.append(ch["edges"])
                if ch["vcol"]:
                    vcol_parts.append(ch["vcol"])
                for path, raw in ch["by_texture"].items():
                    tex_parts.setdefault(path, []).append(raw)
        if geometry:
            data = array("f")
            for f in sc.mesh.faces:
                if not f.attrs.get("hidden") or not sc._layer_state(f)[0]:
                    continue
                color = f.attrs.get("color") or self.DEFAULT_FACE_COLOR
                r, gg, b = self._shaded_color(tuple(color[:3]),
                                              self._normal_of(f))
                for t0, t1, t2 in self._tris_of(f):
                    data.extend([t0.x(), t0.y(), t0.z(), r, gg, b,
                                 t1.x(), t1.y(), t1.z(), r, gg, b,
                                 t2.x(), t2.y(), t2.z(), r, gg, b])
            if data:
                vcol_parts.append(data.tobytes())
        raw = b"".join(edge_parts)
        self._ghost_edges_vbo.bind()
        self._ghost_edges_vbo.allocate(raw or b"\0" * 24, max(len(raw), 24))
        self._ghost_edges_vbo.release()
        n_edges = len(raw) // 12
        raw = b"".join(vcol_parts)
        self._ghost_vcol_vbo.bind()
        self._ghost_vcol_vbo.allocate(raw or b"\0" * 24, max(len(raw), 24))
        self._ghost_vcol_vbo.release()
        n_vcol = len(raw) // 24
        runs: list = []
        blobs: list = []
        off = 0
        for path, parts in tex_parts.items():
            blob = b"".join(parts)
            runs.append((path, off // 20, len(blob) // 20))
            blobs.append(blob)
            off += len(blob)
        raw = b"".join(blobs)
        self._ghost_tex_vbo.bind()
        self._ghost_tex_vbo.allocate(raw or b"\0" * 24, max(len(raw), 24))
        self._ghost_tex_vbo.release()
        self._ghost_tex_runs = runs
        self._ghost_counts = (n_vcol, n_edges)
        self._ghost_key = key
        return bool(n_vcol or n_edges or runs)

    def _hidden_edge_draw(self, mesh, edge_kind="hidden"):
        """Local overlay data, keyed by the mesh bake rather than the document.

        A selection must never materialize a deferred imported mesh merely
        to read its edges; the renderer already has its arrays.
        """
        from core.lazy_mesh import plan_of
        import numpy as np
        cache = getattr(self, "_hidden_edge_draws", None)
        if cache is None:
            cache = self._hidden_edge_draws = {}
        base = self._proto_base_chunk(mesh)
        key = (base["uid"], base.get("rev", 0),
               tuple((ly.key, _effective_layer_visible(self.scene, ly)) for ly in self.scene.layers))
        cache_id = (id(mesh), edge_kind)
        entry = cache.get(cache_id)
        if entry is not None and entry["mesh"] is mesh and entry["key"] == key:
            return entry
        if entry is not None:
            self._destroy_proto_draw_entry(entry)
        if edge_kind == "visible":
            data = base["edges"]
        else:
            plan = plan_of(mesh)
            if plan is not None:
                mask = plan.soft | plan.hidden
                if any(layer is not None for layer in plan.layer):
                    from types import SimpleNamespace
                    visible = {layer: self.scene._layer_state(
                        SimpleNamespace(layer=layer))[0] for layer in set(plan.layer)}
                    mask = mask & np.fromiter((visible[l] for l in plan.layer), bool)
                data = plan.pos[plan.edges[mask]].astype(np.float32).tobytes()
            else:
                raw = array("f")
                for e in mesh.edges:
                    if (e.soft or e.hidden) and self.scene._layer_state(e)[0]:
                        raw.extend((*e.a.toTuple(), *e.b.toTuple()))
                data = raw.tobytes()
        entry = {"mesh": mesh, "key": key, "count": len(data) // 12,
                 "mat_sig": None}
        if data:
            vao, vbo = self._create_dynamic()
            self._program.bind()
            vbo.bind()
            vbo.allocate(data, len(data))
            vbo.release()
            entry.update(plain_vao=vao, edges_vbo=vbo)
            inst = QOpenGLVertexArrayObject(self)
            inst.create()
            inst.bind()
            vbo.bind()
            self._program.enableAttributeArray(self._loc_pos)
            self._program.setAttributeBuffer(self._loc_pos, GL_FLOAT, 0, 3, 12)
            vbo.release()
            matrices = QOpenGLBuffer(QOpenGLBuffer.VertexBuffer)
            matrices.create()
            matrices.bind()
            matrices.allocate(64)
            extra = self.context().extraFunctions()
            for i, loc in enumerate(self._loc_inst):
                self._program.enableAttributeArray(loc)
                self._program.setAttributeBuffer(loc, GL_FLOAT, i * 16, 4, 64)
                extra.glVertexAttribDivisor(loc, 1)
            matrices.release()
            inst.release()
            entry.update(inst_vao=inst, mat_vbo=matrices)
        cache[cache_id] = entry
        return entry

    def _draw_hidden_geometry_edges(self, mvp, w, h):
        """Reveal soft/hidden edges, including nested instances, without
        changing the solid shading or drawing occluded back edges."""
        if (not self.scene.show_hidden_geometry
                or getattr(self, "_nav_fast_feedback", False)):
            return
        self._program.bind()
        self._set_section_clip(True)
        self._program.setUniformValue(self._loc_viewport_px,
                                      QVector2D(float(w), float(h)))
        self._program.setUniformValue1f(
            self._loc_dash_px, 3.0 * max(1.0, float(self.devicePixelRatioF())))
        self._program.setUniformValue(self._loc_stipple, 4)
        ec = self._effective_style().edge_color
        self._set_color(*ec[:3], 1.0)
        self._gl.glDepthMask(GL_FALSE)
        extra = self.context().extraFunctions()
        try:
            for mesh, paint, groups, mirrored in getattr(
                    self, "_frame_instanced", {}).values():
                entry = self._hidden_edge_draw(mesh)
                if not entry["count"]:
                    continue
                for batch, fade in self._instanced_batches(groups):
                    if not batch:
                        continue
                    n = self._update_inst_matrices(entry, batch)
                    self._program.setUniformValue1f(self._loc_fade, fade)
                    entry["inst_vao"].bind()
                    extra.glDrawArraysInstanced(GL_LINES, 0, entry["count"], n)
                    entry["inst_vao"].release()
            # Non-instanced placements and the active editing mesh use the
            # same local buffer, with their world transform in the MVP.
            for g in [None] + self._static_draw_groups(self._placements()):
                if g is None:
                    mesh, xf, fade = self.scene.mesh, None, 0.0
                else:
                    if id(g) in (getattr(self, "_preview_groups", None) or ()):
                        continue
                    fade = self._edit_rest_fade(g)
                    if fade is None:
                        continue
                    bounds = (self._placement_bbox(g) if g.xform is not None
                              else self._group_chunk(g).get("bbox"))
                    if (self._frame_planes is not None and bounds is not None
                            and not self._aabb_visible(
                                self._frame_planes, bounds[0], bounds[1])):
                        continue
                    mesh, xf = g.mesh, g.xform
                entry = self._hidden_edge_draw(mesh)
                if not entry["count"]:
                    continue
                self._program.setUniformValue(self._loc_mvp,
                                              mvp * xf if xf is not None else mvp)
                self._program.setUniformValue1f(self._loc_fade, fade)
                entry["plain_vao"].bind()
                self._gl.glDrawArrays(GL_LINES, 0, entry["count"])
                entry["plain_vao"].release()
        finally:
            self._program.setUniformValue(self._loc_mvp, mvp)
            self._program.setUniformValue1f(self._loc_fade, 0.0)
            self._program.setUniformValue(self._loc_stipple, 0)
            self._gl.glDepthMask(GL_TRUE)

    def _draw_hidden_ghosts(self, mvp) -> None:
        """View ▸ Hidden Objects / Hidden Geometry: what Hide put
        away is drawn as a see-through grid (faces) and dotted lines
        (edges) — its own pass over scratch VBOs, so the normal passes and
        their caches never learn about it. Depth-tested, so it sits where
        it is in the model and still lets everything show through the
        weave; and selectable through the pick index, which is the road
        back to Unhide ▸ Selected."""
        if not self._sync_hidden_ghosts():
            return
        n_vcol, n_edges = self._ghost_counts
        self._program.bind()
        self._program.setUniformValue(self._loc_mvp, mvp)
        self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
        self._gl.glPolygonOffset(1.0, 1.0)
        self._program.setUniformValue(self._loc_stipple, 1)
        if n_vcol:
            self._program.setUniformValue(self._loc_use_vcolor, 1)
            self._ghost_vcol_vao.bind()
            self._gl.glDrawArrays(GL_TRIANGLES, 0, n_vcol)
            self._ghost_vcol_vao.release()
            self._program.setUniformValue(self._loc_use_vcolor, 0)
        if self._ghost_tex_runs:
            # A textured face reads as a plain grey weave: the grid is the
            # message, not the picture.
            self._set_color(0.62, 0.62, 0.62, 1.0)
            self._program.setUniformValue(self._loc_back_color,
                                          QVector4D(0.62, 0.62, 0.62, 1.0))
            self._ghost_tex_vao.bind()
            for _path, start, count in self._ghost_tex_runs:
                self._gl.glDrawArrays(GL_TRIANGLES, start, count)
            self._ghost_tex_vao.release()
            self._set_back_face_color()
        self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
        if n_edges:
            self._program.setUniformValue(self._loc_stipple, 2)
            ec = self._effective_style().edge_color
            self._set_color(ec[0], ec[1], ec[2], 1.0)
            self._ghost_edges_vao.bind()
            self._gl.glDrawArrays(GL_LINES, 0, n_edges)
            self._ghost_edges_vao.release()
        self._program.setUniformValue(self._loc_stipple, 0)

    def _upload_vbo(self, vbo, slot: str, parts, empty: int = 24) -> int:
        """Put ``b"".join(parts)`` in ``vbo``, re-sending only the parts
        that actually changed. Returns the total byte length.

        Every scene-version bump re-uploaded each buffer whole, so an edit
        inside one group re-sent the entire model each drag frame — on
        piscina.igz that is ~25 MB of edges plus more of faces, and the
        telemetry read ``sync=197ms`` against ``faces=11ms edges=7ms``: the
        frame was the upload, not the drawing.

        A part equal to the one the GPU holds at the same offset is not
        sent. Chunk buffers are cached objects, so that comparison is an
        identity hit for every group that did not change; the freshly built
        loose block falls back to a bytes compare, which is memcmp and still
        an order of magnitude under the transfer it saves. The buffer is
        over-allocated so a growing tail usually fits without a reallocation
        (which would discard everything the GPU held)."""
        sizes = [len(p) for p in parts]
        total = sum(sizes)
        prev = self._vbo_parts.get(slot)
        cap = prev[1] if prev is not None else 0
        vbo.bind()
        if prev is not None and total <= cap and len(prev) > 2:
            # Each part is sent unless the GPU holds it already, at the very
            # same offset: an object moved, deleted (its zeros) or pasted
            # into the room left for it sends its own part, not the tail
            # after it. Joining the whole buffer to send its tail was most
            # of putting an object down on the plant of #158 (~0.6 s).
            held = {}
            off = 0
            for b, n in zip(prev[0], prev[2]):
                if n:
                    held[off] = b
                off += n
            off = 0
            for a, n in zip(parts, sizes):
                if n:
                    b = held.get(off)
                    if not (b is not None and (a is b or (len(b) == n
                                                          and a == b))):
                        vbo.write(off, a, n)
                        self._gpu_epoch = getattr(self, "_gpu_epoch", 0) + 1
                off += n
        else:
            if total > cap:
                cap = max(int(total * 1.25) + 4096, empty)
                vbo.allocate(cap)      # reserve; a realloc keeps no contents
            if total:
                vbo.write(0, b"".join(parts), total)
            self._gpu_epoch = getattr(self, "_gpu_epoch", 0) + 1
        vbo.release()
        self._vbo_parts[slot] = (list(parts), cap, sizes)
        return total

    def _sync_edges(self) -> None:
        # The style's Front color bakes into the default faces without a
        # version bump (a style edit is not a document edit): a new tint
        # re-runs the sync, whose content key sees it through the
        # placements epoch (``_layer_look_sig``).
        tint = (None if getattr(self, "plano_style", None) is not None
                else _front_tint(getattr(self, "style_override", None)
                                 or self._view_style()))
        if _cache_ver(self) == self._edges_version \
                and tint == getattr(self, "_edges_front_tint", None):
            return
        self._edges_front_tint = tint
        _st0 = _time_mod.perf_counter() if _PERF else 0.0

        # The scene changed: purge hover/selection references to entities that
        # no longer exist, or deleted geometry keeps ghost-rendering (blue
        # hover / orange selection) until the mouse moves or a click replaces
        # the selection. Renderer-level guarantee — holds no matter which
        # command forgot to discard. Membership is checked per candidate
        # (identity scans in C) instead of materialising 300k-entity sets;
        # a huge candidate list falls back to the set walk.
        hover = self._hover_entity
        cands = [hover] if isinstance(hover, (Edge, Face)) else []
        cands += [s for s in self.scene.selection
                  if isinstance(s, (Edge, Face))]
        if cands:
            if len(cands) > 64:
                # Selected/hovered Edge/Face refs only ever point into the
                # active mesh (loose, or the group being edited) — the old
                # render_edges()/render_faces() sets walked EVERY group's
                # entities (~283k) per edit frame after an explode.
                alive_e: set = set(map(element_identity, self.scene.mesh.edges))
                alive_f: set = set(map(element_identity, self.scene.mesh.faces))

                def alive(ent):
                    return ent.key in (alive_e if isinstance(ent, Edge)
                                       else alive_f)
            else:
                # A mesh the experimental engine has not built yet holds no
                # Edge/Face object, so nothing can be alive in it: asking it
                # built every lazy group of the villa (~0.5 s) on the first
                # click inside an opened group (issue #1006). The active
                # mesh — where a selection inside a group lives — first.
                act = self.scene.mesh
                gmeshes = [g.mesh for g in self._placements()
                           if self.scene.entity_visible(g)
                           and not getattr(g, "billboard", False)
                           and "_deferred" not in g.mesh.__dict__]
                gmeshes.sort(key=lambda gm: gm is not act)

                # A hidden face/edge the hidden view shows is selectable,
                # and a selection it took must not be dropped here: a
                # single click on a ghost face looked like nothing (only
                # a double click, which selected its visible edges too,
                # seemed to work — Marco, 2026-09-21, issue #53).
                def shown(ent):
                    return (self.scene.entity_visible(ent)
                            or self.scene.entity_selectable(ent))

                def alive(ent):
                    if isinstance(ent, Edge):
                        if ent in self.scene.loose_mesh.edges:
                            return shown(ent)
                        return any(ent in gm.edges for gm in gmeshes)
                    if ent in self.scene.loose_mesh.faces:
                        return shown(ent)
                    return any(ent in gm.faces for gm in gmeshes)

            if isinstance(hover, (Edge, Face)) and not alive(hover):
                self._hover_entity = None
            for s in [s for s in self.scene.selection
                      if isinstance(s, (Edge, Face)) and not alive(s)]:
                self.scene.selection.discard(s)

        # Only the selection moved (a click: a view-only bump) — the edge,
        # face and outline buffers hold the model as it is. On the plant of
        # #158 rebuilding them took ~0.4 s of every click.
        ckey = self._sync_content_key()
        if ckey == getattr(self, "_edges_content_key", None):
            self._sync_selection_buffers()
            self._edges_version = _cache_ver(self)
            self._announce_after_paint(self._edges_version)
            return

        # Inside a group, the surroundings can fade out or leave the frame
        # altogether (Model Info ▸ Components). Hidden means gone
        # from the VBOs, not merely skipped at draw time — that is what makes
        # it the fast mode on a heavy import.
        hide_rest = self._rest_is_hidden()
        # Vertex offset where the edited group's own block starts; everything
        # before it is context. ``None`` = no subject (not editing, or the
        # context is hidden and there is nothing to separate it from).
        self._edit_split_e = None
        self._edit_split_f = None
        # Hard edges: loose ones rebuilt fresh, group ones from cached chunks
        # (composition mirrors scene.render_edges()).
        all_loose = array("f")
        if not hide_rest:
            for e in self.scene.loose_mesh.edges:
                if (not self.scene.entity_visible(e)
                        or getattr(e, "soft", False)
                        or getattr(e, "hidden", False)):
                    continue  # hidden layer / curve segment (reads smooth)
                all_loose.extend([
                    e.a.x(), e.a.y(), e.a.z(),
                    e.b.x(), e.b.y(), e.b.z(),
                ])
        edge_parts = [all_loose.tobytes()]
        # Per-chunk draw spans (vertices) for frustum culling: the loose
        # block always draws (bbox None); each group's block carries its
        # chunk's world AABB.
        loose_n = len(all_loose) // 3
        edge_spans = [(None, 0, loose_n)]
        pv = getattr(self, "_preview_groups", None) or ()
        # The group half (every placement's hard edges, in draw order) is
        # keyed on the placements epoch: a loose edit — every frame of a
        # Move drag — re-uses it whole instead of walking 1 300 chunks.
        epoch_of = getattr(self, "_placements_epoch", None)     # stub VPs in tests
        gkey = (epoch_of() if epoch_of is not None else _cache_ver(self),
                hide_rest, tuple(sorted(pv)))
        # The group being edited goes LAST, so the surroundings occupy a
        # contiguous head the fade pass can draw in one go (and, since
        # nothing outside the group can change while you are in it, a head
        # that stays put in the buffer). The order is also what the passes
        # below walk.
        placements = self._placements()
        draw_groups = [g for g in placements
                       if not self._draws_in_edit_context(g)]
        if not hide_rest:
            draw_groups = [g for g in placements
                           if self._draws_in_edit_context(g)] + draw_groups
        # While a group is open, only the SURROUNDINGS are cached on the
        # epoch. The epoch leaves the open mesh's serial out on purpose (it
        # is the loose mesh, and counting it re-keyed every cache on every
        # drag frame), so a cached subject never saw its own edits: what
        # you drew inside a group or component was not drawn until you
        # left it (Rafael, revision 4: «no se ven las líneas»). The subject
        # is re-read each sync instead — its chunk is cached per group and
        # revalidated by the mesh serial, so that is a lookup, not a build.
        editing = self.scene.edit_group is not None
        # Outside a group, the static half is laid out in slots that keep
        # the places of objects taken away (``_static_slots``).
        slots = None
        if not editing and not hide_rest and epoch_of is not None:
            slots = self._static_slots(self._static_draw_groups(draw_groups))
            gkey += (tuple(i for i, (_sl, gone) in enumerate(slots)
                           if gone),)
        else:
            self._slot_layout = None
        head = ([g for g in draw_groups if self._draws_in_edit_context(g)]
                if editing else draw_groups)
        subject = ([g for g in draw_groups
                    if not self._draws_in_edit_context(g)]
                   if editing else [])

        def _collect(groups, rel):
            parts: list = []
            spans: list = []                  # (bbox, start RELATIVE, n)
            for g in groups:
                if (self.scene.entity_visible(g)
                        and not getattr(g, "billboard", False)
                        and not self._instanced_eligible(g)):
                    ch = self._group_chunk(g)
                    n = len(ch["edges"]) // 12
                    if id(g) in pv:
                        # A mover keeps its place as zeros (see _pv_hole).
                        parts.append(_pv_hole(ch["edges"]))
                    else:
                        parts.append(ch["edges"])
                        spans.append((ch.get("bbox"), rel, n))
                    rel += n
            return parts, spans, rel

        def _collect_slots(slots):
            parts: list = []
            spans: list = []
            rel = 0
            for sl, gone in slots:
                if gone:
                    n = sl.sizes.get("e", 0)
                    if n:
                        parts.append(_zeros(n))
                        rel += n // 12
                    continue
                got, sp, rel2 = _collect((sl.g,), rel)
                sl.sizes["e"] = sum(len(p) for p in got)
                parts += got
                spans += sp
                rel = rel2
            return parts, spans, rel

        gcache = getattr(self, "_edge_groups_cache", None)
        if gcache is None or gcache[0] != gkey:
            head_parts, head_spans, head_n = (
                _collect_slots(slots) if slots is not None else _collect(
                    self._static_draw_groups(head), 0))
            gcache = self._edge_groups_cache = (gkey, head_parts, head_spans,
                                                head_n)
        _gk, head_parts, head_spans, head_n = gcache
        # Where the surroundings end and the subject begins. This used to
        # key on `g is scene.edit_group`, which never matched when the group
        # being edited was a nested placement (the list holds its proxy) —
        # and with no split, NOTHING faded.
        split_rel = head_n if editing else None
        subj_parts, subj_spans, _n = _collect(subject, head_n)
        group_parts = head_parts + subj_parts
        group_spans = head_spans + subj_spans
        edge_parts += group_parts
        edge_spans += [(bb, loose_n + start, n) for bb, start, n in group_spans]
        if self._edit_split_e is None and split_rel is not None:
            self._edit_split_e = loose_n + split_rel
        self._edge_spans = edge_spans
        self._edges_count = self._upload_vbo(
            self._edges_vbo, "edges", edge_parts) // 12

        self._sync_selection_buffers()

        # Faces: triangulate each face (fan when simple, hole-aware when the
        # face has been divided) into one VBO, but grouped by material colour
        # (attrs["color"], default cream) so each colour is a single draw call
        # with its own uniform. Loose faces rebuild fresh; untouched groups
        # contribute their cached chunk buffers (a 17k-face reference model
        # made every stroke pay a ~1 s re-triangulation otherwise).
        # The translucent and back-side runs go to the upload as their
        # chunk parts, not joined: a Move, a Delete or a Paste leaves most
        # of them as they were, and the upload passes those over by
        # identity — joining a run of glass again was ~0.1 s of each, on
        # the plant of #158. Small parts (a loose face each) are joined in
        # batches, or the upload would send them one by one.
        def pieces(parts):
            out: list = []
            small: list = []
            for p in parts:
                if len(p) >= 1 << 16:
                    if small:
                        out.append(b"".join(small))
                        small = []
                    out.append(p)
                else:
                    small.append(p)
            if small:
                out.append(b"".join(small))
            return out

        suppressed_faces = self._suppressed_faces
        vcol = array("f")            # loose faces, interleaved pos(3)+rgb(3)
        by_texture: dict = {}        # image path -> interleaved pos+uv array
        # A group with SUPPRESSED faces (a push preview hides the face being
        # extruded) cannot use its cached chunk, so its faces are bucketed one
        # by one — and that output must not land in the loose block at the
        # HEAD of the buffer. There it would be on the context side of the
        # fade split, washing out the very geometry being edited, and it would
        # break the stable prefix the incremental upload rides on. It goes to
        # its own block at the tail instead; ``sink`` is what bucket_face
        # writes through.
        subj_vcol = array("f")
        subj_by_texture: dict = {}
        # Default-back tint (pos-only triangles; see ``back_is_default``):
        # the loose block, then one part per group chunk, subject last.
        dback_loose = array("f")
        subj_dback = array("f")
        sink = {"vcol": vcol, "tex": by_texture, "dback": dback_loose}
        from core.materials import back_is_default
        group_texture: dict = {}     # chunk byte-parts per image path
        face_parts: list = []        # interleaved vcol byte chunks

        tcol_runs: dict = {}         # opacity -> [byte parts] (translucent colour)
        ttex_runs: dict = {}         # (path, opacity) -> [byte parts]
        back_vcol_parts: list = []   # back-side colour override triangles
        back_tex_runs: dict = {}     # (path, shade) -> [byte parts] (back side)
        back_tcol_runs: dict = {}    # opacity -> [parts] (translucent back)
        back_ttex_runs: dict = {}    # ((path, shade), op) -> [parts]
        fcull_vcol_parts: list = []  # front copies culled to the front side

        def bucket_back(face):
            # attrs["back"]: the face is painted DIFFERENTLY on its back side
            # (.skp two-sided paint). Emit an override copy that a culled
            # pass shows only from behind. Returns True when the back is
            # TRANSLUCENT — then the front copy must not show from behind
            # (it gets back-face culling), or the two sides would blend.
            back = face.attrs.get("back")
            if not isinstance(back, dict):
                return False
            kind, payload = self._bucket_back_face(face, back)
            if kind == "bvcol":
                back_vcol_parts.append(payload)
            elif kind == "btex":
                back_tex_runs.setdefault(payload[0], []).append(payload[1])
            elif kind == "btcol":
                back_tcol_runs.setdefault(payload[0], []).append(payload[1])
            elif kind == "bttex":
                back_ttex_runs.setdefault(payload[0], []).append(payload[1])
            return kind in ("btcol", "bttex")

        # Inside an open group its faces are the loose mesh, and the
        # group's own paint (issue #47) must still dress the default ones.
        from core.materials import effective_attrs as _eff_attrs
        ctx_paint = _paint_of(self, self.scene.edit_group)
        # Curved surfaces shade smooth (#131): the faces that can have a
        # smooth corner, and one smoother for the whole pass so faces that
        # share a vertex share its fan.
        from formats.meshexport import SmoothNormals
        curved_loose = (_curved_faces(self.scene.loose_mesh)
                        if not hide_rest else set())
        smoother = SmoothNormals()

        def bucket_face(face):
            if face in suppressed_faces:
                return
            attrs = _eff_attrs(face.attrs, ctx_paint)
            fcull = bucket_back(face)
            if back_is_default(attrs):
                db = sink["dback"]
                for t0, t1, t2 in self._tris_of(face):
                    db.extend([t0.x(), t0.y(), t0.z(),
                               t1.x(), t1.y(), t1.z(),
                               t2.x(), t2.y(), t2.z()])
            tex = attrs.get("texture")
            op = float(attrs.get("opacity", 1.0))
            if tex is not None and tex.get("path"):
                if op < 0.999:
                    tmp: dict = {}
                    self._append_textured_face(tmp, face, tex)
                    for pth, arr in tmp.items():
                        ttex_runs.setdefault((pth, round(op, 3), fcull),
                                             []).append(arr.tobytes())
                    return
                # (textured opaque fronts of glass-backed faces keep the
                # plain double-sided path — the opaque back pass covers
                # opaque backs, and this combination is vanishing rare)
                self._append_textured_face(sink["tex"], face, tex)
                return
            col = attrs.get("color")
            base = tuple(col) if col is not None else self.DEFAULT_FACE_COLOR
            # Bake a subtle diffuse shade from the face normal against a fixed
            # world light — the matte-model look. World-fixed, so
            # it doesn't change as you orbit. The shaded colour rides per
            # vertex, so the whole pass is ONE draw call.
            r, g, b = self._shaded_color(base, self._normal_of(face))
            buf = array("f")
            if face in curved_loose:
                tris = [tuple(t.toTuple() for t in tri)
                        for tri in self._tris_of(face)]
                buf.extend(_smooth_vcol(face, base, (r, g, b), smoother,
                                        tris, self._LIGHT))
            else:
                for t0, t1, t2 in self._tris_of(face):
                    buf.extend([
                        t0.x(), t0.y(), t0.z(), r, g, b,
                        t1.x(), t1.y(), t1.z(), r, g, b,
                        t2.x(), t2.y(), t2.z(), r, g, b,
                    ])
            if op < 0.999:
                tcol_runs.setdefault((round(op, 3), fcull),
                                     []).append(buf.tobytes())
            elif fcull:
                fcull_vcol_parts.append(buf.tobytes())
            else:
                sink["vcol"].extend(buf)

        if not hide_rest:
            for face in self.scene.loose_mesh.faces:
                if self.scene.entity_visible(face):
                    bucket_face(face)
        group_face_spans: list = []   # (bbox, start-within-groups, count)
        gface_start = 0
        dback_parts: list = []        # default-back tint, per group chunk
        dback_spans: list = []        # (bbox, start-within-groups, count)
        dback_start = 0
        self._edit_split_db = None
        subject_bucketed = False
        pv_faces = getattr(self, "_preview_groups", None) or ()
        # Outside a group the walk meets only the placements the instanced
        # pass leaves to it (in the same order: nothing it skips adds a
        # face), not every placement — 1 900 of 21 406 on the plant of
        # #158, whose full walk was ~0.3 s of every edit there.
        # Inside a group, the same for the surroundings: the subject (last,
        # walked whole) is where the fade split is recorded, and what the
        # static filter leaves out of the surroundings adds no face. The
        # walk over every placement was ~0.8 s of each edit inside a group
        # on that plant.
        walk = (self._static_draw_groups(draw_groups)
                if self.scene.edit_group is None
                else self._static_draw_groups(head) + subject)
        if slots is None:
            slots = [(_Slot(g), False) for g in walk]
        for sl, gone in slots:        # context first, edited group last
            if gone:
                # A gone object's place: zeros of what it took (see
                # ``_static_slots``), so what follows keeps its offset.
                sz = sl.sizes
                if sz.get("d"):
                    dback_parts.append(_zeros(sz["d"]))
                    dback_start += sz["d"] // 12
                # (one part per object, empty or not: as many as before)
                face_parts.append(_zeros(sz.get("f", 0)))
                gface_start += sz.get("f", 0) // 24
                for path, n in (sz.get("t") or {}).items():
                    group_texture.setdefault(path, []).append(
                        (_zeros(n), None, False))
                # Its share of the translucent and back-side runs too: the
                # runs keep their sizes, and their joins their parts.
                runs = sz.get("runs") or {}
                for name, dest in (("tcol", tcol_runs), ("ttex", ttex_runs),
                                   ("back_tex", back_tex_runs),
                                   ("back_tcol", back_tcol_runs),
                                   ("back_ttex", back_ttex_runs)):
                    for key, n in (runs.get(name) or {}).items():
                        dest.setdefault(key, []).append(_zeros(n))
                if runs.get("back_vcol"):
                    back_vcol_parts.append(_zeros(runs["back_vcol"]))
                if runs.get("fvcol"):
                    fcull_vcol_parts.append(_zeros(runs["fvcol"]))
                continue
            g = sl.g
            sl.sizes.update(f=0, d=0, t={}, runs={})
            if (self._edit_split_f is None
                    and self.scene.edit_group is not None
                    and not self._draws_in_edit_context(g)):
                # BEFORE the skips, on purpose. The subject may well draw by
                # another path — an instanced placement does — and if the
                # boundary is only recorded for chunks that land in THIS
                # buffer, entering such a group left the split unset and
                # nothing faded at all (Marco, 2026-09-11, second level).
                self._edit_split_f = gface_start   # made absolute below
                self._edit_split_db = dback_start  # same rule, same moment
            if (not self.scene.entity_visible(g)
                    or getattr(g, "billboard", False)
                    or self._instanced_eligible(g)):
                continue
            if id(g) in pv_faces:
                # A mover leaves the buffers as zeros of its own size, not
                # as a gap: everything after it keeps its offset, so picking
                # it up and putting it down re-send its part alone. Closing
                # the gap re-sent the whole tail — up to 0.4 GB of faces on
                # the plant of #158, ~0.3 s each way.
                chunk = self._group_chunk(g)
                if chunk.get("dback"):
                    dback_parts.append(_pv_hole(chunk["dback"]))
                    dback_start += len(chunk["dback"]) // 12
                face_parts.append(_pv_hole(chunk["vcol"]))
                gface_start += len(chunk["vcol"]) // 24
                for path, raw in chunk["by_texture"].items():
                    group_texture.setdefault(path, []).append(
                        (_pv_hole(raw), None, False))
                sl.sizes.update(
                    f=len(chunk["vcol"]), d=len(chunk.get("dback") or b""),
                    t={p: len(r) for p, r in chunk["by_texture"].items()})
                continue
            if suppressed_faces and any(f in suppressed_faces
                                        for f in g.mesh.faces):
                # Bucketed into the SUBJECT block at the tail, not the loose
                # head — see ``sink``. The chunk is unusable while a face of
                # it is hidden.
                sink["vcol"], sink["tex"] = subj_vcol, subj_by_texture
                sink["dback"] = subj_dback
                curved_loose.update(_curved_faces(g.mesh))
                for face in g.mesh.faces:
                    bucket_face(face)   # push/pull preview suppresses faces
                sink["vcol"], sink["tex"] = vcol, by_texture
                sink["dback"] = dback_loose
                subject_bucketed = True
                continue
            chunk = self._group_chunk(g)
            sl.sizes.update(
                f=len(chunk["vcol"]), d=len(chunk.get("dback") or b""),
                t={p: len(r) for p, r in chunk["by_texture"].items()},
                runs={**{name: {k: len(r) for k, r in
                                (chunk.get(name) or {}).items()}
                         for name in ("tcol", "ttex", "back_tex",
                                      "back_tcol", "back_ttex")},
                      "back_vcol": len(chunk.get("back_vcol") or b""),
                      "fvcol": len(chunk.get("fvcol") or b"")})
            if chunk.get("dback"):
                dback_parts.append(chunk["dback"])
                dback_spans.append((chunk.get("bbox"), dback_start,
                                    len(chunk["dback"]) // 12))
                dback_start += len(chunk["dback"]) // 12
            face_parts.append(chunk["vcol"])
            group_face_spans.append((chunk.get("bbox"), gface_start,
                                     len(chunk["vcol"]) // 24))
            gface_start += len(chunk["vcol"]) // 24
            # The SUBJECT of the edit, for the textured pass — which draws
            # in its own runs and decides the fade with this flag, not with
            # the positional split. Comparing by identity against
            # `scene.edit_group` misses a nested group, because the draw list
            # holds a proxy of it: Marco's whole plaza came out washed, the
            # group he was editing included, because every textured face was
            # marked as surroundings (2026-09-11).
            subj = not self._draws_in_edit_context(g)
            for path, raw in chunk["by_texture"].items():
                group_texture.setdefault(path, []).append(
                    (raw, chunk.get("bbox"), subj))
            for a, raw in chunk.get("tcol", {}).items():
                tcol_runs.setdefault(a, []).append(raw)
            for key, raw in chunk.get("ttex", {}).items():
                ttex_runs.setdefault(key, []).append(raw)
            if chunk.get("back_vcol"):
                back_vcol_parts.append(chunk["back_vcol"])
            for key, raw in chunk.get("back_tex", {}).items():
                back_tex_runs.setdefault(key, []).append(raw)
            for key, raw in chunk.get("back_tcol", {}).items():
                back_tcol_runs.setdefault(key, []).append(raw)
            for key, raw in chunk.get("back_ttex", {}).items():
                back_ttex_runs.setdefault(key, []).append(raw)
            if chunk.get("fvcol"):
                fcull_vcol_parts.append(chunk["fvcol"])

        # Kept as a part LIST (not one concatenated blob) so the upload can
        # tell which pieces changed; the trailing runs below append to it.
        subj_raw = subj_vcol.tobytes()
        all_face_parts = [vcol.tobytes()] + face_parts + [subj_raw]
        self._faces_count = sum(len(p) for p in all_face_parts) // 24
        # Loose faces sit at the front of the VBO and always draw; group spans
        # follow them, and a bucketed subject block closes the buffer.
        loose_n = len(vcol) // 6
        self._face_spans = ([(None, 0, loose_n)]
                            + [(bb, loose_n + s, n)
                               for bb, s, n in group_face_spans])
        if self._edit_split_f is not None:
            self._edit_split_f += loose_n   # was relative to the group block
        if subj_raw:
            subj_start = loose_n + gface_start
            self._face_spans.append((None, subj_start, len(subj_raw) // 24))
            if subject_bucketed and self.scene.edit_group is not None:
                # The bucketed block IS the subject, and it is last: the fade
                # split moves to its start so the context still fades.
                self._edit_split_f = subj_start
        # Room after the opaque batch, outside a group: what a Paste adds
        # lands in it, and the runs below keep their offsets — moved, they
        # were the whole tail of the buffer sent again (~0.2 s on the plant
        # of #158). Zeros, never drawn: the opaque pass stops before them.
        if slots is not None and getattr(self, "_slot_layout", None):
            main = self._faces_count * 24
            room = max(24 << 15, min(24 << 19, main // 64 // 24 * 24))
            at = getattr(self, "_face_room_at", None)
            if at is None or at < main or at - main > 2 * room:
                at = main + room
            self._face_room_at = at
            if at > main:
                all_face_parts.append(_zeros(at - main))
        else:
            self._face_room_at = None
        # Translucent colour runs live in the SAME VBO, after the opaque
        # batch; drawn in their own blended pass (depth-write off).
        self._tcol_runs = []
        start = sum(len(p) for p in all_face_parts) // 24
        for key in sorted(tcol_runs):
            a, fc = key
            run = pieces(tcol_runs[key])
            all_face_parts += run
            count = sum(map(len, run)) // 24
            self._tcol_runs.append((a, fc, start, count))
            start += count
        run = pieces(back_vcol_parts)
        self._back_vcol_run = (start, sum(map(len, run)) // 24)
        all_face_parts += run
        start += self._back_vcol_run[1]
        run = pieces(fcull_vcol_parts)
        self._fvcol_run = (start, sum(map(len, run)) // 24)
        all_face_parts += run
        start += self._fvcol_run[1]
        self._back_tcol_runs = []
        for a in sorted(back_tcol_runs):
            run = pieces(back_tcol_runs[a])
            all_face_parts += run
            count = sum(map(len, run)) // 24
            self._back_tcol_runs.append((a, start, count))
            start += count
        self._upload_vbo(self._faces_vbo, "faces", all_face_parts, empty=48)

        # Default-back tint buffer: loose head, group chunks, bucketed
        # subject tail — the same shape as the faces buffer, so the fade
        # split and the frustum spans read the same way.
        db_loose_raw = dback_loose.tobytes()
        db_loose_n = len(db_loose_raw) // 12
        db_subj_raw = subj_dback.tobytes()
        db_parts = [db_loose_raw] + dback_parts + [db_subj_raw]
        self._dback_spans = ([(None, 0, db_loose_n)]
                             + [(bb, db_loose_n + st, n)
                                for bb, st, n in dback_spans])
        if self._edit_split_db is not None:
            self._edit_split_db += db_loose_n
        if db_subj_raw:
            db_subj_start = db_loose_n + dback_start
            self._dback_spans.append((None, db_subj_start,
                                      len(db_subj_raw) // 12))
            if subject_bucketed and self.scene.edit_group is not None:
                self._edit_split_db = db_subj_start
        self._dback_count = self._upload_vbo(
            self._dback_vbo, "dback", db_parts) // 12

        # Textured faces: one interleaved (pos+uv) VBO, a run per image path.
        tex_parts = []
        self._tex_runs = []
        # per run: [(bbox, start, count, is_subject)] — the flag lets the fade
        # pass tell the edited group's textured faces from its surroundings,
        # which a single buffer offset cannot (runs interleave by image).
        self._tex_run_parts = []
        start = 0
        for key in dict.fromkeys(list(by_texture) + list(group_texture)
                                 + list(subj_by_texture)):
            run_start = start
            run_parts: list = []
            parts = ([(by_texture[key].tobytes(), None, False)]
                     if key in by_texture else [])
            parts += group_texture.get(key, [])
            # A bucketed subject's textured faces are the subject too, so the
            # fade pass must not wash them out with the surroundings.
            if key in subj_by_texture:
                parts.append((subj_by_texture[key].tobytes(), None, True))
            for raw, bb, subj in parts:
                tex_parts.append(raw)
                n = len(raw) // 20
                run_parts.append((bb, start, n, subj))
                start += n
            self._tex_runs.append((key, run_start, start - run_start))
            self._tex_run_parts.append(run_parts)
        # Where the OPAQUE textured block ends: the shadow depth pass draws
        # up to here, then adds only the translucent runs that .skp rules cast
        # (opacity ≥ 70 %) — glass must let the sun through.
        self._tex_opaque_count = start
        self._ttex_runs = []
        for full in sorted(ttex_runs):
            key, a, fc = full
            run = pieces(ttex_runs[full])
            tex_parts += run
            count = sum(map(len, run)) // 20
            self._ttex_runs.append((key, a, fc, start, count))
            start += count
        self._back_tex_runs = []
        for key in sorted(back_tex_runs):
            run = pieces(back_tex_runs[key])
            tex_parts += run
            count = sum(map(len, run)) // 20
            self._back_tex_runs.append((key, start, count))
            start += count
        self._back_ttex_runs = []
        for full in sorted(back_ttex_runs):
            key, a = full
            run = pieces(back_ttex_runs[full])
            tex_parts += run
            count = sum(map(len, run)) // 20
            self._back_ttex_runs.append((key, a, start, count))
            start += count
        self._tex_faces_count = self._upload_vbo(
            self._tex_faces_vbo, "tex", tex_parts, empty=40) // 20

        if _PERF:
            _plog("sync_edges", (_time_mod.perf_counter() - _st0) * 1000.0)
        self._edges_version = _cache_ver(self)
        self._edges_content_key = ckey
        self._announce_after_paint(self._edges_version)

    def _draw_selected_profiles(self, groups):
        """Reuse cached soft-edge data for the selected curved outlines."""
        if not self._effective_style().profiles:
            return
        eye = self.camera.eye()
        key = (self._placements_epoch(), tuple(map(id, groups)), eye.toTuple())
        if getattr(self, "_selected_profile_key", None) != key:
            import numpy as np
            parts = []
            planes = self._frame_planes
            e_np = np.array(eye.toTuple())
            for g in groups:
                if g.xform is not None:
                    raw = self._instance_silhouette(g, eye, planes)
                    if raw is not None:
                        parts.append(raw)
                        continue
                ch = self._group_chunk(g)
                if ch["soft_pts"] is None:
                    continue
                bb = ch.get("bbox")
                if bb is not None and planes is not None and not self._aabb_visible(planes, *bb):
                    continue
                s0 = np.einsum("ij,ij->i", ch["soft_n0"], ch["soft_c0"] - e_np)
                s1 = np.einsum("ij,ij->i", ch["soft_n1"], ch["soft_c1"] - e_np)
                mask = ch["soft_single"] | ((s0 < 0) != (s1 < 0))
                parts.append(ch["soft_pts"][mask].tobytes())
            if getattr(self, "_selected_profile_vao", None) is None:
                self._selected_profile_vao, self._selected_profile_vbo = self._create_dynamic()
                self._program.bind()
            raw = b"".join(parts)
            self._selected_profile_vbo.bind()
            self._selected_profile_vbo.allocate(raw or b"\0" * 24, max(24, len(raw)))
            self._selected_profile_vbo.release()
            self._selected_profile_count = len(raw) // 12
            self._selected_profile_key = key
        if self._selected_profile_count:
            self._selected_profile_vao.bind()
            self._gl.glDrawArrays(GL_LINES, 0, self._selected_profile_count)
            self._selected_profile_vao.release()

    def _draw_selected_group_edges(self, mvp, w, h):
        """Tint the selected occurrences, sharing local buffers, not meshes
        baked and copied into the selection VBO on every click."""
        if self._export_size is not None or self._nav_fast_feedback:
            return
        groups = self._selected_group_placements()
        if not groups:
            return
        by_mesh = {}
        loose = []
        for g in groups:
            bounds = (self._placement_bbox(g) if g.xform is not None
                      else self._group_chunk(g).get("bbox"))
            if (bounds is not None and self._frame_planes is not None
                    and not self._aabb_visible(self._frame_planes, *bounds)):
                continue
            if g.xform is None:
                loose.append(g)
            else:
                by_mesh.setdefault(g.mesh, []).append(g)
        extra = self.context().extraFunctions()
        self._program.bind()
        self._set_color(0.95, 0.45, 0.16, 1.0)
        self._gl.glDepthMask(GL_FALSE)
        kinds = ["visible"]
        if self.scene.show_hidden_geometry:
            kinds.append("selected_hidden")
        try:
            for kind in kinds:
                dashed = kind == "selected_hidden"
                self._program.setUniformValue(self._loc_stipple, 4 if dashed else 0)
                if dashed:
                    self._program.setUniformValue(self._loc_viewport_px,
                                                  QVector2D(float(w), float(h)))
                    self._program.setUniformValue1f(self._loc_dash_px,
                        3.0 * max(1.0, float(self.devicePixelRatioF())))
                for mesh, batch in by_mesh.items():
                    entry = self._hidden_edge_draw(mesh, kind)
                    if entry["count"]:
                        n = self._update_inst_matrices(entry, batch)
                        entry["inst_vao"].bind()
                        extra.glDrawArraysInstanced(GL_LINES, 0, entry["count"], n)
                        entry["inst_vao"].release()
                for g in loose:
                    entry = self._hidden_edge_draw(g.mesh, kind)
                    if entry["count"]:
                        entry["plain_vao"].bind()
                        self._gl.glDrawArrays(GL_LINES, 0, entry["count"])
                        entry["plain_vao"].release()
            self._program.setUniformValue(self._loc_stipple, 0)
            self._draw_selected_profiles(groups)
        finally:
            self._program.setUniformValue(self._loc_stipple, 0)
            self._gl.glDepthMask(GL_TRUE)

    def _sync_selection_buffers(self) -> None:
        """The selection's highlight buffers (``_sync_edges``)."""
        pv = getattr(self, "_preview_groups", None) or ()
        # The selection set is heterogeneous (edges, faces and/or whole
        # groups). A selected GROUP highlights via its cached chunk — walking
        # + re-triangulating a 100k-face imported group froze the app the
        # moment the user clicked it.
        sel_loose = array("f")
        sel_edge_parts = []
        for ent in self.scene.selection:
            if isinstance(ent, Edge):
                sel_loose.extend([ent.a.x(), ent.a.y(), ent.a.z(),
                                  ent.b.x(), ent.b.y(), ent.b.z()])
            elif isinstance(ent, Group):
                if getattr(ent, "billboard", False):
                    # A face-me billboard's mesh quad is NOT what's on screen
                    # (the drawn quad rotates to face the camera every frame)
                    # — tinting it painted a stray plane through the figure.
                    # Its selection cue is the per-frame outline drawn in
                    # _draw_billboard_outlines instead.
                    continue
                if id(ent) in pv:
                    # Mid Move/Rotate drag: the scratch preview draws the
                    # mover WITH its own orange cue at the cursor — tinting
                    # the chunk here left an orange GHOST at the origin
                    # until the commit (user report).
                    continue
                # Keep the box; mesh edges are tinted by the shared-buffer
                # selection pass without copying the assembly into this VBO.
                sel_edge_parts.append(_box_edges(*self._group_obb(ent)))
        self._selected_count = self._upload_vbo(
            self._selected_vbo, "sel_edges",
            [sel_loose.tobytes()] + sel_edge_parts) // 12

        sel_face_runs: dict = {}     # (front_dot, back_dot) -> array
        for ent in self.scene.selection:
            if isinstance(ent, Face):
                buf = sel_face_runs.setdefault(
                    self._selection_dot_colors(ent), array("f"))
                for t0, t1, t2 in ent.triangulate():
                    buf.extend([
                        t0.x(), t0.y(), t0.z(),
                        t1.x(), t1.y(), t1.z(),
                        t2.x(), t2.y(), t2.z(),
                    ])
        self._sel_faces_spans = []
        first = 0
        for (front, back), buf in sel_face_runs.items():
            n = len(buf) // 3
            self._sel_faces_spans.append((front, back, first, n))
            first += n
        self._sel_faces_count = self._upload_vbo(
            self._sel_faces_vbo, "sel_faces",
            [buf.tobytes() for buf in sel_face_runs.values()]) // 12

    def _draw_billboards(self) -> None:
        """Per-frame pass: each face-me billboard is a textured cutout quad
        turned toward the camera (2D people). Depth-tested, so it
        hides behind walls correctly; the shader discards transparent texels."""
        # The face-me figures are a handful among thousands of placements:
        # found once per change of the placements, not on every frame.
        bkey = self._placements_epoch()
        bmemo = getattr(self, "_billboard_groups", None)
        if bmemo is not None and bmemo[0] == bkey:
            groups = bmemo[1]
        else:
            groups = [g for g in self._placements()
                      if getattr(g, "billboard", False)
                      and self.scene.entity_visible(g)]
            self._billboard_groups = (bkey, groups)
        if not groups:
            return
        self._program.setUniformValue(self._loc_use_tex, 1)
        # Hard alpha cut for face-me figures: their mipmapped edge alpha
        # under the Bayer dither would stipple dots around the silhouette
        # (user report); scene cutouts keep the dither (distant fences).
        self._program.setUniformValue(self._loc_hard_cutout, 1)
        self._billboard_vao.bind()
        for g in groups:
            fade = self._edit_rest_fade(g)
            if fade is None:
                continue
            self._program.setUniformValue1f(self._loc_fade, fade)
            if getattr(g, "billboard", False) == "mesh":
                # Imported face-me silhouette: its REAL geometry (a cut
                # outline, not a rectangle) turns toward the camera.
                self._draw_faceme_mesh(g)
                continue
            quad = self._billboard_quad(g)
            if quad is None:
                continue
            corners, path = quad
            tex = self._get_texture(path)
            if tex is None:
                continue
            data = array("f")
            uvs = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
            for idx in (0, 1, 2, 0, 2, 3):
                c = corners[idx]
                u, v = uvs[idx]
                data.extend([c.x(), c.y(), c.z(), u, v])
            raw = data.tobytes()
            self._billboard_vbo.bind()
            self._billboard_vbo.allocate(raw, len(raw))
            self._billboard_vbo.release()
            tex.bind(0)
            self._gl.glDrawArrays(GL_TRIANGLES, 0, 6)
            if g in self.scene.selection:
                # Selection cue: repaint with the highlight colour untextured.
                pass
        self._billboard_vao.release()
        self._program.setUniformValue1f(self._loc_fade, 0.0)
        self._program.setUniformValue(self._loc_hard_cutout, 0)
        self._program.setUniformValue(self._loc_use_tex, 0)

    def _draw_billboard_outlines(self) -> None:
        """Selection cue for face-me billboards: an orange outline around the
        quad AS DRAWN this frame (it rotates toward the camera). Tinting the
        group's static mesh painted a stray plane through the figure —
        the usual convention shows a selected 2D person as an outlined box too."""
        sel = [g for g in self.scene.selection
               if isinstance(g, Group) and getattr(g, "billboard", False)
               and self.scene.entity_visible(g)]
        if not sel:
            return
        data = array("f")
        for g in sel:
            quad = self._billboard_quad(g)
            if quad is None:
                continue
            c = quad[0]
            for i, j in ((0, 1), (1, 2), (2, 3), (3, 0)):
                data.extend([c[i].x(), c[i].y(), c[i].z(),
                             c[j].x(), c[j].y(), c[j].z()])
        if not data:
            return
        raw = data.tobytes()
        self._bb_sel_vbo.bind()
        self._bb_sel_vbo.allocate(raw, len(raw))
        self._bb_sel_vbo.release()
        self._set_color(0.95, 0.45, 0.16, 1.0)  # selection orange
        self._bb_sel_vao.bind()
        self._gl.glDrawArrays(GL_LINES, 0, len(data) // 3)
        self._bb_sel_vao.release()

    def _draw_faceme_mesh(self, g) -> None:
        """Draw a mesh face-me billboard: rotate its geometry around the
        vertical axis through its anchor so its plane faces the camera NOW.
        Runs inside the billboard pass (textured, billboard VAO bound)."""
        base = self._faceme_base(g)
        if base is None:
            return
        arrays, colors, anchor, nh = base
        import numpy as np
        if len(anchor) < 3:                      # an older cached entry
            anchor = np.array([anchor[0], anchor[1], 0.0])
        # Preview transform (Move/Rotate drag): the figure follows the
        # cursor through THIS pass, camera-facing with its baked UVs —
        # see _billboard_quad for the simple-quad twin.
        ox = oy = oz = 0.0
        if id(g) in (getattr(self, "_preview_groups", None) or ()):
            m = getattr(self, "_preview_matrix", None)
            if m is not None:
                p = m.map(QVector3D(anchor[0], anchor[1], anchor[2]))
                ox, oy, oz = (p.x() - anchor[0], p.y() - anchor[1],
                              p.z() - anchor[2])
            else:
                off = getattr(self, "_preview_offset", None)
                if off is not None:
                    ox, oy, oz = off.x(), off.y(), off.z()
        fd = self._faceme_dir(QVector3D(anchor[0] + ox, anchor[1] + oy,
                                        anchor[2] + oz))
        d = np.array([fd.x(), fd.y()])
        ln = float(np.hypot(d[0], d[1]))
        d = nh if ln < 1e-9 else d / ln
        cos = float(nh[0] * d[0] + nh[1] * d[1])
        sin = float(nh[0] * d[1] - nh[1] * d[0])

        def _rotated(arr):
            a = arr.copy()
            x = a[:, 0] - anchor[0]
            y = a[:, 1] - anchor[1]
            a[:, 0] = anchor[0] + cos * x - sin * y + ox
            a[:, 1] = anchor[1] + sin * x + cos * y + oy
            if oz:
                a[:, 2] += oz
            return a

        for (path, _shade), arr in arrays.items():
            # billboards turn toward the camera: no fixed-normal shade
            tex = self._get_texture(path)
            if tex is None:
                continue
            a = _rotated(arr)
            raw = a.tobytes()
            self._billboard_vbo.bind()
            self._billboard_vbo.allocate(raw, len(raw))
            self._billboard_vbo.release()
            tex.bind(0)
            self._gl.glDrawArrays(GL_TRIANGLES, 0, len(a))
        if colors:
            # Solid-colour runs (vector-art face-me like Susan): same VAO,
            # texture sampling off, u_color per run; restore for the pass.
            self._program.setUniformValue(self._loc_use_tex, 0)
            for col, arr in colors.items():
                r, gc, b = col[0], col[1], col[2]
                self._program.setUniformValue(
                    self._loc_color, QVector4D(r, gc, b, 1.0))
                self._program.setUniformValue(
                    self._loc_back_color, QVector4D(r, gc, b, 1.0))
                a = _rotated(arr)
                raw = a.tobytes()
                self._billboard_vbo.bind()
                self._billboard_vbo.allocate(raw, len(raw))
                self._billboard_vbo.release()
                self._gl.glDrawArrays(GL_TRIANGLES, 0, len(a))
            self._program.setUniformValue(self._loc_use_tex, 1)

    @staticmethod
    def _hover_face_data(face: Face, placement=None) -> array:
        """The hover marking's triangles, in WORLD coordinates.

        A face inside a component is stored in its part's OWN coordinates;
        ``placement`` (from :meth:`pick_face_placement`) carries the matrix
        that puts it where it is drawn. Without it the marking sat at the
        part's unplaced, assembled position: metres away from a component
        that had been placed and exploded."""
        m = getattr(placement, "xform", None)
        data = array("f")
        for tri in face.triangulate():
            if m is not None:
                tri = [m.map(p) for p in tri]
            for p in tri:
                data.extend([p.x(), p.y(), p.z()])
        return data

    def _upload_hover_face(self, face: Face, placement=None) -> int:
        """Triangulate ``face`` into the hover-faces VBO. Returns vertex count."""
        data = self._hover_face_data(face, placement)
        self._hover_faces_vbo.bind()
        if data:
            raw = data.tobytes()
            self._hover_faces_vbo.allocate(raw, len(raw))
        else:
            self._hover_faces_vbo.allocate(24)
        self._hover_faces_vbo.release()
        return len(data) // 3

    def _sil_inputs(self, key, cam) -> tuple:
        """Everything the outline pass reads: the scene (``key``), the
        camera, the placements, the groups previewed and the edit context."""
        sc = self.scene
        return (key, cam, self._placements_epoch(),
                getattr(self, "_preview_epoch", 0),
                id(sc.edit_group), getattr(self, "_edit_rest_mode", None))

    def _upload_silhouette_edges(self) -> int:
        """Upload the *profile* edges into the silhouette VBO and return the
        vertex count. A soft (hidden) edge is drawn when it lies on the
        silhouette — its two faces straddle the view, one turned toward the
        camera and one away — so a curved surface shows its outline. A soft
        edge with a single face is always a profile (a boundary). View-dependent,
        called each frame."""
        # Throttle: the silhouette is view-dependent but re-deriving it at
        # most ~12×/s is visually indistinguishable, and at 100k soft edges
        # the NumPy pass still costs ~4 ms a frame during orbits.
        #
        # On a heavy model (outlines costing over 30 ms) they are kept while
        # the camera moves and re-derived once it stops, by a booked
        # repaint (issue #158). They are world-space segments: meanwhile
        # they stay on the geometry, only WHICH edges outline lags.
        import time as _time
        now = _time.monotonic()
        key = (_content_ver(self), id(self.scene.mesh))
        last = getattr(self, "_sil_last", None)
        wait = 0.08
        # While the camera is moving (it moved since the last frame) and
        # the outlines are expensive, keep them: re-deriving them mid-orbit
        # made a 300–440 ms hitch every second or so on the model of #158.
        # The booked repaint below redraws them once the camera stops.
        cam = self.camera.projection_matrix() * self.camera.view_matrix()
        cam = tuple(cam.data())
        moving = getattr(self, "_sil_cam", None) not in (None, cam)
        self._sil_cam = cam
        if moving:
            self._sil_cam_t = now
        # Nothing an outline depends on moved since the VBO was filled —
        # the scene (version), the camera, the groups being previewed — so
        # the VBO holds this very outline: no need to wait out the throttle
        # and derive it again. A drag beside a plant of 21 406 placements
        # re-derived them (~340 ms) on every move of the pointer (#158).
        same_view = self._sil_inputs(key, cam)
        if last is not None and len(last) > 3 and last[3] == same_view:
            return last[2]
        # «Stopped» means 600 ms without a camera change: a run of wheel
        # notches is a string of short pauses, and re-deriving after each
        # one blocked the next notch (live profile, #158).
        still = now - getattr(self, "_sil_cam_t", 0.0)
        if getattr(self, "_sil_cost", 0.0) > 0.03 and still < 0.6:
            wait = (now - last[1] if last else 0.0) + (0.6 - still)
        if last is not None and last[0] == key and now - last[1] < wait:
            if getattr(self, "_sil_refresh_booked", False) is False:
                from PySide6.QtCore import QTimer
                self._sil_refresh_booked = True

                def _refresh():
                    self._sil_refresh_booked = False
                    self.update()
                QTimer.singleShot(int((wait - (now - last[1])) * 1000) + 20,
                                  self, _refresh)
            return last[2]        # VBO still holds the last upload

        # Loose soft edges run the SAME vectorised view test as group
        # chunks: per-edge Python here froze orbits after an explode dumped
        # a group's curved surfaces into the loose mesh (38k soft edges =
        # ~600 ms EVERY frame, piscina.igz user report). The cache bakes
        # endpoints + face normals/centroids once per edit; each frame is
        # one einsum.
        import numpy as np
        cached = getattr(self, "_soft_edges_cache", None)
        if cached is None or cached[0] != key:
            softs = [e for e in self.scene.loose_mesh.edges
                     if getattr(e, "soft", False)
                     and not getattr(e, "hidden", False)
                     and self.scene.entity_visible(e) and e.faces]
            if softs:
                pts = np.empty((len(softs), 6))
                single = np.empty(len(softs), dtype=bool)
                # Face planes via one vectorized cross product instead of
                # per-face Python normal()/centroid() (_newell dominated the
                # edit frame at 25k+ faces). For the view-side sign test any
                # point ON the plane works, so the first loop vertex serves
                # as the anchor; the plane normal comes from the first two
                # loop edges (faces are planar).
                tri0 = np.empty((len(softs), 3, 3))
                tri1 = np.empty((len(softs), 3, 3))
                for i, e in enumerate(softs):
                    pts[i] = (e.a.x(), e.a.y(), e.a.z(),
                              e.b.x(), e.b.y(), e.b.z())
                    v = e.faces[0].vertices
                    tri0[i] = ((v[0].x(), v[0].y(), v[0].z()),
                               (v[1].x(), v[1].y(), v[1].z()),
                               (v[2].x(), v[2].y(), v[2].z()))
                    # A 1-face soft edge is an open-surface boundary (always
                    # a profile); a 2-face one straddles the view or hides.
                    single[i] = len(e.faces) != 2
                    v = (e.faces[1] if len(e.faces) == 2 else e.faces[0]).vertices
                    tri1[i] = ((v[0].x(), v[0].y(), v[0].z()),
                               (v[1].x(), v[1].y(), v[1].z()),
                               (v[2].x(), v[2].y(), v[2].z()))
                n0 = np.cross(tri0[:, 1] - tri0[:, 0], tri0[:, 2] - tri0[:, 0])
                n1 = np.cross(tri1[:, 1] - tri1[:, 0], tri1[:, 2] - tri1[:, 0])
                c0 = tri0[:, 0]
                c1 = tri1[:, 0]
                arrays = (pts.astype(np.float32), n0, c0, n1, c1, single)
            else:
                arrays = None
            cached = (key, arrays)
            self._soft_edges_cache = cached
        eye = self.camera.eye()
        chunks: list = []
        if cached[1] is not None:
            pts, n0, c0, n1, c1, single = cached[1]
            e_np = np.array([eye.x(), eye.y(), eye.z()])
            s0 = np.einsum("ij,ij->i", n0, c0 - e_np)
            s1 = np.einsum("ij,ij->i", n1, c1 - e_np)
            mask = single | ((s0 < 0) != (s1 < 0))
            chunks.append(pts[mask].tobytes())
        else:
            chunks.append(b"")
        pv_sil = getattr(self, "_preview_groups", None) or ()
        # Silhouettes are a full-strength black outline; drawing them over a
        # faded (or hidden) context would put the heaviest line in the frame
        # on exactly the geometry meant to recede. Only the subject profiles.
        skip_context = (self.scene.edit_group is not None
                        and self._edit_rest_mode in ("fade", "hide"))
        # Which placements get an outline changes with the placements, not
        # with the camera: the visibility test of every one of them ran on
        # every frame (~100 ms on 21 406 placements, issue #158).
        # The groups being previewed are left out when drawn, not here: the
        # list (and the per-prototype table made from it) then lasts while
        # a Move picks an object up — rebuilt, it was ~1 s on the plant.
        # Matrices alone (a Move picking up or putting down) change none of
        # the three tests: the list is kept, re-stamped (~0.1 s each way).
        gkey = (self._placements_epoch(), skip_context)
        gmemo = getattr(self, "_sil_groups", None)
        if gmemo is not None and gmemo[0] != gkey and \
                gmemo[0][1] == skip_context and \
                gmemo[2] is self.scene.mesh and \
                self._epoch_only_matrices(gmemo[0][0], gkey[0]):
            self._placements()        # nested proxies: composed matrices
            gmemo = self._sil_groups = (gkey,) + gmemo[1:]
        if gmemo is not None and gmemo[0] == gkey:
            groups = gmemo[1]
        else:
            def outlined(g):
                return (self.scene.entity_visible(g)
                        and not getattr(g, "billboard", False)
                        and not (skip_context
                                 and self._draws_in_edit_context(g)))
            groups = None
            if (gmemo is not None and gmemo[0][1] == skip_context
                    and gmemo[2] is self.scene.mesh):
                groups = self._refilter(gmemo[0][0], gmemo[1],
                                        self._placements(), outlined)
            if groups is None:
                groups = [g for g in self._placements() if outlined(g)]
            self._sil_groups = (gkey, groups, self.scene.mesh)
        if groups:
            import numpy as np
            e_np = np.array([eye.x(), eye.y(), eye.z()])
            planes = getattr(self, "_frame_planes", None)
            got, groups = self._instanced_silhouettes(groups, eye, planes,
                                                      skip=pv_sil)
            if pv_sil:
                groups = [g for g in groups if id(g) not in pv_sil]
            if got:
                chunks.append(got)
            for g in groups:
                if getattr(g, "xform", None) is not None:
                    got = self._instance_silhouette(g, eye, planes)
                    if got is not None:
                        if got:
                            chunks.append(got)
                        continue
                ch = self._group_chunk(g)
                if ch["soft_pts"] is None:
                    continue
                bb = ch.get("bbox")
                if (planes is not None and bb is not None
                        and not self._aabb_visible(planes, bb[0], bb[1])):
                    continue        # whole chunk off screen: skip the einsum
                s0 = np.einsum("ij,ij->i", ch["soft_n0"],
                               ch["soft_c0"] - e_np)
                s1 = np.einsum("ij,ij->i", ch["soft_n1"],
                               ch["soft_c1"] - e_np)
                mask = ch["soft_single"] | ((s0 < 0) != (s1 < 0))
                if mask.any():
                    chunks.append(ch["soft_pts"][mask].tobytes())
        raw = b"".join(chunks)
        self._silhouette_vbo.bind()
        if raw:
            self._silhouette_vbo.allocate(raw, len(raw))
        else:
            self._silhouette_vbo.allocate(24)
        self._silhouette_vbo.release()
        count = len(raw) // 12
        self._sil_last = (key, now, count, self._sil_inputs(key, cam))
        self._sil_cost = _time.monotonic() - now
        return count

    def _silhouette_table(self, groups):
        """Per prototype, the arrays the silhouette pass reads for all its
        placements at once — built when the placements change, not per
        frame. ``(table, rest)``: ``rest`` are the placements it does not
        take (loose groups, face-me, a singular matrix)."""
        import numpy as np
        ids = tuple(map(id, groups))
        key = (self._placements_epoch(), ids)
        memo = getattr(self, "_sil_table", None)
        if memo is not None and memo[0] == key:
            return memo[1], memo[2]
        if (memo is not None and memo[0][1] == ids
                and self._epoch_only_matrices(memo[0][0], key[0])
                and self._sil_table_moved(memo[1], memo[2])):
            # Only matrices moved (a Move put an object down): the rows of
            # the placements that moved, not the whole table (~1 s).
            self._sil_table = (key, memo[1], memo[2])
            return memo[1], memo[2]
        # An object or two added or taken away (a Delete, a Paste, their
        # undo): the rows of every unchanged object come from the old
        # table, and a prototype none of whose rows changed keeps its
        # entry — and with it the outline last worked out for this view.
        kept = (self._epoch_kept_tops(memo[0][0], key[0], ctx=True)
                if memo is not None else None)
        old_rows: dict = {}
        old_rest: set = set()
        if kept:
            for t in memo[1]:
                for j, g in enumerate(t["groups"]):
                    old_rows[id(g)] = (t, j)
            old_rest = {id(g) for g in memo[2]}
        top_of = self._placement_top
        inf = float("inf")
        by_proto: dict = {}          # uid → [old entry, base, rows]
        rest: list = []
        for g in groups:
            if kept and top_of(g) in kept:
                hit = old_rows.get(id(g))
                if hit is None:
                    if id(g) in old_rest:
                        rest.append(g)
                    continue
                t, j = hit
                ent = by_proto.get(t["uid"])
                if ent is None:
                    ent = by_proto[t["uid"]] = [t, None, []]
                elif ent[0] is None:
                    ent[0] = t
                ent[2].append((g, t, j))
                continue
            if getattr(g, "xform", None) is None \
                    or getattr(g, "billboard", False):
                rest.append(g)
                continue
            base = self._proto_base_chunk(g.mesh, _paint_of(self, g))
            if base["soft_pts"] is None:
                continue                      # nothing curved to outline
            box, inv = self._placement_frame(g)
            if inv is None:
                rest.append(g)
                continue
            ent = by_proto.get(base["uid"])
            if ent is None:
                ent = by_proto[base["uid"]] = [None, base, []]
            elif ent[1] is None:
                ent[1] = base
            ent[2].append((g, None, (
                np.array(g.xform.data(), np.float64)
                .reshape(4, 4, order="F")[:3],
                np.array(inv.data(), np.float64).reshape(4, 4, order="F")[:3],
                box[0] if box else (-inf, -inf, -inf),
                box[1] if box else (inf, inf, inf))))
        table = []
        for uid, (t, base, rows) in by_proto.items():
            if (t is not None and len(rows) == len(t["groups"])
                    and all(src is t and j == k
                            for k, (_g, src, j) in enumerate(rows))):
                table.append(t)               # as it was, outline and all
                continue
            gl = [r[0] for r in rows]
            fwds, invs, los, his = [], [], [], []
            for g, src, j in rows:
                if src is None:
                    fwd, inv, lo, hi = j
                else:
                    fwd, inv = src["fwd"][j], src["inv"][j]
                    lo, hi = src["lo"][j], src["hi"][j]
                fwds.append(fwd)
                invs.append(inv)
                los.append(lo)
                his.append(hi)
            if base is not None:
                n0 = np.asarray(base["soft_n0"], np.float64)
                n1 = np.asarray(base["soft_n1"], np.float64)
                arrays = {
                    "seg": np.asarray(base["soft_pts"], np.float64).reshape(-1, 2, 3),
                    "n0": n0, "n1": n1,
                    "d0": np.einsum("ij,ij->i", n0, np.asarray(base["soft_c0"], np.float64)),
                    "d1": np.einsum("ij,ij->i", n1, np.asarray(base["soft_c1"], np.float64)),
                    "single": np.asarray(base["soft_single"], bool)}
            else:
                arrays = {k: t[k] for k in ("seg", "n0", "n1", "d0", "d1",
                                            "single")}
            table.append({
                **arrays,
                "fwd": np.stack(fwds), "inv": np.stack(invs),
                "lo": np.asarray(los, np.float64), "hi": np.asarray(his, np.float64),
                "groups": gl, "xf": [tuple(g.xform.data()) for g in gl],
                "ids": frozenset(map(id, gl)), "rev": 0, "uid": uid,
            })
        self._sil_table = (key, table, rest)
        return table, rest

    def _sil_table_moved(self, table, rest) -> bool:
        """Bring the rows of the silhouette table whose placement's matrix
        changed up to date, in place; ``False`` (nothing touched that
        matters) when one cannot be — the table is then made again."""
        import numpy as np
        for g in rest:
            if getattr(g, "xform", None) is not None:
                return False      # a singular matrix may not be one now
        todo = []
        for t in table:
            xf = t["xf"]
            for j, g in enumerate(t["groups"]):
                d = tuple(g.xform.data())
                if d != xf[j]:
                    box, inv = self._placement_frame(g)
                    if inv is None:
                        return False
                    todo.append((t, j, d, box, inv))
        inf = float("inf")
        for t, j, d, box, inv in todo:
            t["fwd"][j] = np.array(d, np.float64).reshape(4, 4, order="F")[:3]
            t["inv"][j] = (np.array(inv.data(), np.float64)
                           .reshape(4, 4, order="F")[:3])
            t["lo"][j] = box[0] if box else (-inf, -inf, -inf)
            t["hi"][j] = box[1] if box else (inf, inf, inf)
            t["xf"][j] = d
            t["rev"] += 1
        return True

    def _instanced_silhouettes(self, groups, eye, planes, skip=()):
        """Silhouette bytes of every component placement in ``groups``,
        prototype by prototype: the frustum cull and the «faces straddle
        the view» test run as one NumPy pass over all the placements of a
        prototype, with the eye in each one's local coordinates (see
        ``_instance_silhouette``, the one-placement version they must
        agree with). A per-placement Python loop cost 1.7 s a frame on a
        model of 21 406 placements (issue #158). Returns ``(bytes, rest)``
        with the placements left to the per-group path. ``skip``: ids of
        placements left out (the ones a preview draws)."""
        import numpy as np
        table, rest = self._silhouette_table(groups)
        if not table:
            return b"", rest
        e = np.array([eye.x(), eye.y(), eye.z()])
        pl = np.asarray(planes, np.float64) if planes is not None else None
        out = []
        # Per prototype, the last answer for this very view: when a Move
        # puts an object down only its prototype's rows moved, and the
        # others need not be worked out again (~0.3 s on the plant).
        view = (tuple(e.tolist()),
                tuple(map(tuple, planes)) if planes is not None else None)
        for t in table:
            # Only the left-out placements that are this prototype's.
            mine = (frozenset(k for k in skip if k in t["ids"])
                    if skip else frozenset())
            memo = t.get("memo")
            if (memo is not None and memo[0] == view and memo[1] == t["rev"]
                    and memo[3] == mine):
                if memo[2]:
                    out.append(memo[2])
                continue
            t_out = []
            self._instanced_silhouette_rows(t, e, pl, mine, t_out)
            raw = b"".join(t_out)
            t["memo"] = (view, t["rev"], raw, mine)
            if raw:
                out.append(raw)
        return b"".join(out), rest

    def _instanced_silhouette_rows(self, t, e, pl, skip, out) -> None:
        """One prototype's part of :meth:`_instanced_silhouettes`."""
        import numpy as np
        if pl is not None:
            n = pl[:, :3]
            pick = np.where(n[:, None, :] >= 0.0, t["hi"][None], t["lo"][None])
            keep = ((pick * n[:, None, :]).sum(axis=2)
                    + pl[:, 3][:, None] >= 0.0).all(axis=0)
        else:
            keep = np.ones(len(t["fwd"]), bool)
        if skip:
            keep &= np.fromiter((id(g) not in skip for g in t["groups"]),
                                bool, count=len(t["groups"]))
        rows = np.flatnonzero(keep)
        if not len(rows):
            return
        ne = len(t["d0"])
        step = max(1, int(4_000_000 // max(ne, 1)))
        for a in range(0, len(rows), step):
            rr = rows[a:a + step]
            inv = t["inv"][rr]
            eye_l = inv[:, :, :3] @ e + inv[:, :, 3]          # (K, 3)
            s0 = t["d0"][None, :] - eye_l @ t["n0"].T
            s1 = t["d1"][None, :] - eye_l @ t["n1"].T
            mask = t["single"][None, :] | ((s0 < 0) != (s1 < 0))
            r, c = np.nonzero(mask)
            if not len(r):
                continue
            m = t["fwd"][rr][r]                               # (N, 3, 4)
            seg = t["seg"][c]                                 # (N, 2, 3)
            pts = np.einsum("nij,nkj->nki", m[:, :, :3], seg) \
                + m[:, None, :, 3]
            out.append(pts.astype(np.float32).tobytes())

    def _instance_silhouette(self, g, eye, planes):
        """Silhouette bytes of a component placement, worked out on the
        PROTOTYPE's shared arrays — ``None`` when it cannot be (a singular
        matrix), and the caller bakes as before.

        Which side of a plane the eye is on does not change under the
        placement's affine map, and a mirror flips BOTH faces of a soft
        edge at once, so «its faces straddle the view» reads the same with
        the eye taken into local coordinates. Only the edges that pass are
        carried to the world. Baking every placement to world coordinates
        for this test was the last per-placement bake of a frame: with
        profiles on, the 21 000 placements of issue #158 baked 14 million
        faces the instanced draw never needed."""
        import numpy as np
        base = self._proto_base_chunk(g.mesh, _paint_of(self, g))
        if base["soft_pts"] is None:
            return b""
        bb, inv = self._placement_frame(g)
        if planes is not None and bb is not None \
                and not self._aabb_visible(planes, bb[0], bb[1]):
            return b""
        if inv is None:
            return None
        le = inv.map(eye)
        e_np = np.array([le.x(), le.y(), le.z()])
        s0 = np.einsum("ij,ij->i", base["soft_n0"], base["soft_c0"] - e_np)
        s1 = np.einsum("ij,ij->i", base["soft_n1"], base["soft_c1"] - e_np)
        mask = base["soft_single"] | ((s0 < 0) != (s1 < 0))
        if not mask.any():
            return b""
        m = np.array(g.xform.data(), dtype=np.float64).reshape(4, 4, order="F")
        seg = base["soft_pts"].reshape(-1, 6)[mask].astype(np.float64)
        pts = seg.reshape(-1, 3) @ m[:3, :3].T + m[:3, 3]
        return pts.astype(np.float32).tobytes()

    def _upload_hover_edge(self, edge: Edge) -> int:
        """Upload the hovered edge — or, for a curve segment, its whole contour
        (what a click would select) — into the hover-edges VBO. Returns the
        vertex count to draw."""
        edges = (self.scene.mesh.curve_edges(edge)
                 if getattr(edge, "curve", None) is not None else [edge])
        if edge not in edges:      # e.g. a group's edge — not in the main mesh
            edges = [edge]
        data = array("f")
        for e in edges:
            data.extend([e.a.x(), e.a.y(), e.a.z(),
                         e.b.x(), e.b.y(), e.b.z()])
        self._hover_edges_vbo.bind()
        raw = data.tobytes()
        self._hover_edges_vbo.allocate(raw, len(raw))
        self._hover_edges_vbo.release()
        return len(data) // 3

    def _draw_preview_faces(self) -> None:
        """Triangulate and draw the active tool's solid preview faces (if any).
        A face carrying attrs shows its REAL look — painted colour, or its
        texture (the tool re-anchors the uvw map to the drag) — so a paste
        drags the actual model; bare faces keep the warm cream of the
        push/pull preview. Depth-tested with a polygon offset so the
        wireframe sits cleanly on top."""
        tool = self.active_tool
        provider = getattr(tool, "preview_faces", None) if tool is not None else None
        if not callable(provider):
            return
        faces = provider()
        if not faces:
            return
        data = array("f")
        runs = []                        # (shaded_rgb, start_vertex, count)
        by_texture: dict = {}            # (path, shade) -> interleaved pos+uv
        for face in faces:
            attrs = getattr(face, "attrs", None) or {}
            tex = attrs.get("texture")
            if tex and tex.get("path"):
                self._append_textured_face(by_texture, face, tex)
                continue
            color = attrs.get("color") or self.DEFAULT_FACE_COLOR
            start = len(data) // 3
            for t0, t1, t2 in self._tris_of(face):
                data.extend([
                    t0.x(), t0.y(), t0.z(),
                    t1.x(), t1.y(), t1.z(),
                    t2.x(), t2.y(), t2.z(),
                ])
            runs.append((self._shaded_color(tuple(color[:3]), self._normal_of(face)),
                         start, len(data) // 3 - start))
        if not data and not by_texture:
            return
        self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
        self._gl.glPolygonOffset(1.0, 1.0)
        if data:
            self._preview_faces_vbo.bind()
            raw = data.tobytes()
            self._preview_faces_vbo.allocate(raw, len(raw))
            self._preview_faces_vbo.release()
            self._preview_faces_vao.bind()
            for (r, g, b), start, count in runs:
                self._set_color(r, g, b, 1.0)
                # A preview has no back: it is an overlay, not a solid with an
                # inside. Its walls are wound however the sweep produced them
                # and nothing surrounds them to hide the ones turned away, so
                # the real back-face tint painted the pocket of a push-in blue
                # over the shape being formed (Marco, 2026-08-27). Both sides
                # take the face's own colour.
                self._program.setUniformValue(self._loc_back_color,
                                              QVector4D(r, g, b, 1.0))
                self._gl.glDrawArrays(GL_TRIANGLES, start, count)
            self._preview_faces_vao.release()
            self._set_back_face_color()          # leave the real tint behind
        if by_texture:
            # A tool may want its textured preview translucent (Position
            # Texture shows the image through).
            opacity = float(getattr(tool, "preview_opacity", 1.0) or 1.0)
            if opacity < 1.0:
                self._program.setUniformValue1f(self._loc_opacity, opacity)
            self._program.setUniformValue(self._loc_use_tex, 1)
            self._preview_tex_vao.bind()
            for key, buf in by_texture.items():
                path, shade = key if isinstance(key, tuple) else (key, 1.0)
                tex_obj = self._get_texture(path)
                if tex_obj is None:
                    continue
                raw = buf.tobytes()
                self._preview_tex_vbo.bind()
                self._preview_tex_vbo.allocate(raw, len(raw))
                self._preview_tex_vbo.release()
                self._program.setUniformValue1f(self._loc_shade, float(shade))
                tex_obj.bind(0)
                self._gl.glDrawArrays(GL_TRIANGLES, 0, len(buf) // 5)
                tex_obj.release(0)
            self._preview_tex_vao.release()
            self._program.setUniformValue1f(self._loc_shade, 1.0)
            self._program.setUniformValue(self._loc_use_tex, 0)
            if opacity < 1.0:
                self._program.setUniformValue1f(self._loc_opacity, 1.0)
        self._gl.glDisable(GL_POLYGON_OFFSET_FILL)

    def _draw_rubber_band(self) -> None:
        self._overlay_rubber = None
        style = self._rubber_band_style()
        if style is None:
            return
        tool = self.active_tool
        segments, color, width = style

        # Depth-tested previews (Push/Pull, Offset, Paste) read like real
        # geometry and need GL hidden-line removal — draw them via GL. The
        # "always on top" previews (Line/Rectangle/Move) are stashed for the
        # QPainter overlay instead, where a thick pen is reliable (Core-profile
        # glLineWidth often clamps to 1px on Mesa, so GL can't thicken them).
        if getattr(tool, "wireframe_depth_tested", False):
            data = array("f")
            for a, b in segments:
                data.extend([a.x(), a.y(), a.z(), b.x(), b.y(), b.z()])
            raw = data.tobytes()
            self._rubber_vbo.bind()
            self._rubber_vbo.allocate(raw, len(raw))
            self._rubber_vbo.release()

            self._set_color(*color)
            self._rubber_vao.bind()
            self._gl.glDrawArrays(GL_LINES, 0, len(data) // 3)
            self._rubber_vao.release()
        else:
            self._overlay_rubber = (segments, color, width)

    def _draw_camera_frame(self, painter: QPainter) -> None:
        """Camera Settings ▸ Frame: the picture's shape (16:9, 4:3…) as the
        largest such rectangle in the view, the bands outside it shaded and
        a thin line around it, so what a render or a sheet will take is
        seen while you frame it."""
        from core.camera import frame_rect
        rect = frame_rect(self.width(), self.height(),
                          getattr(self.camera, "frame_aspect", None))
        if rect is None:
            return
        x, y, w, h = rect
        W, H = float(self.width()), float(self.height())
        painter.save()
        painter.setPen(Qt.NoPen)
        painter.setBrush(QColor(0, 0, 0, 110))
        if x > 0.5:
            painter.drawRect(QRectF(0.0, 0.0, x, H))
            painter.drawRect(QRectF(x + w, 0.0, W - x - w, H))
        if y > 0.5:
            painter.drawRect(QRectF(0.0, 0.0, W, y))
            painter.drawRect(QRectF(0.0, y + h, W, H - y - h))
        painter.setBrush(Qt.NoBrush)
        painter.setPen(QPen(QColor(255, 255, 255, 200), 1.0))
        painter.drawRect(QRectF(x + 0.5, y + 0.5, w - 1.0, h - 1.0))
        painter.restore()

    # ---- 2D overlay (QPainter on top of OpenGL) -----------------------------
    def _draw_overlay(self) -> None:
        painter = QPainter(self)
        painter.setRenderHint(QPainter.Antialiasing, True)
        painter.setRenderHint(QPainter.TextAntialiasing, True)

        # The camera's picture frame first: what lies outside it is
        # shaded, and everything below still draws over it.
        self._draw_camera_frame(painter)

        # Rubber band for the "always on top" tools (Line/Rectangle/Move),
        # drawn here with a thick, reliable pen.
        self._draw_rubber_band_overlay(painter)
        self._draw_guide_preview(painter)

        # The acquired circle centre (Center inference): a small
        # green dot at the centre of the last circle or arc the cursor
        # visited — shown only once the cursor comes NEAR it, not the whole
        # time you are on the face («solo debería aparecer cuando me ponga
        # cerca del medio del círculo», Marco, 2026-09-11).
        ref = self._valid_center_ref()
        snapped_on_it = (self.last_snap is not None
                         and self.last_snap.kind == "center")
        if ref is not None and self.active_tool is not None \
                and self.active_tool.uses_snap \
                and not snapped_on_it and self._last_mouse_pos is not None:
            pc = self._world_to_pixel(ref[0])
            near = pc is not None and self._center_hint_px(ref, pc) > 0.0 \
                and math.hypot(pc[0] - self._last_mouse_pos.x(),
                               pc[1] - self._last_mouse_pos.y()) \
                <= self._center_hint_px(ref, pc)
            if near:
                from core.snap import COLOR_ENDPOINT as _CE
                painter.setPen(QPen(QColor(255, 255, 255, 230), 3.0))
                painter.setBrush(QColor.fromRgbF(*_CE, 1.0))
                painter.drawEllipse(QPointF(*pc), 4.0, 4.0)
                painter.setPen(QPen(QColor.fromRgbF(*_CE, 1.0), 1.5))
                painter.drawEllipse(QPointF(*pc), 4.0, 4.0)
        # The centres of the selected faces, under the snap marker.
        self._draw_selected_face_centers(painter)
        # Snap indicator
        if (
            self.active_tool is not None
            and self.last_snap is not None
            and self.last_snap.kind != "none"
        ):
            self._draw_snap_indicator(painter, self.last_snap)

        # Push/Pull distance-inference marker (a green square on the corner/face
        # the extrusion is snapping level with).
        self._draw_inference_marker(painter)

        # Construction guides (Tape Measure) — fine dashed scaffolding lines.
        self._draw_guides(painter)

        # Borders of the reference images, so one lying flat is still findable.
        self._draw_image_outlines(painter)

        # The Scale tool's grip box (yellow box with green grips).
        self._draw_scale_box(painter)
        self._draw_section_planes(painter)
        # Tool-owned overlay (the Flip planes; the future Axes gizmo): a tool
        # exposing ``draw_overlay(viewport, painter)`` paints after the
        # document annotations, before the edit-group frame.
        hook = getattr(self.active_tool, "draw_overlay", None)
        if callable(hook):
            hook(self, painter)
        # Extensions' overlays (``ExtensionApp.add_overlay``).
        self._draw_extension_overlays(painter)

        # Terrain-surface fills (draped / flat) under the georef paths — Track G.
        self._draw_geo_surfaces(painter)

        # Traced georef paths (roads / boundaries) — Track G.
        self._draw_geo_paths(painter)

        # Imported survey points (GPS / total station) — Track G.
        self._draw_geo_points(painter)

        self._draw_route_marker(painter)

        # Persistent dimension annotations
        self._draw_dimensions(painter)

        # Leader-text annotations
        self._draw_text_labels(painter)

        # Length measurement near rubber band
        self._draw_length_label(painter)

        # Labels in the top-left. Reference > explicit axis lock > soft inference.
        if self.reference_mode is not None:
            self._draw_reference_label(painter)
        elif self.axis_lock is not None:
            self._draw_axis_lock_label(painter)
        elif getattr(self.active_tool, "plane_lock", None) is not None:
            self._draw_plane_lock_label(painter)
        else:
            self._draw_inference_label(painter)

        # Linear-inference toggle state (Alt), shown while not on the default.
        if self.linear_inference_mode != "all":
            self._draw_linear_mode_label(painter)

        # Rubber-band selection box.
        self._draw_selection_box(painter)
        self._draw_zoom_box(painter)

        painter.end()

    def _draw_selection_box(self, painter: QPainter) -> None:
        if not self._box_active or self._box_start is None or self._box_cur is None:
            return
        s, c = self._box_start, self._box_cur
        if math.hypot(c.x() - s.x(), c.y() - s.y()) < self.BOX_DRAG_THRESHOLD_PX:
            return
        rect = QRectF(
            min(s.x(), c.x()), min(s.y(), c.y()),
            abs(c.x() - s.x()), abs(c.y() - s.y()),
        )
        crossing = (c.x() - s.x()) < 0
        if crossing:
            # Crossing: dashed green, selects anything it touches.
            pen = QPen(QColor(40, 158, 92), 1.5, Qt.DashLine)
            fill = QColor(40, 158, 92, 28)
        else:
            # Window: solid blue, selects only fully enclosed.
            pen = QPen(QColor(51, 102, 199), 1.5, Qt.SolidLine)
            fill = QColor(51, 102, 199, 28)
        painter.setPen(pen)
        painter.setBrush(fill)
        painter.drawRect(rect)

    def _draw_zoom_box(self, painter: QPainter) -> None:
        if (not self._zoom_box_active or self._zoom_box_start is None
                or self._zoom_box_cur is None):
            return
        s, c = self._zoom_box_start, self._zoom_box_cur
        rect = QRectF(
            min(s.x(), c.x()), min(s.y(), c.y()),
            abs(c.x() - s.x()), abs(c.y() - s.y()),
        )
        painter.setPen(QPen(QColor(243, 115, 41), 1.5, Qt.DashLine))
        painter.setBrush(QColor(243, 115, 41, 26))
        painter.drawRect(rect)

    def _draw_section_fill(self, mode, style) -> None:
        """Section Fill: paint what the active cut slices out of solids —
        and only that. The outlines come from the cut itself: the chords the
        plane takes out of the visible triangles, chained into closed rings
        with their holes (core.section.fill_triangles), the same rule the
        sheet composer's poché follows. An open surface gives no closed
        ring and stays unfilled.

        It used to be a per-pixel trick — fill the plane wherever a BACK
        face was the visible surface — which also painted faces seen from
        behind that the plane does not cut at all (a lone face, a flipped
        one, a 2D cutout): the fill showed where there was no section.
        Lives in the style (style.section_fill)."""
        if getattr(self, "_clip_vec", None) is None:
            return
        if mode in ("wireframe", "xray"):
            return
        sp = self.scene.active_section()
        if sp is None:
            return
        # The plane's own fill over the style's (the style's switch is the
        # master): solid, hatched, cross-hatched or none, in its colour.
        from core.section import effective_fill
        pattern, fc, spacing = effective_fill(sp, style)
        if pattern == "none":
            return
        if pattern == "layers":
            self._draw_layer_fills(fc)
            return
        tris = self._section_fill_tris()
        if tris is None or not len(tris):
            return
        # The fill lies IN the plane, which clips: unclipped while it
        # draws, nudged a hair behind so the thick cut lines drawn later
        # win the depth test.
        self._set_section_clip(False)
        raw = tris.reshape(-1).astype("float32").tobytes()
        self._section_vbo.bind()
        self._section_vbo.allocate(raw, len(raw))
        self._section_vbo.release()
        # A hatched cut sits on a pale ground of its own colour, so the
        # inside of the solid never shows between the lines.
        base = (fc if pattern == "solid"
                else tuple(c + (1.0 - c) * 0.78 for c in fc[:3]))
        self._gl.glEnable(GL_POLYGON_OFFSET_FILL)
        self._gl.glPolygonOffset(2.0, 2.0)
        self._set_color(base[0], base[1], base[2], 1.0)
        self._program.setUniformValue(
            self._loc_back_color, QVector4D(base[0], base[1], base[2], 1.0))
        self._section_vao.bind()
        self._gl.glDrawArrays(GL_TRIANGLES, 0, len(tris) * 3)
        self._section_vao.release()
        self._gl.glDisable(GL_POLYGON_OFFSET_FILL)
        if pattern in ("hatch", "cross"):
            lines = self._section_hatch(spacing, pattern == "cross")
            if lines is not None and len(lines):
                raw = lines.reshape(-1).astype("float32").tobytes()
                self._section_vbo.bind()
                self._section_vbo.allocate(raw, len(raw))
                self._section_vbo.release()
                self._set_color(fc[0], fc[1], fc[2], 1.0)
                self._program.setUniformValue(
                    self._loc_back_color,
                    QVector4D(fc[0], fc[1], fc[2], 1.0))
                self._section_vao.bind()
                self._gl.glDrawArrays(GL_LINES, 0, len(lines) * 2)
                self._section_vao.release()
        self._set_section_clip(True)

    def _draw_section_cut_edges(self) -> None:
        """Thick section-cut lines: quads in the cut plane, sized
        ~2.5 px, nudged toward the CLIPPED side so they never z-fight the
        remaining geometry. Skipped when cuts are hidden."""
        if getattr(self, "_clip_vec", None) is None:
            return
        segs = self._section_cut_segments()
        if segs is None or not len(segs):
            return
        import numpy as np
        sp = self.scene.active_section()
        n = np.array([sp.normal.x(), sp.normal.y(), sp.normal.z()],
                     dtype=np.float64)
        a = segs[:, 0].astype(np.float64)
        b = segs[:, 1].astype(np.float64)
        t = b - a
        ln = np.linalg.norm(t, axis=1)
        keep = ln > 1e-9
        if not keep.any():
            return
        a, b, t, ln = a[keep], b[keep], t[keep], ln[keep]
        t /= ln[:, None]
        side = np.cross(np.broadcast_to(n, t.shape), t)
        # ~2.5 px of world width at each END's distance from the eye: one
        # width per segment, taken at its middle, made a wall's cut line
        # jump from thin to thick where its face's triangles meet.
        eye = self.camera.eye()
        eyev = np.array([eye.x(), eye.y(), eye.z()], dtype=np.float64)
        h = max(1.0, self.height())
        fov = math.radians(getattr(self.camera, "fov_deg", 45.0))
        scale = 1.25 * math.tan(fov * 0.5) * 2.0 / h
        weight = self._cut_edge_weights(sp, keep)
        wa = np.maximum(np.linalg.norm(a - eyev, axis=1) * scale, 1e-6)
        wb = np.maximum(np.linalg.norm(b - eyev, axis=1) * scale, 1e-6)
        wa = (wa * weight)[:, None]
        wb = (wb * weight)[:, None]
        lift = n * 0.6
        q0 = a - side * wa + lift * wa
        q1 = b - side * wb + lift * wb
        q2 = b + side * wb + lift * wb
        q3 = a + side * wa + lift * wa
        tris = np.empty((len(a) * 2, 9), dtype=np.float32)
        tris[0::2, 0:3] = q0
        tris[0::2, 3:6] = q1
        tris[0::2, 6:9] = q2
        tris[1::2, 0:3] = q0
        tris[1::2, 3:6] = q2
        tris[1::2, 6:9] = q3
        raw = tris.tobytes()
        self._section_vbo.bind()
        self._section_vbo.allocate(raw, len(raw))
        self._section_vbo.release()
        ec = self._frame_style.edge_color
        self._set_color(ec[0], ec[1], ec[2], 1.0)
        self._section_vao.bind()
        self._gl.glDrawArrays(GL_TRIANGLES, 0, len(a) * 6)
        self._section_vao.release()
