import React from "react";
import { Minus, X, ArrowDownToDot, Sun, Moon } from "lucide-react";

export interface CustomHeaderProps {
  title?: string;
  subtitle?: string;
  statusColor?: "cyan" | "green" | "yellow" | "red" | "blue";
  isDarkMode: boolean;
  onToggleTheme: () => void;
  onMinimizeToTray?: () => void;
  onMinimize?: () => void;
  onClose?: () => void;
}

export const CustomHeader: React.FC<CustomHeaderProps> = ({
  title = "Codex Tool",
  subtitle = "Bekliyor",
  statusColor = "cyan",
  isDarkMode,
  onToggleTheme,
  onMinimizeToTray,
  onMinimize,
  onClose,
}) => {
  const ledColorClass = {
    cyan: "bg-sky-400 shadow-[0_0_10px_rgba(56,189,248,0.7)]",
    green: "bg-emerald-400 shadow-[0_0_10px_rgba(52,211,153,0.7)]",
    yellow: "bg-amber-400 shadow-[0_0_10px_rgba(251,191,36,0.7)]",
    red: "bg-rose-500 shadow-[0_0_10px_rgba(244,63,94,0.7)]",
    blue: "bg-blue-500 shadow-[0_0_10px_rgba(59,130,246,0.7)]",
  }[statusColor];

  return (
    <header
      data-tauri-drag-region
      className="flex items-center justify-between px-4 py-3 select-none border-b border-black/[0.06] dark:border-white/[0.06] bg-surface-light/95 dark:bg-surface-dark/95 backdrop-blur-md transition-colors"
    >
      {/* Sol Başlık & Canlı LED */}
      <div className="flex items-center gap-3" data-tauri-drag-region>
        <span
          className={`w-2.5 h-2.5 rounded-full ${ledColorClass} animate-breathe flex-shrink-0`}
        />
        <div className="flex flex-col text-left" data-tauri-drag-region>
          <span className="text-sm font-semibold tracking-wide text-slate-900 dark:text-slate-100 leading-tight">
            {title}
          </span>
          <span className="text-[11px] font-medium text-sky-600 dark:text-sky-400 leading-tight">
            {subtitle}
          </span>
        </div>
      </div>

      {/* Orta Sürüklenebilir Alan */}
      <div className="flex-1 h-full min-h-[30px]" data-tauri-drag-region />

      {/* Sağ Kontrol Butonları */}
      <div className="flex items-center gap-1">
        {/* Tema Değiştirici */}
        <button
          type="button"
          onClick={onToggleTheme}
          title={isDarkMode ? "Açık Mod" : "Karanlık Mod"}
          className="p-1.5 rounded-lg text-slate-500 dark:text-slate-400 hover:text-slate-900 dark:hover:text-slate-100 hover:bg-black/[0.06] dark:hover:bg-white/[0.06] transition-colors"
        >
          {isDarkMode ? <Sun size={14} /> : <Moon size={14} />}
        </button>

        {/* Tepsiye Küçült (Tray) */}
        {onMinimizeToTray && (
          <button
            type="button"
            onClick={onMinimizeToTray}
            title="Tepsiye Küçült (Arka Planda Çalıştır)"
            className="p-1.5 rounded-lg text-slate-500 dark:text-slate-400 hover:text-slate-900 dark:hover:text-slate-100 hover:bg-black/[0.06] dark:hover:bg-white/[0.06] transition-colors"
          >
            <ArrowDownToDot size={14} />
          </button>
        )}

        {/* Görev Çubuğuna Küçült */}
        {onMinimize && (
          <button
            type="button"
            onClick={onMinimize}
            title="Küçült"
            className="p-1.5 rounded-lg text-slate-500 dark:text-slate-400 hover:text-slate-900 dark:hover:text-slate-100 hover:bg-black/[0.06] dark:hover:bg-white/[0.06] transition-colors"
          >
            <Minus size={14} />
          </button>
        )}

        {/* Pencereyi Kapat */}
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            title="Kapat"
            className="p-1.5 rounded-lg text-slate-500 dark:text-slate-400 hover:text-rose-500 hover:bg-rose-500/10 transition-colors"
          >
            <X size={14} />
          </button>
        )}
      </div>
    </header>
  );
};
