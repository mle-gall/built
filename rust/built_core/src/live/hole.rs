// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::exact::{
    bound::{self, EdgeBind},
    Curve, Frame,
};
use crate::store::{edit::Change, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hole {
    pub d: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stations: Vec<(String, [f64; 16])>,
    #[serde(default = "through")]
    pub depth: Value,
    #[serde(default = "segments")]
    pub segments: usize,
    pub countersink: Option<Sink>,
    pub frame: Option<[f64; 16]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sink {
    d: f64,
    angle: f64,
}
fn through() -> Value {
    Value::String("through".into())
}
fn segments() -> usize {
    24
}
impl Hole {
    pub fn valid(&self) -> bool {
        self.d > 0.
            && (3..=512).contains(&self.segments)
            && (self.depth == "through" || self.depth.as_f64().is_some_and(|d| d > 0.))
            && self
                .countersink
                .as_ref()
                .is_none_or(|s| s.d > self.d && s.angle > 0. && s.angle < 180.)
    }
    pub fn pose(&self, scene: &Obj<'_>, t: &LiveTable, row: &Row) -> PyResult<Option<DMat4>> {
        let mut m = self
            .frame
            .as_ref()
            .map(DMat4::from_cols_array)
            .unwrap_or(DMat4::IDENTITY);
        if let Some(a) = &row.anchor {
            if let Reference::Frame { junction, frame } = &a.reference {
                let Some(f) = junction::local_frame(t, &row.owner, junction, frame) else {
                    return Ok(None);
                };
                m = f * m;
            } else {
                let Some(p) = reference::origin(scene, t, &a.reference, row)? else {
                    return Ok(None);
                };
                m.w_axis = p.extend(1.);
            }
            m *= DMat4::from_translation(DVec3::Z * a.offset / 1000.);
        }
        Ok((m.is_finite() && m.determinant().abs() > 1e-12).then_some(m))
    }
    pub fn ring(&self, r: f64, z: f64, m: DMat4) -> Vec<[f64; 3]> {
        (0..self.segments)
            .map(|i| {
                let (sin, cos) =
                    (std::f64::consts::TAU * i as f64 / self.segments as f64).sin_cos();
                m.transform_point3(DVec3::new(r * cos, r * sin, z))
                    .to_array()
            })
            .collect()
    }
    pub fn tool(&self, st: &mut Store, mesh: &Store, m: DMat4) -> Result<(), &'static str> {
        let inv = m.inverse();
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for slot in mesh.live_slots(Kind::Vertex) {
            let z = inv
                .transform_point3(
                    mesh.state
                        .frame
                        .definition_point(*mesh.state.vpos.get(slot as usize))
                        .into(),
                )
                .z;
            lo = lo.min(z);
            hi = hi.max(z);
        }
        let r = self.d / 2000.;
        let top = if self.depth == "through" { hi } else { 0. };
        let bottom = self.depth.as_f64().map_or(lo - 0.001, |d| -d / 1000.);
        let mut sections = vec![(bottom, r)];
        if let Some(s) = &self.countersink {
            let outer = s.d / 2000.;
            let neck = top - (outer - r) / (s.angle.to_radians() / 2.).tan();
            if neck <= bottom {
                return Err("invalid_countersink");
            }
            sections.extend([(neck, r), (top, outer), (top + 0.001, outer)]);
        } else {
            sections.push((top + 0.001, r));
        }
        let rings: Vec<_> = sections
            .iter()
            .map(|&(z, r)| self.ring(r, z, DMat4::IDENTITY))
            .collect();
        let mut c = Change::default();
        let mut bottom = rings[0].clone();
        bottom.reverse();
        st.add_face(&[bottom], &mut c).map_err(|_| "invalid_hole")?;
        for pair in rings.windows(2) {
            for i in 0..self.segments {
                let k = (i + 1) % self.segments;
                st.add_face(
                    &[vec![pair[0][i], pair[0][k], pair[1][k], pair[1][i]]],
                    &mut c,
                )
                .map_err(|_| "invalid_hole")?;
            }
        }
        st.add_face(&[rings.last().unwrap().clone()], &mut c)
            .map_err(|_| "invalid_hole")?;
        Ok(())
    }
    /// Inspect faces in drill coordinates. Any edge crossing the bore's disk,
    /// oblique intersected face, or extra skin delegates to the solid kernel.
    pub fn drill(&self, st: &mut Store, m: DMat4, uid: &str) -> Result<bool, &'static str> {
        if self.depth != "through" || self.countersink.is_some() || !st.state.frame.is_model() {
            return Ok(false);
        }
        let linear = glam::DMat3::from_mat4(m);
        if !(linear.transpose() * linear).abs_diff_eq(glam::DMat3::IDENTITY, 1e-9)
            || linear.determinant() < 0.
        {
            return Ok(false);
        }
        let r = self.d / 2000.;
        let inverse = m.inverse();
        let mut hits = Vec::new();
        for f in st.live_slots(Kind::Face) {
            let mut inside = false;
            let rings = st.face_rings(f);
            let mut height = None;
            let mut flat = true;
            let mut winding = 0.;
            for (index, ring) in rings.iter().enumerate() {
                let p = |v: u32| inverse.transform_point3((*st.state.vpos.get(v as usize)).into());
                let mut a = p(*ring.last().ok_or("invalid_face")?);
                let mut area = 0.;
                for &v in ring {
                    let b = p(v);
                    let h = *height.get_or_insert(b.z);
                    flat &= (b.z - h).abs() < 1e-9;
                    let (u, w) = (a.truncate(), b.truncate());
                    let seg = w - u;
                    let t = if seg.length_squared() > 0. {
                        (-u.dot(seg) / seg.length_squared()).clamp(0., 1.)
                    } else {
                        0.
                    };
                    if (u + t * seg).length() <= r + 1e-9 {
                        return Ok(false);
                    }
                    if (u.y > 0.) != (w.y > 0.) && u.x - u.y * seg.x / seg.y > 0. {
                        inside = !inside;
                    }
                    area += u.perp_dot(w);
                    a = b;
                }
                if index == 0 {
                    winding = area;
                }
            }
            if inside {
                if !flat || st.state.exact.curved_face(f).is_some() {
                    return Ok(false);
                }
                hits.push((height.unwrap(), f, winding));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        // A flange may have a second, disjoint flange behind it. Select the
        // material interval nearest this ply's anchor.
        if hits.len() % 2 != 0
            || hits
                .chunks_exact(2)
                .any(|pair| pair[0].2 >= 0. || pair[1].2 <= 0.)
        {
            return Ok(false);
        }
        let distance = |pair: &[(f64, u32, f64)]| pair[0].0.max(-pair[1].0).max(0.);
        let Some(hits) = hits
            .chunks_exact(2)
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        else {
            return Ok(false);
        };
        let mut c = Change::default();
        let mut caps = Vec::new();
        let mut roles = Vec::new();
        let axis = m.z_axis.truncate().normalize();
        let mut exact = st
            .state
            .exact
            .curved
            .as_deref()
            .cloned()
            .unwrap_or_default();
        for (index, &(z, f, _)) in hits.iter().enumerate() {
            let mut ring = self.ring(r, z, m);
            if index == 1 {
                ring.reverse();
            }
            let mut slots = st.add_hole(f, &ring, &mut c).map_err(|_| "invalid_hole")?;
            if index == 1 {
                slots.reverse();
            }
            let curve = Curve::Circle {
                frame: Frame {
                    origin: m.transform_point3(DVec3::Z * z),
                    axis,
                    reference: m.x_axis.truncate().normalize(),
                },
                radius: r,
            };
            for i in 0..slots.len() {
                let a = slots[i];
                let b = slots[(i + 1) % slots.len()];
                roles.push((format!("hole/{uid}/{index}/vertex/{i}"), Kind::Vertex, a));
                let e = st.find_edge(a, b).ok_or("invalid_hole")?;
                let [p, q] = st
                    .state
                    .ends
                    .get(e as usize)
                    .map(|v| DVec3::from(*st.state.vpos.get(v as usize)));
                exact.edges.insert(
                    e,
                    EdgeBind {
                        interval: curve.interval(p, q, None).ok_or("invalid_hole")?,
                        curve: curve.clone(),
                    },
                );
                roles.push((format!("hole/{uid}/{index}/rim/{i}"), Kind::Edge, e));
            }
            caps.push((slots, curve));
        }
        let support = bound::extrusion(caps[0].1.clone(), axis.into()).ok_or("invalid_hole")?;
        for i in 0..self.segments {
            let k = (i + 1) % self.segments;
            let (a, b) = (&caps[0].0, &caps[1].0);
            let face = st
                .add_welded_face(&[vec![a[k], a[i], b[i], b[k]]], &mut c)
                .map_err(|_| "invalid_hole")?;
            exact.faces.insert(face, support.clone());
            roles.push((format!("hole/{uid}/bore/{i}"), Kind::Face, face));
            roles.push((
                format!("hole/{uid}/seam/{i}"),
                Kind::Edge,
                st.find_edge(a[i], b[i]).ok_or("invalid_hole")?,
            ));
        }
        st.state.exact.curved = Some(Arc::new(exact));
        geometry::assign_roles(st, roles)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn drill_plate_and_flange_preserves_rims_and_exact_bores() {
        let path = [[0., 0., 0.], [1., 0., 0.]];
        let stock = |library: &str| {
            let profile = if library == "basic/flat" {
                json!({"library":library,"params":{"width":190,"height":12}})
            } else {
                json!({"library":library})
            };
            let sweep: sweep::Sweep =
                serde_json::from_value(json!({"profile":profile,"path":{"points":path}})).unwrap();
            let mut st = Store::empty(7, Default::default());
            sweep::generate(&mut st, &sweep, &path, &[]).unwrap();
            st
        };
        let plate = stock("basic/flat");
        let flange = stock("en/he200a");
        let hole: Hole = serde_json::from_value(json!({"d":18,"segments":24})).unwrap();
        let mut samples = Vec::new();
        let mut expected = None;
        for _ in 0..7 {
            let mut plies = [plate.clone(), flange.clone()];
            let start = std::time::Instant::now();
            for st in &mut plies {
                for i in 0..16 {
                    let m =
                        DMat4::from_translation(DVec3::new(0.04 + 0.055 * i as f64, 0.05, 0.10));
                    assert!(hole.drill(st, m, &format!("bolt/{i}")).unwrap());
                }
            }
            samples.push(start.elapsed());
            let stamps = plies.each_ref().map(geometry::stamp);
            assert_eq!(expected.get_or_insert_with(|| stamps.clone()), &stamps);
            for (st, stock) in plies.iter().zip([&plate, &flange]) {
                let exact = st.state.exact.curved.as_ref().unwrap();
                let stock_counts = stock
                    .state
                    .exact
                    .curved
                    .as_ref()
                    .map_or((0, 0), |e| (e.faces.len(), e.edges.len()));
                assert_eq!(exact.faces.len() - stock_counts.0, 16 * hole.segments);
                assert_eq!(exact.edges.len() - stock_counts.1, 32 * hole.segments);

                assert_eq!(
                    st.live_slots(Kind::Face)
                        .into_iter()
                        .filter(|&f| st.face_rings(f).len() == 17)
                        .count(),
                    2
                );
                st.check().unwrap();
            }
        }
        samples.sort();
        eprintln!("32 holes across plate and flange: {:?}", samples[3]);
    }
}
