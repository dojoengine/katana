import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import mkcert from "vite-plugin-mkcert";

// Cartridge Controller's passkey login (WebAuthn) needs a secure context with a
// *trusted* certificate. With CONTROLLER=1 we serve https://localhost:3002 via a
// locally-trusted mkcert CA (first run installs the CA — a one-time prompt). The
// default (operator-account) run stays plain http://localhost:3002.
const useHttps = process.env.CONTROLLER === "1" || process.env.HTTPS === "1";

// Port 3002 — distinct from cross-chain-game (3001) so both demos can run.
export default defineConfig({
  plugins: [react(), ...(useHttps ? [mkcert()] : [])],
  server: { port: 3002, strictPort: true },
});
