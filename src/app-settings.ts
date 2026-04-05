export type AppSettings = {
  autoLockTimeoutSeconds: number;
  blackoutTimeoutSeconds: number;
  launchOnStartup: boolean;
  mediaBridgeEnabled: boolean;
};

export type SettingsOption<T extends string | number = string | number> = {
  label: string;
  value: T;
};

export type MinuteOption = SettingsOption<number>;

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
  mediaBridgeEnabled: true,
};

export const minuteOptions: MinuteOption[] = [
  { label: "\uC0AC\uC6A9 \uC548 \uD568", value: 0 },
  { label: "1\uBD84", value: 1 },
  { label: "3\uBD84", value: 3 },
  { label: "5\uBD84", value: 5 },
  { label: "10\uBD84", value: 10 },
  { label: "15\uBD84", value: 15 },
  { label: "30\uBD84", value: 30 },
  { label: "60\uBD84", value: 60 },
];

export function formatBlackoutLabel(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  return minutes <= 0
    ? "\uB514\uC2A4\uD50C\uB808\uC774 \uB044\uAE30 \uC548 \uD568"
    : `${minutes}\uBD84 \uD6C4 \uB514\uC2A4\uD50C\uB808\uC774 \uB044\uAE30`;
}
