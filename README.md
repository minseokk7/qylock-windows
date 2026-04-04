# qylock-windows

`qylock-windows`는 Windows 시스템 트레이에서 동작하는 Tauri 기반 잠금 화면 앱입니다.  
전역 단축키로 바로 잠그거나, 마지막 입력 이후 일정 시간이 지나면 자동으로 잠그고, 잠금 뒤에는 검은 화면으로 전환할 수 있습니다.

## 주요 기능

- `Ctrl+Alt+L` 단축키로 즉시 잠금
- 마지막 입력 후 자동 잠금
- 잠금 후 지정 시간 뒤 검은 화면 전환
- Windows 로그인 후 자동 실행
- 시스템 트레이에서 설정 열기, 즉시 잠금, 종료
- Windows Hello를 통한 잠금 해제

## 실행 환경

- Windows 10 또는 Windows 11
- Node.js 18 이상
- Rust toolchain
- Microsoft Edge WebView2 Runtime

## 개발 실행

```bash
npm install
npm run tauri dev
```

## 검증과 빌드

프런트엔드 빌드:

```bash
npm run build
```

Rust 검증:

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

릴리스 빌드:

```bash
npm run tauri build
```

## 설정 항목

- `잠금 단축키`: 현재 `Ctrl+Alt+L`
- `자동 잠금`: 마지막 입력 후 자동으로 잠그기까지의 시간
- `잠금 후 검은 화면 켜기`: 잠금 화면이 검은 화면으로 전환되기까지의 시간
- `실행 시 자동으로 켜짐`: Windows 로그인 후 트레이에서 자동 대기

## 릴리스 파일

GitHub Release에는 설치 파일인 `qylock-windows_<version>_x64-setup.exe`만 업로드하는 것을 기준으로 사용합니다.

로컬 릴리스 빌드 결과물은 기본적으로 아래 경로에 생성됩니다.

- `src-tauri/target/release/qylock-windows.exe`
- `src-tauri/target/release/bundle/nsis/qylock-windows_<version>_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/qylock-windows_<version>_x64_en-US.msi`

## 프로젝트 구조

- `src/screens/LockScreen.tsx`: 잠금 화면 UI
- `src/screens/SettingsScreen.tsx`: 설정 창 UI
- `src/components/SettingsDropdown.tsx`: 설정용 드롭다운 컴포넌트
- `src/app-settings.ts`: 설정 타입과 공통 옵션
- `src-tauri/src/lib.rs`: 잠금, 트레이, 설정 저장, 자동 잠금 로직
