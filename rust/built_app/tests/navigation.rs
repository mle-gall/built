//! Exercise winit's gesture dispatch with isolated profiles and a synthetic slab.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn temporary_directory() -> PathBuf {
    let result = Command::new("mktemp").arg("-d").output().unwrap();
    assert!(result.status.success());
    String::from_utf8(result.stdout).unwrap().trim().into()
}

fn run(profile: Option<&str>, gestures: &str) -> Vec<Value> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let home = temporary_directory();
    let settings = temporary_directory();
    if let Some(profile) = profile {
        fs::create_dir(settings.join("IngeTrazo")).unwrap();
        fs::write(settings.join("IngeTrazo/IngeTrazo.ini"), profile).unwrap();
    }
    let script = home.join("gestures.txt");
    fs::write(&script, gestures).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_built"))
        .env("HOME", &home)
        .env("CFFIXED_USER_HOME", &home)
        .env("BUILT_SETTINGS_DIR", &settings)
        .current_dir(&root)
        .args(["--new-instance", "--script"])
        .arg(script)
        .arg(root.join("tests/fixtures/shell/slab-and-cube.igz"))
        .output()
        .unwrap();
    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(settings).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let states: Vec<Value> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("[built] state "))
        .map(|json| serde_json::from_str(json).unwrap())
        .collect();
    assert!(states.len() >= gestures.lines().filter(|line| *line == "state").count());
    assert!(
        states.iter().all(|s| s["errors"] == 0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    states
}
fn displacement(a: &Value, b: &Value, key: &str) -> f64 {
    (0..3)
        .map(|i| (a[key][i].as_f64().unwrap() - b[key][i].as_f64().unwrap()).powi(2))
        .sum::<f64>()
        .sqrt()
}
#[test]
#[ignore = "opens isolated native windows"]
fn settings_change_camera_motion() {
    let gestures = "wait 300\nstate\npan2 500 350 0 0 start\npan2 500 350 18 0\nstate\npan2 500 350 10 0 momentum\nstate\ntouch-slide 500 350 20 8\nstate\nrotate 500 350 9\nstate\npinch 500 350 0.16\nstate\nsmart-zoom 500 350\nstate\nsmart-zoom 500 350\nstate\nmenu select_all\nstate\ntouch-slide 280 190 16 0\nstate\nquit\n";
    let mut zooms = Vec::new();
    for (track, touch, inertia, sensitivity) in [
        ("pan", "orbit", false, 50),
        ("orbit", "pan", true, 50),
        ("pan", "select", false, 200),
    ] {
        let s = run(Some(&format!(
            "[nav]\ntrackpad_slide={track}\ntouch_slide={touch}\ntrackpad_inertia={}\npinch_sensitivity={sensitivity}\n", u8::from(inertia)
        )), gestures);
        assert_eq!(
            displacement(&s[0], &s[1], "camera_forward") > 1e-6,
            track == "orbit"
        );
        assert!(displacement(&s[0], &s[1], "camera_eye") > 0.);
        assert_eq!(displacement(&s[1], &s[2], "camera_eye") > 1e-6, inertia);
        assert_eq!(
            displacement(&s[2], &s[3], "camera_forward") > 1e-6,
            touch == "orbit"
        );
        assert_eq!(
            displacement(&s[2], &s[3], "camera_eye") > 1e-6,
            touch != "select"
        );
        assert_ne!(s[3]["camera_forward"], s[4]["camera_forward"]);
        let zoom = displacement(&s[4], &s[5], "camera_eye");
        assert!(zoom > 0.);
        assert!(displacement(&s[5], &s[6], "camera_eye") > 1e-6);
        assert!(displacement(&s[5], &s[7], "camera_eye") < 1e-6);
        if touch == "select" {
            assert!(s[8]["selection"].as_u64().unwrap() > 0);
            assert_eq!(s[9]["selection"], 0);
        }
        zooms.push(zoom);
    }
    assert!(zooms[2] > 2. * zooms[0]);
}

#[test]
#[ignore = "opens isolated native windows"]
fn trackpad_zooms_and_touchscreen_orbits_with_default_or_saved_zoom() {
    for profile in [None, Some("[nav]\ntrackpad_slide=zoom\n")] {
        let s = run(profile, "wait 300\nstate\npan2 500 350 0 0 start\npan2 500 350 0 -12\nstate\ntouch-slide 500 350 20 8\nstate\nquit\n");
        let before = s[0]["camera_distance"].as_f64().unwrap();
        let after = s[1]["camera_distance"].as_f64().unwrap();
        assert!((after - before).abs() > 1e-6);
        assert!(displacement(&s[0], &s[1], "camera_forward") < 1e-6);
        assert_eq!(s[1]["camera_distance"], s[2]["camera_distance"]);
        assert!(displacement(&s[1], &s[2], "camera_forward") > 1e-6);
    }
}

#[test]
#[ignore = "captures the app window on a native desktop"]
fn display_changes_live_and_restores_the_original_frame() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let home = temporary_directory();
    let settings = temporary_directory();
    let script = home.join("display.txt");
    fs::write(&script,"wait 300\nmsaa off\ncapture off\nmsaa 2\ncapture two\nmsaa 4\ncapture four\nmsaa off\ncapture restored\nquit\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_built"))
        .current_dir(&root)
        .env("HOME", &home)
        .env("CFFIXED_USER_HOME", &home)
        .env("BUILT_SETTINGS_DIR", &settings)
        .args(["--new-instance", "--size", "900x600", "--script"])
        .arg(script)
        .arg("--shots")
        .arg(&home)
        .arg(root.join("tests/fixtures/shell/slab-and-cube.igz"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.matches("[built] MSAA 1").count(), 2, "{stdout}");
    let crop = |name| {
        let file = fs::File::open(home.join(format!("{name}.png"))).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        let w = info.width as usize;
        let h = info.height as usize;
        pixels
            .chunks_exact(w * 4)
            .skip(h / 4)
            .take(h / 2)
            .flat_map(|row| row[w..w * 3].iter().copied())
            .collect::<Vec<_>>()
    };
    let off = crop("off");
    let mut previous = off.clone();
    for (count, name) in [(2, "two"), (4, "four")] {
        if stdout.contains(&format!("[built] MSAA {count}")) {
            let frame = crop(name);
            assert!(previous != frame, "{count}× had no visible effect");
            previous = frame;
        }
    }
    assert!(
        off == crop("restored"),
        "Off did not reproduce its initial frame"
    );
    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(settings).unwrap();
}
