use built_step::{planar::Body, read, step_io::build::Frame, write_document, Definition};
use glam::DVec3;

const NIST: &[u8] = include_bytes!("../../../tests/fixtures/step/NIST_MBE_PMI_1.stp");

#[test]
fn public_domain_ap203() {
    let exchange = read(NIST).unwrap();
    assert!(!exchange.synthetic_file_name);
    assert_eq!(exchange.report.entities_kept, 4113);
    assert_eq!(exchange.model.scene().all_solids().count(), 1);
}
#[test]
fn missing_name_in_derivative_only() {
    let source=b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('FILE_NAME;'),'2;1');\nFILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));\nENDSEC;\nDATA;\n#99=CARTESIAN_POINT('',(0.,0.,0.));\nENDSEC;\nEND-ISO-10303-21;";
    let exchange = read(source).unwrap();
    assert!(exchange.synthetic_file_name);
    assert_eq!(exchange.report.entities_kept, 1);
    assert_eq!(exchange.model.scene().all_points().count(), 1);
}
fn tetrahedron() -> Body {
    Body {
        points: vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z],
        faces: [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]]
            .map(|t| vec![t.to_vec()])
            .to_vec(),
    }
}
#[test]
fn repeated_occurrence_closed_volume_and_bounds() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("assembly.step");
    let at = |x| Frame {
        origin: [x, 0., 0.],
        axis: [0., 0., 1.],
        ref_dir: [1., 0., 0.],
    };
    let defs = [
        Definition {
            name: "assembly".into(),
            bodies: vec![],
            children: vec![(1, at(2.)), (1, at(0.))],
        },
        Definition {
            name: "tetrahedron".into(),
            bodies: vec![tetrahedron()],
            children: vec![],
        },
    ];
    write_document(&defs, &path).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let exchange = read(&bytes).unwrap();
    let scene = exchange.model.scene();
    assert_eq!(scene.all_edges().count(), 6);
    assert_eq!(scene.all_faces().count(), 4);
    let independent = stepmesh::read_step(&bytes, &Default::default()).unwrap();
    assert_eq!(independent.solids.len(), 2);
    let mut offsets = Vec::new();
    for solid in independent.solids {
        assert!((solid.volume() - 1e9 / 6.).abs() < 0.001);
        offsets.push(solid.bounds()[0][0] as i32);
    }
    offsets.sort();
    assert_eq!(offsets, [0, 2000]);
}
#[test]
fn open_shell_does_not_replace_destination() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("original.step");
    std::fs::write(&path, b"original").unwrap();
    let mut open = tetrahedron();
    open.faces.pop();
    let def = Definition {
        name: "incomplete".into(),
        bodies: vec![open],
        children: vec![],
    };
    assert!(write_document(&[def], &path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"original");
}
