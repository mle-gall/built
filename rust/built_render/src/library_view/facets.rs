//! Component facet counts omit the facet being counted. Search, side toggles and
//! removable chips use the same tokens, so their combinations stay reversible.
use super::*;
use std::collections::BTreeMap;

pub(crate) fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

pub(crate) fn toggle(text: &str, token: &str) -> String {
    let mut tokens = words(text);
    if token.eq_ignore_ascii_case("-source:linked") {
        tokens.retain(|t| Term::parse(t).field != "file");
    }
    if tokens.iter().any(|t| t.eq_ignore_ascii_case(token)) {
        tokens.retain(|t| !t.eq_ignore_ascii_case(token));
    } else {
        tokens.push(token.to_owned());
    }
    tokens
        .into_iter()
        .map(|t| {
            if t.contains(char::is_whitespace) {
                format!("\"{t}\"")
            } else {
                t
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn classes(e: &Item) -> impl Iterator<Item = &str> {
    e.classes
        .iter()
        .map(String::as_str)
        .chain(e.classes.is_empty().then_some("Unclassified"))
}

pub(super) fn class(e: &Item) -> &str {
    if !e.explicit_class.is_empty() {
        return &e.explicit_class;
    }
    match e.classes.as_slice() {
        [] => "Unclassified",
        [c] => c,
        _ => "Multiple classes",
    }
}

fn files(e: &Item) -> impl Iterator<Item = &str> {
    e.files.iter().map(String::as_str).chain(
        e.file
            .as_deref()
            .filter(|_| e.files.is_empty() && e.source == "linked"),
    )
}

fn collection(e: &Item) -> &str {
    if !e.collection.is_empty() {
        &e.collection
    } else if e.source == "bundled" {
        &e.category
    } else {
        ""
    }
}
fn bucket(e: &Item) -> &str {
    if e.source.starts_with("collection:") {
        "mine"
    } else {
        &e.source
    }
}

pub(super) fn source(e: &Item) -> &str {
    match e.source.as_str() {
        "model" => "This model",
        "linked" => e
            .file
            .as_deref()
            .and_then(|p| p.rsplit(['/', '\\']).next())
            .unwrap_or("Linked files"),
        "bundled" => "Bundled",
        _ => "My library",
    }
}

struct Term {
    field: &'static str,
    value: String,
    negative: bool,
}
impl Term {
    fn parse(word: &str) -> Self {
        let negative = word.starts_with('-');
        let raw = word.strip_prefix('-').unwrap_or(word).to_lowercase();
        let (field, value) = match raw.split_once(':') {
            Some(("class", v)) => ("class", v),
            Some(("source", v)) => ("source", v),
            Some(("file", v)) => ("file", v),
            Some(("collection", v)) => ("collection", v),
            Some(("is", v @ ("used" | "unused"))) => ("usage", v),
            _ if matches!(raw.as_str(), "used" | "unused") => ("usage", raw.as_str()),
            _ => ("name", raw.as_str()),
        };
        Self {
            field,
            value: value.to_owned(),
            negative,
        }
    }
    fn matches(&self, e: &Item) -> bool {
        match self.field {
            "class" => classes(e).any(|c| c.eq_ignore_ascii_case(&self.value)),
            "source" => {
                e.source.eq_ignore_ascii_case(&self.value)
                    || (self.value == "mine" && e.source.starts_with("collection:"))
            }
            "file" => files(e).any(|f| f.to_lowercase().contains(&self.value)),
            "collection" => collection(e).eq_ignore_ascii_case(&self.value),
            "usage" => {
                matches!(e.source.as_str(), "linked" | "model")
                    && (e.uses > 0) == (self.value == "used")
            }
            _ => e.shown().to_lowercase().contains(&self.value),
        }
    }
}

pub(super) struct Index<'a> {
    pub side: Vec<Value>,
    pub groups: Vec<(String, String, Vec<&'a Item>)>,
    pub tokens: Vec<String>,
}

pub(super) fn index<'a>(items: &'a [Item], st: &Browse) -> Index<'a> {
    let tokens = words(&st.filter);
    let terms: Vec<_> = tokens.iter().map(|t| Term::parse(t)).collect();
    let mut counts = BTreeMap::<(&str, &str), usize>::new();
    let mut found = Vec::new();
    let mut link_files = BTreeSet::new();
    let mut all_classes = BTreeSet::new();
    let mut collections = BTreeSet::new();
    for e in items {
        all_classes.extend(classes(e));
        link_files.extend(files(e));
        if !collection(e).is_empty() {
            collections.insert((bucket(e), collection(e)));
        }
        let passes = |field: &str| {
            let mut positive = false;
            let mut matched = false;
            for t in terms.iter().filter(|t| t.field == field) {
                let hit = t.matches(e);
                if t.negative && hit || field == "name" && !t.negative && !hit {
                    return false;
                }
                if !t.negative {
                    positive = true;
                    matched |= hit;
                }
            }
            !positive || matched
        };
        if !passes("name") {
            continue;
        }
        let src = passes("source") && passes("file") && passes("collection");
        let cls = passes("class");
        let usage = passes("usage");
        if cls && usage {
            *counts.entry(("source", bucket(e))).or_default() += 1;
            for f in files(e) {
                *counts.entry(("file", f)).or_default() += 1;
            }
            *counts.entry(("collection", collection(e))).or_default() += 1;
        }
        if src && usage {
            for c in classes(e) {
                *counts.entry(("class", c)).or_default() += 1;
            }
        }
        if src && cls && matches!(e.source.as_str(), "model" | "linked") {
            *counts
                .entry(("usage", if e.uses > 0 { "used" } else { "unused" }))
                .or_default() += 1;
        }
        if src && cls && usage {
            found.push(e);
        }
    }
    let rank: HashMap<_, _> = st
        .recent
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i))
        .collect();
    found.sort_by_cached_key(|e| {
        let order = match st.sort.as_str() {
            "used" => {
                if matches!(e.source.as_str(), "model" | "linked") {
                    -e.uses
                } else {
                    1
                }
            }
            "recent" => rank.get(e.key.as_str()).copied().unwrap_or(rank.len()) as i64,
            _ => 0,
        };
        (order, natural(e.shown()))
    });
    let mut groups = BTreeMap::<String, Vec<&Item>>::new();
    for e in found {
        groups
            .entry(
                match st.group.as_str() {
                    "class" => class(e),
                    "source" => source(e),
                    _ => "",
                }
                .to_owned(),
            )
            .or_default()
            .push(e);
    }
    let caption = |s: &str| json!({"kind":"caption","label":s});
    let row = |field: &str, value: &str, label: &str, depth: usize| {
        let name = if field == "is" { "usage" } else { field };
        let active = terms
            .iter()
            .find(|t| t.field == name && t.value.eq_ignore_ascii_case(value));
        let excluded_by_default = matches!(field, "source" | "file" | "collection")
            && !terms.iter().any(|t| t.field == name && !t.negative);
        let negative = active.map_or(excluded_by_default, |t| t.negative);
        let token = active
            .and_then(|a| terms.iter().position(|t| std::ptr::eq(t, a)))
            .map(|i| tokens[i].clone())
            .unwrap_or_else(|| format!("{}{field}:{value}", if negative { "-" } else { "" }));
        let on = active.map_or(excluded_by_default, |t| !t.negative);
        json!({"kind":"facet","id":format!("toggle:{token}"),"label":label,"depth":depth,"on":on,
            "right":counts.get(&(if field=="is" {"usage"} else {field},value)).copied().unwrap_or(0).to_string()})
    };
    let mut side = vec![caption("Source")];
    for (id, name) in [
        ("model", "This model"),
        ("linked", "Linked files"),
        ("mine", "My library"),
        ("bundled", "Bundled"),
    ] {
        side.push(row("source", id, name, 0));
        if id == "linked" {
            let mut visible = 0;
            let mut children = Vec::new();
            let parent = side.last().is_some_and(|r| r["on"] == true);
            let mut basenames = HashMap::new();
            for path in &link_files {
                *basenames.entry(base_name(path)).or_insert(0) += 1;
            }
            for f in &link_files {
                let short = base_name(f);
                let mut child = row(
                    "file",
                    f,
                    if basenames[&short] == 1 { &short } else { f },
                    1,
                );
                child["tip"] = json!(f);
                if !parent {
                    child["on"] = json!(false);
                }
                if child["on"] == true {
                    visible += 1;
                }
                children.push(child);
            }
            if !children.is_empty() && visible > 0 && visible < children.len() {
                side.last_mut().unwrap()["kind"] = json!("partial-facet");
            }
            side.extend(children);
        }
        for (_, c) in collections.iter().filter(|(src, _)| *src == id) {
            side.push(row("collection", c, c, 1));
        }
    }
    side.push(caption("IFC class"));
    let mut ranked: Vec<_> = all_classes
        .into_iter()
        .filter(|c| *c != "Unclassified")
        .collect();
    ranked.sort_by_key(|c| std::cmp::Reverse(counts.get(&("class", *c)).copied().unwrap_or(0)));
    for c in ranked.iter().take(5) {
        side.push(row("class", c, c, 0));
    }
    side.push(json!({"id":"classes","label":"All classes…","menu":ranked.iter().chain(std::iter::once(&"Unclassified")).map(|c|(format!("class:{c}"),format!("{}  ·  {}",c,counts.get(&("class",*c)).copied().unwrap_or(0)))).collect::<Vec<_>>()}));
    side.push(row("class", "Unclassified", "Unclassified", 0));
    side.push(caption("Usage"));
    side.push(row("is", "used", "In use", 0));
    side.push(row("is", "unused", "Unused", 0));
    Index {
        side,
        groups: groups
            .into_iter()
            .map(|(k, v)| (k, v.len().to_string(), v))
            .collect(),
        tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_fields_negation_usage_and_free_text() {
        let row = Item {
            name: "Landing beam".into(),
            file: Some("Stairs East.built".into()),
            source: "linked".into(),
            classes: vec!["IfcBeam".into()],
            uses: 3,
            ..Default::default()
        };
        let rows = [row];
        for (query, count) in [
            (r#"file:"Stairs East" class:IfcBeam used landing"#, 1),
            ("is:used -source:linked", 0),
            ("unused", 0),
            ("class:IfcWall class:IfcBeam -temporary", 1),
            ("source:linked -file:East", 0),
        ] {
            let view = index(
                &rows,
                &Browse {
                    filter: query.into(),
                    ..Default::default()
                },
            );
            assert_eq!(
                view.groups.iter().map(|g| g.2.len()).sum::<usize>(),
                count,
                "{query}"
            );
        }
        let text = toggle("used", "-file:Stairs East");
        assert_eq!(toggle(&text, "-file:Stairs East"), "used");
    }

    #[test]
    fn cross_counts_on_three_thousand_definitions() {
        let items: Vec<_> = (0..3000)
            .map(|i| Item {
                key: format!("cm:{i}"),
                name: format!("Definition {i}"),
                source: if i % 3 == 0 { "linked" } else { "model" }.into(),
                file: (i % 3 == 0).then(|| {
                    if i % 2 == 0 {
                        "Stairs East".into()
                    } else {
                        "Stairs West".into()
                    }
                }),
                classes: vec![if i % 2 == 0 { "IfcBeam" } else { "IfcWall" }.into()],
                uses: if i % 5 == 0 { 0 } else { 1 },
                ..Default::default()
            })
            .collect();
        let start = std::time::Instant::now();
        let view = index(
            &items,
            &Browse {
                filter: "class:IfcBeam -source:linked used".into(),
                ..Default::default()
            },
        );
        let elapsed = start.elapsed();
        assert_eq!(view.groups[0].2.len(), 800);
        let count = |label: &str| {
            view.side.iter().find(|r| r["label"] == label).unwrap()["right"]
                .as_str()
                .unwrap()
        };
        assert_eq!(count("This model"), "800");
        assert_eq!(count("Linked files"), "400");
        assert_eq!(count("Stairs East"), "400");
        assert_eq!(count("IfcWall"), "800");
        assert_eq!(count("Unused"), "200");
        eprintln!("3,000-definition facet toggle: {elapsed:?}");
        if !cfg!(debug_assertions) {
            assert!(elapsed < std::time::Duration::from_millis(30));
        }
    }
}
