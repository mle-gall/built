# Depth precision: thin plates

A plate a few millimetres thick is two faces a few millimetres apart. When the
depth buffer cannot tell them apart, the back face wins some pixels and they
flicker while orbiting.

## Metal view: reverse-Z

The Metal renderer (`rust/ingetrazo_render`) uses a 32-bit float depth,
reversed: the near plane is depth 1 and a perspective view's far plane is at
infinity, depth `n / d`. A float has the same relative precision at every
magnitude, so this keeps about 2⁻²⁴ of the distance everywhere: 0.012 mm at
200 m, whatever the near plane. The usual mapping (near 0, far 1) put the
float's precision at the near plane, a few centimetres from the eye.

`rust/ingetrazo_render/src/depth.rs` holds the convention: the clear value,
the comparisons (`compare(Less)` is `Greater`), the depth biases (negated)
and the projection. Every pass that tests, biases, clears, resolves or reads
the camera depth goes through it: faces, edges, profiles, hidden edges,
lines, previews and face marks, the axes, section caps, x-ray, the MSAA
path, the AO depth resolve (nearest sample = largest value), AO's sky test,
the sun receiver's sky/ground ray, and the near-plane clipping of edges,
axes and lines (`w − z` is the distance in front of the near plane). The
sun's own shadow map is a separate light-space depth and keeps the usual
mapping. Picking reads the id target, not depth.

The camera still sends OpenGL-convention matrices; the renderer recovers
near and far in f64 from the matrix's own entries. A far clip set by hand
(Camera Settings ▸ Clipping) is passed as `far_clip=True` and keeps its
plane (depth `n (f − d) / ((f − n) d)`, resolution about `f · 6·10⁻⁸`);
parallel views are flipped, `z' = (w − z) / 2`.

## OpenGL view: near/far fitted to the visible model

macOS OpenGL has no `glClipControl`, so reverse-Z there would not get the
float's precision (a 2026-10 diagnostic with 32-bit float depth, negated
projection and `GEQUAL` still fought at 200 m). The GL view keeps standard
depth (`[0, 1]`, `LEQUAL`, clear 1) and fits the perspective near/far planes
to the boxes of what it draws (`OrbitCamera.fit_depth_range`,
`Viewport._fit_scene_depth`), with a 5 % margin: moving the near plane out
is what matters, `z²(f − n) / (f n (2²⁴ − 1))` per depth step.

The automatic planes stay for parallel views, a near or far clip set by
hand, edit contexts, live group previews, a tool's preview faces, guides,
base map, terrain, survey, shadows, hidden-object ghosts, unknown bounds,
and any box that crosses the eye (moving near inward would only lose
precision). The fit holds for the exact camera it was made for and lapses
when anything about the view changes. Editing rays use the unfitted matrix
(`picking_projection_matrix`) so picks and snaps stay byte-for-byte the
same; depth read-back uses the fitted matrix that drew the frame. Drawing
axes are depth clamped so the fit does not cut them.

The Metal view applies the same fit to the shown placements' box
(`WgpuView._fit_depth`) so the shared camera behaves the same in both views;
with reverse-Z it changes nothing about precision there.

## Measurements

`tests/test_metal_thin_walls.py` (Metal) and `tests/test_depth_fit.py`
(GL), run with `QT_QPA_PLATFORM=cocoa`: 1, 2 and 5 mm plates at 10, 50 and
200 m, two frames 0.01° apart, back-colour pixels inside the top face.

| view | 1 mm (10/50/200 m) | 2 mm | 5 mm |
|---|---|---|---|
| Metal, before | 0 / 612 / 20 832 | 0 / 6 / 11 534 | 0 / 0 / 3 701 |
| Metal, reverse-Z | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| GL, before | 0 / 1 511 / 37 804 | 0 / 0 / 71 873 | 0 / 0 / 31 048 |
| GL, fitted | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
