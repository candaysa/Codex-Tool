import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CustomHeader } from "../components/header/CustomHeader";
import { SummaryCard } from "../components/session/SummaryCard";
import { TaskCard, type TaskItem } from "../components/session/TaskCard";
import { FooterStatus } from "../components/footer/FooterStatus";
import type { AccountModel } from "./DashboardModal";

interface BackendGoal {
  thread_id: string;
  goal_id: string;
  objective: string;
  status: string;
  status_badge: string;
  time_until_reset: string;
  auto_resume: boolean;
  tokens_used: number;
  updated_at_ms: number;
}

interface BackendEngineStatus {
  is_watching: boolean;
  is_paused: boolean;
  waiting_count: number;
  recovering_count: number;
  is_available: boolean;
  status_text: string;
}

export interface CompactViewProps {
  isDarkMode: boolean;
  onToggleTheme: () => void;
  onOpenDashboard: (tab?: "accounts" | "settings") => void;
}

const cleanTimeStr = (str?: string): string => {
  if (!str) return "";
  return str
    .replace(/\s*([+-]\d{2}:\d{2}|Z).*$/, "")
    .replace("T", " ")
    .split(".")[0]
    .trim();
};

export const CompactView: React.FC<CompactViewProps> = ({
  isDarkMode,
  onToggleTheme,
  onOpenDashboard,
}) => {
  const [isPaused, setIsPaused] = useState(false);
  const [waitingCount, setWaitingCount] = useState(0);
  const [recoveringCount, setRecoveringCount] = useState(0);
  const [nextCheckTimeStr, setNextCheckTimeStr] = useState("—");
  const [engineAvailable, setEngineAvailable] = useState(false);
  const [engineStatus, setEngineStatus] = useState("Kurtarma durumu okunuyor…");
  const [actionError, setActionError] = useState("");
  const [activeAccountQuotaText, setActiveAccountQuotaText] = useState(
    "Hesap havuzu kotaları okunuyor..."
  );

  const [tasks, setTasks] = useState<TaskItem[]>([]);

  // Fetch status, tasks, and accounts from Tauri backend
  useEffect(() => {
    let mounted = true;

    const fetchBackendData = async () => {
      try {
        const [status, goals, accounts] = await Promise.all([
          invoke<BackendEngineStatus>("get_engine_status"),
          invoke<BackendGoal[]>("list_thread_goals"),
          invoke<AccountModel[]>("list_accounts"),
        ]);
        if (mounted && status) {
          setWaitingCount(status.waiting_count);
          setRecoveringCount(status.recovering_count);
          setIsPaused(status.is_paused);
          setEngineAvailable(status.is_available);
          setEngineStatus(status.status_text);
        }

        if (mounted) {
          const distinctThreads = goals.filter((goal, index) =>
            goals.findIndex((candidate) => candidate.thread_id === goal.thread_id) === index
          );
          const mappedTasks: TaskItem[] = distinctThreads.slice(0, 3).map((g) => ({
            id: g.thread_id,
            title:
              g.objective.length > 34
                ? `${g.objective.slice(0, 34)} (...`
                : g.objective,
            statusBadge: g.status_badge || "Kullanım sınırı",
            statusDescription: g.time_until_reset
              ? `Beklenen zaman · ${cleanTimeStr(g.time_until_reset)}`
              : "Beklenen zaman bilinmiyor",
            autoResume: g.auto_resume,
          }));
          setTasks(mappedTasks);
          const next = goals.map((goal) => goal.time_until_reset).filter(Boolean).sort()[0];
          setNextCheckTimeStr(cleanTimeStr(next) || "—");
        }

        if (mounted && accounts && accounts.length > 0) {
          const active = accounts.find((a) => a.is_active) || accounts[0];
          const has5h = Boolean(active.tier1_5h);
          const hasWeekly = Boolean(active.tier2_weekly);
          const t1 = active.tier1_5h?.used_percent ?? 0;
          const t2 = active.tier2_weekly?.used_percent ?? 0;
          const resetStr =
            active.tier1_5h?.reset_time_str ||
            active.tier2_weekly?.reset_time_str ||
            "Hazır";
          const emailDisplay = active.email
            ? active.email.length > 20
              ? `${active.email.slice(0, 18)}...`
              : active.email
            : `Hesap #${active.index + 1}`;

          if (has5h && hasWeekly) {
            setActiveAccountQuotaText(
              `Codex (${emailDisplay}): 5 sa %${t1}, haftalık %${t2} • ${resetStr}`
            );
          } else if (hasWeekly) {
            setActiveAccountQuotaText(
              `Codex (${emailDisplay}): Haftalık %${t2} • ${active.tier2_weekly?.reset_time_str || resetStr}`
            );
          } else if (has5h) {
            setActiveAccountQuotaText(
              `Codex (${emailDisplay}): 5 sa %${t1} • ${active.tier1_5h?.reset_time_str || resetStr}`
            );
          } else {
            setActiveAccountQuotaText(`Codex (${emailDisplay}): Kota bilgisi yok`);
          }
        } else if (mounted) {
          setActiveAccountQuotaText("Hesap havuzu boş.");
        }
      } catch {
        // Fallback for standalone preview
      }
    };

    fetchBackendData();
    const interval = setInterval(fetchBackendData, 5000);

    return () => {
      mounted = false;
      clearInterval(interval);
    };
  }, []);

  const handleTogglePause = async () => {
    if (!engineAvailable) {
      onOpenDashboard("settings");
      return;
    }
    const nextPaused = !isPaused;
    try {
      setActionError("");
      setIsPaused(await invoke<boolean>("set_engine_paused", { paused: nextPaused }));
    } catch (error) {
      setActionError(String(error));
    }
  };

  const handleToggleAutoResume = async (taskId: string, enabled: boolean) => {
    try {
      setActionError("");
      const active = await invoke<boolean>("toggle_auto_resume", { taskId, enabled });
      setTasks((prev) => prev.map((t) => t.id === taskId ? { ...t, autoResume: active } : t));
    } catch (error) {
      setActionError(String(error));
    }
  };

  const handleMinimizeToTray = async () => {
    try {
      await getCurrentWindow().hide();
    } catch {}
  };

  const handleMinimize = async () => {
    try {
      await getCurrentWindow().minimize();
    } catch {}
  };

  const handleClose = async () => {
    try {
      await getCurrentWindow().hide();
    } catch {}
  };

  return (
    <div className="flex flex-col h-full w-full bg-surface-light dark:bg-surface-dark text-slate-900 dark:text-slate-100 overflow-hidden rounded-2xl border border-black/[0.08] dark:border-white/[0.08] shadow-2xl transition-colors">
      {/* 1. Özel Başlık Çubuğu */}
      <CustomHeader
        title="Codex Tool"
        subtitle={engineStatus}
        statusColor={isPaused ? "yellow" : "cyan"}
        isDarkMode={isDarkMode}
        onToggleTheme={onToggleTheme}
        onMinimizeToTray={handleMinimizeToTray}
        onMinimize={handleMinimize}
        onClose={handleClose}
      />

      {/* 2. Ana Dikey Gövde */}
      <main className="flex-1 flex flex-col gap-3 p-4 overflow-y-auto">
        {/* 3'lü Sayaç Kartı */}
        <SummaryCard
          waitingCount={waitingCount}
          recoveringCount={recoveringCount}
          nextCheckTimeStr={nextCheckTimeStr}
        />

        {/* Görev Kartları Listesi */}
        <div className="flex flex-col gap-2.5 mt-1">
          {tasks.length === 0 ? (
            <div className="py-6 px-3 rounded-xl bg-black/[0.02] dark:bg-white/[0.02] border border-black/[0.04] dark:border-white/[0.04] text-center">
              <span className="text-xs text-slate-500 dark:text-slate-400">
                {engineAvailable ? "Bekleyen kurtarma yok." : "Kurtarma motoru henüz hazır değil. Ayrıntılar için Terminal’e bakın."}
              </span>
            </div>
          ) : (
            tasks.map((task) => (
              <TaskCard
                key={task.id}
                task={task}
                onToggleAutoResume={handleToggleAutoResume}
              />
            ))
          )}
        </div>
      </main>

      {/* 3. Alt Durum ve Aksiyon Çubuğu */}
      <div className="p-4 pt-0">
        {actionError && <p className="text-xs text-rose-500" role="alert">{actionError}</p>}
        <FooterStatus
          hiddenTasksCount={Math.max(0, waitingCount - tasks.length)}
          quotaSummaryText={activeAccountQuotaText}
          isPaused={isPaused}
          engineAvailable={engineAvailable}
          onTogglePause={handleTogglePause}
          onOpenDashboard={() => onOpenDashboard("accounts")}
        />
      </div>
    </div>
  );
};
