// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed STEP exchange. Source bytes and Part 21 identifiers survive diagnostics.
use std::io::Read;
pub use step_io;
use step_io::{Error, Report, StepModel};

pub struct Exchange {
    pub model: StepModel,
    pub report: Report,
    pub synthetic_file_name: bool,
}

pub fn read(source: &[u8]) -> Result<Exchange, Error> {
    let (model, report, synthetic_file_name) = match step_io::read(source) {
        Ok((model, report)) => (model, report, false),
        Err(e @ Error::MissingHeaderEntity { name: "FILE_NAME" }) => {
            let Some(offset) = missing_name_offset(source) else {
                return Err(e);
            };
            const RECORD: &[u8] = b"\nFILE_NAME('','',(''),(''),'','','');\n";
            let (model, report) =
                step_io::read(source[..offset].chain(RECORD).chain(&source[offset..]))?;
            (model, report, true)
        }
        Err(e) => return Err(e),
    };
    Ok(Exchange {
        model,
        report,
        synthetic_file_name,
    })
}

fn missing_name_offset(source: &[u8]) -> Option<usize> {
    use ruststep::parser::exchange::{header_entity, header_section};
    // Diagnose only the header. Reading arbitrary body bytes as UTF-8 would
    // prevent recovery of legacy encodings in product names.
    let prefix = &source[..source.len().min(1024 * 1024)];
    let text = match std::str::from_utf8(prefix) {
        Ok(text) => text,
        Err(e) => std::str::from_utf8(&prefix[..e.valid_up_to()]).ok()?,
    };
    let header = text
        .trim_start()
        .strip_prefix("ISO-10303-21;")?
        .trim_start();
    let (_, records) = header_section(header).ok()?;
    if records.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()
        != ["FILE_DESCRIPTION", "FILE_SCHEMA"]
    {
        return None;
    }
    let (after_description, _) =
        header_entity(header.strip_prefix("HEADER;")?.trim_start()).ok()?;
    Some(text.len() - after_description.len())
}

pub mod planar;

use std::io::Write;
use std::path::Path;
use step_io::build::Frame;

pub struct Definition {
    pub name: String,
    pub bodies: Vec<planar::Body>,
    /// Direct child definitions with rigid, definition-local placements in metres.
    pub children: Vec<(usize, Frame)>,
}

/// Writes planar solids, one product per definition. Validate the file on disk
/// before replacing an existing destination, including for in-place conversions.
pub fn write_document(definitions: &[Definition], path: &Path) -> Result<(), String> {
    let mut builder = step_io::StepBuilder::new();
    let parts: Vec<_> = definitions.iter().map(|d| builder.part(&d.name)).collect();
    let mut volumes = Vec::new();
    for (definition, &part) in definitions.iter().zip(&parts) {
        for body in &definition.bodies {
            volumes.push(body.add(&mut builder, part, &definition.name)?);
        }
        for (child, at) in &definition.children {
            let &child = parts.get(*child).ok_or("invalid occurrence target")?;
            let rigid = |a: [f64; 3]| glam::DVec3::from(a);
            if (rigid(at.axis).length_squared() - 1.).abs() > 1e-12
                || (rigid(at.ref_dir).length_squared() - 1.).abs() > 1e-12
                || rigid(at.axis).dot(rigid(at.ref_dir)).abs() > 1e-12
                || ![at.origin, at.axis, at.ref_dir]
                    .into_iter()
                    .flatten()
                    .all(f64::is_finite)
            {
                return Err("invalid rigid placement".into());
            }
            builder
                .place(
                    part,
                    child,
                    Frame {
                        origin: at.origin.map(|v| v * 1000.),
                        axis: at.axis,
                        ref_dir: at.ref_dir,
                    },
                )
                .map_err(|e| e.to_string())?;
        }
    }
    if volumes.is_empty() {
        return Err("no STEP bodies".into());
    }
    let mut tmp = tempfile::NamedTempFile::new_in(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .map_err(|e| e.to_string())?;
    {
        let mut out = std::io::BufWriter::new(tmp.as_file_mut());
        step_io::write(&builder.finish(), &mut out).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
    let check =
        step_io::read(tmp.reopen().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let scene = check.0.scene();
    let metres = scene.units().length.ok_or("missing length unit")?.to_si;
    let mut restored = Vec::new();
    for solid in scene.all_solids() {
        restored.push(planar::Body::from_solid(solid, metres)?.volume()?);
    }
    volumes.sort_by(f64::total_cmp);
    restored.sort_by(f64::total_cmp);
    if !check.1.dropped.is_empty()
        || scene.all_product_definitions().count() != parts.len()
        || scene
            .all_product_definitions()
            .map(|d| d.occurrences().count())
            .sum::<usize>()
            != definitions.iter().map(|d| d.children.len()).sum::<usize>()
        || restored.len() != volumes.len()
        || restored
            .iter()
            .zip(&volumes)
            .any(|(a, b)| !a.is_finite() || (a - b).abs() > 1e-21_f64.max(1e-7 * b.abs()))
    {
        return Err("STEP reopen verification failed".into());
    }
    tmp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
