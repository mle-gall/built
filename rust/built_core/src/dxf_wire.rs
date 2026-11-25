// SPDX-License-Identifier: GPL-3.0-or-later
//! Decode binary DXF group records to the same code/value stream as ASCII DXF.
use std::{borrow::Cow, io::Write};
const SIGNATURE: &[u8] = b"AutoCAD Binary DXF\r\n\x1a\0";

fn take<'a, const N: usize>(input: &mut &'a [u8]) -> Option<&'a [u8; N]> {
    let (value, tail) = input.split_first_chunk::<N>()?;
    *input = tail;
    Some(value)
}
fn binary_groups(mut input: &[u8]) -> Option<Vec<u8>> {
    // The first record is SECTION (code 0). R12 uses one byte for codes
    // below 255. Starting with R13 every code occupies two bytes.
    let wide = match input {
        [0, 0, ..] => true,
        [0, b'S', ..] => false,
        _ => return None,
    };
    let mut out = Vec::new();
    let mut groups = 0;
    while !input.is_empty() {
        groups += 1;
        if groups % 4096 == 0 && crate::file_open::cancelled() {
            return None;
        }
        let low = take::<1>(&mut input)?[0];
        let code = if wide {
            u16::from_le_bytes([low, take::<1>(&mut input)?[0]])
        } else if low == 255 {
            u16::from_le_bytes(*take(&mut input)?)
        } else {
            u16::from(low)
        };
        writeln!(out, "{code}").ok()?;
        match code {
            310..=319 | 1004 => {
                let count = take::<1>(&mut input)?[0] as usize;
                let (bytes, tail) = input.split_at_checked(count)?;
                input = tail;
                for b in bytes {
                    write!(out, "{b:02X}").ok()?;
                }
            }
            60..=79 | 170..=179 | 270..=289 | 370..=389 | 400..=409 | 1060..=1070 => {
                write!(out, "{}", i16::from_le_bytes(*take(&mut input)?)).ok()?
            }
            10..=59 | 110..=149 | 210..=239 | 460..=469 | 1010..=1059 => {
                write!(out, "{}", f64::from_le_bytes(*take(&mut input)?)).ok()?
            }
            90..=99 | 420..=429 | 440..=459 | 1071 => {
                write!(out, "{}", i32::from_le_bytes(*take(&mut input)?)).ok()?
            }
            160..=169 => write!(out, "{}", i64::from_le_bytes(*take(&mut input)?)).ok()?,
            290..=299 => write!(out, "{}", take::<1>(&mut input)?[0]).ok()?,
            _ => {
                let end = input.iter().position(|&b| b == 0)?;
                // DXF text records may use binary control characters. Keep
                // them within their record when consumed by the line reader.
                out.extend(input[..end].iter().map(|&b| {
                    if b == b'\n' || b == b'\r' {
                        b' '
                    } else {
                        b
                    }
                }));
                input = &input[end + 1..];
            }
        }
        out.push(b'\n');
    }
    Some(out)
}
fn decode_strings(bytes: &[u8]) -> Option<Cow<'_, str>> {
    let mut lines = bytes.split(|&b| b == b'\n');
    let mut variable = &[][..];
    let mut encoding = encoding_rs::WINDOWS_1252;
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        let code = code.trim_ascii();
        let value = value.trim_ascii();
        if code == b"0" && value == b"ENDSEC" {
            break;
        }
        if code == b"9" {
            variable = value;
            continue;
        }
        match variable {
            b"$ACADVER" if value >= b"AC1021".as_slice() => {
                return std::str::from_utf8(bytes).ok().map(Cow::Borrowed)
            }
            b"$DWGCODEPAGE" => {
                let label = match value.strip_prefix(b"ANSI_").unwrap_or(value) {
                    b"932" => b"shift_jis".as_slice(),
                    b"936" => b"gbk",
                    b"949" => b"euc-kr",
                    b"950" => b"big5",
                    b"874" => b"windows-874",
                    other => {
                        let label = format!("windows-{}", std::str::from_utf8(other).ok()?);
                        encoding = encoding_rs::Encoding::for_label(label.as_bytes())
                            .unwrap_or(encoding_rs::WINDOWS_1252);
                        variable = &[];
                        continue;
                    }
                };
                encoding = encoding_rs::Encoding::for_label(label)?;
            }
            _ => (),
        }
        variable = &[];
    }
    encoding.decode_without_bom_handling_and_without_replacement(bytes)
}
pub(super) fn text(bytes: &[u8]) -> Option<Cow<'_, str>> {
    if let Some(input) = bytes.strip_prefix(SIGNATURE) {
        let decoded = binary_groups(input)?;
        return Some(Cow::Owned(decode_strings(&decoded)?.into_owned()));
    }
    decode_strings(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_layer_names_match_reference() {
        for (bytes, layer) in [
            (
                include_bytes!("../../../tests/fixtures/dxf/binary-r12.dxf").as_slice(),
                "Café",
            ),
            (
                include_bytes!("../../../tests/fixtures/dxf/legacy-cp1251.dxf").as_slice(),
                "Слой",
            ),
        ] {
            let text = text(bytes).unwrap();
            let drawing = super::super::ascii::decode(&text)
                .unwrap()
                .drawing(0.01)
                .unwrap();
            assert_eq!(
                drawing.layers[layer].runs[0].0,
                [
                    glam::DVec3::new(1., 2., 3.),
                    glam::DVec3::new(-1.25, 5., 6.)
                ]
            );
        }
    }
    #[test]
    fn modern_binary_matches_reference() {
        let bytes = include_bytes!("../../../tests/fixtures/dxf/binary-r2018.dxf");
        let data = text(bytes).unwrap();
        let model = super::super::ascii::decode(&data)
            .unwrap()
            .drawing(0.01)
            .unwrap();
        let (line, _) = &model.layers["測量"].runs[0];
        assert_eq!(
            line.as_slice(),
            [
                glam::DVec3::new(1., 2., 3.),
                glam::DVec3::new(-1.25, 5., 6.)
            ]
        );
        assert!(text(&bytes[..bytes.len() - 1]).is_none());
    }
}
