// Dual Kawase blur: halve down a few levels, then double back up. Each
// pass reads one level and writes the next; `texel` is half a source texel.

struct Step { texel: vec4<f32> };
@group(0) @binding(0) var<uniform> step: Step;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

@fragment
fn down(in: Out) -> @location(0) vec4<f32> {
    let h = step.texel.xy;
    var s = textureSample(src, smp, in.uv) * 4.0;
    s += textureSample(src, smp, in.uv - h);
    s += textureSample(src, smp, in.uv + h);
    s += textureSample(src, smp, in.uv + vec2<f32>(h.x, -h.y));
    s += textureSample(src, smp, in.uv - vec2<f32>(h.x, -h.y));
    return s / 8.0;
}

@fragment
fn up(in: Out) -> @location(0) vec4<f32> {
    let h = step.texel.xy;
    var s = textureSample(src, smp, in.uv + vec2<f32>(-h.x * 2.0, 0.0));
    s += textureSample(src, smp, in.uv + vec2<f32>(-h.x, h.y)) * 2.0;
    s += textureSample(src, smp, in.uv + vec2<f32>(0.0, h.y * 2.0));
    s += textureSample(src, smp, in.uv + vec2<f32>(h.x, h.y)) * 2.0;
    s += textureSample(src, smp, in.uv + vec2<f32>(h.x * 2.0, 0.0));
    s += textureSample(src, smp, in.uv + vec2<f32>(h.x, -h.y)) * 2.0;
    s += textureSample(src, smp, in.uv + vec2<f32>(0.0, -h.y * 2.0));
    s += textureSample(src, smp, in.uv + vec2<f32>(-h.x, -h.y)) * 2.0;
    return s / 12.0;
}
