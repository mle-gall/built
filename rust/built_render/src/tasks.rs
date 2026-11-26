// SPDX-License-Identifier: GPL-3.0-or-later
//! Agent tasks: one request to an agent (the built-in assistant or an
//! external one over MCP), what it reported, the elements its edits
//! changed and the user's answer. The document keeps them as BCF 3.0
//! topics (the BCF-API JSON field names) under `plugin_data["bcf"]`; what
//! BCF has no field for (the task's state, its net changes, the 3D pin)
//! rides in each topic's `built` extension.
//!
//! The model is plain data and its state machine; `tasks_host.rs` binds it
//! to the window (storage, attribution of edits, the Tasks destination).
use serde_json::{json, Map, Value};

pub const BCF_VERSION: &str = "3.0";
pub const TOPIC_TYPE: &str = "Agent task";
pub const ORIGIN: &str = "Built";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The agent is executing (or has the request and has not answered).
    Working,
    /// The agent needs an approval or another action from the user.
    Waiting,
    /// The agent asked something and waits for a reply.
    Question,
    /// The agent finished with edits: the user accepts, asks for changes
    /// or reverts.
    Review,
    Done,
    Reverted,
}

impl State {
    pub const ALL: [State; 6] = [State::Working, State::Waiting, State::Question, State::Review, State::Done, State::Reverted];

    pub fn key(self) -> &'static str {
        match self {
            State::Working => "working",
            State::Waiting => "waiting",
            State::Question => "question",
            State::Review => "review",
            State::Done => "done",
            State::Reverted => "reverted",
        }
    }
    /// The words the row's chip shows (and BCF's `topic_status`).
    pub fn label(self) -> &'static str {
        match self {
            State::Working => "Working",
            State::Waiting => "Waiting for you",
            State::Question => "Question",
            State::Review => "To review",
            State::Done => "Done",
            State::Reverted => "Reverted",
        }
    }
    /// A state by key or label, any case ("to review", "review", "Done").
    pub fn parse(text: &str) -> Option<State> {
        let t = text.trim().to_lowercase();
        State::ALL.into_iter().find(|s| s.key() == t || s.label().to_lowercase() == t)
            .or(match t.as_str() {
                "waiting for you" | "waiting_for_you" | "approval" => Some(State::Waiting),
                "to_review" | "in review" => Some(State::Review),
                _ => None,
            })
    }
    pub fn needs_attention(self) -> bool {
        matches!(self, State::Waiting | State::Question | State::Review)
    }
    pub fn settled(self) -> bool {
        matches!(self, State::Done | State::Reverted)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Created,
    Removed,
    Modified,
}

impl ChangeKind {
    pub fn key(self) -> &'static str {
        match self {
            ChangeKind::Created => "created",
            ChangeKind::Removed => "removed",
            ChangeKind::Modified => "modified",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ChangeKind::Created => "Added",
            ChangeKind::Removed => "Removed",
            ChangeKind::Modified => "Changed",
        }
    }
    pub fn parse(text: &str) -> Option<ChangeKind> {
        match text {
            "created" => Some(ChangeKind::Created),
            "removed" => Some(ChangeKind::Removed),
            "modified" => Some(ChangeKind::Modified),
            _ => None,
        }
    }
}

/// One element a task's edits changed: its key (the placement uid), the
/// name it had and how it changed, net of everything the task did to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Changed {
    pub key: String,
    pub label: String,
    pub kind: ChangeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub guid: String,
    pub date: String,
    pub author: String,
    pub text: String,
}

/// Where the task was asked from: the camera (BCF's perspective or
/// orthogonal camera, plus the orbit pose that brings it back exactly) and
/// the selection.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Viewpoint {
    pub guid: String,
    pub eye: [f64; 3],
    pub direction: [f64; 3],
    pub up: [f64; 3],
    pub fov: f64,
    pub ortho: bool,
    pub scale: f64,
    /// target x, y, z, distance, yaw, pitch.
    pub pose: Option<[f64; 6]>,
    pub selection: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub guid: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub agent: String,
    /// "assistant" (built in), "agent" (MCP) or "user".
    pub source: String,
    pub state: State,
    pub created: String,
    pub modified: String,
    pub modified_author: String,
    pub viewpoint: Option<Viewpoint>,
    pub pin: Option<[f64; 3]>,
    pub changes: Vec<Changed>,
    /// The history revisions its edits spanned.
    pub revisions: Option<(u64, u64)>,
    /// History steps attributed to it.
    pub edits: u32,
    pub summary: String,
    pub question: String,
    pub comments: Vec<Comment>,
}

/// Merge `next` into the net change of one element.
fn merge(prev: ChangeKind, next: ChangeKind) -> Option<ChangeKind> {
    use ChangeKind::*;
    match (prev, next) {
        (Created, Removed) => None,
        (Created, _) => Some(Created),
        (Removed, Created) => Some(Modified),
        (Removed, _) => Some(Removed),
        (Modified, Removed) => Some(Removed),
        (Modified, _) => Some(Modified),
    }
}

impl Task {
    pub fn new(guid: &str, title: &str, description: &str, author: &str, agent: &str, source: &str, now: &str) -> Task {
        Task {
            guid: guid.into(),
            title: title.trim().into(),
            description: description.trim().into(),
            author: author.into(),
            agent: agent.into(),
            source: source.into(),
            state: State::Working,
            created: now.into(),
            modified: now.into(),
            modified_author: author.into(),
            viewpoint: None,
            pin: None,
            changes: Vec::new(),
            revisions: None,
            edits: 0,
            summary: String::new(),
            question: String::new(),
            comments: Vec::new(),
        }
    }

    fn touch(&mut self, author: &str, now: &str) {
        self.modified = now.into();
        self.modified_author = author.into();
    }

    pub fn comment(&mut self, guid: &str, author: &str, text: &str, now: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.comments.push(Comment { guid: guid.into(), date: now.into(), author: author.into(), text: text.into() });
        self.touch(author, now);
    }

    fn live(&self) -> Result<(), String> {
        if self.state.settled() {
            Err(format!("The task is {}", self.state.label().to_lowercase()))
        } else {
            Ok(())
        }
    }

    /// The agent says where it is: working, waiting for the user, or asking.
    pub fn report(&mut self, state: State, message: &str, author: &str, comment_guid: &str, now: &str) -> Result<(), String> {
        self.live()?;
        match state {
            State::Working | State::Waiting => self.question.clear(),
            State::Question => {
                if message.trim().is_empty() {
                    return Err("A question needs its text".into());
                }
                self.question = message.trim().into();
            }
            _ => return Err(format!("An agent reports Working, Waiting for you or Question, not {}", state.label())),
        }
        self.state = state;
        self.comment(comment_guid, author, message, now);
        self.touch(author, now);
        Ok(())
    }

    /// The agent is done: with no net edit it settles to Done, with edits
    /// it waits for review.
    pub fn finish(&mut self, summary: &str, author: &str, comment_guid: &str, now: &str) -> Result<State, String> {
        self.live()?;
        self.question.clear();
        if !summary.trim().is_empty() {
            self.summary = summary.trim().into();
        }
        self.state = if self.changes.is_empty() { State::Done } else { State::Review };
        self.comment(comment_guid, author, summary, now);
        self.touch(author, now);
        Ok(self.state)
    }

    /// The user keeps the edits.
    pub fn accept(&mut self, author: &str, now: &str) -> Result<(), String> {
        if self.state != State::Review {
            return Err(format!("Only a task to review can be accepted ({})", self.state.label()));
        }
        self.state = State::Done;
        self.touch(author, now);
        Ok(())
    }

    /// The user reopens the task with feedback: the same agent works again.
    pub fn request_changes(&mut self, text: &str, author: &str, comment_guid: &str, now: &str) -> Result<(), String> {
        if !matches!(self.state, State::Review | State::Done) {
            return Err(format!("Changes are requested on a finished task ({})", self.state.label()));
        }
        if text.trim().is_empty() {
            return Err("Say what to change".into());
        }
        self.state = State::Working;
        self.comment(comment_guid, author, text, now);
        Ok(())
    }

    /// The user answers a question or a request for action.
    pub fn reply(&mut self, text: &str, author: &str, comment_guid: &str, now: &str) -> Result<(), String> {
        if !matches!(self.state, State::Question | State::Waiting) {
            return Err(format!("The task is not waiting for a reply ({})", self.state.label()));
        }
        if text.trim().is_empty() {
            return Err("Write a reply".into());
        }
        self.question.clear();
        self.state = State::Working;
        self.comment(comment_guid, author, text, now);
        Ok(())
    }

    /// Its edits were undone as one step.
    pub fn revert(&mut self, author: &str, now: &str) -> Result<(), String> {
        if !matches!(self.state, State::Review | State::Done) || self.changes.is_empty() {
            return Err("Only a finished task with edits can be reverted".into());
        }
        self.state = State::Reverted;
        self.touch(author, now);
        Ok(())
    }

    /// Fold one history step's changes in, net: an element added then
    /// removed by the same task is no change at all.
    pub fn record(&mut self, changes: &[Changed], revision: u64) {
        for c in changes {
            match self.changes.iter().position(|e| e.key == c.key) {
                Some(at) => match merge(self.changes[at].kind, c.kind) {
                    Some(kind) => {
                        self.changes[at].kind = kind;
                        if !c.label.is_empty() {
                            self.changes[at].label = c.label.clone();
                        }
                    }
                    None => {
                        self.changes.remove(at);
                    }
                },
                None => self.changes.push(c.clone()),
            }
        }
        self.edits += 1;
        self.revisions = Some(match self.revisions {
            Some((first, _)) => (first, revision),
            None => (revision, revision),
        });
    }

    /// Start again from no change (the host replays the steps still in
    /// the history).
    pub fn reset_changes(&mut self) {
        self.changes.clear();
        self.edits = 0;
        self.revisions = None;
    }

    pub fn changed_keys(&self) -> Vec<String> {
        self.changes.iter().map(|c| c.key.clone()).collect()
    }

    // ------------------------------------------------------------- BCF

    pub fn to_bcf(&self) -> Value {
        let comments: Vec<Value> = self.comments.iter().map(|c| json!({
            "guid": c.guid,
            "date": c.date,
            "author": c.author,
            "comment": c.text,
            "topic_guid": self.guid,
        })).collect();
        let viewpoints: Vec<Value> = self.viewpoint.iter().map(|v| {
            let point = |p: [f64; 3]| json!({"x": p[0], "y": p[1], "z": p[2]});
            let mut vp = Map::new();
            vp.insert("guid".into(), json!(v.guid));
            let mut camera = Map::new();
            camera.insert("camera_view_point".into(), point(v.eye));
            camera.insert("camera_direction".into(), point(v.direction));
            camera.insert("camera_up_vector".into(), point(v.up));
            camera.insert("aspect_ratio".into(), json!(1.6));
            if v.ortho {
                camera.insert("view_to_world_scale".into(), json!(v.scale));
                vp.insert("orthogonal_camera".into(), Value::Object(camera));
            } else {
                camera.insert("field_of_view".into(), json!(v.fov));
                vp.insert("perspective_camera".into(), Value::Object(camera));
            }
            let selection: Vec<Value> = v.selection.iter().map(|k| json!({
                "ifc_guid": Value::Null,
                "originating_system": ORIGIN,
                "authoring_tool_id": k,
            })).collect();
            vp.insert("components".into(), json!({"selection": selection}));
            if let Some(pose) = v.pose {
                vp.insert("built".into(), json!({"pose": pose}));
            }
            Value::Object(vp)
        }).collect();
        let changes: Vec<Value> = self.changes.iter().map(|c| json!({
            "key": c.key, "label": c.label, "change": c.kind.key(),
        })).collect();
        let mut built = Map::new();
        built.insert("state".into(), json!(self.state.key()));
        built.insert("source".into(), json!(self.source));
        built.insert("changes".into(), Value::Array(changes));
        built.insert("edits".into(), json!(self.edits));
        if let Some((a, b)) = self.revisions {
            built.insert("revisions".into(), json!([a, b]));
        }
        if let Some(pin) = self.pin {
            built.insert("pin".into(), json!(pin));
        }
        if !self.summary.is_empty() {
            built.insert("summary".into(), json!(self.summary));
        }
        if !self.question.is_empty() {
            built.insert("question".into(), json!(self.question));
        }
        json!({
            "guid": self.guid,
            "topic_type": TOPIC_TYPE,
            "topic_status": self.state.label(),
            "title": self.title,
            "description": self.description,
            "labels": ["Agent"],
            "creation_date": self.created,
            "creation_author": self.author,
            "modified_date": self.modified,
            "modified_author": self.modified_author,
            "assigned_to": self.agent,
            "comments": comments,
            "viewpoints": viewpoints,
            "built": Value::Object(built),
        })
    }

    pub fn from_bcf(v: &Value) -> Option<Task> {
        let s = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let guid = s(v, "guid");
        if guid.is_empty() {
            return None;
        }
        let built = v.get("built").cloned().unwrap_or(Value::Null);
        let state = State::parse(&s(&built, "state")).or_else(|| State::parse(&s(v, "topic_status"))).unwrap_or(State::Done);
        let point = |o: Option<&Value>| -> [f64; 3] {
            let c = |k| o.and_then(|o| o.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
            [c("x"), c("y"), c("z")]
        };
        let viewpoint = v.get("viewpoints").and_then(Value::as_array).and_then(|a| a.first()).map(|vp| {
            let (camera, ortho) = match (vp.get("perspective_camera"), vp.get("orthogonal_camera")) {
                (Some(c), _) => (Some(c), false),
                (None, Some(c)) => (Some(c), true),
                _ => (None, false),
            };
            let num = |k: &str| camera.and_then(|c| c.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
            let pose = vp.get("built").and_then(|b| b.get("pose")).and_then(Value::as_array).and_then(|a| {
                let n: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
                (n.len() == 6).then(|| [n[0], n[1], n[2], n[3], n[4], n[5]])
            });
            Viewpoint {
                guid: s(vp, "guid"),
                eye: point(camera.and_then(|c| c.get("camera_view_point"))),
                direction: point(camera.and_then(|c| c.get("camera_direction"))),
                up: point(camera.and_then(|c| c.get("camera_up_vector"))),
                fov: num("field_of_view"),
                ortho,
                scale: num("view_to_world_scale"),
                pose,
                selection: vp.get("components").and_then(|c| c.get("selection")).and_then(Value::as_array)
                    .map(|a| a.iter().map(|c| s(c, "authoring_tool_id")).filter(|k| !k.is_empty()).collect())
                    .unwrap_or_default(),
            }
        });
        let changes = built.get("changes").and_then(Value::as_array).map(|a| a.iter().filter_map(|c| Some(Changed {
            key: s(c, "key"),
            label: s(c, "label"),
            kind: ChangeKind::parse(&s(c, "change"))?,
        })).filter(|c| !c.key.is_empty()).collect()).unwrap_or_default();
        let comments = v.get("comments").and_then(Value::as_array).map(|a| a.iter().map(|c| Comment {
            guid: s(c, "guid"),
            date: s(c, "date"),
            author: s(c, "author"),
            text: s(c, "comment"),
        }).collect()).unwrap_or_default();
        let pin = built.get("pin").and_then(Value::as_array).and_then(|a| {
            let n: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
            (n.len() == 3).then(|| [n[0], n[1], n[2]])
        });
        let revisions = built.get("revisions").and_then(Value::as_array).and_then(|a| {
            Some((a.first()?.as_u64()?, a.get(1)?.as_u64()?))
        });
        Some(Task {
            guid,
            title: s(v, "title"),
            description: s(v, "description"),
            author: s(v, "creation_author"),
            agent: s(v, "assigned_to"),
            source: { let src = s(&built, "source"); if src.is_empty() { "agent".into() } else { src } },
            state,
            created: s(v, "creation_date"),
            modified: s(v, "modified_date"),
            modified_author: s(v, "modified_author"),
            viewpoint,
            pin,
            changes,
            revisions,
            edits: built.get("edits").and_then(Value::as_u64).unwrap_or(0) as u32,
            summary: s(&built, "summary"),
            question: s(&built, "question"),
            comments,
        })
    }
}

/// The document's tasks, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tasks {
    pub topics: Vec<Task>,
}

impl Tasks {
    pub fn from_bcf(v: &Value) -> Tasks {
        Tasks {
            topics: v.get("topics").and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Task::from_bcf).collect())
                .unwrap_or_default(),
        }
    }
    pub fn to_bcf(&self) -> Value {
        json!({
            "version": BCF_VERSION,
            "topics": self.topics.iter().map(Task::to_bcf).collect::<Vec<_>>(),
        })
    }
    pub fn get(&self, guid: &str) -> Option<&Task> {
        self.topics.iter().find(|t| t.guid == guid)
    }
    pub fn get_mut(&mut self, guid: &str) -> Result<&mut Task, String> {
        self.topics.iter_mut().find(|t| t.guid == guid).ok_or_else(|| format!("No task {guid}"))
    }
    pub fn attention(&self) -> usize {
        self.topics.iter().filter(|t| t.state.needs_attention()).count()
    }
    /// (Needs attention, In progress, Settled), newest first in each.
    pub fn groups(&self) -> [Vec<&Task>; 3] {
        let mut out: [Vec<&Task>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for t in self.topics.iter().rev() {
            let at = if t.state.needs_attention() { 0 } else if t.state.settled() { 2 } else { 1 };
            out[at].push(t);
        }
        out
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for seconds since the epoch (BCF's dates).
pub fn iso_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil from days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    iso_date(secs)
}

/// A random version-4 GUID (BCF identifies topics, comments and
/// viewpoints by GUID).
pub fn guid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let state = std::collections::hash_map::RandomState::new();
    let word = |salt: u64| {
        let mut h = state.build_hasher();
        h.write_u64(salt);
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.finish()
    };
    let (a, b) = (word(1), word(2));
    let b = (b & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    let a = (a & 0xffff_ffff_ffff_0fff) | 0x0000_0000_0000_4000;
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", a >> 32, (a >> 16) & 0xffff, a & 0xffff, b >> 48, b & 0xffff_ffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(key: &str, kind: ChangeKind) -> Changed {
        Changed { key: key.into(), label: format!("Slat {key}"), kind }
    }

    fn task() -> Task {
        Task::new("t1", "Align the backrest slats", "Align them with the frame", "You", "Codex", "agent", "2026-11-26T10:00:00Z")
    }

    #[test]
    fn a_task_without_net_edits_settles_to_done() {
        let mut t = task();
        assert_eq!(t.finish("Nothing to change", "Codex", "c1", "now").unwrap(), State::Done);
        assert!(t.state.settled() && !t.state.needs_attention());
        // Added then removed by the same task: no net edit.
        let mut t = task();
        t.record(&[changed("a", ChangeKind::Created)], 4);
        t.record(&[changed("a", ChangeKind::Removed)], 5);
        assert!(t.changes.is_empty());
        assert_eq!(t.finish("", "Codex", "c1", "now").unwrap(), State::Done);
    }

    #[test]
    fn a_task_with_edits_waits_for_review_then_accept_or_revert() {
        let mut t = task();
        t.record(&[changed("a", ChangeKind::Modified), changed("b", ChangeKind::Created)], 3);
        t.record(&[changed("a", ChangeKind::Removed), changed("b", ChangeKind::Modified)], 4);
        assert_eq!(t.changes.iter().map(|c| (c.key.as_str(), c.kind)).collect::<Vec<_>>(),
                   [("a", ChangeKind::Removed), ("b", ChangeKind::Created)]);
        assert_eq!(t.revisions, Some((3, 4)));
        assert_eq!(t.edits, 2);
        assert_eq!(t.finish("Moved three slats", "Codex", "c1", "now").unwrap(), State::Review);
        assert!(t.state.needs_attention());
        let mut kept = t.clone();
        kept.accept("You", "now").unwrap();
        assert_eq!(kept.state, State::Done);
        assert!(kept.accept("You", "now").is_err());
        t.revert("You", "now").unwrap();
        assert_eq!(t.state, State::Reverted);
        assert!(t.report(State::Working, "", "Codex", "c2", "now").is_err(), "a settled task takes no report");
    }

    #[test]
    fn questions_and_requests_for_changes_reopen_the_same_task() {
        let mut t = task();
        assert!(t.report(State::Question, "  ", "Codex", "c1", "now").is_err());
        t.report(State::Question, "From the backrest or the rear posts?", "Codex", "c1", "now").unwrap();
        assert_eq!((t.state, t.question.as_str()), (State::Question, "From the backrest or the rear posts?"));
        assert!(t.reply("", "You", "c2", "now").is_err());
        t.reply("The rear posts", "You", "c2", "now").unwrap();
        assert_eq!((t.state, t.question.as_str()), (State::Working, ""));
        assert!(t.request_changes("again", "You", "c3", "now").is_err(), "nothing to review yet");
        t.record(&[changed("a", ChangeKind::Modified)], 9);
        t.finish("Done", "Codex", "c3", "now").unwrap();
        t.request_changes("Lower them 2 cm", "You", "c4", "now").unwrap();
        assert_eq!(t.state, State::Working);
        assert_eq!(t.comments.iter().map(|c| c.author.as_str()).collect::<Vec<_>>(), ["Codex", "You", "Codex", "You"]);
        assert!(t.report(State::Review, "", "Codex", "c5", "now").is_err(), "review comes from finish");
        t.report(State::Waiting, "Approve the new material?", "Codex", "c5", "now").unwrap();
        assert!(t.state.needs_attention());
    }

    #[test]
    fn tasks_round_trip_through_bcf_topics() {
        let mut t = task();
        t.viewpoint = Some(Viewpoint {
            guid: "v1".into(), eye: [1., 2., 3.], direction: [0., 1., 0.], up: [0., 0., 1.], fov: 45.,
            ortho: false, scale: 0., pose: Some([0., 0., 1., 10., 30., 20.]), selection: vec!["u1".into(), "u2".into()],
        });
        t.pin = Some([0.5, 1.5, 0.75]);
        t.record(&[changed("u1", ChangeKind::Modified)], 7);
        t.report(State::Question, "Which side?", "Codex", "c1", "2026-11-26T10:05:00Z").unwrap();
        let mut ortho = Task::new("t2", "Check callouts", "", "You", "Built-in assistant", "assistant", "2026-11-26T11:00:00Z");
        ortho.viewpoint = Some(Viewpoint { guid: "v2".into(), ortho: true, scale: 12.5, ..Default::default() });
        let tasks = Tasks { topics: vec![t, ortho] };
        let bcf = tasks.to_bcf();
        assert_eq!(bcf["version"], "3.0");
        let topic = &bcf["topics"][0];
        assert_eq!(topic["topic_status"], "Question");
        assert_eq!(topic["topic_type"], TOPIC_TYPE);
        assert_eq!(topic["comments"][0]["topic_guid"], "t1");
        assert_eq!(topic["viewpoints"][0]["perspective_camera"]["camera_view_point"]["z"], 3.0);
        assert_eq!(topic["viewpoints"][0]["components"]["selection"][1]["authoring_tool_id"], "u2");
        assert_eq!(bcf["topics"][1]["viewpoints"][0]["orthogonal_camera"]["view_to_world_scale"], 12.5);
        let text = serde_json::to_string(&bcf).unwrap();
        let back = Tasks::from_bcf(&serde_json::from_str(&text).unwrap());
        assert_eq!(back, tasks);
        assert_eq!(back.attention(), 1);
        let [attention, progress, settled] = back.groups();
        assert_eq!((attention.len(), progress.len(), settled.len()), (1, 1, 0));
    }

    #[test]
    fn a_foreign_topic_without_the_extension_still_reads() {
        let topic = json!({"guid": "x", "title": "From another tool", "topic_status": "To review",
                           "creation_author": "a@b", "comments": [{"guid": "c", "comment": "hi", "author": "a@b"}]});
        let t = Task::from_bcf(&topic).unwrap();
        assert_eq!((t.state, t.title.as_str(), t.comments.len()), (State::Review, "From another tool", 1));
        assert!(Task::from_bcf(&json!({"title": "no guid"})).is_none());
    }

    #[test]
    fn states_parse_by_key_or_words() {
        for s in State::ALL {
            assert_eq!(State::parse(s.key()), Some(s));
            assert_eq!(State::parse(s.label()), Some(s));
        }
        assert_eq!(State::parse("Waiting for you"), Some(State::Waiting));
        assert_eq!(State::parse("nonsense"), None);
    }

    #[test]
    fn dates_and_guids_have_the_bcf_shape() {
        assert_eq!(iso_date(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_date(1_795_694_400), "2026-11-26T12:00:00Z");
        let (a, b) = (guid(), guid());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
    }
}
