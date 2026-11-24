// SPDX-License-Identifier: GPL-3.0-or-later
//! WGS84 alignment formats. Scene projection happens at insertion time.
use pyo3::{
    prelude::*,
    types::{PyBytes, PyList},
};
use roxmltree::{Document, Node};
use std::path::PathBuf;

#[derive(Default, Debug, PartialEq)]
struct Feature {
    points: Vec<(f64, f64)>,
    closed: bool,
    name: String,
}

impl Feature {
    fn finish(mut self, out: &mut Vec<Self>) {
        if self.closed && self.points.len() > 1 {
            let a = self.points[0];
            let b = self.points[self.points.len() - 1];
            if (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 {
                self.points.pop();
            }
        }
        if self.points.len() >= if self.closed { 3 } else { 2 } {
            out.push(self);
        }
    }
}

fn tag<'a>(node: Node<'_, 'a>) -> &'a str {
    node.tag_name().name()
}

fn kml(text: &str) -> Vec<Feature> {
    let Ok(doc) = Document::parse(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for placemark in doc.descendants().filter(|n| tag(*n) == "Placemark") {
        let name = placemark
            .children()
            .filter(|n| tag(*n) == "name")
            .filter_map(|n| n.text())
            .last()
            .unwrap_or("")
            .trim();
        for geom in placemark
            .descendants()
            .filter(|n| matches!(tag(*n), "LineString" | "Polygon"))
        {
            let coords = geom
                .descendants()
                .filter(|n| tag(*n) == "coordinates")
                .find_map(|n| n.text())
                .unwrap_or("");
            let points = coords
                .split_whitespace()
                .filter_map(|v| {
                    let mut fields = v.split(',');
                    let lon = fields.next()?.parse().ok()?;
                    let lat = fields.next()?.parse().ok()?;
                    Some((lat, lon))
                })
                .collect();
            Feature {
                points,
                closed: tag(geom) == "Polygon",
                name: name.to_string(),
            }
            .finish(&mut out);
        }
    }
    out
}

fn objects(py: Python<'_>, features: Vec<Feature>) -> PyResult<Bound<'_, PyList>> {
    let class = py.import("georef.geoimport")?.getattr("GeoFeature")?;
    let result = PyList::empty(py);
    for f in features {
        result.append(class.call1((f.points, f.closed, f.name))?)?;
    }
    Ok(result)
}

#[pyfunction]
fn parse_kml<'py>(py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyList>> {
    objects(py, py.detach(|| kml(text)))
}

#[pyfunction]
fn parse_kmz(py: Python<'_>, path: PathBuf) -> PyResult<Bound<'_, PyList>> {
    let archive = py.import("zipfile")?.getattr("ZipFile")?.call1((path,))?;
    let blob = (|| -> PyResult<Option<Vec<u8>>> {
        let names: Vec<String> = archive.call_method0("namelist")?.extract()?;
        let Some(name) = names.iter().find(|s| s.as_str() == "doc.kml").or_else(|| {
            names
                .iter()
                .find(|s| s.to_ascii_lowercase().ends_with(".kml"))
        }) else {
            return Ok(None);
        };
        let data = py
            .import("core.zip_limits")?
            .call_method1("read", (&archive, name))?;
        Ok(Some(data.cast::<PyBytes>()?.as_bytes().to_vec()))
    })();
    archive.call_method0("close")?;
    let Some(blob) = blob? else {
        return Ok(PyList::empty(py));
    };
    objects(py, py.detach(|| kml(&String::from_utf8_lossy(&blob))))
}

#[pyfunction]
fn open_alignment(py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
    let worker = crate::file_open::DocumentOpen::parse(move |py, progress| {
        progress.call1((0., "Reading file…"))?;
        let features = py
            .import("georef.geoimport")?
            .call_method1("load_features", (path,))?;
        progress.call1((1., "Done"))?;
        Ok(features.unbind())
    })?;
    let worker = Py::new(py, worker)?;
    py.import("app_shell.modal")?
        .call_method1("run_until", (worker.bind(py).getattr("ready")?,))?;
    let worker = worker.borrow(py);
    worker.wait(py)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(open_alignment, m)?)?;
    m.add_function(wrap_pyfunction!(parse_kml, m)?)?;
    m.add_function(wrap_pyfunction!(parse_kmz, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_mixed_geometry_preserves_order_and_skips_invalid_tokens() {
        let xml = r#"<kml xmlns="urn:kml"><Placemark><name> A &amp; B </name><MultiGeometry><Polygon><coordinates>2,45 3,46 bad 4,46 2,45</coordinates></Polygon><LineString><coordinates>5,47,100 6,48</coordinates></LineString></MultiGeometry></Placemark></kml>"#;
        assert_eq!(
            kml(xml),
            vec![
                Feature {
                    points: vec![(45., 2.), (46., 3.), (46., 4.)],
                    closed: true,
                    name: "A & B".into()
                },
                Feature {
                    points: vec![(47., 5.), (48., 6.)],
                    closed: false,
                    name: "A & B".into()
                }
            ]
        );
    }
}
