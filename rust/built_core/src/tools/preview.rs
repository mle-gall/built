//! What the renderer draws for a tool: lines with a style, shaded faces,
//! the selection box, and where the value field sits.

use super::P3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    /// The rubber band: coloured by the snap or the axis it runs along.
    Rubber,
}

impl LineStyle {
    pub fn name(self) -> &'static str {
        match self {
            LineStyle::Rubber => "rubber",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewLine {
    pub a: P3,
    pub b: P3,
    pub style: LineStyle,
}

/// Where the value field sits on the geometry, and its caption.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueAnchor {
    pub at: P3,
    pub label: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub lines: Vec<PreviewLine>,
    pub faces: Vec<Vec<P3>>,
    /// Select's box in pixels `[x0, y0, x1, y1]` (as dragged) and whether
    /// it is a crossing box.
    pub box_rect: Option<([f64; 4], bool)>,
    pub value: Option<ValueAnchor>,
}

impl Preview {
    pub fn rubber(a: P3, b: P3) -> PreviewLine {
        PreviewLine {
            a,
            b,
            style: LineStyle::Rubber,
        }
    }
}

/// A screen-sized glyph anchored in model space. The host measures its axes
/// with the current camera; geometry and gesture state belong to the tool.
pub struct CursorPlane {
    pub at: glam::DVec3,
    pub axes: [glam::DVec3; 2],
    pub radius: f64,
    pub sweep: Option<f64>,
}

impl CursorPlane {
    fn point(&self, x: f64, y: f64) -> glam::DVec3 {
        self.at + self.axes[0] * x + self.axes[1] * y
    }
    fn rim(&self, angle: f64, radius: f64) -> glam::DVec3 {
        self.point(angle.cos() * radius, angle.sin() * radius)
    }
    pub fn rectangle(&self, centered: bool) -> Preview {
        let low = if centered { -self.radius * 0.5 } else { 0. };
        let high = low + self.radius;
        let corners = [(high, high), (low, high), (low, low), (high, low)];
        let corners = if centered {
            corners
        } else {
            [(low, low), (high, low), (high, high), (low, high)]
        };
        let points = corners.map(|(x, y)| self.point(x, y));
        let mut preview = Preview::default();
        for i in 0..4 {
            preview.segment(points[i], points[(i + 1) % 4]);
        }
        preview
    }
    pub fn ring(&self, count: usize, angle: f64) -> Preview {
        let mut preview = Preview::default();
        let first = self.rim(angle, self.radius);
        let mut previous = first;
        for i in 1..count {
            let next = self.rim(
                angle + std::f64::consts::TAU * i as f64 / count as f64,
                self.radius,
            );
            preview.segment(previous, next);
            previous = next;
        }
        preview.segment(previous, first);
        preview
    }
    pub fn protractor(
        &self,
        centre: Option<P3>,
        reference: Option<P3>,
        hover: Option<P3>,
    ) -> Preview {
        let arm = centre
            .zip(reference)
            .map(|(a, b)| glam::DVec3::from(b) - glam::DVec3::from(a));
        let base = arm.map_or(0., |d| d.dot(self.axes[1]).atan2(d.dot(self.axes[0])));
        let mut preview = self.ring(48, base);
        for tick in 0..24 {
            let angle = base + std::f64::consts::TAU * tick as f64 / 24.;
            let inner = if tick % 6 == 0 { 0.75 } else { 0.86 };
            preview.segment(
                self.rim(angle, inner * self.radius),
                self.rim(angle, self.radius),
            );
        }
        if let (Some(c), Some(h)) = (centre, hover) {
            preview.segment(c.into(), h.into());
            if let Some(r) = reference {
                preview.segment(c.into(), r.into());
            }
            if let Some((arm, deg)) = arm.zip(self.sweep) {
                let steps = (deg.abs() as usize / 10 + 1).max(2);
                let mut previous = self.rim(base, arm.length() * 0.75);
                for i in 1..=steps {
                    let next = self.rim(
                        base + deg.to_radians() * i as f64 / steps as f64,
                        arm.length() * 0.75,
                    );
                    preview.segment(previous, next);
                    previous = next;
                }
            }
        }
        preview
    }
}
impl Preview {
    fn segment(&mut self, a: glam::DVec3, b: glam::DVec3) {
        self.lines.push(Self::rubber(a.into(), b.into()));
    }
}
