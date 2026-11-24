//! The modelling tools as state machines (R2.5, `docs/rust-tools.md`).
//!
//! A tool receives [`ToolEvent`]s, keeps its state as an explicit enum,
//! answers a [`Preview`] for the renderer and commits every edit as a
//! protocol command through [`ToolHost::run`]. Nothing here knows Python:
//! `py.rs` is the only PyO3 face, and `tests.rs` drives the machines
//! against a fake host.

mod change_axes;
mod freehand;
mod followme;
mod move_grips;
pub mod move_hover;
mod place_group;
mod eraser;
mod geopath;
mod annotations;
mod section_tool;
mod north;
mod image;
mod scale_manager;
mod solid;
mod paste;
mod flip;
mod tape;
mod fillet_tool;
mod protractor;
mod paint;
mod offset_tool;
mod walkthrough;
mod scale_grip;
mod scale_tool;
mod rotate;
pub mod gesture;
pub mod line;
pub mod move_tool;
pub mod push_pull;
pub mod plane;
pub mod plane_input;
pub mod host_geometry;
pub mod box_records;
pub mod chunk_selection;
pub mod record_queries;
mod arc_queries;
pub(crate) mod selection_growth;
pub mod bevel_queries;
pub(crate) mod loose_preview;
mod material_queries;
pub mod disc_host;
pub mod transform_pose;
pub mod preview_pack;
pub mod texture_rows;
pub mod push_preview;
pub mod push_references;
pub mod push_targets;
pub mod push_stage;
pub mod packed_selection;
mod protocol_host;
pub mod handles;
pub mod edge_runs;
pub mod view_queries;
pub mod transform_queries;
pub mod preview;
pub mod rectangle;
pub mod arc;
pub mod circle;
pub mod rotated_rectangle;
pub mod py;
pub mod select;
#[cfg(test)]
mod tests;

pub use preview::Preview;

pub type P3 = [f64; 3];

pub(crate) fn sub(a: P3, b: P3) -> P3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
pub(crate) fn add(a: P3, b: P3) -> P3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
pub(crate) fn scale(a: P3, k: f64) -> P3 { [a[0] * k, a[1] * k, a[2] * k] }
pub(crate) fn dot(a: P3, b: P3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
pub(crate) fn len(a: P3) -> f64 { dot(a, a).sqrt() }
pub(crate) fn unit(a: P3) -> P3 {
    let l = len(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

/// Keyboard modifiers as the hosts deliver them. `ctrl` is the primary
/// modifier: Qt's ControlModifier, which is ⌘ on the Mac (`keys.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Mods {
    /// From Qt's KeyboardModifiers bits.
    pub fn from_qt(bits: u32) -> Self {
        Mods {
            shift: bits & 0x0200_0000 != 0,
            ctrl: bits & 0x0400_0000 != 0,
            alt: bits & 0x0800_0000 != 0,
            meta: bits & 0x1000_0000 != 0,
        }
    }
}

/// What the cursor names, as the inference answered it, in tool
/// coordinates (the open store's frame).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snap {
    pub point: P3,
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pointer {
    /// Logical pixels in the view.
    pub px: [f64; 2],
    pub mods: Mods,
    /// The snapped and locked point; `None` where the ray met nothing.
    pub world: Option<P3>,
    pub snap: Option<Snap>,
    /// Milliseconds, any monotonic origin (double and triple clicks).
    pub t_ms: f64,
}

/// A typed value the value field committed with Enter.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Length(f64),
    Pair(f64, f64),
    Triple(f64, f64, f64),
    List(Vec<f64>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolEvent {
    Activate,
    Deactivate,
    Cancel,
    /// ⌘Z while the tool is active: the tool may take it (mid-chain undo).
    Undo,
    Hover(Pointer),
    Press(Pointer),
    Drag(Pointer),
    Release(Pointer),
    Click(Pointer),
    DoubleClick(Pointer),
    TripleClick(Pointer),
    Box { rect: [f64; 4], crossing: bool, mods: Mods },
    Key { key: String, mods: Mods },
    /// An array typed after a copy: `count` copies, `mode` `"x"`
    /// (multiples) or `"/"` (dividing), `step` a spacing (`5x10m`).
    Array { count: i64, mode: String, step: Option<f64> },
    /// A key let go; `tapped`: no other key came in between (a Ctrl that
    /// was part of a chord is no tap, #342).
    KeyRelease { key: String, tapped: bool },
    /// The drawing axes `(origin, x, y, z)` and whether the cursor's snap
    /// names a point of the model (None: no kind known), before an event.
    Frame { axes: [P3; 4], names_point: Option<bool> },
    Value(Value),
    AbsoluteValue(Value),
    ScaleFactors(P3),
    Navigate(walkthrough::Navigation),
    ProtractorFrame { basis: [P3; 2], near: bool, exact: bool },
    /// A segment or side count typed as `Ns` (Circle, Polygon, Arc).
    Segments(i64),
    /// A radius typed as `2r` (the 2-point Arc).
    Radius(f64),
    Profile(f64),
    PasteAt(P3),
    FlipAlong { axis: String, copy: bool },
    /// What only the view knows, before a pointer or value event.
    View(ViewReading),
    /// The plane the operation keeps to (a face clicked first, a plane lock).
    WorkPlane(Option<(P3, P3)>),
}

/// What the view reads for a tool that needs more than the snapped point
/// (the Rotated Rectangle's protractors).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewReading {
    /// The ray under the cursor: origin and unit direction.
    pub ray: Option<(P3, P3)>,
    /// The axis the view's axis lock holds, a unit vector.
    pub axis_lock: Option<P3>,
    /// A protractor at a fixed screen size: its world radius (None when
    /// the view could not measure it) and whether the cursor is near its rim.
    pub protractor: Option<(Option<f64>, bool)>,
    /// The normal of the plane the view draws on.
    pub plane_normal: Option<P3>,
}

/// What a protocol command answered, reduced to what tools read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    pub ok: bool,
    /// History's revision after the command (retype windows compare it).
    pub revision: u64,
    /// Net change in the open mesh's face count.
    pub face_delta: i64,
    pub error: Option<String>,
}

/// A protocol command by name (`core-protocol.md` §1): one History step.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub name: &'static str,
    pub args: serde_json::Value,
}

/// How a pick joins the selection: Shift toggles, Ctrl adds, Shift+Ctrl
/// removes, nothing replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode { Replace, Add, Toggle, Remove }

impl SelectMode {
    pub fn from_mods(m: Mods) -> Self {
        match (m.shift, m.ctrl) {
            (true, true) => SelectMode::Remove,
            (false, true) => SelectMode::Add,
            (true, false) => SelectMode::Toggle,
            _ => SelectMode::Replace,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            SelectMode::Replace => "replace",
            SelectMode::Add => "add",
            SelectMode::Toggle => "toggle",
            SelectMode::Remove => "remove",
        }
    }
}

/// How a pick grows before it is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grow {
    /// A face's smooth surface, an edge's whole curve.
    Natural,
    /// Double click: a face and its edges, an edge and its faces.
    Bounded,
    /// Triple click: everything connected.
    Connected,
}

impl Grow {
    pub fn name(self) -> &'static str {
        match self { Grow::Natural => "natural", Grow::Bounded => "bounded", Grow::Connected => "connected" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind { Face, Edge, Group, SectionPlane, TextLabel, Dimension, Other }

impl PickKind {
    pub fn parse(s: &str) -> Self {
        match s {
            "face" => PickKind::Face,
            "edge" => PickKind::Edge,
            "group" => PickKind::Group,
            "section_plane" => PickKind::SectionPlane,
            "text_label" => PickKind::TextLabel,
            "dimension" => PickKind::Dimension,
            _ => PickKind::Other,
        }
    }
}

/// Something under the cursor. `id` is the host's handle for it: until
/// picking is native it names a host object, later a key or uid path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick { pub id: u64, pub kind: PickKind }

/// What the tools ask of the document and the view. Commits go through
/// [`ToolHost::run`] only; the other calls read the view or change view
/// state (selection, hover, edit context), which is not a History step.
pub trait ToolHost {
    fn run(&mut self, cmd: &Command) -> Answer;
    /// History's revision now.
    fn revision(&mut self) -> u64 { 0 }
    /// The camera position (which way a new vertical face looks).
    fn eye(&mut self) -> Option<P3> { None }

    // Picking. The order of what a click takes is the machine's
    // (`select::pick`); the host answers one kind of thing at a pixel,
    // nearest first with its distance in pixels: `label_block`,
    // `linework_edge`, `linework_groups`, `group`, `visible_edge`,
    // `dimension`, `text_label`, `geopath`, `section_plane`, `guide`,
    // `face`, `image_plane`, `groups_around`.
    fn query(&mut self, _what: &str, _px: [f64; 2]) -> Vec<(Pick, f64)> { vec![] }
    /// Whether the view answers a query kind at all (`linework`).
    fn has(&mut self, _what: &str) -> bool { false }
    /// An edge's point under the cursor is not hidden behind a face.
    fn edge_clear(&mut self, _edge: u64, _px: [f64; 2]) -> bool { false }
    /// A picture lying in front of this face at the pixel.
    fn image_in_front(&mut self, _face: u64, _px: [f64; 2]) -> Option<Pick> { None }
    fn extension_at(&mut self, _px: [f64; 2]) -> Option<u64> { None }
    /// "2 of 3: <what> — click again for the next" (the host translates).
    fn cycle_status(&mut self, _index: usize, _count: usize, _id: u64) {}

    // Selection and hover: view state.
    fn select(&mut self, _ids: &[u64], _mode: SelectMode, _grow: Grow) {}
    fn clear_selection(&mut self) {}
    fn selection_signature(&mut self) -> u64 { 0 }
    fn selection_empty(&mut self) -> bool { true }
    fn box_select(&mut self, _rect: [f64; 4], _crossing: bool, _mode: SelectMode) {}
    fn set_hover(&mut self, _id: Option<u64>) {}
    fn set_extension_pick(&mut self, _id: Option<u64>) {}
    fn has_extension_pick(&mut self) -> bool { false }
    fn erase(&mut self, _selection_only: bool) -> bool { false }

    // Edit context.
    fn editing(&mut self) -> bool { false }
    fn edit_box_contains(&mut self, _px: [f64; 2]) -> bool { false }
    fn open_placement(&mut self, _id: u64) {}
    fn close_placement(&mut self) {}
    /// A verb the host still owns for an annotation or a section plane
    /// (`toggle_section`, `edit_text`, `edit_dimension`).
    fn host_verb(&mut self, _verb: &str, _id: u64) {}
    /// A verb the host still owns for a tool (Move's `grab`, `preview`,
    /// `revert`, `move`, `copy`): JSON in, JSON out.
    fn call(&mut self, _verb: &str, _args: serde_json::Value) -> serde_json::Value { serde_json::Value::Null }
    /// A status message: an English template with `{name}` fields, which
    /// the host translates and shows.
    fn message(&mut self, _template: &str, _args: &[(&str, String)]) {}
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub consumed: bool,
    pub redraw: bool,
    /// Something was committed (the host releases a sticky axis lock).
    pub committed: bool,
}

impl Reply {
    pub fn redraw() -> Self { Reply { consumed: true, redraw: true, committed: false } }
    pub fn ignored() -> Self { Reply::default() }
    pub fn committed() -> Self { Reply { consumed: true, redraw: true, committed: true } }
}

/// What the inference and the host read of a tool: class attributes today.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolInfo {
    pub kind: &'static str,
    pub name: &'static str,
    pub shortcut: &'static str,
    pub description: &'static str,
    pub uses_snap: bool,
    pub box_select: bool,
    pub vcb_label: Option<&'static str>,
    pub magnetic_axis_deg: Option<f64>,
    pub screen_axis_px: Option<f64>,
    pub axis_source: bool,
    pub axis_source_empty_only: bool,
}

impl ToolInfo {
    const fn basic(kind: &'static str, name: &'static str, shortcut: &'static str,
                   description: &'static str, uses_snap: bool, vcb_label: Option<&'static str>) -> Self {
        Self {kind, name, shortcut, description, uses_snap, vcb_label, box_select: false,
              magnetic_axis_deg: None, screen_axis_px: None, axis_source: false, axis_source_empty_only: false}
    }
}

pub trait Tool {
    fn info(&self) -> &'static ToolInfo;
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply;
    /// Surface rings in renderer order, outer first then openings.
    fn preview_surfaces(&self) -> std::borrow::Cow<'_, [Vec<Vec<P3>>]> {
        std::borrow::Cow::Borrowed(&[])
    }
    fn cursor_preview(&self, _plane: &preview::CursorPlane, _hover: Option<P3>) -> Preview {
        self.preview()
    }
    fn preview(&self) -> Preview { Preview::default() }
    /// An operation is under way: Esc cancels it before anything else.
    fn busy(&self) -> bool { false }
    /// What the inference and the host read: named points and planes.
    fn state(&self) -> serde_json::Value { serde_json::Value::Null }
    /// Set one of the `state()` fields (scripts and tests that place a
    /// chain by hand, the host's captured plane). False when unknown.
    fn set(&mut self, _key: &str, _value: &serde_json::Value) -> bool { false }
    /// A read-only question about the tool's geometry (`span`, `corners`…),
    /// for the view and tests; Null when unknown.
    fn query(&self, _what: &str, _arg: &serde_json::Value) -> serde_json::Value { serde_json::Value::Null }
    /// A planar tool's plane lock (one owner: the machine).
    fn plane_lock(&mut self) -> Option<&mut plane::PlaneLock> { None }
}

pub(crate) fn json_p3(v: &serde_json::Value) -> Option<P3> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}

pub fn new_tool(kind: &str) -> Option<Box<dyn Tool>> {
    match kind {
        "freehand" => Some(Box::new(freehand::Freehand::default())),
        "outer_shell" | "solid_union" | "solid_subtract" | "solid_trim" | "solid_intersect" | "solid_split" => Some(Box::new(solid::Solid::new(kind)?)),
        "scale_manager" => Some(Box::new(scale_manager::ScaleManager::default())),
        "image" => Some(Box::new(image::ImageTool::default())),
        "north" => Some(Box::new(north::North::default())),
        "section" => Some(Box::new(section_tool::Section::default())),
        "dimension" => Some(Box::new(annotations::Dimension::default())),
        "text" => Some(Box::new(annotations::Text::default())),
        "geopath" => Some(Box::new(geopath::GeoPath::default())),
        "eraser" => Some(Box::new(eraser::Eraser::default())),
        "place_group" => Some(Box::new(place_group::PlaceGroup::default())),
        "followme" => Some(Box::new(followme::FollowMe::default())),
        "change_axes" => Some(Box::new(change_axes::ChangeAxes::default())),
        "paste" => Some(Box::new(paste::Paste::default())),
        "flip" => Some(Box::new(flip::Flip::default())),
        "tape" => Some(Box::new(tape::Tape::default())),
        "fillet" => Some(Box::new(fillet_tool::Fillet::default())),
        "protractor" => Some(Box::new(protractor::Protractor::default())),
        "paint" => Some(Box::new(paint::Paint)),
        "offset" => Some(Box::new(offset_tool::Offset::default())),
        "position_camera" | "look_around" | "walk" | "first_person" => Some(Box::new(walkthrough::Walkthrough::new(kind))),
        "select" => Some(Box::new(select::SelectTool::default())),
        "line" => Some(Box::new(line::LineTool::default())),
        "rectangle" => Some(Box::new(rectangle::RectangleTool::default())),
        "circle" => Some(Box::new(circle::RadialTool::circle())),
        "polygon" => Some(Box::new(circle::RadialTool::polygon())),
        "rotated_rectangle" => Some(Box::new(rotated_rectangle::RotatedRectangleTool::default())),
        "arc" => Some(Box::new(arc::ArcTool::default())),
        "arc3" => Some(Box::new(arc::ThreePointArcTool::default())),
        "center_arc" => Some(Box::new(arc::CenterArcTool::center())),
        "pie" => Some(Box::new(arc::CenterArcTool::pie())),
        "rotate" => Some(Box::new(rotate::Rotate::default())),
        "scale" => Some(Box::new(scale_tool::ScaleTool::default())),
        "pushpull" => Some(Box::new(push_pull::PushPull::default())),
        "move" => Some(Box::new(move_tool::MoveTool::default())),
        _ => None,
    }
}

pub(crate) fn p3_json(p: P3) -> serde_json::Value { serde_json::json!([p[0], p[1], p[2]]) }
pub(crate) fn opt_p3_json(p: Option<P3>) -> serde_json::Value {
    p.map(p3_json).unwrap_or(serde_json::Value::Null)
}
