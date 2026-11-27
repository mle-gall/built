// SPDX-License-Identifier: GPL-3.0-or-later
//! Convert a sketch's curves in millimetres to oriented rings. The caller
//! retains the curves and their IDs. Faceting only happens for the extrusion.
use super::profile::{Node, Outline};
use glam::DVec2;
use serde_json::Value;
use std::f64::consts::TAU;

pub fn outline(sketch: &Value, segments: usize) -> Result<Outline, &'static str> {
    let mut chains = Vec::new();
    let mut rings = Vec::new();
    for curve in sketch["curves"].as_array().ok_or("profile_missing")? {
        let kind = curve["kind"]
            .as_str()
            .or(curve["type"].as_str())
            .ok_or("invalid_profile")?;
        let point = |key: &str| {
            serde_json::from_value::<DVec2>(curve[key].clone()).map_err(|_| "invalid_profile")
        };
        let id = &curve["id"];
        if !id.is_number() && !id.is_string() {
            return Err("invalid_profile");
        }
        if kind == "line" {
            let a = point("a")?;
            let b = point("b")?;
            chains.push((
                a,
                b,
                vec![Node {
                    p: a,
                    arc: None,
                    role: id.to_string(),
                }],
            ));
        } else if kind == "circle" || kind == "arc" {
            let centre = point("c")?;
            let r = curve["r"]
                .as_f64()
                .filter(|r| *r > 0.)
                .ok_or("invalid_profile")?;
            let (a, b) = if kind == "circle" {
                (0., TAU)
            } else {
                (
                    curve["a0"].as_f64().ok_or("invalid_profile")?,
                    curve["a1"].as_f64().ok_or("invalid_profile")?,
                )
            };
            if (b - a).abs() > TAU + 1e-9 || a == b {
                return Err("invalid_profile");
            }
            let n = curve["segments"]
                .as_u64()
                .map(|n| n as usize)
                .unwrap_or_else(|| ((b - a).abs() / TAU * segments as f64).ceil() as usize)
                .clamp(if kind == "circle" { 3 } else { 1 }, 512);
            let at = |angle| centre + r * DVec2::from_angle(angle);
            let nodes = (0..n)
                .map(|i| Node {
                    p: at(a + (b - a) * i as f64 / n as f64),
                    arc: Some(centre),
                    role: format!("{id}/{i}"),
                })
                .collect();
            if kind == "circle" {
                rings.push(nodes)
            } else {
                chains.push((at(a), at(b), nodes))
            }
        } else {
            return Err("invalid_profile");
        }
    }
    let close = |a: DVec2, b: DVec2| a.distance(b) < 1e-6;
    while let Some((first, mut end, mut ring)) = chains.pop() {
        while !close(first, end) {
            let Some(i) = chains
                .iter()
                .position(|(a, b, _)| close(*a, end) || close(*b, end))
            else {
                return Err("profile_open");
            };
            let (a, b, mut chain) = chains.swap_remove(i);
            if close(a, end) {
                end = b
            } else {
                let mut to = b;
                for node in chain.iter_mut().rev() {
                    std::mem::swap(&mut node.p, &mut to);
                }
                chain.reverse();
                end = a;
            }
            ring.append(&mut chain);
        }
        if ring.len() < 3 {
            return Err("invalid_profile");
        }
        rings.push(ring);
    }
    let area = |ring: &Vec<Node>| {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(a, b)| a.p.perp_dot(b.p))
            .sum::<f64>()
    };
    rings.sort_by(|a, b| area(b).abs().total_cmp(&area(a).abs()));
    if rings.is_empty() {
        return Err("invalid_profile");
    }
    let outer = &rings[0];
    let inside = |p: DVec2| {
        outer
            .iter()
            .zip(outer.iter().cycle().skip(1))
            .filter(|(a, b)| {
                let (a, b) = (a.p, b.p);
                (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x
            })
            .count()
            % 2
            == 1
    };
    if rings.iter().skip(1).any(|r| !inside(r[0].p)) {
        return Err("multiple_regions");
    }
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for ring in &mut rings {
        let ar = area(ring);
        if ar.abs() < 1e-10 {
            return Err("invalid_profile");
        }
        if ar < 0. {
            let mut to = ring[0].p;
            for n in ring.iter_mut().rev() {
                std::mem::swap(&mut n.p, &mut to)
            }
            ring.reverse();
        }
        for n in ring {
            lo = lo.min(n.p);
            hi = hi.max(n.p)
        }
    }
    for n in rings.iter_mut().flatten() {
        n.p -= lo;
        n.arc = n.arc.map(|c| c - lo);
    }
    let mut named = std::collections::BTreeMap::new();
    let origin = sketch
        .get("origin")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value::<DVec2>(v.clone()))
        .transpose()
        .map_err(|_| "invalid_profile")?
        .unwrap_or_default();
    named.insert("origin".to_owned(), origin - lo);
    for point in sketch["points"].as_array().into_iter().flatten() {
        let name = point["name"].as_str().ok_or("invalid_profile")?;
        let at =
            serde_json::from_value::<DVec2>(point["at"].clone()).map_err(|_| "invalid_profile")?;
        named.insert(name.to_owned(), at - lo);
    }
    Ok(Outline {
        named,
        family: "sketch".into(),
        rings,
        width: hi.x - lo.x,
        height: hi.y - lo.y,
        web: 0.,
        flange: 0.,
        radius: 0.,
    })
}
