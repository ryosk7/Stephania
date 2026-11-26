use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/x264_bridge.c");
    if env::var_os("CARGO_FEATURE_NATIVE_X264").is_some() { build_x264(); }
    println!("cargo:rerun-if-changed=src/aac_bridge.c");
    if env::var_os("CARGO_FEATURE_NATIVE_AAC").is_none() { return; }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("aac_bridge.o");
    let archive = out.join("libt3box_aac_bridge.a");
    let flags = Command::new("pkg-config").args(["--cflags", "libavcodec", "libavutil"])
        .output().expect("pkg-config required");
    assert!(flags.status.success(), "install libavcodec-dev and libavutil-dev");
    let mut cc = Command::new(env::var_os("CC").unwrap_or_else(|| "cc".into()));
    cc.args(["-std=c11", "-O2", "-fPIC", "-c", "src/aac_bridge.c", "-o"]).arg(&object);
    cc.args(String::from_utf8(flags.stdout).expect("pkg-config UTF-8").split_whitespace());
    assert!(cc.status().expect("C compiler required").success(), "AAC bridge compile failed");
    assert!(Command::new(env::var_os("AR").unwrap_or_else(|| "ar".into()))
        .arg("crs").arg(&archive).arg(&object).status().expect("ar required").success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=t3box_aac_bridge");
    println!("cargo:rustc-link-lib=avcodec");
    println!("cargo:rustc-link-lib=avutil");
}

fn build_x264() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("x264_bridge.o");
    let archive = out.join("libt3box_x264_bridge.a");
    let flags = Command::new("pkg-config").args(["--cflags", "x264"])
        .output().expect("pkg-config required");
    assert!(flags.status.success(), "install libx264-dev");
    let mut cc = Command::new(env::var_os("CC").unwrap_or_else(|| "cc".into()));
    cc.args(["-std=c11", "-O2", "-fPIC", "-c", "src/x264_bridge.c", "-o"]).arg(&object);
    cc.args(String::from_utf8(flags.stdout).expect("pkg-config UTF-8").split_whitespace());
    assert!(cc.status().expect("C compiler required").success(), "x264 bridge compile failed");
    assert!(Command::new(env::var_os("AR").unwrap_or_else(|| "ar".into()))
        .arg("crs").arg(&archive).arg(&object).status().expect("ar required").success());
    println!("cargo:rustc-link-search=native={}", out.display());
    let libs = Command::new("pkg-config").args(["--libs-only-L", "x264"])
        .output().expect("pkg-config required");
    assert!(libs.status.success(), "libx264 link search failed");
    for flag in String::from_utf8(libs.stdout).expect("pkg-config UTF-8").split_whitespace() {
        if let Some(path) = flag.strip_prefix("-L") { println!("cargo:rustc-link-search=native={path}"); }
    }
    println!("cargo:rustc-link-lib=static=t3box_x264_bridge");
    println!("cargo:rustc-link-lib=x264");
}
