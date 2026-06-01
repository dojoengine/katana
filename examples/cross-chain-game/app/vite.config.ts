import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import mkcert from "vite-plugin-mkcert";
import wasm from "vite-plugin-wasm";
import topLevelAwait from "vite-plugin-top-level-await";

// Controller's passkey login (WebAuthn) needs a secure context with a *trusted*
// certificate. With CONTROLLER=1 we serve https://localhost:3001 via a
// locally-trusted mkcert CA (the first run installs the CA — a one-time prompt).
// The default (dev account) run stays plain http://localhost:3001.
const useHttps = process.env.CONTROLLER === "1" || process.env.HTTPS === "1";

// Port 3001 to avoid clashing with Katana's built-in explorer (3000).
export default defineConfig({
  // wasm + topLevelAwait are needed by @dojoengine/torii-wasm (the Torii gRPC
  // client used for entity/event subscriptions): its web build imports the
  // `.wasm` as an ES module and self-initializes at top level.
  plugins: [wasm(), topLevelAwait(), react(), tailwindcss(), ...(useHttps ? [mkcert()] : [])],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  // The wasm package ships its own glue; don't let esbuild pre-bundle it.
  optimizeDeps: { exclude: ["@dojoengine/torii-wasm"] },
  server: { port: 3001, strictPort: true },
});
