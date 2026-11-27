//! The one verb source of the chrome (architecture review 7, council B5):
//! what a selection is, which verbs apply to it, their labels and keys, how
//! the right-click menu lays them out, which ones the inspector offers as
//! chips or under More, the linked-file rows' verbs, and what runs each one.
//!
//! `viewport_menu.rs` reads the document into a few facts about the
//! selection (`Facts`) and runs the dispatch this module names (a window
//! slot, a Project tree verb on the item's row, a core command, or one of a
//! few named glue actions). The viewport menu, the
//! inspector and the tree's file rows all read this table, so they cannot
//! disagree on a verb or its label. docs/context-menu.md has the menu.
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

/// Kinds, in docs/context-menu.md's order.
pub const KINDS: [&str; 15] = ["faces", "edges", "geometry", "group", "component", "groups", "mixed",
    "link", "linked_part", "annotation", "guide", "empty", "section", "image", "path"];
/// Kinds whose model is not this document's to edit.
pub const READ_ONLY: [&str; 2] = ["link", "linked_part"];
/// Verbs that edit the selected model: never offered on a read-only kind.
pub const EDITING: [&str; 24] = ["open", "rename", "explode", "makegroup", "makecomp", "fileto", "unique",
    "split", "merge", "intersect", "soften", "flip", "reverse", "orient", "heal", "rebuild", "simplify",
    "replace", "keep", "axes", "join", "topath", "status", "repair"];
/// The most rows a kind's menu may have under its header (Repeat included).
pub const MAX_ROWS: usize = 14;

/// What Python knows about the selection (and the window) when a menu or
/// the inspector asks. Every field defaults to "no".
#[derive(Deserialize, Serialize, Default, Debug, Clone)]
#[serde(default)]
pub struct Facts {
    // What is selected.
    pub total: usize,
    pub groups: usize,
    pub faces: usize,
    pub edges: usize,
    pub components: usize,
    pub painted_group: bool,
    pub linked_part: bool,
    pub all_links: bool,
    pub all_annotations: bool,
    pub all_guides: bool,
    pub all_sections: bool,
    pub all_images: bool,
    pub all_paths: bool,
    pub all_locked: bool,
    // What applies.
    pub repeat: Option<String>,
    pub repeat_value: String,
    pub repeat_key: Option<String>,
    pub locked_group: Option<String>,
    pub locked_image: Option<String>,
    pub text3d: bool,
    pub edited_letters: bool,
    pub hideable: bool,
    pub taggable: bool,
    pub makes_faces: bool,
    pub textured_face: bool,
    pub bridgeable: bool,
    pub billboard: bool,
    pub replaceable: bool,
    pub kept: bool,
    pub solids: usize,
    pub loose_edges: usize,
    pub weldable: bool,
    pub curve: bool,
    pub include_nested: Option<bool>,
    pub host: bool,
    pub paste: bool,
    pub isolated: bool,
    pub guides: bool,
    pub section_active: bool,
    pub image_locked: bool,
    pub path_polygon: bool,
    pub reveal_key: Option<String>,
    // The header.
    pub name: String,
    pub filed: String,
    pub context: String,
    pub instances: usize,
    pub file: String,
    pub host_file: String,
    pub area: String,
    pub length: String,
    pub dimension: bool,
    pub text: String,
    pub guide_point: bool,
}

/// One entry of a menu row: (id, label, key, enabled, checked: -1 none, 0, 1).
pub type Entry = (String, String, String, bool, i8);
/// A row: its family label ("" for a plain row) and its entries.
pub type Row = (String, Vec<Entry>);

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// The kind of a selection (docs/context-menu.md).
pub fn kind(f: &Facts) -> &'static str {
    if f.total == 0 {
        return "empty";
    }
    if f.groups > 0 {
        if f.linked_part {
            return "linked_part";
        }
        if f.all_links && f.groups == f.total {
            return "link";
        }
    }
    if f.groups > 0 && f.groups == f.total {
        return if f.groups == 1 { if f.components == 1 { "component" } else { "group" } } else { "groups" };
    }
    if f.faces + f.edges == f.total {
        return if f.edges == 0 { "faces" } else if f.faces == 0 { "edges" } else { "geometry" };
    }
    if f.groups > 0 && f.groups + f.faces + f.edges == f.total {
        return "mixed";
    }
    for (all, kind) in [(f.all_annotations, "annotation"), (f.all_guides, "guide"), (f.all_sections, "section"),
                        (f.all_images, "image"), (f.all_paths, "path")] {
        if all {
            return kind;
        }
    }
    "mixed"
}

fn join(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" · ")
}

/// (title, subtitle): what was hit, and where it is filed.
pub fn header(f: &Facts, kind: &str) -> (String, String) {
    let other = f.total.saturating_sub(f.groups + f.faces + f.edges);
    match kind {
        "empty" => (if f.context.is_empty() { "Model".into() } else { f.context.clone() }, String::new()),
        "group" | "component" => {
            let mut what = if kind == "component" { "Component".to_owned() } else { "Group".to_owned() };
            let name = if f.name.is_empty() { what.clone() } else { f.name.clone() };
            if kind == "component" {
                what = format!("{what} · {}", plural(f.instances, "instance"));
            }
            (name, format!("{what} · {}", f.filed))
        }
        "link" if f.total == 1 => {
            let name = if f.file.is_empty() { f.name.clone() } else { f.file.clone() };
            // "↗" marks a linked file, as in the tree (Inter has no ⛓).
            (format!("{name} ↗"), format!("Linked file · read-only · {}", f.filed))
        }
        "link" => (plural(f.total, "linked file"), "Linked files · read-only".into()),
        "linked_part" => {
            let name = if f.name.is_empty() { "Part".to_owned() } else { f.name.clone() };
            (name, format!("in {} ↗ · read-only", f.host_file))
        }
        "faces" if f.faces == 1 => ("Face".into(), join(&[&f.area, "loose geometry", &f.context])),
        "faces" => (plural(f.faces, "face"), format!("loose geometry · {}", f.context)),
        "edges" if f.edges == 1 => ("Edge".into(), join(&[&f.length, "loose geometry", &f.context])),
        "edges" => (plural(f.edges, "edge"), format!("loose geometry · {}", f.context)),
        "groups" => {
            let plain = f.groups.saturating_sub(f.components);
            let parts: Vec<String> = [(plain, "group"), (f.components, "component")]
                .iter().filter(|(n, _)| *n > 0).map(|(n, w)| plural(*n, w)).collect();
            (plural(f.total, "item"), parts.join(", "))
        }
        "geometry" | "mixed" => {
            let parts: Vec<String> = [(f.groups, "group"), (f.faces, "face"), (f.edges, "edge"), (other, "other")]
                .iter().filter(|(n, _)| *n > 0).map(|(n, w)| plural(*n, w)).collect();
            (plural(f.total, "item"), parts.join(", "))
        }
        "annotation" if f.total > 1 => (plural(f.total, "annotation"), f.context.clone()),
        "annotation" if f.dimension => ("Dimension".into(), f.text.clone()),
        "annotation" => ("Text".into(), if f.text.is_empty() { String::new() } else { format!("“{}”", f.text) }),
        "guide" if f.total > 1 => (plural(f.total, "guide"), String::new()),
        "guide" => (if f.guide_point { "Guide point" } else { "Guide line" }.into(), "construction".into()),
        "section" => (if f.name.is_empty() { "Section plane".into() } else { f.name.clone() }, "Section plane".into()),
        "image" => (if f.name.is_empty() { "Image".into() } else { f.name.clone() }, "Image".into()),
        "path" => ("Path".into(), "Geo path".into()),
        _ => (plural(f.total, "item"), String::new()),
    }
}

fn e(id: &str, label: &str) -> Entry {
    (id.into(), label.into(), String::new(), true, -1)
}
fn ek(id: &str, label: &str, key: &str) -> Entry {
    (id.into(), label.into(), key.into(), true, -1)
}
fn checked(mut entry: Entry, on: bool) -> Entry {
    entry.4 = i8::from(on);
    entry
}

struct Rows(Vec<Row>);

impl Rows {
    fn one(&mut self, entries: Vec<Entry>) {
        if !entries.is_empty() {
            self.0.push((String::new(), entries));
        }
    }
    fn family(&mut self, label: &str, entries: Vec<Entry>) {
        if !entries.is_empty() {
            self.0.push((label.into(), entries));
        }
    }
}

const MAKE_GROUP: &str = "Make group";
const MAKE_COMPONENT: &str = "Make component…";
const EXPLODE: &str = "Explode";
const MAKE_UNIQUE: &str = "Make unique";
const MERGE_GROUPS: &str = "Merge groups";

/// Single-action rows for a keyed project-tree target.
pub(crate) fn tree_menu_rows(entries: impl IntoIterator<Item = (String, String, bool)>) -> Vec<Row> {
    let mut rows = Rows(Vec::new());
    for (id, label, enabled) in entries {
        if id == "-" {
            rows.0.push(("-".into(), Vec::new()));
        } else {
            let label = match id.as_str() {
                "group" => MAKE_GROUP,
                "component" => MAKE_COMPONENT,
                "explode" => EXPLODE,
                "unique" => MAKE_UNIQUE,
                "merge" => MERGE_GROUPS,
                _ => &label,
            };
            let mut entry = e(&id, label);
            entry.3 = enabled;
            rows.one(vec![entry]);
        }
    }
    rows.0
}

fn intersect() -> Vec<Entry> {
    vec![e("intersect:model", "Model"), e("intersect:selection", "Selection"), e("intersect:context", "Context")]
}
fn flip() -> Vec<Entry> {
    vec![e("flip:x", "Red"), e("flip:y", "Green"), e("flip:z", "Blue")]
}
fn soften() -> Entry {
    e("soften", "Soften / Smooth edges…")
}
fn hide_if(f: &Facts) -> Vec<Entry> {
    if f.hideable { vec![e("hide", "Hide")] } else { vec![] }
}
fn isolate_if(f: &Facts) -> Vec<Entry> {
    if f.groups > 0 { vec![e("isolate", "Isolate")] } else { vec![] }
}
fn zoom() -> Vec<Entry> {
    vec![e("zoom", "Zoom to")]
}
fn file_to() -> Vec<Entry> {
    vec![ek("fileto", "File to…", "⌘⇧F")]
}
fn make() -> Vec<Entry> {
    vec![e("makegroup", MAKE_GROUP), e("makecomp", MAKE_COMPONENT)]
}
fn lock(f: &Facts) -> Vec<Entry> {
    vec![e("lock", if f.all_locked { "Unlock" } else { "Lock" })]
}

/// Sheet menu entries carry the same inspector edits and page commands as the dock and keys.
pub fn sheet_menu(doc: &crate::sheet_page::Snapshot, keys: &[u64], clip: Option<&str>) -> (String, Vec<Row>) {
    use crate::sheet_inspector::{edit, style_fields, title_of};
    use crate::ui::tr;
    let items: Vec<_> = keys.iter().filter_map(|k| doc.items.iter().find(|i| i.key == *k)).collect();
    let mut rows = Rows(Vec::new());
    let mut one = |id: &str, label: &str| rows.one(vec![e(id, &tr(label))]);
    if items.is_empty() {
        one("sheets:fit", "Fit page");
        one("sheets:tool\x1fvista", "Place view");
        one("sheets:tool\x1ftexto", "Place text");
        one("sheets:add", "Add sheet");
        return (tr("Sheet"), rows.0);
    }
    let title = if let [item] = items.as_slice() {
        let kind = item.kind.as_str();
        if !item.locked {
            if kind == "text" { one("page:edit_text", "Edit text"); }
            // None as the value means toggle the inspector's checkbox.
            let field = match kind {
                "frame" => Some(("refresh", Some(""), "Update view")),
                "text" => Some(("font", Some(""), "Font…")),
                "dimension" => Some(("show_unit", None, "Show unit")),
                "angular" if item.data["fields"]["ends"] == "arrow" => Some(("ends", Some("0"), "Tick ends")),
                "angular" => Some(("ends", Some("1"), "Arrow ends")),
                "radial" => Some(("centre_mark", None, "Centre mark")),
                "label" => Some(("arrow", None, "Leader arrow")),
                "level" => Some(("mirror", None, "Text to the left")),
                "image" => Some(("border", None, "Image border")),
                "shape" if matches!(item.data["fields"]["kind"].as_str(), Some("rect" | "elipse" | "poligono")) =>
                    Some(("fill", None, "Fill shape")),
                "titleblock" if item.data["fields"]["columns"] == 2 => Some(("columns", Some("1"), "One column")),
                "titleblock" => Some(("columns", Some("2"), "Two columns")),
                _ => None,
            };
            if let Some((field, value, label)) = field {
                let check = value.is_none();
                let on = item.data["fields"][field].as_bool().unwrap_or(false);
                let value = value.unwrap_or(if on { "0" } else { "1" });
                if let Some(action) = edit(doc, &format!("{}:{field}", item.key), value) {
                    let mut entry = e(&format!("sheets:{action}"), &tr(label));
                    if check { entry = checked(entry, on); }
                    rows.one(vec![entry]);
                }
            }
        }
        tr(title_of(kind))
    } else {
        format!("{} {}", items.len(), tr("items"))
    };
    let first = items[0];
    let movable = items.iter().any(|i| i.kind != "titleblock");
    let mut one = |id: &str, label: &str| rows.one(vec![e(id, &tr(label))]);
    if !style_fields(&first.kind).is_empty() && items.len() == 1 { one("page:copy_style", "Copy style"); }
    if items.iter().any(|i| !i.locked && Some(i.kind.as_str()) == clip) { one("page:paste_style", "Paste style"); }
    if items.iter().any(|i| i.data["fields"]["group_id"].as_str().is_some_and(|g| !g.is_empty())) {
        one("page:ungroup", "Ungroup");
    } else if items.len() >= 2 && items.iter().all(|i| i.kind != "titleblock" && !i.locked) {
        one("page:group", "Group");
    }
    let all_locked = items.iter().all(|i| i.locked);
    one("sheets:lock_items", if all_locked { "Unlock" } else { "Lock" });
    if movable { one("sheets:duplicate_items", "Duplicate"); }
    if items.iter().any(|i| !i.locked && i.kind != "titleblock") { one("sheets:delete_items", "Delete"); }
    (title, rows.0)
}

/// The right-click menu's rows for a selection of `kind`.
pub fn menu_rows(f: &Facts, kind: &str) -> Vec<Row> {
    let read_only = READ_ONLY.contains(&kind);
    let mut r = Rows(Vec::new());
    // Repeat, with the value the last command runs with.
    let key = crate::shortcut_text::shortcut_text(f.repeat_key.as_deref().unwrap_or("Shift+R"))
        .map(|sequence| sequence.1).unwrap_or_default();
    let repeat = match &f.repeat {
        Some(label) => {
            let text = if f.repeat_value.is_empty() { format!("Repeat {label}") } else { format!("Repeat {label}  {}", f.repeat_value) };
            (String::from("repeat"), text, key, !read_only, -1)
        }
        None => ("repeat".into(), "Repeat".into(), key, false, -1),
    };
    r.one(vec![repeat]);
    // A locked thing under the pointer the click could not select: its
    // Unlock is reachable only from here (#782).
    if let Some(name) = &f.locked_group {
        r.one(vec![e("unlockgroup", &format!("Unlock “{name}”"))]);
    }
    if let Some(name) = &f.locked_image {
        r.family("Image", vec![e("unlockimage", &format!("Unlock “{name}”")), e("deleteimage", "Delete")]);
    }
    if matches!(kind, "group" | "component") && f.text3d {
        let label = if f.edited_letters { "Edit 3D Text… (letters edited by hand)" } else { "Edit 3D Text…" };
        r.one(vec![("edittext3d".into(), label.into(), String::new(), !f.edited_letters, -1)]);
    }
    match kind {
        "faces" | "edges" | "geometry" => {
            let faces = kind != "edges";
            let edges = kind != "faces";
            r.one(hide_if(f));
            r.one(zoom());
            r.one(make());
            if f.bridgeable { r.one(vec![e("bridge", "Bridge")]); }
            if f.weldable { r.one(vec![e("weld_edges", "Weld Edges")]); }
            if faces {
                r.one(vec![e("reverse", "Reverse faces"), e("orient", "Orient faces")]);
                r.family("Intersect", intersect());
                r.one(vec![soften()]);
                if edges && f.makes_faces {
                    r.one(vec![e("makefaces", "Make faces")]);
                }
                r.one(vec![e("heal", "Heal overlapping faces")]);
                r.one(vec![e("rebuild", "Rebuild faces (planar)")]);
                r.one(vec![e("simplify", "Simplify mesh…")]);
                r.family("Flip along", flip());
                if !edges && f.faces == 1 {
                    r.one(vec![ek("alignview", "Look at Face", "V")]);
                    if f.textured_face {
                        r.family("Texture", vec![e("texpos", "Position"), e("texreset", "Reset")]);
                    }
                }
            } else {
                if f.makes_faces {
                    r.one(vec![e("makefaces", "Make faces")]);
                }
                if f.loose_edges > 0 {
                    r.one(vec![e("divide", "Divide…")]);
                    if f.curve {
                        r.one(vec![e("explodecurve", "Explode curve")]);
                    }
                }
                r.one(vec![soften()]);
                r.family("Flip along", flip());
            }
        }
        "group" | "component" => {
            r.one(vec![ek("open", "Open for editing", "double-click")]);
            r.one(vec![e("rename", "Rename…")]);
            let mut visibility = vec![e("hide", "Hide")];
            visibility.extend(isolate_if(f));
            r.one(visibility);
            r.one(lock(f));
            r.one(zoom());
            r.one(file_to());
            if kind == "group" {
                r.one(vec![e("explode", EXPLODE), e("makecomp", MAKE_COMPONENT)]);
                if !f.billboard {
                    r.one(vec![e("split", "Split into pieces")]);
                }
                r.family("Intersect", intersect());
                r.one(vec![soften()]);
            } else {
                r.one(vec![e("instances", "Select instances"), e("unique", MAKE_UNIQUE)]);
                let mut row = vec![e("explode", EXPLODE)];
                if f.replaceable {
                    row.push(e("replace", "Replace component…"));
                }
                r.one(row);
                r.one(vec![checked(e("keep", "Keep in file"), f.kept)]);
                r.family("Intersect", intersect());
            }
            r.family("Flip along", flip());
            r.one(vec![e("axes", "Change axes")]);
        }
        "groups" => {
            r.one(hide_if(f));
            r.one(isolate_if(f));
            r.one(lock(f));
            r.one(zoom());
            r.one(file_to());
            r.one(make());
            r.one(vec![e("merge", MERGE_GROUPS), e("explode", EXPLODE)]);
            if f.components > 0 {
                let mut definitions = vec![e("unique", MAKE_UNIQUE)];
                if f.replaceable { definitions.push(e("replace", "Replace component…")); }
                definitions.push(checked(e("keep", "Keep in file"), f.kept));
                r.one(definitions);
            }
            r.family("Intersect", intersect());
            if f.solids >= 2 && f.solids == f.groups {
                let mut row = vec![e("solid:solid_union", "Union"), e("solid:solid_intersect", "Intersect")];
                if f.solids == 2 {
                    row.push(e("solid:solid_split", "Split"));
                }
                row.push(e("solid:outer_shell", "Outer shell"));
                r.family("Solid", row);
            }
            r.one(vec![soften()]);
            r.family("Flip along", flip());
        }
        "mixed" => {
            r.one(hide_if(f));
            r.one(isolate_if(f));
            r.one(zoom());
            if f.groups > 0 {
                r.one(file_to());
            }
            r.one(make());
            if f.faces > 0 {
                r.one(vec![e("reverse", "Reverse faces"), e("orient", "Orient faces")]);
            }
            r.family("Intersect", intersect());
            r.one(vec![soften()]);
            r.family("Flip along", flip());
        }
        "link" => {
            r.one(vec![e("contents", "Select contents")]);
            r.one(vec![e("hide", "Hide")]);
            r.one(isolate_if(f));
            // The placement's padlock is the host's, not the linked file's.
            r.one(lock(f));
            r.one(zoom());
            r.one(vec![e("linkopen", "Edit source")]);
            r.one(vec![e("linkreload", "Reload")]);
            if let (1, Some(nested)) = (f.total, f.include_nested) {
                r.one(vec![checked(e("linknested", "Include nested files"), nested)]);
            }
        }
        "linked_part" => {
            if f.host {
                r.one(vec![e("selectlink", "Select its linked file")]);
            }
            // Hiding is this document's view of the part, not an edit of it
            // (the tree's eye does the same): the inspector offered it and
            // the menu did not (council B5).
            r.one(hide_if(f));
            r.one(isolate_if(f));
            r.one(zoom());
            if f.host {
                r.one(vec![e("linkopen", "Edit source")]);
                r.one(vec![e("linkreload", "Reload")]);
            }
        }
        "annotation" => {
            if f.total == 1 {
                r.one(vec![e("edittext", "Edit text…")]);
            }
            r.one(zoom());
        }
        "guide" => {
            r.one(vec![e("deleteguide", if f.total == 1 { "Delete guide" } else { "Delete guides" })]);
            r.one(vec![e("deleteguides", "Delete all guides")]);
            r.one(zoom());
        }
        "empty" => {
            if f.paste {
                r.one(vec![ek("paste", "Paste here", "⌘V")]);
            }
            r.one(vec![ek("selectall", "Select all", "⌘A")]);
            r.one(vec![e("unhideall", "Unhide all")]);
            if f.isolated {
                r.one(vec![e("showall", "Show all")]);
            }
            r.one(vec![ek("zoomextents", "Zoom extents", "⇧⌘E")]);
            if f.guides {
                r.one(vec![e("deleteguides", "Delete all guides")]);
            }
        }
        "section" => {
            r.family("Section", vec![e("secreverse", "Reverse"), checked(e("secactive", "Active cut"), f.section_active),
                                     e("secalign", "Align view")]);
            r.one(vec![e("secname", "Name and symbol…")]);
            r.one(zoom());
        }
        "image" => {
            r.family("Image", vec![e("imgsize", "Size…"), e("imgopacity", "Opacity…")]);
            r.one(vec![e("imgexplode", "Explode image")]);
            r.one(vec![checked(e("imglock", "Lock"), f.image_locked)]);
            r.one(zoom());
        }
        "path" => {
            r.one(vec![e("profile", "Terrain profile")]);
            if f.path_polygon {
                r.family("Terrain surface", vec![e("surf:flat", "Flat"), e("surf:draped", "Draped"), e("surf:none", "None")]);
            }
            r.one(vec![e("convertpath", "Convert path to geometry")]);
            r.one(vec![e("closepath", "Open / close path")]);
        }
        _ => {}
    }
    if !read_only && f.faces + f.edges + f.groups > 0 {
        let mut grow = Vec::new();
        if f.faces + f.edges > 0 { grow.push(e("selconnected", "Connected")); }
        if f.faces > 0 { grow.push(e("selboundary", "Boundary")); }
        if f.faces > 0 || f.painted_group { grow.push(e("selmaterial", "Same material")); }
        grow.push(e("sellayer", "Same tag"));
        if f.components > 0 && kind != "component" { grow.push(e("selinstances", "Instances")); }
        r.family("Select", grow);
    }
    if f.total > 0 {
        if let Some(key) = &f.reveal_key {
            let reveal = ek("revealselection", "Reveal selection", key);
            match r.0.iter_mut().find(|(_, es)| es.iter().any(|e| e.0 == "zoom")) {
                Some(row) => row.1.push(reveal),
                None => r.one(vec![reveal]),
            }
        }
    }
    if f.taggable && !read_only {
        // The same footprint as Zoom to for large selection menus.
        let tag = e("selectiontag", "Tag…");
        if let Some((_, row)) = r.0.iter_mut().find(|(_, row)| row.iter().any(|verb| verb.0 == "zoom")) {
            row.push(tag);
        } else { r.one(vec![tag]); }
    }
    if f.total > 0 && !read_only && !f.all_locked && !f.image_locked && kind != "guide" {
        let erase = e("erase", "Erase");
        if let Some((_, entries)) = r.0.iter_mut().find(|(family, row)| family.is_empty()
            && row.len() == 1 && matches!(row[0].0.as_str(), "lock" | "hide")) {
            entries.push(erase);
        } else {
            r.one(vec![erase]);
        }
    }
    r.0
}

/// The inspector's verbs per kind, in chip order; the fixes go under More.
fn inspector_order(kind: &str) -> &'static [&'static str] {
    match kind {
        "faces" => &["hide", "makegroup", "intersect", "soften", "reverse", "orient", "heal", "rebuild", "simplify"],
        "edges" => &["hide", "makegroup", "soften", "join", "makefaces", "bridge"],
        "geometry" => &["hide", "makegroup", "intersect", "soften", "reverse", "orient", "heal", "rebuild", "simplify", "makefaces"],
        "group" => &["fileto", "isolate", "hide", "open", "explode", "makecomp", "status", "repair", "intersect", "soften", "split"],
        "component" => &["fileto", "isolate", "hide", "open", "explode", "unique", "status", "repair", "intersect"],
        "groups" => &["fileto", "isolate", "hide", "makegroup", "makecomp", "status", "merge", "intersect", "soften"],
        "mixed" => &["fileto", "isolate", "hide", "makegroup", "intersect", "soften", "reverse", "orient"],
        "link" => &["isolate", "hide", "zoom", "contents", "linkopen", "linkreload", "linknested"],
        "linked_part" => &["isolate", "hide", "zoom", "selectlink", "linkopen"],
        "annotation" => &["edittext", "zoom"],
        "guide" => &["deleteguide", "deleteguides"],
        "section" => &["secreverse", "secactive", "secalign", "zoom"],
        "image" => &["imgsize", "imgopacity", "imglock", "zoom"],
        "path" => &["profile", "convertpath", "closepath"],
        _ => &[],
    }
}

/// Whether a verb of `kind`'s inspector table may be offered at all.
pub fn offered(kind: &str, verb: &str) -> bool {
    if READ_ONLY.contains(&kind) && EDITING.contains(&verb) {
        return false;
    }
    inspector_order(kind).contains(&verb)
}

/// The inspector's verbs for this selection: (id, label, key, enabled,
/// under More). Exactly the menu's entries (same ids, same labels, same
/// enabled state) that the kind's inspector shows, in its order; a family
/// ("intersect") brings its segments, named "Intersect ▸ Model".
pub fn inspector(f: &Facts) -> Vec<(String, String, String, bool, bool)> {
    let kind = kind(f);
    let mut out = Vec::new();
    for (family, entries) in menu_rows(f, kind) {
        for (id, label, key, enabled, _) in entries {
            // These verbs need the locked entity under the pointer.
            if matches!(id.as_str(), "unlockgroup" | "unlockimage" | "deleteimage") { continue; }
            let label = if family.is_empty() { label } else { format!("{family} ▸ {label}") };
            let more = !matches!(id.as_str(), "fileto" | "isolate" | "hide" | "open");
            out.push((id, label, key, enabled, more));
        }
    }
    out
}

/// A linked file's row in the tree: the file verbs (id, label), by whether
/// the host places it itself (top) or it comes inside another file
/// (nested), and the link's state.
pub fn file_verbs(nested: bool, state: &str) -> Vec<(String, String)> {
    let all: &[&str] = if nested { &["linkopen", "linkfinder"] } else { &["linkopen", "linkreload", "linkunlink", "linkfinder", "linkrelink"] };
    all.iter()
        .filter(|v| state != "missing" || matches!(**v, "linkunlink" | "linkrelink"))
        .map(|v| {
            let label = match *v {
                "linkopen" => "Edit source",
                "linkreload" if state == "modified" => "Reload changes",
                "linkreload" => "Reload",
                "linkunlink" => "Unlink",
                "linkfinder" => "Show in Finder",
                _ => "Locate…",
            };
            ((*v).to_owned(), label.to_owned())
        })
        .collect()
}

/// What runs a verb of `kind`: (how, name, argument).
/// - `slot`: the window's method `name`, with `argument` ("" none, "sel0"
///   the first selected thing, "face0" the first face, "locked_group" /
///   "locked_image" what the menu was opened over, "none" Python's None,
///   else the literal string);
/// - `tree`: the Project tree's verb `name` on the first selected item's row;
/// - `core`: the core command `name` on the selected placements;
/// - `glue`: a named action in viewport_menu.rs.
pub fn dispatch(kind: &str, id: &str) -> (&'static str, String, String) {
    let slot = |n: &str, a: &str| ("slot", n.to_owned(), a.to_owned());
    let glue = |n: &str, a: &str| ("glue", n.to_owned(), a.to_owned());
    let tree = |n: &str| ("tree", n.to_owned(), String::new());
    let one = matches!(kind, "group" | "component");
    if let Some(mode) = id.strip_prefix("intersect:") {
        return glue("intersect", mode);
    }
    if let Some(axis) = id.strip_prefix("flip:") {
        return slot("_on_flip_along", axis);
    }
    if let Some(tool) = id.strip_prefix("solid:") {
        return slot("_activate_tool", tool);
    }
    if let Some(surface) = id.strip_prefix("surf:") {
        return slot("_on_set_surface", surface);
    }
    match id {
        "erase" => slot("_on_delete_selection", ""),
        "repeat" => slot("repeat_last_command", ""),
        "unlockgroup" => slot("_unlock_group", "locked_group"),
        "unlockimage" => slot("_unlock_image", "locked_image"),
        "deleteimage" => slot("_delete_image", "locked_image"),
        "edittext3d" => slot("_on_edit_3d_text", "sel0"),
        "selconnected" => glue("grow", "all_connected"),
        "selboundary" => glue("grow", "bounding_edges"),
        "selmaterial" => glue("grow", "same_material"),
        "sellayer" => glue("grow", "same_layer"),
        "instances" | "selinstances" => glue("grow", "same_definition"),
        "selectiontag" => glue("selectiontag", ""),
        "hide" if one => tree("hideitem"),
        "hide" if kind == "link" => ("core", "hide".into(), String::new()),
        "hide" => slot("_on_hide", ""),
        "isolate" => glue("isolate", ""),
        "zoom" if one => tree("zoom"),
        "zoom" => slot("_on_zoom_selection", ""),
        "fileto" => glue("fileto", ""),
        "makegroup" => slot("_on_make_group", ""),
        "makecomp" => slot("_on_make_component", ""),
        "soften" => slot("_on_soften_edges", ""),
        "reverse" => slot("_on_reverse_faces", ""),
        "orient" => slot("_on_orient_faces", ""),
        "makefaces" => slot("_on_make_faces", ""),
        "heal" => slot("_on_heal_overlaps", ""),
        "rebuild" => slot("_on_rebuild_planar", ""),
        "simplify" => slot("_on_simplify_mesh", ""),
        "alignview" => slot("_on_align_view_to_face", "face0"),
        "texpos" => slot("_on_texture_position", ""),
        "texreset" => slot("_on_texture_reset", ""),
        "divide" => slot("_on_divide", ""),
        "join" => slot("_on_weld_edges", ""),
        "weld_edges" => slot("_on_weld_edges", ""),
        "explodecurve" => slot("_on_explode_curve", ""),
        "bridge" => slot("_on_bridge", ""),
        "open" => tree("open"),
        "rename" => glue("rename", ""),
        "lock" => glue("lock", ""),
        "explode" => slot("_on_explode_group", ""),
        "split" => slot("_on_split_into_pieces", ""),
        "axes" => slot("_on_change_axes", "sel0"),
        "unique" if kind == "component" => tree("unique"),
        "unique" => slot("_on_make_unique", ""),
        "replace" => slot("_on_replace_component", ""),
        "keep" => glue("keep", ""),
        "merge" => slot("_on_merge_groups", ""),
        "contents" => glue("contents", ""),
        "linkopen" | "linkreload" | "linknested" => glue("link", id),
        "selectlink" => glue("selectlink", ""),
        "edittext" => glue("edittext", ""),
        "deleteguide" => glue("deleteguide", ""),
        "deleteguides" => slot("_on_delete_guides", ""),
        "paste" => slot("_on_paste", ""),
        "selectall" => slot("_on_select_all", ""),
        "unhideall" => slot("_on_unhide_all", ""),
        "showall" => glue("showall", ""),
        "zoomextents" => slot("_on_zoom_extents", ""),
        "secreverse" => slot("_on_reverse_section", ""),
        "secactive" => slot("_on_toggle_active_section", ""),
        "secalign" => slot("_on_align_view_to_section", ""),
        "secname" => glue("secname", ""),
        "imgsize" => slot("_on_image_size", ""),
        "imgopacity" => slot("_on_image_opacity", ""),
        "imgexplode" => slot("_on_explode_image", ""),
        "imglock" => slot("_on_toggle_image_lock", ""),
        "profile" => slot("_on_terrain_profile", ""),
        "convertpath" => slot("_on_convert_geopath", ""),
        "closepath" => slot("_on_toggle_path_closed", ""),
        "revealselection" => glue("reveal", ""),
        _ => ("none", String::new(), String::new()),
    }
}

fn facts(json: &str) -> PyResult<Facts> {
    serde_json::from_str(json).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
}

/// `(kind, title, subtitle, rows)` of the right-click menu for these facts.
#[pyfunction]
pub fn verbs_menu(facts_json: &str) -> PyResult<(String, String, String, Vec<Row>)> {
    let f = facts(facts_json)?;
    let kind = kind(&f);
    let (title, subtitle) = header(&f, kind);
    Ok((kind.to_owned(), title, subtitle, menu_rows(&f, kind)))
}

/// The kind of a selection described by these facts.
#[pyfunction]
pub fn verbs_kind(facts_json: &str) -> PyResult<String> {
    Ok(kind(&facts(facts_json)?).to_owned())
}

/// The inspector's verbs: (id, label, key, enabled, under More).
#[pyfunction]
pub fn verbs_inspector(facts_json: &str) -> PyResult<Vec<(String, String, String, bool, bool)>> {
    Ok(inspector(&facts(facts_json)?))
}

/// Whether the inspector of `kind` offers `verb`; whether names edit in
/// place; whether the kind is read-only.
#[pyfunction]
pub fn verbs_offered(kind: &str, verb: &str) -> bool {
    offered(kind, verb)
}
#[pyfunction]
pub fn verbs_read_only(kind: &str) -> bool {
    READ_ONLY.contains(&kind)
}
#[pyfunction]
pub fn verbs_name_editable(kind: &str) -> bool {
    !READ_ONLY.contains(&kind) && kind != "empty"
}
#[pyfunction]
pub fn verbs_file(nested: bool, state: &str) -> Vec<(String, String)> {
    file_verbs(nested, state)
}
#[pyfunction]
pub fn verbs_dispatch(kind: &str, id: &str) -> (String, String, String) {
    let (how, name, arg) = dispatch(kind, id);
    (how.to_owned(), name, arg)
}

/// The table's constants for Python and its tests: kinds, read-only kinds,
/// editing verbs, the most rows a menu may have.
#[pyfunction]
pub fn verbs_constants() -> (Vec<&'static str>, Vec<&'static str>, Vec<&'static str>, usize) {
    (KINDS.to_vec(), READ_ONLY.to_vec(), EDITING.to_vec(), MAX_ROWS)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    for f in [wrap_pyfunction!(verbs_menu, m)?, wrap_pyfunction!(verbs_kind, m)?, wrap_pyfunction!(verbs_inspector, m)?,
              wrap_pyfunction!(verbs_offered, m)?, wrap_pyfunction!(verbs_read_only, m)?,
              wrap_pyfunction!(verbs_name_editable, m)?, wrap_pyfunction!(verbs_file, m)?,
              wrap_pyfunction!(verbs_dispatch, m)?, wrap_pyfunction!(verbs_constants, m)?] {
        m.add_function(f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(rows: &[Row]) -> Vec<String> {
        rows.iter().flat_map(|(_, es)| es.iter().map(|e| e.0.clone())).collect()
    }

    fn facts_for(kind: &str) -> Facts {
        let mut f = Facts { total: 1, hideable: true, repeat: Some("Push/Pull".into()), ..Default::default() };
        match kind {
            "faces" => f.faces = 1,
            "edges" => { f.edges = 2; f.loose_edges = 2; f.total = 2; }
            "geometry" => { f.faces = 1; f.edges = 1; f.total = 2; }
            "group" => f.groups = 1,
            "component" => { f.groups = 1; f.components = 1; }
            "groups" => { f.groups = 2; f.total = 2; }
            "mixed" => { f.groups = 1; f.faces = 1; f.total = 2; }
            "link" => { f.groups = 1; f.all_links = true; f.include_nested = Some(true); }
            "linked_part" => { f.groups = 1; f.linked_part = true; f.host = true; }
            "annotation" => f.all_annotations = true,
            "guide" => f.all_guides = true,
            "empty" => f.total = 0,
            "section" => f.all_sections = true,
            "image" => f.all_images = true,
            "path" => f.all_paths = true,
            _ => {}
        }
        f
    }

    #[test]
    fn every_kind_is_recognised_and_its_menu_fits() {
        for k in KINDS {
            let mut f = facts_for(k);
            f.taggable = !matches!(k, "empty" | "link" | "linked_part" | "path");
            f.reveal_key = Some("⇧⌘J".into());
            assert_eq!(kind(&f), k);
            let rows = menu_rows(&f, k);
            assert!(rows.len() <= MAX_ROWS, "{k}: {} rows", rows.len());
            assert_eq!(rows[0].1[0].0, "repeat");
            for id in ids(&rows) {
                assert_ne!(dispatch(k, &id).0, "none", "{k}: {id} runs nothing");
            }
        }
    }

    #[test]
    fn read_only_kinds_offer_no_editing_verb() {
        for k in READ_ONLY {
            let f = facts_for(k);
            for id in ids(&menu_rows(&f, k)) {
                let base = id.split(':').next().unwrap();
                assert!(!EDITING.contains(&base), "{k} offers {id}");
            }
            assert!(!menu_rows(&f, k)[0].1[0].3, "{k}: Repeat must be off");
            for (id, ..) in inspector(&f) {
                assert!(!EDITING.contains(&id.split(':').next().unwrap()), "{k}'s inspector offers {id}");
            }
        }
    }

    #[test]
    fn the_inspector_offers_only_what_the_menu_offers_with_its_labels() {
        for k in KINDS {
            let f = facts_for(k);
            let rows = menu_rows(&f, k);
            let menu: Vec<(String, String)> = rows.iter().flat_map(|(_, es)| es.iter().map(|e| (e.0.clone(), e.1.clone()))).collect();
            for (id, label, _, _, _) in inspector(&f) {
                let (_, menu_label) = menu.iter().find(|(m, _)| *m == id).unwrap_or_else(|| panic!("{k}: {id} not in the menu"));
                assert!(label.ends_with(menu_label.as_str()), "{k}: {id} «{label}» vs «{menu_label}»");
            }
        }
        // Loose geometry cannot be isolated (council B5).
        assert!(!inspector(&facts_for("edges")).iter().any(|v| v.0 == "isolate"));
        assert!(inspector(&facts_for("faces")).iter().any(|v| v.0 == "hide"));
    }

    #[test]
    fn headers_name_what_was_hit() {
        let mut f = facts_for("component");
        f.name = "Door".into();
        f.instances = 3;
        f.filed = "Unfiled".into();
        assert_eq!(header(&f, "component"), ("Door".into(), "Component · 3 instances · Unfiled".into()));
        let mut f = facts_for("annotation");
        f.text = "Slab edge".into();
        assert_eq!(header(&f, "annotation"), ("Text".into(), "“Slab edge”".into()));
        let f = facts_for("empty");
        assert_eq!(header(&f, "empty").0, if f.context.is_empty() { "Model" } else { &f.context });
    }

    #[test]
    fn file_rows_follow_their_state() {
        assert_eq!(file_verbs(true, "resolved").len(), 2);
        let missing: Vec<String> = file_verbs(false, "missing").into_iter().map(|v| v.0).collect();
        assert_eq!(missing, ["linkunlink", "linkrelink"]);
        assert!(file_verbs(false, "modified").iter().any(|v| v.1 == "Reload changes"));
    }
}
