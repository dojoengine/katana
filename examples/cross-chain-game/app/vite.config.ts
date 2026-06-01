import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import mkcert from "vite-plugin-mkcert";

// Controller's passkey login (WebAuthn) needs a secure context with a *trusted*
// certificate. With CONTROLLER=1 we serve https://localhost:3001 via a
// locally-trusted mkcert CA (the first run installs the CA — a one-time prompt).
// The default (dev account) run stays plain http://localhost:3001.
const useHttps = process.env.CONTROLLER === "1" || process.env.HTTPS === "1";

// Port 3001 to avoid clashing with Katana's built-in explorer (3000).
export default defineConfig({
  plugins: [react(), tailwindcss(), ...(useHttps ? [mkcert()] : [])],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: { port: 3001, strictPort: true },
});
