use std::{env, process::Command};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(built_bundle)");
    println!("cargo:rerun-if-env-changed=BUILT_MACOS_BUNDLE");
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || env::var_os("BUILT_MACOS_BUNDLE").is_none() {
        return;
    }
    let output = Command::new(env::var_os("PYO3_PYTHON").expect("set PYO3_PYTHON"))
        .args(["-c", "import sysconfig; print(sysconfig.get_path('include'))"])
        .output().expect("query Python headers");
    assert!(output.status.success(), "missing Python headers");
    let src = "../../packaging/native_python.c";
    println!("cargo:rerun-if-changed={src}");
    cc::Build::new().file(src)
        .include(String::from_utf8(output.stdout).unwrap().trim())
        .compile("native_python");
    println!("cargo:rustc-cfg=built_bundle");
}
