//! The explicit profile and XDG backends share the existing Qt INI file.
//! Keep raw, unrelated entries (including opaque widget variants) verbatim.
use super::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    ops::Range,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

fn key_decode(raw: &str) -> String {
    let mut chars = Vec::new();
    let mut rest = raw;
    while !rest.is_empty() {
        let len = if rest.starts_with("%U") {
            6
        } else if rest.starts_with('%') {
            3
        } else {
            0
        };
        if len > 0 {
            let prefix = if len == 6 { 2 } else { 1 };
            if let Some(v) = rest
                .get(prefix..len)
                .and_then(|h| u16::from_str_radix(h, 16).ok())
            {
                chars.push(v);
                rest = &rest[len..];
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        rest = &rest[c.len_utf8()..];
        let c = if c == '\\' { '/' } else { c };
        chars.extend_from_slice(c.encode_utf16(&mut [0; 2]));
    }
    String::from_utf16_lossy(&chars)
}

fn key_encode(s: &str) -> String {
    let mut out = String::new();
    for u in s.encode_utf16() {
        if u == 47 {
            out.push('\\')
        } else if u < 128 && ((u as u8).is_ascii_alphanumeric() || b"_.-".contains(&(u as u8))) {
            out.push(u as u8 as char)
        } else if u <= 255 {
            out.push_str(&format!("%{u:02X}"))
        } else {
            out.push_str(&format!("%U{u:04X}"))
        }
    }
    out
}

fn split_value(raw: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut whitespace = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        if c == ',' && !quoted {
            fields.push(std::mem::take(&mut field));
            whitespace.clear();
            started = false;
            continue;
        }
        if c.is_ascii_whitespace() && !quoted {
            if started {
                whitespace.push(c)
            }
            continue;
        }
        field.push_str(&whitespace);
        whitespace.clear();
        started = true;
        if c == '"' {
            quoted = !quoted;
            continue;
        }
        if c != '\\' {
            field.push(c);
            continue;
        }
        let Some(c) = it.next() else { break };
        match c {
            'x' => {
                let mut v = 0u32;
                while let Some(d) = it.peek().and_then(|c| c.to_digit(16)) {
                    it.next();
                    v = (v * 16 + d) & 65535
                }
                field.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            '0'..='7' => {
                let mut v = c.to_digit(8).unwrap();
                while let Some(d) = it.peek().and_then(|c| c.to_digit(8)) {
                    it.next();
                    v = (v * 8 + d) & 65535
                }
                field.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            'n' => field.push('\n'),
            'r' => field.push('\r'),
            't' => field.push('\t'),
            'a' => field.push('\x07'),
            'b' => field.push('\x08'),
            'f' => field.push('\x0c'),
            'v' => field.push('\x0b'),
            '\n' => {}
            '\r' => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
            }
            '\\' | '"' => field.push(c),
            _ => {}
        }
    }
    fields.push(field);
    fields
}
fn unescape_at(s: String) -> String {
    if s.starts_with("@@") {
        s[1..].into()
    } else {
        s
    }
}
fn decode(raw: &str) -> Value {
    let mut fields = split_value(raw);
    if fields.len() > 1 {
        return Value::Strings(fields.into_iter().map(unescape_at).collect());
    }
    let s = fields.pop().unwrap_or_default();
    if s == "@Invalid()" {
        Value::Null
    } else if let Some(data) = s
        .strip_prefix("@ByteArray(")
        .and_then(|s| s.strip_suffix(')'))
    {
        Value::Bytes(data.chars().map(|c| c as u8).collect())
    } else {
        Value::Text(unescape_at(s))
    }
}
fn encode_string(s: &str) -> String {
    let mut out = String::from("\"");
    if s.starts_with('@') {
        out.push('@')
    }
    for c in s.chars() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push(c)
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\x0\"\""),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn encode(v: &Value) -> String {
    match v {
        Value::Null => "@Invalid()".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Text(s) => encode_string(s),
        Value::Strings(parts) if parts.is_empty() => "@Invalid()".into(),
        Value::Strings(parts) => parts
            .iter()
            .map(|s| encode_string(s))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Bytes(bytes) => format!(
            "@ByteArray({})",
            bytes
                .iter()
                .map(|b| format!("\\x{b:02x}"))
                .collect::<String>()
        ),
    }
}

#[derive(Default)]
struct Document {
    text: String,
    sections: BTreeMap<String, usize>,
    entries: BTreeMap<String, Vec<(Range<usize>, Range<usize>)>>,
}
impl Document {
    fn parse(text: String) -> Self {
        let mut entries: BTreeMap<_, Vec<_>> = BTreeMap::new();
        let mut section = String::new();
        let mut sections = BTreeMap::new();
        let mut start = 0;
        while start < text.len() {
            let mut end = start;
            loop {
                let len = text[end..].find('\n').map_or(text.len() - end, |n| n + 1);
                end += len;
                let line = text[start..end].trim_end_matches(['\r', '\n']);
                if end == text.len()
                    || line.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 0
                {
                    break;
                }
            }
            let line = text[start..end].trim_start_matches('\u{feff}').trim();
            if let Some(raw) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = if raw.eq_ignore_ascii_case("General") {
                    String::new()
                } else if raw.eq_ignore_ascii_case("%General") {
                    "General".into()
                } else {
                    key_decode(raw)
                };
                sections.insert(section.clone(), end);
            } else if !line.starts_with([';', '#']) {
                if let Some((key, val)) = line.split_once('=') {
                    let mut k = key_decode(key.trim());
                    if !section.is_empty() {
                        k = format!("{section}/{k}")
                    }
                    let v = val.as_ptr() as usize - text.as_ptr() as usize;
                    entries
                        .entry(k)
                        .or_default()
                        .push((start..end, v..v + val.len()));
                }
            }
            start = end;
        }
        Self {
            text,
            entries,
            sections,
        }
    }
    fn changed(&self, key: &str, val: Option<&Value>) -> String {
        if let Some(v) = val {
            let mut out = self.text.clone();
            let value = encode(v);
            if let Some((_, span)) = self.entries.get(key).and_then(|rows| rows.last()) {
                out.replace_range(span.clone(), &value);
            } else {
                let (section, leaf) = key.split_once('/').unwrap_or(("", key));
                let line = format!("{}={value}\n", key_encode(leaf));
                if let Some(&offset) = self.sections.get(section) {
                    let newline = if out[..offset].ends_with('\n') {
                        ""
                    } else {
                        "\n"
                    };
                    out.insert_str(offset, &format!("{newline}{line}"));
                } else {
                    let raw = if section.is_empty() {
                        "General".into()
                    } else if section.eq_ignore_ascii_case("General") {
                        "%General".into()
                    } else {
                        key_encode(section)
                    };
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n')
                    }
                    out.push_str(&format!("[{raw}]\n{line}"));
                }
            }
            return out;
        }
        let prefix = format!("{key}/");
        let mut cuts: Vec<_> = self
            .entries
            .iter()
            .filter(|(k, _)| key.is_empty() || k.as_str() == key || k.starts_with(&prefix))
            .flat_map(|(_, entries)| entries.iter().map(|(span, _)| span))
            .collect();
        cuts.sort_by_key(|r| r.start);
        let mut out = String::new();
        let mut at = 0;
        for r in cuts {
            out.push_str(&self.text[at..r.start]);
            at = r.end
        }
        out.push_str(&self.text[at..]);
        out
    }
}

type Cache = HashMap<PathBuf, (Option<(SystemTime, u64)>, Arc<Document>)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
fn load(path: &Path) -> io::Result<Arc<Document>> {
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    let file = match File::open(path) {
        Ok(f) => Some(f),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let stamp = file
        .as_ref()
        .map(|f| -> io::Result<_> {
            let m = f.metadata()?;
            Ok((m.modified()?, m.len()))
        })
        .transpose()?;
    if let Some((old, doc)) = cache.get(path) {
        if *old == stamp {
            return Ok(doc.clone());
        }
    }
    let text = match file {
        Some(f) => io::read_to_string(f)?,
        None => String::new(),
    };
    let doc = Arc::new(Document::parse(text));
    cache.insert(path.to_owned(), (stamp, doc.clone()));
    Ok(doc)
}
pub fn get(path: &Path, key: &str) -> io::Result<Option<Value>> {
    let d = load(path)?;
    Ok(d.entries
        .get(key)
        .and_then(|vs| vs.last())
        .map(|(_, v)| decode(&d.text[v.clone()])))
}
pub fn keys(path: &Path) -> io::Result<Vec<String>> {
    Ok(load(path)?.entries.keys().cloned().collect())
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn private_file(path: &Path) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

fn host_name() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) };
        String::from_utf8_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())])
            .into_owned()
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    }
}
fn abandoned(path: &Path, host: &str) -> bool {
    // All of our writers hold the kernel-backed .writer lock, including
    // while publishing the Qt lock. Our signature here under that lock can
    // only be left over from a previous process.
    fs::read_to_string(path)
        .is_ok_and(|s| s.lines().nth(2) == Some(host) && s.ends_with("built-native\n"))
}
pub fn update(path: &Path, key: &str, val: Option<&Value>) -> io::Result<()> {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    let mut guard_options = OpenOptions::new();
    guard_options.create(true).write(true).truncate(false);
    let guard = guard_options.open(path.with_added_extension("writer"))?;
    guard.lock()?;
    // Join QSettings' own cross-process lock protocol. Publish complete lock
    // metadata at once so a classic writer can identify the live owner.
    let qtlock = path.with_added_extension("lock");
    let host = host_name();
    let stage = path.with_added_extension("built-tmp");
    if stage.exists() {
        fs::remove_file(&stage)?
    }
    let tmp = Temporary(stage);
    let mut f = private_file(&tmp.0)?;
    let exe = std::env::current_exe()?;
    writeln!(
        f,
        "{}\n{}\n{}\n\n\nbuilt-native",
        std::process::id(),
        exe.file_name().unwrap_or_default().to_string_lossy(),
        host
    )?;
    drop(f);
    let start = Instant::now();
    loop {
        match fs::hard_link(&tmp.0, &qtlock) {
            Ok(()) => break,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if abandoned(&qtlock, &host) {
                    fs::remove_file(&qtlock)?;
                    continue;
                }
                if start.elapsed() > Duration::from_secs(5) {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Profile is locked by another writer",
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }
    let _lock = Temporary(qtlock);
    fs::remove_file(&tmp.0)?;
    let current = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let document = Document::parse(current);
    let new = document.changed(key, val);
    if new == document.text {
        return Ok(());
    }
    let mut f = private_file(&tmp.0)?;
    f.write_all(new.as_bytes())?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp.0, path)?;
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(path);
    Ok(())
}
