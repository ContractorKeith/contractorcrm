import { useEffect, useState } from "react";

import { tauriCoreClient, type CoreClient, type HealthReport } from "./api/health";
import { BrandMark } from "./components/BrandMark";
import { loadThemePreference, watchTheme, type ThemePreference } from "./theme";

interface AppProps {
  client?: CoreClient;
}

// Shell window: themed chrome, theme control, core health status, and an
// empty-state workspace. CRM surfaces land here in later milestones.
export function App({ client = tauriCoreClient }: AppProps) {
  const [theme, setTheme] = useState<ThemePreference>(loadThemePreference);
  const [health, setHealth] = useState<HealthReport | null>(null);

  useEffect(
    () =>
      watchTheme(theme, (resolvedTheme) => {
        document.documentElement.dataset.theme = resolvedTheme;
      }),
    [theme],
  );

  // Ping the Rust core once on mount — proves the UI → Rust seam works.
  useEffect(() => {
    let active = true;
    client
      .health()
      .then((report) => {
        if (active) setHealth(report);
      })
      .catch(() => {
        if (active) setHealth(null);
      });

    return () => {
      active = false;
    };
  }, [client]);

  return (
    <div className="app-shell">
      <header className="app-header">
        <a className="brand" href="#main" aria-label="ContractorCRM home">
          <BrandMark />
          <span className="brand__name">
            Contractor<span>CRM</span>
          </span>
        </a>
        <div className="header-controls">
          <label className="theme-control">
            <span>Theme</span>
            <select
              aria-label="Theme"
              value={theme}
              onChange={(event) => setTheme(event.target.value as ThemePreference)}
            >
              <option value="system">System</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </label>
          <div className="storage-state" aria-label="Local storage status">
            <span className="storage-state__dot" />
            {health ? `Core ready · v${health.version}` : "Local SQLite · on this device"}
          </div>
        </div>
      </header>

      <main id="main" className="workspace">
        <section className="workspace-heading" aria-labelledby="crm-heading">
          <div>
            <p className="eyebrow">Contacts &amp; pipeline</p>
            <h1 id="crm-heading">Your book of business, on your machine.</h1>
            <p className="lede">
              Leads, clients, subs, and vendors — with the calls, visits, and quotes behind them —
              stay local unless you choose to export them.
            </p>
          </div>
        </section>

        <section className="crm-section" aria-label="Contacts">
          <div className="section-rule">
            <h2>Local contacts</h2>
            <span>0</span>
          </div>

          <div className="empty-state">
            <span className="registration-mark" aria-hidden="true" />
            <p className="eyebrow">Ready when you are</p>
            <h2>No contacts yet</h2>
            <p>
              Contacts, companies, and the pipeline arrive in the next milestone. Everything will
              be stored in this app&apos;s local database.
            </p>
          </div>
        </section>
      </main>
    </div>
  );
}
