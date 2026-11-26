//! Stamp both the embedded renderer and the wheel with the source revision.
use std::{env, path::PathBuf, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let result = Command::new("git").args(args).output().ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=BUILT_COMMIT");
    // Given the revision (CI does), the build does not watch git's refs: a
    // fresh checkout writes them anew, which recompiled an unchanged renderer.
    let given = env::var_os("BUILT_COMMIT").is_some();
    for reference in [
        Some("HEAD".into()),
        git(&["symbolic-ref", "-q", "HEAD"]),
        Some("packed-refs".into()),
    ]
    .into_iter()
    .flatten()
    {
        if given {
            break;
        }
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            if PathBuf::from(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    let commit = env::var("BUILT_COMMIT")
        .ok()
        .or_else(|| git(&["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "unavailable".into());
    println!("cargo:rustc-env=BUILT_COMMIT={commit}");
}
