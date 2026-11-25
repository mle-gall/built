@group(0) @binding(0) var pixels: texture_2d<f32>;
@group(0) @binding(1) var<uniform> cue: vec4f;
struct Varying {
    @builtin(position) position: vec4<f32>,
};
@vertex fn vs(@builtin(vertex_index) i: u32) -> Varying {
    var xy = array<vec2<f32>, 3>(vec2(-1., -1.), vec2(3., -1.), vec2(-1., 3.));
    var out: Varying;
    out.position = vec4(xy[i], 0., 1.);
    return out;
}
@fragment fn fs(v: Varying) -> @location(0) vec4<f32> {
    let top = textureLoad(pixels, vec2<i32>(v.position.xy), 0);
    if cue.w == 0. { return top; }
    let r = distance(v.position.xy, cue.xy) / cue.z;
    let halo = (1. - smoothstep(5., 6., r)) * 0.9;
    let dot = 1. - smoothstep(4., 5., r);
    let alpha = max(halo, dot) * (1. - top.a);
    // Linear-light equivalents of the endpoint green and its white halo.
    let rgb = mix(vec3f(1.), vec3f(0.022, 0.342, 0.107), dot);
    return vec4f((top.rgb * top.a + rgb * alpha) / max(0.00001, top.a + alpha), top.a + alpha);
}
