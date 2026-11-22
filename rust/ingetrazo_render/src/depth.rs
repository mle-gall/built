//! Reverse-Z: the near plane is depth 1, the far plane (or infinity) 0.
//!
//! A 32-bit float has most of its precision near 0. With the usual mapping
//! (near 0, far 1) that precision sits at the near plane, a few centimetres
//! from the eye, and at 50 to 200 m the two faces of a plate a few
//! millimetres thick share depth values: the back face shows through the
//! front and flickers while orbiting. Reversed, a depth of ``n / d`` keeps
//! the same relative precision at every distance.
//!
//! Every pass that tests, biases, clears, resolves or reads the camera's
//! depth goes through this module, so "closer" has one meaning. The sun's
//! own shadow map is a separate light-space depth and keeps the usual
//! mapping.
use wgpu::{CompareFunction as C, DepthBiasState};

/// What the camera depth is cleared to: nothing drawn, infinitely far.
pub const CLEAR: f32 = 0.0;

/// The comparison that means the same as ``standard`` did with the usual
/// mapping: ``Less`` (closer wins) becomes ``Greater``, and so on.
pub const fn compare(standard: C) -> C {
    match standard {
        C::Less => C::Greater,
        C::LessEqual => C::GreaterEqual,
        C::Greater => C::Less,
        C::GreaterEqual => C::LessEqual,
        other => other,
    }
}

/// A depth bias written for the usual mapping (negative pulls toward the
/// eye), turned for reverse-Z where the eye side is the larger value.
pub const fn bias(standard: DepthBiasState) -> DepthBiasState {
    DepthBiasState { constant: -standard.constant, slope_scale: -standard.slope_scale,
                     clamp: standard.clamp }
}

/// The reversed projection, from the OpenGL-convention matrix the camera
/// sends (column major, clip z from -w at near to +w at far).
///
/// Perspective gets ``z_clip = n`` (infinite far, depth ``n / d``) unless
/// ``far_clip`` asks for the matrix's own far plane (Camera Settings ▸
/// Clipping), then ``n (f - d) / ((f - n) d)``. Near and far are recovered
/// in f64 from the matrix's own entries, so the planes are exactly where
/// OpenGL puts them. Any other projection (parallel views) is flipped:
/// ``z' = (w - z) / 2``.
pub fn projection(gl: &[f32; 16], far_clip: bool) -> glam::Mat4 {
    let p: Vec<f64> = gl.iter().map(|&v| v as f64).collect();
    let mut m = p.clone();
    let perspective = p[3] == 0. && p[7] == 0. && p[11] == -1. && p[15] == 0.
        && p[2] == 0. && p[6] == 0.;
    let (a, b) = (p[10], p[14]);
    if perspective && a != 1. && a != -1. {
        let near = b / (a - 1.);
        let far = b / (a + 1.);
        let (za, zb) = if far_clip && far.is_finite() && far > near {
            (near / (far - near), far * near / (far - near))
        } else {
            (0., near)
        };
        m[2] = 0.; m[6] = 0.; m[10] = za; m[14] = zb;
    } else {
        for col in 0..4 {
            m[col * 4 + 2] = 0.5 * (p[col * 4 + 3] - p[col * 4 + 2]);
        }
    }
    glam::Mat4::from_cols_array(&std::array::from_fn(|i| m[i] as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn depth(m: glam::Mat4, z_eye: f32) -> f32 {
        let c = m * glam::Vec4::new(0., 0., z_eye, 1.);
        c.z / c.w
    }
    #[test]
    fn near_is_one_and_far_is_zero() {
        let gl = glam::Mat4::perspective_rh_gl(0.6, 1.5, 0.1, 10000.).to_cols_array();
        let inf = projection(&gl, false);
        assert!((depth(inf, -0.1) - 1.).abs() < 1e-6);
        assert!(depth(inf, -1e6) > 0. && depth(inf, -1e6) < 1e-6);
        let fin = projection(&gl, true);
        assert!((depth(fin, -0.1) - 1.).abs() < 1e-5);
        assert!(depth(fin, -10000.).abs() < 1e-4);
        assert!(depth(fin, -10100.) < 0.);
        let ortho = glam::Mat4::orthographic_rh_gl(-1., 1., -1., 1., -5., 7.).to_cols_array();
        let o = projection(&ortho, false);
        assert!((depth(o, 5.) - 1.).abs() < 1e-6 && depth(o, -7.).abs() < 1e-6);
    }
    #[test]
    fn a_millimetre_at_two_hundred_metres_has_distinct_depths() {
        let gl = glam::Mat4::perspective_rh_gl(0.6, 1.5, 0.1, 10000.).to_cols_array();
        let m = projection(&gl, false);
        let (a, b) = (depth(m, -200.), depth(m, -200.001));
        assert!(a > b && (a - b) / (a * f32::EPSILON) > 16.);
    }
    #[test]
    fn closer_keeps_its_meaning() {
        assert_eq!(compare(C::Less), C::Greater);
        assert_eq!(compare(C::LessEqual), C::GreaterEqual);
        assert_eq!(compare(C::Greater), C::Less);
        assert_eq!(compare(C::Always), C::Always);
        assert_eq!(bias(DepthBiasState { constant: -2, slope_scale: -1., clamp: 0. }).constant, 2);
    }
}
