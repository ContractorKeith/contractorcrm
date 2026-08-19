import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  test: {
    environment: "jsdom",
    // Agent worktrees live under .claude/ inside the repo; never run their
    // duplicate copies of the suites.
    exclude: ["**/node_modules/**", "**/dist/**", "**/.claude/**", "**/src-tauri/**"],
    // The UI suites are interaction-heavy; one worker avoids 5s false
    // timeouts when several jsdom environments contend on CI or dev laptops.
    maxWorkers: 1,
    environmentOptions: {
      jsdom: {
        url: "http://localhost/",
      },
    },
    setupFiles: ["./src/test/setup.ts"],
  },
});
