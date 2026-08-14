import { useEffect, useState } from "react";

import { tauriCoreClient, type CoreClient } from "./api/client";
import type { HealthReport } from "./api/types";
import { BrandMark } from "./components/BrandMark";
import { loadThemePreference, watchTheme, type ThemePreference } from "./theme";
import { CompaniesView, CompanyDetailView, CompanyFormView } from "./views/companies";
import { ContactDetailView, ContactFormView, ContactsView } from "./views/contacts";

interface AppProps {
  client?: CoreClient;
}

// Plain view state instead of a router — one shell window, a handful of views.
type View =
  | { name: "contacts" }
  | { name: "companies" }
  | { name: "contactDetail"; id: string }
  | { name: "companyDetail"; id: string }
  | { name: "contactForm"; id?: string }
  | { name: "companyForm"; id?: string };

// Shell window: themed chrome, theme control, core health status, and the
// contact/company workspace backed by the Rust core.
export function App({ client = tauriCoreClient }: AppProps) {
  const [theme, setTheme] = useState<ThemePreference>(loadThemePreference);
  const [health, setHealth] = useState<HealthReport | null>(null);
  const [view, setView] = useState<View>({ name: "contacts" });

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
      .then((report: HealthReport) => {
        if (active) setHealth(report);
      })
      .catch(() => {
        if (active) setHealth(null);
      });

    return () => {
      active = false;
    };
  }, [client]);

  // Which top-level tab the current view belongs to.
  const section = view.name.startsWith("company") || view.name === "companies"
    ? "companies"
    : "contacts";

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
        <nav className="view-tabs" aria-label="Records">
          <button
            type="button"
            aria-pressed={section === "contacts"}
            onClick={() => setView({ name: "contacts" })}
          >
            Contacts
          </button>
          <button
            type="button"
            aria-pressed={section === "companies"}
            onClick={() => setView({ name: "companies" })}
          >
            Companies
          </button>
        </nav>

        {view.name === "contacts" ? (
          <ContactsView
            client={client}
            onOpen={(id) => setView({ name: "contactDetail", id })}
            onCreate={() => setView({ name: "contactForm" })}
          />
        ) : null}

        {view.name === "contactDetail" ? (
          <ContactDetailView
            client={client}
            contactId={view.id}
            onBack={() => setView({ name: "contacts" })}
            onEdit={() => setView({ name: "contactForm", id: view.id })}
          />
        ) : null}

        {view.name === "contactForm" ? (
          <ContactFormView
            client={client}
            {...(view.id ? { contactId: view.id } : {})}
            onSaved={(contact) => setView({ name: "contactDetail", id: contact.id })}
            onCancel={() =>
              setView(view.id ? { name: "contactDetail", id: view.id } : { name: "contacts" })
            }
          />
        ) : null}

        {view.name === "companies" ? (
          <CompaniesView
            client={client}
            onOpen={(id) => setView({ name: "companyDetail", id })}
            onCreate={() => setView({ name: "companyForm" })}
          />
        ) : null}

        {view.name === "companyDetail" ? (
          <CompanyDetailView
            client={client}
            companyId={view.id}
            onBack={() => setView({ name: "companies" })}
            onEdit={() => setView({ name: "companyForm", id: view.id })}
          />
        ) : null}

        {view.name === "companyForm" ? (
          <CompanyFormView
            client={client}
            {...(view.id ? { companyId: view.id } : {})}
            onSaved={(company) => setView({ name: "companyDetail", id: company.id })}
            onCancel={() =>
              setView(view.id ? { name: "companyDetail", id: view.id } : { name: "companies" })
            }
          />
        ) : null}
      </main>
    </div>
  );
}
