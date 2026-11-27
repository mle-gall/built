//! The Sketch workspace (general-base §5.3, pass-5 Sketch A with the
//! owner's layout): a flat, true-size drawing space entered like group
//! editing. Tools on the left in the rail's place, the sketch's settings
//! on the right in the dock's place, the canvas between them with the live
//! 3D inset in its bottom-left corner, a breadcrumb with Cancel and Done in
//! the top bar.
//!
//! The working sketch and its local history live here (built_draft2d);
//! the document sees one step on Done: `sketch:done␟{json}` runs
//! `sketch_commit` (sketch_workspace.rs). Cancel sends `sketch:cancel` and
//! keeps the discarded copy so the toast's Undo can bring it back.
use super::*;
use built_draft2d::{
    cleanup::{self, Issue},
    edit,
    geom::{arc3, polar, Shape},
    import::{Drawing, Imported},
    regions::{profile, Problem, Profile},
    sketch::{At, Axis, DimKind, Extrude, NamedPoint, PointRef, Sketch, Use, ATTACH},
    snap::{self, Kind as SnapKind, Snap},
};
use glam::DVec2;

const SEP_S: &str = "\u{1f}";

/// The tool column: (id, label, shortcut, icon), in groups.
pub(super) const TOOLS: &[&[(&str, &str, &str, &str)]] = &[
    &[("select", "Select", "Space", "select")],
    &[
        ("line", "Line", "L", "line"),
        ("arc", "Arc", "A", "arc"),
        ("circle", "Circle", "C", "circle"),
        ("rectangle", "Rectangle", "R", "rectangle"),
    ],
    &[
        ("offset", "Offset", "F", "offset"),
        ("fillet", "Fillet", "G", "fillet"),
        ("trim", "Trim", "T", "sketch_trim"),
        ("extend", "Extend", "X", "sketch_extend"),
    ],
    &[("dimension", "Dimension", "D", "dimension")],
    &[("origin", "Origin", "O", "sketch_origin"), ("attach", "Attachment points", "P", "sketch_attach")],
    &[
        ("scale_edge", "Scale by edge", "", "sketch_scale_edge"),
        ("merge_arcs", "Merge arcs", "", "sketch_merge_arcs"),
        ("drop_fillets", "Drop small fillets", "", "sketch_drop_fillets"),
        ("close_gaps", "Close gaps", "", "sketch_close_gaps"),
    ],
];

fn tool_label(id: &str) -> &'static str {
    TOOLS.iter().flat_map(|g| g.iter()).find(|t| t.0 == id).map_or("Select", |t| t.1)
}

/// What the status bar says while a tool waits.
fn tool_hint(id: &str, step: usize) -> &'static str {
    match (id, step) {
        ("select", _) => "Click a curve to select it · Shift adds · Delete removes · double-click a dimension to type it",
        ("line", 0) => "Click the start · type a length or dx;dy after the first point",
        ("line", _) => "Click the next point · type a length · double-click or Esc ends the chain",
        ("arc", 0) => "Click the arc's start",
        ("arc", 1) => "Click its end",
        ("arc", _) => "Click how far it bulges · type the bulge",
        ("circle", 0) => "Click the centre",
        ("circle", _) => "Click the radius · type it",
        ("rectangle", 0) => "Click a corner",
        ("rectangle", _) => "Click the opposite corner · type w;h · Tab next value",
        ("offset", 0) => "Click a curve or a chain to offset",
        ("offset", _) => "Move to the side and click · type the distance",
        ("fillet", 0) => "Type the radius · click the first line",
        ("fillet", _) => "Click the second line",
        ("trim", _) => "Click the piece to cut away",
        ("extend", _) => "Click near the end to extend",
        ("dimension", 0) => "Click a line or arc · or an end, then another end · Shift-click two lines for an angle",
        ("dimension", _) => "Click the second end or line",
        ("origin", _) => "Click the point a path runs through",
        ("attach", _) => "Click a point to make it the default · Alt-click to add a point · Delete removes an extra",
        ("scale_edge", 0) => "Click an edge, type its real length · every other length follows",
        ("scale_edge", _) => "Type the edge's real length · Enter",
        ("merge_arcs", _) => "Runs of short segments become true arcs, within the tolerance",
        ("drop_fillets", _) => "Fillets under the limit become sharp corners",
        ("close_gaps", _) => "Ends closer than the tolerance meet",
        _ => "",
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum View {
    Isolated,
    #[default]
    Ghosted,
    Sliced,
}

/// The import being cleaned: the drawing as read, the layers kept, the
/// tolerance, and the list it gives.
pub(super) struct Cleaning {
    pub file: String,
    pub drawing: Drawing,
    pub keep: Vec<String>,
    pub tol: f64,
    pub tol_buf: String,
    pub imported: Imported,
    pub layers_open: bool,
    /// The walkable list's position.
    pub at: usize,
}

#[derive(Default)]
pub(super) struct Inset {
    /// World segments around the plane.
    pub segments: Vec<[f64; 6]>,
    pub origin: glam::DVec3,
    pub x: glam::DVec3,
    pub y: glam::DVec3,
    pub n: glam::DVec3,
    /// From where the 3D view looks (eye − target, world).
    pub look: glam::DVec3,
    pub label: String,
}

#[derive(Default)]
pub(super) struct SketchWs {
    pub shown: bool,
    pub doc: Sketch,
    /// As entered: Cancel restores it.
    base: Sketch,
    undo: Vec<Sketch>,
    redo: Vec<Sketch>,
    /// The row it edits; None for a new sketch.
    pub uid: Option<String>,
    crumbs: Vec<String>,
    plane_label: String,
    users: String,
    pub tool: String,
    groups: Vec<Vec<Tool>>,
    /// Points the active tool has collected.
    pts: Vec<DVec2>,
    /// A picked curve (offset, fillet first line, dimension, scale by edge).
    picked: Option<u32>,
    picked_end: Option<PointRef>,
    typed: String,
    typing: bool,
    fillet_r: f64,
    hover: Option<Snap>,
    hover_curve: Option<u32>,
    selection: Vec<u32>,
    /// Screen position of the sketch origin and pixels per millimetre.
    org: Pos2,
    zoom: f32,
    fit: bool,
    canvas: Option<Rect>,
    pub view: View,
    ghost: Vec<Shape>,
    behind: Vec<Shape>,
    pub inset: Inset,
    peek: bool,
    pub cleaning: Option<Cleaning>,
    /// The dimension whose value is being typed.
    editing_dim: Option<u32>,
    /// Esc pressed with nothing to drop: ask Done / Discard / Stay.
    leave_prompt: bool,
    toast: Option<(String, f64, Option<String>)>,
    refusal: Option<String>,
    keys: Vec<(String, u8)>,
    pub texts: Vec<String>,
    /// What Cancel threw away, for the toast's Undo.
    discarded: Option<(Sketch, Option<String>)>,
    problem_at: usize,
    profile: Option<Profile>,
    /// What to frame when the sketch is still empty (the picked face).
    fit_hint: Option<(DVec2, DVec2)>,
    name_buf: Option<String>,
    depth_buf: Option<String>,
    density: f64,
    now: f64,
}


impl SketchWs {
    fn changed(&mut self) {
        self.profile = Some(profile(&self.doc));
    }
    fn profile(&mut self) -> &Profile {
        if self.profile.is_none() {
            self.changed();
        }
        self.profile.as_ref().unwrap()
    }
    /// Remember the state before an edit (the local history).
    fn step(&mut self) {
        self.undo.push(self.doc.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn local_undo(&mut self, redo: bool) {
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        if let Some(prev) = from.pop() {
            to.push(std::mem::replace(&mut self.doc, prev));
            self.selection.retain(|id| self.doc.curve(*id).is_some());
            self.pts.clear();
            self.picked = None;
            self.changed();
            self.refresh_cleaning();
        }
    }
    fn s2w(&self, p: Pos2) -> DVec2 {
        DVec2::new(((p.x - self.org.x) / self.zoom) as f64, ((self.org.y - p.y) / self.zoom) as f64)
    }
    fn w2s(&self, p: DVec2) -> Pos2 {
        pos2(self.org.x + p.x as f32 * self.zoom, self.org.y - p.y as f32 * self.zoom)
    }
    fn curves(&self) -> Vec<(u32, Shape)> {
        self.doc.curves.iter().map(|c| (c.id, c.shape)).collect()
    }
    fn context(&self) -> &[Shape] {
        if self.view == View::Isolated {
            &[]
        } else {
            &self.ghost
        }
    }
    fn nearest_curve(&self, p: DVec2, radius: f64) -> Option<u32> {
        self.doc
            .curves
            .iter()
            .map(|c| (c.id, c.shape.distance(p)))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0)
    }
    pub(super) fn say(&mut self, text: String, undo: Option<&str>) {
        self.toast = Some((text, self.now, undo.map(str::to_owned)));
    }
    fn refresh_cleaning(&mut self) {
        if let Some(c) = self.cleaning.as_mut() {
            let declared = c.imported.unit.clone().or(if c.drawing.to_mm.is_some() { Some("mm".into()) } else { None });
            c.imported.issues = cleanup::issues(&self.doc, c.tol, declared.as_deref());
            c.at = c.at.min(c.imported.issues.len().saturating_sub(1));
        }
    }
    fn parse_typed(&self) -> Vec<f64> {
        self.typed
            .split([';', ',', 'x', '×'])
            .map(|s| s.trim().trim_end_matches("mm").trim())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f64>().ok())
            .collect()
    }

    /// Start: the record, the plane's context, the inset, maybe an import.
    pub(super) fn enter(&mut self, spec: &serde_json::Value) -> Result<(), String> {
        let mut doc = match spec.get("sketch") {
            Some(v) if !v.is_null() => Sketch::from_json(v)?,
            _ => Sketch::new(if spec["use"] == "section" { Use::Section } else { Use::Profile }),
        };
        if doc.next_id == 0 {
            doc.next_id = 1;
        }
        *self = SketchWs {
            shown: true,
            base: doc.clone(),
            doc,
            uid: spec["uid"].as_str().map(str::to_owned),
            crumbs: spec["crumbs"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            plane_label: spec["plane_label"].as_str().unwrap_or("").to_owned(),
            users: spec["users"].as_str().unwrap_or("").to_owned(),
            tool: "select".into(),
            groups: std::mem::take(&mut self.groups),
            fillet_r: 5.0,
            zoom: 1.0,
            fit: true,
            view: View::Ghosted,
            density: spec["density"].as_f64().unwrap_or(7850.0),
            ..Default::default()
        };
        let seg = |v: &serde_json::Value| -> Vec<Shape> {
            v.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| {
                            let s: Vec<f64> = s.as_array()?.iter().filter_map(|x| x.as_f64()).collect();
                            (s.len() == 4).then(|| Shape::Line { a: DVec2::new(s[0], s[1]), b: DVec2::new(s[2], s[3]) })
                        })
                        .filter(|l| l.length() > 1e-6)
                        .collect()
                })
                .unwrap_or_default()
        };
        let ctx = &spec["context"];
        self.ghost = seg(&ctx["ghost"]);
        self.behind = seg(&ctx["behind"]);
        let v3 = |v: &serde_json::Value| {
            let a: Vec<f64> = v.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
            (a.len() == 3).then(|| glam::DVec3::new(a[0], a[1], a[2]))
        };
        let plane = &spec["frame"];
        let face: Vec<DVec2> = plane["loops"].as_array().into_iter().flatten().flat_map(|r| r.as_array().into_iter().flatten())
            .filter_map(|p| Some(DVec2::new(p[0].as_f64()?, p[1].as_f64()?))).collect();
        if !face.is_empty() {
            let (lo, hi) = face.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
            let pad = DVec2::splat((hi - lo).max_element().max(50.0) * 1.5);
            self.fit_hint = Some((lo - pad, hi + pad));
        }
        if let (Some(o), Some(x), Some(y)) = (v3(&plane["origin"]), v3(&plane["x"]), v3(&plane["y"])) {
            self.inset = Inset {
                segments: ctx["inset"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|s| {
                        let s: Vec<f64> = s.as_array()?.iter().filter_map(|x| x.as_f64()).collect();
                        (s.len() == 6).then(|| [s[0], s[1], s[2], s[3], s[4], s[5]])
                    }).collect())
                    .unwrap_or_default(),
                origin: o,
                x,
                y,
                n: x.cross(y).normalize_or_zero(),
                look: v3(&spec["look"]).unwrap_or(glam::DVec3::new(1.0, -1.2, 0.9)),
                label: spec["inset_label"].as_str().unwrap_or("").to_owned(),
            };
        }
        if let Some(path) = spec["import"].as_str() {
            self.start_import(path, spec["import_name"].as_str().unwrap_or(path))?;
        }
        self.changed();
        Ok(())
    }

    fn start_import(&mut self, path: &str, shown_name: &str) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_owned();
        let drawing = built_draft2d::import::read(&ext, &bytes)?;
        let tol = 0.5;
        let imported = built_draft2d::import::sketch(&drawing, None, self.doc.usage, tol);
        if imported.sketch.curves.is_empty() {
            return Err("the drawing has nothing a sketch can hold".into());
        }
        let keep = imported.layers.iter().filter(|l| l.2).map(|l| l.0.clone()).collect();
        let name = std::path::Path::new(shown_name).file_stem().and_then(|s| s.to_str()).unwrap_or("Profile").to_owned();
        let mut s = imported.sketch.clone();
        s.name = if self.doc.name.is_empty() { name } else { self.doc.name.clone() };
        s.plane = self.doc.plane.clone();
        self.doc = s;
        self.base = self.doc.clone();
        self.cleaning = Some(Cleaning {
            file: std::path::Path::new(shown_name).file_name().and_then(|s| s.to_str()).unwrap_or(shown_name).to_owned(),
            drawing,
            keep,
            tol,
            tol_buf: "0.5".into(),
            imported,
            layers_open: false,
            at: 0,
        });
        self.crumbs.push(self.cleaning.as_ref().unwrap().file.clone());
        Ok(())
    }

    /// Rebuild from the drawing with another layer set (a local step).
    fn relayer(&mut self, name: &str) {
        let Some(c) = self.cleaning.as_mut() else { return };
        if let Some(i) = c.keep.iter().position(|k| k == name) {
            c.keep.remove(i);
        } else {
            c.keep.push(name.to_owned());
        }
        let keep = c.keep.clone();
        let imported = built_draft2d::import::sketch(&c.drawing, Some(&keep), self.doc.usage, c.tol);
        self.step();
        let mut s = imported.sketch.clone();
        s.name = self.doc.name.clone();
        s.plane = self.doc.plane.clone();
        s.usage = self.doc.usage;
        self.doc = s;
        let c = self.cleaning.as_mut().unwrap();
        c.imported = imported;
        self.fit = true;
        self.changed();
    }

    /// The row the document keeps: the working sketch.
    pub(super) fn row_json(&self) -> serde_json::Value {
        let mut v = self.doc.to_json();
        if let Some(uid) = &self.uid {
            v["uid"] = uid.clone().into();
        }
        v
    }
    fn dirty(&self) -> bool {
        self.doc != self.base
    }
}

// ---- Tools ------------------------------------------------------------------

impl SketchWs {
    fn arm(&mut self, id: &str) {
        self.tool = id.to_owned();
        self.pts.clear();
        self.picked = None;
        self.picked_end = None;
        self.typed.clear();
        self.typing = false;
        self.refusal = None;
        self.editing_dim = None;
        // One-click fixes run at once and fall back to Select.
        match id {
            "merge_arcs" | "drop_fillets" | "close_gaps" => {
                let tol = self.cleaning.as_ref().map_or(0.5, |c| c.tol);
                let list = cleanup::issues(&self.doc, tol, Some("mm"));
                let want = match id {
                    "merge_arcs" => "segmented",
                    "drop_fillets" => "fillets",
                    _ => "gaps",
                };
                if let Some(issue) = list.into_iter().find(|i| i.key() == want) {
                    self.step();
                    let n = cleanup::fix(&mut self.doc, &issue);
                    self.changed();
                    self.refresh_cleaning();
                    let text = match id {
                        "merge_arcs" => format!("{} {n} {}", tr("Merged"), tr("segments into arcs")),
                        "drop_fillets" => format!("{} {n} {}", tr("Dropped"), tr("small fillets")),
                        _ => format!("{} {n} {}", tr("Closed"), tr("gaps")),
                    };
                    self.say(text, Some("undo"));
                } else {
                    let text = match id {
                        "merge_arcs" => tr("No runs of short segments within the tolerance"),
                        "drop_fillets" => tr("No fillets under the limit"),
                        _ => tr("No gaps within the tolerance"),
                    };
                    self.say(text, None);
                }
                self.tool = "select".into();
            }
            _ => {}
        }
    }

    /// The point under the pointer, with inference.
    fn snapped(&self, at: DVec2, shift: bool) -> Snap {
        let radius = 9.0 / self.zoom as f64;
        let reference = self.pts.last().copied();
        let s = snap::snap(at, radius, &self.curves(), self.context(), self.doc.origin, reference);
        if shift {
            // Shift keeps the axis: no snapping off it.
            if let Some(r) = reference {
                let d = at - r;
                return Snap { at: if d.x.abs() >= d.y.abs() { DVec2::new(at.x, r.y) } else { DVec2::new(r.x, at.y) }, kind: SnapKind::Free, curve: None };
            }
        }
        s
    }

    /// A click on the canvas at sketch point `p` (snapped `s`).
    fn click(&mut self, p: DVec2, s: Snap, double: bool, shift: bool, alt: bool) {
        self.refusal = None;
        let pick_r = 7.0 / self.zoom as f64;
        match self.tool.as_str() {
            "select" => {
                if let Some(d) = self.dim_at(p) {
                    if double {
                        self.editing_dim = Some(d);
                        self.typed.clear();
                        self.typing = true;
                    }
                    return;
                }
                match self.nearest_curve(p, pick_r) {
                    Some(id) if shift => {
                        if let Some(i) = self.selection.iter().position(|x| *x == id) {
                            self.selection.remove(i);
                        } else {
                            self.selection.push(id);
                        }
                    }
                    Some(id) => {
                        if double {
                            // The connected chain.
                            self.selection = edit::chain(&self.doc, id).0.into_iter().map(|c| c.0).collect();
                        } else {
                            self.selection = vec![id];
                        }
                    }
                    None => self.selection.clear(),
                }
            }
            "line" => {
                if double && self.pts.len() >= 1 {
                    self.pts.clear();
                    return;
                }
                self.line_to(s.at);
            }
            "arc" => {
                self.pts.push(s.at);
                if self.pts.len() == 3 {
                    let (a, b, m) = (self.pts[0], self.pts[1], self.pts[2]);
                    self.finish_arc(a, b, m);
                }
            }
            "circle" => {
                if self.pts.is_empty() {
                    self.pts.push(s.at);
                } else {
                    let r = (s.at - self.pts[0]).length();
                    self.add_circle(r);
                }
            }
            "rectangle" => {
                if self.pts.is_empty() {
                    self.pts.push(s.at);
                } else {
                    let a = self.pts[0];
                    self.add_rect(a, s.at);
                }
            }
            "offset" => {
                if let Some(id) = self.picked {
                    let side = p;
                    let c = self.doc.curve(id).map(|c| c.shape);
                    if let Some(c) = c {
                        let d = c.distance(side);
                        self.do_offset(id, d, side);
                    }
                } else if let Some(id) = self.nearest_curve(p, pick_r) {
                    self.picked = Some(id);
                }
            }
            "fillet" => {
                let Some(id) = self.nearest_curve(p, pick_r) else { return };
                if !matches!(self.doc.curve(id).map(|c| c.shape), Some(Shape::Line { .. })) {
                    self.refusal = Some(tr("Fillets join two lines"));
                    return;
                }
                match self.picked {
                    None => self.picked = Some(id),
                    Some(first) if first != id => {
                        let r = self.parse_typed().first().copied().unwrap_or(self.fillet_r);
                        self.fillet_r = r;
                        let mut next = self.doc.clone();
                        match edit::fillet(&mut next, first, id, r) {
                            Ok(_) => {
                                self.step();
                                self.doc = next;
                                self.changed();
                            }
                            Err(e) => self.refusal = Some(tr(e)),
                        }
                        self.picked = None;
                        self.typed.clear();
                    }
                    _ => {}
                }
            }
            "trim" | "extend" => {
                let Some(id) = self.nearest_curve(p, pick_r) else { return };
                let mut next = self.doc.clone();
                let r = if self.tool == "trim" { edit::trim(&mut next, id, p).map(drop) } else { edit::extend(&mut next, id, p) };
                match r {
                    Ok(()) => {
                        self.step();
                        self.doc = next;
                        self.changed();
                    }
                    Err(e) => self.refusal = Some(tr(e)),
                }
            }
            "dimension" => self.dimension_click(p, s, shift),
            "origin" => {
                self.step();
                self.doc.origin = s.at;
                self.changed();
                self.say(format!("{} {:.1}; {:.1}", tr("Origin set at"), s.at.x, s.at.y), Some("undo"));
                self.tool = "select".into();
            }
            "attach" => {
                if alt {
                    self.step();
                    let n = self.doc.points.len() + 1;
                    self.doc.points.push(NamedPoint { name: format!("point_{n}"), at: s.at });
                    self.changed();
                    return;
                }
                if let Some(name) = self.attach_at(p) {
                    self.step();
                    self.doc.attach = name;
                    self.changed();
                }
            }
            "scale_edge" => {
                if let Some(id) = self.nearest_curve(p, pick_r) {
                    self.picked = Some(id);
                    self.typed.clear();
                    self.typing = true;
                }
            }
            _ => {}
        }
    }

    fn line_to(&mut self, q: DVec2) {
        if let Some(&p) = self.pts.last() {
            if (q - p).length() < 1e-6 {
                return;
            }
            self.step();
            self.doc.add(Shape::Line { a: p, b: q });
            self.changed();
            // Closing on the chain's first point ends it.
            if self.pts.len() > 1 && (q - self.pts[0]).length() < 1e-6 {
                self.pts.clear();
                return;
            }
        }
        self.pts.push(q);
    }
    fn finish_arc(&mut self, a: DVec2, b: DVec2, m: DVec2) {
        self.pts.clear();
        match arc3(a, m, b) {
            Some(sh) => {
                self.step();
                self.doc.add(sh);
                self.changed();
            }
            None => self.refusal = Some(tr("The three points are in line: that is a line")),
        }
    }
    fn add_circle(&mut self, r: f64) {
        let c = self.pts[0];
        self.pts.clear();
        if r > 1e-6 {
            self.step();
            self.doc.add(Shape::Circle { c, r });
            self.changed();
        }
    }
    fn add_rect(&mut self, a: DVec2, b: DVec2) {
        self.pts.clear();
        if (a.x - b.x).abs() < 1e-6 || (a.y - b.y).abs() < 1e-6 {
            self.refusal = Some(tr("A rectangle needs a width and a height"));
            return;
        }
        self.step();
        let p = [a, DVec2::new(b.x, a.y), b, DVec2::new(a.x, b.y)];
        for i in 0..4 {
            self.doc.add(Shape::Line { a: p[i], b: p[(i + 1) % 4] });
        }
        self.changed();
    }
    fn do_offset(&mut self, id: u32, d: f64, side: DVec2) {
        let mut next = self.doc.clone();
        match edit::offset(&mut next, id, d, side) {
            Ok(_) => {
                self.step();
                self.doc = next;
                self.changed();
                self.picked = None;
            }
            Err(e) => self.refusal = Some(tr(e)),
        }
    }
    fn dimension_click(&mut self, p: DVec2, s: Snap, shift: bool) {
        let pick_r = 7.0 / self.zoom as f64;
        // A second end for a distance.
        let end_ref = |ws: &SketchWs, at: DVec2| -> Option<PointRef> {
            ws.doc.ends_at(at, 1e-4).first().map(|(curve, at)| PointRef { curve: *curve, at: *at }).or_else(|| {
                ws.doc.curves.iter().find(|c| c.shape.centre().is_some_and(|x| (x - at).length() < 1e-4)).map(|c| PointRef { curve: c.id, at: At::Centre })
            })
        };
        if let Some(first) = self.picked_end {
            if let Some(second) = (s.kind != SnapKind::Free).then(|| end_ref(self, s.at)).flatten() {
                let (pa, pb) = (self.doc.point(first), self.doc.point(second));
                let axis = match (pa, pb) {
                    (Some(a), Some(b)) if (a.y - b.y).abs() < 1e-6 => Axis::Horizontal,
                    (Some(a), Some(b)) if (a.x - b.x).abs() < 1e-6 => Axis::Vertical,
                    _ => Axis::Aligned,
                };
                self.add_dim(DimKind::Distance, first, Some(second), axis);
            }
            self.picked_end = None;
            return;
        }
        if let Some(first) = self.picked {
            if let Some(id) = self.nearest_curve(p, pick_r).filter(|id| *id != first) {
                self.add_dim(DimKind::Angle, PointRef { curve: first, at: At::Mid }, Some(PointRef { curve: id, at: At::Mid }), Axis::Aligned);
            }
            self.picked = None;
            return;
        }
        if matches!(s.kind, SnapKind::End | SnapKind::Centre) {
            if let Some(r) = end_ref(self, s.at) {
                self.picked_end = Some(r);
                return;
            }
        }
        let Some(id) = self.nearest_curve(p, pick_r) else { return };
        let shape = self.doc.curve(id).unwrap().shape;
        match shape {
            Shape::Line { .. } if shift => self.picked = Some(id),
            Shape::Line { .. } => self.add_dim(DimKind::Length, PointRef { curve: id, at: At::Mid }, None, Axis::Aligned),
            _ => self.add_dim(DimKind::Radius, PointRef { curve: id, at: At::Centre }, None, Axis::Aligned),
        }
    }
    fn add_dim(&mut self, kind: DimKind, a: PointRef, b: Option<PointRef>, axis: Axis) {
        let mut next = self.doc.clone();
        match edit::add_dim(&mut next, kind, a, b, axis) {
            Ok(id) => {
                self.step();
                self.doc = next;
                self.editing_dim = Some(id);
                self.typed.clear();
                self.typing = true;
                self.changed();
            }
            Err(e) => self.refusal = Some(tr(e)),
        }
    }

    /// Where a dimension's text sits on screen.
    fn dim_label(&self, id: u32) -> Option<(Pos2, Pos2, Pos2, String)> {
        let d = self.doc.dims.iter().find(|d| d.id == id)?;
        let value = edit::measure(&self.doc, d)?;
        let text = if d.kind == DimKind::Angle {
            format!("{value:.1}°")
        } else if d.kind == DimKind::Radius {
            format!("R{}", fmt_mm(value))
        } else {
            fmt_mm(value)
        };
        let off = (d.offset as f32).max(6.0) * self.zoom.min(3.0).max(0.6);
        match d.kind {
            DimKind::Length => {
                let Shape::Line { a, b } = self.doc.curve(d.a.curve)?.shape else { return None };
                let (sa, sb) = (self.w2s(a), self.w2s(b));
                let n = (sb - sa).normalized().rot90() * -off;
                Some((sa + n, sb + n, sa.lerp(sb, 0.5) + n, text))
            }
            DimKind::Distance => {
                let (a, b) = (self.doc.point(d.a)?, self.doc.point(d.b?)?);
                let (a2, b2) = match d.axis {
                    Axis::Horizontal => (a, DVec2::new(b.x, a.y)),
                    Axis::Vertical => (a, DVec2::new(a.x, b.y)),
                    Axis::Aligned => (a, b),
                };
                let (sa, sb) = (self.w2s(a2), self.w2s(b2));
                let n = if (sb - sa).length() > 1.0 { (sb - sa).normalized().rot90() * -off } else { vec2(0.0, -off) };
                Some((sa + n, sb + n, sa.lerp(sb, 0.5) + n, text))
            }
            DimKind::Radius => {
                let s = self.doc.curve(d.a.curve)?.shape;
                let (c, r) = (s.centre()?, s.radius()?);
                let at = match s {
                    Shape::Arc { a0, a1, .. } => polar(c, r, (a0 + a1) / 2.0),
                    _ => polar(c, r, std::f64::consts::FRAC_PI_4),
                };
                let (sc, sa) = (self.w2s(c), self.w2s(at));
                Some((sc, sa, sa + (sa - sc).normalized() * 14.0, text))
            }
            DimKind::Angle => {
                let a = self.doc.curve(d.a.curve)?.shape;
                let mid = self.w2s(a.mid());
                Some((mid, mid, mid + vec2(16.0, -16.0), text))
            }
        }
    }
    fn dim_at(&self, p: DVec2) -> Option<u32> {
        let sp = self.w2s(p);
        self.doc.dims.iter().map(|d| d.id).find(|id| self.dim_label(*id).is_some_and(|(_, _, t, _)| (t - sp).length() < 16.0))
    }
    fn attach_points(&self) -> Vec<(String, DVec2)> {
        ATTACH.iter().map(|n| n.to_string()).chain(self.doc.points.iter().map(|p| p.name.clone())).filter_map(|n| self.doc.attachment(&n).map(|p| (n, p))).collect()
    }
    fn attach_at(&self, p: DVec2) -> Option<String> {
        let r = 8.0 / self.zoom as f64;
        self.attach_points().into_iter().filter(|(_, q)| (*q - p).length() <= r).min_by(|a, b| (a.1 - p).length().total_cmp(&(b.1 - p).length())).map(|x| x.0)
    }

    /// Enter in the value field: the typed value applied to what the tool
    /// is doing.
    fn apply_typed(&mut self, cursor: Option<DVec2>) {
        let v = self.parse_typed();
        let Some(&first) = v.first() else { return };
        self.refusal = None;
        if let Some(dim) = self.editing_dim {
            let mut next = self.doc.clone();
            match edit::drive(&mut next, dim, first) {
                Ok(()) => {
                    self.step();
                    self.doc = next;
                    self.changed();
                }
                Err(e) => self.refusal = Some(tr(e)),
            }
            self.editing_dim = None;
            self.typed.clear();
            self.typing = false;
            return;
        }
        let dir = |from: DVec2| cursor.map(|c| (c - from).try_normalize().unwrap_or(DVec2::X)).unwrap_or(DVec2::X);
        match self.tool.as_str() {
            "line" => {
                if let Some(&p) = self.pts.last() {
                    let q = if v.len() >= 2 { p + DVec2::new(v[0], v[1]) } else { p + dir(p) * first };
                    self.line_to(q);
                }
            }
            "circle" if !self.pts.is_empty() => self.add_circle(first),
            "rectangle" if !self.pts.is_empty() => {
                let a = self.pts[0];
                let c = cursor.unwrap_or(a + DVec2::ONE);
                let (sx, sy) = (if c.x >= a.x { 1.0 } else { -1.0 }, if c.y >= a.y { 1.0 } else { -1.0 });
                let h = v.get(1).copied().unwrap_or(first);
                self.add_rect(a, a + DVec2::new(first * sx, h * sy));
            }
            "arc" if self.pts.len() == 2 => {
                let (a, b) = (self.pts[0], self.pts[1]);
                let side = cursor.map(|c| if (b - a).perp_dot(c - a) >= 0.0 { 1.0 } else { -1.0 }).unwrap_or(1.0);
                let m = (a + b) / 2.0 + (b - a).normalize().perp() * first * side;
                self.finish_arc(a, b, m);
            }
            "offset" => {
                if let (Some(id), Some(c)) = (self.picked, cursor) {
                    self.do_offset(id, first, c);
                }
            }
            "fillet" => {
                self.fillet_r = first;
                self.say(format!("{} {}", tr("Fillet radius"), fmt_mm(first)), None);
            }
            "scale_edge" => {
                if let Some(id) = self.picked {
                    if let Some(k) = cleanup::edge_factor(&self.doc, id, first) {
                        self.step();
                        self.doc.scale(k);
                        self.changed();
                        self.refresh_cleaning();
                        self.fit = true;
                        let unit = if (k - 25.4).abs() < 0.05 { format!(" · {}", tr("inches → mm")) } else { String::new() };
                        self.say(format!("{} × {}{unit}", tr("Scaled everything"), trim_num(k, 4)), Some("undo"));
                    }
                    self.picked = None;
                }
            }
            _ => {}
        }
        self.typed.clear();
        self.typing = false;
    }

    /// Esc: drop what the tool holds, then the tool, then ask to leave.
    fn escape(&mut self) -> bool {
        if self.leave_prompt {
            self.leave_prompt = false;
            return false;
        }
        if self.typing || !self.typed.is_empty() {
            self.typed.clear();
            self.typing = false;
            self.editing_dim = None;
            return false;
        }
        if !self.pts.is_empty() || self.picked.is_some() || self.picked_end.is_some() {
            self.pts.clear();
            self.picked = None;
            self.picked_end = None;
            return false;
        }
        if self.tool != "select" {
            self.arm("select");
            return false;
        }
        if !self.selection.is_empty() {
            self.selection.clear();
            return false;
        }
        self.leave_prompt = true;
        true
    }
}

pub(super) fn fmt_mm(v: f64) -> String {
    trim_num(v, 2)
}
fn trim_num(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

// ---- Chrome -------------------------------------------------------------------

impl Ui {
    pub(super) fn sketch_enter(&mut self, spec: &str) -> Result<(), String> {
        let v: serde_json::Value = serde_json::from_str(spec).map_err(|e| e.to_string())?;
        if self.sketch.groups.is_empty() {
            self.sketch.groups = self.sketch_tool_groups();
        }
        self.sketch.enter(&v)?;
        self.sheets.shown = false;
        Ok(())
    }
    fn sketch_tool_groups(&mut self) -> Vec<Vec<Tool>> {
        TOOLS
            .iter()
            .map(|g| {
                g.iter()
                    .map(|(id, label, key, icon)| {
                        let tex = crate::tool_cursors::rail_pixels(icon).map(|img| {
                            let size = [img.width() as usize, img.height() as usize];
                            self.ctx.load_texture(format!("sketch-{icon}"), egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR)
                        });
                        Tool { enabled: true, id: (*id).into(), label: tr(label), key: (*key).into(), icon: tex }
                    })
                    .collect()
            })
            .collect()
    }
    pub(super) fn sketch_areas(&self) -> [(&'static str, bool); 5] {
        let on = self.sketch.shown && !self.review && !self.lsheet.open;
        let peek = self.sketch.peek;
        [("sketch_canvas", on && !peek), ("sketch_tools", on), ("sketch_pane", on), ("sketch_inset", on), ("sketch_prompt", on && self.sketch.leave_prompt)]
    }
    pub(super) fn sketch_claims_key(&self) -> bool {
        self.sketch.shown && !self.lsheet.open && self.sheet.is_none() && self.dialog.is_none() && !self.cmd_open
    }
    /// A key while the workspace is up: a pane field takes it while it has
    /// focus; else the workspace's own handling (next frame).
    pub(super) fn sketch_key(&mut self, name: &str, pressed: bool, mods: u8) -> bool {
        if self.ctx.wants_keyboard_input() {
            if let Some(key) = egui::Key::from_name(name) {
                self.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods_from(mods) });
            }
            return true;
        }
        if pressed && !name.is_empty() {
            self.sketch.keys.push((name.to_owned(), mods));
        }
        true
    }
    pub(super) fn sketch_text(&mut self, text: String) -> bool {
        if self.ctx.wants_keyboard_input() {
            self.events.push(egui::Event::Text(text));
        } else {
            self.sketch.texts.push(text);
        }
        true
    }
    /// A window command (⌘Z from the menu) while sketching.
    pub(super) fn sketch_command(&mut self, cmd: &str) -> bool {
        if !self.sketch.shown {
            return false;
        }
        match cmd {
            "undo" => self.sketch.local_undo(false),
            "redo" => self.sketch.local_undo(true),
            "done" => self.sketch_done(),
            "cancel" => self.sketch_cancel(),
            _ => return false,
        }
        true
    }
    fn sketch_done(&mut self) {
        let row = self.sketch.row_json();
        self.actions.push(format!("sketch:done{SEP_S}{row}"));
        self.sketch.shown = false;
        self.sketch.discarded = None;
    }
    fn sketch_cancel(&mut self) {
        let kept = (self.sketch.dirty()).then(|| (self.sketch.doc.clone(), self.sketch.uid.clone()));
        self.actions.push("sketch:cancel".into());
        self.sketch.shown = false;
        self.sketch.discarded = kept;
    }
    /// The toast's Undo after Cancel: back into the sketch as it was.
    pub(super) fn sketch_reopen_discarded(&mut self) -> Option<String> {
        let (doc, uid) = self.sketch.discarded.take()?;
        Some(serde_json::json!({"sketch": doc.to_json(), "uid": uid}).to_string())
    }

    /// The breadcrumb in the top bar, from the left group's end.
    pub(super) fn sketch_crumbs(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
        if text_button_response(ui, "‹", &tr("Leave the sketch (Esc Esc)"), true).clicked() {
            self.sketch.leave_prompt = true;
        }
        let mut crumbs = self.sketch.crumbs.clone();
        let name = if self.sketch.doc.name.is_empty() { tr(if self.sketch.doc.usage == Use::Section { "New section" } else { "New profile" }) } else { self.sketch.doc.name.clone() };
        if crumbs.last() != Some(&name) && self.sketch.cleaning.is_none() {
            crumbs.push(name);
        }
        // Fit the room left of Cancel and Done: shorten the longest crumb
        // (never the last) until the trail fits.
        let right = self.top_right_w.max(230.0) + 2.0 * SPACE_M;
        let budget = (ui.available_width() - right).max(120.0);
        let width = |ui: &egui::Ui, c: &[String]| -> f32 {
            c.iter().enumerate().map(|(i, t)| ui.painter().layout_no_wrap(t.clone(), if i + 1 == c.len() { strong(TYPE_BODY) } else { body() }, INK).size().x + 18.0).sum()
        };
        let n = crumbs.len();
        let mut guard = 0;
        while width(ui, &crumbs) > budget && guard < 400 {
            guard += 1;
            let Some(i) = (0..n.saturating_sub(1)).filter(|i| crumbs[*i].chars().count() > 4).max_by_key(|i| crumbs[*i].chars().count()) else { break };
            let keep = crumbs[i].chars().count().saturating_sub(4);
            crumbs[i] = crumbs[i].chars().take(keep).collect::<String>().trim_end_matches(['…', ' ']).to_owned() + "…";
        }
        for (i, c) in crumbs.iter().enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new("›").font(body()).color(OFF));
            }
            let last = i + 1 == n;
            let full = self.sketch.crumbs.get(i).cloned().unwrap_or_default();
            let r = ui.label(egui::RichText::new(c).font(if last { strong(TYPE_BODY) } else { body() }).color(if last { INK } else { DIM }));
            if c.ends_with('…') {
                r.on_hover_text(full);
            }
        }
    }
    /// The top bar's right end: scale and unit, Cancel, Done.
    pub(super) fn sketch_bar_right(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
        ui.label(egui::RichText::new(format!("{} · 1:1 · mm", tr("Sketch"))).font(meta()).color(FAINT));
        let cancel = kit::panel_button(ui, &tr("Cancel"), "", 0, None).on_hover_text(tr("Discard the changes made in this sketch"));
        self.cluster_buttons.push(("sketch_cancel".into(), cancel.rect.center().into()));
        if cancel.clicked() {
            self.sketch_cancel();
        }
        let done = kit::panel_button(ui, &tr("Done"), "↩", 1, None).on_hover_text(tr("Keep the sketch: one step in the model's history"));
        self.cluster_buttons.push(("sketch_done".into(), done.rect.center().into()));
        if done.clicked() {
            self.sketch_done();
        }
    }

    /// The workspace's frame under the top bar.
    pub(super) fn sketch_frame(&mut self, ctx: &egui::Context, screen: Rect) {
        self.sketch.now = self.start.elapsed().as_secs_f64();
        let (top, bottom) = self.band(ctx, screen);
        // Left: the tool column in the rail's place.
        let active = self.sketch.tool.clone();
        let (clicked, rail_w) = super::rail::show(ctx, "sketch_tools", &self.sketch.groups, &active, top, bottom - top, 0.0);
        if let Some(id) = clicked {
            self.sketch.arm(&id);
        }
        // Right: the settings pane in the dock's place.
        let pane_w = self.dock_w.min((screen.width() * 0.45).max(DOCK_MIN_W)).min(DOCK_W + 20.0);
        let pane_rect = Rect::from_min_max(pos2(screen.max.x - GUT - pane_w, top), pos2(screen.max.x - GUT, bottom));
        self.sketch_pane(ctx, pane_rect);
        let canvas = Rect::from_min_max(pos2(GUT + rail_w + GUT, top), pos2(pane_rect.min.x - GUT, bottom));
        self.sketch.canvas = Some(canvas);
        self.sketch_canvas(ctx, screen, canvas);
        self.sketch_inset(ctx, canvas);
        self.sketch_bottom(ctx, screen);
        self.sketch_prompt(ctx, canvas);
        self.sketch_toast(ctx, canvas);
    }

    fn sketch_pane(&mut self, ctx: &egui::Context, r: Rect) {
        let s = &mut self.sketch;
        let mut actions: Vec<String> = Vec::new();
        egui::Area::new(egui::Id::new("sketch_pane"))
            .constrain(false)
            .fixed_pos(r.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Panel).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(r.width() - 20.0);
                    ui.set_max_width(r.width() - 20.0);
                    ui.set_height(r.height() - 20.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                    egui::ScrollArea::vertical().id_salt("sketch_pane_scroll").show(ui, |ui| {
                        sketch_record(ui, s, &mut actions);
                    });
                });
            });
        for a in actions {
            self.actions.push(a);
        }
    }

    fn sketch_bottom(&mut self, ctx: &egui::Context, screen: Rect) {
        let s = &self.sketch;
        let icon = s.groups.iter().flatten().find(|t| t.id == s.tool).and_then(|t| t.icon.clone());
        let step = match s.tool.as_str() {
            "offset" | "fillet" | "scale_edge" => usize::from(s.picked.is_some()),
            "dimension" => usize::from(s.picked.is_some() || s.picked_end.is_some()),
            _ => s.pts.len(),
        };
        let hint = s.refusal.clone().unwrap_or_else(|| tr(tool_hint(&s.tool, step)));
        let refused = s.refusal.is_some();
        let cursor = s.hover.map(|h| format!("{}; {}", fmt_mm(h.at.x), fmt_mm(h.at.y))).unwrap_or_default();
        let label = tr(tool_label(&s.tool));
        egui::Area::new(egui::Id::new("bottom"))
            .constrain(false)
            .fixed_pos(pos2(0.0, screen.max.y - BOT_H))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Bar).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.set_width(screen.width() - 16.0);
                    ui.set_height(BOT_H - 8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                        ui.label(egui::RichText::new("mm").font(meta()).color(DIM));
                        bar_divider(ui);
                        ui.add_space(SPACE_S);
                        if let Some(tex) = &icon {
                            let (r, _) = ui.allocate_exact_size(vec2(22.0, 28.0), Sense::hover());
                            egui::Image::new(tex).tint(INK).paint_at(ui, Rect::from_center_size(r.center(), vec2(20.0, 20.0)));
                        }
                        ui.label(egui::RichText::new(label).font(strong(TYPE_BODY)).color(INK));
                        ui.add_space(SPACE_S);
                        let room = (ui.available_width() - 160.0).max(40.0);
                        let mut job = egui::text::LayoutJob::single_section(hint, egui::TextFormat { font_id: body(), color: if refused { WARN } else { DIM }, ..Default::default() });
                        job.wrap.max_width = room;
                        job.wrap.max_rows = 1;
                        job.wrap.break_anywhere = true;
                        let g = ui.painter().layout_job(job);
                        let (r, _) = ui.allocate_exact_size(vec2(room, 28.0), Sense::hover());
                        ui.painter().galley(pos2(r.min.x, r.center().y - g.size().y / 2.0), g, DIM);
                        ui.label(egui::RichText::new(cursor).font(meta()).color(FAINT));
                    });
                });
            });
    }

    fn sketch_prompt(&mut self, ctx: &egui::Context, canvas: Rect) {
        if !self.sketch.leave_prompt {
            return;
        }
        let mut answer = None;
        egui::Area::new(egui::Id::new("sketch_prompt"))
            .pivot(egui::Align2::CENTER_CENTER)
            .fixed_pos(canvas.center())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(14)).show(ui, |ui| {
                    ui.set_width(300.0);
                    ui.label(egui::RichText::new(tr("Leave the sketch?")).font(title_font()).color(INK));
                    ui.add_space(SPACE_XS);
                    ui.label(egui::RichText::new(tr("Done keeps it as one step; Discard drops the changes.")).font(body()).color(DIM));
                    ui.add_space(SPACE_M);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                        if kit::panel_button(ui, &tr("Stay"), "", 0, None).clicked() {
                            answer = Some("stay");
                        }
                        if kit::panel_button(ui, &tr("Discard"), "", 2, None).clicked() {
                            answer = Some("discard");
                        }
                        if kit::panel_button(ui, &tr("Done"), "↩", 1, None).clicked() {
                            answer = Some("done");
                        }
                    });
                });
            });
        match answer {
            Some("stay") => self.sketch.leave_prompt = false,
            Some("discard") => {
                self.sketch.leave_prompt = false;
                self.sketch_cancel();
            }
            Some("done") => {
                self.sketch.leave_prompt = false;
                self.sketch_done();
            }
            _ => {}
        }
    }

    fn sketch_toast(&mut self, ctx: &egui::Context, canvas: Rect) {
        let Some((text, at, undo)) = self.sketch.toast.clone() else { return };
        if self.sketch.now - at > 6.0 {
            self.sketch.toast = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let mut do_undo = false;
        egui::Area::new(egui::Id::new("sketch_toast"))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(canvas.center().x, canvas.max.y - SPACE_M))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                        let (r, _) = ui.allocate_exact_size(vec2(8.0, 18.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, OK);
                        ui.label(egui::RichText::new(&text).font(body()).color(INK));
                        if undo.is_some() && kit::chip_button(ui, &tr("Undo")) {
                            do_undo = true;
                        }
                    });
                });
            });
        if do_undo {
            self.sketch.local_undo(false);
            self.sketch.toast = None;
        }
    }

    // ---- The canvas ---------------------------------------------------------------

    fn sketch_canvas(&mut self, ctx: &egui::Context, screen: Rect, canvas: Rect) {
        let _ = screen;
        if self.sketch.peek {
            // Holding the inset shows the 3D view through the canvas.
            return;
        }
        let (keys, texts) = (std::mem::take(&mut self.sketch.keys), std::mem::take(&mut self.sketch.texts));
        let mut leave: Option<&str> = None;
        let s = &mut self.sketch;
        if s.fit {
            s.fit = false;
            let bounds = s.doc.bounds().or(s.fit_hint).or_else(|| {
                s.ghost.iter().map(|g| g.bounds()).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
            });
            let room = canvas.shrink(60.0);
            match bounds {
                Some((lo, hi)) => {
                    let size = (hi - lo).max(DVec2::splat(1.0));
                    s.zoom = ((room.width() as f64 / size.x).min(room.height() as f64 / size.y) as f32).clamp(0.02, 400.0);
                    let mid = (lo + hi) / 2.0;
                    s.org = pos2(room.center().x - mid.x as f32 * s.zoom, room.center().y + mid.y as f32 * s.zoom);
                }
                None => {
                    s.zoom = 2.0;
                    s.org = canvas.center();
                }
            }
        }
        let resp = egui::Area::new(egui::Id::new("sketch_canvas"))
            .constrain(false)
            .fixed_pos(canvas.min)
            .order(egui::Order::Background)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(canvas.size(), Sense::click_and_drag());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, R_CONTROL, WELL);
                (rect, resp)
            })
            .inner;
        let (rect, resp) = resp;
        let (mods, scroll, pointer) = ctx.input(|i| (i.modifiers, i.raw_scroll_delta, i.pointer.hover_pos()));
        // Pan with the middle button or Space-less two-finger slide; zoom with ⌘/Ctrl+wheel or the wheel.
        if resp.dragged_by(egui::PointerButton::Middle) || resp.dragged_by(egui::PointerButton::Secondary) {
            s.org += resp.drag_delta();
        }
        if resp.hovered() && scroll != Vec2::ZERO {
            if let Some(p) = pointer {
                if mods.shift && !mods.command {
                    s.org += scroll;
                } else {
                    let k = (1.0 + scroll.y * 0.0025).clamp(0.5, 2.0);
                    let new = (s.zoom * k).clamp(0.01, 800.0);
                    let k = new / s.zoom;
                    s.org = p + (s.org - p) * k;
                    s.zoom = new;
                }
            }
        }
        let cursor = pointer.filter(|p| rect.contains(*p)).map(|p| s.s2w(p));
        s.hover = cursor.map(|c| s.snapped(c, mods.shift));
        s.hover_curve = cursor.and_then(|c| s.nearest_curve(c, 7.0 / s.zoom as f64));
        if resp.clicked_by(egui::PointerButton::Primary) || resp.double_clicked() {
            if let (Some(c), Some(h)) = (cursor, s.hover) {
                if s.leave_prompt {
                    s.leave_prompt = false;
                } else {
                    s.click(c, h, resp.double_clicked(), mods.shift, mods.alt);
                }
            }
        }
        // Keys and typing.
        for t in texts {
            for ch in t.chars() {
                if ch.is_ascii_digit() || matches!(ch, '.' | ';' | ',' | '-' | '+') || (s.typing && matches!(ch, 'x' | '×')) {
                    s.typed.push(ch);
                    s.typing = true;
                } else if !s.typing {
                    let tool = match ch.to_ascii_lowercase() {
                        ' ' => Some("select"),
                        'l' => Some("line"),
                        'a' => Some("arc"),
                        'c' => Some("circle"),
                        'r' => Some("rectangle"),
                        'f' => Some("offset"),
                        'g' => Some("fillet"),
                        't' => Some("trim"),
                        'x' => Some("extend"),
                        'd' => Some("dimension"),
                        'o' => Some("origin"),
                        'p' => Some("attach"),
                        _ => None,
                    };
                    if let Some(t) = tool {
                        s.arm(t);
                    }
                }
            }
        }
        for (name, m) in keys {
            let command = m & (2 | 8) != 0;
            match name.as_str() {
                "Escape" => {
                    s.escape();
                }
                "Enter" => {
                    if s.leave_prompt {
                        leave = Some("done");
                    } else if !s.typed.is_empty() {
                        s.apply_typed(cursor);
                    } else if s.tool == "line" && !s.pts.is_empty() {
                        s.pts.clear();
                    } else if s.pts.is_empty() && s.picked.is_none() && !s.typing {
                        leave = Some("done");
                    }
                }
                "Tab" => {
                    if s.typing {
                        s.typed.push(';');
                    }
                }
                "Backspace" if s.typing || !s.typed.is_empty() => {
                    s.typed.pop();
                    if s.typed.is_empty() && s.editing_dim.is_none() {
                        s.typing = false;
                    }
                }
                "Backspace" | "Delete" => {
                    if s.tool == "attach" {
                        if let Some(name) = cursor.and_then(|c| s.attach_at(c)) {
                            if let Some(i) = s.doc.points.iter().position(|p| p.name == name) {
                                s.step();
                                s.doc.points.remove(i);
                                s.changed();
                            }
                        }
                    } else if !s.selection.is_empty() {
                        s.step();
                        for id in std::mem::take(&mut s.selection) {
                            s.doc.remove(id);
                        }
                        s.changed();
                        s.refresh_cleaning();
                    }
                }
                "Z" if command => s.local_undo(m & 4 != 0),
                "Y" if command => s.local_undo(true),
                "A" if command => s.selection = s.doc.curves.iter().map(|c| c.id).collect(),
                _ => {}
            }
        }
        // Paint.
        // The canvas area's own layer, over its well fill.
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("sketch_canvas"))).with_clip_rect(rect);
        sketch_paint(&painter, s, rect, cursor);
        // The value field at the cursor.
        let caption = value_caption(s);
        if let (Some(cap), Some(p)) = (caption, pointer.filter(|p| rect.contains(*p))) {
            let shown = if s.typed.is_empty() { live_reading(s, cursor) } else { s.typed.clone() };
            egui::Area::new(egui::Id::new("sketch_value"))
                .fixed_pos(p + vec2(18.0, 14.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    kit::value_pill_tone(ui, &cap, &shown, s.typing, false, s.refusal.is_some());
                });
        }
        if let Some(l) = leave {
            if l == "done" {
                self.sketch_done();
            }
        }
    }

    fn sketch_inset(&mut self, ctx: &egui::Context, canvas: Rect) {
        let s = &mut self.sketch;
        if s.inset.n == glam::DVec3::ZERO {
            return;
        }
        let w = (canvas.width() * 0.26).clamp(180.0, 300.0);
        let r = Rect::from_min_size(pos2(canvas.min.x + SPACE_M, canvas.max.y - SPACE_M - w * 0.64), vec2(w, w * 0.64));
        let resp = egui::Area::new(egui::Id::new("sketch_inset"))
            .constrain(false)
            .fixed_pos(r.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| ui.allocate_exact_size(r.size(), Sense::click_and_drag()).1)
            .inner;
        s.peek = resp.is_pointer_button_down_on();
        if s.peek {
            ctx.request_repaint();
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("sketch_inset_paint"))).with_clip_rect(r);
        painter.rect_filled(r, R_CONTROL, if s.peek { PEEK_SCRIM } else { CARD });
        painter.rect_stroke(r, R_CONTROL, Stroke::new(1.0_f32, if resp.hovered() { ACCENT_LINE } else { EDGE }), egui::StrokeKind::Inside);
        if !s.peek {
            inset_paint(&painter, s, r.shrink(10.0));
        }
        let label = if s.inset.label.is_empty() { tr("Sketch plane") } else { s.inset.label.clone() };
        let g = painter.layout_no_wrap(label, meta(), INK_2);
        let tag = Rect::from_min_size(r.min + vec2(6.0, 6.0), g.size() + vec2(10.0, 4.0));
        painter.rect_filled(tag, R_INSET, SHEET);
        painter.galley(tag.min + vec2(5.0, 2.0), g, INK_2);
        resp.on_hover_text(tr("Hold to look at the 3D view"));
    }
}

fn value_caption(s: &SketchWs) -> Option<String> {
    if s.editing_dim.is_some() {
        return Some(tr("Dimension"));
    }
    Some(tr(match (s.tool.as_str(), s.pts.len(), s.picked.is_some()) {
        ("line", n, _) if n >= 1 => "Length",
        ("arc", 2, _) => "Bulge",
        ("circle", 1, _) => "Radius",
        ("rectangle", 1, _) => "w; h",
        ("offset", _, true) => "Distance",
        ("fillet", _, _) => "Radius",
        ("scale_edge", _, true) => "Edge is",
        _ => return None,
    }))
}

fn live_reading(s: &SketchWs, cursor: Option<DVec2>) -> String {
    let Some(c) = s.hover.map(|h| h.at).or(cursor) else { return String::new() };
    match (s.tool.as_str(), s.pts.as_slice()) {
        ("line", [.., p]) => fmt_mm((c - *p).length()),
        ("circle", [p]) => fmt_mm((c - *p).length()),
        ("rectangle", [p]) => format!("{}; {}", fmt_mm((c.x - p.x).abs()), fmt_mm((c.y - p.y).abs())),
        ("arc", [a, b]) => fmt_mm(((c - *a).perp_dot((*b - *a).normalize())).abs()),
        ("offset", _) => s.picked.and_then(|id| s.doc.curve(id)).map(|cv| fmt_mm(cv.shape.distance(c))).unwrap_or_default(),
        ("fillet", _) => fmt_mm(s.fillet_r),
        ("scale_edge", _) => s.picked.and_then(|id| s.doc.curve(id)).map(|cv| fmt_mm(cv.shape.length())).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Grid step in mm for a zoom (px per mm): about 14 px or more.
fn grid_step(zoom: f32) -> f64 {
    let mut step = 1e-3;
    loop {
        for k in [1.0, 2.0, 5.0] {
            if step * k * zoom as f64 >= 14.0 {
                return step * k;
            }
        }
        step *= 10.0;
    }
}

fn shape_points(s: &SketchWs, sh: &Shape) -> Vec<Pos2> {
    let segs = (sh.length() * s.zoom as f64 / 4.0).clamp(12.0, 256.0) as u32;
    let turn = match sh {
        Shape::Arc { a0, a1, .. } => ((segs as f64) * std::f64::consts::TAU / (a1 - a0).max(1e-6)) as u32,
        _ => segs,
    };
    sh.polyline(turn.max(12)).into_iter().map(|p| s.w2s(p)).collect()
}

fn sketch_paint(painter: &egui::Painter, s: &mut SketchWs, rect: Rect, cursor: Option<DVec2>) {
    // Grid.
    let step = grid_step(s.zoom);
    let (lo, hi) = (s.s2w(rect.left_bottom()), s.s2w(rect.right_top()));
    let minor = SKETCH_GRID;
    let major = SKETCH_GRID_MAJOR;
    let mut x = (lo.x / step).floor() * step;
    let mut guard = 0;
    while x <= hi.x && guard < 400 {
        let sx = s.w2s(DVec2::new(x, 0.0)).x;
        let is_major = ((x / step).round() as i64) % 5 == 0;
        painter.vline(sx, rect.y_range(), Stroke::new(1.0_f32, if is_major { major } else { minor }));
        x += step;
        guard += 1;
    }
    let mut y = (lo.y / step).floor() * step;
    guard = 0;
    while y <= hi.y && guard < 400 {
        let sy = s.w2s(DVec2::new(0.0, y)).y;
        let is_major = ((y / step).round() as i64) % 5 == 0;
        painter.hline(rect.x_range(), sy, Stroke::new(1.0_f32, if is_major { major } else { minor }));
        y += step;
        guard += 1;
    }
    // Axes through the origin.
    let o = s.w2s(s.doc.origin);
    let axis_len = 40.0;
    painter.line_segment([o, o + vec2(axis_len, 0.0)], Stroke::new(1.5_f32, AXIS_X));
    painter.line_segment([o, o - vec2(0.0, axis_len)], Stroke::new(1.5_f32, AXIS_Y));
    // Context.
    if s.view == View::Sliced {
        for b in &s.behind {
            let pts = shape_points(s, b);
            painter.add(egui::Shape::line(pts, Stroke::new(1.0_f32, SKETCH_BEHIND)));
        }
    }
    if s.view != View::Isolated {
        for g in &s.ghost {
            let pts = shape_points(s, g);
            painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0_f32, SKETCH_GHOST), 5.0, 3.0));
        }
    }
    // The regions, filled faintly when they close.
    let prof = s.profile().clone();
    for region in &prof.regions {
        let poly: Vec<Pos2> = region.outer.polygon(64).into_iter().map(|p| s.w2s(p)).collect();
        if poly.len() >= 3 {
            let mesh = fill_polygon(&poly, SKETCH_REGION);
            painter.add(mesh);
        }
    }
    // Curves.
    for c in &s.doc.curves {
        let selected = s.selection.contains(&c.id) || s.picked == Some(c.id);
        let hovered = s.hover_curve == Some(c.id) && s.tool != "line" && s.tool != "rectangle" && s.tool != "circle" && s.tool != "arc";
        let colour = if selected { ACCENT } else if hovered { BRIGHT } else { INK };
        let width: f32 = if selected || hovered { 2.0 } else { 1.5 };
        let pts = shape_points(s, &c.shape);
        painter.add(egui::Shape::line(pts, Stroke::new(width, colour)));
    }
    // Problems: open ends red, crossings amber.
    for p in &prof.problems {
        match p {
            Problem::OpenEnd { at, .. } => {
                painter.circle_stroke(s.w2s(*at), 5.0, Stroke::new(1.5_f32, ERROR));
            }
            Problem::Branch { at, .. } | Problem::Crossing { at, .. } => {
                painter.circle_stroke(s.w2s(*at), 5.0, Stroke::new(1.5_f32, WARN));
            }
            Problem::Empty => {}
        }
    }
    // Cleaning: the current issue's curves amber.
    if let Some(c) = &s.cleaning {
        if let Some(issue) = c.imported.issues.get(c.at) {
            let ids: Vec<u32> = match issue {
                Issue::Segmented { runs } => runs.iter().flatten().copied().collect(),
                Issue::SmallFillets { arcs, .. } => arcs.clone(),
                Issue::Gaps { pairs } => pairs.iter().flat_map(|p| [p.0, p.1]).collect(),
                Issue::Duplicates { curves } => curves.clone(),
                Issue::Units { .. } => vec![],
            };
            for id in ids {
                if let Some(cv) = s.doc.curve(id) {
                    let pts = shape_points(s, &cv.shape);
                    for q in &pts {
                        painter.circle_filled(*q, 2.0, WARN);
                    }
                    painter.add(egui::Shape::line(pts, Stroke::new(2.0_f32, WARN)));
                }
            }
        }
    }
    // Dimensions.
    for id in s.doc.dims.iter().map(|d| d.id).collect::<Vec<_>>() {
        let Some((a, b, t, text)) = s.dim_label(id) else { continue };
        let editing = s.editing_dim == Some(id);
        let ink = if editing { ACCENT } else { INK_2 };
        painter.line_segment([a, b], Stroke::new(1.0_f32, ink));
        for e in [a, b] {
            painter.circle_filled(e, 1.8, ink);
        }
        let g = painter.layout_no_wrap(if editing && !s.typed.is_empty() { s.typed.clone() } else { text }, meta(), ink);
        let box_r = Rect::from_center_size(t, g.size() + vec2(8.0, 4.0));
        painter.rect_filled(box_r, R_INSET, WELL);
        painter.galley(box_r.min + vec2(4.0, 2.0), g, ink);
    }
    // Attachment points.
    if s.tool == "attach" || s.doc.usage == Use::Profile {
        let show_all = s.tool == "attach";
        for (name, p) in s.attach_points() {
            let sp = s.w2s(p);
            let default = name == s.doc.attach;
            let extra = !ATTACH.contains(&name.as_str());
            if default {
                painter.circle_filled(sp, 4.5, ACCENT);
            } else if show_all {
                painter.circle_stroke(sp, 4.0, Stroke::new(1.2_f32, if extra { WARN } else { INK_2 }));
            }
            if extra && (show_all || default) {
                painter.text(sp + vec2(0.0, 14.0), egui::Align2::CENTER_CENTER, name.replace('_', " "), meta(), WARN);
            }
        }
    }
    // The tool's preview and the snap.
    if let Some(h) = s.hover {
        let c = h.at;
        let preview = Stroke::new(1.5_f32, ACCENT);
        match (s.tool.as_str(), s.pts.as_slice()) {
            ("line", [.., p]) => {
                painter.line_segment([s.w2s(*p), s.w2s(c)], preview);
            }
            ("circle", [p]) => {
                let r = (c - *p).length();
                painter.add(egui::Shape::line(shape_points(s, &Shape::Circle { c: *p, r }), preview));
            }
            ("rectangle", [p]) => {
                let q = [*p, DVec2::new(c.x, p.y), c, DVec2::new(p.x, c.y), *p];
                painter.add(egui::Shape::line(q.iter().map(|x| s.w2s(*x)).collect(), preview));
            }
            ("arc", [a]) => {
                painter.line_segment([s.w2s(*a), s.w2s(c)], Stroke::new(1.0_f32, ACCENT_LINE));
            }
            ("arc", [a, b]) => {
                if let Some(sh) = arc3(*a, c, *b) {
                    painter.add(egui::Shape::line(shape_points(s, &sh), preview));
                }
            }
            _ => {}
        }
        if let ("offset", Some(id)) = (s.tool.as_str(), s.picked) {
            if let Some(cv) = s.doc.curve(id) {
                let d = cv.shape.distance(c);
                let mut trial = s.doc.clone();
                if let Ok(ids) = edit::offset(&mut trial, id, d.max(1e-3), c) {
                    for nid in ids {
                        if let Some(n) = trial.curve(nid) {
                            painter.add(egui::Shape::line(shape_points(s, &n.shape), preview));
                        }
                    }
                }
            }
        }
        if let Some(e) = s.picked_end.and_then(|r| s.doc.point(r)) {
            painter.line_segment([s.w2s(e), s.w2s(c)], Stroke::new(1.0_f32, ACCENT_LINE));
        }
        let sp = s.w2s(c);
        let (colour, label) = match h.kind {
            SnapKind::End => (OK, h.kind.label()),
            SnapKind::Mid => (SNAP_MID, h.kind.label()),
            SnapKind::Centre | SnapKind::Crossing | SnapKind::Origin => (WARN, h.kind.label()),
            SnapKind::On => (ERROR, h.kind.label()),
            SnapKind::AxisX => (AXIS_X, h.kind.label()),
            SnapKind::AxisY => (AXIS_Y, h.kind.label()),
            SnapKind::Free => (INK_2, ""),
        };
        if h.kind != SnapKind::Free && !matches!(s.tool.as_str(), "select" | "trim" | "extend") {
            painter.circle_filled(sp, 4.0, colour);
            painter.circle_stroke(sp, 4.0, Stroke::new(1.0_f32, WELL));
            if !label.is_empty() {
                let g = painter.layout_no_wrap(tr(label), meta(), INK);
                let r = Rect::from_min_size(sp + vec2(10.0, -24.0), g.size() + vec2(8.0, 4.0));
                painter.rect_filled(r, R_INSET, POPOVER);
                painter.galley(r.min + vec2(4.0, 2.0), g, INK);
            }
        }
        if let Some(r) = s.pts.last().filter(|_| matches!(h.kind, SnapKind::AxisX | SnapKind::AxisY)) {
            painter.add(egui::Shape::dashed_line(&[s.w2s(*r), sp], Stroke::new(1.0_f32, colour), 4.0, 3.0));
        }
        let _ = cursor;
    }
    for p in &s.pts {
        painter.circle_filled(s.w2s(*p), 3.0, ACCENT);
    }
}

/// A convex-or-not polygon fill by ear clipping (screen points).
fn fill_polygon(poly: &[Pos2], colour: Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    for p in poly {
        mesh.colored_vertex(*p, colour);
    }
    let mut idx: Vec<usize> = (0..poly.len()).collect();
    let area: f32 = (0..poly.len()).map(|i| {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        a.x * b.y - b.x * a.y
    }).sum();
    let ccw = area > 0.0;
    let mut guard = 0;
    while idx.len() > 3 && guard < 10_000 {
        guard += 1;
        let n = idx.len();
        let mut cut = false;
        for i in 0..n {
            let (a, b, c) = (poly[idx[(i + n - 1) % n]], poly[idx[i]], poly[idx[(i + 1) % n]]);
            let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            if (cross > 0.0) != ccw || cross.abs() < 1e-6 {
                continue;
            }
            let inside = idx.iter().any(|&k| {
                let p = poly[k];
                if p == a || p == b || p == c {
                    return false;
                }
                let s1 = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
                let s2 = (c.x - b.x) * (p.y - b.y) - (c.y - b.y) * (p.x - b.x);
                let s3 = (a.x - c.x) * (p.y - c.y) - (a.y - c.y) * (p.x - c.x);
                (s1 > 0.0) == ccw && (s2 > 0.0) == ccw && (s3 > 0.0) == ccw
            });
            if inside {
                continue;
            }
            mesh.add_triangle(idx[(i + n - 1) % n] as u32, idx[i] as u32, idx[(i + 1) % n] as u32);
            idx.remove(i);
            cut = true;
            break;
        }
        if !cut {
            break;
        }
    }
    if idx.len() == 3 {
        mesh.add_triangle(idx[0] as u32, idx[1] as u32, idx[2] as u32);
    }
    egui::Shape::mesh(mesh)
}

/// The live inset: the model's edges near the plane from the 3D view's
/// direction, the plane in amber, the sketch on it as drawn now.
fn inset_paint(painter: &egui::Painter, s: &SketchWs, r: Rect) {
    let i = &s.inset;
    let look = i.look.normalize_or(glam::DVec3::new(1.0, -1.0, 1.0).normalize());
    let up0 = if look.z.abs() > 0.95 { glam::DVec3::Y } else { glam::DVec3::Z };
    let right = up0.cross(look).normalize();
    let up = look.cross(right);
    let proj = |p: glam::DVec3| DVec2::new((p - i.origin).dot(right), (p - i.origin).dot(up));
    let to3 = |q: DVec2| i.origin + i.x * (q.x / 1000.0) + i.y * (q.y / 1000.0);
    // Sketch curves in 3D.
    let mut sketch: Vec<Vec<DVec2>> = Vec::new();
    for c in &s.doc.curves {
        sketch.push(c.shape.polyline(48).into_iter().map(|p| proj(to3(p))).collect());
    }
    let plane_size = s.doc.bounds().map(|(lo, hi)| (hi - lo).max_element().max(100.0) * 0.8).unwrap_or(400.0);
    let quad: Vec<DVec2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|(a, b)| {
            let centre = s.doc.bounds().map(|(lo, hi)| (lo + hi) / 2.0).unwrap_or(DVec2::ZERO);
            proj(to3(centre + DVec2::new(*a, *b) * plane_size))
        })
        .collect();
    let mut lo = DVec2::splat(f64::MAX);
    let mut hi = DVec2::splat(f64::MIN);
    for p in quad.iter().chain(sketch.iter().flatten()) {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    // Context within three plane sizes of the sketch.
    let reach = (hi - lo).max_element() * 1.5;
    let centre = (lo + hi) / 2.0;
    let segs: Vec<(DVec2, DVec2)> = i
        .segments
        .iter()
        .map(|g| (proj(glam::DVec3::new(g[0], g[1], g[2])), proj(glam::DVec3::new(g[3], g[4], g[5]))))
        .filter(|(a, b)| (*a - centre).length() < reach && (*b - centre).length() < reach)
        .collect();
    for (a, b) in &segs {
        lo = lo.min(*a).min(*b);
        hi = hi.max(*a).max(*b);
    }
    let size = (hi - lo).max(DVec2::splat(1e-6));
    let k = (r.width() as f64 / size.x).min(r.height() as f64 / size.y);
    let mid = (lo + hi) / 2.0;
    let to_s = |p: DVec2| pos2(r.center().x + ((p.x - mid.x) * k) as f32, r.center().y - ((p.y - mid.y) * k) as f32);
    for (a, b) in segs {
        painter.line_segment([to_s(a), to_s(b)], Stroke::new(1.0_f32, INK_2.gamma_multiply(0.7)));
    }
    let q: Vec<Pos2> = quad.iter().map(|p| to_s(*p)).collect();
    painter.add(egui::Shape::convex_polygon(q.clone(), SKETCH_PLANE, Stroke::new(1.0_f32, WARN)));
    for line in sketch {
        painter.add(egui::Shape::line(line.into_iter().map(to_s).collect(), Stroke::new(1.5_f32, ACCENT)));
    }
}

/// The settings pane: the sketch's record, its problems, the import list.
fn sketch_record(ui: &mut egui::Ui, s: &mut SketchWs, out: &mut Vec<String>) {
    let w = ui.available_width();
    let title = if s.doc.name.is_empty() { tr(if s.doc.usage == Use::Section { "New section" } else { "New profile" }) } else { s.doc.name.clone() };
    ui.label(egui::RichText::new(title).font(title_font()).color(INK));
    ui.add_space(SPACE_S);
    let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            ui.set_min_height(CONTROL_H);
            ui.add_sized(vec2(84.0, CONTROL_H), egui::Label::new(egui::RichText::new(tr(label)).font(body()).color(DIM)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| add(ui));
        });
    };
    // Use.
    let mut usage = None;
    row(ui, "Use", &mut |ui| {
        let at = usize::from(s.doc.usage == Use::Section);
        if let Some(i) = kit::segmented(ui, &[&tr("Profile"), &tr("Section")], at, 150.0) {
            usage = Some(if i == 1 { Use::Section } else { Use::Profile });
        }
    });
    if let Some(u) = usage.filter(|u| *u != s.doc.usage) {
        s.step();
        s.doc.usage = u;
        if u == Use::Section && s.doc.extrude.is_none() {
            s.doc.extrude = Some(Extrude::default());
        }
    }
    // Name.
    let mut name_commit = None;
    row(ui, "Name", &mut |ui| {
        let buf = s.name_buf.get_or_insert_with(|| s.doc.name.clone());
        let resp = ui.add(kit::field(buf).desired_width(w - 100.0).font(body()));
        if resp.lost_focus() || (resp.changed() && false) {
            name_commit = Some(buf.clone());
        }
    });
    if let Some(n) = name_commit {
        if n != s.doc.name {
            s.step();
            s.doc.name = n.trim().to_owned();
        }
        s.name_buf = None;
    }
    if !s.plane_label.is_empty() {
        let label = s.plane_label.clone();
        row(ui, "On", &mut |ui| {
            ui.label(egui::RichText::new(&label).font(body()).color(INK));
        });
    }
    if s.doc.usage == Use::Section {
        let ex = s.doc.extrude.clone().unwrap_or_default();
        let extents = ["through", "depth", "both", "to_face"];
        let names = [tr("Through all"), tr("To depth"), tr("Both sides"), tr("Up to face")];
        let at = extents.iter().position(|e| *e == ex.extent).unwrap_or(0);
        let mut next = None;
        row(ui, "Extrude", &mut |ui| {
            if kit::dropdown(ui, "", &names[at], false, w - 100.0).clicked() {
                next = Some((at + 1) % extents.len());
            }
        });
        if let Some(i) = next {
            s.step();
            s.doc.extrude.get_or_insert_with(Extrude::default).extent = extents[i].into();
        }
        if matches!(ex.extent.as_str(), "depth" | "both") {
            let mut commit = None;
            row(ui, "Depth", &mut |ui| {
                ui.label(egui::RichText::new("mm").font(meta()).color(FAINT));
                let buf = s.depth_buf.get_or_insert_with(|| fmt_mm(ex.depth));
                let resp = ui.add(kit::field(buf).desired_width(w - 130.0).font(body()));
                if resp.lost_focus() {
                    commit = buf.trim().parse::<f64>().ok();
                }
            });
            if let Some(d) = commit {
                s.step();
                s.doc.extrude.get_or_insert_with(Extrude::default).depth = d.max(0.0);
                s.depth_buf = None;
            }
        }
        let op = ex.operation.clone();
        row(ui, "Operation", &mut |ui| {
            ui.label(egui::RichText::new(tr(&capitalise(&op))).font(body()).color(INK));
        });
        if !s.users.is_empty() {
            let users = s.users.clone();
            row(ui, "Cuts", &mut |ui| {
                ui.label(egui::RichText::new(&users).font(body()).color(INK));
            });
        }
    } else {
        let p = s.profile().clone();
        let area_cm2 = p.area / 100.0;
        let kg_m = p.area * 1e-6 * s.density;
        let area = if p.ok() { format!("{} cm² · {} kg/m", trim_num(area_cm2, 2), trim_num(kg_m, 2)) } else { tr("not closed") };
        row(ui, "Area", &mut |ui| {
            ui.label(egui::RichText::new(&area).font(body()).color(if p.ok() { INK } else { WARN }));
        });
    }
    ui.add_space(SPACE_S);
    if s.doc.usage == Use::Profile {
        kit::caption(ui, &tr("Attachment points"));
        let default = s.doc.attach.replace('_', " ");
        row(ui, "Default", &mut |ui| {
            ui.label(egui::RichText::new(capitalise(&default)).font(body()).color(INK));
        });
        let extras: Vec<String> = s.doc.points.iter().map(|p| p.name.replace('_', " ")).collect();
        let mut arm = false;
        row(ui, "Extra", &mut |ui| {
            if kit::row_link(ui, &tr("+ Add point")).clicked() {
                arm = true;
            }
            if !extras.is_empty() {
                ui.label(egui::RichText::new(extras.join(" · ")).font(body()).color(INK));
            }
        });
        if arm {
            s.arm("attach");
        }
        ui.add_space(SPACE_S);
    }
    let sep = |ui: &mut egui::Ui| {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
        ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, EDGE));
    };
    sep(ui);
    row(ui, "Units", &mut |ui| {
        ui.label(egui::RichText::new("mm · 1:1").font(body()).color(INK));
    });
    let mut view = None;
    row(ui, "View", &mut |ui| {
        let at = match s.view {
            View::Isolated => 0,
            View::Ghosted => 1,
            View::Sliced => 2,
        };
        if let Some(i) = kit::segmented(ui, &[&tr("Isolated"), &tr("Ghosted"), &tr("Sliced")], at, 180.0) {
            view = Some([View::Isolated, View::Ghosted, View::Sliced][i]);
        }
    });
    if let Some(v) = view {
        s.view = v;
    }
    ui.add_space(SPACE_S);
    sep(ui);
    let saves_to = tr(if s.doc.usage == Use::Section { "Sections" } else { "Profiles" });
    row(ui, "Saves to", &mut |ui| {
        ui.label(egui::RichText::new(format!("{} › {}", tr("Library"), saves_to)).font(body()).color(INK));
    });
    if s.users.is_empty() || s.doc.usage == Use::Profile {
        let users = if s.users.is_empty() { tr("nothing yet") } else { s.users.clone() };
        row(ui, "Used by", &mut |ui| {
            ui.label(egui::RichText::new(&users).font(body()).color(INK));
        });
    }
    ui.add_space(SPACE_XS);
    if kit::panel_button(ui, &tr("Save to Library…"), "", 0, None).clicked() {
        let name = if s.doc.name.is_empty() { tr(if s.doc.usage == Use::Section { "Section" } else { "Profile" }) } else { s.doc.name.clone() };
        let file = built_draft2d::file::ProfileFile::drawn(&name, s.doc.clone());
        out.push(format!("sketch:save{SEP_S}{}", file.to_string()));
    }
    // Problems: a walkable list.
    let problems = s.profile().problems.clone();
    if !problems.is_empty() && !(problems.len() == 1 && problems[0] == Problem::Empty) {
        ui.add_space(SPACE_M);
        kit::caption(ui, &format!("{} · {}", tr("Not a closed profile yet"), problems.len()));
        for (i, p) in problems.iter().enumerate() {
            let (text, at) = match p {
                Problem::OpenEnd { at, .. } => (tr("Open end"), Some(*at)),
                Problem::Branch { at, curves } => (format!("{} · {}", tr("Branch"), curves.len()), Some(*at)),
                Problem::Crossing { at, .. } => (tr("Crossing"), Some(*at)),
                Problem::Empty => (tr("Nothing drawn"), None),
            };
            let label = match at {
                Some(a) => format!("{text}  {}; {}", fmt_mm(a.x), fmt_mm(a.y)),
                None => text,
            };
            let selected = i == s.problem_at;
            let (r, resp) = kit::list_row(ui, ROW_H, selected);
            ui.painter().circle_filled(pos2(r.min.x + 10.0, r.center().y), 3.5, if matches!(p, Problem::OpenEnd { .. }) { ERROR } else { WARN });
            ui.painter().text(pos2(r.min.x + 22.0, r.center().y), egui::Align2::LEFT_CENTER, label, body(), INK);
            if resp.clicked() {
                s.problem_at = i;
                if let Some(a) = at {
                    let c = s.canvas.map_or(Pos2::ZERO, |r| r.center());
                    s.org = pos2(c.x - a.x as f32 * s.zoom, c.y + a.y as f32 * s.zoom);
                }
            }
        }
    }
    // Import clean-up.
    if s.cleaning.is_some() {
        ui.add_space(SPACE_M);
        sep(ui);
        import_place(ui, s, w);
    }
}

fn capitalise(t: &str) -> String {
    let mut c = t.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn import_place(ui: &mut egui::Ui, s: &mut SketchWs, w: f32) {
    let (file, layers, open, tol_buf, issues, at, skipped) = {
        let c = s.cleaning.as_ref().unwrap();
        (c.file.clone(), c.imported.layers.clone(), c.layers_open, c.tol_buf.clone(), c.imported.issues.clone(), c.at, c.imported.skipped.clone())
    };
    let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            ui.set_min_height(CONTROL_H);
            ui.add_sized(vec2(84.0, CONTROL_H), egui::Label::new(egui::RichText::new(tr(label)).font(body()).color(DIM)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| add(ui));
        });
    };
    row(ui, "From", &mut |ui| {
        ui.label(egui::RichText::new(&file).font(body()).color(INK));
    });
    let kept = layers.iter().filter(|l| l.2).count();
    let summary = match layers.iter().find(|l| l.2) {
        Some(first) if kept == 1 && layers.len() > 1 => format!("{} ✓ · {} {}", first.0, layers.len() - 1, tr("more")),
        _ => format!("{kept} {} {}", tr("of"), layers.len()),
    };
    let mut toggle_open = false;
    row(ui, "Layers", &mut |ui| {
        if kit::dropdown(ui, "", &summary, open, w - 100.0).clicked() {
            toggle_open = true;
        }
    });
    if toggle_open {
        s.cleaning.as_mut().unwrap().layers_open = !open;
    }
    if open {
        let mut flip = None;
        for (name, n, on) in &layers {
            let mut v = *on;
            if kit::toggle(ui, &mut v, &format!("{name} · {n}")).changed() {
                flip = Some(name.clone());
            }
        }
        if let Some(name) = flip {
            s.relayer(&name);
        }
    }
    if !skipped.is_empty() {
        let text = skipped.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(" · ");
        row(ui, "Not read", &mut |ui| {
            ui.label(egui::RichText::new(&text).font(meta()).color(FAINT));
        });
    }
    ui.add_space(SPACE_S);
    kit::caption(ui, &format!("{} · {}", tr("Clean up"), issues.len()));
    let mut tol_commit = None;
    let mut buf = tol_buf;
    row(ui, "Tolerance", &mut |ui| {
        ui.label(egui::RichText::new("mm").font(meta()).color(FAINT));
        let resp = ui.add(kit::field(&mut buf).desired_width(w - 130.0).font(body()));
        if resp.lost_focus() || resp.changed() {
            tol_commit = buf.trim().parse::<f64>().ok().filter(|t| *t > 0.0);
        }
    });
    s.cleaning.as_mut().unwrap().tol_buf = buf;
    if let Some(t) = tol_commit {
        s.cleaning.as_mut().unwrap().tol = t;
        s.refresh_cleaning();
    }
    let mut fix = None;
    for (i, issue) in issues.iter().enumerate() {
        let (glyph, text, action) = match issue {
            Issue::Segmented { runs } => ("◠", format!("{} {} → {} {}", issue.count(), tr("short segments"), runs.len(), tr("arcs")), tr("Merge arcs")),
            Issue::SmallFillets { arcs, under } => ("⌜", format!("{} {} {} mm", arcs.len(), tr("fillets under"), fmt_mm(*under)), tr("Drop")),
            Issue::Gaps { pairs } => {
                let worst = pairs.iter().map(|p| p.2).fold(0.0, f64::max);
                ("⟋", format!("{} {} · {} mm", pairs.len(), tr(if pairs.len() == 1 { "gap" } else { "gaps" }), fmt_mm(worst)), tr("Close"))
            }
            Issue::Duplicates { curves } => ("≡", format!("{} {}", curves.len(), tr("duplicates")), tr("Remove")),
            Issue::Units { factor, from, size } => {
                let note = format!("{} {}? {} × {} {}", tr("Units:"), from, fmt_mm(size.x), fmt_mm(size.y), from);
                ("⚠", note, if (*factor - 25.4).abs() < 1e-9 { tr("Use mm") } else { format!("× {}", trim_num(*factor, 4)) })
            }
        };
        let selected = i == at;
        ui.horizontal(|ui| {
            ui.set_min_height(CONTROL_H);
            let _ = glyph;
            let (r, _) = ui.allocate_exact_size(vec2(12.0, CONTROL_H), Sense::hover());
            ui.painter().circle_filled(r.center(), 3.5, WARN);
            let resp = ui.add(egui::Label::new(egui::RichText::new(&text).font(body()).color(if selected { INK } else { INK_2 })).truncate().sense(Sense::click()));
            if resp.clicked() {
                s.cleaning.as_mut().unwrap().at = i;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::row_link(ui, &action).clicked() {
                    fix = Some(i);
                }
            });
        });
    }
    if let Some(i) = fix {
        if let Some(issue) = issues.get(i) {
            s.step();
            cleanup::fix(&mut s.doc, issue);
            if matches!(issue, Issue::Units { .. }) {
                s.fit = true;
                let f = if let Issue::Units { factor, .. } = issue { *factor } else { 1.0 };
                s.say(format!("{} × {}", tr("Units set: everything scaled"), trim_num(f, 4)), Some("undo"));
            }
            s.changed();
            s.refresh_cleaning();
        }
    }
    ui.add_space(SPACE_XS);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
        let any = !issues.is_empty();
        if kit::panel_button(ui, &tr("Fix all"), "", 0, None).clicked() && any {
            let tol = s.cleaning.as_ref().unwrap().tol;
            let declared = s.cleaning.as_ref().unwrap().imported.unit.clone().or(if s.cleaning.as_ref().unwrap().drawing.to_mm.is_some() { Some("mm".into()) } else { None });
            s.step();
            let n = cleanup::fix_all(&mut s.doc, tol, declared.as_deref());
            s.changed();
            s.refresh_cleaning();
            s.fit = true;
            s.say(format!("{} · {n} {}", tr("Cleaned"), tr("curves changed")), Some("undo"));
        }
        ui.label(egui::RichText::new(tr("one undo step")).font(meta()).color(FAINT));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> SketchWs {
        let mut s = SketchWs::default();
        s.enter(&serde_json::json!({"use": "profile"})).unwrap();
        s.zoom = 2.0;
        s.org = pos2(400.0, 400.0);
        s
    }
    fn free(p: DVec2) -> Snap {
        Snap { at: p, kind: SnapKind::Free, curve: None }
    }
    fn type_in(s: &mut SketchWs, text: &str, cursor: DVec2) {
        s.typed = text.into();
        s.typing = true;
        s.apply_typed(Some(cursor));
    }

    #[test]
    fn a_typed_rectangle_and_chained_lines_close_into_a_profile() {
        let mut s = ws();
        s.arm("rectangle");
        s.click(DVec2::ZERO, free(DVec2::ZERO), false, false, false);
        type_in(&mut s, "60;20", DVec2::new(10.0, 10.0));
        assert!(s.profile().ok());
        assert!((s.profile().area - 1200.0).abs() < 1e-9);
        // Lines: a length typed along the cursor, dx;dy relative.
        let mut t = ws();
        t.arm("line");
        t.click(DVec2::ZERO, free(DVec2::ZERO), false, false, false);
        type_in(&mut t, "100", DVec2::new(5.0, 0.0));
        type_in(&mut t, "0;50", DVec2::ZERO);
        type_in(&mut t, "-100;0", DVec2::ZERO);
        t.click(DVec2::ZERO, free(DVec2::ZERO), false, false, false);
        assert_eq!(t.doc.curves.len(), 4);
        assert!(t.profile().ok() && (t.profile().area - 5000.0).abs() < 1e-9);
        assert!(t.pts.is_empty(), "closing on the first point ends the chain");
    }

    #[test]
    fn local_history_steps_back_and_forth_and_cancel_keeps_the_entry_state() {
        let mut s = ws();
        s.arm("circle");
        s.click(DVec2::ZERO, free(DVec2::ZERO), false, false, false);
        type_in(&mut s, "25", DVec2::X);
        assert_eq!(s.doc.curves.len(), 1);
        s.local_undo(false);
        assert!(s.doc.curves.is_empty());
        s.local_undo(true);
        assert_eq!(s.doc.curves.len(), 1);
        assert!(s.dirty());
        assert!(s.base.curves.is_empty());
    }

    #[test]
    fn escape_drops_the_point_then_the_tool_then_asks_to_leave() {
        let mut s = ws();
        s.arm("line");
        s.click(DVec2::ZERO, free(DVec2::ZERO), false, false, false);
        assert!(!s.escape() && s.pts.is_empty() && s.tool == "line");
        assert!(!s.escape() && s.tool == "select");
        assert!(s.escape() && s.leave_prompt);
        assert!(!s.escape() && !s.leave_prompt, "a second Esc stays");
    }

    #[test]
    fn a_dimension_typed_drives_the_line() {
        let mut s = ws();
        s.arm("rectangle");
        s.click(DVec2::ZERO, free(DVec2::ZERO), false, false, false);
        type_in(&mut s, "60;20", DVec2::ONE);
        s.arm("dimension");
        let bottom = DVec2::new(30.0, 0.0);
        s.click(bottom, free(bottom), false, false, false);
        assert_eq!(s.doc.dims.len(), 1);
        assert!(s.editing_dim.is_some() && s.typing);
        type_in(&mut s, "80", DVec2::ONE);
        let (lo, hi) = s.doc.bounds().unwrap();
        assert!((hi.x - lo.x - 80.0).abs() < 1e-9, "{lo} {hi}");
    }

    #[test]
    fn one_click_fixes_report_what_they_did() {
        let mut s = ws();
        for l in cleanup::segmented_arc(DVec2::ZERO, 10.0, 0.0, std::f64::consts::FRAC_PI_2, 6) {
            s.doc.add(l);
        }
        s.arm("merge_arcs");
        assert_eq!(s.doc.curves.len(), 1);
        assert!(matches!(s.doc.curves[0].shape, Shape::Arc { .. }));
        assert_eq!(s.tool, "select");
        assert!(s.toast.as_ref().is_some_and(|t| t.0.contains('6')));
    }
}
