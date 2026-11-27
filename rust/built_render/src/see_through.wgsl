// The carried objects over the finished scene (see_through.rs): their
// nearest faces at the kit's see-through opacity, their edges near solid.
override SEE_ALPHA: f32 = 0.35;
@fragment fn see_fs(i: Out, @builtin(front_facing) front: bool) -> @location(0) vec4f {
 let p = face_pixel(i, front);
 return vec4f(p.color.rgb, SEE_ALPHA);
}
@fragment fn see_edge_fs(i: EdgeOut) -> @location(0) vec4f {
 let p = edge_pixel(i);
 return vec4f(p.color.rgb, 0.85);
}
