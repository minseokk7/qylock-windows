import { createPortal } from "react-dom";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  type MenuPosition,
  type SettingsOption,
  minuteOptions,
} from "../app-settings";

type SettingsDropdownProps = {
  value: string | number;
  onChange: (value: string | number) => void;
  ariaLabel: string;
  options?: SettingsOption[];
};

const menuGap = 10;
const viewportPadding = 16;
const minMenuHeight = 180;

function SettingsDropdown({
  value,
  onChange,
  ariaLabel,
  options = minuteOptions,
}: SettingsDropdownProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [menuPosition, setMenuPosition] = useState<MenuPosition | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  const selectedOption = useMemo(
    () => options.find((option) => option.value === value) ?? options[0],
    [options, value],
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
              aria-label={ariaLabel}
              style={{
                left: `${menuPosition.left}px`,
                top: `${menuPosition.top}px`,
                width: `${menuPosition.width}px`,
                maxHeight: `${menuPosition.maxHeight}px`,
              }}
            >
              {options.map((option) => {
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

export default SettingsDropdown;
