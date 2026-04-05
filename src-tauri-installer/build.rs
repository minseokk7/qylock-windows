use std::{fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../package.json");

    let package_json = fs::read_to_string("../package.json").expect("failed to read package.json");
    let package: serde_json::Value =
        serde_json::from_str(&package_json).expect("failed to parse package.json");
    let version = package["version"]
        .as_str()
        .expect("package.json version must be a string");

    let setup_path = PathBuf::from(format!(
        "../src-tauri/target/release/bundle/nsis/qylock-windows_{version}_x64-setup.exe"
    ));

    println!("cargo:rerun-if-changed={}", setup_path.display());
    println!("cargo:rustc-env=QYLOCK_TARGET_VERSION={version}");

    tauri_build::build()
}
