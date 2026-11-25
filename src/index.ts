import fs from "node:fs";
import path from "node:path";
import util from "node:util";
import { loadConfig } from "./config.js";
import { createServer } from "./server.js";
import { ZaloSession } from "./session.js";
import { ChatStore } from "./store.js";

// LOG_FILE (set by the Android app): copy console output there, so a server that won't start can be diagnosed.
const logFile = process.env.LOG_FILE;
if (logFile) {
    try {
        if (fs.existsSync(logFile) && fs.statSync(logFile).size > 256 * 1024) fs.renameSync(logFile, logFile + ".1");
    } catch {
        // Logging must never stop the server.
    }
    for (const level of ["log", "warn", "error"] as const) {
        const original = console[level].bind(console);
        console[level] = (...args: unknown[]) => {
            original(...args);
            try {
                const text = args.map((a) => (a instanceof Error ? (a.stack ?? a.message) : typeof a === "string" ? a : util.inspect(a))).join(" ");
                fs.appendFileSync(logFile, `${new Date().toISOString()} ${level} ${text}\n`);
            } catch {
                // Ignore: the file may be gone.
            }
        };
    }
}
process.on("uncaughtException", (err) => {
    console.error("Uncaught exception:", err);
    process.exit(1);
});

const config = loadConfig();
const store = new ChatStore(path.join(config.dataDir, "messages.json"));
const session = new ZaloSession(store, config.dataDir, config.readReceipts);
const server = createServer(config, session);

// Another server on this port (another app on the phone, a second copy): say so instead of serving nothing.
server.on("error", (err: NodeJS.ErrnoException) => {
    console.error(err.code === "EADDRINUSE" ? `Port ${config.port} is already in use; the Zalỏ server can't start` : err);
    store.flush();
    process.exit(1);
});

server.listen(config.port, config.host, () => {
    console.log(`Zalỏ server listening on http://${config.host}:${config.port}`);
});

void session.resume();

for (const signal of ["SIGINT", "SIGTERM"] as const) {
    process.on(signal, () => {
        store.flush();
        process.exit(0);
    });
}
