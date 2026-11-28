use super::{label, send};
use crate::ui::{kit, tokens};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Records {
    pub(super) editor: super::record_editor::Editor,
    pub rows: BTreeMap<Vec<String>, Row>,
    sweep:Value,
    cuts:Value,
    junction:Value,
    repeat:Value,
    sweep_editable:bool,
    total: usize,
    writable: Vec<String>,
    set_name: String,
    property_name: String,
    property_value: String,
    property_kind: String,
    error: String,
}
#[derive(Default)]
pub(super) struct Row {
    value: Value,
    mixed: bool,
    count: usize,
    owner: String,
    targets: Vec<String>,
    draft: String,
    editing: bool,
    kind: String,
}

impl Records {
    pub fn new(data: &Value, selection_count: usize) -> Self {
        let mut result = Self::default();
        for object in data.as_array().into_iter().flatten() {
            if selection_count==1{
                result.sweep=object["sweep"].clone();
                result.repeat=object["repeat"].clone();
                result.cuts=object["cut_rows"].clone();
                result.junction=object["junction"].clone();
                result.sweep_editable=object["editable"]==true;
            }
            result.total += 1;
            if object["editable"] == true {
                result.writable.push(label(&object["uid"]).into());
            }
            for f in object["fields"].as_array().into_iter().flatten() {
                let path = f["path"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| label(v).to_owned())
                    .collect();
                let r = result.rows.entry(path).or_insert_with(|| Row {
                    value: f["value"].clone(),
                    owner: label(&f["owner"]).into(),
                    kind: label(&f["kind"]).into(),
                    ..Default::default()
                });
                r.count += 1;
                r.mixed |= r.value != f["value"];
                if r.owner != label(&f["owner"]) {
                    r.owner = "Mixed sources".into();
                }
                if f["editable"] == true && r.kind == label(&f["kind"]) {
                    r.targets.push(label(&object["uid"]).into());
                }
            }
        }
        result.total = result.total.max(selection_count);
        for r in result.rows.values_mut() {
            r.mixed |= r.count != result.total;
            r.draft = if r.mixed {
                String::new()
            } else {
                r.value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| r.value.to_string())
            };
        }
        result
    }

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        out: &mut Vec<String>,
        layers: &mut Vec<egui::Id>,
        measured: &[(String, String)],
    ) {
        if self.repeat.is_object() {
            let row = &self.repeat;
            kit::disclosure(ui, "repeat record", "Repeat", true, |ui| {
                if let Some(names) = row["breadcrumb"].as_array().filter(|names| names.len() > 1) {
                    ui.label(
                        names
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" › "),
                    );
                }
                if let Some(name) = row["repeat_preset"]["name"].as_str() {
                    kit::property_control(ui, "Preset", |ui| {
                        ui.label(name);
                    });
                }
                ui.label(format!(
                    "{} copies · {} distinct parts",
                    row["copies"], row["distinct"]
                ));
                ui.add_enabled_ui(self.sweep_editable, |ui| {
                    let rules = ["count", "spacing", "divide", "fit_max"];
                    let names = ["Count", "Spacing", "Divide", "Fit ≤"];
                    let current = rules
                        .iter()
                        .position(|k| row["rule"][*k].is_number())
                        .unwrap_or(0);
                    kit::property_control(ui, "Rule", |ui| {
                        let (choice, popup) =
                            kit::select(ui, "repeat rule", names[current], &names);
                        layers.extend(popup);
                        if let Some(i) = choice {
                            let mut next = row.clone();
                            next["rule"] = json!({(rules[i]): if i%2==0{4}else{600}});
                            send(out, "repeat.edit", next);
                        }
                    });
                    let key = rules[current];
                    let old = row["rule"][key].as_f64().unwrap_or(1.);
                    kit::property_control(ui, names[current], |ui| {
                        let id = ui.make_persistent_id(("repeat value", row["uid"].as_str(), key));
                        let mut n = ui.data(|d| d.get_temp(id)).unwrap_or(old);
                        let r = ui.add(kit::number(
                            &mut n,
                            if current % 2 == 0 {
                                ""
                            } else if row["run"]["polar"].is_object() {
                                "°"
                            } else {
                                " mm"
                            },
                        ));
                        if r.changed() {
                            ui.data_mut(|d| d.insert_temp(id, n));
                        }
                        if r.drag_stopped() || r.lost_focus() {
                            ui.data_mut(|d| d.remove::<f64>(id));
                            if old != n && n > 0. {
                                let mut next = row.clone();
                                next["rule"][key] = if current % 2 == 0 {
                                    json!(n.round() as u64)
                                } else {
                                    json!(n)
                                };
                                send(out, "repeat.edit", next);
                            }
                        }
                    });
                    for (field, title, options) in [
                        ("adapt", "Copies", &["auto", "identical", "adaptive"][..]),
                        (
                            "orientation",
                            "Orientation",
                            &["fixed", "follow", "alternate"][..],
                        ),
                    ] {
                        if field == "adapt" && row["can_adapt"] != true {
                            continue;
                        }
                        kit::property_control(ui, title, |ui| {
                            let (choice, popup) = kit::select(
                                ui,
                                field,
                                row[field].as_str().unwrap_or(
                                    if field == "orientation" && row["run"]["polar"].is_object() {
                                        "follow"
                                    } else {
                                        options[0]
                                    },
                                ),
                                options,
                            );
                            layers.extend(popup);
                            if let Some(i) = choice {
                                let mut next = row.clone();
                                next[field] = json!(options[i]);
                                send(out, "repeat.edit", next);
                            }
                        });
                    }
                    if let Some(hosts) = row["between_options"].as_array().filter(|h| !h.is_empty())
                    {
                        let names: Vec<_> = hosts
                            .iter()
                            .map(|h| h[1].as_str().unwrap_or("Repeat"))
                            .collect();
                        kit::property_control(ui, "Between", |ui| {
                            let current = hosts
                                .iter()
                                .position(|h| h[0] == row["run"]["between"])
                                .map(|i| names[i])
                                .unwrap_or("Choose Repeat…");
                            let (choice, popup) =
                                kit::select(ui, "between repeat", current, &names);
                            layers.extend(popup);
                            if let Some(i) = choice {
                                let mut next = row.clone();
                                next["run"] = json!({"between":hosts[i][0]});
                                send(out, "repeat.edit", next);
                            }
                        });
                    }
                    if kit::quiet(ui, "Edit source").clicked() {
                        send(out, "repeat.source", row["source_path"].clone());
                    }
                    if kit::quiet(ui, "Edit run").clicked() {
                        send(out, "repeat.run", row.clone());
                    }
                    if kit::quiet(ui, "Release").clicked() {
                        send(out, "repeat.release", row["uid"].clone());
                    }
                    kit::disclosure(ui, "repeat ends", "Ends and skips", false, |ui| {
                        let polar = row["run"]["polar"].is_object();
                        let measure = row["measure"].as_str().unwrap_or("pitch");
                        if !polar {
                            kit::property_control(ui, "Measure", |ui| {
                                let options = ["pitch", "gap"];
                                let (choice, popup) = kit::select(ui, "repeat measure", measure, &options);
                                layers.extend(popup);
                                if let Some(i) = choice {
                                    let mut next = row.clone();
                                    next["measure"] = json!(options[i]);
                                    send(out, "repeat.edit", next);
                                }
                            });
                        }
                        for (key, title, flag, default) in [
                            ("start", "Start inset", "at_start", false),
                            ("end", "End inset", "at_end", true),
                        ] {
                            kit::property_control(ui, title, |ui| {
                                let old = row["ends"][key].as_f64().unwrap_or(0.);
                                let id = ui.make_persistent_id(("repeat inset", row["uid"].as_str(), key));
                                let mut n = ui.data(|d| d.get_temp(id)).unwrap_or(old);
                                let response = ui.add(kit::number(&mut n, if polar { "°" } else { " mm" }));
                                if response.changed() {
                                    ui.data_mut(|d| d.insert_temp(id, n));
                                }
                                if response.drag_stopped() || response.lost_focus() {
                                    ui.data_mut(|d| d.remove::<f64>(id));
                                    if n >= 0. && n != old {
                                        let mut next = row.clone();
                                        next["ends"][key] = json!(n);
                                        send(out, "repeat.edit", next);
                                    }
                                }
                            });
                            let mut include = row["ends"][flag].as_bool().unwrap_or(default);
                            if ui.checkbox(&mut include, if default { "Include end" } else { "Include start" }).changed() {
                                let mut next = row.clone();
                                next["ends"][flag] = json!(include);
                                send(out, "repeat.edit", next);
                            }
                        }
                        for (index, skip) in row["skips"].as_array().into_iter().flatten().enumerate() {
                            ui.horizontal(|ui| {
                                let at = skip["at"].as_f64().unwrap_or_default();
                                let unit = if polar { "°" } else { " mm" };
                                let from = skip["from"].as_str().unwrap_or("start");
                                ui.label(format!("{at:.1}{unit} from {from}"));
                                if kit::quiet(ui, "Restore copy").clicked() {
                                    let mut next = row.clone();
                                    next["skips"].as_array_mut().unwrap().remove(index);
                                    send(out, "repeat.edit", next);
                                }
                            });
                        }
                    });
                });
            });
        }
        if let Some(row)=self.sweep.as_object(){
            kit::disclosure(ui,"sweep record","Sweep",true,|ui|{
                let property = |ui: &mut egui::Ui, name: &str, value: &str| {
                    kit::property_control(ui, name, |ui| { ui.label(value); });
                };
                let owner=&self.sweep["owner"];
                ui.add_enabled_ui(self.sweep_editable,|ui|{
                    for (key,title,display) in [("preset","Preset","preset"),("profile","Profile","profile_name")]{
                        let entries:Vec<_>=self.sweep[format!("{key}_options")].as_array().into_iter().flatten()
                            .filter(|p|key!="preset" || p["preset"]["primitive"]=="sweep").collect();
                        let names:Vec<_>=entries.iter().map(|p|label(&p["name"])).collect();
                        kit::property_control(ui,title,|ui|{
                            let (choice,popup)=kit::select(ui,key,self.sweep[display].as_str().unwrap_or("Choose…"),&names);
                            layers.extend(popup);
                            if let Some(i)=choice{
                                let p=entries[i];
                                let val=if key=="preset"{p["preset"].clone()}else if let Some(d)=p.get("sketch"){
                                    if p["key"].as_str().is_some_and(|k|k.starts_with("sketch:")){json!({"sketch":d["uid"]})}else{json!({"drawing":d})}
                                }else{json!({"library":p["key"]})};
                                send(out,"sweep.field",json!([owner,key,val]));
                            }
                        });
                    }
                    kit::property_control(ui,"Attachment",|ui|{
                        let names=["centre","bottom","bottom_left","left","top_left","top","top_right","right","bottom_right"];
                        let (choice,popup)=kit::select(ui,"sweep attachment",label(&self.sweep["attachment"]),&names);
                        layers.extend(popup);
                        if let Some(i)=choice{send(out,"sweep.field",json!([owner,"attachment",names[i]]));}
                    });
                    kit::property_control(ui,"Rotation",|ui|{
                        let id=ui.make_persistent_id(("sweep rotation",owner.as_str()));
                        let old=self.sweep["rotation"].as_f64().unwrap_or(0.);
                        let mut value=ui.data(|d|d.get_temp(id)).unwrap_or(old);
                        let response=ui.add(kit::number(&mut value,"°"));
                        if response.changed(){ui.data_mut(|d|d.insert_temp(id,value));}
                        if response.drag_stopped() || response.lost_focus() || (response.changed() && !response.has_focus() && !response.dragged()){
                            ui.data_mut(|d|d.remove::<f64>(id));
                            if value!=old{send(out,"sweep.field",json!([owner,"rotation",value]));}
                        }
                    });
                    kit::property_control(ui,"Mirror",|ui|{
                        let mut on=self.sweep["mirror"]==true;
                        if kit::toggle(ui,&mut on,"Mirror section").clicked(){send(out,"sweep.field",json!([owner,"mirror",on]));}
                    });
                });
                if let Some(state)=row.get("state").and_then(Value::as_str){property(ui,"State",state);}
                if let Some(path) = self.sweep["resolved_points"].as_array() {
                    let count = path.len().saturating_sub(1) + usize::from(self.sweep["path"]["closed"] == true);
                    property(ui, "Path", &format!("{count} {}", if count == 1 { "span" } else { "spans" }));
                    if path.len()>2{
                        let current=usize::from(self.sweep["corner"]=="butt");
                        let mut selected=current;
                        ui.add_enabled_ui(self.sweep_editable,|ui|ui.horizontal(|ui|{
                            ui.label("Corner");
                            kit::choice(ui,"sweep corner",&mut selected,&["Mitre","Butt"]);
                        }));
                        if current!=selected{send(out,if selected==1{"sweep.butt"}else{"sweep.mitre"},self.sweep["owner"].clone());}
                    }
                }
                ui.add_enabled_ui(self.sweep_editable,|ui|ui.horizontal(|ui|{
                    if self.sweep["path"]["points"].is_array() && kit::quiet(ui,"Edit path").clicked(){
                        send(out,"sweep.path",self.sweep["owner"].clone());
                    }
                    if kit::quiet(ui,"Release").clicked(){send(out,"sweep.release",self.sweep["owner"].clone());}
                }));
            });
        }
        if self.junction.is_object() {
            self.junction_record(ui, out);
        }
        if let Some(cuts)=self.cuts.as_array().filter(|v|!v.is_empty()){
            kit::disclosure(ui,"cut record","Cuts",true,|ui|{
                for cut in cuts{
                    let kind=cut["cutter"].as_object().and_then(|c|c.keys().next()).map(String::as_str).unwrap_or("Cut");
                    let title=format!("{} · {kind}",cut["name"].as_str().or(cut["operation"].as_str()).unwrap_or("subtract"));
                    kit::property_control(ui,&title,|ui|{
                        let on=cut["on"]!=false;
                        ui.add_enabled_ui(self.sweep_editable,|ui|{
                            let (rect,eye)=ui.allocate_exact_size(egui::vec2(20.,20.),egui::Sense::click());
                            kit::paint_eye(ui,rect,on);
                            if eye.on_hover_text("Enable or disable cut").clicked(){
                                let mut edited=cut.clone();edited["on"]=json!(!on);
                                send(out,"cut.edit",edited);
                            }
                        });
                        ui.add_enabled_ui(self.sweep_editable,|ui|{
                            if kit::quiet(ui,"Release").on_hover_text("Bake through this cut").clicked(){send(out,"cut.release",cut["uid"].clone());}
                        });
                        if let Some(state)=cut["state"].as_str().filter(|s|*s!="ok"){ui.label(state);}
                    });
                }
            });
        }
        kit::disclosure(ui, "record identity", "Identity", true, |ui| {
            self.editor.identity(ui, out, layers)
        });
        let ready = self.editor.ready();
        kit::disclosure(ui, "record psets", "Property sets", false, |ui| {
            ui.add_enabled_ui(ready, |ui| self.properties(ui, out))
        });
        kit::disclosure(ui, "record quantities", "Quantities", false, |ui| {
            for (unit, total) in measured {
                kit::prop_row(ui, &format!("Measured · {unit}"), total);
            }
            ui.add_enabled_ui(ready && !self.writable.is_empty(), |ui| {
                self.editor.quantities(ui, out)
            })
        });
        kit::disclosure(
            ui,
            "record classifications",
            "Classifications",
            false,
            |ui| {
                ui.add_enabled_ui(ready && !self.writable.is_empty(), |ui| {
                    self.editor.classifications(ui, out)
                })
            },
        );
    }
    /// The junction record (docs/junctions.md §6.1, §11): status with its
    /// ways out, roles with Swap, "All N · This one", parameters with their
    /// relation and amber overrides, parts, and the use's actions.
    fn junction_record(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        let j = self.junction.clone();
        let uid = j["uid"].clone();
        let uses = j["uses"].as_u64().unwrap_or(0);
        let editable = self.sweep_editable || !self.writable.is_empty();
        kit::disclosure(ui, "junction record", "Junction", true, |ui| {
            let name = j["detail"]["name"].as_str().unwrap_or("Custom junction");
            kit::inspector_title(ui, name, &if j["detail"].is_null() { "Custom".to_string() } else if uses == 1 { "1 use".to_string() } else { format!("{uses} uses") });
            let (status, tone) = match j["state"].as_str().unwrap_or("") {
                "ok" if j["forced"] == true => ("custom fit · outside its rule".to_string(), tokens::WARN),
                "ok" => ("fits".to_string(), tokens::OK),
                "outside" => (format!("outside: {} {}", j["reason"]["criterion"].as_str().unwrap_or(""), j["reason"]["value"].as_f64().map(|v| format!("{:.1}", v)).unwrap_or_default()), tokens::WARN),
                "failed" => (format!("failed: {}", j["reason"]["criterion"].as_str().or(j["reason"]["message"].as_str()).unwrap_or("")), tokens::WARN),
                "lost" => ("lost: a member is gone".to_string(), tokens::WARN),
                other => (other.to_string(), tokens::DIM),
            };
            kit::property_control(ui, "Status", |ui| kit::status_chip(ui, &status, tone));
            if j["state"] == "outside" {
                ui.horizontal(|ui| {
                    if kit::quiet(ui, "Widen the detail").clicked() { send(out, "junction.widen", uid.clone()); }
                    if kit::quiet(ui, "Keep as custom").clicked() { send(out, "junction.detach", uid.clone()); }
                });
            }
            if let Some(parent) = j["parent"].as_object() {
                kit::prop_row(ui, "Inside", parent.get("name").and_then(Value::as_str).unwrap_or("junction"));
            }
            if j["stations"].is_number() {
                kit::prop_row(ui, "Along the Repeat", &format!("{} meetings", j["stations"]));
            }
            for (i, role) in j["roles"].as_array().into_iter().flatten().enumerate() {
                let label = match role["role"].as_str().unwrap_or("") { "main" => "Main", "joining" => "Joining", r => r };
                kit::property_control(ui, label, |ui| {
                    ui.label(egui::RichText::new(format!("{} · {} · {}", role["name"].as_str().unwrap_or(""), role["profile"].as_str().unwrap_or(""), role["how"].as_str().unwrap_or(""))).font(tokens::meta()).color(tokens::INK_2));
                    if i == 0 && editable && kit::row_link(ui, "Swap").clicked() { send(out, "junction.swap", uid.clone()); }
                });
            }
            let id = ui.make_persistent_id(("junction scope", uid.as_str()));
            let mut this_one = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
            if !j["detail"].is_null() && uses > 1 {
                let all = format!("All {uses}");
                if let Some(i) = kit::segmented(ui, &[all.as_str(), "This one"], usize::from(this_one), ui.available_width().min(220.)) {
                    this_one = i == 1;
                    ui.data_mut(|d| d.insert_temp(id, this_one));
                }
            }
            let scope = if this_one || j["detail"].is_null() { "one" } else { "all" };
            for p in j["params"].as_array().into_iter().flatten() {
                let key = p["key"].as_str().unwrap_or("");
                let overridden = !p["override"].is_null();
                let label = format!("{}{}", if overridden { "● " } else { "" }, p["name"].as_str().unwrap_or(key));
                kit::property_control(ui, &label, |ui| {
                    let mut v = p["value"].as_f64().unwrap_or(0.);
                    let before = v;
                    ui.add_enabled_ui(editable && p["derived"] != true, |ui| {
                        let r = ui.add(kit::number(&mut v, p["unit"].as_str().unwrap_or("mm")));
                        if (r.lost_focus() || r.drag_stopped()) && (v - before).abs() > 1e-9 {
                            send(out, "junction.param", json!({"uid": uid, "key": key, "value": v, "scope": scope}));
                        }
                    });
                    if let Some(rel) = p["relation"].as_str().filter(|r| !r.is_empty() && !overridden) {
                        ui.label(egui::RichText::new(rel).font(tokens::meta()).color(tokens::ACCENT));
                    }
                    if overridden {
                        ui.label(egui::RichText::new("this one").font(tokens::meta()).color(tokens::WARN));
                    } else if p["overridden_uses"].as_u64().unwrap_or(0) > 0 && scope == "all" {
                        ui.label(egui::RichText::new(format!("{} own", p["overridden_uses"])).font(tokens::meta()).color(tokens::DIM));
                    }
                });
            }
            let contents = j["contents"].as_array().cloned().unwrap_or_default();
            if !contents.is_empty() {
                kit::caption(ui, &format!("Parts · {}", contents.len()));
                for c in &contents {
                    let class = c["class"].as_str().unwrap_or("");
                    kit::prop_row(ui, &format!("{}{}", c["label"].as_str().unwrap_or(""), if c["local"] == true { " · this one" } else { "" }), if class == "cut" { "Cut" } else { class });
                }
            }
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if kit::quiet(ui, "Edit inside").on_hover_text("Double-click the junction marker or its row").clicked() { send(out, "junction.edit", uid.clone()); }
                    if uses > 1 && kit::quiet(ui, &format!("Select all {uses}")).clicked() { send(out, "junction.select_all", j["detail"]["uid"].clone()); }
                    if !j["detail"].is_null() && kit::quiet(ui, "Similar…").clicked() { send(out, "junction.review", j["detail"]["uid"].clone()); }
                    if j["overrides"].as_array().is_some_and(|o| !o.is_empty()) {
                        if kit::quiet(ui, "Upgrade to detail").on_hover_text("This use's values become the detail's").clicked() { send(out, "junction.upgrade", uid.clone()); }
                        if kit::quiet(ui, "Reset").on_hover_text("Drop this use's own values").clicked() { send(out, "junction.reset", uid.clone()); }
                    }
                    if !j["detail"].is_null() && kit::quiet(ui, "Detach").on_hover_text("Keep it as a custom junction").clicked() { send(out, "junction.detach", uid.clone()); }
                    if kit::quiet(ui, "Release").on_hover_text("One level: its cuts stay live, its parts stay").clicked() { send(out, "junction.release", uid.clone()); }
                    if kit::quiet(ui, "Release all").on_hover_text("Bake its cuts and parts too").clicked() { send(out, "junction.release_all", uid.clone()); }
                });
            });
        });
    }

    fn properties(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        for (path, r) in self
            .rows
            .iter_mut()
            .filter(|(path, _)| path.first().is_some_and(|s| s == "psets"))
        {
            ui.push_id(path, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let title = path[1..].join(" › ");
                    ui.strong(title);
                    ui.add_space(tokens::SPACE_S);
                    ui.weak(&r.owner);
                    ui.add_space(tokens::SPACE_XS);
                    if path[0] == "psets"
                        && r.owner == "This object"
                        && kit::quiet(ui, "Reset")
                            .on_hover_text("Restore inherited value")
                            .clicked()
                    {
                        send(out, "record.patch", json!([r.targets, path, Value::Null]));
                    }
                });
                if r.targets.is_empty() {
                    ui.label(if r.mixed { "Mixed" } else { &r.draft });
                    return;
                }
                let response = ui.add(
                    kit::field(&mut r.draft)
                        .desired_width(ui.available_width())
                        .hint_text(if r.mixed { "Mixed" } else { "Enter value" }),
                );
                r.editing |= response.changed();
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    r.draft = if r.mixed {
                        String::new()
                    } else {
                        r.value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| r.value.to_string())
                    };
                    r.editing = false;
                }
                if r.editing
                    && response.lost_focus()
                    && ui
                        .input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Tab))
                {
                    let val = match r.kind.as_str() {
                        "text" | "label" | "enum" => Some(json!(r.draft)),
                        "boolean" => r.draft.parse::<bool>().ok().map(Value::Bool),
                        "integer" => r.draft.parse::<i64>().ok().map(Value::from),
                        _ => r
                            .draft
                            .parse::<f64>()
                            .ok()
                            .filter(|n| n.is_finite())
                            .map(Value::from),
                    };
                    if let Some(v) = val {
                        r.editing = false;
                        self.error.clear();
                        let targets = if r.targets.len() == r.count {
                            &self.writable
                        } else {
                            &r.targets
                        };
                        send(out, "record.patch", json!([targets, path, v, r.kind]));
                    } else {
                        self.error = format!("Enter a {} value", r.kind);
                        response.request_focus();
                    }
                }
            });
        }
        if !self.error.is_empty() {
            ui.label(&self.error);
        }
        if !self.writable.is_empty() {
            kit::disclosure(ui, "add property", "Add property", false, |ui| {
                ui.add(kit::field(&mut self.set_name).hint_text("Property set"));
                ui.add(kit::field(&mut self.property_name).hint_text("Property name"));
                let kinds = [
                    "Automatic",
                    "text",
                    "label",
                    "enum",
                    "boolean",
                    "integer",
                    "real",
                    "length",
                    "area",
                    "volume",
                ];
                kit::property_control(ui, "Property kind", |ui| {
                    let value = if self.property_kind.is_empty() {
                        kinds[0]
                    } else {
                        &self.property_kind
                    };
                    if let (Some(index), _) = kit::select(ui, "property kind", value, &kinds) {
                        self.property_kind = if index == 0 {
                            String::new()
                        } else {
                            kinds[index].into()
                        };
                    }
                });
                ui.add(
                    kit::field(&mut self.property_value).hint_text("Text, number, or true / false"),
                );
                if ui
                    .add_enabled_ui(
                        !self.set_name.trim().is_empty() && !self.property_name.trim().is_empty(),
                        |ui| kit::quiet(ui, "Add property"),
                    )
                    .inner
                    .clicked()
                {
                    let path = vec![
                        "psets".to_string(),
                        self.set_name.trim().into(),
                        self.property_name.trim().into(),
                    ];
                    if self.rows.contains_key(&path) {
                        self.error = "This property already has a row above".into();
                    } else {
                        let raw = &self.property_value;
                        let (kind, value) = match serde_json::from_str::<Value>(raw) {
                            Ok(v @ Value::Bool(_)) => ("boolean", v),
                            Ok(v @ Value::Number(_)) if v.is_i64() => ("integer", v),
                            Ok(v @ Value::Number(_)) => ("real", v),
                            _ => ("text", json!(raw)),
                        };
                        self.error.clear();
                        let (kind, value) = match self.property_kind.as_str() {
                            "" => (kind, value),
                            k @ ("text" | "label" | "enum") => (k, json!(raw)),
                            k @ ("real" | "length" | "area" | "volume") if value.is_number() => {
                                (k, value)
                            }
                            k if k == kind => (k, value),
                            k => {
                                self.error = format!("Enter a {k} value");
                                return;
                            }
                        };
                        send(
                            out,
                            "record.patch",
                            json!([self.writable, path, value, kind]),
                        );
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_patch_excludes_linked_and_absent_fields() {
        let field = |value, editable| json!({"path":["psets","Survey","Checked"],"value":value,"owner":"This object","kind":"boolean","editable":editable});
        let data = json!([{"uid":"a","fields":[field(true,true)]},{"uid":"b","fields":[field(false,true)]},{"uid":"link","fields":[field(true,false)]},{"uid":"wall","fields":[]}]);
        let vm = Records::new(&data, 4);
        let row = vm.rows.values().next().unwrap();
        assert!(row.mixed);
        assert_eq!(vm.total, 4);
        assert_eq!(row.targets, ["a", "b"]);
    }
}
