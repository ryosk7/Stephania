use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/x264_bridge.c");
    if env::var_os("CARGO_FEATURE_NATIVE_X264").is_some() { build_x264(); }
    println!("cargo:rerun-if-changed=src/aac_bridge.c");
    println!("cargo:rerun-if-changed=src/h264dec_bridge.c");
    if env::var_os("CARGO_FEATURE_NATIVE_AAC").is_none() { return; }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let archive = out.join("libstephania_aac_bridge.a");
    let flags = Command::new("pkg-config").args(["--cflags", "libavcodec", "libavutil"])
        .output().expect("pkg-config required");
    assert!(flags.status.success(), "install libavcodec-dev and libavutil-dev");
    // The same libavcodec archive carries the AAC encoder and the H.264
    // decoder for phone camera tracks (crate::h264_decode).
    let cflags = String::from_utf8(flags.stdout).expect("pkg-config UTF-8");
    let mut objects = Vec::new();
    for src in ["aac_bridge", "h264dec_bridge"] {
        let object = out.join(format!("{src}.o"));
        let mut cc = Command::new(env::var_os("CC").unwrap_or_else(|| "cc".into()));
        cc.args(["-std=c11", "-O2", "-fPIC", "-c"]).arg(format!("src/{src}.c")).arg("-o").arg(&object);
        cc.args(cflags.split_whitespace());
        assert!(cc.status().expect("C compiler required").success(), "{src} compile failed");
        objects.push(object);
    }
    let _ = std::fs::remove_file(&archive);
    assert!(Command::new(env::var_os("AR").unwrap_or_else(|| "ar".into()))
        .arg("crs").arg(&archive).args(&objects).status().expect("ar required").success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=stephania_aac_bridge");
    println!("cargo:rustc-link-lib=avcodec");
    println!("cargo:rustc-link-lib=avutil");
}

fn build_x264() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("x264_bridge.o");
    let archive = out.join("libstephania_x264_bridge.a");
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
    println!("cargo:rustc-link-lib=static=stephania_x264_bridge");
    println!("cargo:rustc-link-lib=x264");
}
