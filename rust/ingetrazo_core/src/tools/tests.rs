//! The machines against a fake host: the commands they run, the selection
//! calls they make, the state the inference reads.

use super::*;

#[derive(Default)]
struct Fake {
    log: Vec<String>,
    /// Face delta each `add_edge` answers, in order (0 once exhausted).
    faces: Vec<i64>,
    refuse: bool,
    token: u64,
    picks: Vec<Pick>,
    editing: bool,
    inside_box: bool,
    selected: Vec<u64>,
}

impl ToolHost for Fake {
    fn run(&mut self, cmd: &Command) -> Answer {
        self.log.push(format!("{} {}", cmd.name, cmd.args));
        if self.refuse && cmd.name == "add_edge" {
            return Answer::default();
        }
        self.token += 1;
        let face_delta = if cmd.name == "add_edge" && !self.faces.is_empty() { self.faces.remove(0) } else { 0 };
        Answer { ok: true, revision: self.token, face_delta, error: None }
    }
    fn revision(&mut self) -> u64 { self.token }
    fn has(&mut self, what: &str) -> bool { what == "linework" }
    fn query(&mut self, what: &str, _px: [f64; 2]) -> Vec<(Pick, f64)> {
        let kind = match what {
            "linework_edge" => PickKind::Edge,
            "group" => PickKind::Group,
            "face" => PickKind::Face,
            _ => return vec![],
        };
        self.picks.iter().filter(|p| p.kind == kind).map(|p| (*p, 1.0)).collect()
    }
    fn select(&mut self, ids: &[u64], mode: SelectMode, grow: Grow) {
        self.log.push(format!("select {ids:?} {} {}", mode.name(), grow.name()));
        if mode == SelectMode::Replace {
            self.selected = ids.to_vec();
        }
    }
    fn clear_selection(&mut self) {
        self.log.push("clear".into());
        self.selected.clear();
    }
    fn selection_signature(&mut self) -> u64 { self.selected.iter().sum::<u64>() + 1000 * self.selected.len() as u64 }
    fn selection_empty(&mut self) -> bool { self.selected.is_empty() }
    fn box_select(&mut self, rect: [f64; 4], crossing: bool, mode: SelectMode) {
        self.log.push(format!("box {rect:?} {crossing} {}", mode.name()));
    }
    fn editing(&mut self) -> bool { self.editing }
    fn edit_box_contains(&mut self, _px: [f64; 2]) -> bool { self.inside_box }
    fn open_placement(&mut self, id: u64) { self.log.push(format!("open {id}")); }
    fn close_placement(&mut self) { self.log.push("close".into()); }
    fn host_verb(&mut self, verb: &str, id: u64) { self.log.push(format!("{verb} {id}")); }
}

fn at(x: f64, y: f64, z: f64) -> Pointer { Pointer { world: Some([x, y, z]), ..Pointer::default() } }
fn px(x: f64, y: f64, mods: Mods, t: f64) -> Pointer { Pointer { px: [x, y], mods, t_ms: t, ..Pointer::default() } }
fn shift() -> Mods { Mods { shift: true, ..Mods::default() } }
fn ctrl() -> Mods { Mods { ctrl: true, ..Mods::default() } }

#[test]
fn line_chains_and_ends_on_a_face() {
    let mut t = line::LineTool::default();
    let mut h = Fake { faces: vec![0, 0, 1], ..Fake::default() };
    for p in [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 0., 0.]] {
        t.event(&ToolEvent::Click(at(p[0], p[1], p[2])), &mut h);
    }
    assert_eq!(h.log.iter().filter(|l| l.starts_with("add_edge")).count(), 3);
    assert!(!t.busy(), "the closing segment made a face and ended the chain");
}

#[test]
fn line_close_snap_ends_the_chain_and_start_click_draws_nothing() {
    let mut t = line::LineTool::default();
    let mut h = Fake::default();
    t.event(&ToolEvent::Click(at(0., 0., 0.)), &mut h);
    t.event(&ToolEvent::Click(at(0., 0., 0.)), &mut h);
    assert!(h.log.is_empty());
    let mut close = at(2., 0., 0.);
    close.snap = Some(Snap { point: [2., 0., 0.], kind: "close".into() });
    t.event(&ToolEvent::Click(close), &mut h);
    assert!(!t.busy());
}

#[test]
fn line_typed_length_runs_along_the_band_and_negative_away() {
    let mut t = line::LineTool::default();
    let mut h = Fake::default();
    t.event(&ToolEvent::Click(at(0., 0., 0.)), &mut h);
    t.event(&ToolEvent::Hover(at(0., 3., 0.)), &mut h);
    let r = t.event(&ToolEvent::Value(Value::Length(-2.0)), &mut h);
    assert!(r.committed);
    assert!(h.log[0].contains("\"b\":[0.0,-2.0,0.0]"), "{}", h.log[0]);
    assert_eq!(t.state()["start_point"], serde_json::json!([0.0, -2.0, 0.0]));
    // A pair means nothing to a line.
    assert!(!t.event(&ToolEvent::Value(Value::Pair(1., 2.)), &mut h).consumed);
}

#[test]
fn line_retype_undoes_and_redraws_from_the_same_start() {
    let mut t = line::LineTool::default();
    let mut h = Fake::default();
    t.event(&ToolEvent::Click(at(0., 0., 0.)), &mut h);
    t.event(&ToolEvent::Hover(at(1., 0., 0.)), &mut h);
    t.event(&ToolEvent::Click(at(1., 0., 0.)), &mut h);
    // The cursor still on the end: a value retypes the segment.
    t.event(&ToolEvent::Value(Value::Length(4.0)), &mut h);
    assert_eq!(h.log[1], "undo {}");
    assert!(h.log[2].contains("\"a\":[0.0,0.0,0.0]") && h.log[2].contains("\"b\":[4.0,0.0,0.0]"), "{:?}", h.log);
    // Again: still one segment's worth (undo + add).
    t.event(&ToolEvent::Value(Value::Length(5.0)), &mut h);
    assert_eq!(h.log[3], "undo {}");
    // Anything done in between closes the window.
    h.token += 7;
    t.event(&ToolEvent::Hover(at(5., 2., 0.)), &mut h);
    t.event(&ToolEvent::Value(Value::Length(1.0)), &mut h);
    assert!(h.log.last().unwrap().contains("\"a\":[5.0,0.0,0.0]"), "{:?}", h.log);
}

#[test]
fn line_undo_mid_chain_steps_back() {
    let mut t = line::LineTool::default();
    let mut h = Fake::default();
    for p in [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]] {
        t.event(&ToolEvent::Click(at(p[0], p[1], p[2])), &mut h);
    }
    assert!(t.event(&ToolEvent::Undo, &mut h).consumed);
    assert_eq!(t.state()["start_point"], serde_json::json!([1.0, 0.0, 0.0]));
    t.event(&ToolEvent::Undo, &mut h);
    t.event(&ToolEvent::Undo, &mut h);
    assert!(!t.busy());
    assert!(!t.event(&ToolEvent::Undo, &mut h).consumed, "idle: the ordinary undo runs");
}

#[test]
fn select_modes_follow_the_modifiers() {
    assert_eq!(SelectMode::from_mods(Mods::default()), SelectMode::Replace);
    assert_eq!(SelectMode::from_mods(shift()), SelectMode::Toggle);
    assert_eq!(SelectMode::from_mods(ctrl()), SelectMode::Add);
    assert_eq!(SelectMode::from_mods(Mods { shift: true, ctrl: true, ..Mods::default() }), SelectMode::Remove);
}

#[test]
fn select_press_release_is_a_click_and_a_drag_a_box() {
    let mut t = select::SelectTool::default();
    let face = Pick { id: 7, kind: PickKind::Face };
    let mut h = Fake { picks: vec![face], ..Fake::default() };
    t.event(&ToolEvent::Press(px(10., 10., Mods::default(), 0.)), &mut h);
    t.event(&ToolEvent::Release(px(11., 11., Mods::default(), 50.)), &mut h);
    assert_eq!(h.log, ["select [7] replace natural"]);
    t.event(&ToolEvent::Press(px(100., 10., Mods::default(), 1000.)), &mut h);
    t.event(&ToolEvent::Drag(px(40., 60., Mods::default(), 1010.)), &mut h);
    assert_eq!(t.preview().box_rect, Some(([100., 10., 40., 60.], true)));
    t.event(&ToolEvent::Release(px(40., 60., ctrl(), 1020.)), &mut h);
    assert_eq!(h.log[1], "box [40.0, 10.0, 100.0, 60.0] true add");
    assert_eq!(t.preview().box_rect, None);
}

#[test]
fn select_repeated_click_cycles_the_candidates() {
    let mut t = select::SelectTool::default();
    let picks = vec![Pick { id: 1, kind: PickKind::Edge }, Pick { id: 2, kind: PickKind::Group }];
    let mut h = Fake { picks, ..Fake::default() };
    for _ in 0..3 {
        t.event(&ToolEvent::Click(px(5., 5., Mods::default(), 0.)), &mut h);
    }
    assert_eq!(h.log, ["select [1] replace natural", "select [2] replace natural", "select [1] replace natural"]);
    // Leaving the spot ends the cycle.
    t.event(&ToolEvent::Hover(px(50., 5., Mods::default(), 0.)), &mut h);
    t.event(&ToolEvent::Click(px(5., 5., Mods::default(), 0.)), &mut h);
    assert_eq!(h.log.last().unwrap(), "select [1] replace natural");
}

#[test]
fn select_empty_click_leaves_the_edit_context_outside_its_box() {
    let mut t = select::SelectTool::default();
    let mut h = Fake { editing: true, ..Fake::default() };
    t.event(&ToolEvent::Click(px(5., 5., Mods::default(), 0.)), &mut h);
    assert_eq!(h.log, ["close"]);
    h.inside_box = true;
    t.event(&ToolEvent::Click(px(5., 5., Mods::default(), 0.)), &mut h);
    assert_eq!(h.log[1], "clear");
}

#[test]
fn select_double_opens_a_group_and_bounds_a_face_then_triple_connects() {
    let mut t = select::SelectTool::default();
    let mut h = Fake { picks: vec![Pick { id: 3, kind: PickKind::Group }], ..Fake::default() };
    t.event(&ToolEvent::DoubleClick(px(5., 5., Mods::default(), 0.)), &mut h);
    assert_eq!(h.log, ["open 3"]);
    h.picks = vec![Pick { id: 4, kind: PickKind::Face }];
    t.event(&ToolEvent::DoubleClick(px(5., 5., Mods::default(), 100.)), &mut h);
    assert_eq!(h.log[1], "select [4] replace bounded");
    // A press in place right after the double is the triple.
    t.event(&ToolEvent::Press(px(6., 5., Mods::default(), 300.)), &mut h);
    assert_eq!(h.log[2], "select [4] replace connected");
    // and its release is no click.
    t.event(&ToolEvent::Release(px(6., 5., Mods::default(), 320.)), &mut h);
    assert_eq!(h.log.len(), 3);
}

#[test]
fn select_cascade_takes_a_label_then_lines_then_groups_then_faces() {
    struct Layers(Vec<(&'static str, Pick)>);
    impl ToolHost for Layers {
        fn run(&mut self, _c: &Command) -> Answer { Answer::default() }
        fn has(&mut self, what: &str) -> bool { what == "linework" }
        fn query(&mut self, what: &str, _px: [f64; 2]) -> Vec<(Pick, f64)> {
            self.0.iter().filter(|(w, _)| *w == what).map(|(_, p)| (*p, 2.0)).collect()
        }
        fn image_in_front(&mut self, face: u64, _px: [f64; 2]) -> Option<Pick> {
            (face == 9).then_some(Pick { id: 10, kind: PickKind::Other })
        }
    }
    let face = Pick { id: 9, kind: PickKind::Face };
    let group = Pick { id: 5, kind: PickKind::Group };
    let line = Pick { id: 6, kind: PickKind::Group };
    let mut h = Layers(vec![("face", face), ("group", group)]);
    assert_eq!(select::pick(&mut h, [0.0, 0.0], true), Some(group));
    h.0.push(("linework_groups", line));
    assert_eq!(select::pick(&mut h, [0.0, 0.0], true), Some(line));
    assert_eq!(select::pick(&mut h, [0.0, 0.0], false), Some(group));
    assert_eq!(select::candidates(&mut h, [0.0, 0.0]).iter().map(|p| p.id).collect::<Vec<_>>(), vec![6, 5, 10]);
    // A picture in front of the face takes the click (#426).
    let mut h = Layers(vec![("face", face)]);
    assert_eq!(select::pick(&mut h, [0.0, 0.0], true).map(|p| p.id), Some(10));
}

#[test]
fn circle_retypes_in_place_and_sides_before_the_centre() {
    let mut t = circle::RadialTool::polygon();
    let mut h = Fake::default();
    // A number before the centre is the side count.
    assert!(t.event(&ToolEvent::Value(Value::Length(8.0)), &mut h).consumed);
    assert!(!t.event(&ToolEvent::Value(Value::Length(2.0)), &mut h).consumed);
    assert_eq!(t.state()["sides"], 8);
    t.event(&ToolEvent::Click(at(0., 0., 0.)), &mut h);
    t.event(&ToolEvent::Hover(at(1., 0., 0.)), &mut h);
    t.event(&ToolEvent::Click(at(1., 0., 0.)), &mut h);
    assert_eq!(h.log.len(), 1);
    assert!(h.log[0].starts_with("add_face") && h.log[0].contains("\"curve\":true"));
    // Right after: «5s» and a radius redraw it, an undo before each.
    assert!(t.event(&ToolEvent::Segments(5), &mut h).consumed);
    assert!(t.event(&ToolEvent::Value(Value::Length(2.0)), &mut h).committed);
    let names: Vec<&str> = h.log.iter().map(|l| l.split(' ').next().unwrap()).collect();
    assert_eq!(names, ["add_face", "undo", "add_face", "undo", "add_face"]);
    // Anything in between closes the window: a number is a count again.
    h.token += 1;
    assert!(t.event(&ToolEvent::Value(Value::Length(4.0)), &mut h).consumed);
    assert_eq!(t.state()["sides"], 4);
    assert_eq!(h.log.len(), 5);
}

#[test]
fn circle_ctrl_draws_a_guide_and_a_chord_takes_it_back() {
    let mut t = circle::RadialTool::circle();
    let mut h = Fake::default();
    t.event(&ToolEvent::Key { key: "Control".into(), mods: ctrl() }, &mut h);
    t.event(&ToolEvent::KeyRelease { key: "Control".into(), tapped: false }, &mut h);
    assert_eq!(t.state()["guide_mode"], false);
    t.event(&ToolEvent::Key { key: "Control".into(), mods: ctrl() }, &mut h);
    t.event(&ToolEvent::KeyRelease { key: "Control".into(), tapped: true }, &mut h);
    t.event(&ToolEvent::Click(at(0., 0., 0.)), &mut h);
    t.event(&ToolEvent::Hover(at(0., 2., 0.)), &mut h);
    assert!(t.preview().lines.is_empty(), "the guide is dashed, not a rubber band");
    assert_eq!(t.state()["guide_ring"].as_array().unwrap().len(), 24);
    t.event(&ToolEvent::Click(at(0., 2., 0.)), &mut h);
    assert!(h.log[0].starts_with("add_guide_circle") && h.log[0].contains("\"radius\":2.0"));
    assert!(!t.busy());
}

#[test]
fn scale_retype_runs_protocol_commands_and_expires_after_another_edit() {
    #[derive(Default)]
    struct ScaleHost(Fake);
    impl ToolHost for ScaleHost {
        fn call(&mut self, name: &str, _: serde_json::Value) -> serde_json::Value {
            match name {
                "pick_grip" => true.into(),
                "grip_spec" => serde_json::json!({
                    "extents": [4, 3, 0], "mask": [true, false, false], "uniform": false
                }),
                "scale_targets" => serde_json::json!({"vertices": ["42"], "anchor": [0, 0, 0]}),
                _ => serde_json::Value::Null,
            }
        }
        fn revision(&mut self) -> u64 { self.0.token }
        fn run(&mut self, cmd: &Command) -> Answer { self.0.run(cmd) }
    }

    let mut host = ScaleHost::default();
    let mut tool = new_tool("scale").unwrap();
    tool.event(&ToolEvent::Click(Pointer::default()), &mut host);
    assert!(tool.busy());
    assert!(tool.event(&ToolEvent::AbsoluteValue(Value::Length(8.0)), &mut host).committed);
    assert!(!tool.busy());
    assert!(host.0.log[0].contains("\"factors\":[2.0,1.0,1.0]"));
    assert!(tool.event(&ToolEvent::Value(Value::Length(-1.0)), &mut host).committed);
    assert_eq!(host.0.log.iter().map(|l| l.split_whitespace().next().unwrap()).collect::<Vec<_>>(),
               ["scale_selection", "undo", "scale_selection"]);
    host.0.token += 1;
    assert!(!tool.event(&ToolEvent::Value(Value::Length(3.0)), &mut host).consumed);
    assert_eq!(host.0.log.len(), 3);
}
