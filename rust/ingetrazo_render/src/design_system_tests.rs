//! Product-colour policy applies to the UI sources. Renderer colour spaces,
//! texture byte uploads and WGSL shaders are separate from chrome colours.
use std::{fs, path::Path};

fn check_source(path: &Path, exempt: &Path, failures: &mut Vec<String>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            check_source(&entry.unwrap().path(), exempt, failures);
        }
    } else if path.extension().is_some_and(|ext| ext == "rs") && path != exempt {
        let file = fs::read_to_string(path).unwrap();
        let source: String = file.lines().map(|l| l.split("//").next().unwrap())
            .flat_map(str::chars).filter(|c| !c.is_whitespace()).collect();
        // Include greys, alpha-only constructors, constants and fully qualified paths.
        if source.contains("Color32::") {
            failures.push(path.display().to_string());
        }
    }
}

#[test]
fn chrome_colors_come_from_tokens() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut failures = Vec::new();
    for entry in fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap().to_string_lossy().starts_with("ui") {
            check_source(&path, &src.join("ui/tokens.rs"), &mut failures);
        }
    }
    assert!(failures.is_empty(), "Use ui::tokens for chrome colours in {failures:?}");
}

#[test]
fn graphite_surfaces_are_achromatic() {
    use crate::ui::tokens::*;
    for c in [GLASS_BAR, PANEL, SHEET, CARD, CARD_HOVER, WELL, FIELD, SIDE, POPOVER, SCRIM] {
        assert_eq!((c.r(), c.g()), (c.g(), c.b()));
    }
}
