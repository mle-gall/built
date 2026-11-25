//! Scale in the grip box frame. Translating the frame has no effect on a
//! factor, so the host passes the grip, anchor and ray in local coordinates.
use glam::DVec3;

pub const FLAT: f64 = 1e-6;
pub const MIN_FACTOR: f64 = 1e-4;

pub struct Grip {
    pub extents: [f64; 3],
    pub mask: [bool; 3],
    pub uniform: bool,
}

impl Grip {
    fn scaled(&self, axis: usize) -> bool {
        if self.uniform {
            self.extents[axis] > FLAT
        } else {
            self.mask[axis]
        }
    }

    fn along_line(&self) -> bool {
        self.uniform || self.mask.iter().filter(|&&active| active).count() == 1
    }

    fn single(&self, factor: f64) -> [f64; 3] {
        std::array::from_fn(|i| if self.scaled(i) { factor } else { 1.0 })
    }

    /// A suffix gives the new absolute extent. A single size on a corner
    /// is measured along its first active axis and preserves proportions.
    pub fn typed(&self, values: &[f64], absolute: bool) -> Option<[f64; 3]> {
        if values
            .iter()
            .any(|v| !v.is_finite() || v.abs() < MIN_FACTOR)
        {
            return None;
        }
        let axes = (0..3).filter(|&i| self.scaled(i));
        let factor = |v, i: usize| {
            if !absolute {
                Some(v)
            } else if self.extents[i] > FLAT {
                Some(v / self.extents[i])
            } else {
                None
            }
        };
        if let [v] = values {
            return Some(self.single(factor(*v, axes.into_iter().next()?)?));
        }
        if axes.clone().count() != values.len() {
            return None;
        }
        let mut factors = [1.0; 3];
        for (i, v) in axes.zip(values) {
            factors[i] = factor(*v, i)?;
        }
        Some(factors)
    }

    /// All positions are relative to the anchor.
    pub fn point(&self, grip: DVec3, point: DVec3) -> Option<[f64; 3]> {
        if self.along_line() {
            let length = grip.length();
            if length < 1e-12 {
                return None;
            }
            return Some(self.single(point.dot(grip) / (length * length)));
        }
        let mut factors = [1.0; 3];
        for i in 0..3 {
            if self.mask[i] {
                if grip[i].abs() < 1e-12 {
                    return None;
                }
                factors[i] = point[i] / grip[i];
            }
        }
        Some(factors)
    }

    pub fn ray(&self, grip: DVec3, origin: DVec3, dir: DVec3) -> Option<[f64; 3]> {
        if self.along_line() {
            let length = grip.length();
            if length < 1e-12 {
                return None;
            }
            let axis = grip / length;
            let b = dir.dot(axis);
            let denom = 1.0 - b * b;
            if denom.abs() < 1e-9 {
                return None;
            }
            let distance = (axis.dot(origin) - b * dir.dot(origin)) / denom;
            return Some(self.single(distance / length));
        }
        let normal = self.mask.iter().position(|&active| !active)?;
        if dir[normal].abs() < 1e-9 {
            return None;
        }
        let t = (grip[normal] - origin[normal]) / dir[normal];
        if t < 0.0 {
            return None;
        }
        self.point(grip, origin + dir * t)
    }
}

/// Normalized box coordinates for its grips and wireframe. Flat axes have
/// just one grip coordinate, so a sheet and a line never get duplicate grips.
pub struct BoxGeometry {
    pub grips: Vec<([f64; 3], Vec<usize>)>,
    pub edges: Vec<([f64; 3], [f64; 3])>,
}

impl BoxGeometry {
    pub fn new(extents: [f64; 3]) -> Self {
        let active = extents.map(|extent| extent > FLAT);
        let count = active.iter().filter(|&&a| a).count();
        let samples = |i: usize| -> &[f64] {
            if active[i] {
                &[0.0, 0.5, 1.0]
            } else {
                &[0.5]
            }
        };
        let mut grips = Vec::new();
        for &x in samples(0) {
            for &y in samples(1) {
                for &z in samples(2) {
                    let p = [x, y, z];
                    let mask: Vec<_> = (0..3).filter(|&i| p[i] != 0.5).collect();
                    if !mask.is_empty() {
                        grips.push((p, mask));
                    }
                }
            }
        }
        let mut edges = Vec::new();
        for axis in (0..3).filter(|&i| active[i]) {
            let others: Vec<_> = (0..3).filter(|&i| i != axis).collect();
            let ends = |i: usize| -> &[f64] {
                if active[i] {
                    &[0., 1.]
                } else if count == 1 {
                    &[0.5]
                } else {
                    &[0.]
                }
            };
            for &a in ends(others[0]) {
                for &b in ends(others[1]) {
                    let mut start = [0.; 3];
                    start[others[0]] = a;
                    start[others[1]] = b;
                    let mut end = start;
                    end[axis] = 1.;
                    edges.push((start, end));
                }
            }
        }
        Self { grips, edges }
    }
}
