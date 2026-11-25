// The module carries a fingerprint of the sources it was built from, so a
// test can tell an installed module that no longer matches rust/ (a stale
// build would measure yesterday's kernels). FNV-1a 64 over Cargo.toml,
// Cargo.lock, Rust sources and embedded JSON by path, CR dropped; tests/test_rust_engine_switch
// computes the same.
use std::fs;
use std::path::Path;

fn main() {
    let mut files = vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()];
    // Include modules and their embedded records (src/store/ was once missed:
    // a wheel with a stale store passed the check).
    fn walk(dir: &str, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let name = format!("{dir}/{}", e.file_name().to_string_lossy());
            if e.path().is_dir() {
                walk(&name, out);
            } else if name.ends_with(".rs") || name.ends_with(".json") {
                out.push(name);
            }
        }
    }
    let mut srcs = Vec::new();
    walk("src", &mut srcs);
    srcs.sort();
    files.extend(srcs);
    let mut h: u64 = 0xcbf29ce484222325;
    for f in &files {
        println!("cargo:rerun-if-changed={f}");
        let bytes = fs::read(Path::new(f)).unwrap();
        for &b in f.as_bytes().iter().chain([0u8].iter()).chain(bytes.iter()) {
            if b == b'\r' {
                continue;
            }
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rustc-env=BUILT_SOURCE_HASH={h:016x}");
}
