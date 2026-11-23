// Bound the visible receivers in the original light projection. All casters
// retain their original depth range when the XY projection is narrowed.
@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> screen_to_light: mat4x4f;
@group(0) @binding(2) var<storage, read_write> bounds: array<atomic<i32>, 4>;

@compute @workgroup_size(8, 8)
fn fit(@builtin(global_invocation_id) id: vec3u) {
 let pixel = id.xy * 8u;
 let size = textureDimensions(depth);
 if any(pixel >= size) { return; }
 let d = textureLoad(depth, vec2i(pixel), 0);
 if d <= 0. { return; }
 let ndc = ((vec2f(pixel) + 0.5) / vec2f(size) * 2. - 1.) * vec2f(1., -1.);
 let clip = screen_to_light * vec4f(ndc, d, 1.);
 let xy = vec2i(clamp(clip.xy / clip.w, vec2f(-1.), vec2f(1.)) * 1048576.);
 atomicMin(&bounds[0], xy.x);
 atomicMin(&bounds[1], xy.y);
 atomicMax(&bounds[2], xy.x);
 atomicMax(&bounds[3], xy.y);
}
