//! Paper sizing shared by one-off SVG exports and saved deliveries.
//! Millimetres are the persisted value; the preset and proportion describe
//! how subsequent edits should change those dimensions.
use serde_json::{json, Value};

const PRESETS: &[(&str, [f64; 2])] = &[
    ("A4", [210., 297.]),
    ("A3", [297., 420.]),
    ("A2", [420., 594.]),
    ("A1", [594., 841.]),
    ("A0", [841., 1189.]),
];
const RATIOS: &[&str] = &["Free", "Screen", "1:1", "16:9", "4:3", "3:2", "2:1"];

fn dimension(value: f64) -> f64 {
    value.round().clamp(1., 10000.)
}

fn parse_measure(raw: &str) -> Option<f64> {
    raw.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|n| (1.0..=10000.0).contains(n))
}

pub(super) struct SvgPaper {
    mode: usize,
    preset: usize,
    ratio: usize,
    portrait: bool,
    screen: [f64; 2],
    size: [f64; 2],
    editing: [Option<String>; 2],
}

impl SvgPaper {
    pub(super) fn restore(options: &Value, screen: [f64; 2]) -> Self {
        let size = options["paper"].as_array().and_then(|a| {
            let [w, h] = a.as_slice() else { return None };
            let (w, h) = (w.as_f64()?, h.as_f64()?);
            (w > 0. && h > 0.).then_some([w, h])
        });
        let mode = if size.is_some() {
            2
        } else if options["paper"] == "screen" {
            1
        } else {
            0
        };
        let size = size.unwrap_or(screen);
        let layout = &options["paper_layout"];
        let preset = PRESETS
            .iter()
            .position(|(name, [w, h])| {
                layout["preset"] == *name && (size == [*w, *h] || size == [*h, *w])
            })
            .map_or(0, |i| i + 1);
        let ratio = RATIOS
            .iter()
            .position(|name| layout["ratio"] == *name)
            .unwrap_or(0);
        Self {
            mode,
            preset,
            ratio,
            portrait: size[0] < size[1],
            screen,
            size,
            editing: [None, None],
        }
    }

    fn proportion(&self) -> Option<f64> {
        let value = match self.ratio {
            1 => self.screen[0].max(self.screen[1]) / self.screen[0].min(self.screen[1]).max(1.),
            2 => 1.,
            3 => 16. / 9.,
            4 => 4. / 3.,
            5 => 1.5,
            6 => 2.,
            _ => return None,
        };
        Some(if self.portrait { 1. / value } else { value })
    }

    fn orient(&mut self) {
        if self.preset > 0 {
            let [w, h] = PRESETS[self.preset - 1].1;
            self.size = if self.portrait { [w, h] } else { [h, w] };
        } else if let Some(ratio) = self.proportion() {
            self.size[1] = dimension(self.size[0] / ratio);
        } else if (self.size[0] < self.size[1]) != self.portrait {
            self.size.swap(0, 1);
        }
    }

    pub(super) fn change(&mut self, id: &str, raw: &str) {
        let index = raw.parse::<usize>().ok();
        if !matches!(id, "width" | "height") {
            self.editing = [None, None];
        }
        match (id, index) {
            ("size", Some(i @ 0..=2)) => {
                if self.mode != i && i == 2 {
                    self.size = self.screen.map(dimension);
                    self.preset = 0;
                    self.ratio = 1;
                    self.portrait = self.size[0] < self.size[1];
                }
                self.mode = i;
            }
            ("paper_preset", Some(i)) if i <= PRESETS.len() => {
                self.preset = i;
                self.ratio = 0;
                self.orient();
            }
            ("paper_ratio", Some(i)) if i < RATIOS.len() => {
                self.ratio = i;
                self.orient();
            }
            ("paper_orientation", Some(i @ 0..=1)) => {
                self.portrait = i == 1;
                self.orient();
            }
            ("width" | "height", _) => {
                let side = usize::from(id == "height");
                self.editing[side] = Some(raw.to_owned());
                let Some(n) = parse_measure(raw) else { return };
                if n == self.size[side] {
                    return;
                }
                if self.preset > 0 {
                    self.preset = 0;
                    self.ratio = 0;
                }
                self.size[side] = dimension(n);
                if let Some(ratio) = self.proportion() {
                    self.editing[1 - side] = None;
                    self.size[1 - side] = dimension(if side == 0 {
                        self.size[0] / ratio
                    } else {
                        self.size[1] * ratio
                    });
                } else if self.size[0] != self.size[1] {
                    self.portrait = self.size[0] < self.size[1];
                }
            }
            _ => {}
        }
    }

    pub(super) fn options(&self) -> Value {
        match self.mode {
            1 => json!({"paper":"screen"}),
            2 => json!({"paper":self.size,"paper_layout":{
                "preset":self.preset.checked_sub(1).map(|i| PRESETS[i].0),
                "ratio":RATIOS[self.ratio]}}),
            _ => json!({}),
        }
    }
}

impl SvgPaper {
    pub(super) fn owns(&self, id: &str) -> bool {
        matches!(
            id,
            "size" | "paper_preset" | "paper_ratio" | "paper_orientation" | "width" | "height"
        )
    }

    pub(super) fn rows(&self) -> Vec<crate::ui::dialog::Field> {
        let choice = |id: &str, label: &str, i: usize, items: Vec<&str>, enabled: bool| {
            (
                id.into(),
                "choice".into(),
                label.into(),
                String::new(),
                i as f64,
                0.,
                items.len() as f64,
                items.into_iter().map(str::to_owned).collect(),
                if enabled {
                    0
                } else {
                    crate::ui::dialog::DISABLED
                },
                String::new(),
            )
        };
        let mut rows = vec![choice(
            "size",
            "Paper size",
            self.mode,
            vec!["Drawing scale", "Screen size", "Custom paper"],
            true,
        )];
        if self.mode != 2 {
            return rows;
        }
        rows.push(choice(
            "paper_preset",
            "Paper sheet",
            self.preset,
            std::iter::once("Custom")
                .chain(PRESETS.iter().map(|p| p.0))
                .collect(),
            true,
        ));
        rows.push(choice(
            "paper_ratio",
            "Proportion",
            self.ratio,
            RATIOS.to_vec(),
            self.preset == 0,
        ));
        rows.push(choice(
            "paper_orientation",
            "Orientation",
            usize::from(self.portrait),
            vec!["Landscape", "Portrait"],
            self.preset > 0 || self.proportion() != Some(1.),
        ));
        for (side, (id, label)) in [("width", "Paper width"), ("height", "Paper height")]
            .into_iter()
            .enumerate()
        {
            let text = self.editing[side]
                .clone()
                .unwrap_or_else(|| self.size[side].to_string());
            rows.push((
                id.into(),
                "number".into(),
                label.into(),
                text,
                self.size[side],
                1.,
                10000.,
                vec![" mm".into()],
                0,
                String::new(),
            ));
        }
        rows
    }

    pub(super) fn validation_error(&self) -> Option<&'static str> {
        if self.mode != 2 {
            return None;
        }
        self.editing
            .iter()
            .flatten()
            .any(|raw| parse_measure(raw).is_none())
            .then_some("Enter a paper dimension between 1 and 10,000 mm.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paper_presets_turn_and_return_to_free_measures() {
        let mut paper = SvgPaper::restore(&json!({}), [300., 200.]);
        paper.change("size", "2");
        assert_eq!(
            (paper.size, paper.preset, paper.ratio),
            ([300., 200.], 0, 1)
        );
        paper.change("paper_preset", "1");
        assert_eq!(paper.size, [297., 210.]);
        paper.change("paper_orientation", "1");
        assert_eq!(paper.size, [210., 297.]);
        paper.change("width", "250");
        assert_eq!(
            (paper.preset, paper.ratio, paper.size),
            (0, 0, [250., 297.])
        );
        assert_eq!(
            SvgPaper::restore(&paper.options(), [600., 500.]).size,
            paper.size
        );
    }

    #[test]
    fn locked_ratio_works_from_either_dimension() {
        let mut paper = SvgPaper::restore(&json!({"paper":[300,200]}), [300., 200.]);
        paper.change("paper_ratio", "3");
        paper.change("width", "320");
        assert_eq!(paper.size, [320., 180.]);
        paper.change("paper_orientation", "1");
        assert_eq!(paper.size, [320., 569.]);
        paper.change("height", "640");
        assert_eq!(paper.size, [360., 640.]);
        paper.change("paper_ratio", "2");
        assert_eq!(paper.size, [360., 360.]);
        let turn_enabled = |p: &SvgPaper| {
            p.rows()
                .into_iter()
                .find(|row| row.0 == "paper_orientation")
                .unwrap()
                .8
                == 0
        };
        assert!(!turn_enabled(&paper));
        paper.change("width", "250");
        assert_eq!(paper.size, [250., 250.]);
        paper.change("paper_ratio", "4");
        assert!(turn_enabled(&paper));
        paper.change("paper_orientation", "0");
        assert_eq!(paper.size, [250., 188.]);
        paper.change("paper_ratio", "0");
        paper.change("height", "100");
        assert_eq!(paper.size, [250., 100.]);
        paper.change("paper_orientation", "1");
        assert_eq!(paper.size, [100., 250.]);
        paper.change("width", "400");
        assert!(!paper.portrait);
        paper.change("width", "-");
        assert!(paper.validation_error().is_some());
        paper.change("height", "160");
        assert!(paper.validation_error().is_some());
        paper.change("width", "125");
        assert!(paper.validation_error().is_none());
    }
}
