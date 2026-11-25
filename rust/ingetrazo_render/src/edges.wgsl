override PROFILES: bool = true;
// Classify in projected space so perspective, orthographic and mirrored placements
// use the same face-side test without a camera-dependent CPU topology upload.
fn facing(p: vec4f, q: vec4f, r: vec4f) -> f32 {
 // Homogeneous signed area avoids divisions at the near plane.
 return determinant(mat3x3f(p.xyw,q.xyw,r.xyw));
}
// Each endpoint carries its opposite endpoint, so expansion remains constant
// in physical pixels under perspective and nonuniform instance transforms.
// Scalar arrays keep this storage record tightly packed at 48 bytes.
struct Edge { a: array<f32,3>, b: array<f32,3>, normals: array<f32,6> }
@group(0) @binding(3) var<storage, read> edges: array<Edge>;
struct EdgePlacement { eye: vec4f, transform: mat4x4f }
@group(0) @binding(4) var<storage, read> viewpoints: array<EdgePlacement>;
@vertex fn edge_vs(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> EdgeOut {
 // Four vertices a quad, drawn as two triangles through the scene's quad
 // indices (0 1 2, 0 2 3): corner 0 and 1 at this end, 2 and 3 at the other.
 let first = vertex / 4u;
 let edge = edges[first];
 let id = survivors[index];
 let k = vertex % 4u;
 let end = k >= 2u;
 let side_bit = k == 1u || k == 2u;
 let a0 = vec3f(edge.a[0],edge.a[1],edge.a[2]);
 let b0 = vec3f(edge.b[0],edge.b[1],edge.b[2]);
 let pos = select(a0,b0,end);
 let other = select(b0,a0,end);
 let packed_normal = vec3f(edge.normals[0],edge.normals[1],edge.normals[2]);
 // The hard-only indirect template never includes soft records. Specializing
 // this flag removes normal fetches as well as profile arithmetic.
 let soft = PROFILES && dot(packed_normal,packed_normal) > 2.;
 let n0 = packed_normal * select(1.,0.5,soft);
 let n1 = vec3f(edge.normals[3],edge.normals[4],edge.normals[5]);
 let corner = vec4f(select(0.,1.,side_bit),select(0.,1.,end),select(0.,1.,soft),0.);
 let transform = viewpoints[id].transform;
 var a = transform * vec4f(pos, 1.);
 var b = transform * vec4f(other, 1.);
 var profile = false;
 // Hard edges need classification only when it can change their width.
 if PROFILES && style.profiles.x > 0. && (corner.b > 0.5 || style.profiles.y > style.edge.w) {
  let boundary = dot(n0,n0) > 0. && dot(n1,n1) == 0.;
  let eye = viewpoints[id].eye;
  if eye.w <= 1. {
   let direction = eye.xyz - pos * eye.w;
   profile = boundary || ((dot(n0,direction)>0.) != (dot(n1,direction)>0.));
  } else {
   // Singular placements retain the projected-area fallback.
   let r0 = transform * vec4f(pos + cross(n0,other-pos),1.);
   let r1 = transform * vec4f(pos + cross(n1,other-pos),1.);
   profile = boundary || ((facing(a,b,r0)>0.) != (facing(a,b,r1)>0.));
  }
 }
 if corner.b > 0.5 && !profile {
  var hidden: EdgeOut;
  hidden.position = vec4f(0.,0.,2.,1.);
  return hidden;
 }
 // Clip at the near plane before dividing by w; crossing edges must not
 // expand across the screen or flip their normal behind the camera.
 // Reverse-Z: the near plane is z = w, so w - z is the signed distance
 // in front of it.
 let na = a.w-a.z; let nb = b.w-b.z;
 if na < 0. && nb >= 0. { a = mix(a,b,-na/(nb-na)); }
 if nb < 0. && na >= 0. { b = mix(b,a,-nb/(na-nb)); }
 let direction = (b.xy/max(b.w,1e-6)-a.xy/max(a.w,1e-6))*camera.size.xy;
 // Reject zero-area edges after clipping. A nonzero subpixel threshold
 // changes coverage, including at the default width; retain those edges.
 if length(direction) == 0. {
  var hidden: EdgeOut; hidden.position = vec4f(0.,0.,2.,1.); return hidden;
 }
 let normal = vec2f(-direction.y,direction.x)/max(length(direction),1e-6);
 let width = select(style.edge.w,max(style.edge.w,style.profiles.y),profile && style.profiles.x > 0.);
 let side = (corner.r*2.-1.)*(1.-2.*corner.g);
 // Half-pixel square caps overlap adjacent segments at a rim; the old
 // butt caps left uncovered MSAA samples at each independently expanded join.
 // Do not inflate subpixel machinery into a pixel-long dash: the ride
 // contains millions of those segments. Only visible (>4 px) joins need caps.
 let cap = direction/length(direction)/camera.size.xy*a.w*select(0.,1.,length(direction)>8.);
 a = vec4f(a.xy + normal*side*width/camera.size.xy*a.w - cap,a.zw);
 var o: EdgeOut;
 o.section_distance = dot(inst[id].world * vec4f(pos,1.), style.section_plane);
 o.distance = (1. - corner.g) * length(direction) * 0.5;
 o.position = a; o.id = id+1u;
 o.decorated = (inst[id].info.z & 70u) | select(0u,16u,corner.b > 0.5 && profile && style.profiles.x > 0.);
 return o;
}
