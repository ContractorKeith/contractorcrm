import { invoke } from "@tauri-apps/api/core";

// Report from the Rust core's `health` command — proves the UI → Rust seam works.
export interface HealthReport {
  app: string;
  version: string;
  status: string;
}

// Seam for talking to the Rust core; tests inject a fake, the app uses Tauri.
export interface CoreClient {
  health(): Promise<HealthReport>;
}

export const tauriCoreClient: CoreClient = {
  health: () => invoke<HealthReport>("health"),
};
