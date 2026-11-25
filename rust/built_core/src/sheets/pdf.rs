// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Sheets as PDF, from their display lists (docs/rust-sheets.md §5):
//! every item's primitives in stacking order, then the sheet's border, in
//! paper millimetres on a page of the sheet's size. Lines, polygons,
//! ellipses, rounded rectangles and paths as PDF paths with their pens;
//! clips scoped as the painters scoped them; pictures as images (a frame's
//! render, an image item's file); text laid out as Qt's
//! `drawText(rect, flags)` lays it out on the sheet font's metrics, in the
//! embedded Inter (CIDFontType2, Identity-H, its own widths, kerning as
//! `TJ` adjustments; italic slanted, the font set having none).
//!
//! It replaces `QPdfWriter` and `_paint_sheet` (views/composer.py).

use super::metrics::{face_for, wrap_lines, Face};
use super::paint::{align, Cap, Op, Pen, Prim, Rgba};
use flate2::{write::ZlibEncoder, Compression};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;

const PT_PER_MM: f64 = 72.0 / 25.4;
static INTER_REGULAR: &[u8] = include_bytes!("../../../../resources/fonts/inter/Inter-Regular.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../../../../resources/fonts/inter/Inter-SemiBold.ttf");

// ---- The font -----------------------------------------------------------------

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn bi16(b: &[u8], o: usize) -> i16 {
    be16(b, o) as i16
}

/// What the writer needs of a TrueType font: its glyph for each character,
/// its widths and the descriptor's numbers.
pub struct Font {
    data: &'static [u8],
    cmap: HashMap<u32, u16>,
    widths: Vec<u16>,
    pub upem: f64,
    bbox: [i16; 4],
    ascent: i16,
    descent: i16,
    cap_height: i16,
    name: &'static str,
}

impl Font {
    pub fn parse(data: &'static [u8], name: &'static str) -> Font {
        let n = be16(data, 4) as usize;
        let mut tables = HashMap::new();
        for i in 0..n {
            let r = 12 + 16 * i;
            tables.insert(&data[r..r + 4], be32(data, r + 8) as usize);
        }
        let t = |tag: &[u8]| tables.get(tag).copied().unwrap_or(0);
        let head = t(b"head");
        let upem = f64::from(be16(data, head + 18));
        let bbox = [bi16(data, head + 36), bi16(data, head + 38), bi16(data, head + 40), bi16(data, head + 42)];
        let hhea = t(b"hhea");
        let (ascent, descent) = (bi16(data, hhea + 4), bi16(data, hhea + 6));
        let n_hm = be16(data, hhea + 34) as usize;
        let maxp = t(b"maxp");
        let n_glyphs = be16(data, maxp + 4) as usize;
        let hmtx = t(b"hmtx");
        let mut widths = Vec::with_capacity(n_glyphs);
        for g in 0..n_glyphs {
            widths.push(be16(data, hmtx + 4 * g.min(n_hm.saturating_sub(1))));
        }
        let os2 = t(b"OS/2");
        let cap_height = if os2 != 0 && be16(data, os2) >= 2 { bi16(data, os2 + 88) } else { (f64::from(ascent) * 0.7) as i16 };
        Font { data, cmap: cmap(data, t(b"cmap")), widths, upem, bbox, ascent, descent, cap_height, name }
    }
    pub fn glyph(&self, c: char) -> u16 {
        self.cmap.get(&(c as u32)).copied().unwrap_or(0)
    }
    fn width(&self, g: u16) -> f64 {
        f64::from(self.widths.get(g as usize).copied().unwrap_or(0)) * 1000.0 / self.upem
    }
}

/// The Unicode map (format 12, else format 4).
fn cmap(d: &[u8], at: usize) -> HashMap<u32, u16> {
    let mut out = HashMap::new();
    let n = be16(d, at + 2) as usize;
    let mut f4 = None;
    let mut f12 = None;
    for i in 0..n {
        let r = at + 4 + 8 * i;
        let (pid, eid, off) = (be16(d, r), be16(d, r + 2), at + be32(d, r + 4) as usize);
        let format = be16(d, off);
        if format == 12 && (pid == 0 || (pid == 3 && eid == 10)) {
            f12 = Some(off);
        } else if format == 4 && (pid == 0 || (pid == 3 && eid == 1)) {
            f4 = Some(off);
        }
    }
    if let Some(o) = f12 {
        let groups = be32(d, o + 12) as usize;
        for g in 0..groups {
            let r = o + 16 + 12 * g;
            let (start, end, glyph) = (be32(d, r), be32(d, r + 4), be32(d, r + 8));
            for c in start..=end {
                out.insert(c, (glyph + c - start) as u16);
            }
        }
    } else if let Some(o) = f4 {
        let segs = be16(d, o + 6) as usize / 2;
        let (ends, starts, deltas, ranges) = (o + 14, o + 16 + 2 * segs, o + 16 + 4 * segs, o + 16 + 6 * segs);
        for s in 0..segs {
            let (end, start) = (be16(d, ends + 2 * s), be16(d, starts + 2 * s));
            let (delta, range) = (be16(d, deltas + 2 * s), be16(d, ranges + 2 * s));
            for c in start..=end {
                if c == 0xffff {
                    break;
                }
                let g = if range == 0 {
                    c.wrapping_add(delta)
                } else {
                    let p = ranges + 2 * s + range as usize + 2 * (c - start) as usize;
                    match be16(d, p) {
                        0 => 0,
                        g => g.wrapping_add(delta),
                    }
                };
                out.insert(u32::from(c), g);
            }
        }
    }
    out
}

pub fn fonts() -> &'static [Font; 2] {
    static F: std::sync::OnceLock<[Font; 2]> = std::sync::OnceLock::new();
    F.get_or_init(|| [Font::parse(INTER_REGULAR, "Inter-Regular"), Font::parse(INTER_SEMIBOLD, "Inter-SemiBold")])
}

// ---- Content --------------------------------------------------------------------

/// A picture an image op names: RGBA pixels.
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// One item on the page: its origin (paper mm) and display list.
pub struct Placed {
    pub origin: [f64; 2],
    pub ops: Vec<Op>,
}

pub struct Page {
    pub size: [f64; 2],
    pub items: Vec<Placed>,
}

struct Writer {
    objects: Vec<Vec<u8>>,
}

impl Writer {
    fn reserve(&mut self) -> usize {
        self.objects.push(Vec::new());
        self.objects.len()
    }
    fn set(&mut self, id: usize, body: Vec<u8>) {
        self.objects[id - 1] = body;
    }
    fn add(&mut self, body: Vec<u8>) -> usize {
        self.objects.push(body);
        self.objects.len()
    }
    fn stream(&mut self, dict: &str, data: &[u8]) -> usize {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(data).expect("deflate");
        let packed = z.finish().expect("deflate");
        let mut body = format!("<< {dict} /Filter /FlateDecode /Length {} >>\nstream\n", packed.len()).into_bytes();
        body.extend(packed);
        body.extend(b"\nendstream");
        self.add(body)
    }
}

fn n(v: f64) -> String {
    let s = format!("{:.4}", if v.abs() < 1e-9 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.into() }
}

/// The content stream of one page and what it uses.
struct Content {
    out: String,
    /// Alpha → ExtGState name.
    alphas: BTreeMap<(u8, u8), String>,
    /// Picture source → XObject name.
    images: BTreeMap<String, String>,
    /// Glyphs used per font.
    used: [BTreeMap<u16, char>; 2],
    clip_open: bool,
}

impl Content {
    fn gs(&mut self, stroke: u8, fill: u8) {
        if stroke == 255 && fill == 255 {
            return;
        }
        let k = self.alphas.len();
        let name = self.alphas.entry((stroke, fill)).or_insert_with(|| format!("A{k}")).clone();
        self.out += &format!("/{name} gs\n");
    }
    fn rgb(c: Rgba) -> String {
        format!("{} {} {}", n(f64::from(c.0) / 255.0), n(f64::from(c.1) / 255.0), n(f64::from(c.2) / 255.0))
    }
    /// Pen and brush state for one op (inside its own q … Q).
    fn style(&mut self, pen: Option<&Pen>, brush: Option<Rgba>) {
        let pen = pen.filter(|p| !p.none);
        let sa = pen.map_or(255, |p| p.color.3);
        let fa = brush.map_or(255, |b| b.3);
        self.gs(sa, fa);
        if let Some(p) = pen {
            self.out += &format!("{} RG {} w {} J 1 j\n", Self::rgb(p.color), n(p.width), match p.cap {
                Cap::Flat => 0,
                Cap::Round => 1,
                Cap::Square => 2,
            });
            if !p.dash.is_empty() {
                let d: Vec<String> = p.dash.iter().map(|d| n(*d)).collect();
                self.out += &format!("[{}] 0 d\n", d.join(" "));
            }
        }
        if let Some(b) = brush {
            self.out += &format!("{} rg\n", Self::rgb(b));
        }
    }
    fn ring(&mut self, pts: &[[f64; 2]], close: bool) {
        for (i, p) in pts.iter().enumerate() {
            self.out += &format!("{} {} {}\n", n(p[0]), n(p[1]), if i == 0 { "m" } else { "l" });
        }
        if close {
            self.out += "h\n";
        }
    }
    fn paint_path(&mut self, pen: bool, brush: bool, closed: bool, even_odd: bool) {
        let op = match (pen, brush && closed) {
            (true, true) => if even_odd { "B*" } else { "B" },
            (false, true) => if even_odd { "f*" } else { "f" },
            (true, false) => "S",
            (false, false) => "n",
        };
        self.out += op;
        self.out += "\n";
    }
    fn set_clip(&mut self, rect: Option<&[[f64; 2]; 4]>, rings: &[Vec<[f64; 2]>]) {
        if self.clip_open {
            self.out += "Q\n";
        }
        self.out += "q\n";
        match rect {
            Some(r) => self.ring(r, true),
            None => {
                for ring in rings {
                    self.ring(ring, true);
                }
            }
        }
        self.out += "W* n\n";
        self.clip_open = true;
    }
    fn end_clip(&mut self) {
        if self.clip_open {
            self.out += "Q\n";
            self.clip_open = false;
        }
    }
    fn op(&mut self, op: &Op) {
        let pen = op.pen.as_ref().filter(|p| !p.none);
        match &op.prim {
            Prim::Clip { rect, rings } => return self.set_clip(rect.as_ref(), rings),
            Prim::Unclip { clip } => {
                self.end_clip();
                if let Some(Prim::Clip { rect, rings }) = clip.as_deref() {
                    self.set_clip(rect.as_ref(), rings);
                }
                return;
            }
            Prim::Text { .. } => return self.text(op),
            _ => {}
        }
        self.out += "q\n";
        self.style(pen, op.brush);
        match &op.prim {
            Prim::Line { pts } => {
                if pen.is_some() {
                    self.out += &format!("{} {} m {} {} l S\n", n(pts[0]), n(pts[1]), n(pts[2]), n(pts[3]));
                }
            }
            Prim::Poly { pts, closed } => {
                self.ring(pts, *closed);
                self.paint_path(pen.is_some(), op.brush.is_some(), *closed, true);
            }
            Prim::Ellipse { center: c, ax, ay } => {
                const K: f64 = 0.552_284_749_830_793_6;
                let at = |s: f64, t: f64| [c[0] + ax[0] * s + ay[0] * t, c[1] + ax[1] * s + ay[1] * t];
                let quads = [((1.0, 0.0), (0.0, 1.0)), ((0.0, 1.0), (-1.0, 0.0)), ((-1.0, 0.0), (0.0, -1.0)), ((0.0, -1.0), (1.0, 0.0))];
                let p0 = at(1.0, 0.0);
                self.out += &format!("{} {} m\n", n(p0[0]), n(p0[1]));
                for ((s0, t0), (s1, t1)) in quads {
                    let c1 = at(s0 + K * s1, t0 + K * t1);
                    let c2 = at(s1 + K * s0, t1 + K * t0);
                    let e = at(s1, t1);
                    self.out += &format!("{} {} {} {} {} {} c\n", n(c1[0]), n(c1[1]), n(c2[0]), n(c2[1]), n(e[0]), n(e[1]));
                }
                self.out += "h\n";
                self.paint_path(pen.is_some(), op.brush.is_some(), true, false);
            }
            Prim::RRect { pts, rx, ry } => {
                let ring = rounded(pts, *rx, *ry);
                self.ring(&ring, true);
                self.paint_path(pen.is_some(), op.brush.is_some(), true, false);
            }
            Prim::Path { rings } => {
                for r in rings {
                    self.ring(r, true);
                }
                self.paint_path(pen.is_some(), op.brush.is_some(), true, true);
            }
            Prim::Image { rect, source } => {
                let k = self.images.len();
                let name = self.images.entry(source.clone()).or_insert_with(|| format!("I{k}")).clone();
                // The unit square: (0,0) the rect's bottom-left, y toward its top.
                let (o, x1, y1) = (rect[3], rect[2], rect[0]);
                self.out += &format!("{} {} {} {} {} {} cm /{name} Do\n", n(x1[0] - o[0]), n(x1[1] - o[1]),
                                     n(y1[0] - o[0]), n(y1[1] - o[1]), n(o[0]), n(o[1]));
            }
            _ => {}
        }
        self.out += "Q\n";
    }

    /// Qt's `drawText(rect, flags, text)` on the sheet font's metrics.
    fn text(&mut self, op: &Op) {
        let Prim::Text { rect, flags, text, size, bold, italic, underline, color } = &op.prim else { return };
        let Some(lay) = text_lines(rect, *flags, text, *size, *bold) else { return };
        let (un, vn) = (lay.u, lay.v);
        let face = face_for(*bold);
        let fi = usize::from(*bold);
        let font = &fonts()[fi];
        self.out += "q\n";
        self.gs(255, color.3);
        self.out += &format!("{} rg\n", Self::rgb(*color));
        for line in &lay.lines {
            let o = line.origin;
            if !line.text.is_empty() {
                // Glyph space: x along the box, up against it (the page runs y down).
                let (a, b, c, d) = (un[0], un[1], -vn[0], -vn[1]);
                let (c, d) = if *italic { (c + 0.2 * a, d + 0.2 * b) } else { (c, d) };
                self.out += &format!("BT /F{fi} {} Tf {} {} {} {} {} {} Tm [", n(*size), n(a), n(b), n(c), n(d), n(o[0]), n(o[1]));
                let mut prev: Option<char> = None;
                for ch in line.text.chars() {
                    if let Some(p) = prev {
                        let kern = face.kern(p, ch);
                        if kern != 0.0 {
                            // Thousandths of an em; positive moves left.
                            self.out += &format!("> {} <", n(-kern * 10.0));
                        }
                    } else {
                        self.out += "<";
                    }
                    let g = font.glyph(ch);
                    self.used[fi].entry(g).or_insert(ch);
                    self.out += &format!("{g:04X}");
                    prev = Some(ch);
                }
                self.out += "> ] TJ ET\n";
            }
            if *underline && !line.text.is_empty() {
                let (a, b, t) = underline_of(&lay, line, *size);
                self.out += &format!("{} RG {} w 0 J {} {} m {} {} l S\n", Self::rgb(*color), n(t), n(a[0]), n(a[1]), n(b[0]), n(b[1]));
            }
        }
        self.out += "Q\n";
    }
}

/// One laid-out line: its baseline origin (paper mm), what it shows and its width.
pub struct Line {
    pub origin: [f64; 2],
    pub text: String,
    pub width: f64,
}

/// A text box laid out: the box's axes (unit, along and down) and its lines.
pub struct Laid {
    pub u: [f64; 2],
    pub v: [f64; 2],
    pub lines: Vec<Line>,
}

/// Qt's `drawText(rect, flags, text)` on the sheet font's metrics: typed
/// breaks (and word wrap with `TextWordWrap`), the first baseline at the
/// ascent, the lines a height and 1/64 apart, each aligned in the box.
pub fn text_lines(rect: &[[f64; 2]; 4], flags: i64, text: &str, size: f64, bold: bool) -> Option<Laid> {
    if text.is_empty() || size <= 0.0 {
        return None;
    }
    let (u, v) = ([rect[1][0] - rect[0][0], rect[1][1] - rect[0][1]], [rect[3][0] - rect[0][0], rect[3][1] - rect[0][1]]);
    let (w, h) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
    let un = if w > 0.0 { [u[0] / w, u[1] / w] } else { [1.0, 0.0] };
    let vn = if h > 0.0 { [v[0] / h, v[1] / h] } else { [0.0, 1.0] };
    let face: &Face = face_for(bold);
    let k = size / 100.0; // mm per metrics unit
    let raw: Vec<(String, bool)> = if flags & align::WORD_WRAP != 0 {
        wrap_lines(text, w / k, face)
    } else {
        text.split('\n').map(|l| (l.to_string(), false)).collect()
    };
    let line_h = (face.ascent + face.descent) * k;
    let step = line_h + k / 64.0;
    let block = line_h + (raw.len() - 1) as f64 * step;
    let y0 = if flags & align::BOTTOM != 0 {
        h - block
    } else if flags & align::VCENTER != 0 {
        (h - block) / 2.0
    } else {
        0.0
    };
    let lines = raw.iter().enumerate().map(|(i, (line, wrapped))| {
        let shown = if *wrapped { line.trim_end_matches(' ') } else { line.as_str() };
        let lw = face.advance(shown) * k;
        let dx = if flags & align::RIGHT != 0 {
            w - lw
        } else if flags & align::HCENTER != 0 {
            (w - lw) / 2.0
        } else {
            0.0
        };
        let by = y0 + face.ascent * k + i as f64 * step;
        Line { origin: [rect[0][0] + un[0] * dx + vn[0] * by, rect[0][1] + un[1] * dx + vn[1] * by],
               text: shown.to_string(), width: lw }
    }).collect();
    Some(Laid { u: un, v: vn, lines })
}

/// A line's underline: its ends and thickness.
pub fn underline_of(lay: &Laid, line: &Line, size: f64) -> ([f64; 2], [f64; 2], f64) {
    let (un, vn, o) = (lay.u, lay.v, line.origin);
    let d = 0.11 * size;
    let a = [o[0] + vn[0] * d, o[1] + vn[1] * d];
    let b = [a[0] + un[0] * line.width, a[1] + un[1] * line.width];
    (a, b, (size / 14.0).max(0.05))
}

pub fn rounded(p: &[[f64; 2]; 4], rx: f64, ry: f64) -> Vec<[f64; 2]> {
    let u = [p[1][0] - p[0][0], p[1][1] - p[0][1]];
    let v = [p[3][0] - p[0][0], p[3][1] - p[0][1]];
    let (w, h) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
    if w <= 0.0 || h <= 0.0 {
        return p.to_vec();
    }
    let (un, vn) = ([u[0] / w, u[1] / w], [v[0] / h, v[1] / h]);
    let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
    let mut out = Vec::new();
    let corners = [(w - rx, ry, -90.0f64), (w - rx, h - ry, 0.0), (rx, h - ry, 90.0), (rx, ry, 180.0)];
    for (cx, cy, start) in corners {
        for i in 0..=12 {
            let a = (start + 90.0 * f64::from(i) / 12.0).to_radians();
            let (x, y) = (cx + rx * a.cos(), cy + ry * a.sin());
            out.push([p[0][0] + un[0] * x + vn[0] * y, p[0][1] + un[1] * x + vn[1] * y]);
        }
    }
    out
}

fn font_objects(w: &mut Writer, font: &Font, used: &BTreeMap<u16, char>) -> usize {
    let file = w.stream(&format!("/Length1 {}", font.data.len()), font.data);
    let s = 1000.0 / font.upem;
    let sc = |v: i16| (f64::from(v) * s).round() as i64;
    let descriptor = w.add(format!(
        "<< /Type /FontDescriptor /FontName /{} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {file} 0 R >>",
        font.name, sc(font.bbox[0]), sc(font.bbox[1]), sc(font.bbox[2]), sc(font.bbox[3]),
        sc(font.ascent), sc(font.descent), sc(font.cap_height)).into_bytes());
    let widths: Vec<String> = used.keys().map(|g| format!("{g} [{}]", n(font.width(*g)))).collect();
    let cid = w.add(format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor} 0 R /CIDToGIDMap /Identity /DW 0 /W [{}] >>",
        font.name, widths.join(" ")).into_bytes());
    let mut cmap = String::from("/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Adobe-Identity-UCS def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n");
    let pairs: Vec<(&u16, &char)> = used.iter().collect();
    for chunk in pairs.chunks(100) {
        cmap += &format!("{} beginbfchar\n", chunk.len());
        for (g, c) in chunk {
            let mut units = [0u16; 2];
            let hex: String = c.encode_utf16(&mut units).iter().map(|u| format!("{u:04X}")).collect();
            cmap += &format!("<{g:04X}> <{hex}>\n");
        }
        cmap += "endbfchar\n";
    }
    cmap += "endcmap CMapName currentdict /CMap defineresource pop end end";
    let to_unicode = w.stream("", cmap.as_bytes());
    w.add(format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /{} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {to_unicode} 0 R >>",
        font.name).into_bytes())
}

fn image_object(w: &mut Writer, pic: &Picture) -> usize {
    let px = (pic.w * pic.h) as usize;
    let mut rgb = Vec::with_capacity(px * 3);
    let mut alpha = Vec::with_capacity(px);
    for c in pic.rgba.chunks_exact(4).take(px) {
        rgb.extend_from_slice(&c[..3]);
        alpha.push(c[3]);
    }
    let smask = if alpha.iter().any(|a| *a != 255) {
        let id = w.stream(&format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8",
                                   pic.w, pic.h), &alpha);
        format!(" /SMask {id} 0 R")
    } else {
        String::new()
    };
    w.stream(&format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Interpolate true{smask}",
                      pic.w, pic.h), &rgb)
}

/// The PDF of `pages`, with `pictures` for the image ops' sources (an
/// image item's file is read when it is not among them).
pub fn document(pages: &[Page], pictures: &HashMap<String, Picture>) -> Vec<u8> {
    let mut w = Writer { objects: Vec::new() };
    let catalog = w.reserve();
    let pages_id = w.reserve();
    let font_ids = [w.reserve(), w.reserve()];
    let mut used: [BTreeMap<u16, char>; 2] = [BTreeMap::new(), BTreeMap::new()];
    let mut kids = Vec::new();
    let mut loaded: HashMap<String, Option<usize>> = HashMap::new();
    for page in pages {
        let (pw, ph) = (page.size[0] * PT_PER_MM, page.size[1] * PT_PER_MM);
        let mut c = Content { out: String::new(), alphas: BTreeMap::new(), images: BTreeMap::new(),
                              used: [BTreeMap::new(), BTreeMap::new()], clip_open: false };
        // Paper millimetres, y down.
        c.out += &format!("{} 0 0 {} 0 {} cm\n", n(PT_PER_MM), n(-PT_PER_MM), n(ph));
        for item in &page.items {
            c.out += &format!("q 1 0 0 1 {} {} cm\n", n(item.origin[0]), n(item.origin[1]));
            for op in &item.ops {
                c.op(op);
            }
            c.end_clip();
            c.out += "Q\n";
        }
        for (i, u) in c.used.iter().enumerate() {
            used[i].extend(u.iter().map(|(g, ch)| (*g, *ch)));
        }
        let stream = w.stream("", c.out.as_bytes());
        let mut xobjects = String::new();
        for (source, name) in &c.images {
            let id = *loaded.entry(source.clone()).or_insert_with(|| {
                let file;
                let pic = match pictures.get(source) {
                    Some(p) => Some(p),
                    None => {
                        file = load_picture(source);
                        file.as_ref()
                    }
                };
                pic.filter(|p| p.w > 0 && p.h > 0 && p.rgba.len() >= (p.w * p.h * 4) as usize)
                    .map(|p| image_object(&mut w, p))
            });
            if let Some(id) = id {
                xobjects += &format!("/{name} {id} 0 R ");
            }
        }
        let gstates: String = c.alphas.iter()
            .map(|((s, f), name)| format!("/{name} << /CA {} /ca {} >> ", n(f64::from(*s) / 255.0), n(f64::from(*f) / 255.0)))
            .collect();
        let page_id = w.add(format!(
            "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Contents {stream} 0 R /Resources << /Font << /F0 {} 0 R /F1 {} 0 R >> /XObject << {xobjects}>> /ExtGState << {gstates}>> >> >>",
            n(pw), n(ph), font_ids[0], font_ids[1]).into_bytes());
        kids.push(page_id);
    }
    for (i, id) in font_ids.iter().enumerate() {
        if used[i].is_empty() {
            // Never drawn with: not embedded.
            w.set(*id, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
            continue;
        }
        let body = font_objects(&mut w, &fonts()[i], &used[i]);
        let obj = std::mem::take(&mut w.objects[body - 1]);
        w.set(*id, obj);
        w.set(body, b"null".to_vec());
    }
    let kid_refs: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
    w.set(pages_id, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kid_refs.join(" "), kids.len()).into_bytes());
    w.set(catalog, format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes());
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in w.objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", w.objects.len() + 1).into_bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").into_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n", w.objects.len() + 1).into_bytes());
    out
}

// ---- Python -------------------------------------------------------------------------

use super::model::ItemKind;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Write sheets ``comps`` to the PDF at ``path``, one page each, every
/// item in stacking order and the border on top, as `_paint_sheet` did.
/// ``contents``: ``{frame key: content}`` (see ``sheet_frame_display``);
/// ``pictures``: ``{source: (w, h, rgba bytes)}`` (``frame:<key>`` for a
/// frame's render); ``profiles``: the terrain profiles by item key;
/// ``scene``/``doc_path``: the field context (``{lamina}``, ``{archivo}``).
#[pyfunction]
#[pyo3(signature = (path, comps, contents=None, pictures=None, profiles=None, scene=None, doc_path=None))]
#[allow(clippy::too_many_arguments)]
pub fn sheet_pdf(py: Python<'_>, path: std::path::PathBuf, comps: Vec<Bound<'_, PyAny>>,
                 contents: Option<Bound<'_, PyDict>>, pictures: Option<Bound<'_, PyDict>>,
                 profiles: Option<Bound<'_, PyAny>>, scene: Option<Bound<'_, PyAny>>,
                 doc_path: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let pages = pages_of(py, &comps, contents.as_ref(), profiles.as_ref(), scene.as_ref(), doc_path.as_ref())?;
    let pics = pictures_of(pictures.as_ref())?;
    let bytes = py.detach(|| document(&pages, &pics));
    std::fs::write(&path, bytes).map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("{}: {e}", path.display())))
}

/// Each sheet as a page: its items' display lists in stacking order (frames
/// with their content), the border on top, the field context set per sheet.
pub fn pages_of(py: Python<'_>, comps: &[Bound<'_, PyAny>], contents: Option<&Bound<'_, PyDict>>,
                profiles: Option<&Bound<'_, PyAny>>, scene: Option<&Bound<'_, PyAny>>,
                doc_path: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Page>> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let composition = py.import("core.composition")?;
    // The field context is set per sheet and put back after: it is the
    // caller's (the page's sheet), never left on the last sheet written.
    let ctx = composition.getattr("_FIELD_CTX")?;
    let before = ctx.call_method0("copy")?;
    let pages = pages_in(py, &composition, comps, contents, profiles, scene, doc_path);
    ctx.call_method0("clear")?;
    ctx.call_method1("update", (before,))?;
    pages
}

#[allow(clippy::too_many_arguments)]
fn pages_in(py: Python<'_>, composition: &Bound<'_, PyModule>, comps: &[Bound<'_, PyAny>],
            contents: Option<&Bound<'_, PyDict>>, profiles: Option<&Bound<'_, PyAny>>,
            scene: Option<&Bound<'_, PyAny>>, doc_path: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Page>> {
    let mut pages = Vec::new();
    let total = comps.len();
    for (index, comp) in comps.iter().enumerate() {
        let kw = PyDict::new(py);
        kw.set_item("comp", comp)?;
        kw.set_item("scene", scene)?;
        kw.set_item("path", doc_path)?;
        kw.set_item("index", index)?;
        kw.set_item("total", total)?;
        composition.getattr("set_field_context")?.call((), Some(&kw))?;
        let (pw, ph): (f64, f64) = comp.call_method0("page_size_mm")?.extract()?;
        let mut items: Vec<(f64, Bound<'_, PyAny>)> = Vec::new();
        for m in comp.call_method0("all_items")?.try_iter()? {
            let m = m?;
            let z: f64 = m.getattr("z").and_then(|z| z.extract()).unwrap_or(0.0);
            items.push((z, m));
        }
        // sorted(..., key=z): stable, as Python's.
        items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut placed = Vec::new();
        for (_, m) in items {
            if m.getattr("hidden").and_then(|h| h.is_truthy()).unwrap_or(false) {
                continue;
            }
            let class = m.get_type().name()?.to_string();
            let Some(kind) = ItemKind::of_class(&class) else { continue };
            let key = crate::doc::py::handle_key(&m)?;
            let f = super::model::read_fields(key)?;
            let profile = match (profiles, kind) {
                (Some(p), ItemKind::Profile) if !p.is_none() => p.get_item(key).ok(),
                _ => None,
            };
            let x = super::display::extra(py, &m, kind, &f, profile.as_ref())?;
            let ops = if kind == ItemKind::Frame {
                let content = match contents {
                    Some(c) => c.get_item(key)?.map(|v| v.cast_into::<PyDict>()).transpose()?,
                    None => None,
                };
                let c = super::display::frame_content(content.as_ref(), key)?;
                let mut p = super::paint::Painter::default();
                super::items_d::paint_frame(&mut p, &f, &x, &c);
                p.ops
            } else {
                super::display::paint(kind, &f, &x).unwrap_or_default()
            };
            placed.push(Placed { origin: [f.num("x_mm", 0.0), f.num("y_mm", 0.0)], ops });
        }
        let cf = super::model::read_fields(crate::doc::py::handle_key(comp)?)?;
        let mut border = super::paint::Painter::default();
        super::items_c::paint_sheet_border(&mut border, &cf, pw, ph);
        placed.push(Placed { origin: [0.0, 0.0], ops: border.ops });
        pages.push(Page { size: [pw, ph], items: placed });
    }
    Ok(pages)
}

/// ``{source: (w, h, rgba bytes)}`` as pictures.
pub fn pictures_of(pictures: Option<&Bound<'_, PyDict>>) -> PyResult<HashMap<String, Picture>> {
    let mut pics = HashMap::new();
    if let Some(p) = pictures {
        for (k, v) in p.iter() {
            let (w, h, data): (u32, u32, Vec<u8>) = v.extract()?;
            pics.insert(k.extract::<String>()?, Picture { w, h, rgba: data });
        }
    }
    Ok(pics)
}

/// The picture an image op names that is not handed in: an image item's
/// file, composited as the item shows it (sheets::picture).
pub fn load_picture(source: &str) -> Option<Picture> {
    let (path, look) = super::picture::split_source(source);
    let im = image::open(path).ok()?.to_rgba8();
    let (w, h) = (im.width(), im.height());
    let raw = im.into_raw();
    let (w, h, rgba) = match look {
        Some(l) => super::picture::composite(&raw, w, h, &l),
        None => (w, h, raw),
    };
    Some(Picture { w, h, rgba })
}
