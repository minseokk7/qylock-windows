import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { MouseEvent } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { type AppSettings, defaultSettings } from "../app-settings";

type LockScreenProps = {
  isMainLockWindow: boolean;
};

type NowPlayingInfo = {
  sourceKind: string;
  title: string;
  artist: string;
  album: string;
  thumbnail: string | null;
  status: string;
  positionMs: number;
  durationMs: number;
  updatedAt: number;
};

type MediaControlAction = "previous" | "togglePlayPause" | "next";

function formatPlaybackStatus(status: string) {
  switch (status) {
    case "playing":
      return "\uC7AC\uC0DD \uC911";
    case "paused":
      return "\uC77C\uC2DC\uC815\uC9C0";
    default:
      return "\uB300\uAE30 \uC911";
  }
}

function LockScreen({ isMainLockWindow }: LockScreenProps) {
  const [time, setTime] = useState(new Date());
  const [, setSettings] = useState<AppSettings>(defaultSettings);
  const [nowPlaying, setNowPlaying] = useState<NowPlayingInfo | null>(null);
  const [displayPositionMs, setDisplayPositionMs] = useState(0);
  const blackoutTimerRef = useRef<number | null>(null);
  const blackoutSecondsRef = useRef(0);
  const isLockedRef = useRef(false);

  const handleMediaControl = async (
    action: MediaControlAction,
    event: MouseEvent<HTMLButtonElement>,
  ) => {
    event.stopPropagation();

    try {
      await invoke<boolean>("control_now_playing", { action });
    } catch (error) {
      console.error("Failed to control media session:", error);
    }
  };

  const progressRatio = useMemo(() => {
    if (!nowPlaying || nowPlaying.durationMs <= 0) {
      return 0;
    }

    return Math.max(
      0,
      Math.min(100, (displayPositionMs / nowPlaying.durationMs) * 100),
    );
  }, [displayPositionMs, nowPlaying]);

  const clearBlackoutTimer = () => {
    if (blackoutTimerRef.current !== null) {
      window.clearTimeout(blackoutTimerRef.current);
      blackoutTimerRef.current = null;
    }
  };

  const restartBlackoutTimer = (seconds: number, locked: boolean) => {
    clearBlackoutTimer();

    if (!locked || seconds <= 0) {
      return;
    }

    blackoutTimerRef.current = window.setTimeout(() => {
      void invoke("turn_off_display").catch((error) => {
        console.error("Failed to turn off display:", error);
      });
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
    let unlistenMedia: UnlistenFn | null = null;

    void invoke<NowPlayingInfo | null>("get_now_playing_snapshot")
      .then((snapshot) => {
        if (active) {
          setNowPlaying(snapshot);
        }
      })
      .catch((error) => {
        console.error("Failed to read now playing snapshot:", error);
        if (active) {
          setNowPlaying(null);
        }
      });

    void listen<NowPlayingInfo | null>("media-now-playing-updated", (event) => {
      if (active) {
        setNowPlaying(event.payload);
      }
    }).then((unlisten) => {
      unlistenMedia = unlisten;
    });

    return () => {
      active = false;
      if (unlistenMedia) {
        void unlistenMedia();
      }
    };
  }, []);

  useEffect(() => {
    if (!nowPlaying) {
      setDisplayPositionMs(0);
      return;
    }

    const syncDisplayedPosition = () => {
      if (nowPlaying.status !== "playing") {
        setDisplayPositionMs(nowPlaying.positionMs);
        return;
      }

      const elapsed = Math.max(0, Date.now() - nowPlaying.updatedAt);
      const nextPosition = Math.min(
        nowPlaying.durationMs || Number.MAX_SAFE_INTEGER,
        nowPlaying.positionMs + elapsed,
      );
      setDisplayPositionMs(nextPosition);
    };

    syncDisplayedPosition();
    const intervalId = window.setInterval(syncDisplayedPosition, 250);

    return () => window.clearInterval(intervalId);
  }, [nowPlaying]);

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

  const refreshDisplayOffTimer = () => {
    if (blackoutSecondsRef.current > 0) {
      restartBlackoutTimer(blackoutSecondsRef.current, isLockedRef.current);
    }
  };

  const handleUnlock = async () => {
    try {
      await invoke("verify_hello");
    } catch (error) {
      console.error("Error during verification:", error);
    }
  };

  return (
    <div
      className="lock-container"
      onDoubleClick={handleUnlock}
      onMouseMove={refreshDisplayOffTimer}
      onMouseDown={refreshDisplayOffTimer}
      onTouchStart={refreshDisplayOffTimer}
    >
      <video className="video-bg" autoPlay loop muted playsInline>
        <source src="/bg.mp4" type="video/mp4" />
      </video>

      {import.meta.env.DEV && isMainLockWindow ? (
        <div className="close-btn" onClick={() => invoke("close_app")}>
          &times;
        </div>
      ) : null}

      <div className="overlay">
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

        <div className="lock-actions">
          <div className="unlock-prompt" onClick={handleUnlock}>
            TAP TO UNLOCK
          </div>

          {nowPlaying ? (
            <div
              className="now-playing"
              aria-live="polite"
              onClick={(event) => event.stopPropagation()}
              onDoubleClick={(event) => event.stopPropagation()}
            >
              <div className="now-playing-art">
                {nowPlaying.thumbnail ? (
                  <img
                    className="now-playing-art-image"
                    src={nowPlaying.thumbnail}
                    alt={`${nowPlaying.title} \uC568\uBC94 \uC544\uD2B8`}
                  />
                ) : (
                  <div className="now-playing-art-placeholder">TIDAL</div>
                )}
              </div>

              <div className="now-playing-body">
                <div className="now-playing-topline">
                  <span className="now-playing-app">TIDAL</span>
                  <span
                    className={`now-playing-status is-${nowPlaying.status}`}
                  >
                    {formatPlaybackStatus(nowPlaying.status)}
                  </span>
                </div>

                <div className="now-playing-title">{nowPlaying.title}</div>

                {nowPlaying.artist ? (
                  <div className="now-playing-artist">{nowPlaying.artist}</div>
                ) : null}

                {nowPlaying.album ? (
                  <div className="now-playing-album">{nowPlaying.album}</div>
                ) : null}

                {nowPlaying.durationMs > 0 ? (
                  <div className="now-playing-progress">
                    <div
                      className="now-playing-progress-fill"
                      style={{ width: `${progressRatio}%` }}
                    />
                  </div>
                ) : null}

                <div className="now-playing-controls">
                  <button
                    className="now-playing-control-button"
                    type="button"
                    aria-label="이전 곡"
                    onClick={(event) => void handleMediaControl("previous", event)}
                  >
                    <span className="media-icon media-icon-previous" aria-hidden="true" />
                  </button>
                  <button
                    className="now-playing-control-button is-primary"
                    type="button"
                    aria-label={
                      nowPlaying.status === "playing" ? "일시정지" : "재생"
                    }
                    onClick={(event) =>
                      void handleMediaControl("togglePlayPause", event)
                    }
                  >
                    <span
                      className={`media-icon ${
                        nowPlaying.status === "playing"
                          ? "media-icon-pause"
                          : "media-icon-play"
                      }`}
                      aria-hidden="true"
                    />
                  </button>
                  <button
                    className="now-playing-control-button"
                    type="button"
                    aria-label="다음 곡"
                    onClick={(event) => void handleMediaControl("next", event)}
                  >
                    <span className="media-icon media-icon-next" aria-hidden="true" />
                  </button>
                </div>
              </div>
            </div>
          ) : null}
        </div>
      </div>

    </div>
  );
}

export default LockScreen;
