use base64::Engine;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use semver::Version;
use std::{
    env, fs,
    io::ErrorKind,
    mem::size_of,
    path::PathBuf,
    process::Command,
    sync::{mpsc, Arc, Condvar, LazyLock, Mutex, OnceLock},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::MenuBuilder, tray::TrayIconBuilder, window::Monitor, AppHandle, Emitter, Manager,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use windows::{
    core::{factory, w},
    Foundation::TypedEventHandler,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE,
    VK_F4, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowExW, FindWindowW, GetMessageW, IsWindowVisible, PostQuitMessage, RegisterClassW,
    SendMessageTimeoutW, SetWindowsHookExW, ShowWindow, TranslateMessage, UnhookWindowsHookEx,
    UnregisterClassW, HHOOK, HWND_BROADCAST, HWND_MESSAGE,
    KBDLLHOOKSTRUCT, KBDLLHOOKSTRUCT_FLAGS, LLKHF_ALTDOWN, MSG, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, SC_MONITORPOWER, SMTO_ABORTIFHUNG, SW_HIDE, SW_SHOW, WH_KEYBOARD_LL,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_KEYDOWN, WM_POWERBROADCAST, WM_SYSCOMMAND,
    WM_SYSKEYDOWN, WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
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
    last_display_off_input_tick: Option<u32>,
}

static LOCK_STATE: LazyLock<Mutex<LockState>> = LazyLock::new(|| Mutex::new(LockState::default()));
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static APP_SETTINGS: LazyLock<Mutex<AppSettings>> =
    LazyLock::new(|| Mutex::new(AppSettings::default()));
static AUTO_LOCK_WATCHER_SIGNAL: LazyLock<(Mutex<u64>, Condvar)> =
    LazyLock::new(|| (Mutex::new(0), Condvar::new()));
static MEDIA_BRIDGE_SIGNAL: LazyLock<(Mutex<u64>, Condvar)> =
    LazyLock::new(|| (Mutex::new(0), Condvar::new()));
static MEDIA_BRIDGE_STATE: LazyLock<Mutex<MediaBridgeState>> =
    LazyLock::new(|| Mutex::new(MediaBridgeState::default()));

const MAIN_WINDOW_LABEL: &str = "main";
const SETTINGS_WINDOW_LABEL: &str = "settings";
const AUX_WINDOW_PREFIX: &str = "lock-screen-monitor-";
const LOCK_SHORTCUT_LABEL: &str = "Ctrl+Alt+L";
const DISPLAY_OFF_SHORTCUT_LABEL: &str = "Ctrl+Alt+O";
const GITHUB_RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/minseokk7/qylock-windows/releases/latest";
const GITHUB_RELEASES_PAGE_URL: &str = "https://github.com/minseokk7/qylock-windows/releases/latest";
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
    #[serde(default = "default_media_bridge_enabled")]
    media_bridge_enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct NowPlayingInfo {
    source_kind: String,
    title: String,
    artist: String,
    album: String,
    thumbnail: Option<String>,
    status: String,
    position_ms: u64,
    duration_ms: u64,
    updated_at: u64,
}

#[derive(Debug, Default)]
struct MediaBridgeState {
    now_playing: Option<NowPlayingInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCheckResult {
    current_version: String,
    latest_version: String,
    update_available: bool,
    release_url: String,
    download_url: Option<String>,
    published_at: Option<String>,
    release_name: Option<String>,
    summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubLatestRelease {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
    name: Option<String>,
    body: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionProbe {
    NotFound,
    Inactive,
    Playing(NowPlayingInfo),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
enum MediaControlAction {
    Previous,
    TogglePlayPause,
    Next,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_lock_timeout_seconds: 0,
            blackout_timeout_seconds: 0,
            launch_on_startup: false,
            media_bridge_enabled: default_media_bridge_enabled(),
        }
    }
}

fn default_media_bridge_enabled() -> bool {
    true
}

fn app_version_string() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn normalize_version_tag(raw: &str) -> String {
    raw.trim().trim_start_matches(['v', 'V']).to_string()
}

fn parse_release_version(raw: &str) -> Result<Version, String> {
    let normalized = normalize_version_tag(raw);
    Version::parse(&normalized).map_err(|error| format!("failed to parse version `{raw}`: {error}"))
}

fn summarize_release_notes(body: Option<&str>) -> Option<String> {
    let first_line = body?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;

    let mut summary = first_line.to_string();
    if summary.chars().count() > 120 {
        summary = summary.chars().take(117).collect::<String>();
        summary.push_str("...");
    }

    Some(summary)
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
        let is_left_windows_down = (GetAsyncKeyState(VK_LWIN.0 as i32) as u16 & 0x8000) != 0;
        let is_right_windows_down = (GetAsyncKeyState(VK_RWIN.0 as i32) as u16 & 0x8000) != 0;
        let is_windows_combo =
            vk == VK_LWIN || vk == VK_RWIN || is_left_windows_down || is_right_windows_down;
        let is_q = vk == VIRTUAL_KEY(0x51);
        let is_l = vk == VIRTUAL_KEY(0x4C);
        let is_o = vk == VIRTUAL_KEY(0x4F);

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

        if is_key_down && is_o && is_ctrl && (is_alt || is_menu) && !is_shift {
            if cfg!(debug_assertions) {
                eprintln!("keyboard hook detected {DISPLAY_OFF_SHORTCUT_LABEL}");
            }
            if let Some(app) = APP_HANDLE.get().cloned() {
                let app_handle = app.clone();
                if let Err(error) = app.run_on_main_thread(move || {
                    if !is_locked {
                        if let Err(error) = lock_screen_impl(&app_handle) {
                            eprintln!(
                                "failed to lock before display-off from keyboard hook: {error}"
                            );
                            return;
                        }
                    }

                    if !turn_off_display_impl() {
                        eprintln!("failed to turn off display from keyboard hook");
                    } else {
                        reset_display_off_baseline();
                    }
                }) {
                    eprintln!(
                        "failed to schedule keyboard-hook display-off on main thread: {error}"
                    );
                }
            }
            return LRESULT(1);
        }

        if cfg!(debug_assertions) && is_ctrl && is_alt && is_q {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        if is_locked
            && (is_windows_combo
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

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn timespan_to_ms(duration: i64) -> u64 {
    duration.max(0) as u64 / 10_000
}

fn same_now_playing(
    left: &Option<NowPlayingInfo>,
    right: &Option<NowPlayingInfo>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.source_kind == right.source_kind
                && left.title == right.title
                && left.artist == right.artist
                && left.album == right.album
                && left.thumbnail == right.thumbnail
                && left.status == right.status
                && left.position_ms == right.position_ms
                && left.duration_ms == right.duration_ms
        }
        _ => false,
    }
}

fn current_now_playing_snapshot() -> Option<NowPlayingInfo> {
    MEDIA_BRIDGE_STATE.lock().unwrap().now_playing.clone()
}

fn update_media_bridge_state(next_now_playing: Option<NowPlayingInfo>) -> bool {
    let mut state = MEDIA_BRIDGE_STATE.lock().unwrap();
    if same_now_playing(&state.now_playing, &next_now_playing) {
        return false;
    }

    state.now_playing = next_now_playing;
    true
}

fn emit_media_bridge_update(
    app: &AppHandle,
    next_now_playing: Option<NowPlayingInfo>,
) -> Result<(), String> {
    app.emit("media-now-playing-updated", next_now_playing)
        .map_err(|error| format!("failed to emit media bridge update: {error}"))
}

fn sync_media_bridge_state(
    app: &AppHandle,
    next_now_playing: Option<NowPlayingInfo>,
) -> Result<(), String> {
    if update_media_bridge_state(next_now_playing.clone()) {
        emit_media_bridge_update(app, next_now_playing)?;
    }

    Ok(())
}

fn playback_status_label(
    status: GlobalSystemMediaTransportControlsSessionPlaybackStatus,
) -> &'static str {
    match status {
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing => "playing",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => "paused",
        _ => "stopped",
    }
}

fn build_now_playing(
    source_kind: &str,
    title: String,
    artist: String,
    album: String,
    thumbnail: Option<String>,
    status: &str,
    position_ms: u64,
    duration_ms: u64,
) -> Option<NowPlayingInfo> {
    let title = title.trim().to_string();
    let artist = artist.trim().to_string();
    let album = album.trim().to_string();

    if title.is_empty() {
        return None;
    }

    Some(NowPlayingInfo {
        source_kind: source_kind.to_string(),
        title,
        artist,
        album,
        thumbnail,
        status: status.to_string(),
        position_ms,
        duration_ms,
        updated_at: current_unix_millis(),
    })
}

fn read_media_thumbnail_data_url(
    session: &GlobalSystemMediaTransportControlsSession,
) -> Result<Option<String>, String> {
    let media_properties = session
        .TryGetMediaPropertiesAsync()
        .map_err(|error| format!("failed to request media properties for thumbnail: {error}"))?
        .join()
        .map_err(|error| format!("failed to await media properties for thumbnail: {error}"))?;

    let thumbnail_ref = match media_properties.Thumbnail() {
        Ok(thumbnail_ref) => thumbnail_ref,
        Err(_) => return Ok(None),
    };
    let stream = match thumbnail_ref.OpenReadAsync() {
        Ok(operation) => operation
            .join()
            .map_err(|error| format!("failed to await media thumbnail stream: {error}"))?,
        Err(_) => return Ok(None),
    };

    let size = stream
        .Size()
        .map_err(|error| format!("failed to read media thumbnail size: {error}"))?
        as u32;
    if size == 0 {
        return Ok(None);
    }

    let reader = DataReader::CreateDataReader(&stream)
        .map_err(|error| format!("failed to create media thumbnail reader: {error}"))?;
    reader
        .LoadAsync(size)
        .map_err(|error| format!("failed to start media thumbnail read: {error}"))?
        .join()
        .map_err(|error| format!("failed to finish media thumbnail read: {error}"))?;

    let mut bytes = vec![0u8; size as usize];
    reader
        .ReadBytes(&mut bytes)
        .map_err(|error| format!("failed to read media thumbnail bytes: {error}"))?;

    Ok(Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

fn request_media_session_manager() -> Result<GlobalSystemMediaTransportControlsSessionManager, String> {
    GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .map_err(|error| format!("failed to request media session manager: {error}"))?
        .join()
        .map_err(|error| format!("failed to await media session manager: {error}"))
}

fn read_media_session_app_id(
    session: &GlobalSystemMediaTransportControlsSession,
) -> Result<String, String> {
    session
        .SourceAppUserModelId()
        .map_err(|error| format!("failed to read media session app id: {error}"))
        .map(|app_id| app_id.to_string())
}

fn is_tidal_media_session(session: &GlobalSystemMediaTransportControlsSession) -> Result<bool, String> {
    Ok(read_media_session_app_id(session)?
        .to_ascii_lowercase()
        .contains("tidal"))
}

fn read_tidal_session_now_playing(
    session: &GlobalSystemMediaTransportControlsSession,
) -> Result<Option<NowPlayingInfo>, String> {
    if !is_tidal_media_session(session)? {
        return Ok(None);
    }

    let playback_info = session
        .GetPlaybackInfo()
        .map_err(|error| format!("failed to read playback info: {error}"))?;
    let playback_status = playback_info
        .PlaybackStatus()
        .map_err(|error| format!("failed to read playback status: {error}"))?;
    match playback_status {
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing
        | GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => {}
        _ => return Ok(None),
    }

    let media_properties = session
        .TryGetMediaPropertiesAsync()
        .map_err(|error| format!("failed to request media properties: {error}"))?
        .join()
        .map_err(|error| format!("failed to await media properties: {error}"))?;
    let timeline = session
        .GetTimelineProperties()
        .map_err(|error| format!("failed to read timeline properties: {error}"))?;

    let title = media_properties
        .Title()
        .map_err(|error| format!("failed to read media title: {error}"))?
        .to_string();
    let artist = media_properties
        .Artist()
        .map_err(|error| format!("failed to read media artist: {error}"))?
        .to_string();
    let album = media_properties
        .AlbumTitle()
        .map_err(|error| format!("failed to read media album: {error}"))?
        .to_string();
    let thumbnail = read_media_thumbnail_data_url(session)?;

    Ok(build_now_playing(
        "tidal-session",
        title,
        artist,
        album,
        thumbnail,
        playback_status_label(playback_status),
        timespan_to_ms(
            timeline
                .Position()
                .map_err(|error| format!("failed to read current media position: {error}"))?
                .Duration,
        ),
        timespan_to_ms(
            timeline
                .EndTime()
                .map_err(|error| format!("failed to read media duration: {error}"))?
                .Duration,
        ),
    ))
}

fn collect_tidal_session_now_playing_from_manager(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
) -> Result<SessionProbe, String> {
    let sessions = manager
        .GetSessions()
        .map_err(|error| format!("failed to enumerate media sessions: {error}"))?;
    let session_count = sessions
        .Size()
        .map_err(|error| format!("failed to read media session count: {error}"))?;
    let mut saw_tidal_session = false;

    for index in 0..session_count {
        let session = sessions
            .GetAt(index)
            .map_err(|error| format!("failed to read media session at index {index}: {error}"))?;
        if !is_tidal_media_session(&session)? {
            continue;
        }

        saw_tidal_session = true;
        if let Some(now_playing) = read_tidal_session_now_playing(&session)? {
            return Ok(SessionProbe::Playing(now_playing));
        }
    }

    if saw_tidal_session {
        Ok(SessionProbe::Inactive)
    } else {
        Ok(SessionProbe::NotFound)
    }
}

fn find_tidal_media_session() -> Result<Option<GlobalSystemMediaTransportControlsSession>, String> {
    let manager = request_media_session_manager()?;
    let sessions = manager
        .GetSessions()
        .map_err(|error| format!("failed to enumerate media sessions: {error}"))?;
    let session_count = sessions
        .Size()
        .map_err(|error| format!("failed to read media session count: {error}"))?;

    for index in 0..session_count {
        let session = sessions
            .GetAt(index)
            .map_err(|error| format!("failed to read media session at index {index}: {error}"))?;
        if is_tidal_media_session(&session)? {
            return Ok(Some(session));
        }
    }

    Ok(None)
}

fn current_settings() -> AppSettings {
    APP_SETTINGS.lock().unwrap().clone()
}

fn reset_auto_lock_baseline() {
    LOCK_STATE.lock().unwrap().last_auto_lock_input_tick = current_system_idle_state()
        .ok()
        .map(|(_, last_input_tick)| last_input_tick);
}

fn reset_display_off_baseline() {
    LOCK_STATE.lock().unwrap().last_display_off_input_tick = current_system_idle_state()
        .ok()
        .map(|(_, last_input_tick)| last_input_tick);
}

fn notify_auto_lock_watcher() {
    let (signal_mutex, signal) = &*AUTO_LOCK_WATCHER_SIGNAL;
    let mut generation = signal_mutex.lock().unwrap();
    *generation = generation.wrapping_add(1);
    signal.notify_all();
}

fn wait_for_auto_lock_signal(timeout: Duration) {
    let (signal_mutex, signal) = &*AUTO_LOCK_WATCHER_SIGNAL;
    let generation = signal_mutex.lock().unwrap();
    let _ = signal.wait_timeout(generation, timeout).unwrap();
}

fn notify_media_bridge() {
    let (signal_mutex, signal) = &*MEDIA_BRIDGE_SIGNAL;
    let mut generation = signal_mutex.lock().unwrap();
    *generation = generation.wrapping_add(1);
    signal.notify_all();
}

fn wait_for_media_bridge_signal(timeout: Duration) {
    let (signal_mutex, signal) = &*MEDIA_BRIDGE_SIGNAL;
    let generation = signal_mutex.lock().unwrap();
    let _ = signal.wait_timeout(generation, timeout).unwrap();
}

struct MediaSessionSubscription {
    session: GlobalSystemMediaTransportControlsSession,
    playback_info_token: i64,
    media_properties_token: i64,
}

impl Drop for MediaSessionSubscription {
    fn drop(&mut self) {
        let _ = self
            .session
            .RemovePlaybackInfoChanged(self.playback_info_token);
        let _ = self
            .session
            .RemoveMediaPropertiesChanged(self.media_properties_token);
    }
}

struct MediaManagerSubscription {
    manager: GlobalSystemMediaTransportControlsSessionManager,
    current_session_changed_token: i64,
    sessions_changed_token: i64,
}

impl Drop for MediaManagerSubscription {
    fn drop(&mut self) {
        let _ = self
            .manager
            .RemoveCurrentSessionChanged(self.current_session_changed_token);
        let _ = self.manager.RemoveSessionsChanged(self.sessions_changed_token);
    }
}

fn subscribe_to_media_session(
    session: &GlobalSystemMediaTransportControlsSession,
) -> Result<MediaSessionSubscription, String> {
    let playback_info_token = session
        .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            notify_media_bridge();
            Ok(())
        }))
        .map_err(|error| format!("failed to register playback info handler: {error}"))?;
    let media_properties_token = session
        .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
            notify_media_bridge();
            Ok(())
        }))
        .map_err(|error| format!("failed to register media properties handler: {error}"))?;

    Ok(MediaSessionSubscription {
        session: session.clone(),
        playback_info_token,
        media_properties_token,
    })
}

fn refresh_media_session_subscriptions(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
    session_subscriptions: &Arc<Mutex<Vec<MediaSessionSubscription>>>,
) -> Result<(), String> {
    let sessions = manager
        .GetSessions()
        .map_err(|error| format!("failed to enumerate media sessions for subscriptions: {error}"))?;
    let session_count = sessions
        .Size()
        .map_err(|error| format!("failed to read media session subscription count: {error}"))?;
    let mut next_subscriptions = Vec::new();

    for index in 0..session_count {
        let session = sessions.GetAt(index).map_err(|error| {
            format!("failed to read media session for subscriptions at index {index}: {error}")
        })?;
        if !is_tidal_media_session(&session)? {
            continue;
        }

        next_subscriptions.push(subscribe_to_media_session(&session)?);
    }

    *session_subscriptions.lock().unwrap() = next_subscriptions;
    Ok(())
}

fn subscribe_to_media_manager(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
) -> Result<MediaManagerSubscription, String> {
    let current_session_changed_token = manager
        .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            notify_media_bridge();
            Ok(())
        }))
        .map_err(|error| format!("failed to register current session handler: {error}"))?;
    let sessions_changed_token = manager
        .SessionsChanged(&TypedEventHandler::new(move |_, _| {
            notify_media_bridge();
            Ok(())
        }))
        .map_err(|error| format!("failed to register sessions changed handler: {error}"))?;

    Ok(MediaManagerSubscription {
        manager: manager.clone(),
        current_session_changed_token,
        sessions_changed_token,
    })
}

fn refresh_media_bridge_snapshot(
    app: &AppHandle,
    manager: &GlobalSystemMediaTransportControlsSessionManager,
) -> Result<(), String> {
    if !current_settings().media_bridge_enabled {
        return sync_media_bridge_state(app, None);
    }

    let next_now_playing = match collect_tidal_session_now_playing_from_manager(manager)? {
        SessionProbe::Playing(now_playing) => Some(now_playing),
        SessionProbe::Inactive | SessionProbe::NotFound => None,
    };

    sync_media_bridge_state(app, next_now_playing)
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

fn restore_lock_window_after_system_resume(reason: &str) {
    let is_locked = LOCK_STATE.lock().unwrap().is_locked;
    if !is_locked {
        return;
    }

    if let Some(app) = APP_HANDLE.get().cloned() {
        let app_handle = app.clone();
        if let Err(error) = app.run_on_main_thread(move || {
            set_lock_windows_topmost(&app_handle, true, true);
        }) {
            eprintln!("failed to restore lock window after {reason}: {error}");
        }
    }
}

fn handle_system_resume() {
    reset_auto_lock_baseline();
    notify_auto_lock_watcher();
    notify_media_bridge();
    restore_lock_window_after_system_resume("power resume");
}

fn handle_session_transition(event_code: u32) {
    match event_code {
        WTS_SESSION_LOCK => {
            reset_auto_lock_baseline();
            notify_auto_lock_watcher();
        }
        WTS_SESSION_UNLOCK => {
            reset_auto_lock_baseline();
            notify_auto_lock_watcher();
            restore_lock_window_after_system_resume("session unlock");
        }
        _ => {}
    }
}

unsafe extern "system" fn system_event_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_POWERBROADCAST => {
            let event_code = wparam.0 as u32;
            if event_code == PBT_APMRESUMEAUTOMATIC || event_code == PBT_APMRESUMESUSPEND {
                handle_system_resume();
            }
            LRESULT(1)
        }
        WM_WTSSESSION_CHANGE => {
            handle_session_transition(wparam.0 as u32);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn start_system_event_watcher() {
    thread::spawn(|| unsafe {
        let class_name = w!("QylockSystemEvents");
        let instance = match GetModuleHandleW(None) {
            Ok(module) => HINSTANCE(module.0),
            Err(error) => {
                eprintln!("failed to get module handle for system event watcher: {error}");
                return;
            }
        };

        let window_class = WNDCLASSW {
            lpfnWndProc: Some(system_event_window_proc),
            hInstance: instance,
            lpszClassName: class_name,
            ..Default::default()
        };

        if RegisterClassW(&window_class) == 0 {
            eprintln!("failed to register system event watcher window class");
            return;
        }

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("qylock-system-events"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                eprintln!("failed to create system event watcher window: {error}");
                let _ = UnregisterClassW(class_name, Some(instance));
                return;
            }
        };

        if let Err(error) = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) {
            eprintln!("failed to register session notifications: {error}");
        }

        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == -1 {
                eprintln!("system event watcher message loop failed");
                break;
            }
            if result == 0 {
                break;
            }
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);
        }

        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(class_name, Some(instance));
    });
}

fn start_auto_lock_watcher(app: AppHandle) {
    thread::spawn(move || loop {
        let settings = current_settings();
        let is_locked = LOCK_STATE.lock().unwrap().is_locked;

        if settings.auto_lock_timeout_seconds == 0 || is_locked {
            wait_for_auto_lock_signal(Duration::from_secs(5));
            continue;
        }

        let (idle_seconds, last_input_tick) = match current_system_idle_state() {
            Ok(state) => state,
            Err(error) => {
                eprintln!("{error}");
                wait_for_auto_lock_signal(Duration::from_secs(2));
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
            wait_for_auto_lock_signal(Duration::from_millis(250));
            continue;
        }

        let remaining_seconds = settings.auto_lock_timeout_seconds.saturating_sub(idle_seconds);
        wait_for_auto_lock_signal(Duration::from_secs(remaining_seconds.clamp(1, 2)));
    });
}

fn start_display_off_watcher(app: AppHandle) {
    thread::spawn(move || loop {
        let settings = current_settings();
        let is_locked = LOCK_STATE.lock().unwrap().is_locked;

        if settings.blackout_timeout_seconds == 0 || !is_locked {
            wait_for_auto_lock_signal(Duration::from_secs(2));
            continue;
        }

        let (idle_seconds, last_input_tick) = match current_system_idle_state() {
            Ok(state) => state,
            Err(error) => {
                eprintln!("{error}");
                wait_for_auto_lock_signal(Duration::from_secs(2));
                continue;
            }
        };

        let should_turn_off_display = {
            let mut state = LOCK_STATE.lock().unwrap();
            if !state.is_locked || idle_seconds < settings.blackout_timeout_seconds {
                false
            } else if state.last_display_off_input_tick == Some(last_input_tick) {
                false
            } else {
                state.last_display_off_input_tick = Some(last_input_tick);
                true
            }
        };

        if should_turn_off_display {
            if let Err(error) = app.run_on_main_thread(move || {
                if !turn_off_display_impl() {
                    eprintln!("failed to turn off display after lock timeout");
                }
            }) {
                eprintln!("failed to schedule display-off timeout on main thread: {error}");
            }
            wait_for_auto_lock_signal(Duration::from_millis(250));
            continue;
        }

        let remaining_seconds = settings.blackout_timeout_seconds.saturating_sub(idle_seconds);
        wait_for_auto_lock_signal(Duration::from_secs(remaining_seconds.clamp(1, 2)));
    });
}

fn start_media_bridge(app: AppHandle) {
    thread::spawn(move || loop {
        let manager = match request_media_session_manager() {
            Ok(manager) => manager,
            Err(error) => {
                eprintln!("failed to start media bridge manager: {error}");
                wait_for_media_bridge_signal(Duration::from_secs(15));
                continue;
            }
        };
        let session_subscriptions = Arc::new(Mutex::new(Vec::new()));

        if let Err(error) = refresh_media_session_subscriptions(&manager, &session_subscriptions) {
            eprintln!("failed to subscribe to TIDAL media sessions: {error}");
            wait_for_media_bridge_signal(Duration::from_secs(15));
            continue;
        }

        let _manager_subscription = match subscribe_to_media_manager(&manager) {
            Ok(subscription) => subscription,
            Err(error) => {
                eprintln!("failed to subscribe to media manager events: {error}");
                wait_for_media_bridge_signal(Duration::from_secs(15));
                continue;
            }
        };

        if let Err(error) = refresh_media_bridge_snapshot(&app, &manager) {
            eprintln!("failed to initialize media bridge state: {error}");
        }

        loop {
            wait_for_media_bridge_signal(Duration::from_secs(60));

            if let Err(error) = refresh_media_session_subscriptions(&manager, &session_subscriptions)
            {
                eprintln!("failed to refresh media session subscriptions: {error}");
                break;
            }

            if let Err(error) = refresh_media_bridge_snapshot(&app, &manager) {
                eprintln!("failed to refresh media bridge state: {error}");
                break;
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
    .inner_size(760.0, 650.0)
    .min_inner_size(720.0, 650.0)
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
            .title("qylock 잠금 화면")
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
        state.last_auto_lock_input_tick = current_system_idle_state()
            .ok()
            .map(|(_, last_input_tick)| last_input_tick);
        state.last_display_off_input_tick = None;
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
    notify_auto_lock_watcher();
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
    reset_display_off_baseline();
    let _ = app.emit("lock-state-changed", true);
    notify_auto_lock_watcher();

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
        .tooltip(format!("qylock ({LOCK_SHORTCUT_LABEL})"))
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
                    &"qylock 잠금 해제".into(),
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
fn get_now_playing_snapshot() -> Option<NowPlayingInfo> {
    current_now_playing_snapshot()
}

#[tauri::command]
fn control_now_playing(action: MediaControlAction) -> Result<bool, String> {
    let Some(session) = find_tidal_media_session()? else {
        return Err("TIDAL media session not found".to_string());
    };

    let result = match action {
        MediaControlAction::Previous => session
            .TrySkipPreviousAsync()
            .map_err(|error| format!("failed to request previous track: {error}"))?
            .join()
            .map_err(|error| format!("failed to await previous track command: {error}"))?,
        MediaControlAction::TogglePlayPause => session
            .TryTogglePlayPauseAsync()
            .map_err(|error| format!("failed to request play/pause toggle: {error}"))?
            .join()
            .map_err(|error| format!("failed to await play/pause toggle: {error}"))?,
        MediaControlAction::Next => session
            .TrySkipNextAsync()
            .map_err(|error| format!("failed to request next track: {error}"))?
            .join()
            .map_err(|error| format!("failed to await next track command: {error}"))?,
    };

    Ok(result)
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

fn send_monitor_power_signal(target: HWND) -> bool {
    unsafe {
        SendMessageTimeoutW(
            target,
            WM_SYSCOMMAND,
            WPARAM(SC_MONITORPOWER as usize),
            LPARAM(2),
            SMTO_ABORTIFHUNG,
            300,
            None,
        )
        .0 != 0
    }
}

fn turn_off_display_impl() -> bool {
    let mut signaled = false;

    unsafe {
        if let Ok(progman) = FindWindowW(w!("Progman"), None) {
            if !progman.is_invalid() {
                signaled |= send_monitor_power_signal(progman);
            }
        }

        if let Ok(taskbar) = FindWindowW(w!("Shell_TrayWnd"), None) {
            if !taskbar.is_invalid() {
                signaled |= send_monitor_power_signal(taskbar);
            }
        }
    }

    signaled |= send_monitor_power_signal(HWND_BROADCAST);

    signaled
}

#[tauri::command]
fn turn_off_display(app: AppHandle) -> Result<bool, String> {
    let (result_tx, result_rx) = mpsc::channel();

    app.run_on_main_thread(move || {
        let result = turn_off_display_impl();
        if result {
            reset_display_off_baseline();
        }
        let _ = result_tx.send(result);
    })
    .map_err(|error| format!("failed to schedule display-off on main thread: {error}"))?;

    result_rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| format!("failed to receive display-off result: {error}"))
}

#[tauri::command]
fn get_app_version() -> String {
    app_version_string()
}

#[tauri::command]
async fn check_for_updates() -> Result<UpdateCheckResult, String> {
    let current_version_raw = app_version_string();
    let current_version = parse_release_version(&current_version_raw)?;

    let client = reqwest::Client::builder()
        .build()
        .map_err(|error| format!("failed to create update client: {error}"))?;

    let response = client
        .get(GITHUB_RELEASES_LATEST_URL)
        .header(
            reqwest::header::USER_AGENT,
            format!("qylock-windows/{current_version_raw}"),
        )
        .header(
            reqwest::header::ACCEPT,
            "application/vnd.github+json",
        )
        .send()
        .await
        .map_err(|error| format!("failed to check latest release: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("update check failed with status {status}"));
    }

    let latest_release = response
        .json::<GitHubLatestRelease>()
        .await
        .map_err(|error| format!("failed to parse latest release response: {error}"))?;

    let latest_version_raw = normalize_version_tag(&latest_release.tag_name);
    let latest_version = parse_release_version(&latest_release.tag_name)?;

    let download_url = latest_release.assets.iter()
        .find(|asset| asset.name == "qylock-installer.exe")
        .map(|asset| asset.browser_download_url.clone());

    Ok(UpdateCheckResult {
        current_version: current_version_raw,
        latest_version: latest_version_raw,
        update_available: latest_version > current_version,
        release_url: if latest_release.html_url.is_empty() {
            GITHUB_RELEASES_PAGE_URL.to_string()
        } else {
            latest_release.html_url
        },
        download_url,
        published_at: latest_release.published_at,
        release_name: latest_release.name,
        summary: summarize_release_notes(latest_release.body.as_deref()),
    })
}

#[tauri::command]
async fn download_and_install_update(app: AppHandle, url: String) -> Result<(), String> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "qylock-windows-updater")
        .send()
        .await
        .map_err(|error| format!("failed to start update download: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "failed to download update: server returned {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("failed to read update download bytes: {error}"))?;

    let temp_dir = env::temp_dir().join("qylock-updates");
    fs::create_dir_all(&temp_dir)
        .map_err(|error| format!("failed to create update temp directory: {error}"))?;

    let installer_path = temp_dir.join("qylock-installer-latest.exe");
    fs::write(&installer_path, bytes)
        .map_err(|error| format!("failed to save update installer: {error}"))?;

    // Launch the installer with --silent and exit the app
    Command::new(&installer_path)
        .arg("--silent")
        .spawn()
        .map_err(|error| format!("failed to launch update installer: {error}"))?;

    quit_app(&app);
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
        LOCK_STATE.lock().unwrap().last_auto_lock_input_tick = current_system_idle_state()
            .ok()
            .map(|(_, last_input_tick)| last_input_tick);
    }
    notify_auto_lock_watcher();
    notify_media_bridge();
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
            get_now_playing_snapshot,
            control_now_playing,
            hide_settings_window,
            turn_off_display,
            get_app_version,
            check_for_updates,
            download_and_install_update,
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
            start_system_event_watcher();
            start_auto_lock_watcher(app.handle().clone());
            start_display_off_watcher(app.handle().clone());
            start_media_bridge(app.handle().clone());
            build_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}


