// SPDX-License-Identifier: GPL-3.0-or-later
//! Exact 2D drafting shared by the Sketch workspace, sketch recipe rows and
//! Library profiles (general-base §5.3 "Shared 2D kit").
//!
//! - [`geom`]: lines, counter-clockwise arcs, circles; intersections, bounds
//! - [`sketch`]: the sketch document (curves with persistent ids, named
//!   points, driving dimensions, use, plane, extrude)
//! - [`regions`]: loops, outer boundaries and holes, area, open ends
//! - [`edit`]: Trim, Extend, Fillet, Offset, driving dimensions
//! - [`cleanup`]: segmented arcs, small fillets, gaps, duplicates, units
//! - [`dxf`], [`svg`], [`import`]: drawings into sketches
//! - [`file`]: `.builtprofile` Library files
//! - [`snap`]: inference for the workspace's tools
pub mod cleanup;
pub mod dxf;
pub mod edit;
pub mod file;
pub mod geom;
pub mod import;
pub mod regions;
pub mod sketch;
pub mod snap;
pub mod svg;

pub use geom::Shape;
pub use regions::{profile, Profile};
pub use sketch::{Curve, Sketch, Use};
