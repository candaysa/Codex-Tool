import React from "react";

export interface ToggleSwitchProps {
  id?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  size?: "sm" | "md" | "lg";
  label?: string;
}

export const ToggleSwitch: React.FC<ToggleSwitchProps> = ({
  id,
  checked,
  onChange,
  disabled = false,
  size = "md",
  label,
}) => {
  const sizeClasses = {
    sm: {
      track: "w-8 h-4",
      knob: "w-3 h-3",
      translate: "translate-x-4",
    },
    md: {
      track: "w-11 h-6",
      knob: "w-4 h-4",
      translate: "translate-x-5",
    },
    lg: {
      track: "w-14 h-7",
      knob: "w-5 h-5",
      translate: "translate-x-7",
    },
  }[size];

  return (
    <label
      htmlFor={id}
      className={`inline-flex items-center gap-2 cursor-pointer select-none ${
        disabled ? "opacity-50 cursor-not-allowed" : ""
      }`}
    >
      <div className="relative">
        <input
          type="checkbox"
          role="switch"
          aria-checked={checked}
          id={id}
          checked={checked}
          onChange={(e) => !disabled && onChange(e.target.checked)}
          disabled={disabled}
          className="sr-only peer"
        />
        {/* Slider Track */}
        <div
          className={`${sizeClasses.track} rounded-full transition-colors duration-200 ease-in-out peer-focus-visible:ring-2 peer-focus-visible:ring-brand-400 ${
            checked
              ? "bg-brand-500 shadow-[0_0_12px_rgba(14,165,233,0.4)]"
              : "bg-slate-300 dark:bg-slate-700/80 hover:bg-slate-400 dark:hover:bg-slate-700"
          }`}
        />
        {/* Sliding Knob */}
        <div
          className={`absolute top-1 left-1 bg-white rounded-full transition-transform duration-200 ease-in-out shadow-sm ${
            sizeClasses.knob
          } ${checked ? sizeClasses.translate : "translate-x-0"}`}
        />
      </div>
      {label && (
        <span className="text-xs font-medium text-slate-700 dark:text-slate-300">
          {label}
        </span>
      )}
    </label>
  );
};
