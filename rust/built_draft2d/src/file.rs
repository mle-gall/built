// SPDX-License-Identifier: GPL-3.0-or-later
//! `.builtprofile` files: Library profiles and sections. A drawn one holds
//! the same sketch a document's `sketch` row holds, so a profile saved from
//! the Sketch workspace and one opened from the Library are one kind of
//! thing. Catalogue families (general-base §2.7) share the envelope with a
//! `family` table instead of a `sketch`; this module reads both envelopes
//! and leaves the family table to its generator.
use crate::sketch::{Sketch, Use};
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "built.profile";
pub const EXTENSION: &str = "builtprofile";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileFile {
    pub format: String,
    pub v: u32,
    pub name: String,
    #[serde(rename = "use")]
    pub usage: Use,
    /// Free text: "Z · cold-formed", "Steel EN 10365".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub family: String,
    /// Where the data came from (a standard, a supplier drawing).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provenance: String,
    /// A drawn profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch: Option<Sketch>,
    /// A catalogue family's table (lane 1's generator reads it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<serde_json::Value>,
}

impl ProfileFile {
    pub fn drawn(name: &str, mut sketch: Sketch) -> Self {
        sketch.name = name.to_owned();
        // A Library profile has no plane: that is where a user puts it.
        sketch.plane = serde_json::Value::Null;
        Self {
            format: FORMAT.into(),
            v: 1,
            name: name.to_owned(),
            usage: sketch.usage,
            family: String::new(),
            provenance: String::new(),
            sketch: Some(sketch),
            table: None,
        }
    }
    pub fn to_string(&self) -> String {
        serde_json::to_string_pretty(self).expect("profile serialises")
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut f: ProfileFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if f.format != FORMAT {
            return Err(format!("not a {FORMAT} file"));
        }
        if f.v != 1 {
            return Err(format!("{FORMAT} version {} is newer than this reader", f.v));
        }
        if let Some(s) = f.sketch.as_mut() {
            s.validate()?;
            s.usage = f.usage;
        }
        if f.sketch.is_none() && f.table.is_none() {
            return Err("a profile needs a sketch or a family table".into());
        }
        Ok(f)
    }
}

/// A file name for a profile name: kept readable, never a path.
pub fn file_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '×' | '(' | ')') { c } else { '-' })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_owned();
    if out.is_empty() {
        out = "Profile".into();
    }
    format!("{out}.{EXTENSION}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Shape;
    use glam::DVec2;
    #[test]
    fn a_drawn_profile_round_trips_without_its_plane() {
        let mut s = Sketch::new(Use::Profile);
        s.add(Shape::Circle { c: DVec2::ZERO, r: 10. });
        s.plane = serde_json::json!({"ref": {"placement": "0123456789abcdef"}, "frame": "face"});
        let f = ProfileFile::drawn("Rod 20", s);
        let back = ProfileFile::parse(&f.to_string()).unwrap();
        assert_eq!(back, f);
        assert!(back.sketch.unwrap().plane.is_null());
        assert!(ProfileFile::parse(r#"{"format":"built.profile","v":2,"name":"x","use":"profile"}"#).is_err());
        assert_eq!(file_name("Zed 200 × 65 / 2.0"), "Zed 200 × 65 - 2.0.builtprofile");
    }
}
