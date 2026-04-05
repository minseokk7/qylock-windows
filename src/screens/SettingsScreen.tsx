import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { type MouseEvent as ReactMouseEvent, useEffect, useState } from "react";
import SettingsDropdown from "../components/SettingsDropdown";
import { type AppSettings, defaultSettings } from "../app-settings";

type ToastState = {
  message: string;
  tone: "success" | "error";
};

type UpdateCheckResult = {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  releaseUrl: string;
  downloadUrl: string | null;
  publishedAt: string | null;
  releaseName: string | null;
  summary: string | null;
};

function formatPublishedDate(value: string | null) {
  if (!value) {
    return null;
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat("ko-KR", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(date);
}

function SettingsScreen() {
  const currentWindow = getCurrentWindow();
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [isSaving, setIsSaving] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [isShortcutHelpOpen, setIsShortcutHelpOpen] = useState(false);
  const [currentVersion, setCurrentVersion] = useState("");
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [isDownloadingUpdate, setIsDownloadingUpdate] = useState(false);
  const [isUpdateModalOpen, setIsUpdateModalOpen] = useState(false);

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | null = null;

    void invoke<AppSettings>("get_settings").then((nextSettings) => {
      if (active) {
        setSettings(nextSettings);
      }
    });

    void invoke<string>("get_app_version").then((version) => {
      if (active) {
        setCurrentVersion(version);
      }
    });

    void listen<AppSettings>("settings-updated", (event) => {
      if (active) {
        setSettings(event.payload);
      }
    }).then((dispose) => {
      unlisten = dispose;
    });

    return () => {
      active = false;
      if (unlisten) {
        void unlisten();
      }
    };
  }, []);

  useEffect(() => {
    if (!toast) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setToast(null);
    }, 2200);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [toast]);

  const handleSave = async () => {
    const normalizedSettings: AppSettings = {
      autoLockTimeoutSeconds: Math.max(
        0,
        Math.min(3600, Math.floor(settings.autoLockTimeoutSeconds || 0)),
      ),
      blackoutTimeoutSeconds: Math.max(
        0,
        Math.min(3600, Math.floor(settings.blackoutTimeoutSeconds || 0)),
      ),
      launchOnStartup: Boolean(settings.launchOnStartup),
      mediaBridgeEnabled: Boolean(settings.mediaBridgeEnabled),
    };

    setIsSaving(true);
    setToast(null);

    try {
      await invoke("save_settings", { settings: normalizedSettings });
      setSettings(normalizedSettings);
      setToast({
        message: "\uC124\uC815\uC774 \uC800\uC7A5\uB418\uC5C8\uC2B5\uB2C8\uB2E4.",
        tone: "success",
      });
    } catch (error) {
      console.error(error);
      setToast({
        message: "\uC124\uC815\uC744 \uC800\uC7A5\uD558\uC9C0 \uBABB\uD588\uC2B5\uB2C8\uB2E4.",
        tone: "error",
      });
    } finally {
      setIsSaving(false);
    }
  };

  const handleHide = async () => {
    setToast(null);
    setIsShortcutHelpOpen(false);

    try {
      await invoke("hide_settings_window");
    } catch (error) {
      console.error(error);
      setToast({
        message: "\uC124\uC815 \uCC3D\uC744 \uC228\uAE30\uC9C0 \uBABB\uD588\uC2B5\uB2C8\uB2E4.",
        tone: "error",
      });
    }
  };

  const handleLockNow = async () => {
    await invoke("lock_screen");
    await currentWindow.hide();
  };

  const handleCheckForUpdates = async () => {
    setIsCheckingUpdate(true);
    setToast(null);

    try {
      console.log("Checking for updates...");
      const result = await invoke<UpdateCheckResult>("check_for_updates");
      console.log("Update check result:", result);
      
      setUpdateResult(result);
      setCurrentVersion(result.currentVersion);

      if (result.updateAvailable) {
        console.log("Opening update modal...");
        setIsUpdateModalOpen(true);
      } else {
        console.log("No update available.");
        setToast({
          message: "이미 최신 버전을 사용 중입니다.",
          tone: "success",
        });
      }
    } catch (error) {
      console.error("Failed to check for updates:", error);
      setToast({
        message: "업데이트를 확인하지 못했습니다.",
        tone: "error",
      });
    } finally {
      setIsCheckingUpdate(false);
    }
  };

  const handleOpenReleasePage = async () => {
    const releaseUrl =
      updateResult?.releaseUrl ?? "https://github.com/minseokk7/qylock-windows/releases/latest";

    try {
      await openUrl(releaseUrl);
    } catch (error) {
      console.error(error);
      setToast({
        message: "다운로드 페이지를 열지 못했습니다.",
        tone: "error",
      });
    }
  };

  const handleDownloadAndInstall = async () => {
    if (!updateResult?.downloadUrl) {
      return;
    }

    setIsDownloadingUpdate(true);
    setToast(null);

    try {
      await invoke("download_and_install_update", { url: updateResult.downloadUrl });
      // 앱이 성공적으로 종료될 것이므로 상태 리셋은 필요 없음
    } catch (error) {
      console.error(error);
      setToast({
        message: "업데이트 다운로드에 실패했습니다.",
        tone: "error",
      });
      setIsDownloadingUpdate(false);
    }
  };

  const handleDragMouseDown = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      return;
    }

    event.preventDefault();
    void currentWindow.startDragging().catch((error) => {
      console.error("Failed to start dragging settings window:", error);
    });
  };

  return (
    <div className="settings-shell">
      <div className="settings-panel">
        {toast ? (
          <div className={`settings-toast is-${toast.tone}`} role="status" aria-live="polite">
            {toast.message}
          </div>
        ) : null}

        <div className="settings-header">
          <div className="settings-window-bar">
            <div
              className="settings-window-drag-area"
              data-tauri-drag-region
              onMouseDown={handleDragMouseDown}
              aria-hidden="true"
              title={"\uCC3D \uC774\uB3D9"}
            />

            <div className="settings-window-actions">
              <button
                type="button"
                className={`settings-header-button${isShortcutHelpOpen ? " is-active" : ""}`}
                aria-label={"\uB2E8\uCD95\uD0A4 \uBCF4\uAE30"}
                aria-expanded={isShortcutHelpOpen}
                onClick={() => setIsShortcutHelpOpen((current) => !current)}
              >
                ?
              </button>
              <button
                type="button"
                className="settings-header-button"
                aria-label={"\uCD5C\uC18C\uD654"}
                onClick={() => void currentWindow.minimize()}
              >
                -
              </button>
              <button
                type="button"
                className="settings-header-button is-close"
                aria-label={"\uB2EB\uAE30"}
                onClick={() => void handleHide()}
              >
                x
              </button>

              {isShortcutHelpOpen ? (
                <div className="settings-shortcut-popover" role="dialog" aria-label={"단축키"}>
                  <p className="settings-shortcut-title">{"\uB2E8\uCD95\uD0A4"}</p>
                  <div className="settings-shortcut-row">
                    <span>{"\uC7A0\uAE08"}</span>
                    <strong>Ctrl+Alt+L</strong>
                  </div>
                  <div className="settings-shortcut-row">
                    <span>{"\uC7A0\uAE08 + \uB514\uC2A4\uD50C\uB808\uC774 \uB044\uAE30"}</span>
                    <strong>Ctrl+Alt+O</strong>
                  </div>
                </div>
              ) : null}
            </div>
          </div>

          <div
            className="settings-header-copy"
            data-tauri-drag-region
            onMouseDown={handleDragMouseDown}
          >
            <p className="settings-kicker">qylock</p>
            <h1>{"\uC124\uC815"}</h1>
            <p className="settings-subtitle">
              {
                "\uC2DC\uC2A4\uD15C \uD2B8\uB808\uC774\uC5D0\uC11C \uCC3D\uC744 \uC5F4\uC5B4 \uC7A0\uAE08 \uB3D9\uC791\uC744 \uC870\uC815\uD560 \uC218 \uC788\uC2B5\uB2C8\uB2E4."
              }
            </p>
          </div>
        </div>

        <div className="settings-grid">
          <div className="settings-card settings-card-update">
            <div className="settings-update-header">
              <div>
                <span className="settings-label">{"업데이트"}</span>
                <p className="settings-help">
                  {currentVersion
                    ? `현재 버전: ${currentVersion}`
                    : "버전 정보를 확인하는 중..."}
                </p>
              </div>
              <button
                type="button"
                className="settings-button settings-button-inline"
                onClick={() => void handleCheckForUpdates()}
                disabled={isCheckingUpdate}
              >
                {isCheckingUpdate ? "확인 중.." : "업데이트 확인"}
              </button>
            </div>
          </div>

          <div className="settings-card settings-card-feature">
            <label className="settings-label">{"\uC790\uB3D9 \uC7A0\uAE08"}</label>
            <SettingsDropdown
              value={Math.floor(settings.autoLockTimeoutSeconds / 60)}
              onChange={(minutes) =>
                setSettings((current) => ({
                  ...current,
                  autoLockTimeoutSeconds: Number(minutes) * 60,
                }))
              }
              ariaLabel={"\uC790\uB3D9 \uC7A0\uAE08"}
            />
            <p className="settings-help">
              {
                "\uB9C8\uC9C0\uB9C9 \uC785\uB825 \uD6C4 \uC124\uC815\uD55C \uC2DC\uAC04\uC774 \uC9C0\uB098\uBA74 qylock\uC774 \uC790\uB3D9\uC73C\uB85C \uC7A0\uAE08 \uD654\uBA74\uC744 \uC5FD\uB2C8\uB2E4."
              }
            </p>
          </div>

          <div className="settings-card settings-card-feature">
            <label className="settings-label">
              {"\uC7A0\uAE08 \uD6C4 \uB514\uC2A4\uD50C\uB808\uC774 \uB044\uAE30"}
            </label>
            <SettingsDropdown
              value={Math.floor(settings.blackoutTimeoutSeconds / 60)}
              onChange={(minutes) =>
                setSettings((current) => ({
                  ...current,
                  blackoutTimeoutSeconds: Number(minutes) * 60,
                }))
              }
              ariaLabel={"\uC7A0\uAE08 \uD6C4 \uB514\uC2A4\uD50C\uB808\uC774 \uB044\uAE30"}
            />
            <p className="settings-help">
              {
                "\uC124\uC815\uD55C \uC2DC\uAC04\uC774 \uC9C0\uB098\uBA74 Windows \uB514\uC2A4\uD50C\uB808\uC774 \uB044\uAE30 \uC2E0\uD638\uB97C \uBCF4\uB0C5\uB2C8\uB2E4."
              }
            </p>
          </div>

          <div className="settings-card settings-card-toggle">
            <div className="settings-toggle">
              <div className="settings-toggle-copy">
                <span className="settings-label">
                  {"\uC2E4\uD589 \uC2DC \uC790\uB3D9\uC73C\uB85C \uCF1C\uC9D0"}
                </span>
                <p className="settings-help">
                  {
                    "Windows \uB85C\uADF8\uC778 \uD6C4 qylock\uC774 \uC790\uB3D9 \uC2E4\uD589\uB418\uC5B4 \uD2B8\uB808\uC774\uC5D0\uC11C \uB300\uAE30\uD569\uB2C8\uB2E4."
                  }
                </p>
              </div>
              <button
                type="button"
                className={`settings-switch${settings.launchOnStartup ? " is-on" : ""}`}
                role="switch"
                aria-checked={settings.launchOnStartup}
                aria-label={"\uC2E4\uD589 \uC2DC \uC790\uB3D9\uC73C\uB85C \uCF1C\uC9D0"}
                onClick={() =>
                  setSettings((current) => ({
                    ...current,
                    launchOnStartup: !current.launchOnStartup,
                  }))
                }
              >
                <span className="settings-switch-thumb" />
              </button>
            </div>
          </div>

          <div className="settings-card settings-card-toggle">
            <div className="settings-toggle">
              <div className="settings-toggle-copy">
                <span className="settings-label">
                  {"\uD604\uC7AC \uC7AC\uC0DD \uC815\uBCF4 \uD45C\uC2DC"}
                </span>
                <p className="settings-help">
                  {
                    "\uC7A0\uAE08 \uD654\uBA74\uC5D0\uC11C TIDAL \uD604\uC7AC \uC7AC\uC0DD \uC815\uBCF4\uC640 \uBBF8\uB514\uC5B4 \uCEE8\uD2B8\uB864\uC744 \uD45C\uC2DC\uD569\uB2C8\uB2E4."
                  }
                </p>
              </div>
              <button
                type="button"
                className={`settings-switch${settings.mediaBridgeEnabled ? " is-on" : ""}`}
                role="switch"
                aria-checked={settings.mediaBridgeEnabled}
                aria-label={"\uD604\uC7AC \uC7AC\uC0DD \uC815\uBCF4 \uD45C\uC2DC"}
                onClick={() =>
                  setSettings((current) => ({
                    ...current,
                    mediaBridgeEnabled: !current.mediaBridgeEnabled,
                  }))
                }
              >
                <span className="settings-switch-thumb" />
              </button>
            </div>
          </div>
        </div>

        <div className="settings-actions">
          <button className="settings-button primary" onClick={handleSave} disabled={isSaving}>
            {isSaving ? "\uC800\uC7A5 \uC911.." : "\uC800\uC7A5"}
          </button>
          <button className="settings-button" onClick={handleLockNow}>
            {"\uC9C0\uAE08 \uC7A0\uAE08\uD558\uAE30"}
          </button>
          <button className="settings-button" onClick={() => void handleHide()}>
            {"\uD2B8\uB808\uC774\uB85C \uC228\uAE30\uAE30"}
          </button>
        </div>
      </div>

      {isUpdateModalOpen && updateResult && (
        <div
          className="settings-modal-overlay"
          onClick={() => !isDownloadingUpdate && setIsUpdateModalOpen(false)}
        >
          <div className="settings-modal-container" onClick={(e) => e.stopPropagation()}>
            <div className="settings-modal-header">
              <div>
                <p className="settings-kicker">New Update Available</p>
                <h2 className="settings-modal-title">새로운 버전이 나왔습니다!</h2>
              </div>
              <button
                type="button"
                className="settings-modal-close"
                onClick={() => setIsUpdateModalOpen(false)}
                disabled={isDownloadingUpdate}
              >
                ✕
              </button>
            </div>

            <div className="settings-modal-body">
              <div className="settings-modal-version-badges">
                <div className="settings-modal-version-badge">{updateResult.currentVersion}</div>
                <div className="settings-modal-arrow">➜</div>
                <div className="settings-modal-version-badge is-new">
                  {updateResult.latestVersion}
                </div>
              </div>

              <div className="settings-modal-release-notes">
                <h4>릴리즈 정보</h4>
                <div className="settings-modal-notes-content">
                  <p style={{ fontWeight: "bold", marginBottom: "8px", color: "#fff" }}>
                    {updateResult.releaseName || "커뮤니티 릴리즈"}
                  </p>
                  <p style={{ margin: 0 }}>
                    {updateResult.summary || "상세한 릴리즈 노트가 제공되지 않았습니다."}
                  </p>
                </div>
                {updateResult.publishedAt && (
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      alignItems: "center",
                      marginTop: "12px",
                    }}
                  >
                    <p className="settings-help" style={{ margin: 0 }}>
                      배포일: {formatPublishedDate(updateResult.publishedAt)}
                    </p>
                    <button
                      type="button"
                      className="settings-help"
                      style={{
                        background: "none",
                        border: "none",
                        padding: 0,
                        color: "#ffb84d",
                        cursor: "pointer",
                        textDecoration: "underline",
                      }}
                      onClick={() => void handleOpenReleasePage()}
                    >
                      상세 보기 (GitHub)
                    </button>
                  </div>
                )}
              </div>
            </div>

            <div className="settings-modal-footer">
              <button
                type="button"
                className="settings-modal-button is-later"
                onClick={() => setIsUpdateModalOpen(false)}
                disabled={isDownloadingUpdate}
              >
                나중에
              </button>
              <button
                type="button"
                className="settings-modal-button is-primary"
                onClick={() => void handleDownloadAndInstall()}
                disabled={isDownloadingUpdate}
              >
                {isDownloadingUpdate ? "다운로드 중..." : "지금 설치 및 다시 시작"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default SettingsScreen;
