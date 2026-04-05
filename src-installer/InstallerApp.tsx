import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useState } from "react";

type InstallerMetadata = {
  appVersion: string;
  installerVersion: string;
  bundledSetupName: string;
};

type InstallPhase = "idle" | "installing" | "success" | "error";

const isTauriRuntime = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function InstallerApp() {
  const [phase, setPhase] = useState<InstallPhase>("idle");
  const [message, setMessage] = useState("설치 대기 중입니다.");
  const [metadata, setMetadata] = useState<InstallerMetadata | null>(null);
  const [isConsentOpen, setIsConsentOpen] = useState(false);

  useEffect(() => {
    void invoke<InstallerMetadata>("get_installer_metadata")
      .then((value) => setMetadata(value))
      .catch(() => {
        setMetadata(null);
      });
  }, []);

  const progressWidth = useMemo(() => {
    switch (phase) {
      case "idle":
        return "0%";
      case "installing":
        return "72%";
      case "success":
      case "error":
        return "100%";
    }
  }, [phase]);

  const handleMinimize = async () => {
    if (!isTauriRuntime()) {
      return;
    }

    await getCurrentWindow().minimize();
  };

  const handleClose = async () => {
    if (!isTauriRuntime()) {
      return;
    }

    await getCurrentWindow().close();
  };

  const handleInstall = async () => {
    setPhase("installing");
    setMessage("qylock 설치 파일을 준비하고 있습니다.");

    window.setTimeout(() => {
      setMessage("설치 프로그램을 조용히 실행하는 중입니다.");
    }, 550);

    try {
      await invoke("install_qylock");
      setPhase("success");
      setMessage("설치가 완료되었습니다. 창을 닫고 qylock을 실행하면 됩니다.");
    } catch (error) {
      console.error(error);
      setPhase("error");
      setMessage("설치를 완료하지 못했습니다. 다시 시도해 주세요.");
    }
  };

  const handleInstallClick = () => {
    if (phase === "installing" || phase === "success") {
      return;
    }

    setIsConsentOpen(true);
  };

  const handleConsentConfirm = async () => {
    setIsConsentOpen(false);
    await handleInstall();
  };

  return (
    <div className="installer-shell">
      <div className="installer-frame">
        <div className="installer-topbar">
          <div data-tauri-drag-region className="installer-drag-region" />
          <button
            type="button"
            className="installer-window-button"
            aria-label="최소화"
            onClick={() => void handleMinimize()}
          >
            -
          </button>
          <button
            type="button"
            className="installer-window-button is-close"
            aria-label="닫기"
            onClick={() => void handleClose()}
          >
            x
          </button>
        </div>

        <section className="installer-hero">
          <div className="installer-brand-block">
            <p className="installer-kicker">QYLOCK INSTALLER</p>
            <h1>데스크톱 잠금을 빠르게 설치</h1>
            <p className="installer-copy">
              Windows Hello 기반 잠금 화면, 디스플레이 끄기 타이머, 트레이 실행 흐름을
              한 번에 설치합니다.
            </p>

            <div className="installer-version-row">
              <span>{`앱 버전 ${metadata?.appVersion ?? "0.1.4"}`}</span>
              <span>{`인스톨러 ${metadata?.installerVersion ?? "custom"}`}</span>
            </div>
          </div>

          <div className="installer-stage">
            <div className="installer-stage-orb" />
            <div className="installer-stage-panel">
              <p className="installer-stage-label">설치 대상</p>
              <strong>{metadata?.bundledSetupName ?? "qylock-windows setup"}</strong>
              <p className="installer-stage-help">
                현재 사용자 기준으로 조용히 설치되며, 별도 콘솔 창 없이 마무리됩니다.
              </p>
            </div>
          </div>
        </section>

        <section className="installer-body">
          <div className="installer-column">
            <div className="installer-block">
              <p className="installer-block-label">포함 기능</p>
              <ul className="installer-feature-list">
                <li>Windows Hello 잠금 해제</li>
                <li>잠금 후 디스플레이 끄기</li>
                <li>TIDAL 미디어 세션 표시</li>
                <li>트레이 상주 및 자동 실행</li>
              </ul>
            </div>

            <div className="installer-block">
              <p className="installer-block-label">설치 방식</p>
              <p className="installer-body-copy">
                내부적으로 qylock 설치 파일을 silent 모드로 실행합니다. 설치가 끝날
                때까지 이 창을 닫지 않는 편이 안전합니다.
              </p>
            </div>
          </div>

          <div className="installer-column">
            <div className="installer-progress-block">
              <div className="installer-progress-head">
                <span>설치 진행</span>
                <strong>
                  {phase === "idle"
                    ? "대기"
                    : phase === "installing"
                      ? "진행 중"
                      : phase === "success"
                        ? "완료"
                        : "오류"}
                </strong>
              </div>

              <div className="installer-progress-track" aria-hidden="true">
                <div
                  className={`installer-progress-fill is-${phase}`}
                  style={{ width: progressWidth }}
                />
              </div>

              <p className="installer-progress-message">{message}</p>

              <div className="installer-step-list">
                <div className={`installer-step ${phase !== "idle" ? "is-done" : ""}`}>
                  설치 파일 준비
                </div>
                <div
                  className={`installer-step ${
                    phase === "installing" || phase === "success" ? "is-done" : ""
                  }`}
                >
                  조용한 설치 실행
                </div>
                <div className={`installer-step ${phase === "success" ? "is-done" : ""}`}>
                  완료 및 마무리
                </div>
              </div>
            </div>

            <div className="installer-actions">
              <button
                type="button"
                className="installer-button is-primary"
                disabled={phase === "installing" || phase === "success"}
                onClick={() => void handleInstallClick()}
              >
                {phase === "installing"
                  ? "설치 중..."
                  : phase === "success"
                    ? "설치 완료"
                    : "설치 시작"}
              </button>
            </div>
          </div>
        </section>

        {isConsentOpen ? (
          <div className="installer-consent-backdrop">
            <div className="installer-consent-modal">
              <p className="installer-block-label">설치 동의</p>
              <h2>qylock를 설치할까요?</h2>
              <p className="installer-body-copy">
                설치를 진행하면 현재 사용자 계정 기준으로 qylock이 조용히 설치됩니다.
                설치가 시작된 뒤에는 완료될 때까지 잠시 기다려 주세요.
              </p>
              <ul className="installer-consent-list">
                <li>{`설치 파일: ${metadata?.bundledSetupName ?? "qylock-windows setup"}`}</li>
                <li>설치 방식: 백그라운드 무인 설치</li>
                <li>설치 후 시작 메뉴와 실행 파일이 등록됩니다.</li>
              </ul>

              <div className="installer-consent-actions">
                <button
                  type="button"
                  className="installer-button"
                  onClick={() => setIsConsentOpen(false)}
                >
                  취소
                </button>
                <button
                  type="button"
                  className="installer-button is-primary"
                  onClick={() => void handleConsentConfirm()}
                >
                  동의하고 설치
                </button>
              </div>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export default InstallerApp;
