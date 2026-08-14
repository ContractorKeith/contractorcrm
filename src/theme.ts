export type ThemePreference = "system" | "light" | "dark";

const THEME_STORAGE_KEY = "contractorcrm.theme";

// Read the persisted theme preference; anything unexpected falls back to system.
export function loadThemePreference(): ThemePreference {
  const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

// Persist the preference and keep the resolved theme in sync with the OS
// while "system" is selected. Returns a cleanup function.
export function watchTheme(
  preference: ThemePreference,
  onResolved: (theme: "light" | "dark") => void,
): () => void {
  window.localStorage.setItem(THEME_STORAGE_KEY, preference);
  const media = window.matchMedia?.("(prefers-color-scheme: dark)");
  const resolve = () => {
    onResolved(preference === "system" ? (media?.matches ? "dark" : "light") : preference);
  };

  resolve();
  if (preference !== "system" || !media) return () => undefined;

  media.addEventListener("change", resolve);
  return () => media.removeEventListener("change", resolve);
}
