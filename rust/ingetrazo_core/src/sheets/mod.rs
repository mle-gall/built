// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Sheets: the document side of the sheet composer (docs/rust-sheets.md).
//!
//! - `drawing`: a frame's line drawing (the classification pass of HLR);
//! - `svg`: the SVG writer of a frame's line groups;
//! - `model`: typed views of the sheet records in the document;
//! - `page`: the page display list the renderer paints.

pub mod camera;
pub mod create;
pub mod display;
pub mod drawing;
pub mod dxf;
pub mod geometry;
pub mod items;
pub mod items_b;
pub mod items_c;
pub mod items_d;
pub mod metrics;
pub mod paint;
pub mod pdf;
pub mod model;
pub mod page;
pub mod py;
pub mod profile;
pub mod pymath;
pub mod raster;
pub mod svg;
pub mod visibility;
