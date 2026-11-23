struct Camera { vp: mat4x4f, size: vec4f }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var sample_atlas: sampler;
// rgb is the edit-context colour, w is the blend toward it.
struct Appearance { fade: vec4f, opacity: vec4f }
@group(0) @binding(3) var<uniform> appearance: Appearance;
struct Out { @builtin(position) position: vec4f, @location(0) uv: vec2f }
@vertex fn vs(@location(0) point: vec3f, @location(1) uv: vec2f) -> Out {
    var o: Out;
    o.position = camera.vp * vec4f(point, 1.);
    o.uv = uv;
    return o;
}
@fragment fn fs(i: Out) -> @location(0) vec4f {
    let texel = textureSample(atlas, sample_atlas, i.uv);
    let alpha = texel.a * appearance.opacity.x;
    if alpha == 0. { discard; }
    return vec4f(mix(texel.rgb, appearance.fade.rgb, appearance.fade.a), alpha);
}
