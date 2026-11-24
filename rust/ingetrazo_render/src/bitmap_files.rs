//! Portable mono bitmaps, X pixmaps and Windows cursor resources.
use image::{ImageFormat, ImageReader, Rgba, RgbaImage};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Cursor, Read},
    path::Path,
    sync::OnceLock,
};

const MAX_BYTES: u64 = 128 * 1024 * 1024;

fn size(w: u32, h: u32) -> Option<(u32, u32)> {
    (w > 0 && h > 0 && u64::from(w) * u64::from(h) <= 64 * 1024 * 1024).then_some((w, h))
}

enum Layout {
    Wbmp,
    Xbm,
    Xpm,
    Cursor,
}

fn layout(path: &Path, prefix: &[u8]) -> Option<Layout> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = String::from_utf8_lossy(prefix);
    if prefix.starts_with(&[0, 0, 2, 0]) || ext == "cur" {
        Some(Layout::Cursor)
    } else if ext == "xpm"
        || text.trim_start().starts_with("/* XPM */")
        || text.starts_with("! XPM2")
    {
        Some(Layout::Xpm)
    } else if ext == "xbm" || text.trim_start().starts_with("#define") {
        Some(Layout::Xbm)
    } else if ext == "wbmp" || prefix.starts_with(&[0, 0]) {
        Some(Layout::Wbmp)
    } else {
        None
    }
}

/// Probe first. Unknown file types can be large and belong to a platform codec.
pub(super) fn read(path: &Path) -> Option<RgbaImage> {
    let mut file = BufReader::new(File::open(path).ok()?);
    let mut head = [0u8; 128];
    let n = file.read(&mut head).ok()?;
    let format = layout(path, &head[..n])?;
    if file.get_ref().metadata().ok()?.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = head[..n].to_vec();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    match format {
        Layout::Wbmp => wbmp(&bytes),
        Layout::Xbm => xbm(std::str::from_utf8(&bytes).ok()?),
        Layout::Xpm => xpm(&bytes),
        Layout::Cursor => {
            let source = || ImageReader::with_format(Cursor::new(&bytes), ImageFormat::Ico);
            let (w, h) = source().into_dimensions().ok()?;
            size(w, h)?;
            Some(source().decode().ok()?.into_rgba8())
        }
    }
}

fn wbmp(bytes: &[u8]) -> Option<RgbaImage> {
    let mut rest = bytes.strip_prefix(&[0, 0])?.iter();
    let mut integer = || {
        let mut n = 0u32;
        for &byte in rest.by_ref().take(5) {
            n = n.checked_mul(128)?.checked_add(u32::from(byte & 127))?;
            if byte & 128 == 0 {
                return Some(n);
            }
        }
        None
    };
    let (w, h) = size(integer()?, integer()?)?;
    let stride = w.div_ceil(8) as usize;
    let data = rest.as_slice();
    if data.len() != stride.checked_mul(h as usize)? {
        return None;
    }
    Some(RgbaImage::from_fn(w, h, |x, y| {
        let white = data[y as usize * stride + x as usize / 8] & (128 >> (x % 8)) != 0;
        grey(white)
    }))
}
fn grey(white: bool) -> Rgba<u8> {
    let c = if white { 255 } else { 0 };
    Rgba([c, c, c, 255])
}
fn xbm(text: &str) -> Option<RgbaImage> {
    let (header, body) = text.split_once('{')?;
    let define = |suffix: &str| {
        header.lines().find_map(|line| {
            let mut parts = line.split_ascii_whitespace();
            (parts.next()? == "#define" && parts.next()?.ends_with(suffix)).then_some(())?;
            parts.next()?.parse().ok()
        })
    };
    let (w, h) = size(define("_width")?, define("_height")?)?;
    let shorts = header.split_ascii_whitespace().any(|word| word == "short");
    let bits = if shorts { 16 } else { 8 };
    let stride = w.div_ceil(bits) as usize;
    let count = stride.checked_mul(h as usize)?;
    let mut words = Vec::new();
    for token in body
        .split_once('}')?
        .0
        .split([',', ' ', '\t', '\r', '\n'])
        .filter(|s| !s.is_empty())
        .take(count)
    {
        words.push(
            if let Some(hex) = token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
            {
                u16::from_str_radix(hex, 16).ok()?
            } else {
                token.parse().ok()?
            },
        );
    }
    if words.len() != count {
        return None;
    }
    Some(RgbaImage::from_fn(w, h, |x, y| {
        let word = words[y as usize * stride + (x / bits) as usize];
        grey(word & (1 << (x % bits)) == 0)
    }))
}

/// XPM symbols may themselves be whitespace, quotes or backslashes.
fn c_strings(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut lines = vec![];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            i += 2 + bytes[i + 2..].windows(2).position(|s| s == b"*/")? + 2;
        } else if bytes[i..].starts_with(b"//") {
            i += bytes[i..]
                .iter()
                .position(|&c| c == b'\n')
                .unwrap_or(bytes.len() - i);
        } else if bytes[i] == b'"' {
            let mut row = vec![];
            i += 1;
            while *bytes.get(i)? != b'"' {
                let mut c = bytes[i];
                i += 1;
                if c == b'\\' {
                    c = *bytes.get(i)?;
                    i += 1;
                    if (b'0'..=b'7').contains(&c) {
                        let mut value = u16::from(c - b'0');
                        for _ in 0..2 {
                            let Some(&d @ b'0'..=b'7') = bytes.get(i) else {
                                break;
                            };
                            value = value * 8 + u16::from(d - b'0');
                            i += 1;
                        }
                        c = value as u8;
                    }
                }
                row.push(c);
            }
            lines.push(row);
            i += 1;
        } else {
            i += 1;
        }
    }
    Some(lines)
}

fn xpm(bytes: &[u8]) -> Option<RgbaImage> {
    let strings = if bytes.starts_with(b"! XPM2") {
        bytes
            .split(|&c| c == b'\n')
            .skip(1)
            .filter(|l| !l.starts_with(b"!"))
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec())
            .collect()
    } else {
        c_strings(bytes)?
    };
    let mut lines = strings.iter();
    let mut header = std::str::from_utf8(lines.next()?)
        .ok()?
        .split_ascii_whitespace();
    let (w, h) = size(header.next()?.parse().ok()?, header.next()?.parse().ok()?)?;
    let n: usize = header.next()?.parse().ok()?;
    let cpp: usize = header.next()?.parse().ok()?;
    if cpp == 0 || n == 0 || n.checked_add(h as usize)? > lines.len() {
        return None;
    }
    let mut colors = HashMap::new();
    for line in lines.by_ref().take(n) {
        let (symbol, spec) = line.split_at_checked(cpp)?;
        let fields: Vec<_> = std::str::from_utf8(spec)
            .ok()?
            .split_ascii_whitespace()
            .collect();
        let mut value = None;
        for key in ["c", "g", "g4", "m"] {
            if let Some(at) = fields.iter().position(|&s| s == key) {
                let name: String = fields[at + 1..]
                    .iter()
                    .copied()
                    .take_while(|s| !matches!(*s, "s" | "c" | "g" | "g4" | "m"))
                    .collect();
                if !name.is_empty() {
                    value = Some(color(&name));
                    break;
                }
            }
        }
        colors.insert(symbol, value?);
    }
    let mut data = Vec::new();
    for row in lines.take(h as usize) {
        for symbol in row.get(..cpp.checked_mul(w as usize)?)?.chunks_exact(cpp) {
            data.extend_from_slice(colors.get(symbol)?);
        }
    }
    RgbaImage::from_raw(w, h, data)
}

fn color(name: &str) -> [u8; 4] {
    let name = name.to_ascii_lowercase();
    if name == "none" {
        return [0; 4];
    }
    if let Some(hex) = name.strip_prefix('#') {
        if hex.is_ascii() && [3, 6, 9, 12].contains(&hex.len()) {
            let digits = hex.len() / 3;
            let max = (1u32 << (digits * 4)) - 1;
            let mut out = [255; 4];
            for (i, part) in hex.as_bytes().chunks_exact(digits).enumerate() {
                let Ok(v) = u32::from_str_radix(std::str::from_utf8(part).unwrap(), 16) else {
                    return [0, 0, 0, 255];
                };
                out[i] = ((v * 255 + max / 2) / max) as u8;
            }
            return out;
        }
    }
    static NAMES: OnceLock<HashMap<&str, [u8; 4]>> = OnceLock::new();
    *NAMES
        .get_or_init(|| {
            let mut colors = HashMap::new();
            for line in include_str!("x11_colors.txt").lines() {
                let mut names = line.split_ascii_whitespace();
                let value = u32::from_str_radix(names.next().unwrap(), 16)
                    .unwrap()
                    .to_be_bytes();
                for alias in names {
                    colors.insert(alias, [value[1], value[2], value[3], 255]);
                }
            }
            colors
        })
        .get(name.as_str())
        .unwrap_or(&[0, 0, 0, 255])
}
