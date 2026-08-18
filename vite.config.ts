import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;
// Android emulator uses 10.0.2.2 as an alias for the host loopback — it is not a local NIC.
const emulatorHost = host === "10.0.2.2";
const origin = host ? `http://${host}:1420` : undefined;

export default defineConfig({
  plugins: [
    react(),
    // Tauri Android serves the page from http://tauri.localhost (JNI proxy). Native
    // ESM then fetches every module through that proxy and never finishes. Point
    // the entry script at the real Vite origin so the graph loads over TCP.
    origin
      ? {
          name: "lumen-android-dev-origin",
          transformIndexHtml(html: string) {
            return html.replace(
              'src="/src/main.tsx"',
              `src="${origin}/src/main.tsx"`,
            );
          },
        }
      : null,
  ].filter(Boolean),
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: emulatorHost ? "0.0.0.0" : host || false,
    origin,
    cors: true,
    // Vite 5.4+ blocks unknown Host headers (would 403 the emulator).
    allowedHosts: host ? [host, "localhost", "tauri.localhost"] : undefined,
    hmr: host
      ? {
          protocol: "ws",
          host,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  optimizeDeps: {
    include: [
      "react",
      "react-dom",
      "react-i18next",
      "i18next",
      "zustand",
      "@tauri-apps/api/core",
      "@tauri-apps/api/window",
    ],
  },
});
