// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::exact::{
    bound::{self, EdgeBind},
    Curve, Frame,
};
use crate::store::{edit::Change, Store};
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Table {
    bolts: Vec<String>,
    sizes: Vec<Size>,
    lengths: Vec<f64>,
}
#[derive(Deserialize)]
struct Size {
    d: f64,
    flats: f64,
    head: f64,
    nut: f64,
    washer_od: f64,
    washer_id: f64,
    washer_t: f64,
    pitch: f64,
}
static METRIC: LazyLock<Table> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../../resources/fasteners/metric.builtfastener"
    ))
    .expect("metric fasteners")
});

/// Loft a closed shell between coaxial polygons (hex flats or round sections).
/// Independent shells preserve the head, shank, washers and nut at contact.
fn prism(
    st: &mut Store,
    sides: usize,
    diameter: f64,
    bore: f64,
    z: f64,
    height: f64,
) -> Result<(), &'static str> {
    let mut c = Change::default();
    let mut exact = st
        .state
        .exact
        .curved
        .take()
        .map(Arc::unwrap_or_clone)
        .unwrap_or_default();
    let mut caps = [Vec::new(), Vec::new()];
    for (inner, n, d) in [(false, sides, diameter), (true, 24, bore)] {
        if d == 0. {
            continue;
        }
        let mut rings = [Vec::new(), Vec::new()];
        for (end, ring) in rings.iter_mut().enumerate() {
            for i in 0..n {
                let (s, cosa) = (i as f64 * std::f64::consts::TAU / n as f64).sin_cos();
                ring.push(
                    st.shell_vertex(
                        [
                            d * cosa / 2000.,
                            d * s / 2000.,
                            (z + end as f64 * height) / 1000.,
                        ],
                        &mut c,
                    )
                    .map_err(|_| "invalid_fastener")?,
                );
            }
            if inner {
                ring.reverse();
            }
        }
        for i in 0..n {
            let k = (i + 1) % n;
            let face = st
                .add_welded_face(
                    &[vec![rings[0][i], rings[0][k], rings[1][k], rings[1][i]]],
                    &mut c,
                )
                .map_err(|_| "invalid_fastener")?;
            if n != 6 {
                for (end, ring) in rings.iter().enumerate() {
                    let curve = Curve::Circle {
                        frame: Frame {
                            origin: DVec3::Z * ((z + end as f64 * height) / 1000.),
                            axis: DVec3::Z,
                            reference: DVec3::X,
                        },
                        radius: d / 2000.,
                    };
                    if end == 0 {
                        exact.faces.insert(
                            face,
                            bound::extrusion(curve.clone(), DVec3::Z.into())
                                .ok_or("invalid_fastener")?,
                        );
                    }
                    let edge = st.find_edge(ring[i], ring[k]).ok_or("invalid_fastener")?;
                    let [a, b] = st
                        .state
                        .ends
                        .get(edge as usize)
                        .map(|v| DVec3::from(*st.state.vpos.get(v as usize)));
                    exact.edges.insert(
                        edge,
                        EdgeBind {
                            interval: curve.interval(a, b, None).ok_or("invalid_fastener")?,
                            curve,
                        },
                    );
                }
            }
        }
        rings[0].reverse();
        for (cap, ring) in caps.iter_mut().zip(rings) {
            cap.push(ring);
        }
    }
    for cap in caps {
        st.add_welded_face(&cap, &mut c)
            .map_err(|_| "invalid_fastener")?;
    }
    if !exact.edges.is_empty() {
        st.state.exact.curved = Some(Arc::new(exact));
    }
    Ok(())
}

pub(super) fn update(
    scene: &Obj<'_>,
    g: &Obj<'_>,
    spec: &Value,
    params: &expr::Values,
    dims: &expr::Values,
) -> PyResult<()> {
    let number = |key: &str, default: f64| match spec.get(key) {
        Some(v) => expr::field(v, params, dims).map_err(err),
        None => Ok(params.get(key).copied().unwrap_or(default)),
    };
    let d = number("d", 16.)?;
    let standard = spec["standard"].as_str().unwrap_or("ISO 4017");
    let size = METRIC
        .sizes
        .iter()
        .find(|s| s.d == d)
        .filter(|_| METRIC.bolts.iter().any(|s| s == standard))
        .ok_or_else(|| err("Unknown bolt size or standard"))?;
    let grip = number(
        "grip",
        dims.iter()
            .filter(|(key, _)| key.starts_with("ply_") && key.ends_with(".thickness"))
            .map(|(_, v)| v)
            .sum(),
    )?;
    let minimum = grip + 2. * size.washer_t + size.nut + 2. * size.pitch;
    let length = number(
        "length",
        METRIC
            .lengths
            .iter()
            .find(|l| **l >= minimum)
            .copied()
            .unwrap_or(0.),
    )?;
    if grip <= 0. || length < minimum || !METRIC.lengths.contains(&length) {
        return Err(err("No standard bolt length for this grip"));
    }
    let key = eid::derived_uid(&format!("built:fastener:v1:{standard}:{d}:{length}:{grip}"));
    let py = scene.py();
    let definition = if let Some(g) = cut::source(scene, &key)? {
        g
    } else {
        let def = py.import("built_core.group")?.getattr("Group")?.call0()?;
        let mesh = def.getattr("mesh")?;
        let handle = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
        let mut b = handle.try_borrow_mut()?;
        let st = &mut b.inner;
        let hex = size.flats / (std::f64::consts::PI / 6.).cos();
        let w = size.washer_t;
        prism(st, 6, hex, 0., w, size.head).map_err(err)?;
        prism(st, 24, d, 0., w - length, length).map_err(err)?;
        for z in [0., -grip - w] {
            prism(st, 24, size.washer_od, size.washer_id, z, w).map_err(err)?;
        }
        prism(st, 6, hex, d, -grip - w - size.nut, size.nut).map_err(err)?;
        let roles = Kind::ALL
            .into_iter()
            .flat_map(|kind| {
                st.live_slots(kind)
                    .into_iter()
                    .enumerate()
                    .map(move |(i, slot)| (format!("fastener/{kind:?}/{i}"), kind, slot))
            })
            .collect();
        geometry::assign_roles(st, roles).map_err(err)?;
        drop(b);
        eid::set_definition_uid(&mesh, &key)?;
        def.setattr("uid", &key)?;
        def.setattr("name", format!("{standard} M{d} × {length}"))?;
        def.setattr("component", true)?;
        scene
            .getattr("kept_components")?
            .call_method1("append", (&def,))?;
        def
    };
    let mesh = definition.getattr("mesh")?;
    if !g.getattr("mesh")?.is(&mesh) {
        g.setattr("mesh", mesh)?;
    }
    g.setattr("name", definition.getattr("name")?)?;
    g.setattr("ifc",to_py(py,&json!({"class":"IfcMechanicalFastener","predefined_type":"BOLT","standard":standard,"grade":spec.get("grade").cloned().unwrap_or(json!("8.8")),"d":d,"length":length,"grip":grip}))?)
}

/// Supply one bore for every bound ply of a bolt detail. An explicit hole on
/// a ply carries its custom depth, countersink, or placement.
pub(super) fn contents(j: &Row, p: &presets::Preset) -> PyResult<Vec<Value>> {
    let mut items = p
        .values
        .get("contents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(part) = items
        .iter()
        .find_map(|item| item.get("part").filter(|v| v.get("fastener").is_some()))
        .cloned()
    else {
        return Ok(items);
    };
    let spec = &part["fastener"];
    let parameter = |key: &str, default: f64| {
        spec.get(key).cloned().unwrap_or_else(|| {
            if p.values
                .get("params")
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(|v| v["key"] == key))
            {
                json!(key)
            } else {
                json!(default)
            }
        })
    };
    let expression = |v: Value| {
        if let Some(s) = v.as_str() {
            s.to_owned()
        } else {
            v.to_string()
        }
    };
    let diameter = format!(
        "({}) + ({})",
        expression(parameter("d", 16.)),
        expression(parameter("clearance", 0.))
    );
    let frame = part["at"]["frame"].as_str().unwrap_or("@meet");
    let offset = part["at"]
        .get("offset")
        .cloned()
        .unwrap_or_else(|| json!([0., 0., 0.]));
    for ply in j
        .joint
        .roles
        .as_object()
        .into_iter()
        .flat_map(|r| r.keys())
        .filter(|k| k.starts_with("ply_"))
    {
        if items
            .iter()
            .any(|v| v["on"] == *ply && v["row"]["cutter"].get("hole").is_some())
        {
            continue;
        }
        let mut matrix = json!(DMat4::IDENTITY.to_cols_array());
        let values = offset
            .as_array()
            .filter(|v| v.len() == 3)
            .ok_or_else(|| err("Invalid bolt offset"))?;
        for (i, v) in values.iter().enumerate() {
            matrix[12 + i] = if let Some(n) = v.as_f64() {
                json!(n / 1000.)
            } else {
                json!(format!("({}) / 1000", expression(v.clone())))
            };
        }
        items.push(json!({"key":format!("hole_{ply}"),"on":ply,"row":{"kind":"cut","operation":"subtract","anchor":{"ref":frame},"cutter":{"hole":{"d":diameter,"frame":matrix}}}}));
    }
    Ok(items)
}
