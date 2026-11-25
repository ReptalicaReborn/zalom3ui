import fs from "node:fs";
import path from "node:path";
import util from "node:util";
import { loadConfig } from "./config.js";
import { createServer } from "./server.js";
import { ZaloSession } from "./session.js";
import { ChatStore } from "./store.js";

// LOG_FILE (set by the Android app): copy console output there, so a server that won't start can be diagnosed.
const LOG_STARTUP_ROTATE_BYTES = 256 * 1024;
const LOG_MAX_BYTES = 2 * 1024 * 1024;
const logFile = process.env.LOG_FILE;
if (logFile) {
    let logBytes = 0;
    try {
        if (fs.existsSync(logFile)) {
            const size = fs.statSync(logFile).size;
            if (size > LOG_STARTUP_ROTATE_BYTES) {
                fs.renameSync(logFile, logFile + ".1");
            } else {
                logBytes = size;
            }
        }
    } catch {
        // Logging must never stop the server.
    }
    for (const level of ["log", "warn", "error"] as const) {
        const original = console[level].bind(console);
        console[level] = (...args: unknown[]) => {
            original(...args);
            try {
                const text = args.map((a) => (a instanceof Error ? (a.stack ?? a.message) : typeof a === "string" ? a : util.inspect(a))).join(" ");
                const line = `${new Date().toISOString()} ${level} ${text}\n`;
                const lineBytes = Buffer.byteLength(line);
                if (logBytes + lineBytes > LOG_MAX_BYTES && fs.existsSync(logFile)) {
                    fs.renameSync(logFile, logFile + ".1");
                    logBytes = 0;
                }
                fs.appendFileSync(logFile, line, { mode: 0o600 });
                logBytes += lineBytes;
            } catch {
                // Ignore: the file may be gone.
            }
        };
    }
}

let store: ChatStore | null = null;
process.on("uncaughtException", (err) => {
    console.error("Uncaught exception:", err);
    try {
        store?.flush();
    } catch {
        // Best effort before exiting.
    }
    process.exit(1);
});

const config = loadConfig();
store = new ChatStore(path.join(config.dataDir, "messages.json"));
const session = new ZaloSession(store, config.dataDir, config.readReceipts);
const server = createServer(config, session);

// Another server on this port (another app on the phone, a second copy): say so instead of serving nothing.
server.on("error", (err: NodeJS.ErrnoException) => {
    console.error(err.code === "EADDRINUSE" ? `Port ${config.port} is already in use; the Zalỏ server can't start` : err);
    try {
        store?.flush();
    } catch {
        // Best effort before exiting.
    }
    process.exit(1);
});

server.listen(config.port, config.host, () => {
    console.log(`Zalỏ server listening on http://${config.host}:${config.port}`);
});

void session.resume();

let shuttingDown = false;
for (const signal of ["SIGINT", "SIGTERM"] as const) {
    process.on(signal, () => {
        if (shuttingDown) return;
        shuttingDown = true;
        try {
            session.shutdown();
        } catch {
            // Best effort.
        }
        try {
            store?.flush();
        } catch {
            // Best effort.
        }
        const forceExit = setTimeout(() => process.exit(0), 3_000);
        forceExit.unref();
        try {
            server.close(() => {
                try {
                    store?.flush();
                } catch {
                    // Best effort.
                }
                process.exit(0);
            });
            server.closeAllConnections?.();
        } catch {
            process.exit(0);
        }
    });
}
