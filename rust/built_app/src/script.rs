//! Scripted input (`built --script file`): the same dispatch real winit
//! events take (`App::pointer`, `App::key`, ...), one step per loop turn,
//! with frame captures. Coordinates are logical pixels from the window's
//! top-left.
//!
//! ```text
//! open tests/fixtures/x.igz     # a document (relative to the cwd)
//! open-async file.built          # keep dispatching script input during the open
//! wait-progress 0.6              # wait for this fraction, or completion
//! wait-open                      # wait for the published frame
//! wait 300                      # milliseconds of normal event-loop time
//! resize 1280 800               # window content size in logical pixels
//! move 400 300                  # pointer moves (hover)
//! click 400 300 [left|right|middle] [mods]
//! click-command orbit            # click the rendered top-bar button by id
//! press 400 300                  # hold the button across move/wait, then release X Y
//! double 400 300
//! drag 400 300 600 300 [middle] [steps] [mods]   # press, moves, release
//! key escape [mods]             # a key press and release ("l", "enter", "f3")
//! type 2.5                      # characters as key presses, then nothing else
//! ime ka                        # an input method composing (preedit), then:
//! commit か                     # what it commits
//! tool pushpull                 # a menu command (tool:<key>, undo, view:iso...)
//! wheel 400 300 -40 [phase]     # pixel delta (trackpad slide)
//! pan2 400 300 20 8 [phase]   # two-finger slide in logical pixels
//! rotate 400 300 15            # orbit by gesture rotation in degrees
//! touch-slide 400 300 20 8     # one-finger drag in logical pixels
//! smart-zoom 400 300           # two-finger double tap
//! notch 400 300 1               # mouse-wheel notches
//! pinch 400 300 0.1             # trackpad pinch (begin, zoom, end)
//! msaa off                      # change Display quality (off, 2, 4)
//! capture name                  # build/shell-shots/<name>.png
//! state                         # print Host.state() as JSON
//! quit
//! ```

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Open(String),
    OpenAsync(String),
    WaitOpen,
    WaitProgress(f64),
    Wait(u64),
    Resize(u32, u32),
    Move(f64, f64),
    Button(bool, f64, f64, u32, u32),
    Click(f64, f64, u32, u32),
    ClickCommand(String),
    Double(f64, f64, u32, u32),
    Drag { from: (f64, f64), to: (f64, f64), button: u32, steps: u32, mods: u32 },
    Key(String, u32),
    Type(String),
    /// An input method's composition (as winit's Ime::Preedit), then its commit.
    Preedit(String),
    Commit(String),
    Menu(String),
    Wheel(f64, f64, f64, String),
    Notch(f64, f64, f64),
    Pinch(f64, f64, f64),
    Pan2(f64, f64, f64, f64, String),
    TouchSlide(f64, f64, f64, f64),
    Rotate(f64, f64, f64),
    SmartZoom(f64, f64),
    Msaa(u32),
    Capture(String),
    State,
    Quit,
}

pub fn button(name: &str) -> Option<u32> {
    match name {
        "left" => Some(1),
        "right" => Some(2),
        "middle" => Some(4),
        _ => None,
    }
}

fn num(s: Option<&&str>, line: usize) -> Result<f64, String> {
    s.ok_or_else(|| format!("line {line}: missing number"))?
        .parse()
        .map_err(|_| format!("line {line}: not a number"))
}

pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let body = raw.split('#').next().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        let w: Vec<&str> = body.split_whitespace().collect();
        let rest = |n: usize| w.get(n..).map(|r| r.join(" ")).unwrap_or_default();
        // Optional trailing words: a button name, a step count, modifiers.
        let tail = |from: usize| {
            let (mut b, mut s, mut m) = (None, None, 0);
            for word in w.iter().skip(from) {
                if let Some(x) = button(word) {
                    b = Some(x);
                } else if let Ok(n) = word.parse::<u32>() {
                    s = Some(n);
                } else {
                    m |= crate::keys::qt_modifiers_by_name(word);
                }
            }
            (b, s, m)
        };
        let step = match w[0] {
            "open-async" => Step::OpenAsync(rest(1)),
            "wait-open" => Step::WaitOpen,
            "wait-progress" => {
                let fraction = num(w.get(1), line)?;
                if !(0.0..=1.0).contains(&fraction) {
                    return Err(format!("line {line}: expected a fraction from zero to one"));
                }
                Step::WaitProgress(fraction)
            }
            "open" => Step::Open(rest(1)),
            "wait" => Step::Wait(num(w.get(1), line)? as u64),
            "resize" => {
                let dimension = |word: Option<&&str>| {
                    word.and_then(|s| s.parse::<u32>().ok()).filter(|n| *n > 0)
                        .ok_or_else(|| format!("line {line}: expected positive window dimensions"))
                };
                Step::Resize(dimension(w.get(1))?, dimension(w.get(2))?)
            }
            "move" => Step::Move(num(w.get(1), line)?, num(w.get(2), line)?),
            "click-command" => Step::ClickCommand(w.get(1).ok_or(format!("line {line}: missing command id"))?.to_string()),
            "press" | "release" => {
                let (b, _, mods) = tail(3);
                Step::Button(w[0] == "press", num(w.get(1), line)?, num(w.get(2), line)?, b.unwrap_or(1), mods)
            }
            "click" | "double" => {
                let (b, _, m) = tail(3);
                let (x, y) = (num(w.get(1), line)?, num(w.get(2), line)?);
                if w[0] == "click" {
                    Step::Click(x, y, b.unwrap_or(1), m)
                } else {
                    Step::Double(x, y, b.unwrap_or(1), m)
                }
            }
            "drag" => {
                let (b, s, m) = tail(5);
                Step::Drag {
                    from: (num(w.get(1), line)?, num(w.get(2), line)?),
                    to: (num(w.get(3), line)?, num(w.get(4), line)?),
                    button: b.unwrap_or(1),
                    steps: s.unwrap_or(12).max(1),
                    mods: m,
                }
            }
            "key" => Step::Key(
                w.get(1).ok_or(format!("line {line}: key name"))?.to_string(),
                w.get(2).map(|m| crate::keys::qt_modifiers_by_name(m)).unwrap_or(0),
            ),
            "type" => Step::Type(rest(1)),
            "ime" => Step::Preedit(rest(1)),
            "commit" => Step::Commit(rest(1)),
            "tool" => Step::Menu(format!("tool:{}", rest(1))),
            "menu" => Step::Menu(rest(1)),
            "wheel" => Step::Wheel(
                num(w.get(1), line)?,
                num(w.get(2), line)?,
                num(w.get(3), line)?,
                w.get(4).unwrap_or(&"move").to_string(),
            ),
            "notch" => Step::Notch(num(w.get(1), line)?, num(w.get(2), line)?, num(w.get(3), line)?),
            "pan2" => Step::Pan2(
                num(w.get(1), line)?,
                num(w.get(2), line)?,
                num(w.get(3), line)?,
                num(w.get(4), line)?,
                w.get(5).unwrap_or(&"move").to_string(),
            ),
            "touch-slide" => Step::TouchSlide(
                num(w.get(1), line)?,
                num(w.get(2), line)?,
                num(w.get(3), line)?,
                num(w.get(4), line)?,
            ),
            "smart-zoom" => Step::SmartZoom(num(w.get(1), line)?, num(w.get(2), line)?),
            "rotate" => Step::Rotate(
                num(w.get(1), line)?,
                num(w.get(2), line)?,
                num(w.get(3), line)?,
            ),
            "pinch" => Step::Pinch(num(w.get(1), line)?, num(w.get(2), line)?, num(w.get(3), line)?),
            "msaa" => {
                let count = match w.get(1).copied() {
                    Some("off") => 1,
                    Some("2") => 2,
                    Some("4") => 4,
                    _ => return Err(format!("line {line}: expected msaa off, 2 or 4")),
                };
                Step::Msaa(count)
            }
            "capture" => Step::Capture(rest(1)),
            "state" => Step::State,
            "quit" => Step::Quit,
            other => return Err(format!("line {line}: unknown step {other:?}")),
        };
        steps.push(step);
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_step() {
        let s = parse(
            "open a.igz\nwait 50 # settle\nmove 1 2\nclick 3 4\nclick 3 4 right shift\n\
             drag 1 2 3 4 middle 6\nkey escape\nkey z cmd\ntype 2.5\ntool line\n\
             wheel 5 6 -40 start\nnotch 1 1 2\npinch 1 1 0.1\ncapture shot\nstate\nquit\n",
        )
        .unwrap();
        assert_eq!(s[0], Step::Open("a.igz".into()));
        assert_eq!(s[1], Step::Wait(50));
        assert_eq!(s[4], Step::Click(3., 4., 2, crate::keys::SHIFT));
        assert_eq!(s[5], Step::Drag { from: (1., 2.), to: (3., 4.), button: 4, steps: 6, mods: 0 });
        assert_eq!(s[7], Step::Key("z".into(), crate::keys::CONTROL));
        assert_eq!(s[9], Step::Menu("tool:line".into()));
        assert_eq!(s[10], Step::Wheel(5., 6., -40., "start".into()));
        assert_eq!(*s.last().unwrap(), Step::Quit);
    }

    #[test]
    fn rejects_unknown_steps() {
        assert!(parse("jump 1 2").is_err());
        assert!(parse("move x 2").is_err());
    }
}
