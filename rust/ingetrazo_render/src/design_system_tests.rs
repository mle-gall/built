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

fn check_source(path: &Path, exempt: &Path, violates: fn(&str) -> bool, failures: &mut Vec<String>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            check_source(&entry.unwrap().path(), exempt, violates, failures);
        }
    } else if path.extension().is_some_and(|ext| ext == "rs") && path != exempt {
        let file = fs::read_to_string(path).unwrap();
        if violates(&file) {
            failures.push(path.display().to_string());
        }
    }
}

fn check_chrome(violates: fn(&str) -> bool, policy: &str) {
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
            check_source(&path, &src.join("ui/tokens.rs"), violates, &mut failures);
        }
    }
    assert!(
        failures.is_empty(),
        "{policy} in {failures:?}"
    );
}

#[test]
fn chrome_colors_come_from_tokens() {
    check_chrome(declares_colour, "Use ui::tokens for chrome colours");
}

fn literal_font_size(source: &str) -> bool {
    use syn::{visit::{self, Visit}, Expr, Lit};
    fn numeric(expr: &Expr) -> bool {
        matches!(expr, Expr::Lit(v) if matches!(v.lit, Lit::Int(_) | Lit::Float(_)))
    }
    #[derive(Default)]
    struct Scan(bool);
    impl<'a> Visit<'a> for Scan {
        fn visit_expr_call(&mut self, call: &'a syn::ExprCall) {
            if let Expr::Path(path) = &*call.func {
                let mut names = path.path.segments.iter().rev();
                let name = names.next().unwrap().ident.to_string();
                let font = name == "strong" || names.next().is_some_and(|s| s.ident == "FontId")
                    && matches!(name.as_str(), "new" | "proportional" | "monospace");
                self.0 |= font && call.args.first().is_some_and(numeric);
            }
            visit::visit_expr_call(self, call);
        }
        fn visit_expr_struct(&mut self, expr: &'a syn::ExprStruct) {
            if expr.path.segments.last().is_some_and(|s| s.ident == "FontId") {
                self.0 |= expr.fields.iter().any(|f| matches!(&f.member, syn::Member::Named(n) if n == "size") && numeric(&f.expr));
            }
            visit::visit_expr_struct(self, expr);
        }
        fn visit_expr_method_call(&mut self, call: &'a syn::ExprMethodCall) {
            // size() is also a geometry setter on egui widgets. Follow the
            // receiver chain to distinguish inline RichText construction.
            let mut receiver = &*call.receiver;
            while let Expr::MethodCall(method) = receiver {
                receiver = &method.receiver;
            }
            let rich_text = matches!(receiver, Expr::Call(c) if matches!(&*c.func, Expr::Path(p)
                if p.path.segments.iter().any(|s| s.ident == "RichText")));
            self.0 |= (call.method == "font_size" || call.method == "size" && rich_text)
                && call.args.first().is_some_and(numeric);
            visit::visit_expr_method_call(self, call);
        }
    }
    let mut scan = Scan::default();
    scan.visit_file(&syn::parse_file(source).unwrap());
    scan.0
}

#[test]
fn chrome_font_sizes_come_from_tokens() {
    check_chrome(literal_font_size, "Use ui::tokens for chrome type");
}

#[test]
fn recognises_literal_font_sizes() {
    for expr in ["egui::FontId::new(12., family)", "strong(12.5)",
                 "egui::RichText::new(text).color(ink).size(9.0)", "FontId { size: 15., family }"] {
        assert!(literal_font_size(&format!("fn example() {{ {expr}; }}")), "{expr}");
    }
    assert!(!literal_font_size("fn example() { strong(TYPE_BODY); Spinner::new().size(14.0); }"));
}

#[test]
fn graphite_surfaces_are_achromatic() {
    use crate::ui::tokens::*;
    for c in [
        GLASS_BAR, PANEL, SHEET, CARD, CARD_HOVER, WELL, FIELD, SIDE, POPOVER, SCRIM, CONTROL, TRACK,
        THUMB, HOVER, PRESS,
    ] {
        assert_eq!((c.r(), c.g()), (c.g(), c.b()));
    }
}

#[test]
fn glass_levels_lighten_and_lift_with_elevation() {
    use crate::ui::tokens::Surface;
    let levels = Surface::ALL;
    for pair in levels.windows(2) {
        assert!(pair[0].grey() < pair[1].grey(), "{:?} vs {:?}", pair[0], pair[1]);
        assert!(pair[0].shadow().color.a() < pair[1].shadow().color.a());
    }
}

#[test]
fn type_scale_has_four_sizes() {
    use crate::ui::tokens::*;
    let sizes = [meta().size, body().size, title().size, heading().size, caption_font().size];
    let mut distinct: Vec<f32> = sizes.to_vec();
    distinct.sort_by(f32::total_cmp);
    distinct.dedup();
    assert_eq!(distinct, [TYPE_META, TYPE_BODY, TYPE_TITLE, TYPE_HEADING]);
}
