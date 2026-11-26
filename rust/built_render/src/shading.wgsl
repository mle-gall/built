override TEXTURED: bool = true;
@group(2) @binding(0) var material_images: texture_2d_array<f32>;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var<storage, read> materials: array<vec4u>;
struct Recipe { images: vec4u, extra: vec4u, factors: vec4f }
@group(2) @binding(3) var<storage, read> recipes: array<Recipe>;
// GL uses display-referred colors; explicitly decode for the sRGB surface.
struct Style { front: vec4f, back: vec4f, edge: vec4f, background: vec4f, sky: vec4f, ground: vec4f, profiles: vec4f, section_plane: vec4f, fade: vec4f, raster: vec4f, light: vec4f }
@group(1) @binding(0) var<uniform> style: Style;
fn linear(c: vec3f) -> vec3f { return select(c/12.92,pow((c+0.055)/1.055,vec3f(2.4)),c>vec3f(0.04045)); }
struct Instance { world: mat4x4f, info: vec4u }
struct Camera { vp: mat4x4f, size: vec4f, eye: vec4f }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> inst: array<Instance>;
@group(0) @binding(2) var<storage, read> survivors: array<u32>;
struct Out { @location(10) world: vec3f, @location(11) normal: vec3f, @location(12) @interpolate(flat) recipe: vec2u, @location(9) opacity: f32, @location(8) section_distance: f32, @builtin(position) position: vec4f, @location(0) color: vec3f, @location(1) @interpolate(flat) id: u32, @location(2) shade: f32, @location(3) @interpolate(flat) decorated: u32, @location(7) @interpolate(linear) distance: f32, @location(4) uv: vec4f, @location(5) @interpolate(flat) back: vec4f, @location(6) @interpolate(flat) textures: vec2u }
@vertex fn vs(@location(0) pos: vec3f, @location(1) normal: vec3f, @location(2) color: vec4f, @builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> Out {
 let id = survivors[index]; let i = inst[id];
 let a=i.world[0].xyz; let b=i.world[1].xyz; let c=i.world[2].xyz;
 let n = normalize(mat3x3f(cross(b,c),cross(c,a),cross(a,b))*normal);
 var o: Out; o.position = camera.vp*i.world*vec4f(pos,1);
 o.world = (i.world * vec4f(pos,1.)).xyz; o.normal = n;
 o.section_distance = dot(i.world * vec4f(pos,1.), style.section_plane);
 o.shade = select(0.62+0.38*abs(dot(n,normalize(vec3f(0.35,0.25,1.)))), 1., (i.info.z & 8u) != 0u);
 let tint = unpack4x8unorm(i.info.y);
 // In the new ABI only byte zero is unpainted; legacy keeps its provenance test.
 let unpainted = select(color.a < 0.5, color.a == 0., style.raster.x > 0.);
 o.opacity = select(1., color.a, style.raster.x > 0. && !unpainted);
 var base = select(color.rgb, tint.rgb, unpainted);
 if unpainted && tint.a < 0.5 { base = vec3f(0.96,0.95,0.925)*style.front.rgb; }
 o.color = base * o.shade;
 if camera.size.z == 2 || camera.size.z == 3 { o.color = style.front.rgb; }
 if (i.info.w >> 24u) != 0u { o.color = unpack4x8unorm(i.info.w).rgb; }

 o.decorated = select(0u,16u,dot(a,cross(b,c)) < 0.) | (i.info.z & (70u | 256u)) | select(0u,32u,(i.info.z & 8u) != 0u) | select(0u,8u,(i.info.w >> 24u) != 0u);
 let header = materials[i.info.x];
 if header.z > 0u {
  let at = header.y + (vertex-header.x)*2u;
  o.uv = bitcast<vec4f>(materials[at]);
  let mat = materials[at+1u];
  o.back = unpack4x8unorm(mat.x); o.textures = mat.yz;
  if arrayLength(&recipes) > 1u {
   o.recipe = select(vec2u(0u), mat.yz, vec2u(recipes[mat.y].extra.z, recipes[mat.z].extra.z) != vec2u(0u));
   o.textures = vec2u(recipes[mat.y].images.x, recipes[mat.z].images.x);
  }
 }
 if (header.w & 1u) != 0u { o.back = color; }
 if style.raster.x == 0. && u32(round(color.a*255.)) == 254u { o.back = vec4f(color.rgb,1.); }
 // Unpainted faces inherit container paint on both sides.
 if unpainted && tint.a > 0.5 && o.back.a < 0.5 { o.back = tint; }
 o.distance = 0.; o.id = id+1u; return o;
}
struct Pixel { @location(0) color: vec4f, @location(1) id: u32 }
fn unit(v: vec3f) -> vec3f { return v * inverseSqrt(max(dot(v,v), 1e-20)); }

fn pbr_light(r: Recipe, base: vec3f, normal: vec3f, world: vec3f, uv: vec2f,
             dx: vec2f, dy: vec2f, px: vec3f, py: vec3f) -> vec3f {
 var n = unit(normal);
 let det = dx.x * dy.y - dx.y * dy.x;
 if r.images.y != 0u && abs(det) > 1e-15 {
  let tangent = (px * dy.y - py * dx.y) * sign(det);
  let t = unit(tangent - n * dot(n, tangent));
  let b = cross(n,t) * sign(dot(cross(n,t), (py*dx.x-px*dy.x)*sign(det)));
  let map = textureSampleGrad(material_images, material_sampler, uv, i32(r.images.y), dx, dy).rgb * 2. - 1.;
  n = unit(n * map.z + r.factors.z * (t * map.x + b * map.y * select(1., -1., r.extra.w != 0u)));
 }
 var orm = vec3f(1.);
 if r.images.z != 0u {
  orm = textureSampleGrad(material_images, material_sampler, uv, i32(r.images.z), dx, dy).rgb;
 } else {
  if r.images.w != 0u { orm.r = textureSampleGrad(material_images,material_sampler,uv,i32(r.images.w),dx,dy).r; }
  if r.extra.x != 0u { orm.g = textureSampleGrad(material_images,material_sampler,uv,i32(r.extra.x),dx,dy).r; }
  if r.extra.y != 0u { orm.b = textureSampleGrad(material_images,material_sampler,uv,i32(r.extra.y),dx,dy).r; }
 }
 let metal = clamp(orm.b*r.factors.x,0.,1.);
 let rough = clamp(orm.g*r.factors.y,0.06,1.);
 let v = unit(camera.eye.xyz - world * camera.eye.w);
 let l = unit(style.light.xyz);
 let h = unit(l+v);
 let nv = max(dot(n,v),0.001); let nl = max(dot(n,l),0.);
 let nh = max(dot(n,h),0.); let vh = max(dot(v,h),0.);
 let a2 = pow(rough,4.);
 let denom = nh*nh*(a2-1.)+1.;
 let distribution = a2 / max(3.14159*denom*denom,1e-8);
 let k = (rough+1.)*(rough+1.)/8.;
 let visibility = 0.25/((nv*(1.-k)+k)*(nl*(1.-k)+k));
 let albedo = linear(base);
 let f0 = mix(vec3f(0.04),albedo,metal);
 let fresnel = f0 + (1.-f0)*pow(1.-vh,5.);
 let diffuse = (1.-fresnel)*(1.-metal)*albedo/3.14159;
 let ambient = albedo * 0.30 * mix(1.,orm.r,r.factors.w);
 let radiance = ambient + nl*(diffuse + distribution*visibility*fresnel)*2.;
 return select(12.92*radiance,1.055*pow(radiance,vec3f(1./2.4))-0.055,radiance>vec3f(0.0031308));
}
fn face_pixel(i: Out, facing: bool) -> Pixel {
 if i.section_distance < 0. { discard; }
 let front = facing != ((i.decorated & 16u) != 0u);
 var c = i.color;
 var opacity = i.opacity;
 if !front && style.raster.x > 0. && i.back.a > 0. { opacity = i.back.a; }
 if !front && camera.size.z != 2 && (i.decorated & 8u) == 0u { c = select(style.back.rgb, i.back.rgb, i.back.a > select(0.5,0.,style.raster.x > 0.) && camera.size.z != 3) * select(i.shade,1.,camera.size.z == 3); }
 // GL X-ray keeps a painted face's colour through its back; averaging the
 // default blue-grey back into every closed coloured box washed out the tint.
 if camera.size.z == 1. && i.back.a == 0. { c = i.color; }
 let tex = select(i.textures.y,i.textures.x,front);
 let uv = select(i.uv.zw,i.uv.xy,front);
 let uv_dx = dpdx(uv); let uv_dy = dpdy(uv);
 let recipe_id = select(i.recipe.y,i.recipe.x,front);
 let base_factor = c/max(i.shade,0.001);
 let px = dpdx(i.world); let py = dpdy(i.world);
 if TEXTURED && tex != 0u && camera.size.z < 2 && ((i.decorated & 8u) == 0u || (i.decorated & 32u) != 0u) {
  var sampled = textureSampleGrad(material_images,material_sampler,uv,i32(tex),uv_dx,uv_dy);
  if camera.size.w > 0.5 {
   sampled = textureSampleLevel(material_images,material_sampler,vec2f(0.5),i32(tex),f32(textureNumLevels(material_images)-1u));
  }
  if (i.decorated & 32u) != 0u || style.raster.x == 0. {
   if sampled.a < 0.5 { discard; }
  } else { opacity *= sampled.a; }
  if opacity <= 0. { discard; }
  if (i.decorated & 8u) == 0u { c = sampled.rgb * i.shade; }
 }
 if recipe_id != 0u && camera.size.z < 2. && (i.decorated & 40u) == 0u {
  var base = c/max(i.shade,0.001);
  if tex != 0u { base *= base_factor; }
  c = pbr_light(recipes[recipe_id], base, select(-i.normal,i.normal,facing), i.world, uv, uv_dx, uv_dy, px, py);
 }
 if camera.size.z == 3. && opacity < 1. { c = vec3f(0.5); }
 // The selected task uses the pin accent (ui/tokens.rs), under selection dots.
 if (i.decorated & 256u) != 0u { c = mix(c, unpack4x8unorm(0xffffa56au).rgb * i.shade, 0.62); }
 if (i.decorated & 6u) != 0u && style.raster.y == 0. && all((vec2f(i.position.x, camera.size.y - i.position.y) % vec2f(6.)) < vec2f(2.)) {
  let surface = c / max(i.shade, 0.001);
  let high = max(surface.r,max(surface.g,surface.b));
  let low = min(surface.r,min(surface.g,surface.b));
  let delta = high-low;
  var hue = 0.;
  if delta > 0.0001 {
   if high == surface.r { hue = (surface.g-surface.b)/delta/6.; }
   else if high == surface.g { hue = ((surface.b-surface.r)/delta+2.)/6.; }
   else { hue = ((surface.r-surface.g)/delta+4.)/6.; }
   hue = fract(hue+1.);
  }
  c = select(vec3f(0.95,0.45,0.16),vec3f(0.10,0.25,0.85),
             delta/max(high,0.001) >= 0.35 && high >= 0.35 && (hue <= 0.14 || hue >= 0.96));
 }
 if (i.decorated & 64u) != 0u { c = mix(c, style.fade.rgb, style.fade.a); }
 var p: Pixel; p.color=vec4f(c,select(opacity,0.6,camera.size.z == 1)); p.id=i.id; return p;
}

// An edge carries only what its pixel reads. The faces' record on every
// vertex of every edge quad was most of what the GPU stored for a frame of
// a large model.
struct EdgeOut { @builtin(position) position: vec4f, @location(1) @interpolate(flat) id: u32, @location(3) @interpolate(flat) decorated: u32, @location(7) @interpolate(linear) distance: f32, @location(8) section_distance: f32 }
fn edge_pixel(i: EdgeOut) -> Pixel { if i.section_distance < 0. { discard; } var p: Pixel; var c = style.edge.rgb; if (i.decorated & 2u) != 0u { c = vec3f(0.95,0.45,0.16); } else if (i.decorated & 256u) != 0u { c = unpack4x8unorm(0xffffa56au).rgb; } else if (i.decorated & 4u) != 0u { c = vec3f(0.30,0.55,0.95); } if (i.decorated & 64u) != 0u { c = mix(c, style.fade.rgb, style.fade.a); } p.color=vec4f(c,1); p.id=i.id; return p; }

@fragment fn color_fs(i: Out, @builtin(front_facing) front: bool) -> @location(0) vec4f {
 let p = face_pixel(i,front);
 if style.raster.x > 0. && camera.size.z != 1. && p.color.a < 1. { discard; }
 return p.color;
}
@fragment fn color_edge_fs(i: EdgeOut) -> @location(0) vec4f { return edge_pixel(i).color; }

@fragment fn fs(i: Out, @builtin(front_facing) front: bool) -> Pixel { let p = face_pixel(i,front); if style.raster.x > 0. && camera.size.z != 1. && p.color.a < 1. { discard; } return p; }
@fragment fn edge_fs(i: EdgeOut) -> Pixel { return edge_pixel(i); }

// IDs require alpha rejection, never lighting, color conversion or selection dots.
@fragment fn depth_fs(i: Out, @builtin(front_facing) facing: bool) -> Pixel {
 if i.section_distance < 0. { discard; }
 let front = facing != ((i.decorated & 16u) != 0u);
 let tex = select(i.textures.y,i.textures.x,front);
 let uv = select(i.uv.zw,i.uv.xy,front);
 let dx = dpdx(uv); let dy = dpdy(uv);
 if TEXTURED && tex != 0u && camera.size.z < 2 && ((i.decorated & 8u) == 0u || (i.decorated & 32u) != 0u) {
  var alpha = textureSampleGrad(material_images,material_sampler,uv,i32(tex),dx,dy).a;
  if camera.size.w > 0.5 { alpha = textureSampleLevel(material_images,material_sampler,vec2f(0.5),i32(tex),f32(textureNumLevels(material_images)-1u)).a; }
  if alpha < select(0.5,0.001,style.raster.x > 0. && (i.decorated & 32u) == 0u) { discard; }
 }
 var p: Pixel; p.id = i.id; return p;
}
@fragment fn hidden_edge_fs(i: EdgeOut) -> Pixel {
 var p = edge_pixel(i);
 if camera.size.z == 1. { var c = mix(style.edge.rgb,style.background.rgb,0.6); if (i.decorated & 64u) != 0u { c = mix(c, style.fade.rgb, style.fade.a); } p.color = vec4f(c,1.); }
 else { if (i.distance % max(style.profiles.w*2.,1.)) >= style.profiles.w { discard; } }
 return p;
}

@fragment fn profile_fs(i: EdgeOut) -> Pixel {
 if (i.decorated & 16u) == 0u { discard; }
 return edge_pixel(i);
}
