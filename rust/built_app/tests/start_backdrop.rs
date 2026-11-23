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
    let script = shots.join("start-and-preview.txt");
    let resize_script = include_str!("../scripts/start-resize.txt")
        .trim_end()
        .strip_suffix("quit")
        .unwrap();
    // Save a real thumbnail in an isolated document, then close its tab.
    let preview_file = shots.join("recent.built");
    fs::write(
        &script,
        format!(
            "{resize_script}menu save_to:{}\nwait 400\nmenu close_tab\nwait 1200\nstate\nquit\n",
            preview_file.display()
        ),
    )
    .unwrap();
    let output = Command::new(root.join(".venv/bin/python"))
        .current_dir(&root)
        .arg("scripts/isolated_launch.py")
        .arg(env!("CARGO_BIN_EXE_built"))
        .args(["--new-instance", "--script"])
        .arg(&script)
        .arg("--shots")
        .arg(&shots)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let states: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("[built] state "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(states.len(), 7, "{stdout}");
    for (state, sheet) in states
        .iter()
        .zip([true, true, false, true, false, true, true])
    {
        assert_eq!(state["errors"], 0, "{state}");
        assert_eq!(state["large_sheet"], if sheet { "start" } else { "" });
        for area in ["tool_rail", "dock", "bottom_bar"] {
            assert_eq!(state["input"][area], !sheet, "{area} in {state}");
        }
        assert_eq!(state["cluster"].as_array().unwrap().is_empty(), sheet);
    }
    assert_eq!(states[3]["faces"], 6);
    assert_eq!(states[3]["input"]["recent_preview"], false);
    assert_eq!(states[5]["input"]["recent_preview"], true);
    let mut blank_samples = Vec::new();
    for (name, width, height) in [
        ("after-small", 1000, 650),
        ("after-large", 1420, 810),
        ("after-model", 1420, 810),
        ("after-model-small", 1000, 650),
    ] {
        let path = shots.join(format!("{name}.png"));
        let mut reader = png::Decoder::new(BufReader::new(fs::File::open(path).unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        let scale = info.width as f64 / width as f64;
        assert_eq!(info.height, (height as f64 * scale).round() as u32);
        let margin = ((width as f64 * 0.06).clamp(24.0, 80.0)).max((width as f64 - 1180.0) / 2.0);
        let top = 40.0 + (height as f64 - 40.0) * 0.08;
        let mut samples = Vec::new();
        for (index, rgba) in pixels.chunks_exact(4).enumerate() {
            let x = (index % info.width as usize) as f64 / scale;
            let y = (index / info.width as usize) as f64 / scale;
            let in_card = (margin..width as f64 - margin).contains(&x)
                && (top..height as f64 - top + 40.0).contains(&y);
            if !in_card {
                assert!(
                    !rgba[..3].iter().all(|&v| v >= 246),
                    "{name} white at ({x}, {y})"
                );
            }
            // The lower left body is empty in both Start states. Keep away
            // from controls, dividers, text and the sheet's outer shadow.
            if (0.12..0.48).contains(&(x / width as f64))
                && (0.70..0.80).contains(&(y / height as f64))
            {
                samples.push(rgba[1]);
            }
        }
        let (low, high) = (samples.iter().min().unwrap(), samples.iter().max().unwrap());
        assert!(high - low > 5, "{name}: uniform glass body ({low}..{high})");
        if name == "after-large" {
            blank_samples = samples;
        } else if name == "after-model" {
            let changed = samples
                .iter()
                .zip(&blank_samples)
                .filter(|(a, b)| a.abs_diff(**b) > 3)
                .count();
            assert!(
                changed > samples.len() / 100,
                "the scene change must show through the card"
            );
        }
    }
    fs::remove_dir_all(shots).unwrap();
}
