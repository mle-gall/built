//! SVG appearance, framing and page controls used by both delivery editors.
use crate::ui::dialog::Field;
mod paper;
mod preview;
mod view;
use paper::SvgPaper;
use preview::Preview;
use pyo3::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const GUIDES: &[&str] = &[
    "None",
    "Thirds",
    "Golden section",
    "Grid",
    "Diagonals",
    "Centre",
];

const FACES: &[&str] = &[
    "Lines only",
    "Picture of the view",
    "Materials and textures",
    "Materials, no textures",
    "Default colour only",
];

#[pyclass]
pub struct SvgOptions {
    view: Option<view::View>,
    paper: SvgPaper,
    fill: usize,
    axes: bool,
    lighting: bool,
    shadows: bool,
    framed: bool,
    frame: [f64; 3],
    scale: f64,
    draft: BTreeMap<String, String>,
    form: Option<Py<PyAny>>,
    preview: Preview,
    guide: usize,
}

fn number(id: &str, label: &str, n: f64, limits: [f64; 2], raw: Option<&String>) -> Field {
    (
        id.into(),
        "number".into(),
        label.into(),
        raw.cloned().unwrap_or_else(|| n.to_string()),
        n,
        limits[0],
        limits[1],
        vec![],
        0,
        String::new(),
    )
}

fn check(id: &str, label: &str, on: bool) -> Field {
    (
        id.into(),
        "check".into(),
        label.into(),
        String::new(),
        f64::from(on),
        0.,
        1.,
        vec![],
        0,
        String::new(),
    )
}

impl SvgOptions {
    pub(crate) fn is_current_view(&self) -> bool {
        self.view.is_some()
    }

    fn restore(options: &Value, screen: [f64; 2]) -> Self {
        let faces = &options["faces"];
        let fill = if options["picture"] == true {
            1
        } else if faces.is_object() {
            if faces["textures"] == true {
                2
            } else if faces["materials"] == true {
                3
            } else {
                4
            }
        } else {
            0
        };
        let frame = serde_json::from_value::<[f64; 3]>(options["frame"].clone()).ok();
        Self {
            view: None,
            paper: SvgPaper::restore(options, screen),
            fill,
            axes: options["axes"].as_bool().unwrap_or(true),
            lighting: faces["as_view"].as_bool().unwrap_or(true),
            shadows: faces["shadows"] == true,
            framed: frame.is_some(),
            frame: frame.unwrap_or([0., 0., 1.]),
            scale: options["scale"].as_f64().unwrap_or(100.),
            draft: BTreeMap::new(),
            form: None,
            preview: Preview::default(),
            guide: 1,
        }
    }

    fn fields(&self) -> Vec<Field> {
        match &self.view {
            Some(view) => self.view_rows(view),
            None => self.delivery_fields(),
        }
    }

    fn delivery_fields(&self) -> Vec<Field> {
        let mut rows = vec![(
            "fill".into(),
            "choice".into(),
            "Faces".into(),
            String::new(),
            self.fill as f64,
            0.,
            FACES.len() as f64,
            FACES.iter().map(|s| s.to_string()).collect(),
            0,
            String::new(),
        )];
        if self.fill == 1 {
            rows.push(check("axes", "Show axes", self.axes));
        }
        if self.fill >= 2 {
            rows.push(check("as_view", "Viewport lighting", self.lighting));
            rows.push(check("shadows", "Shadows", self.shadows));
        }
        rows.push(check("svg_preview", "Preview", self.preview.enabled));
        if self.preview.enabled {
            rows.push((
                "svg_guides".into(),
                "choice".into(),
                "Guides".into(),
                String::new(),
                self.guide as f64,
                0.,
                GUIDES.len() as f64,
                GUIDES.iter().map(|s| s.to_string()).collect(),
                0,
                String::new(),
            ));
        }
        rows.extend(self.paper.rows());
        rows.push(check("framed", "Adjust framing", self.framed));
        if self.framed {
            for (i, (key, label)) in [
                ("pan_x", "Pan across"),
                ("pan_y", "Pan up"),
                ("zoom", "Zoom"),
            ]
            .into_iter()
            .enumerate()
            {
                rows.push(number(
                    key,
                    label,
                    self.frame[i],
                    Self::bounds(key),
                    self.draft.get(key),
                ));
            }
        }
        rows.push(number(
            "scale",
            "Scale 1 ∶",
            self.scale,
            Self::bounds("scale"),
            self.draft.get("scale"),
        ));
        rows
    }

    fn bounds(key: &str) -> [f64; 2] {
        match key {
            "scale" => [1., 100000.],
            "zoom" => [0.01, 100.],
            _ => [-100., 100.],
        }
    }

    fn record(&self) -> Value {
        let mut result = self.paper.options();
        result["scale"] = json!(self.scale);
        if self.fill == 1 {
            result["picture"] = true.into();
            result["axes"] = self.axes.into();
        } else if self.fill >= 2 {
            result["faces"] = json!({"materials":self.fill != 4, "textures":self.fill == 2,
                "as_view":self.lighting, "shadows":self.shadows});
        }
        if self.framed {
            result["frame"] = json!(self.frame);
        }
        result
    }

    fn edit(&mut self, id: &str, raw: &str) {
        if self.edit_view(id, raw) {
            return;
        }
        if self.paper.owns(id) {
            self.paper.change(id, raw);
            return;
        }
        let on = raw == "1" || raw.eq_ignore_ascii_case("true");
        match id {
            "fill" => {
                if let Ok(i) = raw.parse::<usize>() {
                    if i < FACES.len() {
                        self.fill = i;
                    }
                }
            }
            "svg_preview" => {
                self.preview.enabled = on;
                if !on {
                    self.preview.update(None);
                }
            }
            "svg_guides" => {
                if let Ok(i) = raw.parse::<usize>() {
                    if i < GUIDES.len() {
                        self.guide = i;
                    }
                }
            }
            "axes" => self.axes = on,
            "as_view" => self.lighting = on,
            "shadows" => self.shadows = on,
            "framed" => self.framed = on,
            "scale" | "zoom" | "pan_x" | "pan_y" => {
                self.draft.insert(id.into(), raw.into());
                if let Some(value) = Self::parse(id, raw) {
                    match id {
                        "scale" => self.scale = value,
                        "zoom" => self.frame[2] = value,
                        "pan_x" => self.frame[0] = value,
                        _ => self.frame[1] = value,
                    }
                }
            }
            _ => {}
        }
    }

    fn parse(key: &str, raw: &str) -> Option<f64> {
        let [lo, hi] = Self::bounds(key);
        raw.trim()
            .replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|n| (lo..=hi).contains(n))
    }
}

type Wire = (
    String,
    String,
    String,
    f32,
    Vec<Field>,
    Vec<(String, String, String, u8)>,
);

#[pymethods]
impl SvgOptions {
    #[staticmethod]
    #[pyo3(signature=(preferences,screen,axes,shadows,selection,scenes,frame_scale=None,switch=None))]
    fn current_view(
        preferences: &str,
        screen: [f64; 2],
        axes: bool,
        shadows: bool,
        selection: bool,
        scenes: Vec<String>,
        frame_scale: Option<f64>,
        switch: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        Self::from_view(
            preferences,
            screen,
            axes,
            shadows,
            selection,
            scenes,
            frame_scale,
            switch,
        )
    }

    fn destination(&self, path: &str) -> String {
        let path = path.trim();
        if path.to_ascii_lowercase().ends_with(".svg") {
            path.into()
        } else {
            format!("{path}.svg")
        }
    }

    fn validate_view(
        &self,
        form: &Bound<'_, PyAny>,
        _answer: &str,
    ) -> PyResult<Option<&'static str>> {
        let path: String = form.call_method1("value", ("destination",))?.extract()?;
        Ok(if path.trim().is_empty() {
            Some("Choose a destination.")
        } else {
            self.validation_error()
        })
    }

    fn view_preferences(&self) -> PyResult<String> {
        self.view
            .as_ref()
            .map(|v| self.saved_view_preferences(v))
            .ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("Expected a current-view export")
            })
    }

    #[new]
    fn new(options: &str, screen_mm: [f64; 2]) -> PyResult<Self> {
        let options = serde_json::from_str(options)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(Self::restore(&options, screen_mm))
    }

    fn bind(&mut self, py: Python<'_>, form: &Bound<'_, PyAny>) -> PyResult<()> {
        self.form = Some(
            py.import("weakref")?
                .getattr("ref")?
                .call1((form,))?
                .unbind(),
        );
        Ok(())
    }

    fn preview_ticket(&mut self, revision: Option<String>) -> Option<u64> {
        self.preview
            .update(revision.filter(|_| self.preview_enabled()))
    }

    #[getter]
    pub(crate) fn preview_enabled(&self) -> bool {
        self.preview.enabled && self.view.as_ref().is_none_or(|v| v.advanced)
    }

    pub(crate) fn begin_preview(&mut self, ticket: u64) -> bool {
        self.preview.begin(ticket)
    }

    fn preview_current(&self, ticket: u64) -> bool {
        self.preview.current(ticket)
    }

    pub(crate) fn finish_preview(&mut self, ticket: u64, error: String) -> bool {
        self.preview.finish(ticket, error)
    }

    /// Our SVG writer puts millimetres and scale before the layer stream. Read
    /// only that header, including when the first layer is a large photograph.
    fn preview_header(&mut self, ticket: u64, header: &str) {
        if !self.preview.current(ticket) {
            return;
        }
        let Some(v) = self.view.as_mut() else {
            return;
        };
        let page = || {
            let (_, root) = header.split_once("<svg ")?;
            let measure = |key: &str| -> Option<f64> {
                root.split_once(key)?.1.split_once("mm\"")?.0.parse().ok()
            };
            let scale = root.split_once("<desc>")?.1.split_once("</desc>")?.0;
            Some((
                measure(" width=\"")?,
                measure(" height=\"")?,
                scale.to_string(),
            ))
        };
        v.preview_page = page();
    }

    pub(crate) fn preview_caption(&self) -> Option<String> {
        let (w, h, scale) = self.view.as_ref()?.preview_page.as_ref()?;
        Some(format!("{w:.0} × {h:.0} mm · {scale}"))
    }

    fn preview_block(&self) -> (bool, String, String) {
        (
            self.preview.ready,
            GUIDES[self.guide].to_owned(),
            self.preview.error.clone(),
        )
    }

    fn move_frame(&mut self, dx: f64, dy: f64, zoom: f64) {
        if !self.preview_enabled()
            || self.view.as_ref().is_some_and(|v| !v.can_frame())
            || !dx.is_finite()
            || !dy.is_finite()
            || !zoom.is_finite()
        {
            return;
        }
        let old = if self.framed {
            self.frame
        } else {
            [0., 0., 1.]
        };
        self.framed = true;
        self.frame = [
            (old[0] - dx / old[2]).clamp(-100., 100.),
            (old[1] + dy / old[2]).clamp(-100., 100.),
            (old[2] * zoom).clamp(0.05, 50.),
        ];
        for key in ["pan_x", "pan_y", "zoom"] {
            self.draft.remove(key);
        }
    }

    fn options_json(&self) -> String {
        match &self.view {
            Some(v) => self.view_result(v),
            None => self.record(),
        }
        .to_string()
    }

    fn field(&self, id: &str) -> Option<String> {
        self.fields()
            .iter()
            .any(|f| f.0 == id)
            .then(|| id.to_owned())
    }

    fn receive(&mut self, py: Python<'_>, id: &str, raw: &str) -> PyResult<()> {
        self.edit(id, raw);
        if id == "svg_scene" {
            self.switch_scene(py)?;
        }
        if let Some(form) = &self.form {
            let form = form.bind(py).call0()?;
            if !form.is_none() {
                if self.view.is_none() && id == "fill" && self.fill == 1 {
                    form.call_method1("set", ("scope", "visible"))?;
                } else {
                    form.call_method0("changed")?;
                }
            }
        }
        Ok(())
    }

    fn validation_error(&self) -> Option<&'static str> {
        if self
            .draft
            .iter()
            .any(|(id, raw)| (self.framed || id == "scale") && Self::parse(id, raw).is_none())
        {
            return Some("Enter a value within the displayed range.");
        }
        self.paper.validation_error()
    }

    fn wire(&self, py: Python<'_>) -> PyResult<Wire> {
        let tr = py.import("ingetrazo_core.i18n")?.getattr("tr")?;
        let translate = |s: &str| tr.call1((s,))?.extract::<String>();
        let mut fields = self.fields();
        for row in &mut fields {
            row.2 = translate(&row.2)?;
            if row.1 == "choice" {
                for option in &mut row.7 {
                    *option = translate(option)?;
                }
            }
        }
        Ok((
            "svg".into(),
            String::new(),
            String::new(),
            480.,
            fields,
            vec![],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reopens_existing_delivery_appearance_and_frame() {
        let record = json!({"scale":50.0,"paper":"screen","frame":[0.1,0.2,1.5],
            "faces":{"materials":true,"textures":true,"as_view":false,"shadows":true}});
        let mut style = SvgOptions::restore(&record, [297., 210.]);
        assert_eq!(style.record(), record);
        style.edit("fill", "1");
        style.edit("axes", "0");
        assert_eq!(style.record()["picture"], true);
        assert_eq!(style.record()["axes"], false);
        assert!(style.record().get("faces").is_none());
        style.edit("fill", "0");
        assert!(style.record().get("picture").is_none());
    }
}
