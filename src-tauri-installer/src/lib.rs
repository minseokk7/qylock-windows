use serde::Serialize;
use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TARGET_VERSION: &str = env!("QYLOCK_TARGET_VERSION");
const EMBEDDED_SETUP_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src-tauri/target/release/bundle/nsis/qylock-windows_",
    env!("QYLOCK_TARGET_VERSION"),
    "_x64-setup.exe"
));

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallerMetadata {
    app_version: String,
    installer_version: String,
    bundled_setup_name: String,
}

fn bundled_setup_name() -> String {
    format!("qylock-windows_{TARGET_VERSION}_x64-setup.exe")
}

fn write_embedded_setup() -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("failed to compute timestamp: {error}"))?
        .as_secs();

    let temp_dir = env::temp_dir().join("qylock-installer");
    fs::create_dir_all(&temp_dir)
        .map_err(|error| format!("failed to create installer temp directory: {error}"))?;

    let setup_path = temp_dir.join(format!("qylock-{TARGET_VERSION}-{stamp}.exe"));
    fs::write(&setup_path, EMBEDDED_SETUP_BYTES)
        .map_err(|error| format!("failed to write embedded setup executable: {error}"))?;

    Ok(setup_path)
}

fn run_embedded_install() -> Result<(), String> {
    let setup_path = write_embedded_setup()?;
    let status = Command::new(&setup_path)
        .arg("/S")
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("failed to launch bundled setup executable: {error}"))?;

    let _ = fs::remove_file(&setup_path);

    if !status.success() {
        return Err(format!(
            "setup process exited with status {}",
            status.code().unwrap_or_default()
        ));
    }

    Ok(())
}

#[tauri::command]
fn get_installer_metadata() -> InstallerMetadata {
    InstallerMetadata {
        app_version: TARGET_VERSION.to_string(),
        installer_version: env!("CARGO_PKG_VERSION").to_string(),
        bundled_setup_name: bundled_setup_name(),
    }
}

#[tauri::command]
async fn install_qylock() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(run_embedded_install)
        .await
        .map_err(|error| format!("failed to wait for install task: {error}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_installer_metadata,
            install_qylock
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_shadow(false);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running installer application");
}
