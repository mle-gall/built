// Sun receiver: GL's matte multiplier and a bilinear 3x3 PCF kernel, or
// GL's percentage-closer soft shadows (#1281) when a level is set.
// settings = (ground catcher, spread: penumbra width per distance behind the
// caster, metres across one map side, metres along the sun over the depth).
struct Sun { inverse_vp: mat4x4f, light_vp: mat4x4f, direction_dark: vec4f, settings: vec4f }
@group(0) @binding(0) var<uniform> sun: Sun;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var shadow: texture_depth_2d;
@group(0) @binding(3) var compare: sampler_comparison;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
 let p = array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.)); return vec4f(p[i],0.,1.);
}
fn world(uv: vec2f, d: f32) -> vec3f { let p = sun.inverse_vp*vec4f(uv.x*2.-1.,1.-uv.y*2.,d,1.); return p.xyz/p.w; }
// GL's basic.frag disc: 32 points of radius 1, the first 16 for the search.
const POISSON = array<vec2f,32>(
 vec2f(-0.6123,0.3315),vec2f(0.4211,-0.7346),vec2f(0.8622,0.3911),
 vec2f(-0.1843,-0.9318),vec2f(0.0897,0.6214),vec2f(-0.8915,-0.3426),
 vec2f(0.5503,0.0712),vec2f(-0.3364,-0.3992),vec2f(-0.0412,0.0567),
 vec2f(0.2938,0.9348),vec2f(-0.4621,0.8506),vec2f(0.9547,-0.2459),
 vec2f(-0.9618,0.1574),vec2f(0.6845,-0.6432),vec2f(0.2371,-0.3365),
 vec2f(-0.6834,-0.7175),vec2f(0.1528,0.2896),vec2f(-0.2531,0.3608),
 vec2f(0.6627,0.6876),vec2f(-0.7765,0.6151),vec2f(0.3917,0.4954),
 vec2f(-0.5398,-0.0745),vec2f(0.0174,-0.6293),vec2f(0.7614,-0.0124),
 vec2f(-0.1287,0.8634),vec2f(0.4637,-0.4215),vec2f(-0.3946,-0.8237),
 vec2f(0.9265,0.1287),vec2f(-0.8213,-0.0567),vec2f(0.1843,-0.9471),
 vec2f(-0.2145,-0.1563),vec2f(0.5638,0.2964));
// The stored depth at uv, nearest texel as GL's packed map reads it.
fn stored(uv: vec2f) -> f32 {
 let size = vec2i(textureDimensions(shadow));
 return textureLoad(shadow,clamp(vec2i(floor(uv*vec2f(size))),vec2i(0),size-1),0);
}
// GL's soft_shadow_light: a 16-tap blocker search says how far the point
// lies behind what casts it; a 32-tap disc, turned once per pixel, filters
// over a penumbra widening with that distance. Widths are in the world:
// at most half a metre, never crisper than the 3x3 PCF.
fn soft_light(uvz: vec3f, grad: vec2f, bias: f32, step: vec2f, pixel: vec2f) -> f32 {
 let a = 6.2831853*fract(52.9829189*fract(dot(pixel,vec2f(0.06711056,0.00583715))));
 let rot = mat2x2f(cos(a),sin(a),-sin(a),cos(a));
 let per_uv = sun.settings.y*sun.settings.w/sun.settings.z;
 let least = 1.5*step.x;
 let most = max(0.5/sun.settings.z,least);
 let search = clamp(per_uv*uvz.z,least,most);
 var sum = 0.; var n = 0.;
 for(var i=0;i<16;i++) {
  let off = rot*POISSON[i]*search;
  let d = stored(uvz.xy+off);
  if d < uvz.z+dot(grad,off)-bias { sum += d; n += 1.; }
 }
 if n == 0. { return 1.; }
 let pen = clamp(per_uv*(uvz.z-sum/n),least,most);
 var lit = 0.;
 for(var i=0;i<32;i++) {
  let off = rot*POISSON[i]*pen;
  lit += select(0.,1.,uvz.z+dot(grad,off)-bias <= stored(uvz.xy+off));
 }
 return lit/32.;
}
fn light(world_position: vec3f, pixel: vec2f) -> f32 {
 let lp = sun.light_vp*vec4f(world_position,1.);
 let uvz = vec3f(lp.x/lp.w*0.5+0.5,0.5-lp.y/lp.w*0.5,lp.z/lp.w);
 let dx = dpdx(uvz); let dy = dpdy(uvz); let det = dx.x*dy.y-dx.y*dy.x;
 var grad = vec2f(0.);
 if abs(det)>1e-12 { grad = vec2f(dy.y*dx.z-dx.y*dy.z,dx.x*dy.z-dy.x*dx.z)/det; }
 grad *= min(1.,8./max(length(grad),1e-10));
 let step = 1./vec2f(textureDimensions(shadow));
 let bias = 0.0006+length(grad)*step.x*1.5;
 let outside = any(uvz.xy<=vec2f(0.)) || any(uvz.xy>=vec2f(1.)) || uvz.z>=1.;
 if sun.settings.y > 0. && sun.settings.z > 0. {
  if outside { return 1.; }
  return soft_light(uvz,grad,bias,step,pixel);
 }
 var lit = 0.;
 for(var y=-1;y<=1;y++) { for(var x=-1;x<=1;x++) {
  let offset = vec2f(f32(x),f32(y))*step;
  lit += textureSampleCompareLevel(shadow,compare,uvz.xy+offset,uvz.z+dot(grad,offset)-bias);
 }}
 return select(lit/9.,1.,outside);
}
@fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
 let size = vec2f(textureDimensions(depth)); let uv = p.xy/size;
 let d = textureLoad(depth,vec2i(p.xy),0);
 // Reverse-Z (crate::depth): the near plane is depth 1, sky is 0 and a
 // perspective view's far plane is at infinity, so the ground ray ends at
 // a tiny depth instead (kilometres out, about where GL's far plane is).
 var pos = world(uv,d); let near = world(uv,1.); let far = world(uv,1e-7);
 var normal = normalize(cross(dpdx(pos),dpdy(pos)));
 // WGPU's framebuffer Y runs downward; orient toward the camera explicitly.
 if dot(normal,near-pos)<0. { normal = -normal; }
 var ground = false;
 if d<=0. {
  let ray = far-near;
  let t = (-0.005-near.z)/ray.z;
  pos = near+ray*t; normal=vec3f(0.,0.,1.); ground = true;
  if sun.settings.x == 0. || t<0. || t>1. { discard; }
 }
 let lit = light(pos,p.xy);
 var multiplier = mix(sun.direction_dark.w,1.,clamp(dot(normal,sun.direction_dark.xyz)*1.5,0.,1.)*lit);
 if ground { multiplier = 1.-(1.-sun.direction_dark.w)*(1.-lit); }
 // sRGB render targets multiply linear light; encode GL's display multiplier.
 let linear_multiplier = pow(max(multiplier,0.),2.2);
 return vec4f(vec3f(linear_multiplier),1.);
}
