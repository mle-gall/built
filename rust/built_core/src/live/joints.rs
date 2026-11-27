// SPDX-License-Identifier: GPL-3.0-or-later
use crate::store::sweep::Station;
use glam::DVec3;

/// A point and normal in the generated definition's frame. Both end planes
/// face in the direction of travel, as in the IFC extrusion writer.
pub type Plane = [[f64; 3]; 2];

/// Keep the incoming member square across the complete width of the next
/// member. The outgoing end lands on the incoming section's outside tangent
/// plane. The two component shells keep their individual end faces for filing.
pub fn butt_stations(
    mitres: &[Station],
    axes: &[[f64; 3]],
    path: &[[f64; 3]],
    closed: bool,
) -> Result<(Vec<Station>, Vec<[Plane; 2]>), &'static str> {
    let mut stations = Vec::with_capacity(2 * axes.len());
    let mut trims = Vec::with_capacity(axes.len());
    for (i, axis) in axes.iter().enumerate() {
        let next = (i + 1) % path.len();
        stations.extend([mitres[i].clone(), mitres[next].clone()]);
        trims.push([[path[i], *axis], [path[next], *axis]]);
    }
    let shift = |p: [f64; 3], anchor: DVec3, along: DVec3, normal: DVec3, distance: f64| {
        let v = DVec3::from(p);
        (v + along * ((distance - (v - anchor).dot(normal)) / along.dot(normal))).to_array()
    };
    for joint in (if closed { 0 } else { 1 })..axes.len() {
        let previous = (joint + axes.len() - 1) % axes.len();
        let anchor = DVec3::from(path[joint]);
        let incoming = DVec3::from(axes[previous]);
        let outgoing = DVec3::from(axes[joint]);
        let Some(side) = (outgoing - incoming * incoming.dot(outgoing)).try_normalize() else {
            continue;
        };
        let outer = &mitres[joint][0];
        let distance = outer
            .iter()
            .map(|p| (DVec3::from(*p) - anchor).dot(side))
            .fold(f64::NEG_INFINITY, f64::max);
        let start = &mut stations[2 * joint];
        for p in start.iter_mut().flatten() {
            *p = shift(*p, anchor, outgoing, side, distance);
        }
        trims[joint][0] = [(anchor + side * distance).to_array(), side.to_array()];
        let reach = start[0]
            .iter()
            .map(|p| (DVec3::from(*p) - anchor).dot(incoming))
            .fold(f64::NEG_INFINITY, f64::max);
        for p in stations[2 * previous + 1].iter_mut().flatten() {
            *p = shift(*p, anchor, incoming, incoming, reach);
        }
        trims[previous][1] = [(anchor + incoming * reach).to_array(), incoming.to_array()];
    }
    for (pair, axis) in stations.chunks_exact(2).zip(axes) {
        if pair[0][0]
            .iter()
            .zip(&pair[1][0])
            .any(|(a, b)| (DVec3::from(*b) - DVec3::from(*a)).dot((*axis).into()) <= 1e-9)
        {
            return Err("invalid_path");
        }
    }
    Ok((stations, trims))
}

/// Project both cap outlines onto their span axis. The middle has a constant
/// section, while only the two end intervals contain an oblique trim. Distances
/// are signed metres from the path station (a butt can extend past that station).
/// Empty intervals describe square ends and require no separate solid.
pub fn span_zones(
    stations: &[Station],
    axes: &[[f64; 3]],
    path: &[[f64; 3]],
    butt: bool,
) -> Vec<[[f64; 2]; 3]> {
    axes.iter()
        .enumerate()
        .map(|(i, dir)| {
            let from = if butt { 2 * i } else { i };
            let to = (from + 1) % stations.len();
            let origin = DVec3::from(path[i]);
            let dir = DVec3::from(*dir);
            let extent = |s: usize| {
                let mut bounds = [f64::INFINITY, f64::NEG_INFINITY];
                for p in &stations[s][0] {
                    let t = (DVec3::from(*p) - origin).dot(dir);
                    bounds = [bounds[0].min(t), bounds[1].max(t)];
                }
                if bounds[1] - bounds[0] < 1e-12 {
                    bounds[1] = bounds[0];
                }
                bounds
            };
            let start = extent(from);
            let end = extent(to);
            // On a short span the two end cuts can meet. Their common boundary
            // leaves an empty middle, so downstream cuts use the entire span.
            let split = (0.5 * (start[1] + end[0])).clamp(start[0], end[1].max(start[0]));
            let a = start[1].min(split);
            let b = end[0].max(split);
            [[start[0], a], [a, b], [b, end[1]]]
        })
        .collect()
}
