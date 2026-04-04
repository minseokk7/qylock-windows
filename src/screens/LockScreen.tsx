import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import {
  type AppSettings,
  defaultSettings,
  formatBlackoutLabel,
} from "../app-settings";

type LockScreenProps = {
  isMainLockWindow: boolean;
};

function LockScreen({ isMainLockWindow }: LockScreenProps) {
  const [time, setTime] = useState(new Date());
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [isBlackout, setIsBlackout] = useState(false);
  const blackoutTimerRef = useRef<number | null>(null);
  const blackoutSecondsRef = useRef(0);
  const isLockedRef = useRef(false);

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
        syncSettings(nextSettings, isLockedRef.current);
      }
    });

    void invoke<boolean>("get_lock_state").then((locked) => {
      if (active) {
        isLockedRef.current = locked;
        restartBlackoutTimer(blackoutSecondsRef.current, locked);
      }
    });

    void listen<AppSettings>("settings-updated", (event) => {
      if (active) {
        syncSettings(event.payload, isLockedRef.current);
      }
    }).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    void listen<boolean>("lock-state-changed", (event) => {
      if (!active) {
        return;
      }

      isLockedRef.current = event.payload;
      restartBlackoutTimer(blackoutSecondsRef.current, event.payload);
    }).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    return () => {
      active = false;
      clearBlackoutTimer();
      unlisteners.forEach((unlisten) => void unlisten());
    };
  }, []);

  const wakeBlackout = () => {
    if (blackoutSecondsRef.current > 0) {
      restartBlackoutTimer(blackoutSecondsRef.current, isLockedRef.current);
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

export default LockScreen;
