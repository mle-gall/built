//! Editable modelspace geometry in paper millimetres. DXF's y axis points up.
use super::*;
use i_overlay::{
    core::{fill_rule::FillRule, overlay_rule::OverlayRule},
    float::single::SingleFloatOverlay,
};
use pyo3::types::{PyBytes, PyDict};
use std::path::Path;

struct Dxf<'p> {
    doc: Bound<'p, PyAny>,
    space: Bound<'p, PyAny>,
    height: f64,
    clip: Option<Rings>,
    path: &'p Path,
    patterns: HashMap<Vec<u64>, String>,
}
impl Dxf<'_> {
    fn point(&self, [x, y]: Point) -> (f64, f64) {
        (x, self.height - y)
    }
    fn attrs(&self, c: Rgba) -> PyResult<Bound<'_, PyDict>> {
        let attrs = PyDict::new(self.doc.py());
        attrs.set_item("true_color", u32::from_be_bytes([0, c.0, c.1, c.2]))?;
        if c.3 != 255 {
            attrs.set_item("transparency", 0x02000000u32 + u32::from(c.3))?;
        }
        Ok(attrs)
    }
    fn linetype(&mut self, p: &Pen) -> PyResult<Option<String>> {
        if p.dash.is_empty() {
            return Ok(None);
        }
        let lengths: Vec<_> = p.dash.iter().map(|v| v * p.width.max(0.01)).collect();
        let key = lengths.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        if let Some(name) = self.patterns.get(&key) {
            return Ok(Some(name.clone()));
        }
        let name = format!("DASH{}", self.patterns.len() + 1);
        let mut pattern = vec![lengths.iter().sum::<f64>()];
        pattern.extend(
            lengths
                .iter()
                .enumerate()
                .map(|(i, v)| if i % 2 == 0 { *v } else { -v }),
        );
        self.doc
            .getattr("linetypes")?
            .call_method1("add", (&name, pattern))?;
        self.patterns.insert(key, name.clone());
        Ok(Some(name))
    }
    fn clipped(&self, rings: &Rings) -> Rings {
        match &self.clip {
            None => rings.clone(),
            Some(c) => rings
                .overlay(c, OverlayRule::Intersect, FillRule::EvenOdd)
                .into_iter()
                .flatten()
                .collect(),
        }
    }
}
impl Writer for Dxf<'_> {
    fn clip(&mut self, rings: Option<Rings>) -> PyResult<()> {
        self.clip = rings;
        Ok(())
    }
    fn path(
        &mut self,
        rings: &[Vec<Point>],
        closed: bool,
        pen: Option<&Pen>,
        fill: Option<Rgba>,
    ) -> PyResult<()> {
        if let Some(color) = fill.filter(|_| closed) {
            let loops = self.clipped(&rings.to_vec());
            if !loops.is_empty() {
                let hatch = self
                    .space
                    .call_method1("add_hatch", (7, self.attrs(color)?))?;
                let paths = hatch.getattr("paths")?;
                for ring in loops.iter().filter(|r| r.len() >= 3) {
                    paths.call_method1(
                        "add_polyline_path",
                        (
                            ring.iter().map(|p| self.point(*p)).collect::<Vec<_>>(),
                            true,
                        ),
                    )?;
                }
            }
        }
        let Some(p) = pen else { return Ok(()) };
        let name = self.linetype(p)?;
        let attrs = self.attrs(p.color)?;
        if let Some(name) = name {
            attrs.set_item("linetype", name)?;
        }
        // Only these hundredths-of-a-millimetre weights are legal DXF values.
        let weights = [
            0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140,
            158, 200, 211,
        ];
        let width = (p.width * 100.).round() as i32;
        attrs.set_item(
            "lineweight",
            weights.iter().min_by_key(|v| (**v - width).abs()).unwrap(),
        )?;
        for ring in rings {
            if ring.len() < 2 {
                continue;
            }
            let count = if closed { ring.len() } else { ring.len() - 1 };
            let lines = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(count)
                .map(|(a, b)| [a[0], a[1], b[0], b[1]])
                .collect::<Vec<_>>();
            let segments = match &self.clip {
                Some(c) => crate::hatch_region::hatch_clip_segments(lines, c.clone()),
                None => lines.into_iter().map(|[x, y, u, v]| (x, y, u, v)).collect(),
            };
            for (x, y, u, v) in segments {
                self.space
                    .call_method1("add_line", (self.point([x, y]), self.point([u, v]), &attrs))?;
            }
        }
        Ok(())
    }
    fn text(
        &mut self,
        origin: Point,
        u: Point,
        value: &str,
        size: f64,
        bold: bool,
        italic: bool,
        color: Rgba,
    ) -> PyResult<()> {
        if value.is_empty() {
            return Ok(());
        }
        let a = self.attrs(color)?;
        a.set_item("insert", self.point(origin))?;
        a.set_item(
            "height",
            size * super::super::metrics::face_for(bold).cap_height / 100.,
        )?;
        a.set_item("rotation", (-u[1]).atan2(u[0]).to_degrees())?;
        a.set_item("style", if bold { "BuiltBold" } else { "BuiltRegular" })?;
        if italic {
            a.set_item("oblique", 11.3)?;
        }
        let kwargs = PyDict::new(self.doc.py());
        kwargs.set_item("dxfattribs", &a)?;
        self.space.call_method("add_text", (value,), Some(&kwargs))?;
        Ok(())
    }
    fn image(&mut self, r: [Point; 4], pic: &Picture) -> PyResult<()> {
        let bytes = png(pic)?;
        let py = self.doc.py();
        let digest: String = py
            .import("hashlib")?
            .call_method1("sha256", (PyBytes::new(py, &bytes),))?
            .call_method0("hexdigest")?
            .extract()?;
        let file = format!(
            "{}-{}.png",
            self.path.file_stem().unwrap_or_default().to_string_lossy(),
            &digest[..16]
        );
        let dest = self.path.with_file_name(&file);
        if !dest.exists() {
            std::fs::write(&dest, bytes)?;
        }
        let def = self
            .doc
            .call_method1("add_image_def", (&file, (pic.w, pic.h)))?;
        let (dx, dy) = (r[1][0] - r[0][0], r[1][1] - r[0][1]);
        let w = dx.hypot(dy);
        let h = (r[3][0] - r[0][0]).hypot(r[3][1] - r[0][1]);
        if w == 0. || h == 0. {
            return Ok(());
        }
        let outlines = self.clipped(&vec![r.to_vec()]);
        for outline in outlines {
            let im = self.space.call_method1(
                "add_image",
                (&def, self.point(r[3]), (w, h), (-dy).atan2(dx).to_degrees()),
            )?;
            if self.clip.is_some() {
                let boundary = outline
                    .iter()
                    .map(|p| {
                        let (x, y) = (p[0] - r[0][0], p[1] - r[0][1]);
                        (
                            (x * dx + y * dy) * pic.w as f64 / (w * w) - 0.5,
                            (x * (r[3][0] - r[0][0]) + y * (r[3][1] - r[0][1])) * pic.h as f64
                                / (h * h)
                                - 0.5,
                        )
                    })
                    .collect::<Vec<_>>();
                im.call_method1("set_boundary_path", (boundary,))?;
            }
        }
        Ok(())
    }
}
pub(super) fn write(
    py: Python,
    path: &Path,
    page: &Page,
    pics: &mut HashMap<String, Picture>,
) -> PyResult<()> {
    let doc = py.import("ezdxf")?.call_method1("new", ("R2010",))?;
    doc.setattr("units", 4)?;
    for (name, font) in [
        ("BuiltRegular", "Inter-Regular.ttf"),
        ("BuiltBold", "Inter-SemiBold.ttf"),
    ] {
        let kw = PyDict::new(py);
        kw.set_item("font", font)?;
        doc.getattr("styles")?
            .call_method("add", (name,), Some(&kw))?;
    }
    let space = doc.call_method0("modelspace")?;
    let mut writer = Dxf {
        space,
        doc,
        height: page.size[1],
        path,
        clip: None,
        patterns: HashMap::new(),
    };
    draw(page, pics, &mut writer)?;
    writer.doc.call_method1("saveas", (path,))?;
    Ok(())
}
