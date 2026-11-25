//! Divider positions are fractions of the model area, as stored in views_home.
use egui::{pos2, vec2, Pos2, Rect};

#[derive(Clone, Copy, Default)]
struct Handles {
    columns: bool,
    rows: [bool; 2],
}

impl Handles {
    fn cursor(self) -> Option<&'static str> {
        match (self.columns, self.rows.contains(&true)) {
            (true, true) => Some("move"),
            (true, false) => Some("col-resize"),
            (false, true) => Some("row-resize"),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Drag {
    handles: Handles,
    start: Pos2,
    proposed: [f32; 3],
}

pub struct Dividers {
    pub shares: [f32; 3],
    area: Rect,
    columns: bool,
    rows: [bool; 2],
    drag: Option<Drag>,
}

impl Default for Dividers {
    fn default() -> Self {
        Self {
            shares: [0.5; 3],
            area: Rect::NOTHING,
            columns: false,
            rows: [false; 2],
            drag: None,
        }
    }
}

impl Dividers {
    pub fn configure(&mut self, bounds: [i32; 4], cols: &[&[usize]], maximized: bool) {
        let [x, y, w, h] = bounds;
        self.area = Rect::from_min_size(pos2(x as f32, y as f32), vec2(w as f32, h as f32));
        self.columns = !maximized && cols.len() > 1;
        self.rows = [0, 1].map(|i| !maximized && cols.get(i).is_some_and(|c| c.len() > 1));
    }
    fn handles(&self, p: Pos2) -> Handles {
        if !self.area.contains(p) {
            return Handles::default();
        }
        let [x, y0, y1] = self.lines(self.shares);
        let columns = self.columns && (x - p.x).abs() <= 4.;
        let col = usize::from(self.columns && p.x >= x);
        let mut rows = [false; 2];
        for (i, y) in [y0, y1].into_iter().enumerate() {
            rows[i] = self.rows[i] && (i == col || columns) && (y - p.y).abs() <= 5.;
        }
        // At a crossing, moving the block brings both horizontal halves together.
        if columns && rows.contains(&true) {
            rows = self.rows;
        }
        Handles { columns, rows }
    }
    fn lines(&self, shares: [f32; 3]) -> [f32; 3] {
        [
            self.area.left() + self.area.width() * shares[0],
            self.area.top() + self.area.height() * shares[1],
            self.area.top() + self.area.height() * shares[2],
        ]
    }
    pub fn cancel(&mut self) -> bool {
        self.drag.take().is_some()
    }
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn pointer(
        &mut self,
        kind: &str,
        p: Pos2,
        primary: bool,
        tolerance: f32,
    ) -> Option<&'static str> {
        if let Some(mut drag) = self.drag {
            let diff = p - drag.start;
            let area = self.area;
            let snap = |n: f32, min: f32, size: f32, other: Option<f32>| {
                let guard = (min / size).min(0.5);
                let desired = n.clamp(guard, 1. - guard);
                [Some(0.5), other]
                    .into_iter()
                    .flatten()
                    .filter(|t| (desired - t).abs() * size <= tolerance)
                    .min_by(|a, b| (desired - a).abs().total_cmp(&(desired - b).abs()))
                    .unwrap_or(desired)
                    .clamp(guard, 1. - guard)
            };
            if drag.handles.columns {
                drag.proposed[0] = snap(
                    self.shares[0] + diff.x / area.width(),
                    80.,
                    area.width(),
                    None,
                );
            }
            for i in 0..2 {
                if drag.handles.rows[i] {
                    let y = if drag.handles.columns {
                        (p.y - area.top()) / area.height()
                    } else {
                        self.shares[1 + i] + diff.y / area.height()
                    };
                    let other =
                        (!drag.handles.columns && self.rows[1 - i]).then_some(self.shares[2 - i]);
                    drag.proposed[1 + i] = snap(y, 60., area.height(), other);
                }
            }
            if kind == "release" && primary {
                self.shares = drag.proposed;
                self.drag = None;
            } else {
                self.drag = Some(drag);
            }
            return drag.handles.cursor();
        }
        let handles = self.handles(p);
        let cursor = handles.cursor()?;
        match kind {
            "move" => Some(cursor),
            "press" if primary => {
                self.drag = Some(Drag {
                    handles,
                    start: p,
                    proposed: self.shares,
                });
                Some(cursor)
            }
            _ => None,
        }
    }
    pub fn guides(&self) -> Vec<[f32; 4]> {
        let Some(drag) = self.drag else {
            return vec![];
        };
        let [x, y0, y1] = self.lines(drag.proposed);
        let mut lines = vec![];
        if drag.handles.columns {
            lines.push([x, self.area.top(), x, self.area.bottom()]);
        }
        let middle = if self.columns { x } else { self.area.right() };
        for (i, (from, to, y)) in [
            (self.area.left(), middle, y0),
            (middle, self.area.right(), y1),
        ]
        .into_iter()
        .enumerate()
        {
            if drag.handles.rows[i] {
                lines.push([from, y, to, y]);
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resize_one_half_snap_the_other_and_cancel_a_crossing() {
        let mut d = Dividers::default();
        d.configure([0, 0, 800, 600], &[&[0, 2], &[1, 3]], false);
        d.pointer("press", pos2(100., 300.), true, 16.);
        d.pointer("move", pos2(100., 400.), false, 16.);
        assert_eq!(d.guides(), [[0., 400., 400., 400.]]);
        assert_eq!(d.shares[1], 0.5);
        d.pointer("release", pos2(100., 400.), true, 16.);
        d.pointer("press", pos2(600., 300.), true, 16.);
        d.pointer("release", pos2(600., 394.), true, 16.);
        assert_eq!(d.shares[1], d.shares[2]);
        d.pointer("press", pos2(400., 400.), true, 16.);
        d.pointer("move", pos2(750., 200.), false, 16.);
        assert_eq!(d.guides().len(), 3);
        assert!(d.cancel());
        assert_eq!(d.shares[0], 0.5);
    }
}
