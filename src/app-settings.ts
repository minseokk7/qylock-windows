export type AppSettings = {
  autoLockTimeoutSeconds: number;
  blackoutTimeoutSeconds: number;
  launchOnStartup: boolean;
};

export type MinuteOption = {
  label: string;
  value: number;
};

export type MenuPosition = {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
  direction: "down" | "up";
};

export const defaultSettings: AppSettings = {
  autoLockTimeoutSeconds: 0,
  blackoutTimeoutSeconds: 0,
  launchOnStartup: false,
};

export const minuteOptions: MinuteOption[] = [
  { label: "사용 안 함", value: 0 },
  { label: "1분", value: 1 },
  { label: "3분", value: 3 },
  { label: "5분", value: 5 },
  { label: "10분", value: 10 },
  { label: "15분", value: 15 },
  { label: "30분", value: 30 },
  { label: "60분", value: 60 },
];

export function formatBlackoutLabel(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  return minutes <= 0 ? "검은 화면 전환 없음" : `${minutes}분 후 검은 화면`;
}
