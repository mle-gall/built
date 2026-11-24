//! Pane identities stay stable across layouts, so each camera survives a rearrangement.
mod session;
pub(crate) use session::DocumentViews;
use pyo3::{exceptions::PyValueError, prelude::*};

type Columns = &'static [&'static [usize]];
type Preset = (&'static str, &'static str, Columns, &'static [u32]);
const PRESETS: &[Preset] = &[
    ("layout:single", "One view", &[&[1]], &[1]),
    ("layout:four", "Four views", &[&[0, 2], &[1, 3]], &[1, 1]),
    ("layout:two", "Two views", &[&[0], &[1]], &[1, 1]),
    ("layout:three", "Three views", &[&[1], &[0, 2]], &[1, 1]),
    (
        "layout:two_stacked",
        "Two views, one above the other",
        &[&[1, 3]],
        &[1],
    ),
    (
        "layout:three_mirror",
        "Three views, the large one on the right",
        &[&[0, 2], &[1]],
        &[1, 1],
    ),
    (
        "layout:two_wide",
        "Two views, one wider",
        &[&[1], &[0]],
        &[2, 1],
    ),
];

pub(super) fn presets() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    PRESETS
        .iter()
        .map(|&(id, title, _, _)| (id, preset_key(id), title))
}

fn preset_key(id: &'static str) -> &'static str {
    id.strip_prefix("layout:").expect("model layout command")
}

#[pyfunction]
pub fn view_layouts() -> Vec<(&'static str, &'static str, Vec<Vec<usize>>, Vec<u32>)> {
    PRESETS
        .iter()
        .map(|&(key, title, cols, weights)| {
            (
                preset_key(key),
                title,
                cols.iter().map(|v| v.to_vec()).collect(),
                weights.to_vec(),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct Pane {
    id: usize,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[pyclass]
pub struct ModelViews {
    preset: usize,
    #[pyo3(get)]
    active: usize,
    maximized: bool,
    capture: Option<usize>,
    dividers: crate::pane_dividers::Dividers,
    panes: Vec<Pane>,
}

#[pymethods]
impl ModelViews {
    #[new]
    fn new() -> Self {
        Self {
            preset: 0,
            active: 1,
            maximized: false,
            capture: None,
            dividers: Default::default(),
            panes: vec![],
        }
    }
    #[getter]
    fn layout(&self) -> &'static str {
        preset_key(PRESETS[self.preset].0)
    }
    fn choose(&mut self, key: &str) -> PyResult<()> {
        let i = PRESETS
            .iter()
            .position(|p| preset_key(p.0) == key)
            .ok_or_else(|| PyValueError::new_err("unknown model view layout"))?;
        self.preset = i;
        if !PRESETS[i].2.iter().any(|col| col.contains(&self.active)) {
            self.active = 1;
        }
        let weights = PRESETS[i].3;
        self.dividers.shares = [
            weights[0] as f32 / weights.iter().sum::<u32>() as f32,
            0.5,
            0.5,
        ];
        self.maximized = false;
        self.cancel();
        self.panes.clear();
        Ok(())
    }
    fn toggle_maximized(&mut self) {
        if self.preset != 0 {
            self.maximized = !self.maximized;
        }
        self.cancel();
    }
    fn cancel(&mut self) {
        self.capture = None;
        self.dividers.cancel();
    }
    fn cancel_divider(&mut self) -> bool {
        self.dividers.cancel()
    }
    fn divider(
        &mut self,
        kind: &str,
        x: f32,
        y: f32,
        primary: bool,
        tolerance: f32,
    ) -> Option<&'static str> {
        if self.capture.is_some() {
            return None;
        }
        self.dividers
            .pointer(kind, egui::pos2(x, y), primary, tolerance)
    }
    fn guides(&self) -> Vec<[f32; 4]> {
        self.dividers.guides()
    }
    #[getter]
    fn shares(&self) -> [f32; 3] {
        self.dividers.shares
    }
    /// Read the existing archive schema, tolerating malformed optional fields.
    fn restore(&mut self, home: &Bound<'_, PyAny>) {
        let Some(key) = home
            .get_item("layout")
            .ok()
            .and_then(|k| k.extract::<String>().ok())
        else {
            return;
        };
        if self.choose(&key).is_err() {
            return;
        }
        let rows = home
            .get_item("rows")
            .ok()
            .filter(|r| r.len().ok() == Some(2));
        let fields = [
            home.get_item("columns").ok(),
            rows.as_ref().and_then(|r| r.get_item(0).ok()),
            rows.as_ref().and_then(|r| r.get_item(1).ok()),
        ];
        for (slot, value) in self.dividers.shares.iter_mut().zip(fields) {
            if let Some(v) = value
                .and_then(|v| v.extract::<f32>().ok())
                .filter(|&n| n > 0. && n < 1.)
            {
                *slot = v;
            }
        }
    }
    fn archive<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, pyo3::types::PyDict>>> {
        if self.preset == 0 {
            return Ok(None);
        }
        let [columns, a, b] = self.dividers.shares;
        let state = pyo3::types::PyDict::new(py);
        state.set_item("layout", self.layout())?;
        state.set_item(
            "columns",
            if PRESETS[self.preset].2.len() == 1 {
                0.5
            } else {
                columns
            },
        )?;
        state.set_item("rows", [a, b])?;
        Ok(Some(state))
    }
    fn title(&self, direction: [f64; 3], projection: &str) -> &'static str {
        if projection == "two_point" {
            return "Two-point perspective";
        }
        if projection == "perspective" {
            return "Perspective";
        }
        for (value, negative, positive) in [
            (direction[2], "Top", "Bottom"),
            (direction[1], "Back", "Front"),
            (direction[0], "Right", "Left"),
        ] {
            if value.abs() > 0.99999 {
                return if value < 0. { negative } else { positive };
            }
        }
        "Parallel projection"
    }
    fn initial_view(&self, id: usize) -> Option<&'static str> {
        if self.layout() == "two_stacked" && id == 3 {
            Some("top")
        } else {
            ["top", "perspective", "front", "right"].get(id).copied()
        }
    }
    #[getter]
    fn captured(&self) -> bool {
        self.capture.is_some() || self.dividers.dragging()
    }

    /// Logical pixels, with the same half-open rectangles used for picking and painting.
    fn arrange(
        &mut self,
        width: i32,
        height: i32,
        free_ndc: [f64; 4],
    ) -> Vec<(usize, i32, i32, i32, i32)> {
        let x = |n: f64| ((n + 1.) * 0.5 * f64::from(width)).round() as i32;
        let y = |n: f64| ((1. - n) * 0.5 * f64::from(height)).round() as i32;
        let [left, bottom, right, top] = free_ndc;
        let (left, top, w, h) = (x(left), y(top), x(right) - x(left), y(bottom) - y(top));
        self.panes.clear();
        if w <= 0 || h <= 0 {
            return vec![];
        }
        let cols = PRESETS[self.preset].2;
        self.dividers
            .configure([left, top, w, h], cols, self.maximized);
        if self.maximized {
            self.panes.push(Pane {
                id: self.active,
                x: left,
                y: top,
                width: w,
                height: h,
            });
        } else {
            let [columns, row_a, row_b] = self.dividers.shares;
            let middle = left + (w as f32 * columns).round() as i32;
            for (c, col) in cols.iter().enumerate() {
                let x = if c == 0 { left } else { middle };
                let end = if c + 1 == cols.len() {
                    left + w
                } else {
                    middle
                };
                let split = top + (h as f32 * [row_a, row_b][c]).round() as i32;
                for (r, &id) in col.iter().enumerate() {
                    let y = if r == 0 { top } else { split };
                    let bottom = if r + 1 == col.len() { top + h } else { split };
                    self.panes.push(Pane {
                        id,
                        x,
                        y,
                        width: end - x,
                        height: bottom - y,
                    });
                }
            }
        }
        self.panes
            .iter()
            .map(|p| (p.id, p.x, p.y, p.width, p.height))
            .collect()
    }
    fn pointer(&mut self, kind: &str, x: f64, y: f64, held: bool) -> Option<(usize, f64, f64)> {
        let p = self.panes.iter().find(|p| match self.capture {
            Some(id) => p.id == id,
            None => {
                x >= p.x.into()
                    && y >= p.y.into()
                    && x < (p.x + p.width).into()
                    && y < (p.y + p.height).into()
            }
        })?;
        if matches!(kind, "press" | "double" | "wheel" | "gesture") {
            self.active = p.id;
        }
        if matches!(kind, "press" | "double") {
            self.capture = Some(p.id);
        }
        if kind == "release" && !held {
            self.capture = None;
        }
        Some((p.id, x - f64::from(p.x), y - f64::from(p.y)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_drag_crosses_a_divider_and_release_restores_hit_testing() {
        let mut views = ModelViews::new();
        views.choose("two_wide").unwrap();
        assert_eq!(
            views.arrange(900, 600, [-1., -1., 1., 1.]),
            [(1, 0, 0, 600, 600), (0, 600, 0, 300, 600)]
        );
        assert_eq!(views.pointer("press", 610., 10., true), Some((0, 10., 10.)));
        assert_eq!(
            views.pointer("release", 500., 5., false),
            Some((0, -100., 5.))
        );
        assert_eq!(views.pointer("move", 500., 5., false), Some((1, 500., 5.)));
        views.toggle_maximized();
        assert_eq!(
            views.arrange(900, 600, [-1., -1., 1., 1.]),
            [(0, 0, 0, 900, 600)]
        );
        views.choose("single").unwrap();
        assert_eq!(views.active, 1);
    }
}
