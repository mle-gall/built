//! Run explicitly on a desktop with the native Python environment installed.
use std::{fs, io::BufReader, path::Path, process::Command};

#[test]
#[ignore = "opens a native window"]
fn welcome_survives_resize() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let run = Command::new("mktemp").arg("-d").output().unwrap();
    assert!(run.status.success());
    let shots = std::path::PathBuf::from(String::from_utf8(run.stdout).unwrap().trim());
    let output = Command::new(root.join(".venv/bin/python"))
        .current_dir(&root)
        .arg("scripts/isolated_launch.py")
        .arg(env!("CARGO_BIN_EXE_built"))
        .args([
            "--new-instance",
            "--script",
            "rust/built_app/scripts/start-resize.txt",
            "--shots",
        ])
        .arg(&shots)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("\"large_sheet\": \"start\"") && stdout.contains("\"errors\": 0"),
        "{stdout}"
    );
    assert!(
        stdout.contains("slab-and-cube.igz") && !stdout.contains("\"faces\": 0,"),
        "{stdout}"
    );
    for (name, width, height, left, right) in [
        ("after-small", 1000, 650, 24, 976),
        ("after-large", 1420, 810, 88, 1104),
        ("after-model", 1420, 810, 88, 1104),
    ] {
        let path = shots.join(format!("{name}.png"));
        let mut reader = png::Decoder::new(BufReader::new(fs::File::open(path).unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        let scale = info.width as f64 / width as f64;
        assert_eq!(info.height, (height as f64 * scale).round() as u32);
        let mut gutter = Vec::new();
        for (index, rgba) in pixels.chunks_exact(4).enumerate() {
            let x = (index % info.width as usize) as f64 / scale;
            let y = (index / info.width as usize) as f64 / scale;
            if (left as f64..right as f64).contains(&x) && (56.0..height as f64 - 48.0).contains(&y)
            {
                continue;
            }
            assert!(
                !rgba[..3].iter().all(|&v| v >= 246),
                "{name} white at ({x}, {y})"
            );
            if (2.0..4.0).contains(&x) && (56.0..height as f64 - 48.0).contains(&y) {
                gutter.push(rgba[1] as f64);
            }
        }
        // Compare averaged bands so glass dithering alone cannot pass.
        let n = gutter.len() / 8;
        let mean = |strip: &[f64]| strip.iter().sum::<f64>() / strip.len() as f64;
        assert!(mean(&gutter) > 20.0, "{name} black gutter");
        assert!(
            (mean(&gutter[..n]) - mean(&gutter[gutter.len() - n..])).abs() > 1.0,
            "{name} flat gutter"
        );
    }
    fs::remove_dir_all(shots).unwrap();
}
