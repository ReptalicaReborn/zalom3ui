import path from "node:path";
import { loadConfig } from "./config.js";
import { createServer } from "./server.js";
import { ZaloSession } from "./session.js";
import { ChatStore } from "./store.js";

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
