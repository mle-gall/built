// World-space transient meshes: 28-byte vertex ABI, no resident ownership.
struct Camera { vp: mat4x4f, size: vec4f }
struct Style { front: vec4f, back: vec4f, edge: vec4f, background: vec4f, sky: vec4f, ground: vec4f, profiles: vec4f, section_plane: vec4f, fade: vec4f, raster: vec4f }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> style: Style;
struct MarkPose { matrix: mat4x4f, counts: vec4u }
@group(0) @binding(2) var<uniform> mark_pose: MarkPose;
override MARK: bool = false;
override GHOST: bool = false;
override EDGE: bool = false;
struct Out { @builtin(position) position: vec4f, @location(0) color: vec4f, @location(1) section: f32 }
@vertex fn vs(@builtin(vertex_index) index: u32, @location(0) pos: vec3f, @location(1) normal: vec3f, @location(2) color: vec4f) -> Out {
 var point = vec4f(pos, 1.);
 if mark_pose.counts.y != 0u || (MARK && select(index < mark_pose.counts.x, normal.x > 0.5, EDGE)) {
  point = mark_pose.matrix * point;
 }
 var o: Out; o.position = camera.vp * point;
 let shade = select(0.62+0.38*abs(dot(normalize(normal),normalize(vec3f(0.35,0.25,1.)))),1.,EDGE || MARK);
 let unpainted = select(style.front.rgb, style.back.rgb, mark_pose.counts.y != 0u && color.r == 1.);
 o.color = vec4f(select(color.rgb,unpainted,color.a == 0.)*shade,select(color.a,1.,color.a == 0.));
 o.section = dot(point,style.section_plane); return o;
}
@fragment fn fs(i: Out) -> @location(0) vec4f {
 if i.section < 0. { discard; }
 // The stipple is anchored to the lower left of the framebuffer.
 let pixel = vec2f(i.position.x, camera.size.y - i.position.y);
 if GHOST {
  if EDGE { if ((pixel.x+pixel.y)%6.)>=3. { discard; } }
  else { if all((pixel%vec2f(4.))>=vec2f(1.)) { discard; } }
 }
 if MARK && !EDGE && !all((pixel % vec2f(6.)) < vec2f(2.)) { discard; }
 let c = i.color.rgb;
 return vec4f(c,i.color.a);
}

struct TextureVertex { @builtin(position) position: vec4f, @location(0) material: vec4f, @location(1) section: f32 }
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var repeat_sampler: sampler;

@vertex fn texture_vs(@location(0) point: vec3f, @location(1) material: vec4f) -> TextureVertex {
 var v: TextureVertex;
 var world = vec4f(point, 1.);
 if mark_pose.counts.y != 0u { world = mark_pose.matrix * world; }
 v.position = camera.vp * world;
 v.section = dot(world, style.section_plane);
 v.material = material;
 return v;
}
@fragment fn texture_fs(v: TextureVertex) -> @location(0) vec4f {
 let texel = textureSample(image, repeat_sampler, v.material.xy);
 let alpha = texel.a * v.material.w;
 if alpha <= 0. || v.section < 0. { discard; }
 return vec4f(texel.rgb * v.material.z, alpha);
}
