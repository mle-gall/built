// SPDX-License-Identifier: GPL-3.0-or-later
//! VFF appends a ZIP to the .skp header. Archives exist with offsets
//! relative to either the physical file or the embedded ZIP, including ZIP64.
use flate2::read::DeflateDecoder;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid .skp ZIP directory or member",
    )
}
fn word(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes(b[i..i + 2].try_into().unwrap())
}
fn dword(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn qword(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}
fn read_at<const N: usize>(f: &mut File, at: u64) -> io::Result<[u8; N]> {
    let mut b = [0; N];
    f.seek(SeekFrom::Start(at))?;
    f.read_exact(&mut b)?;
    Ok(b)
}
struct Entry {
    offset: u64,
    packed: u64,
    size: u64,
    method: u16,
    crc: u32,
    name: Vec<u8>,
}
pub(super) struct Archive {
    file: File,
    entries: HashMap<String, Entry>,
    pub names: Vec<String>,
    directory: u64,
    pub version: String,
}
impl Archive {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        let mut header = vec![0; length.min(4096) as usize];
        file.read_exact(&mut header)?;
        if !(header.starts_with(&[255, 254]) || header.starts_with(b"PK\x03\x04")) {
            return Err(invalid());
        }
        let version = header
            .windows(3)
            .skip(4)
            .position(|w| w == [255, 254, 255])
            .and_then(|i| {
                let text = String::from_utf16_lossy(
                    &header
                        .get(i + 8..)?
                        .chunks_exact(2)
                        .take(64)
                        .map(|s| word(s, 0))
                        .collect::<Vec<_>>(),
                );
                Some(text.get(text.find('{')?..=text.find('}')?)?.to_owned())
            })
            .unwrap_or_default();
        let size = length.min(65535 + 22);
        let mut tail = vec![0; size as usize];
        file.seek(SeekFrom::Start(length - size))?;
        file.read_exact(&mut tail)?;
        let e = tail
            .windows(22)
            .enumerate()
            .rev()
            .find(|(i, w)| {
                w.starts_with(b"PK\x05\x06") && i + 22 + word(w, 20) as usize == tail.len()
            })
            .map(|(i, _)| i)
            .ok_or_else(invalid)?;
        let end = &tail[e..];
        if word(end, 4) != 0 || word(end, 6) != 0 || word(end, 8) != word(end, 10) {
            return Err(invalid());
        }
        let mut anchor = length - size + e as u64;
        let mut count = u64::from(word(end, 10));
        let mut bytes = u64::from(dword(end, 12));
        let mut offset = u64::from(dword(end, 16));
        if anchor >= 76 {
            let locator = read_at::<20>(&mut file, anchor - 20)?;
            if locator.starts_with(b"PK\x06\x07") {
                if dword(&locator, 4) != 0 || dword(&locator, 16) != 1 {
                    return Err(invalid());
                }
                let pos = qword(&locator, 8);
                let candidates = [anchor - 76, pos];
                let (start, rec) = candidates
                    .iter()
                    .filter(|&&p| p <= anchor - 76)
                    .find_map(|&p| {
                        let r = read_at::<56>(&mut file, p).ok()?;
                        (r.starts_with(b"PK\x06\x06")
                            && p.checked_add(12)?.checked_add(qword(&r, 4))? == anchor - 20)
                            .then_some((p, r))
                    })
                    .ok_or_else(invalid)?;
                if dword(&rec, 16) != 0
                    || dword(&rec, 20) != 0
                    || qword(&rec, 24) != qword(&rec, 32)
                {
                    return Err(invalid());
                }
                anchor = start;
                count = qword(&rec, 32);
                bytes = qword(&rec, 40);
                offset = qword(&rec, 48);
            }
        }
        let directory = anchor.checked_sub(bytes).ok_or_else(invalid)?;
        let prefix = directory.checked_sub(offset).ok_or_else(invalid)?;
        if count > bytes / 46 {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(directory))?;
        let mut entries = HashMap::new();
        let mut names = Vec::new();
        let mut input = std::io::BufReader::new((&mut file).take(bytes));
        for _ in 0..count {
            let mut b = [0; 46];
            input.read_exact(&mut b)?;
            if !b.starts_with(b"PK\x01\x02") || word(&b, 34) != 0 {
                return Err(invalid());
            }
            let mut name = vec![0; word(&b, 28) as usize];
            input.read_exact(&mut name)?;
            let mut extra = vec![0; word(&b, 30) as usize];
            input.read_exact(&mut extra)?;
            io::copy(&mut (&mut input).take(word(&b, 32).into()), &mut io::sink())?;
            let mut size = u64::from(dword(&b, 24));
            let mut packed = u64::from(dword(&b, 20));
            let mut start = u64::from(dword(&b, 42));
            let mut fields = extra.as_slice();
            while fields.len() >= 4 {
                let tag = word(fields, 0);
                let (value, rest) = fields[4..]
                    .split_at_checked(word(fields, 2) as usize)
                    .ok_or_else(invalid)?;
                fields = rest;
                if tag == 1 {
                    let mut v = value;
                    for n in [&mut size, &mut packed, &mut start] {
                        if *n == u32::MAX as u64 {
                            if v.len() < 8 {
                                return Err(invalid());
                            }
                            *n = qword(v, 0);
                            v = &v[8..];
                        }
                    }
                }
            }
            let label = if word(&b, 8) & 0x800 != 0 {
                String::from_utf8(name.clone()).map_err(|_| invalid())?
            } else {
                // PKWARE's original filename encoding.
                const HIGH:&str="ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■ ";
                name.iter()
                    .map(|&n| {
                        if n < 128 {
                            char::from(n)
                        } else {
                            HIGH.chars().nth(usize::from(n - 128)).unwrap()
                        }
                    })
                    .collect()
            };
            let offset = start.checked_add(prefix).ok_or_else(invalid)?;
            if offset >= directory {
                return Err(invalid());
            }
            names.push(label.clone());
            entries.insert(
                label,
                Entry {
                    offset,
                    packed,
                    size,
                    method: word(&b, 10),
                    crc: dword(&b, 16),
                    name,
                },
            );
        }
        drop(input);
        Ok(Self {
            file,
            entries,
            names,
            directory,
            version,
        })
    }
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }
    pub fn read(&mut self, name: &str) -> io::Result<Vec<u8>> {
        let e = self
            .entries
            .get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name))?;
        if e.size > 16 << 30 || (e.size >= 1 << 20 && e.size > e.packed.saturating_mul(1000)) {
            return Err(invalid());
        }
        let h = read_at::<30>(&mut self.file, e.offset)?;
        if !h.starts_with(b"PK\x03\x04")
            || word(&h, 6) & 1 != 0
            || word(&h, 8) != e.method
            || word(&h, 26) as usize != e.name.len()
        {
            return Err(invalid());
        }
        let mut actual = vec![0; e.name.len()];
        self.file.read_exact(&mut actual)?;
        if actual != e.name {
            return Err(invalid());
        }
        let start = self.file.seek(SeekFrom::Current(i64::from(word(&h, 28))))?;
        if start
            .checked_add(e.packed)
            .is_none_or(|end| end > self.directory)
        {
            return Err(invalid());
        }
        let compressed = (&mut self.file).take(e.packed);
        let r: Box<dyn Read> = match e.method {
            0 if e.size == e.packed => Box::new(compressed),
            8 => Box::new(DeflateDecoder::new(compressed)),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "unsupported .skp ZIP compression",
                ))
            }
        };
        let mut result = Vec::new();
        r.take(e.size + 1).read_to_end(&mut result)?;
        if result.len() as u64 != e.size || crc32fast::hash(&result) != e.crc {
            return Err(invalid());
        }
        Ok(result)
    }
}
