// SPDX-License-Identifier: GPL-3.0-or-later
//! AxisMagnet callbacks on the live document.
use super::*;

fn magnet_on<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    true.into_bound_py_any(py)
}
fn magnet_off_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.base")?;
    let tool = a.get_item(0)?;
    (!(tool.is_instance(&module.getattr("PlaneLock")?)?)).into_bound_py_any(py)
}
fn magnetic_axis_deg<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.call_method0("magnet_on")?.is_truthy()? {
        tool.getattr("_MAGNET_DEG")?
    } else {
        none(py)
    })
}
fn screen_axis_px<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.call_method0("magnet_on")?.is_truthy()? {
        return Ok(none(py));
    }
    tool.getattr("_MAGNET_PX")
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("magnet_on", Mode::Method, magnet_on, &["self"], "bool", ""),
    ("magnet_off_plane", Mode::Method, magnet_off_plane, &["self"], "bool", "Whether an inference may take the point OFF the plane the tool\nis drawing on.\n\nA shape that LIVES in one plane (``PlaneLock``: circle, polygon,\nthe arcs, the rotated rectangle) says no: the snap engine then\nkeeps the magnet — world and screen detectors alike — on the axes\nthe plane HOLDS (``axis_in_plane``). Over a face, following the one\naxis the plane cannot hold (its own normal) left the rim off the\nface while the radius was measured in it, and the circle came out\n«a metade ou menos ainda» (Marco, #545). A Line, a Text and a\nDimension are free placements and may leave the plane."),
    ("magnetic_axis_deg", Mode::Property, magnetic_axis_deg, &["self"], "", ""),
    ("screen_axis_px", Mode::Property, screen_axis_px, &["self"], "", "The pixel-space detector's threshold (9 px), for every magnetised\ntool. A planar shape's answer stays confined to its plane by the\nsnap engine's ``axis_in_plane``, so here it only widens the capture\nfrom the 3° world cone to the screen's 9 px (#962)."),
];
