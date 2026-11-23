//! Product-colour policy applies to the UI sources. Renderer colour spaces,
//! texture byte uploads and WGSL shaders are separate from chrome colours.
use proc_macro2::{TokenStream, TokenTree};
use std::{collections::HashSet, fs, path::Path};

fn tokens(stream: TokenStream, words: &mut Vec<String>) {
    for token in stream {
        match token {
            TokenTree::Group(group) => {
                words.push("(".into());
                tokens(group.stream(), words);
                words.push(")".into());
            }
            TokenTree::Literal(_) => words.push("literal".into()),
            _ => words.push(token.to_string()),
        }
    }
}

fn declares_colour(source: &str) -> bool {
    let mut words = Vec::new();
    tokens(source.parse().expect("Rust token stream"), &mut words);
    let mut names: HashSet<&str> = ["Color32", "Rgba", "Hsva", "HsvaGamma"].into();
    for w in words.windows(3) {
        if w[1] == "as" && names.contains(w[0].as_str()) {
            names.insert(&w[2]);
        }
    }
    for w in words.windows(4) {
        if w[0] == "type" && w[2] == "=" && names.contains(w[3].as_str()) {
            names.insert(&w[1]);
        }
    }
    words
        .windows(3)
        .any(|w| names.contains(w[0].as_str()) && w[1] == ":" && w[2] == ":")
}

#[test]
fn recognises_qualified_and_aliased_colour_constructors() {
    assert!(declares_colour("egui::Color32 /* role */ :: from_gray(12)"));
    assert!(declares_colour(
        "use egui::Color32 as C; C::from_black_alpha(22)"
    ));
    assert!(declares_colour("type Ink = Color32; Ink::RED"));
    assert!(!declares_colour("egui::ColorImage::from_rgb(size, pixels)"));
    assert!(!declares_colour(r#"let help = "Color32::RED";"#));
}

fn check_source(path: &Path, exempt: &Path, failures: &mut Vec<String>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            check_source(&entry.unwrap().path(), exempt, failures);
        }
    } else if path.extension().is_some_and(|ext| ext == "rs") && path != exempt {
        let file = fs::read_to_string(path).unwrap();
        if declares_colour(&file) {
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
        if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("ui")
            || path.ends_with("inspector")
        {
            check_source(&path, &src.join("ui/tokens.rs"), &mut failures);
        }
    }
    assert!(
        failures.is_empty(),
        "Use ui::tokens for chrome colours in {failures:?}"
    );
}

#[test]
fn graphite_surfaces_are_achromatic() {
    use crate::ui::tokens::*;
    for c in [
        GLASS_BAR, PANEL, SHEET, CARD, CARD_HOVER, WELL, FIELD, SIDE, POPOVER, SCRIM,
    ] {
        assert_eq!((c.r(), c.g()), (c.g(), c.b()));
    }
}
