// Frosted glass behind the chrome: the frame just drawn, blurred
// (glass.rs), seen through a rounded rectangle, desaturated a little and
// tinted. Output is premultiplied, as egui blends.

struct Pane {
    rect: vec4<f32>,    // x0, y0, x1, y1 in physical pixels
    params: vec4<f32>,  // corner radius px, screen w, screen h, saturation
    tint: vec4<f32>,    // linear rgb, amount
    edge: vec4<f32>,    // rim light at top-left, at bottom-right, rim width px, top glow
};
@group(0) @binding(0) var<uniform> pane: Pane;
@group(0) @binding(1) var blurred: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn rounded(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let centre = (pane.rect.xy + pane.rect.zw) * 0.5;
    let half = (pane.rect.zw - pane.rect.xy) * 0.5;
    let d = rounded(pos.xy - centre, half, pane.params.x);
    let a = clamp(0.5 - d, 0.0, 1.0);
    if (d > 2.0) {
        discard;
    }
    var c = textureSample(blurred, smp, pos.xy / pane.params.yz).rgb;
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = mix(vec3<f32>(l), c, pane.params.w);
    // Keep text legible on pale models: cap the backdrop's brightness
    // (linear 0.16 ≈ 44 % sRGB) before the tint, keeping its colour.
    let lum = max(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4);
    c = c * min(1.0, 0.16 / lum);
    c = mix(c, pane.tint.rgb, pane.tint.a);
    // Edge details: a rim lit from the top-left, fading to the bottom-right,
    // a soft glow just inside the top edge, a dark line just outside.
    let t = clamp(dot((pos.xy - pane.rect.xy) / max(pane.rect.zw - pane.rect.xy, vec2<f32>(1.0)),
                      vec2<f32>(0.5, 0.5)), 0.0, 1.0);
    let rim = (1.0 - smoothstep(0.0, pane.edge.z, -d)) * mix(pane.edge.x, pane.edge.y, t);
    let glow = pane.edge.w * (1.0 - smoothstep(0.0, 18.0, pos.y - pane.rect.y));
    c = mix(c, vec3<f32>(1.0), clamp(rim + glow, 0.0, 1.0));
    let outer = clamp(1.0 - abs(d - 0.75), 0.0, 1.0) * 0.35;
    let alpha = max(a, outer);
    let col = c * a;  // the dark outline adds coverage, no colour
    return vec4<f32>(col, alpha);
}
