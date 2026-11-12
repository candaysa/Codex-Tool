import React from "react";
import { ToggleSwitch } from "../common/ToggleSwitch";

export interface TaskItem {
  id: string;
  title: string;
  statusBadge: string;
  statusDescription: string;
  autoResume: boolean;
}

export interface TaskCardProps {
  task: TaskItem;
  onToggleAutoResume: (id: string, enabled: boolean) => void;
}

export const TaskCard: React.FC<TaskCardProps> = ({
  task,
  onToggleAutoResume,
}) => {
  return (
    <div className="flex flex-col gap-2 rounded-2xl bg-white dark:bg-surface-card border border-black/[0.08] dark:border-white/[0.08] p-3.5 shadow-sm text-left select-none transition-colors">
      {/* Üst Kısım: Başlık ve Durum Rozeti */}
      <div className="flex items-start justify-between gap-2">
        <h4
          className="text-xs font-semibold text-slate-900 dark:text-slate-100 line-clamp-1 flex-1 leading-snug"
          title={task.title}
        >
          {task.title}
        </h4>
        <span className="shrink-0 text-[10px] font-medium px-2 py-0.5 rounded-full bg-sky-50 dark:bg-sky-950/60 text-sky-600 dark:text-sky-300 border border-sky-500/20">
          {task.statusBadge}
        </span>
      </div>

      {/* Orta Kısım: Durum Açıklaması */}
      <p className="text-[11px] text-slate-500 dark:text-slate-400 font-mono tracking-tight -mt-0.5">
        {task.statusDescription}
      </p>

      {/* Alt Kısım: Otomatik Sürdürme Slider'ı */}
      <div className="flex items-center justify-between pt-1 border-t border-black/[0.04] dark:border-white/[0.04] mt-0.5">
        <span className="text-[11px] text-slate-600 dark:text-slate-300 font-medium">
          Otomatik sürdürme
        </span>
        <ToggleSwitch
          id={`task-toggle-${task.id}`}
          size="sm"
          checked={task.autoResume}
          onChange={(checked) => onToggleAutoResume(task.id, checked)}
        />
      </div>
    </div>
  );
};
