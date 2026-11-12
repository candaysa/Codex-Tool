import React from "react";

export interface FooterStatusProps {
  hiddenTasksCount: number;
  quotaSummaryText: string;
  isPaused: boolean;
  engineAvailable: boolean;
  onTogglePause: () => void;
  onOpenDashboard: () => void;
}

export const FooterStatus: React.FC<FooterStatusProps> = ({
  hiddenTasksCount,
  quotaSummaryText,
  isPaused,
  engineAvailable,
  onTogglePause,
  onOpenDashboard,
}) => {
  return (
    <footer className="flex flex-col gap-2.5 pt-1 text-center select-none">
      {/* Gizli Görev Sayısı Bildirimi */}
      {hiddenTasksCount > 0 && (
        <span className="text-[11px] font-medium text-slate-500 dark:text-slate-400">
          Gösterge Paneli'nde +{hiddenTasksCount} daha
        </span>
      )}

      {/* Kota ve Kullanım Özeti Metni */}
      <p className="text-[10px] text-slate-500 dark:text-slate-400 leading-tight px-2">
        {quotaSummaryText}
      </p>

      {/* Aksiyon Butonları */}
      <div className="grid grid-cols-2 gap-2 mt-1">
        {/* Duraklat / Sürdür Butonu */}
        <button
          type="button"
          onClick={onTogglePause}
          className="w-full py-2.5 px-3 rounded-xl bg-slate-100 dark:bg-slate-800/80 hover:bg-slate-200 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 text-xs font-semibold border border-black/[0.08] dark:border-white/[0.08] transition-colors shadow-sm"
        >
          {engineAvailable ? (isPaused ? "Kurtarmayı başlat" : "Kurtarmayı duraklat") : "Kurtarma Ayarları"}
        </button>

        {/* Gösterge Paneli'ni Aç Butonu */}
        <button
          type="button"
          onClick={onOpenDashboard}
          className="w-full py-2.5 px-3 rounded-xl bg-sky-500 hover:bg-sky-400 text-white text-xs font-semibold shadow-md shadow-sky-500/20 transition-colors"
        >
          Gösterge Paneli'ni aç
        </button>
      </div>
    </footer>
  );
};
