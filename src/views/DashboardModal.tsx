import React, { useState, useEffect, useRef } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
  X,
  Download,
  Upload,
  FolderOpen,
  ArrowLeft,
  CheckCircle2,
  AlertCircle,
  Users,
  Settings,
  Plus,
  RefreshCw,
  Trash2,
  Check,
  Clock,
  Sparkles,
  Copy,
  Terminal,
} from "lucide-react";
import { ToggleSwitch } from "../components/common/ToggleSwitch";

export interface AccountQuotaWindow {
  used_percent: number;
  window_minutes: number;
  reset_at_ms?: number;
  is_exhausted: boolean;
  reset_time_str: string;
}


export interface TerminalLogEntry {
  id: number;
  timestamp: string;
  level: string;
  message: string;
}
export interface AccountModel {
  index: number;
  id: string;
  email: string;
  label: string;
  plan_type: string;
  is_active: boolean;
  tier1_5h?: AccountQuotaWindow;
  tier2_weekly?: AccountQuotaWindow;
  status: string;
  last_used?: number;
}

export interface DashboardModalProps {
  isOpen: boolean;
  onClose: () => void;
  initialTab?: "accounts" | "settings" | "terminal";
}

export const DashboardModal: React.FC<DashboardModalProps> = ({
  isOpen,
  onClose,
  initialTab = "accounts",
}) => {
  const [activeTab, setActiveTab] = useState<"accounts" | "settings" | "terminal">(initialTab);

  // Accounts state
  const [accounts, setAccounts] = useState<AccountModel[]>([]);
  const [isLoadingAccounts, setIsLoadingAccounts] = useState(false);
  const [isRefreshingQuota, setIsRefreshingQuota] = useState(false);
  const [isAddingAccount, setIsAddingAccount] = useState(false);
  const [loginUrl, setLoginUrl] = useState("");
  const [deleteConfirmIndex, setDeleteConfirmIndex] = useState<number | null>(null);

  // Settings state (Tümü slider buton ile kontrol edilir!)
  const [continueMessage, setContinueMessage] = useState("");
  const [messageReady, setMessageReady] = useState(false);
  const [messageError, setMessageError] = useState("");
  const [savingMessage, setSavingMessage] = useState(false);
  const [isUpdatingProxy, setIsUpdatingProxy] = useState(false);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [startInTrayEnabled, setStartInTrayEnabled] = useState(true);
  const [closeToTrayEnabled, setCloseToTrayEnabled] = useState(true);
  const [toastNotificationsEnabled, setToastNotificationsEnabled] = useState(true);
  const [selectedLanguage, setSelectedLanguage] = useState<"tr" | "en">("tr");
  const [terminalLogs, setTerminalLogs] = useState<TerminalLogEntry[]>([]);
  const [autoScroll, setAutoScroll] = useState<boolean>(true);
  const logsEndRef = useRef<HTMLDivElement | null>(null);

  const fetchLogs = async () => {
    try {
      const logs = await invoke<TerminalLogEntry[]>("get_terminal_logs");
      setTerminalLogs(logs);
    } catch {
      // Ignore
    }
  };

  const handleClearLogs = async () => {
    try {
      await invoke("clear_terminal_logs");
      setTerminalLogs([]);
      showNotification("Terminal logları temizlendi.", "info");
    } catch (e: unknown) {
      showNotification(String(e), "error");
    }
  };

  const handleCopyLogs = () => {
    if (terminalLogs.length === 0) return;
    const text = terminalLogs.map((l) => `[${l.timestamp}] [${l.level}] ${l.message}`).join("\n");
    navigator.clipboard.writeText(text);
    showNotification("Tüm loglar panoya kopyalandı.", "success");
  };

  useEffect(() => {
    let timer: ReturnType<typeof setInterval> | null = null;
    if (isOpen) {
      fetchLogs();
      timer = setInterval(fetchLogs, 1500);
    }
    return () => {
      if (timer) clearInterval(timer);
    };
  }, [isOpen, activeTab]);

  useEffect(() => {
    if (autoScroll && logsEndRef.current) {
      logsEndRef.current.scrollIntoView({ behavior: "smooth" });
    }
  }, [terminalLogs, autoScroll]);


  const [statusMessage, setStatusMessage] = useState<{
    text: string;
    type: "success" | "info" | "error";
  } | null>(null);

  const notificationTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const loginUrlRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    return () => {
      if (notificationTimerRef.current) {
        clearTimeout(notificationTimerRef.current);
      }
    };
  }, []);

  const showNotification = (
    text: string,
    type: "success" | "info" | "error" = "success"
  ) => {
    if (notificationTimerRef.current) {
      clearTimeout(notificationTimerRef.current);
    }
    setStatusMessage({ text, type });
    notificationTimerRef.current = setTimeout(() => {
      setStatusMessage(null);
      notificationTimerRef.current = null;
    }, 4000);
  };

  const loadAccounts = async () => {
    setIsLoadingAccounts(true);
    try {
      const data = await invoke<AccountModel[]>("list_accounts");
      setAccounts(data);
    } catch {
      // Browser fallback
    } finally {
      setIsLoadingAccounts(false);
    }
  };

  useEffect(() => {
    if (isOpen) {
      setActiveTab(initialTab);
      loadAccounts();

      const loadSettings = async () => {
        try {
          const { isEnabled } = await import("@tauri-apps/plugin-autostart");
          const enabled = await isEnabled();
          setAutostartEnabled(enabled);
        } catch {}
      };

      loadSettings();
    }
  }, [isOpen, initialTab]);

  const handleSwitchAccount = async (targetIndex: number) => {
    try {
      const msg = await invoke<string>("switch_account", { targetIndex });
      showNotification(msg, "success");
      await loadAccounts();
    } catch (e: unknown) {
      showNotification(`Geçiş hatası: ${e instanceof Error ? e.message : String(e)}`, "error");
    }
  };

  const handleRemoveAccount = async (targetIndex: number) => {
    try {
      const msg = await invoke<string>("remove_account", { targetIndex });
      showNotification(msg, "info");
      setDeleteConfirmIndex(null);
      await loadAccounts();
    } catch (e: unknown) {
      showNotification(`Silme hatası: ${e instanceof Error ? e.message : String(e)}`, "error");
    }
  };

  const handleAddAccount = async () => {
    setIsAddingAccount(true);
    const onUrl = new Channel<string>();
    onUrl.onmessage = setLoginUrl;
    try {
      const msg = await invoke<string>("add_account_login", { onUrl });
      showNotification(msg, "info");
    } catch (e: unknown) {
      showNotification(String(e), "error");
    } finally {
      setLoginUrl("");
      setIsAddingAccount(false);
      await loadAccounts();
    }
  };

  const handleCopyLoginUrl = async () => {
    try {
      await navigator.clipboard.writeText(loginUrl);
      showNotification("Giriş bağlantısı kopyalandı.");
    } catch {
      loginUrlRef.current?.select();
      if (document.execCommand("copy")) {
        showNotification("Giriş bağlantısı kopyalandı.");
      } else {
        showNotification("Bağlantı seçildi. Kopyalamak için Ctrl+C kullanın.", "info");
      }
    }
  };

  const handleRefreshQuota = async () => {
    setIsRefreshingQuota(true);
    try {
      const msg = await invoke<string>("refresh_accounts_quota");
      showNotification(msg, "success");
    } catch (e: unknown) {
      showNotification(String(e), "error");
    } finally {
      await loadAccounts();
      setIsRefreshingQuota(false);
    }
  };

  useEffect(() => {
    if (!isOpen || activeTab !== "settings") return;
    let current = true;
    setMessageReady(false);
    setMessageError("");
    invoke<string>("continuation_message", { message: null })
      .then((value) => {
        if (current) {
          setContinueMessage(value);
          setMessageReady(true);
        }
      })
      .catch((error) => { if (current) setMessageError(String(error)); });
    return () => { current = false; };
  }, [isOpen, activeTab]);

  const handleSaveMessage = async () => {
    setSavingMessage(true);
    try {
      const saved = await invoke<string>("continuation_message", { message: continueMessage });
      setContinueMessage(saved);
      showNotification("Devam mesajı kaydedildi.");
    } catch (error) {
      showNotification(String(error), "error");
    } finally {
      setSavingMessage(false);
    }
  };

  const handleResetCodex = async () => {
    setIsUpdatingProxy(true);
    try {
      showNotification(await invoke<string>("reset_codex_integration"));
    } catch (error) {
      showNotification(String(error), "error");
    } finally {
      setIsUpdatingProxy(false);
    }
  };

  const handleEnableCodex = async (keepExistingChats: boolean) => {
    setIsUpdatingProxy(true);
    try {
      showNotification(await invoke<string>("enable_codex_integration", { keepExistingChats }));
    } catch (error) {
      showNotification(String(error), "error");
    } finally {
      setIsUpdatingProxy(false);
    }
  };

  const handleToggleAutostart = async (enabled: boolean) => {
    setAutostartEnabled(enabled);
    try {
      const { enable, disable } = await import("@tauri-apps/plugin-autostart");
      if (enabled) {
        await enable();
        showNotification("Başlangıçta çalıştırma aktif edildi.", "success");
      } else {
        await disable();
        showNotification("Başlangıçta çalıştırma kapatıldı.", "info");
      }
    } catch {
      showNotification("Sistem başlangıcı ayarlandı", "info");
    }
  };

  const handleExportBackup = async () => {
    try {
      const msg = await invoke<string>("export_backup");
      showNotification(msg, "success");
    } catch (e: unknown) {
      showNotification(`Yedek alma hatası: ${e instanceof Error ? e.message : String(e)}`, "error");
    }
  };

  const handleFileSelected = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    try {
      const text = await file.text();
      const msg = await invoke<string>("import_backup", { backupJson: text });
      showNotification(msg, "success");
      await loadAccounts();
    } catch (err: unknown) {
      showNotification(
        `Yedek yükleme hatası: ${err instanceof Error ? err.message : String(err)}`,
        "error"
      );
    } finally {
      e.target.value = "";
    }
  };

  const handleOpenFolder = async () => {
    try {
      await invoke("open_backups_folder");
    } catch {
      showNotification("Yedek klasörü programımızın backups/ dizininde yer almaktadır.", "info");
    }
  };

  if (!isOpen) return null;

  return (
    <div className="absolute inset-0 z-50 flex flex-col bg-surface-light dark:bg-surface-dark text-slate-900 dark:text-slate-100 rounded-2xl overflow-hidden select-none animate-in fade-in zoom-in-95 duration-150 transition-colors shadow-2xl">
      {/* 1. Üst Başlık & Sekmeler */}
      <header className="flex flex-col border-b border-black/[0.08] dark:border-white/[0.08] bg-slate-100/90 dark:bg-surface-card/80 backdrop-blur-md">
        <div className="flex items-center justify-between px-4 py-3">
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={onClose}
              className="p-1.5 rounded-lg text-slate-500 dark:text-slate-400 hover:text-slate-900 dark:hover:text-slate-100 hover:bg-black/[0.06] dark:hover:bg-white/[0.06] transition-colors"
            >
              <ArrowLeft size={16} />
            </button>
            <div className="flex flex-col">
              <span className="text-sm font-bold text-slate-900 dark:text-slate-100 leading-tight">
                Codex Tool • Gösterge Paneli
              </span>
              <span className="text-[10px] text-slate-500 dark:text-slate-400">
                Çoklu Hesap Havuzu & Kota İzleyici
              </span>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="p-1.5 rounded-lg text-slate-500 dark:text-slate-400 hover:text-rose-500 hover:bg-rose-500/10 transition-colors"
          >
            <X size={16} />
          </button>
        </div>

        {/* Tab Seçici */}
        <div className="flex px-4 gap-2 pb-2">
          <button
            type="button"
            onClick={() => setActiveTab("accounts")}
            className={`flex-1 flex items-center justify-center gap-1.5 py-1.5 text-xs font-semibold rounded-lg transition-all ${
              activeTab === "accounts"
                ? "bg-sky-500 text-white shadow-sm"
                : "text-slate-600 dark:text-slate-400 hover:bg-black/[0.05] dark:hover:bg-white/[0.05]"
            }`}
          >
            <Users size={14} />
            Hesaplar & Kotalar ({accounts.length})
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("terminal")}
            className={`flex-1 flex items-center justify-center gap-1.5 py-1.5 text-xs font-semibold rounded-lg transition-all ${
              activeTab === "terminal"
                ? "bg-sky-500 text-white shadow-sm"
                : "text-slate-600 dark:text-slate-400 hover:bg-black/[0.05] dark:hover:bg-white/[0.05]"
            }`}
          >
            <Terminal size={14} />
            Canlı Terminal
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("settings")}
            className={`flex-1 flex items-center justify-center gap-1.5 py-1.5 text-xs font-semibold rounded-lg transition-all ${
              activeTab === "settings"
                ? "bg-sky-500 text-white shadow-sm"
                : "text-slate-600 dark:text-slate-400 hover:bg-black/[0.05] dark:hover:bg-white/[0.05]"
            }`}
          >
            <Settings size={14} />
            Ayarlar & Yedek
          </button>
        </div>
      </header>

      {/* Durum Bildirim Balonu */}
      {statusMessage && (
        <div
          className={`mx-4 mt-3 p-2.5 rounded-xl text-xs flex items-center gap-2 transition-all ${
            statusMessage.type === "success"
              ? "bg-emerald-500/15 border border-emerald-500/30 text-emerald-600 dark:text-emerald-300"
              : statusMessage.type === "error"
              ? "bg-rose-500/15 border border-rose-500/30 text-rose-600 dark:text-rose-300"
              : "bg-sky-500/15 border border-sky-500/30 text-sky-600 dark:text-sky-300"
          }`}
        >
          {statusMessage.type === "success" ? (
            <CheckCircle2 size={14} className="shrink-0" />
          ) : (
            <AlertCircle size={14} className="shrink-0" />
          )}
          <span>{statusMessage.text}</span>
        </div>
      )}

      {/* 2. Ana Gövde */}
      <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-3 text-left">
        {/* ========================================================= */}
        {/* SEKME 1: HESAPLAR & KOTA YÖNETİMİ                        */}
        {/* ========================================================= */}
        {activeTab === "accounts" && (
          <div className="flex flex-col gap-3">
            {/* Üst İşlem Butonları */}
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-semibold text-slate-700 dark:text-slate-300">
                Aktif Hesap Havuzu
              </span>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={handleRefreshQuota}
                  disabled={isRefreshingQuota}
                  className="flex items-center gap-1 py-1.5 px-2.5 rounded-lg bg-slate-200/80 dark:bg-slate-800 text-slate-800 dark:text-slate-200 text-xs font-medium hover:bg-slate-300 dark:hover:bg-slate-700 transition-colors"
                  title="Tüm hesapların kotalarını yenile"
                >
                  <RefreshCw
                    size={12}
                    className={isRefreshingQuota ? "animate-spin" : ""}
                  />
                  Yenile
                </button>
                <button
                  type="button"
                  onClick={handleAddAccount}
                  disabled={isAddingAccount}
                  className="flex items-center gap-1 py-1.5 px-3 rounded-lg bg-sky-500 hover:bg-sky-600 text-white text-xs font-semibold shadow-sm transition-colors"
                >
                  <Plus size={13} />
                  {isAddingAccount ? "Giriş bekleniyor…" : "Yeni Hesap Ekle (OAuth)"}
                </button>
              </div>
            </div>

            {loginUrl && (
              <div className="flex gap-2 text-xs">
                <input
                  ref={loginUrlRef}
                  aria-label="Giriş bağlantısı"
                  className="min-w-0 flex-1 rounded-lg bg-slate-100 dark:bg-slate-800 px-2 select-text"
                  value={loginUrl}
                  readOnly
                />
                <button
                  type="button"
                  onClick={handleCopyLoginUrl}
                  className="flex items-center gap-1 rounded-lg bg-sky-600 text-white px-2 py-1.5"
                >
                  <Copy size={12} /> Bağlantıyı Kopyala
                </button>
              </div>
            )}

            {/* Hesap Kartları */}
            {isLoadingAccounts ? (
              <div className="py-12 flex flex-col items-center justify-center gap-2 text-slate-400">
                <RefreshCw size={24} className="animate-spin text-sky-400" />
                <span className="text-xs">Hesaplar yükleniyor...</span>
              </div>
            ) : accounts.length === 0 ? (
              <div className="py-12 px-4 rounded-2xl bg-white dark:bg-surface-card border border-black/[0.08] dark:border-white/[0.08] flex flex-col items-center justify-center text-center gap-3">
                <Users size={32} className="text-slate-400" />
                <span className="text-xs text-slate-600 dark:text-slate-300">
                  Henüz kayıtlı bir hesap bulunamadı.
                </span>
                <button
                  type="button"
                  onClick={handleAddAccount}
                  disabled={isAddingAccount}
                  className="py-1.5 px-4 rounded-xl bg-sky-500 text-white text-xs font-semibold hover:bg-sky-600 transition-colors"
                >
                  + OpenAI Hesabı ile Giriş Yap
                </button>
              </div>
            ) : (
              accounts.map((acc) => {
                const tier1 = acc.tier1_5h;
                const tier2 = acc.tier2_weekly;
                const isConfirmingDelete = deleteConfirmIndex === acc.index;

                return (
                  <div
                    key={JSON.stringify([acc.id, acc.email, acc.index])}
                    className={`rounded-2xl p-3.5 flex flex-col gap-2.5 transition-all border ${
                      acc.is_active
                        ? "bg-sky-50/80 dark:bg-sky-950/20 border-sky-500/40 shadow-sm"
                        : "bg-white dark:bg-surface-card border-black/[0.08] dark:border-white/[0.08]"
                    }`}
                  >
                    {/* Kart Üst Başlık: Email + Rozetler + Aksiyon */}
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex items-center gap-2 min-w-0">
                        <span
                          className={`w-2 h-2 rounded-full shrink-0 ${
                            acc.is_active
                              ? "bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,0.8)]"
                              : "bg-slate-400 dark:bg-slate-600"
                          }`}
                        />
                        <div className="flex flex-col min-w-0">
                          <div className="flex items-center gap-1.5">
                            <span className="text-xs font-bold truncate text-slate-900 dark:text-slate-100">
                              {acc.email || acc.label || `Hesap ${acc.index + 1}`}
                            </span>
                            <span className="text-[10px] uppercase font-bold px-1.5 py-0.5 rounded bg-sky-500/10 text-sky-600 dark:text-sky-400 border border-sky-500/20">
                              {acc.plan_type}
                            </span>
                          </div>
                          <span className="text-[10px] text-slate-500 dark:text-slate-400 truncate">
                            {acc.label || `Hesap #${acc.index + 1}`}
                          </span>
                        </div>
                      </div>

                      <div className="flex items-center gap-1.5 shrink-0">
                        {acc.is_active ? (
                          <span className="flex items-center gap-1 text-[11px] font-semibold text-emerald-600 dark:text-emerald-400 bg-emerald-500/10 px-2 py-1 rounded-lg border border-emerald-500/20">
                            <Check size={12} />
                            Aktif Hesap
                          </span>
                        ) : (
                          <button
                            type="button"
                            onClick={() => handleSwitchAccount(acc.index)}
                            className="text-[11px] font-medium py-1 px-2.5 rounded-lg bg-slate-100 dark:bg-slate-800 text-slate-800 dark:text-slate-200 hover:bg-sky-500 hover:text-white transition-colors border border-black/[0.08] dark:border-white/[0.08]"
                          >
                            Aktif Yap
                          </button>
                        )}

                        {/* Sil Butonu */}
                        {isConfirmingDelete ? (
                          <div className="flex items-center gap-1">
                            <button
                              type="button"
                              onClick={() => handleRemoveAccount(acc.index)}
                              className="text-[10px] font-bold py-1 px-2 rounded-lg bg-rose-600 text-white hover:bg-rose-700 transition-colors"
                            >
                              Onayla
                            </button>
                            <button
                              type="button"
                              onClick={() => setDeleteConfirmIndex(null)}
                              className="text-[10px] py-1 px-1.5 rounded-lg bg-slate-200 dark:bg-slate-700 text-slate-700 dark:text-slate-300"
                            >
                              İptal
                            </button>
                          </div>
                        ) : (
                          <button
                            type="button"
                            onClick={() => setDeleteConfirmIndex(acc.index)}
                            title="Hesabı Havuzdan Kaldır"
                            className="p-1 rounded-lg text-slate-400 hover:text-rose-500 hover:bg-rose-500/10 transition-colors"
                          >
                            <Trash2 size={13} />
                          </button>
                        )}
                      </div>
                    </div>

                    {/* Kotalar Bölümü (Tier 1 & Tier 2) */}
                    <div className="flex flex-col gap-2 pt-1 border-t border-black/[0.04] dark:border-white/[0.04]">
                      {!tier1 && !tier2 && (
                        <span className="text-xs text-slate-500">{acc.status}</span>
                      )}
                      {/* Tier 1: 5 Saatlik Kota */}
                      {tier1 && (
                        <div className="flex flex-col gap-1">
                          <div className="flex items-center justify-between text-[11px]">
                            <span className="text-slate-600 dark:text-slate-400 font-medium flex items-center gap-1">
                              <Clock size={11} className="text-sky-400" />
                              5 Saatlik Kota (Tier 1)
                            </span>
                            <span className="font-semibold text-slate-800 dark:text-slate-200">
                              %{tier1.used_percent} Kullanıldı
                            </span>
                          </div>
                          {/* İlerleme Çubuğu */}
                          <div className="w-full h-1.5 rounded-full bg-black/[0.06] dark:bg-white/[0.08] overflow-hidden">
                            <div
                              className={`h-full rounded-full transition-all duration-300 ${
                                tier1.used_percent >= 100
                                  ? "bg-rose-500"
                                  : tier1.used_percent >= 80
                                  ? "bg-amber-500"
                                  : "bg-sky-500"
                              }`}
                              style={{ width: `${Math.min(100, tier1.used_percent)}%` }}
                            />
                          </div>
                          <span className="text-[10px] text-slate-500 dark:text-slate-400">
                            {tier1.reset_time_str}
                          </span>
                        </div>
                      )}

                      {/* Tier 2: Haftalık Kota */}
                      {tier2 && (
                        <div className="flex flex-col gap-1 pt-1">
                          <div className="flex items-center justify-between text-[11px]">
                            <span className="text-slate-600 dark:text-slate-400 font-medium flex items-center gap-1">
                              <Sparkles size={11} className="text-indigo-400" />
                              {tier2.window_minutes === 10080 ? "7 Günlük Kota" : `${tier2.window_minutes / 60} Saatlik Kota`}
                            </span>
                            <span className="font-semibold text-slate-800 dark:text-slate-200">
                              %{tier2.used_percent} Kullanıldı
                            </span>
                          </div>
                          {/* İlerleme Çubuğu */}
                          <div className="w-full h-1.5 rounded-full bg-black/[0.06] dark:bg-white/[0.08] overflow-hidden">
                            <div
                              className={`h-full rounded-full transition-all duration-300 ${
                                tier2.used_percent >= 100
                                  ? "bg-rose-500"
                                  : tier2.used_percent >= 80
                                  ? "bg-amber-500"
                                  : "bg-indigo-500"
                              }`}
                              style={{ width: `${Math.min(100, tier2.used_percent)}%` }}
                            />
                          </div>
                          <span className="text-[10px] text-slate-500 dark:text-slate-400">
                            {tier2.reset_time_str === "Kullanıma Hazır" ? "Kullanıma Hazır" : `Sıfırlanma · ${tier2.reset_time_str}`}
                          </span>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })
            )}

            <p className="text-[10px] text-slate-500 dark:text-slate-400 pt-1 text-center">
              Kotalar her hesabın ChatGPT oturumundan okunur. Hesap değiştirmek için Aktif Yap’ı kullanın.
            </p>
          </div>
        )}

        {/* ========================================================= */}
        {/* SEKME 2: CANLI TERMİNAL / PROXY VE MOTOR LOGLARI          */}
        {/* ========================================================= */}
        {activeTab === "terminal" && (
          <div className="flex-1 flex flex-col min-h-[350px] gap-2.5 overflow-hidden">
            {/* Terminal Üst Bilgi Barı */}
            <div className="flex items-center justify-between gap-2 p-2.5 rounded-xl bg-slate-900 text-white border border-slate-800 text-xs shadow-sm">
              <div className="flex items-center gap-2.5 flex-wrap">
                <span className="text-[10px] px-2 py-0.5 rounded bg-slate-800 text-slate-300 font-mono">
                  {terminalLogs.length} satır
                </span>
              </div>

              <div className="flex items-center gap-2">
                <label className="flex items-center gap-1.5 text-[11px] text-slate-300 cursor-pointer select-none">
                  <ToggleSwitch
                    id="setting-autoscroll"
                    checked={autoScroll}
                    onChange={setAutoScroll}
                  />
                  <span>Oto-Kaydır</span>
                </label>
                <button
                  type="button"
                  onClick={fetchLogs}
                  className="p-1 px-2 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-200 text-[11px] flex items-center gap-1 transition-colors"
                  title="Logları Yenile"
                >
                  <RefreshCw size={11} />
                  Yenile
                </button>
                <button
                  type="button"
                  onClick={handleCopyLogs}
                  disabled={terminalLogs.length === 0}
                  className="p-1 px-2 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-200 text-[11px] flex items-center gap-1 transition-colors disabled:opacity-50"
                  title="Panoya Kopyala"
                >
                  <Copy size={11} />
                  Kopyala
                </button>
                <button
                  type="button"
                  onClick={handleClearLogs}
                  disabled={terminalLogs.length === 0}
                  className="p-1 px-2 rounded-lg bg-rose-500/20 hover:bg-rose-500/30 text-rose-300 text-[11px] flex items-center gap-1 transition-colors disabled:opacity-50"
                  title="Logları Temizle"
                >
                  <Trash2 size={11} />
                  Temizle
                </button>
              </div>
            </div>

            {/* Terminal Konsol Ekranı */}
            <div className="flex-1 min-h-[300px] max-h-[420px] bg-[#090d16] border border-slate-800 rounded-xl p-3 overflow-y-auto font-mono text-[11px] flex flex-col gap-1 shadow-inner select-text">
              {terminalLogs.length === 0 ? (
                <div className="py-20 flex flex-col items-center justify-center text-center gap-2 text-slate-500">
                  <Terminal size={32} className="text-slate-600 opacity-60" />
                  <span className="text-xs text-slate-400">Henüz kaydedilmiş log bulunmuyor.</span>
                  <span className="text-[10px] text-slate-600 max-w-sm">
                    Codex Desktop üzerinden istek atıldığında, hesap rotasyonu gerçekleştiğinde veya arka plan kota taraması yapıldığında tüm canlı akış burada görüntülenecektir.
                  </span>
                </div>
              ) : (
                terminalLogs.map((log) => {
                  let badgeColor = "bg-sky-500/20 text-sky-400 border-sky-500/30";
                  if (log.level === "PROXY") badgeColor = "bg-emerald-500/20 text-emerald-400 border-emerald-500/30";
                  if (log.level === "QUOTA") badgeColor = "bg-amber-500/20 text-amber-400 border-amber-500/30";
                  if (log.level === "WARN") badgeColor = "bg-orange-500/20 text-orange-400 border-orange-500/30";
                  if (log.level === "ERROR") badgeColor = "bg-rose-500/20 text-rose-400 border-rose-500/30";
                  if (log.level === "RESUME") badgeColor = "bg-purple-500/20 text-purple-400 border-purple-500/30";

                  return (
                    <div key={log.id} className="flex items-start gap-2 leading-relaxed hover:bg-white/[0.03] px-1 py-0.5 rounded">
                      <span className="text-slate-500 shrink-0 select-none">[{log.timestamp}]</span>
                      <span className={`px-1.5 py-0.5 rounded text-[9px] font-bold uppercase shrink-0 border ${badgeColor}`}>
                        {log.level}
                      </span>
                      <span className="text-slate-200 break-all">{log.message}</span>
                    </div>
                  );
                })
              )}
              <div ref={logsEndRef} />
            </div>
          </div>
        )}

        {/* ========================================================= */}
        {/* SEKME 3: GENEL AYARLAR & ENTEGRASYON                    */}
        {/* ========================================================= */}
        {activeTab === "settings" && (
          <div className="flex flex-col gap-4">
            <section className="rounded-2xl border border-sky-500/20 p-4 flex flex-col gap-2">
              <label htmlFor="continue-message" className="text-xs font-semibold">Otomatik devam mesajı</label>
              <textarea
                id="continue-message"
                value={continueMessage}
                onChange={(event) => setContinueMessage(event.target.value)}
                disabled={!messageReady || savingMessage}
                rows={3}
                className="rounded-lg p-2 text-xs bg-slate-100 dark:bg-slate-800"
                placeholder="Varsayılan mesajı kullanmak için boş bırakın."
              />
              {messageError && <p className="text-xs text-amber-500">{messageError}</p>}
              <button type="button" disabled={!messageReady || savingMessage} onClick={handleSaveMessage}
                className="self-start rounded-lg bg-sky-600 text-white px-3 py-1.5 text-xs disabled:opacity-50">
                Mesajı Kaydet
              </button>
            </section>
            <section className="rounded-2xl border border-slate-500/20 p-4 flex flex-col gap-2">
              <button type="button" onClick={() => handleEnableCodex(false)} disabled={isUpdatingProxy}
                className="self-start rounded-lg bg-sky-600 text-white px-3 py-1.5 text-xs">
                Profili Gizleyerek Bağlan
              </button>
              <p className="text-[11px] text-slate-500">Yeni sohbetlerde hesap bilgilerini gizler.</p>
              <button type="button" onClick={() => handleEnableCodex(true)} disabled={isUpdatingProxy}
                className="self-start rounded-lg bg-slate-200 dark:bg-slate-800 px-3 py-1.5 text-xs">
                Eski OpenAI Sohbetlerini Aç
              </button>
              <p className="text-[11px] text-slate-500">OpenAI sağlayıcısını ve hesap geçişini kullanır. Codex profili görünür.</p>
              <button type="button" onClick={handleResetCodex} disabled={isUpdatingProxy}
                className="self-start rounded-lg bg-slate-200 dark:bg-slate-800 px-3 py-1.5 text-xs">
                Codex’i Fabrika Ayarlarına Döndür
              </button>
              <p className="text-[11px] text-slate-500">Codex Tool bağlantısını kaldırır ve önceki sağlayıcı ayarlarını geri yükler.</p>
            </section>

            {/* 2. Yedekleme & Geri Yükleme */}
            <section className="rounded-2xl bg-white dark:bg-surface-card border border-black/[0.08] dark:border-white/[0.08] p-4 flex flex-col gap-2.5 shadow-sm">
              <span className="text-xs font-semibold text-slate-900 dark:text-slate-100">
                Hesap Havuzu ve Veri Yedekleme
              </span>
              <p className="text-[11px] text-slate-600 dark:text-slate-400 leading-normal">
                Yedekler doğrudan programımızın <code className="text-sky-500 font-semibold">backups/</code> klasörüne kaydedilir. İstediğiniz bir yedeği elle dosya seçerek geri yükleyebilirsiniz.
              </p>
              <input
                type="file"
                ref={fileInputRef}
                accept=".json,.bak"
                className="hidden"
                onChange={handleFileSelected}
              />
              <div className="grid grid-cols-3 gap-2 pt-1">
                <button
                  type="button"
                  onClick={handleExportBackup}
                  className="flex items-center justify-center gap-1.5 py-2 px-2.5 rounded-xl bg-slate-100 dark:bg-slate-800/80 hover:bg-slate-200 dark:hover:bg-slate-700 text-slate-800 dark:text-slate-200 border border-black/[0.08] dark:border-white/[0.08] text-xs font-medium transition-colors"
                >
                  <Download size={13} />
                  Yedek Al
                </button>
                <button
                  type="button"
                  onClick={() => fileInputRef.current?.click()}
                  className="flex items-center justify-center gap-1.5 py-2 px-2.5 rounded-xl bg-sky-50 dark:bg-sky-950/40 hover:bg-sky-100 dark:hover:bg-sky-900/40 text-sky-600 dark:text-sky-400 border border-sky-500/20 text-xs font-medium transition-colors"
                >
                  <Upload size={13} />
                  Elle Seç & Yükle
                </button>
                <button
                  type="button"
                  onClick={handleOpenFolder}
                  className="flex items-center justify-center gap-1.5 py-2 px-2 rounded-xl bg-slate-100 dark:bg-slate-800/80 hover:bg-slate-200 dark:hover:bg-slate-700 text-slate-800 dark:text-slate-200 border border-black/[0.08] dark:border-white/[0.08] text-xs font-medium transition-colors"
                >
                  <FolderOpen size={13} />
                  Klasörü Aç
                </button>
              </div>
            </section>

            {/* 3. Sistem Tercihleri (Tümü Slider Butonlar!) */}
            <section className="rounded-2xl bg-white dark:bg-surface-card border border-black/[0.08] dark:border-white/[0.08] p-4 flex flex-col gap-3 shadow-sm">
              <span className="text-xs font-semibold text-slate-900 dark:text-slate-100">
                Sistem Tercihleri
              </span>

              {/* Autostart */}
              <div className="flex items-center justify-between py-1 border-b border-black/[0.04] dark:border-white/[0.04]">
                <div className="flex flex-col">
                  <span className="text-xs text-slate-800 dark:text-slate-200">
                    Sistem Açılışında Başlat
                  </span>
                  <span className="text-[10px] text-slate-500 dark:text-slate-400">
                    Windows/Linux başladığında otomatik çalıştırır
                  </span>
                </div>
                <ToggleSwitch
                  id="setting-autostart"
                  checked={autostartEnabled}
                  onChange={handleToggleAutostart}
                />
              </div>

              {/* Start in Tray */}
              <div className="flex items-center justify-between py-1 border-b border-black/[0.04] dark:border-white/[0.04]">
                <div className="flex flex-col">
                  <span className="text-xs text-slate-800 dark:text-slate-200">
                    Sistem Tepsisinde Başlat
                  </span>
                  <span className="text-[10px] text-slate-500 dark:text-slate-400">
                    Açılışta pencereyi gizleyip tepsiye yerleşir
                  </span>
                </div>
                <ToggleSwitch
                  id="setting-start-in-tray"
                  checked={startInTrayEnabled}
                  onChange={setStartInTrayEnabled}
                />
              </div>

              {/* Close to Tray */}
              <div className="flex items-center justify-between py-1 border-b border-black/[0.04] dark:border-white/[0.04]">
                <div className="flex flex-col">
                  <span className="text-xs text-slate-800 dark:text-slate-200">
                    Kapatıldığında Tepsiye Küçült
                  </span>
                  <span className="text-[10px] text-slate-500 dark:text-slate-400">
                    X butonuna basıldığında arka planda çalışmaya devam eder
                  </span>
                </div>
                <ToggleSwitch
                  id="setting-close-to-tray"
                  checked={closeToTrayEnabled}
                  onChange={setCloseToTrayEnabled}
                />
              </div>

              {/* Toast Notifications */}
              <div className="flex items-center justify-between py-1 border-b border-black/[0.04] dark:border-white/[0.04]">
                <div className="flex flex-col">
                  <span className="text-xs text-slate-800 dark:text-slate-200">
                    Sistem Bildirimleri
                  </span>
                  <span className="text-[10px] text-slate-500 dark:text-slate-400">
                    Kota dolumu ve otomatik kurtarma bildirimleri
                  </span>
                </div>
                <ToggleSwitch
                  id="setting-toast"
                  checked={toastNotificationsEnabled}
                  onChange={setToastNotificationsEnabled}
                />
              </div>

              {/* Dil Değiştirme (Slider Buton ile!) */}
              <div className="flex items-center justify-between py-1">
                <div className="flex flex-col">
                  <span className="text-xs text-slate-800 dark:text-slate-200">
                    Dil / Language
                  </span>
                  <span className="text-[10px] text-slate-500 dark:text-slate-400">
                    {selectedLanguage === "tr" ? "Türkçe (TR)" : "English (EN)"}
                  </span>
                </div>
                <div className="flex items-center gap-1.5">
                  <span className="text-[10px] font-semibold text-slate-500">TR</span>
                  <ToggleSwitch
                    id="setting-language"
                    checked={selectedLanguage === "en"}
                    onChange={(isEn) => setSelectedLanguage(isEn ? "en" : "tr")}
                  />
                  <span className="text-[10px] font-semibold text-slate-500">EN</span>
                </div>
              </div>
            </section>
          </div>
        )}
      </div>
    </div>
  );
};
