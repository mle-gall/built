//! Uncommitted site parameters and the independent map camera.
use super::{crs, valid};
use pyo3::prelude::*;
use serde_json::{json, Value};
type Obj<'p> = Bound<'p, PyAny>;

pub struct Placement {
    code: u32,
    filter: String,
    fields: [String; 4],
    pub camera: (f64, f64),
    footprint: Vec<[f64; 2]>,
    controls: Vec<(String, f64, f64, f64, f64)>,
    references: usize,
    previous: String,
}

// Reduce each mesh before combining instances, so the final hull scales with
// their outlines rather than the total number of faces in the project.
fn footprint(scene: &Obj) -> PyResult<Vec<[f64; 2]>> {
    use crate::model::group_frame::hull_2d;
    use numpy::PyReadonlyArray2;
    let py = scene.py();
    let arrays = py.import("built_core.geometry_arrays")?;
    let groups = py.import("built_core.group")?;
    let mut outline = Vec::new();
    let mut add = |bundle: Obj| -> PyResult<()> {
        let pos = bundle
            .getattr("pos")?
            .call_method1("astype", ("float64",))?
            .extract::<PyReadonlyArray2<f64>>()?;
        outline.extend(hull_2d(
            pos.as_array()
                .rows()
                .into_iter()
                .map(|r| [r[0], r[1]])
                .collect(),
        ));
        Ok(())
    };
    add(arrays.call_method1("arrays_of", (scene.getattr("mesh")?,))?)?;
    for g in scene.getattr("groups")?.try_iter()? {
        let g = g?;
        let placements = groups.call_method1("iter_placements", (&g,))?;
        let bundles = arrays.call_method1("world_arrays", (g, py.None(), py.None(), placements))?;
        for bundle in bundles.try_iter()? {
            add(bundle?)?;
        }
    }
    Ok(hull_2d(outline))
}

impl Placement {
    pub fn new(scene: &Obj, camera: (f64, f64)) -> PyResult<Self> {
        let datum = scene.getattr("georef")?;
        let mut p = Self {
            code: 4326,
            filter: String::new(),
            fields: [String::new(), String::new(), "0".into(), "0".into()],
            camera,
            footprint: Vec::new(),
            controls: Vec::new(),
            references: 0,
            previous: String::new(),
        };
        let angle: f64 = scene
            .py()
            .import("built_core.sun")?
            .call_method1("scene_north_deg", (scene,))?
            .extract()?;
        p.fields[3] = angle.to_string();
        if !datum.is_none() {
            let (c, e, n) = datum
                .getattr("placement")?
                .extract::<Option<(u32, f64, f64)>>()?
                .unwrap_or((4326, camera.1, camera.0));
            p.code = c;
            p.fields = [
                e.to_string(),
                n.to_string(),
                datum.getattr("alt")?.str()?.to_string(),
                angle.to_string(),
            ];
            p.previous = format!("EPSG:{c} · {e:.6}, {n:.6} · {} m · {angle}°", p.fields[2]);
        }
        p.footprint = footprint(scene)?;
        let points = scene.getattr("geo_points")?;
        p.references = points.len()?;
        if !datum.is_none() {
            for point in points.try_iter()?.take(32) {
                let point = point?;
                let local = point.getattr("position")?;
                let (lat, lon, _): (f64, f64, f64) = datum
                    .call_method1("local_to_geodetic", (&local,))?
                    .extract()?;
                p.controls.push((
                    point.getattr("name")?.str()?.to_string(),
                    lat,
                    lon,
                    local.call_method0("x")?.extract()?,
                    local.call_method0("y")?.extract()?,
                ));
            }
        }
        let mesh = scene.getattr("photo_mesh")?;
        if !mesh.is_none() {
            p.references += 1;
            let anchor = mesh.getattr("anchor")?;
            if !anchor.is_none() && !datum.is_none() {
                let (lat, lon) = super::utm_inverse(
                    anchor.getattr("east")?.extract()?,
                    anchor.getattr("north")?.extract()?,
                    anchor.getattr("zone")?.extract()?,
                    anchor.getattr("northern")?.extract()?,
                );
                let local = datum.call_method1("geodetic_to_local", (lat, lon))?;
                p.controls.push((
                    "Scan anchor".into(),
                    lat,
                    lon,
                    local.call_method0("x")?.extract()?,
                    local.call_method0("y")?.extract()?,
                ));
            }
        }
        Ok(p)
    }
    fn set_position(&mut self, lat: f64, lon: f64) {
        if let Some((e, n)) = crs::lookup(self.code).and_then(|c| c.forward(lat, lon)) {
            let digits = if self.code == 4326 { 8 } else { 4 };
            self.fields[0] = format!("{e:.digits$}");
            self.fields[1] = format!("{n:.digits$}");
        }
    }
    pub fn position(&self) -> Option<(f64, f64)> {
        crs::lookup(self.code)?.inverse(
            self.fields[0].trim().parse().ok()?,
            self.fields[1].trim().parse().ok()?,
        )
    }
    pub fn parameters(&self) -> Option<(u32, f64, f64, f64, f64)> {
        self.position()?;
        let num = |i: usize| {
            self.fields[i]
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
        };
        Some((self.code, num(0)?, num(1)?, num(2)?, num(3)?))
    }
    pub fn use_camera(&mut self) {
        self.set_position(self.camera.0, self.camera.1);
    }
    pub fn edit(&mut self, key: &str, value: &str) {
        match key {
            "site_filter" => self.filter = value.into(),
            "site_crs" => {
                if let Some(code) = value.parse().ok().filter(|c| crs::lookup(*c).is_some()) {
                    let pos = self.position();
                    self.code = code;
                    if let Some((lat, lon)) = pos {
                        self.set_position(lat, lon);
                    }
                }
            }
            _ => {
                if let Some(i) = [
                    "site_easting",
                    "site_northing",
                    "site_elevation",
                    "site_angle",
                ]
                .iter()
                .position(|s| *s == key)
                {
                    self.fields[i] = value.into();
                    if let Some(p) = self.position() {
                        self.camera = p;
                    }
                }
            }
        }
    }
    pub fn groups(&self, tr: &Obj, query: &str) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let edit = |id: &str, label: &str, value: &str| -> PyResult<Value> {
            Ok(json!({"id":id,"kind":"edit","label":t(label)?,"value":value}))
        };
        let mut options = vec![];
        for c in std::iter::once(crs::lookup(self.code).unwrap())
            .chain(crs::search(&self.filter).filter(|c| c.code != self.code))
        {
            options.push((c.code.to_string(), format!("EPSG:{} · {}", c.code, c.name)));
        }
        let geographic = self.code == 4326;
        Ok(json!([
            ["locate",t("Locate")?,[edit("site_query","Address or lat, lon",query)?]],
            ["crs",t("Coordinate reference system")?,[edit("site_filter","Find CRS",&self.filter)?,{"id":"site_crs","kind":"choice","label":t("CRS")?,"value":self.code.to_string(),"options":options}]],
            ["base",t("Project base point")?,[
                edit("site_easting",if geographic{"Longitude (°)"}else{"Easting (m)"},&self.fields[0])?,
                edit("site_northing",if geographic{"Latitude (°)"}else{"Northing (m)"},&self.fields[1])?,
                edit("site_elevation","Elevation (m)",&self.fields[2])?,edit("site_angle","True north (°)",&self.fields[3])?]]
        ]).to_string())
    }
    fn transform(&self) -> Option<crs::MapTransform> {
        let (code, e, n, _, angle) = self.parameters()?;
        crs::MapTransform::new(code, e, n, angle)
    }
    pub fn overlay(&self) -> Vec<(f64, f64)> {
        let Some(t) = self.transform() else {
            return Vec::new();
        };
        self.footprint
            .iter()
            .filter_map(|[x, y]| t.geographic(*x, *y))
            .collect()
    }
    pub fn markers(&self) -> Vec<(f64, f64)> {
        self.controls
            .iter()
            .map(|(_, y, x, _, _)| (*y, *x))
            .collect()
    }
    pub fn check(&self, tr: &Obj) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let mut text = t(if self.footprint.is_empty() {
            "No model geometry"
        } else {
            "Model footprint"
        })?;
        if !self.controls.is_empty() {
            text += &format!("\n{}", t("Offsets from saved survey positions")?);
            if let Some(t) = self.transform() {
                for (i, (name, lat, lon, x, y)) in self.controls.iter().enumerate() {
                    if let Some((rx, ry)) = t.crs.forward(*lat, *lon) {
                        let dx = t.east + t.cos * x - t.sin * y - rx;
                        let dy = t.north + t.sin * x + t.cos * y - ry;
                        text += &format!("\n{} · {}   ΔE {dx:+.2} m   ΔN {dy:+.2} m", i + 1, name);
                    }
                }
            }
        } else {
            text += &format!(
                "\n{}",
                t(if self.references > 0 {
                    "Survey reference has no known CRS"
                } else {
                    "No survey control points"
                })?
            );
        }
        if !self.previous.is_empty() {
            text += &format!("\n{}  {}", t("Saved placement")?, self.previous);
        }
        Ok(text)
    }
    pub fn import(&mut self, path: &str) -> Result<(), String> {
        use std::io::Read;
        let mut raw = String::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(65537).read_to_string(&mut raw))
            .map_err(|e| e.to_string())?;
        if raw.len() > 65536 {
            return Err("File exceeds 64 KiB".into());
        }
        if raw.trim_start().starts_with('{') {
            #[derive(serde::Deserialize)]
            struct Calibration {
                epsg: u32,
                easting: f64,
                northing: f64,
                elevation: f64,
                north: f64,
            }
            let c: Calibration = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            let pos = crs::lookup(c.epsg)
                .and_then(|crs| crs.inverse(c.easting, c.northing))
                .ok_or("Unsupported CRS or coordinates")?;
            self.code = c.epsg;
            self.fields = [c.easting, c.northing, c.elevation, c.north].map(|v| v.to_string());
            self.camera = pos;
        } else {
            // Accept only a root EPSG authority, never the nested datum or unit identifier.
            let re = regex::Regex::new(
                r#"(?i)(?:AUTHORITY|ID)\s*\[\s*"EPSG"\s*,\s*"?(\d+)"?\s*\]\s*\]\s*$"#,
            )
            .unwrap();
            let code = re
                .captures(&raw)
                .and_then(|m| m[1].parse().ok())
                .filter(|c| crs::lookup(*c).is_some())
                .ok_or("Projection file needs a supported root EPSG identifier")?;
            self.edit("site_crs", &u32::to_string(&code));
        }
        Ok(())
    }
    pub fn search_coordinates(&mut self, query: &str) -> bool {
        if let Some((code, xy)) = query.trim().split_once(char::is_whitespace) {
            if let Some(crs) = code
                .to_ascii_uppercase()
                .strip_prefix("EPSG:")
                .and_then(|v| v.parse().ok())
                .and_then(crs::lookup)
            {
                if let Some(pos) = xy
                    .split_once(',')
                    .and_then(|(e, n)| crs.inverse(e.trim().parse().ok()?, n.trim().parse().ok()?))
                {
                    self.camera = pos;
                    return true;
                }
            }
        }
        if let Some((lat, lon)) = query
            .split_once(',')
            .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
            .filter(|(lat, lon)| valid(*lat, *lon))
        {
            self.camera = (lat, lon);
            return true;
        }
        false
    }
}
