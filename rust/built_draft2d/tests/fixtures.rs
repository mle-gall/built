// SPDX-License-Identifier: GPL-3.0-or-later
//! Import fixtures (tests/fixtures/sketch): segmented arcs, gaps, an inch
//! file without units, bulges and SVG arcs, through read, clean and profile.
use built_draft2d::{
    cleanup::{fix_all, issues, Issue},
    import::{read, sketch},
    profile, Shape, Use,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/../../tests/fixtures/sketch/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn an_inch_drawing_without_units_doubts_them_and_cleans_into_one_closed_profile() {
    let d = read("dxf", &fixture("zed-inch-segmented.dxf")).unwrap();
    assert_eq!(d.unit, None);
    let imported = sketch(&d, None, Use::Profile, 0.5);
    // Annotation layers are off by default.
    let kept: Vec<_> = imported.layers.iter().filter(|l| l.2).map(|l| l.0.as_str()).collect();
    assert_eq!(kept, ["Contour"]);
    let keys: Vec<_> = imported.issues.iter().map(Issue::key).collect();
    assert_eq!(keys, ["segmented", "gaps", "units"], "{:?}", imported.issues);
    let Issue::Segmented { runs } = &imported.issues[0] else { panic!() };
    assert_eq!((runs.len(), imported.issues[0].count()), (4, 24));
    let Issue::Units { factor, .. } = imported.issues[2] else { panic!() };
    assert_eq!(factor, 25.4);
    let mut s = imported.sketch;
    assert!(!profile(&s).ok());
    fix_all(&mut s, 0.5, None);
    assert!(issues(&s, 0.5, Some("mm")).is_empty(), "{:?}", issues(&s, 0.5, Some("mm")));
    let p = profile(&s);
    assert!(p.ok(), "{:?}", p.problems);
    assert_eq!(s.curves.iter().filter(|c| matches!(c.shape, Shape::Arc { .. })).count(), 4);
    // 200 mm high once in millimetres.
    let (lo, hi) = s.bounds().unwrap();
    assert!(((hi.y - lo.y) - 200.0).abs() < 0.01, "{}", hi.y - lo.y);
}

#[test]
fn a_millimetre_plate_keeps_its_arcs_and_bulges_exact() {
    let d = read("dxf", &fixture("plate-mm.dxf")).unwrap();
    assert_eq!(d.unit.as_deref(), Some("mm"));
    let imported = sketch(&d, None, Use::Profile, 0.5);
    let mut s = imported.sketch;
    let holes: Vec<_> = s.curves.iter().filter(|c| matches!(c.shape, Shape::Arc { r, .. } if (r - 5.0).abs() < 1e-9)).collect();
    assert_eq!(holes.len(), 2, "the bulged hole is two exact half circles");
    let keys: Vec<_> = imported.issues.iter().map(Issue::key).collect();
    assert_eq!(keys, ["fillets", "gaps"]);
    fix_all(&mut s, 0.5, Some("mm"));
    let p = profile(&s);
    assert!(p.ok(), "{:?}", p.problems);
    assert_eq!(p.regions.len(), 1);
    assert_eq!(p.regions[0].holes.len(), 1);
    let expect = 5000.0 - (100.0 - 25.0 * std::f64::consts::PI) - 25.0 * std::f64::consts::PI;
    assert!((p.area - expect).abs() < 1e-6, "{}", p.area);
}

#[test]
fn an_svg_notch_reads_true_size_with_its_arc() {
    let d = read("svg", &fixture("notch-mm.svg")).unwrap();
    let imported = sketch(&d, None, Use::Section, 0.5);
    assert_eq!(imported.layers.iter().filter(|l| l.2).map(|l| l.0.as_str()).collect::<Vec<_>>(), ["Notch"]);
    let s = imported.sketch;
    let p = profile(&s);
    assert!(p.ok(), "{:?}", p.problems);
    let (lo, hi) = s.bounds().unwrap();
    assert!(((hi.x - lo.x) - 55.0).abs() < 1e-9 && ((hi.y - lo.y) - 20.0).abs() < 1e-9);
    assert_eq!(s.curves.iter().filter(|c| matches!(c.shape, Shape::Arc { .. })).count(), 1);
    assert!(imported.issues.is_empty());
}
