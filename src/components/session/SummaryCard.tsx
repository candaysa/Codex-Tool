import React from "react";

export interface SummaryCardProps {
  waitingCount: number;
  recoveringCount: number;
  nextCheckTimeStr: string;
}

export const SummaryCard: React.FC<SummaryCardProps> = ({
  waitingCount,
  recoveringCount,
  nextCheckTimeStr,
}) => {
  return (
    <div className="grid grid-cols-3 divide-x divide-black/[0.06] dark:divide-white/[0.08] rounded-2xl bg-white dark:bg-surface-card border border-black/[0.08] dark:border-white/[0.08] p-3 text-center shadow-sm select-none transition-colors">
      {/* 1. Bekleyen Görevler */}
      <div className="flex flex-col items-center justify-center px-1">
        <span className="text-[11px] font-medium text-slate-500 dark:text-slate-400">
          Bekliyor
        </span>
        <span className="text-base font-bold text-slate-900 dark:text-slate-100 mt-0.5">
          {waitingCount}
        </span>
      </div>

      {/* 2. Kurtarılan Görevler */}
      <div className="flex flex-col items-center justify-center px-1">
        <span className="text-[11px] font-medium text-slate-500 dark:text-slate-400">
          Kurtarılıyor
        </span>
        <span className="text-base font-bold text-emerald-600 dark:text-emerald-400 mt-0.5">
          {recoveringCount}
        </span>
      </div>

      {/* 3. Sonraki Kontrol Zamanı */}
      <div className="flex flex-col items-center justify-center px-1">
        <span className="text-[11px] font-medium text-slate-500 dark:text-slate-400">
          Beklenen zaman
        </span>
        <span className="text-xs font-semibold text-slate-800 dark:text-slate-200 mt-0.5 tracking-tight font-mono">
          {nextCheckTimeStr}
        </span>
      </div>
    </div>
  );
};
