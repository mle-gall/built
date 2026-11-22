// World-space scaffold lines (edit box, guides, selected hidden edges):
// one 40-byte instance per segment, expanded to a screen-space quad here so
// the width is in physical pixels at any depth, depth-tested against the
// scene like GL's GL_LINES with depth writes off.
struct Camera { vp: mat4x4f, size: vec4f }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Segment {
 @location(0) a: vec3f,
 @location(1) b: vec3f,
 @location(2) color: vec4f,
 // x: width in physical pixels; y, z: dash and gap lengths in physical
 // pixels (y == 0: solid).
 @location(3) style: vec3f,
}
struct Out {
 @builtin(position) position: vec4f,
 @location(0) color: vec4f,
 // Screen-space distance from the segment's b end, interpolated without
 // perspective so it stays in pixels: the dash is a screen pattern, and
 // the camera never needs a re-upload.
 @location(1) @interpolate(linear) along: f32,
 @location(2) @interpolate(flat) dash: vec2f,
}
fn hidden() -> Out { var o: Out; o.position = vec4f(0.,0.,2.,1.); return o; }
@vertex fn vs(@builtin(vertex_index) k: u32, s: Segment) -> Out {
 var a = camera.vp * vec4f(s.a,1.);
 var b = camera.vp * vec4f(s.b,1.);
 // Reverse-Z: the near plane is z = w, so w - z is the signed distance
 // in front of it.
 let na = a.w-a.z; let nb = b.w-b.z;
 if na < 0. && nb < 0. { return hidden(); }
 // Clip at the near plane before dividing by w (as edge_vs): a segment
 // crossing it must not expand across the screen.
 if na < 0. { a = mix(a,b,-na/(nb-na)); }
 if nb < 0. { b = mix(b,a,-nb/(na-nb)); }
 let half = camera.size.xy*0.5;
 let pa = a.xy/max(a.w,1e-6)*half;
 let pb = b.xy/max(b.w,1e-6)*half;
 let d = pb-pa;
 let length_px = length(d);
 if length_px == 0. { return hidden(); }
 let dir = d/length_px;
 let normal = vec2f(-dir.y,dir.x);
 let end = k == 2u || k == 4u || k == 5u;
 let side = select(-1.,1.,k == 1u || k == 2u || k == 4u);
 // Half-pixel square caps close the corners of a box.
 let cap = 0.5*select(-1.,1.,end);
 var p = select(a,b,end);
 let offset = normal*side*s.style.x*0.5 + dir*cap;
 p = vec4f(p.xy + offset/half*p.w, p.zw);
 var o: Out;
 o.position = p;
 o.color = s.color;
 // GL measures its stipple from the provoking (last) vertex: b.
 o.along = select(length_px+0.5, -0.5, end);
 o.dash = s.style.yz;
 return o;
}
@fragment fn fs(i: Out) -> @location(0) vec4f {
 if i.dash.x > 0. {
  let period = i.dash.x + max(i.dash.y,0.);
  let t = i.along - floor(i.along/period)*period;
  if t >= i.dash.x { discard; }
 }
 let c = i.color.rgb;
 return vec4f(select(c/12.92,pow((c+0.055)/1.055,vec3f(2.4)),c>vec3f(0.04045)),i.color.a);
}
