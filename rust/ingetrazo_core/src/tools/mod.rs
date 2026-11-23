//! The modelling tools as state machines (R2.5, `docs/rust-tools.md`).
//!
//! A tool receives [`ToolEvent`]s, keeps its state as an explicit enum,
//! answers a [`Preview`] for the renderer and commits every edit as a
//! protocol command through [`ToolHost::run`]. Nothing here knows Python:
//! `py.rs` is the only PyO3 face, and `tests.rs` drives the machines
//! against a fake host.

pub mod gesture;
pub mod line;
pub mod preview;
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
    Value(Value),
    /// The plane the operation keeps to (a face clicked first, a plane lock).
    WorkPlane(Option<(P3, P3)>),
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

    // Picking: host queries until picking is native.
    fn pick(&mut self, _px: [f64; 2]) -> Option<Pick> { None }
    fn candidates(&mut self, px: [f64; 2]) -> Vec<Pick> { self.pick(px).into_iter().collect() }
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

pub trait Tool {
    fn info(&self) -> &'static ToolInfo;
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply;
    fn preview(&self) -> Preview { Preview::default() }
    /// An operation is under way: Esc cancels it before anything else.
    fn busy(&self) -> bool { false }
    /// What the inference and the host read: named points and planes.
    fn state(&self) -> serde_json::Value { serde_json::Value::Null }
}

pub fn new_tool(kind: &str) -> Option<Box<dyn Tool>> {
    match kind {
        "select" => Some(Box::new(select::SelectTool::default())),
        "line" => Some(Box::new(line::LineTool::default())),
        _ => None,
    }
}

pub(crate) fn p3_json(p: P3) -> serde_json::Value { serde_json::json!([p[0], p[1], p[2]]) }
pub(crate) fn opt_p3_json(p: Option<P3>) -> serde_json::Value {
    p.map(p3_json).unwrap_or(serde_json::Value::Null)
}
