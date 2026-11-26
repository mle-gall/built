// SPDX-License-Identifier: GPL-3.0-or-later
//! Changes Blender sent, waiting in Built's Render drawer. Each item names a
//! key (`transform:<uid>`, `material:<name>`, `light:<id>`, `camera:<name>`),
//! the value Blender started from (`base`: what Built last sent it) and the
//! value it proposes. When Built's current value no longer equals the base,
//! both apps changed the same thing since the last sync: a conflict, shown
//! with "Keep Built" / "Take Blender" instead of Apply / Ignore.
use serde_json::{json, Map, Value};

/// Positions in metres, factors 0–1: a tenth of a millimetre is "the same".
const TOLERANCE: f64 = 1e-4;

/// Equal up to `TOLERANCE` for numbers; a missing key equals null.
pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
            (x - y).abs() <= TOLERANCE * 1f64.max(x.abs().max(y.abs()))
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b)),
        (Value::Object(x), Value::Object(y)) => x
            .keys()
            .chain(y.keys())
            .all(|k| same(x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null))),
        _ => a == b,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub base: Value,
    pub value: Value,
    /// Built's value when the item arrived or was last revalidated.
    pub built: Value,
    pub conflict: bool,
    /// Why Built cannot take it (an area light, a material Built lacks).
    pub unsupported: Option<String>,
}

impl Item {
    pub fn to_json(&self) -> Value {
        json!({"key": self.key, "kind": self.kind, "label": self.label, "base": self.base,
               "value": self.value, "built": self.built, "conflict": self.conflict,
               "unsupported": self.unsupported})
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Received {
    pub added: usize,
    /// Items whose value already equals Built's (nothing to do).
    pub same: Vec<String>,
}

#[derive(Default)]
pub struct Review {
    items: Vec<Item>,
}

fn kind_of(key: &str) -> &str {
    key.split_once(':').map_or("", |(k, _)| k)
}

impl Review {
    pub fn items(&self) -> &[Item] {
        &self.items
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn conflicts(&self) -> usize {
        self.items.iter().filter(|i| i.conflict).count()
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Take a batch from Blender. `current` maps keys to Built's values
    /// (null: Built has none, e.g. a new light).
    pub fn receive(&mut self, batch: &[Value], current: &Map<String, Value>) -> Received {
        let mut out = Received::default();
        for raw in batch {
            let Some(key) = raw.get("key").and_then(Value::as_str) else { continue };
            let built = current.get(key).cloned().unwrap_or(Value::Null);
            let value = raw.get("value").cloned().unwrap_or(Value::Null);
            let unsupported = raw.get("unsupported").and_then(Value::as_str).map(str::to_owned);
            if unsupported.is_none() && same(&value, &built) {
                self.items.retain(|i| i.key != key);
                out.same.push(key.to_owned());
                continue;
            }
            let base = raw.get("base").cloned().unwrap_or(Value::Null);
            let item = Item {
                key: key.to_owned(),
                kind: raw.get("kind").and_then(Value::as_str).unwrap_or(kind_of(key)).to_owned(),
                label: raw.get("label").and_then(Value::as_str).unwrap_or(key).to_owned(),
                conflict: !same(&base, &built),
                base,
                value,
                built,
                unsupported,
            };
            match self.items.iter_mut().find(|i| i.key == item.key) {
                Some(slot) => *slot = item,
                None => {
                    self.items.push(item);
                    out.added += 1;
                }
            }
        }
        out
    }

    /// Compare again with Built's values (before applying, after an edit in
    /// Built). An item Built now already matches drops out.
    pub fn revalidate(&mut self, current: &Map<String, Value>) -> Vec<String> {
        let mut gone = Vec::new();
        self.items.retain_mut(|i| {
            let Some(now) = current.get(&i.key) else { return true };
            if i.unsupported.is_none() && same(&i.value, now) {
                gone.push(i.key.clone());
                return false;
            }
            i.built = now.clone();
            i.conflict = !same(&i.base, now);
            true
        });
        gone
    }

    /// Remove and return the named items (all when `keys` is None).
    pub fn take(&mut self, keys: Option<&[String]>) -> Vec<Item> {
        let (taken, kept) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|i| keys.is_none_or(|k| k.contains(&i.key)));
        self.items = kept;
        taken
    }

    /// One row per item for the drawer: what changed, Built's and Blender's text.
    pub fn rows(&self) -> Vec<Value> {
        self.items
            .iter()
            .map(|i| {
                let (field, built, blender) = describe(i);
                let label = if field.is_empty() { i.label.clone() } else { format!("{} · {field}", i.label) };
                json!({"key": i.key, "kind": i.kind, "label": label, "built": built, "blender": blender,
                       "conflict": i.conflict, "unsupported": i.unsupported})
            })
            .collect()
    }
}

fn num(v: &Value, i: usize) -> f64 {
    v.get(i).and_then(Value::as_f64).unwrap_or(0.)
}
fn metres(x: f64) -> String {
    format!("{x:.2} m")
}
fn triple(v: &Value) -> String {
    format!("{:.2}, {:.2}, {:.2} m", num(v, 0), num(v, 1), num(v, 2))
}

/// Translation, per-axis scale and the rotation part of a column-major matrix.
fn decompose(m: &Value) -> Option<([f64; 3], [f64; 3], [[f64; 3]; 3])> {
    let a = m.as_array().filter(|a| a.len() == 16)?;
    let f: Vec<f64> = a.iter().map(|v| v.as_f64().unwrap_or(0.)).collect();
    let col = |c: usize| [f[4 * c], f[4 * c + 1], f[4 * c + 2]];
    let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let cols = [col(0), col(1), col(2)];
    let scale = cols.map(len);
    let rot = [0, 1, 2].map(|c| cols[c].map(|x| if scale[c] > 1e-12 { x / scale[c] } else { 0. }));
    Some(([f[12], f[13], f[14]], scale, rot))
}

fn angle_between(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> f64 {
    // trace(Aᵀ B) = 1 + 2 cos θ
    let t: f64 = (0..3).map(|c| (0..3).map(|r| a[c][r] * b[c][r]).sum::<f64>()).sum();
    ((t - 1.) / 2.).clamp(-1., 1.).acos().to_degrees()
}

/// What changed in one item, as (field, Built's text, Blender's text).
pub fn describe(i: &Item) -> (String, String, String) {
    if let Some(why) = &i.unsupported {
        return (String::new(), "—".into(), why.clone());
    }
    let dash = || "—".to_string();
    match i.kind.as_str() {
        "transform" => {
            let (Some((t0, s0, r0)), Some((t1, s1, r1))) = (
                decompose(i.built.get("matrix").unwrap_or(&Value::Null)),
                decompose(i.value.get("matrix").unwrap_or(&Value::Null)),
            ) else {
                return ("Transform".into(), dash(), dash());
            };
            let moved: Vec<usize> = (0..3).filter(|&k| (t0[k] - t1[k]).abs() > TOLERANCE).collect();
            let turned = angle_between(&r0, &r1);
            let scaled = (0..3).any(|k| (s0[k] - s1[k]).abs() > TOLERANCE * s0[k].abs().max(1.));
            let mut fields = Vec::new();
            let (mut b, mut x) = (String::new(), String::new());
            if moved.len() == 1 {
                let axis = ["X", "Y", "Z"][moved[0]];
                fields.push(format!("Position {axis}"));
                b = metres(t0[moved[0]]);
                x = metres(t1[moved[0]]);
            } else if !moved.is_empty() {
                fields.push("Position".into());
                b = triple(&json!(t0));
                x = triple(&json!(t1));
            }
            if turned > 0.01 {
                fields.push("Rotation".into());
                if b.is_empty() {
                    b = "—".into();
                    x = format!("turned {turned:.1}°");
                }
            }
            if scaled {
                fields.push("Scale".into());
                if b.is_empty() {
                    b = format!("{:.3} × {:.3} × {:.3}", s0[0], s0[1], s0[2]);
                    x = format!("{:.3} × {:.3} × {:.3}", s1[0], s1[1], s1[2]);
                }
            }
            (fields.join(", "), b, x)
        }
        "material" => {
            let mut fields = Vec::new();
            let (mut b, mut x) = (Vec::new(), Vec::new());
            for (k, label) in [("color", "Colour"), ("roughness", "Roughness"), ("metallic", "Metallic"), ("texture", "Image")] {
                let (v0, v1) = (i.built.get(k).unwrap_or(&Value::Null), i.value.get(k).unwrap_or(&Value::Null));
                if same(v0, v1) {
                    continue;
                }
                fields.push(label);
                let show = |v: &Value| match (k, v) {
                    ("color", Value::Array(_)) => format!("#{:02X}{:02X}{:02X}", (num(v, 0).clamp(0., 1.) * 255.).round() as u8,
                                                          (num(v, 1).clamp(0., 1.) * 255.).round() as u8, (num(v, 2).clamp(0., 1.) * 255.).round() as u8),
                    ("texture", Value::Object(t)) => t.get("name").and_then(Value::as_str).unwrap_or("image").to_owned(),
                    (_, Value::Number(n)) => format!("{:.2}", n.as_f64().unwrap_or(0.)),
                    _ => "—".into(),
                };
                b.push(show(v0));
                x.push(show(v1));
            }
            (fields.join(", "), b.join(", "), x.join(", "))
        }
        "light" => {
            let show = |v: &Value| {
                if v.is_null() {
                    return "—".to_string();
                }
                let kind = if v.get("kind").and_then(Value::as_str) == Some("spot") { "Spot" } else { "Point" };
                format!("{kind} · {:.0} W", v.get("power").and_then(Value::as_f64).unwrap_or(0.))
            };
            let field = if i.built.is_null() { "New light" } else { "Light" };
            (field.into(), show(&i.built), show(&i.value))
        }
        "camera" => {
            if i.built.is_null() {
                return ("New scene".into(), dash(), "Camera".into());
            }
            let eye = |v: &Value| v.get("eye").cloned().unwrap_or(Value::Null);
            if !same(&eye(&i.built), &eye(&i.value)) {
                ("Camera position".into(), triple(&eye(&i.built)), triple(&eye(&i.value)))
            } else {
                ("Camera".into(), "Built view".into(), "Blender view".into())
            }
        }
        _ => (String::new(), dash(), dash()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(x: f64) -> Value {
        json!({"matrix": [1,0,0,0, 0,1,0,0, 0,0,1,0, x,0,0.4,1]})
    }
    fn current(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn a_change_only_blender_made_is_not_a_conflict() {
        let mut r = Review::default();
        let got = r.receive(&[json!({"key": "transform:u1", "label": "Bench seat", "base": moved(1.25), "value": moved(1.10)})],
                            &current(&[("transform:u1", moved(1.25))]));
        assert_eq!(got.added, 1);
        assert_eq!(r.conflicts(), 0);
        let row = &r.rows()[0];
        assert_eq!(row["label"], "Bench seat · Position X");
        assert_eq!(row["built"], "1.25 m");
        assert_eq!(row["blender"], "1.10 m");
    }

    #[test]
    fn both_apps_changing_the_same_thing_is_a_conflict() {
        let mut r = Review::default();
        r.receive(&[json!({"key": "transform:u1", "label": "Bench seat", "base": moved(1.0), "value": moved(1.10)})],
                  &current(&[("transform:u1", moved(1.25))]));
        assert_eq!(r.conflicts(), 1);
        assert_eq!(r.rows()[0]["conflict"], true);
    }

    #[test]
    fn a_value_built_already_has_is_dropped_and_tiny_float_noise_is_equal() {
        let mut r = Review::default();
        let got = r.receive(&[json!({"key": "transform:u1", "base": moved(1.0), "value": moved(1.25 + 1e-7)})],
                            &current(&[("transform:u1", moved(1.25))]));
        assert_eq!(got.added, 0);
        assert_eq!(got.same, vec!["transform:u1".to_string()]);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn a_newer_batch_replaces_the_pending_item_for_the_same_key() {
        let mut r = Review::default();
        let now = current(&[("transform:u1", moved(1.0))]);
        r.receive(&[json!({"key": "transform:u1", "base": moved(1.0), "value": moved(2.0)})], &now);
        let got = r.receive(&[json!({"key": "transform:u1", "base": moved(1.0), "value": moved(3.0)})], &now);
        assert_eq!(got.added, 0);
        assert_eq!(r.len(), 1);
        assert!(same(&r.items()[0].value, &moved(3.0)));
    }

    #[test]
    fn revalidating_after_a_built_edit_turns_an_item_into_a_conflict() {
        let mut r = Review::default();
        r.receive(&[json!({"key": "material:Oak", "kind": "material", "label": "Oak",
                           "base": {"color": [0.5, 0.3, 0.1], "roughness": 0.8},
                           "value": {"color": [0.5, 0.3, 0.1], "roughness": 0.35}})],
                  &current(&[("material:Oak", json!({"color": [0.5, 0.3, 0.1], "roughness": 0.8}))]));
        assert_eq!(r.conflicts(), 0);
        assert_eq!(r.rows()[0]["label"], "Oak · Roughness");
        assert_eq!(r.rows()[0]["blender"], "0.35");
        let gone = r.revalidate(&current(&[("material:Oak", json!({"color": [0.9, 0.3, 0.1], "roughness": 0.8}))]));
        assert!(gone.is_empty());
        assert_eq!(r.conflicts(), 1);
        let gone = r.revalidate(&current(&[("material:Oak", json!({"color": [0.5, 0.3, 0.1], "roughness": 0.35}))]));
        assert_eq!(gone, vec!["material:Oak".to_string()]);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn new_lights_and_cameras_and_unsupported_items_read_plainly() {
        let mut r = Review::default();
        r.receive(&[
            json!({"key": "light:a1", "kind": "light", "label": "Studio key", "base": null,
                   "value": {"kind": "point", "power": 700.0, "pos": [0, 0, 3]}}),
            json!({"key": "camera:Front", "kind": "camera", "label": "Front", "base": null,
                   "value": {"eye": [0, -5, 1.6], "target": [0, 0, 1], "up": [0, 0, 1], "fov_deg": 40, "perspective": true}}),
            json!({"key": "light:a2", "kind": "light", "label": "Fill", "base": null, "value": {"kind": "area"},
                   "unsupported": "Area lights stay in Blender"}),
        ], &Map::new());
        let rows = r.rows();
        assert_eq!(rows[0]["label"], "Studio key · New light");
        assert_eq!(rows[0]["blender"], "Point · 700 W");
        assert_eq!(rows[0]["conflict"], false);
        assert_eq!(rows[1]["label"], "Front · New scene");
        assert_eq!(rows[2]["blender"], "Area lights stay in Blender");
        let taken = r.take(Some(&["light:a1".to_string()]));
        assert_eq!(taken.len(), 1);
        assert_eq!(r.len(), 2);
        assert_eq!(r.take(None).len(), 2);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn rotation_and_scale_are_named() {
        let i = Item {
            key: "transform:u".into(), kind: "transform".into(), label: "Seat".into(),
            base: Value::Null, value: json!({"matrix": [0,1,0,0, -1,0,0,0, 0,0,2,0, 0,0,0,1]}),
            built: json!({"matrix": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1]}), conflict: false, unsupported: None,
        };
        let (field, _, blender) = describe(&i);
        assert_eq!(field, "Rotation, Scale");
        assert_eq!(blender, "turned 90.0°");
    }
}
