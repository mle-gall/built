//! Release review and the non-interrupting startup notice.
pub(crate) mod service;
pub(crate) mod check;
use crate::ui::kit::ProseSpan;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use pyo3::prelude::*;
use serde_json::{json, Value};

#[pyclass(frozen)]
pub struct ReleaseReview {
    #[pyo3(get)]
    version: String,
    current: String,
    date: String,
    url: String,
    blocks: Vec<Value>,
    links: Vec<String>,
    quiet: bool,
    newer: bool,
    skipped: bool,
}

#[pymethods]
impl ReleaseReview {
    #[new]
    fn new(
        current: String,
        version: String,
        date: String,
        notes: &str,
        url: String,
        quiet: bool,
        newer: bool,
        skipped: bool,
    ) -> Self {
        let (blocks, links) = notes_blocks(notes);
        Self {
            version,
            current,
            date,
            url,
            blocks,
            links,
            quiet,
            newer,
            skipped,
        }
    }

    fn presentation(&self, busy: bool) -> &'static str {
        if self.version.is_empty() {
            return if self.quiet { "silent" } else { "error" };
        }
        if !self.newer {
            return if self.quiet { "silent" } else { "current" };
        }
        if !self.quiet {
            return "review";
        }
        if self.skipped {
            return "silent";
        }
        if busy {
            "wait"
        } else {
            "notice"
        }
    }

    fn notice(&self, tr: &Bound<'_, PyAny>) -> PyResult<(String, String, &'static str, String)> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let (text, tone, action) =
            if self.version.is_empty() {
                (t("Could not check for updates. Check your internet connection and try again.")?,
             "warning", String::new())
            } else if !self.newer {
                (
                    t("You have the latest version ({version}).")?
                        .replace("{version}", &self.current),
                    "info",
                    String::new(),
                )
            } else {
                (
                    t("Built {version} is available")?.replace("{version}", &self.version),
                    "info",
                    t("What's new:")?,
                )
            };
        Ok((t("Updates")?, text, tone, action))
    }

    fn sheet(&self, tr: &Bound<'_, PyAny>) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let title = t("Built {version} is available")?.replace("{version}", &self.version);
        let sub = t("You have version {current}. Published on {date}.")?
            .replace("{current}", &self.current)
            .replace("{date}", &self.date);
        let mut blocks = vec![json!({"kind":"caption","text":t("What's new:")?})];
        blocks.extend(self.blocks.iter().cloned());
        Ok(
            json!({"id":"update","title":t("Updates")?,"width":820,"default_verb":"download",
          "content":{"title":title,"sub":sub,"blocks":blocks},
          "verbs":[{"id":"skip","label":t("Skip This Version")?},
                   {"id":"later","label":t("Remind Me Later")?},
                   {"id":"download","label":t("Download")?,"role":1,"key":"↩"}]})
            .to_string(),
        )
    }

    fn action(&self, id: &str) -> Option<(&'static str, &str, bool)> {
        match id {
            "skip" => Some(("skip", &self.version, true)),
            "later" => Some(("close", "", true)),
            "download" => Some(("open", &self.url, true)),
            _ => self
                .links
                .get(id.strip_prefix("link-")?.parse::<usize>().ok()?)
                .map(|url| ("open", url.as_str(), false)),
        }
    }
}

fn notes_blocks(notes: &str) -> (Vec<Value>, Vec<String>) {
    let mut blocks = Vec::new();
    let mut spans = Vec::<ProseSpan>::new();
    let mut links = Vec::new();
    let mut style = ProseSpan::default();
    let mut lists = Vec::new();
    let flush = |spans: &mut Vec<ProseSpan>, blocks: &mut Vec<Value>| {
        if !spans.is_empty() {
            blocks.push(json!({"kind":"prose","spans":spans}));
            spans.clear();
        }
    };
    for event in Parser::new(notes) {
        let text = match event {
            Event::Start(Tag::Heading { .. }) => {
                style.heading = true;
                continue;
            }
            Event::Start(Tag::Strong) => {
                style.bold = true;
                continue;
            }
            Event::Start(Tag::Emphasis) => {
                style.italic = true;
                continue;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                style.code = true;
                continue;
            }
            Event::Start(Tag::List(first)) => {
                flush(&mut spans, &mut blocks);
                lists.push(first);
                continue;
            }
            Event::Start(Tag::Item) => {
                flush(&mut spans, &mut blocks);
                match lists.last_mut() {
                    Some(Some(n)) => {
                        let prefix = format!("{n}. ");
                        *n += 1;
                        prefix
                    }
                    _ => "• ".into(),
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                if dest_url.starts_with("https://") || dest_url.starts_with("http://") {
                    style.link = format!("link-{}", links.len());
                    links.push(dest_url.into_string());
                }
                continue;
            }
            Event::End(TagEnd::Link) => {
                style.link.clear();
                continue;
            }
            Event::End(TagEnd::Emphasis) => {
                style.italic = false;
                continue;
            }
            Event::End(TagEnd::Strong) => {
                style.bold = false;
                continue;
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                continue;
            }
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock,
            ) => {
                flush(&mut spans, &mut blocks);
                style.heading = false;
                style.code = false;
                continue;
            }
            Event::Rule => {
                blocks.push(json!({"kind":"separator"}));
                continue;
            }
            Event::Text(text) => text.into_string(),
            Event::Code(text) => {
                spans.push(ProseSpan {
                    text: text.into_string(),
                    code: true,
                    ..style.clone()
                });
                continue;
            }
            Event::HardBreak => "\n".into(),
            Event::SoftBreak => " ".into(),
            _ => continue,
        };
        spans.push(ProseSpan {
            text,
            ..style.clone()
        });
    }
    flush(&mut spans, &mut blocks);
    (blocks, links)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notes_preserve_headings_emphasis_and_safe_links() {
        let (blocks, urls) = notes_blocks("## Fixed\n\n1. **Faces** and [notes](https://example.com).\n2. [local](file:///tmp/test)");
        assert_eq!(urls, ["https://example.com"]);
        assert_eq!(blocks[0]["spans"][0]["heading"], true);
        let spans = blocks[1]["spans"].as_array().unwrap();
        assert_eq!(spans[0]["text"], "1. ");
        assert!(spans
            .iter()
            .any(|s| s["text"] == "Faces" && s["bold"] == true));
        assert!(spans
            .iter()
            .any(|s| s["text"] == "notes" && s["link"] == "link-0"));
        assert_eq!(blocks[2]["spans"][0]["text"], "2. ");
    }
}
