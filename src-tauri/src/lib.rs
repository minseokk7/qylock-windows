use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::ErrorKind,
    mem::size_of,
    path::PathBuf,
    sync::{LazyLock, Mutex, OnceLock},
    thread,
    time::Duration,
};
use tauri::{
    menu::MenuBuilder, tray::TrayIconBuilder, window::Monitor, AppHandle, Emitter, Manager,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use windows::core::{factory, w};
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE,
    VK_F4, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, FindWindowExW, FindWindowW, IsWindowVisible, SetWindowsHookExW, ShowWindow,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, KBDLLHOOKSTRUCT_FLAGS, LLKHF_ALTDOWN, SW_HIDE,
    SW_SHOW, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

struct SafeHHook(HHOOK);
unsafe impl Send for SafeHHook {}
unsafe impl Sync for SafeHHook {}

#[derive(Default)]
struct LockState {
    hook: Option<SafeHHook>,
    aux_window_labels: Vec<String>,
    hidden_taskbar_handles: Vec<isize>,
    is_locked: bool,
    allow_exit: bool,
    last_auto_lock_input_tick: Option<u32>,
}

static LOCK_STATE: LazyLock<Mutex<LockState>> = LazyLock::new(|| Mutex::new(LockState::default()));
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static APP_SETTINGS: LazyLock<Mutex<AppSettings>> =
    LazyLock::new(|| Mutex::new(AppSettings::default()));

const MAIN_WINDOW_LABEL: &str = "main";
const SETTINGS_WINDOW_LABEL: &str = "settings";
const AUX_WINDOW_PREFIX: &str = "lock-screen-monitor-";
const LOCK_SHORTCUT_LABEL: &str = "Ctrl+Alt+L";
const TRAY_SETTINGS_ID: &str = "tray-open-settings";
const TRAY_LOCK_ID: &str = "tray-lock-now";
const TRAY_QUIT_ID: &str = "tray-quit";
const STARTUP_REGISTRY_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const STARTUP_VALUE_NAME: &str = "qylock";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AppSettings {
    auto_lock_timeout_seconds: u64,
    blackout_timeout_seconds: u64,
    launch_on_startup: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_lock_timeout_seconds: 0,
            blackout_timeout_seconds: 0,
            launch_on_startup: false,
        }
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kbd = *(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = VIRTUAL_KEY(kbd.vkCode as u16);
        let message = wparam.0 as u32;
        let is_key_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;

        let is_alt = (kbd.flags & LLKHF_ALTDOWN) != KBDLLHOOKSTRUCT_FLAGS(0);
        let is_ctrl = (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
        let is_shift = (GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
        let is_menu = (GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0;
        let is_q = vk == VIRTUAL_KEY(0x51);
        let is_l = vk == VIRTUAL_KEY(0x4C);

        let is_locked = {
            let state = LOCK_STATE.lock().unwrap();
            state.is_locked
        };

        if !is_locked && is_key_down && is_l && is_ctrl && (is_alt || is_menu) && !is_shift {
            if cfg!(debug_assertions) {
                eprintln!("keyboard hook detected Ctrl+Alt+L");
            }
            if let Some(app) = APP_HANDLE.get().cloned() {
                let app_handle = app.clone();
                if let Err(error) = app.run_on_main_thread(move || {
                    if let Err(error) = lock_screen_impl(&app_handle) {
                        eprintln!("failed to lock from keyboard hook: {error}");
                    }
                }) {
                    eprintln!("failed to schedule keyboard-hook lock on main thread: {error}");
                }
            }
            return LRESULT(1);
        }

        if cfg!(debug_assertions) && is_ctrl && is_alt && is_q {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        if is_locked
            && (vk == VK_LWIN
                || vk == VK_RWIN
                || (vk == VK_TAB && is_alt)
                || (vk == VK_F4 && is_alt)
                || (vk == VK_ESCAPE && (is_alt || is_ctrl)))
        {
            return LRESULT(1);
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

fn same_monitor(a: &Monitor, b: &Monitor) -> bool {
    a.position() == b.position() && a.size() == b.size()
}

fn aux_window_label(index: usize) -> String {
    format!("{AUX_WINDOW_PREFIX}{index}")
}

fn destroy_aux_windows(app: &AppHandle, labels: &[String]) {
    for label in labels {
        if let Some(window) = app.get_webview_window(label) {
            if let Err(error) = window.destroy() {
                eprintln!("failed to destroy window `{label}`: {error}");
            }
        }
    }
}

fn current_lock_window_labels() -> Vec<String> {
    let state = LOCK_STATE.lock().unwrap();
    let mut labels = Vec::with_capacity(1 + state.aux_window_labels.len());
    labels.push(MAIN_WINDOW_LABEL.to_string());
    labels.extend(state.aux_window_labels.iter().cloned());
    labels
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("failed to resolve app config directory: {error}"))?;
    fs::create_dir_all(&config_dir).map_err(|error| {
        format!(
            "failed to create config directory `{}`: {error}",
            config_dir.display()
        )
    })?;
    Ok(config_dir.join("settings.json"))
}

fn load_settings_from_disk(app: &AppHandle) -> Result<AppSettings, String> {
    let path = settings_path(app)?;
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).map_err(|error| {
            format!(
                "failed to parse settings file `{}`: {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(error) => Err(format!(
            "failed to read settings file `{}`: {error}",
            path.display()
        )),
    }
}

fn persist_settings_to_disk(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let contents = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("failed to serialize settings: {error}"))?;
    fs::write(&path, contents).map_err(|error| {
        format!(
            "failed to write settings file `{}`: {error}",
            path.display()
        )
    })
}

fn startup_command_value() -> Result<String, String> {
    let exe_path = env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    Ok(format!("\"{}\"", exe_path.display()))
}

fn sync_launch_on_startup(settings: &AppSettings) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = hkcu
        .create_subkey(STARTUP_REGISTRY_KEY)
        .map_err(|error| format!("failed to open startup registry key: {error}"))?;

    if settings.launch_on_startup {
        let command = startup_command_value()?;
        run_key
            .set_value(STARTUP_VALUE_NAME, &command)
            .map_err(|error| format!("failed to enable launch on startup: {error}"))?;
    } else if let Err(error) = run_key.delete_value(STARTUP_VALUE_NAME) {
        if error.kind() != ErrorKind::NotFound {
            return Err(format!("failed to disable launch on startup: {error}"));
        }
    }

    Ok(())
}

fn current_settings() -> AppSettings {
    APP_SETTINGS.lock().unwrap().clone()
}

fn current_system_idle_state() -> Result<(u64, u32), String> {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        ..Default::default()
    };

    let last_input_tick = unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return Err("failed to query last input info".to_string());
        }
        info.dwTime
    };
    let current_tick = unsafe { GetTickCount() };
    let idle_millis = current_tick.wrapping_sub(last_input_tick) as u64;

    Ok((idle_millis / 1000, last_input_tick))
}

fn start_auto_lock_watcher(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(2));

        let settings = current_settings();
        if settings.auto_lock_timeout_seconds == 0 {
            continue;
        }

        let (idle_seconds, last_input_tick) = match current_system_idle_state() {
            Ok(state) => state,
            Err(error) => {
                eprintln!("{error}");
                continue;
            }
        };

        let should_lock = {
            let mut state = LOCK_STATE.lock().unwrap();
            if state.is_locked || idle_seconds < settings.auto_lock_timeout_seconds {
                false
            } else if state.last_auto_lock_input_tick == Some(last_input_tick) {
                false
            } else {
                state.last_auto_lock_input_tick = Some(last_input_tick);
                true
            }
        };

        if should_lock {
            let app_handle = app.clone();
            if let Err(error) = app.run_on_main_thread(move || {
                if let Err(error) = lock_screen_impl(&app_handle) {
                    eprintln!("failed to auto-lock after idle timeout: {error}");
                }
            }) {
                eprintln!("failed to schedule idle auto-lock on main thread: {error}");
            }
        }
    });
}

fn open_settings_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(
        app,
        SETTINGS_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("qylock 설정")
    .inner_size(520.0, 860.0)
    .min_inner_size(500.0, 840.0)
    .decorations(false)
    .resizable(false)
    .center()
    .focused(true)
    .visible(true)
    .build()
    .map_err(|error| format!("failed to create settings window: {error}"))?;

    Ok(())
}

fn set_lock_windows_topmost(app: &AppHandle, topmost: bool, focus_main: bool) {
    for label in current_lock_window_labels() {
        if let Some(window) = app.get_webview_window(&label) {
            if let Err(error) = window.set_always_on_top(topmost) {
                eprintln!("failed to update always-on-top for `{label}`: {error}");
            }
        }
    }

    if focus_main {
        if let Some(main_window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
            if let Err(error) = main_window.set_focus() {
                eprintln!("failed to refocus main window: {error}");
            }
        }
    }
}

fn taskbar_window_handles() -> Vec<HWND> {
    let mut handles = Vec::new();

    unsafe {
        if let Ok(primary_taskbar) = FindWindowW(w!("Shell_TrayWnd"), None) {
            if !primary_taskbar.is_invalid() {
                handles.push(primary_taskbar);
            }
        }

        let mut previous = None;
        while let Ok(taskbar) = FindWindowExW(None, previous, w!("Shell_SecondaryTrayWnd"), None) {
            if taskbar.is_invalid() {
                break;
            }

            handles.push(taskbar);
            previous = Some(taskbar);
        }
    }

    handles
}

fn hide_taskbars() {
    let mut hidden_handles = Vec::new();

    for hwnd in taskbar_window_handles() {
        unsafe {
            if IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_HIDE);
                hidden_handles.push(hwnd.0 as isize);
            }
        }
    }

    let mut state = LOCK_STATE.lock().unwrap();
    state.hidden_taskbar_handles = hidden_handles;
}

fn restore_taskbars() {
    let stored_handles = {
        let mut state = LOCK_STATE.lock().unwrap();
        std::mem::take(&mut state.hidden_taskbar_handles)
    };
    let mut handles_to_restore: Vec<isize> = taskbar_window_handles()
        .into_iter()
        .map(|hwnd| hwnd.0 as isize)
        .collect();

    for hwnd in stored_handles {
        if !handles_to_restore.iter().any(|existing| *existing == hwnd) {
            handles_to_restore.push(hwnd);
        }
    }

    for hwnd in handles_to_restore {
        unsafe {
            let hwnd = HWND(hwnd as *mut core::ffi::c_void);
            if !hwnd.is_invalid() {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
    }
}

fn hwnd_for_window(window: &WebviewWindow) -> Result<HWND, String> {
    let handle = window
        .window_handle()
        .map_err(|error| format!("failed to read native window handle: {error}"))?;

    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => Err("unsupported native window handle for Windows Hello interop".into()),
    }
}

fn install_keyboard_hook() -> Result<(), String> {
    let mut state = LOCK_STATE.lock().unwrap();
    if state.hook.is_none() {
        let hook = unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0)
                .map_err(|error| format!("failed to install keyboard hook: {error}"))?
        };
        state.hook = Some(SafeHHook(hook));
        if cfg!(debug_assertions) {
            eprintln!("keyboard hook installed");
        }
    }
    Ok(())
}

fn apply_main_window_state(window: &WebviewWindow, monitor: &Monitor) -> Result<(), String> {
    window
        .set_fullscreen(false)
        .map_err(|error| format!("failed to disable fullscreen: {error}"))?;
    window
        .set_position(monitor.position().to_owned())
        .map_err(|error| format!("failed to position main window: {error}"))?;
    window
        .set_size(monitor.size().to_owned())
        .map_err(|error| format!("failed to resize main window: {error}"))?;
    window
        .set_decorations(false)
        .map_err(|error| format!("failed to remove window decorations: {error}"))?;
    window
        .set_resizable(false)
        .map_err(|error| format!("failed to lock window size: {error}"))?;
    window
        .set_always_on_top(true)
        .map_err(|error| format!("failed to pin main window on top: {error}"))?;
    window
        .set_skip_taskbar(true)
        .map_err(|error| format!("failed to hide main window from taskbar: {error}"))?;
    window
        .set_visible_on_all_workspaces(true)
        .map_err(|error| format!("failed to show main window on all workspaces: {error}"))?;
    window
        .set_content_protected(true)
        .map_err(|error| format!("failed to protect main window content: {error}"))?;
    window
        .set_shadow(false)
        .map_err(|error| format!("failed to disable main window shadow: {error}"))?;
    window
        .show()
        .map_err(|error| format!("failed to show main window: {error}"))?;
    window
        .set_focus()
        .map_err(|error| format!("failed to focus main window: {error}"))?;
    Ok(())
}

fn ensure_aux_window(app: &AppHandle, label: &str, monitor: &Monitor) -> Result<(), String> {
    let window = if let Some(existing) = app.get_webview_window(label) {
        existing
    } else {
        WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
            .title("Pixel Skyscrapers Lock Screen")
            .decorations(false)
            .resizable(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible_on_all_workspaces(true)
            .content_protected(true)
            .shadow(false)
            .focused(false)
            .focusable(false)
            .visible(false)
            .build()
            .map_err(|error| format!("failed to create auxiliary window `{label}`: {error}"))?
    };

    window
        .set_fullscreen(false)
        .map_err(|error| format!("failed to disable fullscreen for `{label}`: {error}"))?;
    window
        .set_position(monitor.position().to_owned())
        .map_err(|error| format!("failed to position `{label}`: {error}"))?;
    window
        .set_size(monitor.size().to_owned())
        .map_err(|error| format!("failed to resize `{label}`: {error}"))?;
    window
        .show()
        .map_err(|error| format!("failed to show `{label}`: {error}"))?;

    Ok(())
}

fn sync_aux_windows(app: &AppHandle, primary_monitor: &Monitor) -> Result<Vec<String>, String> {
    let monitors = app
        .available_monitors()
        .map_err(|error| format!("failed to enumerate monitors: {error}"))?;
    let previous_labels = {
        let state = LOCK_STATE.lock().unwrap();
        state.aux_window_labels.clone()
    };
    let mut desired_labels = Vec::new();

    for (index, monitor) in monitors
        .iter()
        .filter(|monitor| !same_monitor(monitor, primary_monitor))
        .enumerate()
    {
        let label = aux_window_label(index);
        ensure_aux_window(app, &label, monitor)?;
        desired_labels.push(label);
    }

    for stale_label in previous_labels {
        if !desired_labels.iter().any(|label| label == &stale_label) {
            if let Some(window) = app.get_webview_window(&stale_label) {
                if let Err(error) = window.destroy() {
                    eprintln!("failed to destroy stale window `{stale_label}`: {error}");
                }
            }
        }
    }

    Ok(desired_labels)
}

fn unlock_and_unhook(app: &AppHandle) {
    let aux_window_labels = {
        let mut state = LOCK_STATE.lock().unwrap();
        state.is_locked = false;
        std::mem::take(&mut state.aux_window_labels)
    };

    restore_taskbars();

    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        if let Err(error) = window.hide() {
            eprintln!("failed to hide main window: {error}");
        }
    }

    destroy_aux_windows(app, &aux_window_labels);
    let _ = app.emit("lock-state-changed", false);
}

fn quit_app(app: &AppHandle) {
    {
        let mut state = LOCK_STATE.lock().unwrap();
        state.allow_exit = true;
    }
    restore_taskbars();
    unlock_and_unhook(app);
    let hook = {
        let mut state = LOCK_STATE.lock().unwrap();
        state.hook.take()
    };
    if let Some(SafeHHook(hook)) = hook {
        unsafe {
            if let Err(error) = UnhookWindowsHookEx(hook) {
                eprintln!("failed to remove keyboard hook: {error}");
            }
        }
    }
    app.exit(0);
}

fn lock_screen_impl(app: &AppHandle) -> Result<(), String> {
    {
        let state = LOCK_STATE.lock().unwrap();
        if state.is_locked {
            drop(state);
            set_lock_windows_topmost(app, true, true);
            return Ok(());
        }
    }

    if let Some(settings_window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let _ = settings_window.hide();
    }

    let primary_monitor = app
        .primary_monitor()
        .map_err(|error| format!("failed to resolve primary monitor: {error}"))?
        .ok_or_else(|| "no primary monitor found".to_string())?;
    let aux_window_labels = sync_aux_windows(app, &primary_monitor)?;

    let main_window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| "main window not found".to_string())?;
    apply_main_window_state(&main_window, &primary_monitor)?;
    install_keyboard_hook()?;
    hide_taskbars();

    let mut state = LOCK_STATE.lock().unwrap();
    state.is_locked = true;
    state.aux_window_labels = aux_window_labels;
    drop(state);
    let _ = app.emit("lock-state-changed", true);

    Ok(())
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let lock_label = format!("지금 잠그기 ({LOCK_SHORTCUT_LABEL})");
    let tray_menu = MenuBuilder::new(app)
        .text(TRAY_SETTINGS_ID, "설정 열기")
        .separator()
        .text(TRAY_LOCK_ID, lock_label)
        .separator()
        .text(TRAY_QUIT_ID, "qylock 종료")
        .build()?;

    let mut tray_builder = TrayIconBuilder::with_id("main-tray")
        .menu(&tray_menu)
        .tooltip(format!("qylock-windows ({LOCK_SHORTCUT_LABEL})"))
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if event.id == TRAY_SETTINGS_ID {
                if let Err(error) = open_settings_window(app) {
                    eprintln!("failed to open settings window from tray: {error}");
                }
            } else if event.id == TRAY_LOCK_ID {
                if let Err(error) = lock_screen_impl(app) {
                    eprintln!("failed to lock from tray: {error}");
                }
            } else if event.id == TRAY_QUIT_ID {
                quit_app(app);
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray_builder = tray_builder.icon(icon);
    }

    let _ = tray_builder.build(app)?;
    Ok(())
}

#[tauri::command]
async fn verify_hello(app: AppHandle) -> Result<bool, String> {
    let availability = UserConsentVerifier::CheckAvailabilityAsync()
        .map_err(|error| format!("failed to check Windows Hello availability: {error}"))?
        .await
        .map_err(|error| format!("failed to await Windows Hello availability: {error}"))?;

    match availability {
        UserConsentVerifierAvailability::Available => {}
        UserConsentVerifierAvailability::DeviceNotPresent => {
            return Err("Windows Hello device is not present.".into())
        }
        UserConsentVerifierAvailability::NotConfiguredForUser => {
            return Err("Windows Hello is not configured for this user.".into())
        }
        UserConsentVerifierAvailability::DisabledByPolicy => {
            return Err("Windows Hello is disabled by policy.".into())
        }
        UserConsentVerifierAvailability::DeviceBusy => {
            return Err("Windows Hello device is busy. Please try again.".into())
        }
        _ => return Err("Windows Hello is unavailable.".into()),
    }

    set_lock_windows_topmost(&app, false, false);
    hide_taskbars();

    let operation = {
        let main_window = app
            .get_webview_window(MAIN_WINDOW_LABEL)
            .ok_or_else(|| "main window not found".to_string())?;
        let hwnd = hwnd_for_window(&main_window)?;
        let interop = factory::<UserConsentVerifier, IUserConsentVerifierInterop>()
            .map_err(|error| format!("failed to load Windows Hello interop factory: {error}"))?;

        unsafe {
            interop
                .RequestVerificationForWindowAsync::<windows_future::IAsyncOperation<UserConsentVerificationResult>>(
                    hwnd,
                    &"Unlock Pixel Skyscrapers".into(),
                )
        }
        .map_err(|error| format!("failed to open Windows Hello prompt: {error}"))?
    };
    let result = operation
        .await
        .map_err(|error| format!("failed to await Windows Hello prompt: {error}"));

    let result = match result {
        Ok(result) => result,
        Err(error) => {
            hide_taskbars();
            set_lock_windows_topmost(&app, true, true);
            return Err(error);
        }
    };

    match result {
        UserConsentVerificationResult::Verified => {
            unlock_and_unhook(&app);
            Ok(true)
        }
        UserConsentVerificationResult::Canceled
        | UserConsentVerificationResult::RetriesExhausted => {
            hide_taskbars();
            set_lock_windows_topmost(&app, true, true);
            Ok(false)
        }
        status => {
            hide_taskbars();
            set_lock_windows_topmost(&app, true, true);
            Err(format!(
                "Windows Hello verification failed with status: {:?}",
                status
            ))
        }
    }
}

#[tauri::command]
fn emergency_unlock(app: AppHandle) {
    unlock_and_unhook(&app);
}

#[tauri::command]
fn close_app(app: AppHandle) {
    quit_app(&app);
}

#[tauri::command]
fn lock_screen(app: AppHandle) -> Result<(), String> {
    lock_screen_impl(&app)
}

#[tauri::command]
fn get_settings() -> AppSettings {
    current_settings()
}

#[tauri::command]
fn get_lock_state() -> bool {
    LOCK_STATE.lock().unwrap().is_locked
}

#[tauri::command]
fn hide_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        window
            .hide()
            .map_err(|error| format!("failed to hide settings window: {error}"))?;
    }

    Ok(())
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: AppSettings) -> Result<(), String> {
    let previous_settings = current_settings();
    persist_settings_to_disk(&app, &settings)?;
    if let Err(error) = sync_launch_on_startup(&settings) {
        let _ = persist_settings_to_disk(&app, &previous_settings);
        return Err(error);
    }
    {
        let mut state = APP_SETTINGS.lock().unwrap();
        *state = settings.clone();
    }
    if previous_settings.auto_lock_timeout_seconds != settings.auto_lock_timeout_seconds {
        LOCK_STATE.lock().unwrap().last_auto_lock_input_tick = None;
    }
    app.emit("settings-updated", settings)
        .map_err(|error| format!("failed to emit settings update event: {error}"))?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = LOCK_STATE.lock().unwrap();
                if state.allow_exit {
                    return;
                }

                api.prevent_close();

                if window.label() == SETTINGS_WINDOW_LABEL {
                    let _ = window.hide();
                } else if window.label() == MAIN_WINDOW_LABEL && !state.is_locked {
                    let _ = window.hide();
                }
            }
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            verify_hello,
            lock_screen,
            emergency_unlock,
            close_app,
            get_lock_state,
            hide_settings_window,
            get_settings,
            save_settings
        ])
        .setup(|app| {
            let _ = APP_HANDLE.set(app.handle().clone());
            restore_taskbars();
            install_keyboard_hook()?;
            match load_settings_from_disk(app.handle()) {
                Ok(settings) => {
                    if let Err(error) = sync_launch_on_startup(&settings) {
                        eprintln!("{error}");
                    }
                    let mut state = APP_SETTINGS.lock().unwrap();
                    *state = settings;
                }
                Err(error) => {
                    eprintln!("{error}");
                }
            }
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                let _ = window.hide();
                let _ = window.set_skip_taskbar(true);
                let _ = window.set_content_protected(true);
                let _ = window.set_shadow(false);
            }
            start_auto_lock_watcher(app.handle().clone());
            build_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
