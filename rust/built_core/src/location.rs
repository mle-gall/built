//! WGS84 site coordinates, the editable location draft and its document edit.
pub(crate) mod crs;
mod placement;

use pyo3::{exceptions::PyValueError, prelude::*};
use serde_json::json;
use std::collections::BTreeMap;

const A: f64 = 6378137.;
const F: f64 = 1. / 298.257223563;
const E2: f64 = F * (2. - F);
const EP2: f64 = E2 / (1. - E2);
const K: f64 = 0.9996;
fn meridian(zone: i32) -> f64 {
    ((f64::from(zone) - 1.) * 6. - 177.).to_radians()
}
fn m0() -> f64 {
    1. - E2 / 4. - 3. * E2.powi(2) / 64. - 5. * E2.powi(3) / 256.
}

#[pyfunction]
pub fn zone_for_lon(lon: f64) -> i32 {
    (((lon + 180.) / 6.).floor() + 1.).clamp(1., 60.) as i32
}

#[pyfunction]
pub fn utm_forward(lat: f64, lon: f64, zone: i32) -> (f64, f64) {
    let phi = lat.to_radians();
    let n = A / (1. - E2 * phi.sin().powi(2)).sqrt();
    let t = phi.tan().powi(2);
    let c = EP2 * phi.cos().powi(2);
    let a = phi.cos() * (lon.to_radians() - meridian(zone));
    let m = A
        * (m0() * phi
            - (3. * E2 / 8. + 3. * E2.powi(2) / 32. + 45. * E2.powi(3) / 1024.) * (2. * phi).sin()
            + (15. * E2.powi(2) / 256. + 45. * E2.powi(3) / 1024.) * (4. * phi).sin()
            - 35. * E2.powi(3) / 3072. * (6. * phi).sin());
    let east = 500000.
        + K * n
            * (a + (1. - t + c) * a.powi(3) / 6.
                + (5. - 18. * t + t * t + 72. * c - 58. * EP2) * a.powi(5) / 120.);
    let north = K
        * (m + n
            * phi.tan()
            * (a * a / 2.
                + (5. - t + 9. * c + 4. * c * c) * a.powi(4) / 24.
                + (61. - 58. * t + t * t + 600. * c - 330. * EP2) * a.powi(6) / 720.));
    (east, north + if lat < 0. { 10000000. } else { 0. })
}
#[pyfunction]
pub fn utm_inverse(easting: f64, northing: f64, zone: i32, northern: bool) -> (f64, f64) {
    let y = northing - if northern { 0. } else { 10000000. };
    let mu = y / K / (A * m0());
    let e = (1. - (1. - E2).sqrt()) / (1. + (1. - E2).sqrt());
    let p = mu
        + (3. * e / 2. - 27. * e.powi(3) / 32.) * (2. * mu).sin()
        + (21. * e.powi(2) / 16. - 55. * e.powi(4) / 32.) * (4. * mu).sin()
        + 151. * e.powi(3) / 96. * (6. * mu).sin()
        + 1097. * e.powi(4) / 512. * (8. * mu).sin();
    let c = EP2 * p.cos().powi(2);
    let t = p.tan().powi(2);
    let den = 1. - E2 * p.sin().powi(2);
    let n = A / den.sqrt();
    let r = A * (1. - E2) / den.powf(1.5);
    let d = (easting - 500000.) / (n * K);
    let lat = p
        - (n * p.tan() / r)
            * (d * d / 2. - (5. + 3. * t + 10. * c - 4. * c * c - 9. * EP2) * d.powi(4) / 24.
                + (61. + 90. * t + 298. * c + 45. * t * t - 252. * EP2 - 3. * c * c) * d.powi(6)
                    / 720.);
    let lon = meridian(zone)
        + (d - (1. + 2. * t + c) * d.powi(3) / 6.
            + (5. - 2. * c + 28. * t - 3. * c * c + 8. * EP2 + 24. * t * t) * d.powi(5) / 120.)
            / p.cos();
    (lat.to_degrees(), lon.to_degrees())
}

fn valid(lat: f64, lon: f64) -> bool {
    (-85.0..=85.).contains(&lat) && (-180.0..=180.).contains(&lon)
}

#[pyclass]
pub struct LocationDraft {
    placement: Option<placement::Placement>,
    #[pyo3(get, set)]
    offline: bool,
    values: BTreeMap<String, String>,
    #[pyo3(get)]
    map: bool,
    #[pyo3(get)]
    query: String,
    #[pyo3(get, set)]
    note: String,
    zoom: u8,
    revision: u64,
    drawing: bool,
    limited: bool,
    corners: Option<[f64; 4]>,
    places: Vec<(String, f64, f64)>,
    #[pyo3(get)]
    mode: String,
}
#[pymethods]
impl LocationDraft {
    #[new]
    fn new(lat: f64, lon: f64, mode: &str) -> Self {
        let mut draft = Self {
            placement: None,
            offline: false,
            values: Default::default(),
            map: false,
            query: String::new(),
            note: String::new(),
            zoom: 12,
            revision: 0,
            drawing: false,
            limited: false,
            corners: None,
            places: Vec::new(),
            mode: if mode == "utm" { "utm" } else { "geo" }.into(),
        };
        draft.center(lat, lon);
        draft
    }
    fn start_placement(&mut self, scene: &Bound<'_, PyAny>, lat: f64, lon: f64) -> PyResult<()> {
        self.placement = Some(placement::Placement::new(scene, (lat, lon))?);
        self.map = true;
        self.zoom = 19;
        Ok(())
    }
    #[getter]
    fn placing(&self) -> bool {
        self.placement.is_some()
    }
    fn parameters(&self) -> Option<(u32, f64, f64, f64, f64)> {
        self.placement.as_ref()?.parameters()
    }
    fn use_camera(&mut self) {
        if let Some(p) = &mut self.placement {
            p.use_camera();
        }
    }
    fn search_coordinates(&mut self) -> bool {
        self.placement
            .as_mut()
            .is_some_and(|p| p.search_coordinates(&self.query))
    }
    fn load_placement(&mut self, path: &str) {
        if let Some(p) = &mut self.placement {
            self.note = match p.import(path) {
                Ok(()) => "File loaded for review".into(),
                Err(e) => e,
            };
        }
    }
    fn placement_check(&self, tr: &Bound<'_, PyAny>) -> PyResult<String> {
        self.placement
            .as_ref()
            .map(|p| p.check(tr))
            .unwrap_or_else(|| Ok(String::new()))
    }
    fn center(&mut self, lat: f64, lon: f64) {
        if !valid(lat, lon) {
            return;
        }
        let zone = zone_for_lon(lon);
        let (east, north) = utm_forward(lat, lon, zone);
        self.values.extend([
            ("lat".into(), format!("{lat:.6}")),
            ("lon".into(), format!("{lon:.6}")),
            ("zone".into(), zone.to_string()),
            ("hemisphere".into(), if lat < 0. { "S" } else { "N" }.into()),
            ("easting".into(), format!("{east:.2}")),
            ("northing".into(), format!("{north:.2}")),
        ]);
    }
    fn position(&self) -> Option<(f64, f64)> {
        if let Some(p) = &self.placement {
            return p.position();
        }
        let number = |key: &str| {
            self.values
                .get(key)?
                .trim()
                .replace(',', ".")
                .parse::<f64>()
                .ok()
        };
        let (lat, lon) = if self.mode == "utm" {
            let zone = self.values.get("zone")?.parse::<i32>().ok()?;
            let east = number("easting")?;
            let north = number("northing")?;
            if !(1..=60).contains(&zone)
                || !(100000.0..=900000.).contains(&east)
                || !(0.0..=10000000.).contains(&north)
            {
                return None;
            }
            utm_inverse(east, north, zone, self.values.get("hemisphere")? == "N")
        } else {
            (number("lat")?, number("lon")?)
        };
        valid(lat, lon).then_some((lat, lon))
    }
    fn moved(&self, lat: f64, lon: f64) -> bool {
        self.selected().is_some_and(|(y, x, _, _)| {
            (y - lat).abs() > 0.00000051 || (x - lon).abs() > 0.00000051
        })
    }
    fn edit(&mut self, key: &str, value: &str) -> bool {
        match key {
            "site_query" => {
                self.query = value.into();
                return false;
            }
            "site_draw" => {
                self.drawing = matches!(value, "True" | "true" | "1");
                return false;
            }
            _ => (),
        }
        if let Some(p) = &mut self.placement {
            p.edit(key, value);
            return false;
        }
        if key == "site_mode" && ["geo", "utm"].contains(&value) && self.mode != value {
            let Some((lat, lon)) = self.position() else {
                return false;
            };
            self.center(lat, lon);
            self.mode = value.into();
            return true;
        }
        if let Some(field) = key
            .strip_prefix("site_")
            .and_then(|key| self.values.get_mut(key))
        {
            *field = value.into();
            self.corners = None;
        }
        false
    }
    fn groups(&self, tr: &Bound<'_, PyAny>) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        if let Some(p) = &self.placement {
            return p.groups(tr, &self.query);
        }
        let mut rows = vec![
            json!({"id":"site_mode","kind":"choice","label":t("Coordinates:")?,"value":self.mode,
            "options":[["geo",t("Geographic (lat/lon)")?],["utm",t("UTM WGS84")?]]}),
        ];
        let fields: &[(&str, &str)] = if self.mode == "geo" {
            &[("lat", "Latitude"), ("lon", "Longitude")]
        } else {
            &[
                ("zone", "Zone:"),
                ("hemisphere", "Hemisphere"),
                ("easting", "Easting (m)"),
                ("northing", "Northing (m)"),
            ]
        };
        for (key, label) in fields {
            let mut row = json!({"id":format!("site_{key}"),"kind":"edit","label":t(label)?,"value":self.values.get(*key)});
            if *key == "hemisphere" {
                row["kind"] = json!("choice");
                row["options"] = json!([["N", t("North")?], ["S", t("South")?]]);
            }
            rows.push(row);
        }
        let mut groups = vec![json!(["site", t("Model origin (0,0)")?, rows])];
        if self.map {
            groups.push(json!(["site_search_group",t("Map")?,[
                {"id":"site_query","kind":"edit","label":t("Place or address")?,"value":self.query},
                {"id":"site_draw","kind":"check","label":t("Draw capture area (drag a rectangle on the map)")?,"value":self.drawing}]]));
        }
        Ok(json!(groups).to_string())
    }
    fn map_toggle(&mut self) {
        self.map = !self.map && self.position().is_some();
        if !self.map {
            self.corners = None;
            self.drawing = false;
            self.note.clear();
        }
    }
    fn move_pin(&mut self, lat: f64, lon: f64, zoom: u8) {
        if valid(lat, lon) {
            if let Some(p) = &mut self.placement {
                p.camera = (lat, lon);
            } else {
                self.center(lat, lon);
            }
            self.zoom = zoom.clamp(1, 22);
        }
    }
    fn bounds(&mut self, coords: [f64; 4]) {
        let [a, b, c, d] = coords;
        if !valid(a, b) || !valid(c, d) {
            return;
        }
        let dy = c - a;
        let dx = (d - b + 180.).rem_euclid(360.) - 180.;
        if dy.abs() < 1e-7 || dx.abs() < 1e-7 {
            self.corners = None;
            return;
        }
        let area = dy.abs() * dx.abs() * 111.32_f64.powi(2) * a.to_radians().cos();
        let factor = (100. / area).sqrt().min(1.);
        self.limited = factor < 1.;
        self.corners = Some([
            a,
            b,
            a + dy * factor,
            (b + dx * factor + 180.).rem_euclid(360.) - 180.,
        ]);
    }
    fn selected(&self) -> Option<(f64, f64, Option<f64>, Option<f64>)> {
        let (lat, lon) = self.position()?;
        if let Some([a, b, c, d]) = self.corners {
            let dx = (d - b + 180.).rem_euclid(360.) - 180.;
            let lat = (a + c) / 2.;
            let w = (dx.abs() * 111320. * lat.to_radians().cos()).max(200.);
            let h = ((c - a).abs() * 111320.).max(200.);
            return Some((
                lat,
                (b + dx / 2. + 180.).rem_euclid(360.) - 180.,
                Some(w),
                Some(h),
            ));
        }
        Some((lat, lon, None, None))
    }
    fn places(&mut self, items: Vec<(String, f64, f64)>) {
        self.places = items
            .into_iter()
            .filter(|(_, y, x)| valid(*y, *x))
            .take(8)
            .collect();
        self.note = if self.places.is_empty() {
            "No matches. Pan the map to your site, or type coordinates."
        } else {
            ""
        }
        .into();
    }
    fn choose(&mut self, verb: &str) -> bool {
        let Some(i) = verb
            .strip_prefix("site_place_")
            .and_then(|s| s.parse::<usize>().ok())
        else {
            return false;
        };
        if let Some((name, y, x)) = self.places.get(i).cloned() {
            self.move_pin(y, x, 15);
            self.note = name;
            self.places.clear();
            self.corners = None;
        }
        true
    }
    fn tile_ready(&mut self) {
        self.revision += 1;
    }
    fn map_blocks(
        &self,
        tr: &Bound<'_, PyAny>,
        source: &str,
        cache: &str,
        max_zoom: u8,
        attribution: &str,
    ) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let mut blocks = vec![];
        if let Some(p) = &self.placement {
            let (lat, lon) = p.camera;
            blocks.push(json!({"kind":"map","map":{"placement":true,"pin":p.position(),"footprint":p.overlay(),"markers":p.markers(),"offline":if self.offline{t("Basemap offline · coordinates remain editable")?}else{t("Waiting for basemap")?},"lat":lat,"lon":lon,"source":source,"cache":cache,"zoom":self.zoom.min(max_zoom),"max_zoom":max_zoom,"origin":t("Base point")?,"attribution":attribution,"drawing":false,"limited":false,"revision":self.revision}}));
            blocks.push(json!({"kind":"buttons","align":"left","buttons":[
                {"id":"site_search","label":t("Search")?,"enabled":!self.query.trim().is_empty()},
                {"id":"site_use_camera","label":t("Use map centre")?}
            ]}));
            if !self.places.is_empty() {
                let matches: Vec<_> = self
                    .places
                    .iter()
                    .enumerate()
                    .map(|(i, (name, _, _))| json!({"id":format!("site_place_{i}"),"label":name}))
                    .collect();
                blocks.push(json!({"kind":"buttons","align":"fill","buttons":matches}));
            }
            return Ok(json!(blocks).to_string());
        }
        if self.map {
            let mut verbs = vec![
                json!({"id":"site_search","label":t("Search")?,"enabled":!self.query.trim().is_empty()}),
                json!({"id":"site_locate","label":t("Locate me")?}),
            ];
            blocks.push(json!({"kind":"buttons","align":"left","buttons":verbs}));
            verbs = Vec::new();
            for (i, (name, _, _)) in self.places.iter().enumerate() {
                verbs.push(json!({"id":format!("site_place_{i}"),"label":name}));
            }
            if !verbs.is_empty() {
                blocks.push(json!({"kind":"buttons","align":"fill","buttons":verbs}));
            }
            if let Some((lat, lon)) = self.position() {
                blocks.push(json!({"kind":"map","map":{"lat":lat,"lon":lon,"source":source,"cache":cache,
                    "zoom":self.zoom.min(max_zoom),"max_zoom":max_zoom,"origin":t("origin (0,0)")?,"attribution":attribution,
                    "drawing":self.drawing,"corners":self.corners,"limited":self.limited,"revision":self.revision}}));
            }
            if !self.note.is_empty() {
                blocks.push(json!({"kind":"text","text":t(&self.note)?}));
            }
            if let Some((_, _, Some(w), Some(h))) = self.selected() {
                let size = t("Capture area: {w} × {l} m")?
                    .replace("{w}", &format!("{w:.0}"))
                    .replace("{l}", &format!("{h:.0}"));
                let extra = if self.limited {
                    "(maximum area reached)"
                } else if w * h > 25e6 {
                    "(large — detail will be reduced to stay fast)"
                } else {
                    ""
                };
                blocks.push(json!({"kind":"text","text":format!("{size} {}",t(extra)?).trim()}));
            }
        }
        if self.position().is_none() {
            blocks.push(json!({"kind":"text","tone":"error","text":t(if self.mode=="utm"{"Enter a UTM zone from 1 to 60 and valid easting and northing."}else{"Enter latitude from −85 to 85 and longitude from −180 to 180."})?}));
        }
        Ok(json!(blocks).to_string())
    }
    fn buttons(&self, tr: &Bound<'_, PyAny>) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        Ok(json!([{"id":"site_map","label":t(if self.map{"Hide map"}else{"Pick on map…"})?,"enabled":self.map||self.position().is_some()},{"id":"site_cancel","label":t("Cancel")?},{"id":"site_apply","label":t("Apply")?,"role":1,"enabled":self.position().is_some()}]).to_string())
    }
}

/// Georeferencing lives on the scene host until the remaining document fields
/// are native. History calls the same edit in both directions.
#[pyclass]
pub struct LocationEdit {
    old: Py<PyAny>,
    new: Py<PyAny>,
    terrain: Py<PyAny>,
}
#[pymethods]
impl LocationEdit {
    #[new]
    #[pyo3(signature=(scene,lat,lon,placement=None))]
    fn new(
        scene: &Bound<'_, PyAny>,
        lat: f64,
        lon: f64,
        placement: Option<(u32, f64, f64, f64, f64)>,
    ) -> PyResult<Self> {
        if !valid(lat, lon) {
            return Err(PyValueError::new_err("Invalid site coordinates"));
        }
        let py = scene.py();
        let old = scene.getattr("georef")?;
        let north = py
            .import("built_core.sun")?
            .call_method1("scene_north_deg", (scene,))?;
        let alt = if old.is_none() {
            0.
        } else {
            let dy = lat - old.getattr("lat")?.extract::<f64>()?;
            let dx = (lon - old.getattr("lon")?.extract::<f64>()? + 180.).rem_euclid(360.) - 180.;
            if dy.hypot(dx * lat.to_radians().cos()) * 111320. < 1000. {
                old.getattr("alt")?.extract()?
            } else {
                0.
            }
        };
        let new = py
            .import("built_core.georef.datum")?
            .getattr("SceneDatum")?;
        let new = if let Some((code, e, n, z, angle)) = placement {
            let (lat, lon) = crs::lookup(code)
                .and_then(|c| c.inverse(e, n))
                .ok_or_else(|| PyValueError::new_err("Invalid site coordinates"))?;
            new.call1((lat, lon, z, angle, (code, e, n)))?
        } else {
            new.call1((lat, lon, alt, north))?
        };
        Ok(Self {
            old: old.unbind(),
            new: new.unbind(),
            terrain: scene.getattr("terrain")?.unbind(),
        })
    }
    fn apply(&self, scene: &Bound<'_, PyAny>, forward: bool) -> PyResult<()> {
        scene.setattr("georef", if forward { &self.new } else { &self.old })?;
        if forward {
            scene.setattr("terrain", scene.py().None())?
        } else {
            scene.setattr("terrain", &self.terrain)?
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utm_round_trip_matches_the_survey_in_both_hemispheres() {
        for (lat, lon) in [(-15.8402, -70.0219), (48.85, 2.35), (0., 180.)] {
            let zone = zone_for_lon(lon);
            let (e, n) = utm_forward(lat, lon, zone);
            let (y, x) = utm_inverse(e, n, zone, lat >= 0.);
            assert!((y - lat).abs() < 1e-7 && (x - lon).abs() < 1e-7);
        }
        assert_eq!(zone_for_lon(180.), 60);
    }
    #[test]
    fn capture_wraps_at_the_date_line_and_limits_area_without_shortening_thin_strips() {
        let mut site = LocationDraft::new(0., 179.99, "geo");
        site.bounds([0., 179.99, 0.1, -179.99]);
        let (lat, lon, Some(w), Some(h)) = site.selected().unwrap() else {
            panic!()
        };
        assert!((lat - 0.05).abs() < 1e-8 && lon.abs() > 179.999);
        assert!((w - 2226.4).abs() < 1. && (h - 11132.).abs() < 1.);
        site.bounds([0., 0., 2., 1.]);
        let (_, _, Some(w), Some(h)) = site.selected().unwrap() else {
            panic!()
        };
        assert!(site.limited && w * h <= 100e6 && w * h > 99e6);
        assert!((h / w - 2.).abs() < 0.001);
    }
}
