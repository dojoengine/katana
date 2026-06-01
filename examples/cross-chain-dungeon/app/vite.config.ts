import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import mkcert from "vite-plugin-mkcert";

// HTTPS by default — Cartridge Controller's passkey login (WebAuthn) needs a
// trusted secure context, and serving https everywhere keeps dev/prod parity.
// Served at https://localhost:3002 via a locally-trusted mkcert CA (the first run
// installs the CA — a one-time OS prompt). Set HTTP=1 to serve plain http instead.
const useHttps = process.env.HTTP !== "1";

// Port 3002 — distinct from cross-chain-game (3001) so both demos can run.
export default defineConfig({
  plugins: [react(), ...(useHttps ? [mkcert()] : [])],
  server: { port: 3002, strictPort: true },
});
