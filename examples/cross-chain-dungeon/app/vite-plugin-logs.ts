// Dev-only Vite plugin: tail the demo's service logs (.run/*.log) and stream them
// to the browser over Server-Sent Events, so the UI can show live logs without a
// separate process or port. Runs inside the Vite dev server (same origin as the app).
//
//   GET /api/logs                  -> { services: [...] }
//   GET /api/logs/<service>/stream -> text/event-stream of log lines (tail + follow)
import { open } from "node:fs/promises";
import { statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";

const RUN_DIR = resolve(dirname(fileURLToPath(import.meta.url)), "../.run");

// service id -> log file under .run/ (the Sepolia torii's file is historically
// named torii-score.log; it indexes the bank world).
const SERVICES: Record<string, string> = {
  appchain: "appchain.log",
  saya: "saya.log",
  "torii-game": "torii-game.log",
  "torii-bank": "torii-score.log",
};

const ANSI = /\[[0-9;]*m/g;
const strip = (s: string) => s.replace(ANSI, "");

async function readRange(path: string, from: number, to: number): Promise<string> {
  const fh = await open(path, "r");
  try {
    const buf = Buffer.alloc(to - from);
    await fh.read(buf, 0, buf.length, from);
    return buf.toString("utf8");
  } finally {
    await fh.close();
  }
}

export function logStreamPlugin(): Plugin {
  return {
    name: "dungeon-log-stream",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/api/logs", async (req, res) => {
        const parts = (req.url ?? "/").split("?")[0].split("/").filter(Boolean);

        // GET /api/logs -> service list
        if (parts.length === 0) {
          res.setHeader("content-type", "application/json");
          res.end(JSON.stringify({ services: Object.keys(SERVICES) }));
          return;
        }

        const file = SERVICES[parts[0]];
        if (!file || parts[1] !== "stream") {
          res.statusCode = 404;
          res.end("not found");
          return;
        }
        const path = resolve(RUN_DIR, file);

        res.writeHead(200, {
          "content-type": "text/event-stream",
          "cache-control": "no-cache, no-transform",
          connection: "keep-alive",
        });
        res.write(": connected\n\n");
        const send = (line: string) => res.write(`data: ${strip(line)}\n\n`);

        let pos = 0;
        // Initial tail: last ~64KB, capped to 300 lines.
        try {
          const { size } = statSync(path);
          const start = Math.max(0, size - 64 * 1024);
          const text = await readRange(path, start, size);
          const lines = (start > 0 ? text.split("\n").slice(1) : text.split("\n")).slice(-300);
          for (const l of lines) if (l.length) send(l);
          pos = size;
        } catch {
          send(`(waiting for ${file} …)`);
        }

        // Follow: poll the file size; emit whole new lines, keep any partial tail.
        const poll = setInterval(async () => {
          try {
            const { size } = statSync(path);
            if (size < pos) pos = 0; // truncated / rotated
            if (size <= pos) return;
            const text = await readRange(path, pos, size);
            pos = size;
            const lines = text.split("\n");
            for (let i = 0; i < lines.length - 1; i++) if (lines[i].length) send(lines[i]);
            const partial = lines[lines.length - 1];
            if (partial.length) pos -= Buffer.byteLength(partial, "utf8"); // re-read next tick
          } catch {
            /* file gone for a moment — ignore */
          }
        }, 700);
        const ping = setInterval(() => res.write(": ping\n\n"), 20_000);

        req.on("close", () => {
          clearInterval(poll);
          clearInterval(ping);
        });
      });
    },
  };
}
