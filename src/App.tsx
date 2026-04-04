import { getCurrentWindow } from "@tauri-apps/api/window";
import LockScreen from "./screens/LockScreen";
import SettingsScreen from "./screens/SettingsScreen";
import "./App.css";

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
  return isSettingsWindow ? (
    <SettingsScreen />
  ) : (
    <LockScreen isMainLockWindow={isMainLockWindow} />
  );
}

export default App;
