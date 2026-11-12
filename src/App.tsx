import { useState, useEffect } from "react";
import { CompactView } from "./views/CompactView";
import { DashboardModal } from "./views/DashboardModal";

export function App() {
  const [isDarkMode, setIsDarkMode] = useState(true);
  const [isDashboardOpen, setIsDashboardOpen] = useState(false);
  const [dashboardTab, setDashboardTab] = useState<"accounts" | "settings">("accounts");

  useEffect(() => {
    const root = document.documentElement;
    if (isDarkMode) {
      root.classList.add("dark");
      root.classList.remove("light");
    } else {
      root.classList.remove("dark");
      root.classList.add("light");
    }
  }, [isDarkMode]);

  const handleToggleTheme = () => {
    setIsDarkMode((prev) => !prev);
  };

  const handleOpenDashboard = (tab: "accounts" | "settings" = "accounts") => {
    setDashboardTab(tab);
    setIsDashboardOpen(true);
  };

  return (
    <div className="relative h-full w-full overflow-hidden flex flex-col bg-transparent select-none">
      {/* 1. Dikey Kompakt Görünüm */}
      <CompactView
        isDarkMode={isDarkMode}
        onToggleTheme={handleToggleTheme}
        onOpenDashboard={handleOpenDashboard}
      />

      {/* 2. Gösterge Paneli & Çoklu Hesap / Ayarlar Modalı */}
      <DashboardModal
        isOpen={isDashboardOpen}
        onClose={() => setIsDashboardOpen(false)}
        initialTab={dashboardTab}
      />
    </div>
  );
}

export default App;
