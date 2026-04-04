import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { type MouseEvent as ReactMouseEvent, useEffect, useState } from "react";
import SettingsDropdown from "../components/SettingsDropdown";
import { type AppSettings, defaultSettings } from "../app-settings";

type ToastState = {
  message: string;
  tone: "success" | "error";
};

function SettingsScreen() {
  const currentWindow = getCurrentWindow();
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [isSaving, setIsSaving] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | null = null;

    void invoke<AppSettings>("get_settings").then((nextSettings) => {
      if (active) {
        setSettings(nextSettings);
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
    };

    setIsSaving(true);
    setToast(null);

    try {
      await invoke("save_settings", { settings: normalizedSettings });
      setSettings(normalizedSettings);
      setToast({ message: "설정을 저장했습니다.", tone: "success" });
    } catch (error) {
      console.error(error);
      setToast({ message: "설정을 저장하지 못했습니다.", tone: "error" });
    } finally {
      setIsSaving(false);
    }
  };

  const handleHide = async () => {
    setToast(null);

    try {
      await invoke("hide_settings_window");
    } catch (error) {
      console.error(error);
      setToast({ message: "설정 창을 숨기지 못했습니다.", tone: "error" });
    }
  };

  const handleLockNow = async () => {
    await invoke("lock_screen");
    await currentWindow.hide();
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
              title="창 이동"
            />

            <div className="settings-window-actions">
              <button
                type="button"
                className="settings-header-button"
                aria-label="최소화"
                onClick={() => void currentWindow.minimize()}
              >
                -
              </button>
              <button
                type="button"
                className="settings-header-button is-close"
                aria-label="닫기"
                onClick={() => void handleHide()}
              >
                x
              </button>
            </div>
          </div>

          <div
            className="settings-header-copy"
            data-tauri-drag-region
            onMouseDown={handleDragMouseDown}
          >
            <p className="settings-kicker">qylock</p>
            <h1>설정</h1>
            <p className="settings-subtitle">
              시스템 트레이에서 창을 열어 잠금 동작을 조정할 수 있습니다.
            </p>
          </div>
        </div>

        <div className="settings-card">
          <label className="settings-label" htmlFor="hotkey">
            잠금 단축키
          </label>
          <input
            id="hotkey"
            className="settings-input is-readonly"
            value="Ctrl+Alt+L"
            readOnly
          />
          <p className="settings-help">
            qylock이 트레이에 실행 중일 때 누르면 잠금 화면이 열립니다.
          </p>
        </div>

        <div className="settings-card">
          <label className="settings-label">자동 잠금</label>
          <SettingsDropdown
            valueMinutes={Math.floor(settings.autoLockTimeoutSeconds / 60)}
            onChange={(minutes) =>
              setSettings((current) => ({
                ...current,
                autoLockTimeoutSeconds: minutes * 60,
              }))
            }
            ariaLabel="자동 잠금"
          />
          <p className="settings-help">
            마지막 입력 후 설정한 시간이 지나면 qylock이 자동으로 잠금 화면을 엽니다.
          </p>
        </div>

        <div className="settings-card">
          <label className="settings-label">잠금 후 검은 화면 켜기</label>
          <SettingsDropdown
            valueMinutes={Math.floor(settings.blackoutTimeoutSeconds / 60)}
            onChange={(minutes) =>
              setSettings((current) => ({
                ...current,
                blackoutTimeoutSeconds: minutes * 60,
              }))
            }
            ariaLabel="잠금 후 검은 화면 켜기"
          />
          <p className="settings-help">
            설정한 시간이 지나면 잠금 화면이 검은 화면으로 전환됩니다.
          </p>
        </div>

        <div className="settings-card">
          <div className="settings-toggle">
            <div className="settings-toggle-copy">
              <span className="settings-label">실행 시 자동으로 켜짐</span>
              <p className="settings-help">
                Windows 로그인 후 qylock이 자동 실행되어 트레이에서 대기합니다.
              </p>
            </div>
            <button
              type="button"
              className={`settings-switch${settings.launchOnStartup ? " is-on" : ""}`}
              role="switch"
              aria-checked={settings.launchOnStartup}
              aria-label="실행 시 자동으로 켜짐"
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

        <div className="settings-actions">
          <button className="settings-button primary" onClick={handleSave} disabled={isSaving}>
            {isSaving ? "저장 중..." : "저장"}
          </button>
          <button className="settings-button" onClick={handleLockNow}>
            지금 잠그기
          </button>
          <button className="settings-button" onClick={() => void handleHide()}>
            트레이로 숨기기
          </button>
        </div>
      </div>
    </div>
  );
}

export default SettingsScreen;
