use std::{env, path::PathBuf, process::Command};

fn main() {
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
