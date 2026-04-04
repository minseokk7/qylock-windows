import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import "./App.css";

type AppSettings = {
  autoLockTimeoutSeconds: number;
  blackoutTimeoutSeconds: number;
  launchOnStartup: boolean;
};

type MinuteOption = {
  label: string;
  value: number;
};

type MenuPosition = {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
  direction: "down" | "up";
};

const defaultSettings: AppSettings = {
  autoLockTimeoutSeconds: 0,
  blackoutTimeoutSeconds: 0,
  launchOnStartup: false,
};

const blackoutMinuteOptions: MinuteOption[] = [
  { label: "사용 안 함", value: 0 },
  { label: "1분", value: 1 },
  { label: "3분", value: 3 },
  { label: "5분", value: 5 },
  { label: "10분", value: 10 },
  { label: "15분", value: 15 },
  { label: "30분", value: 30 },
  { label: "60분", value: 60 },
];

const windowLabel = (() => {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
})();

const isSettingsWindow = windowLabel === "settings";
const isMainLockWindow = windowLabel === "main";

function App() {
  return isSettingsWindow ? <SettingsScreen /> : <LockScreen />;
}

function formatBlackoutLabel(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  return minutes <= 0 ? "검은 화면 전환 없음" : `${minutes}분 후 검은 화면`;
}

function LockScreen() {
  const [time, setTime] = useState(new Date());
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [isLocked, setIsLocked] = useState(false);
  const [isBlackout, setIsBlackout] = useState(false);
  const blackoutTimerRef = useRef<number | null>(null);
  const blackoutSecondsRef = useRef(0);

  const clearBlackoutTimer = () => {
    if (blackoutTimerRef.current !== null) {
      window.clearTimeout(blackoutTimerRef.current);
      blackoutTimerRef.current = null;
    }
  };

  const restartBlackoutTimer = (seconds: number, locked: boolean) => {
    clearBlackoutTimer();
    setIsBlackout(false);

    if (!locked || seconds <= 0) {
      return;
    }

    blackoutTimerRef.current = window.setTimeout(() => {
      setIsBlackout(true);
    }, seconds * 1000);
  };

  useEffect(() => {
    const timer = window.setInterval(() => {
      setTime(new Date());
    }, 1000);

    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let active = true;
    const unlisteners: UnlistenFn[] = [];

    const syncSettings = (nextSettings: AppSettings, locked: boolean) => {
      blackoutSecondsRef.current = nextSettings.blackoutTimeoutSeconds;
      setSettings(nextSettings);
      restartBlackoutTimer(nextSettings.blackoutTimeoutSeconds, locked);
    };

    void invoke<AppSettings>("get_settings").then((nextSettings) => {
      if (active) {
        syncSettings(nextSettings, isLocked);
      }
    });

    void invoke<boolean>("get_lock_state").then((locked) => {
      if (active) {
        setIsLocked(locked);
        restartBlackoutTimer(blackoutSecondsRef.current, locked);
      }
    });

    void listen<AppSettings>("settings-updated", (event) => {
      if (active) {
        syncSettings(event.payload, isLocked);
      }
    }).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    void listen<boolean>("lock-state-changed", (event) => {
      if (!active) {
        return;
      }

      setIsLocked(event.payload);
      restartBlackoutTimer(blackoutSecondsRef.current, event.payload);
    }).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    return () => {
      active = false;
      clearBlackoutTimer();
      unlisteners.forEach((unlisten) => void unlisten());
    };
  }, [isLocked]);

  const wakeBlackout = () => {
    if (blackoutSecondsRef.current > 0) {
      restartBlackoutTimer(blackoutSecondsRef.current, isLocked);
    } else {
      setIsBlackout(false);
    }
  };

  const handleUnlock = async () => {
    if (isBlackout) {
      wakeBlackout();
      return;
    }

    try {
      await invoke("verify_hello");
    } catch (error) {
      console.error("Error during verification:", error);
    }
  };

  return (
    <div
      className={`lock-container${isBlackout ? " is-blackout" : ""}`}
      onDoubleClick={handleUnlock}
      onMouseMove={wakeBlackout}
      onMouseDown={wakeBlackout}
      onTouchStart={wakeBlackout}
    >
      <video className="video-bg" autoPlay loop muted playsInline>
        <source src="/bg.mp4" type="video/mp4" />
      </video>

      {import.meta.env.DEV && isMainLockWindow ? (
        <div className="close-btn" onClick={() => invoke("close_app")}>
          &times;
        </div>
      ) : null}

      <div className={`overlay${isBlackout ? " is-hidden" : ""}`}>
        <div className="time">
          {time.toLocaleTimeString("en-US", {
            hour12: false,
            hour: "2-digit",
            minute: "2-digit",
          })}
        </div>
        <div className="date">
          {time.toLocaleDateString("ko-KR", {
            weekday: "long",
            year: "numeric",
            month: "long",
            day: "numeric",
          })}
        </div>

        <div className="unlock-prompt" onClick={handleUnlock}>
          TAP TO UNLOCK
        </div>

        {settings.blackoutTimeoutSeconds > 0 ? (
          <div className="timeout-note">{formatBlackoutLabel(settings.blackoutTimeoutSeconds)}</div>
        ) : null}
      </div>

      {isBlackout ? <div className="blackout-layer" /> : null}
    </div>
  );
}

function SettingsScreen() {
  const currentWindow = getCurrentWindow();
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [isSaving, setIsSaving] = useState(false);
  const [status, setStatus] = useState("");

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
    setStatus("");

    try {
      await invoke("save_settings", { settings: normalizedSettings });
      setSettings(normalizedSettings);
      setStatus("설정을 저장했습니다.");
    } catch (error) {
      console.error(error);
      setStatus("설정을 저장하지 못했습니다.");
    } finally {
      setIsSaving(false);
    }
  };

  const handleHide = async () => {
    setStatus("");

    try {
      await invoke("hide_settings_window");
    } catch (error) {
      console.error(error);
      setStatus("설정 창을 숨기지 못했습니다.");
    }
  };

  const handleLockNow = async () => {
    await invoke("lock_screen");
    await currentWindow.hide();
  };

  const handleDragMouseDown = (event: React.MouseEvent<HTMLDivElement, MouseEvent>) => {
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
              —
            </button>
            <button
              type="button"
              className="settings-header-button is-close"
              aria-label="닫기"
              onClick={() => void handleHide()}
            >
              ×
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
            {isSaving ? "저장 중.." : "저장"}
          </button>
          <button className="settings-button" onClick={handleLockNow}>
            지금 잠그기
          </button>
          <button className="settings-button" onClick={() => void handleHide()}>
            트레이로 숨기기
          </button>
        </div>

        {status ? <p className="settings-status">{status}</p> : null}
      </div>
    </div>
  );
}

function SettingsDropdown({
  valueMinutes,
  onChange,
}: {
  valueMinutes: number;
  onChange: (minutes: number) => void;
}) {
  const menuGap = 10;
  const viewportPadding = 16;
  const minMenuHeight = 180;
  const [isOpen, setIsOpen] = useState(false);
  const [menuPosition, setMenuPosition] = useState<MenuPosition | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  const selectedOption = useMemo(
    () =>
      blackoutMinuteOptions.find((option) => option.value === valueMinutes) ??
      blackoutMinuteOptions[0],
    [valueMinutes],
  );

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    const updatePosition = () => {
      const rect = triggerRef.current?.getBoundingClientRect();
      if (!rect) {
        return;
      }

      const availableBelow = window.innerHeight - rect.bottom - viewportPadding;
      const availableAbove = rect.top - viewportPadding;
      const shouldOpenUp = availableBelow < minMenuHeight && availableAbove > availableBelow;
      const maxHeight = Math.max(
        120,
        shouldOpenUp ? availableAbove - menuGap : availableBelow - menuGap,
      );

      setMenuPosition({
        left: rect.left,
        top: shouldOpenUp ? rect.top - menuGap : rect.bottom + menuGap,
        width: rect.width,
        maxHeight,
        direction: shouldOpenUp ? "up" : "down",
      });
    };

    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (!rootRef.current?.contains(target) && !menuRef.current?.contains(target)) {
        setIsOpen(false);
      }
    };

    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsOpen(false);
      }
    };

    updatePosition();
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);

    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [isOpen]);

  return (
    <div className={`settings-dropdown${isOpen ? " is-open" : ""}`} ref={rootRef}>
      <button
        ref={triggerRef}
        type="button"
        className="settings-dropdown-trigger"
        aria-haspopup="listbox"
        aria-expanded={isOpen}
        onClick={() => setIsOpen((current) => !current)}
      >
        <span>{selectedOption.label}</span>
        <span className="settings-dropdown-caret" />
      </button>

      {isOpen && menuPosition
        ? createPortal(
            <div
              ref={menuRef}
              className={`settings-dropdown-menu is-portal${
                menuPosition.direction === "up" ? " opens-up" : ""
              }`}
              role="listbox"
              aria-label="잠금 후 검은 화면 켜기"
              style={{
                left: `${menuPosition.left}px`,
                top: `${menuPosition.top}px`,
                width: `${menuPosition.width}px`,
                maxHeight: `${menuPosition.maxHeight}px`,
              }}
            >
              {blackoutMinuteOptions.map((option) => {
                const selected = option.value === selectedOption.value;

                return (
                  <button
                    key={option.value}
                    type="button"
                    className={`settings-dropdown-option${selected ? " is-selected" : ""}`}
                    role="option"
                    aria-selected={selected}
                    onClick={() => {
                      onChange(option.value);
                      setIsOpen(false);
                    }}
                  >
                    {option.label}
                  </button>
                );
              })}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}

export default App;
