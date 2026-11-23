// Frosted glass behind the chrome: the frame just drawn, blurred
// (glass.rs), seen through a rounded rectangle, desaturated a little and
// tinted. Output is premultiplied, as egui blends.

struct Pane {
    rect: vec4<f32>,    // x0, y0, x1, y1 in physical pixels
    params: vec4<f32>,  // corner radius px, screen w, screen h, saturation
    tint: vec4<f32>,    // linear rgb, amount
    edge: vec4<f32>,    // lit rim, quiet rim, rim width (physical px), top highlight
    detail: vec4<f32>,  // backdrop peak, pixels per point, inner shadow, unused
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
    // Council 2026-11-22 (glass specialist): a soft shoulder instead of a
    // hard brightness cap, a rim lit from the shape's normal, a narrow top
    // highlight, a faint inner shadow on the far side, a crisp outline.
    // Derivatives and the sample stay in uniform control flow: no discard.
    let centre = (pane.rect.xy + pane.rect.zw) * 0.5;
    let half = (pane.rect.zw - pane.rect.xy) * 0.5;
    let d = rounded(pos.xy - centre, half, pane.params.x);
    let a = clamp(0.5 - d, 0.0, 1.0);

    let scene = textureSample(blurred, smp, pos.xy / pane.params.yz).rgb;
    let y = dot(scene, vec3<f32>(0.2126, 0.7152, 0.0722));
    let muted = mix(vec3<f32>(y), scene, pane.params.w);
    // Sheet glass preserves the scene's luminance differences. Compact
    // chrome uses a shoulder to keep its small controls visually quiet.
    let shoulder = select(pane.detail.x * 1.5 / (y + 0.5), pane.detail.x, pane.detail.w > 0.0);
    var c = mix(muted * shoulder, pane.tint.rgb, pane.tint.a);

    let ppp = pane.detail.y;
    let inward = max(-d, 0.0);
    let normal = normalize(vec2<f32>(dpdx(d) + 1e-6, dpdy(d)));
    let facing = dot(normal, normalize(vec2<f32>(-0.45, -0.89)));
    let in_shadow = smoothstep(-0.05, 0.8, -facing) * (1.0 - smoothstep(0.0, 2.5 * ppp, inward));
    c *= 1.0 - pane.detail.z * in_shadow;

    let lit = smoothstep(-0.40, 0.85, facing);
    let rim = mix(pane.edge.y, pane.edge.x, lit) * (1.0 - smoothstep(0.15, pane.edge.z, inward));
    let x = clamp((pos.x - pane.rect.x) / max(pane.rect.z - pane.rect.x, 1.0), 0.0, 1.0);
    let top = pane.edge.w * (1.0 - smoothstep(0.0, 5.0 * ppp, pos.y - pane.rect.y)) * mix(1.0, 0.65, x);
    c = mix(c, vec3<f32>(0.38643, 0.48515, 0.63076), clamp(rim + top, 0.0, 1.0));

    let radial = length((pos.xy / pane.params.yz - 0.5) * 1.414214);
    c *= 1.0 - pane.detail.w * smoothstep(0.25, 1.0, radial);

    // ±0.5/255 dither so the flat graphite does not band.
    let n = fract(sin(dot(floor(pos.xy), vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5;
    c += vec3<f32>(n / 255.0);
    let outside = clamp(1.5 - d, 0.0, 1.0) - a;
    return vec4<f32>(c * a, a + 0.48 * outside);
}
