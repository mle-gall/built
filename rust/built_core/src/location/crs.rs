//! Common EPSG definitions (PROJ database), in traditional GIS x/y order.
use proj4rs::{adaptors::transform_vertex_2d, Proj};
use std::sync::LazyLock;

pub struct Crs {
    pub code: u32,
    pub name: String,
    projection: Proj,
}

pub static CATALOG: LazyLock<Vec<Crs>> = LazyLock::new(|| {
    let mut rows = Vec::new();
    let mut add = |code, name, spec: &str| {
        rows.push(Crs {
            code,
            name,
            projection: Proj::from_proj_string(spec).expect("bundled EPSG definition"),
        })
    };
    let definitions: Vec<(u32, String, &str)> =
        serde_json::from_str(include_str!("crs.json")).expect("bundled CRS catalog");
    for (code, name, definition) in definitions {
        add(code, name, definition);
    }
    for (base, suffix, hemisphere) in [(32600, "N", ""), (32700, "S", " +south")] {
        for zone in 1..=60 {
            add(
                base + zone,
                format!("WGS 84 / UTM zone {zone}{suffix}"),
                &format!("+proj=utm +zone={zone}{hemisphere} +datum=WGS84 +units=m"),
            );
        }
    }
    rows
});

pub fn lookup(code: u32) -> Option<&'static Crs> {
    CATALOG.iter().find(|c| c.code == code)
}

pub fn search(query: &str) -> impl Iterator<Item = &'static Crs> {
    let q = query
        .trim()
        .to_lowercase()
        .replace("epsg:", "")
        .replace(' ', "");
    CATALOG.iter().filter(move |c| {
        format!("{}{}", c.code, c.name)
            .to_lowercase()
            .replace(' ', "")
            .contains(&q)
    })
}

impl Crs {
    pub fn forward(&self, lat: f64, lon: f64) -> Option<(f64, f64)> {
        if self.code == 4326 {
            return Some((lon, lat));
        }
        transform_vertex_2d(
            &CATALOG[0].projection,
            &self.projection,
            (lon.to_radians(), lat.to_radians()),
        )
        .ok()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
    }
    pub fn inverse(&self, east: f64, north: f64) -> Option<(f64, f64)> {
        let (lat, lon) = if self.code == 4326 {
            (north, east)
        } else {
            let (lon, lat) =
                transform_vertex_2d(&self.projection, &CATALOG[0].projection, (east, north))
                    .ok()?;
            (lat.to_degrees(), lon.to_degrees())
        };
        super::valid(lat, lon).then_some((lat, lon))
    }
}

/// A rigid, metre-based survey transform. Resolve geographic anchors in their
/// local UTM grid and account for the difference between grid and true north.
pub struct MapTransform {
    pub crs: &'static Crs,
    pub east: f64,
    pub north: f64,
    pub cos: f64,
    pub sin: f64,
}
impl MapTransform {
    pub fn new(code: u32, east: f64, north: f64, angle: f64) -> Option<Self> {
        let source = lookup(code)?;
        let (lat, lon) = source.inverse(east, north)?;
        let (crs, east, north) = if code == 4326 {
            let zone = super::zone_for_lon(lon) as u32;
            let c = lookup((if lat < 0. { 32700 } else { 32600 }) + zone)?;
            let (e, n) = c.forward(lat, lon)?;
            (c, e, n)
        } else {
            (source, east, north)
        };
        let (a, b) = crs.forward(lat, lon)?;
        let (c, d) = crs.forward(lat + 1e-5, lon)?;
        let (sin, cos) = (angle.to_radians() - (c - a).atan2(d - b)).sin_cos();
        Some(Self {
            crs,
            east,
            north,
            cos,
            sin,
        })
    }
    pub fn geographic(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.crs.inverse(
            self.east + self.cos * x - self.sin * y,
            self.north + self.sin * x + self.cos * y,
        )
    }
    pub fn local(&self, lat: f64, lon: f64) -> Option<(f64, f64)> {
        let (e, n) = self.crs.forward(lat, lon)?;
        let (x, y) = (e - self.east, n - self.north);
        Some((self.cos * x + self.sin * y, -self.sin * x + self.cos * y))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_and_project_common_survey_systems() {
        assert_eq!(search("EPSG:2154").next().unwrap().code, 2154);
        assert_eq!(search("UTM zone").count(), 120);
        for (code, lat, lon) in [
            (2154, 46.5, 3.),
            (3948, 48., 2.),
            (27700, 51.5, -0.12),
            (26943, 38., -121.),
            (32719, -15., -71.),
            (3857, 40., 40.),
        ] {
            let crs = lookup(code).unwrap();
            let (e, n) = crs.forward(lat, lon).unwrap();
            let (a, b) = crs.inverse(e, n).unwrap();
            assert!(
                (a - lat).abs() < 1e-7 && (b - lon).abs() < 1e-7,
                "{code} {a} {b}"
            );
            if code == 2154 {
                assert!((e - 700000.).abs() < 0.001 && (n - 6600000.).abs() < 0.001);
            }
        }
    }
}
