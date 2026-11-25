import crypto from "node:crypto";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import express, { type NextFunction, type Request, type Response } from "express";
import { WebSocketServer, type WebSocket } from "ws";
import type { Config } from "./config.js";
import { HttpError } from "./errors.js";
import type { MuteOption, UploadFile, ZaloSession } from "./session.js";
import { REACTIONS, type MentionInfo, type ThreadKind } from "./store.js";

const PUBLIC_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "public");
const MAX_TEXT_LENGTH = 4000;
const MUTE_OPTIONS = new Set<MuteOption>(["1h", "4h", "8am", "forever"]);
/** Zalo itself takes larger files; this is what a phone comfortably holds in memory while it uploads. */
const MAX_UPLOAD_BYTES = 100 * 1024 * 1024;
const MAX_FRIEND_MESSAGE = 150;

export function createServer(config: Config, session: ZaloSession): http.Server {
    const app = express();
    app.disable("x-powered-by");
    app.use(express.json({ limit: "64kb" }));
    app.use(express.static(PUBLIC_DIR));

    const api = express.Router();
    api.use((req, res, next) => {
        if (!isAuthorized(config, req.headers.authorization, req.query.token)) {
            res.status(401).json({ error: "Unauthorized" });
            return;
        }
        next();
    });

    api.get("/status", (_req, res) => {
        res.json(session.getStatus());
    });

    api.post("/login", (_req, res) => {
        void session.startQRLogin();
        res.status(202).json(session.getStatus());
    });

    /** Retry a saved session that couldn't reach Zalo, without waiting for the next attempt. */
    api.post("/retry", (_req, res) => {
        session.retryNow();
        res.status(202).json(session.getStatus());
    });

    api.post("/logout", wrap(async (_req, res) => {
        await session.logout();
        res.json(session.getStatus());
    }));

    api.post("/reconnect", (_req, res) => {
        session.reconnect();
        res.status(202).json(session.getStatus());
    });

    api.get("/reactions", (_req, res) => {
        res.json(Object.entries(REACTIONS).map(([key, r]) => ({ key, emoji: r.emoji })));
    });

    // ---- Conversations ----

    api.get("/threads", (_req, res) => {
        res.json(session.store.listThreads());
    });

    api.get("/threads/:kind/:id/info", wrap(async (req, res) => {
        const kind = parseKind(req.params.kind);
        const { id } = req.params;
        if (kind === "user") {
            res.json({ lastOnline: await session.lastOnline(id).catch(() => null) });
        } else {
            res.json({ members: await session.groupMembers(id) });
        }
    }));

    api.post("/threads/:kind/:id/read", wrap(async (req, res) => {
        await session.markRead(parseKind(req.params.kind), req.params.id);
        res.status(204).end();
    }));

    api.post("/threads/:kind/:id/unread", wrap(async (req, res) => {
        await session.markUnread(parseKind(req.params.kind), req.params.id);
        res.status(204).end();
    }));

    api.put("/threads/:kind/:id/pin", wrap(async (req, res) => {
        if (typeof req.body?.pinned !== "boolean") throw new HttpError(400, "pinned must be a boolean");
        await session.setPinned(parseKind(req.params.kind), req.params.id, req.body.pinned);
        res.status(204).end();
    }));

    api.put("/threads/:kind/:id/mute", wrap(async (req, res) => {
        const option = req.body?.duration ?? null;
        if (option !== null && !MUTE_OPTIONS.has(option)) {
            throw new HttpError(400, `duration must be one of ${[...MUTE_OPTIONS].join(", ")} or null`);
        }
        await session.setMuted(parseKind(req.params.kind), req.params.id, option);
        res.status(204).end();
    }));

    api.post("/threads/:kind/:id/typing", wrap(async (req, res) => {
        await session.sendTyping(parseKind(req.params.kind), req.params.id);
        res.status(204).end();
    }));

    api.delete("/threads/:kind/:id", wrap(async (req, res) => {
        await session.deleteConversation(parseKind(req.params.kind), req.params.id);
        res.status(204).end();
    }));

    /** The chat a client shows (or `{}` for none). Clients repeat it while it stays open. */
    api.put("/presence", (req, res) => {
        const { kind, id } = (req.body ?? {}) as { kind?: unknown; id?: unknown };
        if (kind === undefined && id === undefined) {
            session.setViewing(null, null);
        } else {
            if (typeof id !== "string" || !id) throw new HttpError(400, "id is required");
            session.setViewing(parseKind(String(kind)), id);
        }
        res.status(204).end();
    });

    // ---- Messages ----

    api.get("/threads/:kind/:id/messages", wrap(async (req, res) => {
        const kind = parseKind(req.params.kind);
        const { id } = req.params;
        const before = req.query.before !== undefined ? Number(req.query.before) : undefined;
        const limit = req.query.limit !== undefined ? Number(req.query.limit) : undefined;

        // Groups have a history endpoint, so fill a thin cache from the server on first open.
        if (kind === "group" && before === undefined && session.store.getMessages(kind, id).length < 20) {
            await session.fetchGroupHistory(id).catch((err) => console.warn("Group history failed:", err));
        }
        res.json({ thread: session.store.getThread(kind, id) ?? null, messages: session.store.getMessages(kind, id, { before, limit }) });
    }));

    /**
     * One photo or file as the raw request body; `name` is its file name (the extension decides photo or file),
     * `width` / `height` the pixel size of a photo.
     */
    api.post(
        "/threads/:kind/:id/attachments",
        express.raw({ type: () => true, limit: MAX_UPLOAD_BYTES }),
        wrap(async (req, res) => {
            const kind = parseKind(req.params.kind);
            const name = typeof req.query.name === "string" ? path.basename(req.query.name.trim()) : "";
            if (!name || !name.includes(".")) throw new HttpError(400, "name must be a file name with an extension");
            if (!Buffer.isBuffer(req.body) || req.body.length === 0) throw new HttpError(400, "The request body is the file");
            const file: UploadFile = { data: req.body, filename: name, width: positiveInt(req.query.width), height: positiveInt(req.query.height) };
            const msgIds = await session.sendAttachment(kind, req.params.id, file);
            res.status(201).json({ msgIds });
        }),
    );

    api.get("/stickers", wrap(async (req, res) => {
        const q = typeof req.query.q === "string" ? req.query.q.trim() : "";
        if (!q) throw new HttpError(400, "q is required");
        res.json(await session.searchStickers(q));
    }));

    api.post("/threads/:kind/:id/stickers", wrap(async (req, res) => {
        const id = Number(req.body?.id);
        if (!Number.isInteger(id) || id <= 0) throw new HttpError(400, "id must be a sticker id");
        await session.sendSticker(parseKind(req.params.kind), req.params.id, id);
        res.status(201).end();
    }));

    api.post("/threads/:kind/:id/messages", wrap(async (req, res) => {
        const kind = parseKind(req.params.kind);
        const text = typeof req.body?.text === "string" ? req.body.text.trim() : "";
        if (!text) throw new HttpError(400, "text is required");
        if (text.length > MAX_TEXT_LENGTH) throw new HttpError(400, `text is longer than ${MAX_TEXT_LENGTH} characters`);
        const quoteId = req.body?.quoteId === undefined ? undefined : String(req.body.quoteId);
        const mentions = parseMentions(req.body?.mentions, text);

        const msgId = await session.sendText(kind, req.params.id, text, { quoteId, mentions });
        res.status(201).json({ msgId });
    }));

    api.put("/threads/:kind/:id/messages/:msgId/reaction", wrap(async (req, res) => {
        const reaction = req.body?.reaction ?? null;
        if (reaction !== null && typeof reaction !== "string") throw new HttpError(400, "reaction must be a string or null");
        await session.react(parseKind(req.params.kind), req.params.id, req.params.msgId, reaction);
        res.status(204).end();
    }));

    api.post("/threads/:kind/:id/messages/:msgId/recall", wrap(async (req, res) => {
        await session.recall(parseKind(req.params.kind), req.params.id, req.params.msgId);
        res.status(204).end();
    }));

    api.delete("/threads/:kind/:id/messages/:msgId", wrap(async (req, res) => {
        await session.deleteForMe(parseKind(req.params.kind), req.params.id, req.params.msgId);
        res.status(204).end();
    }));

    api.post("/threads/:kind/:id/messages/:msgId/forward", wrap(async (req, res) => {
        const targets = req.body?.to;
        if (!Array.isArray(targets) || targets.length === 0) throw new HttpError(400, "to must be a non-empty array");
        const parsed = targets.map((t: unknown) => {
            const { kind, id } = (t ?? {}) as { kind?: unknown; id?: unknown };
            if (typeof id !== "string" || !id) throw new HttpError(400, "each target needs an id");
            return { kind: parseKind(String(kind)), id };
        });
        await session.forward(parseKind(req.params.kind), req.params.id, req.params.msgId, parsed);
        res.status(204).end();
    }));

    // ---- Friends ----

    api.get("/friends", (_req, res) => {
        res.json(session.friends());
    });

    api.get("/friend-requests", wrap(async (_req, res) => {
        res.json(await session.friendRequests());
    }));

    api.post("/friend-requests/:userId/accept", wrap(async (req, res) => {
        await session.acceptFriendRequest(req.params.userId);
        res.status(204).end();
    }));

    api.post("/friend-requests/:userId/reject", wrap(async (req, res) => {
        await session.rejectFriendRequest(req.params.userId);
        res.status(204).end();
    }));

    /** Sends a friend request: `{ "userId", "message"? }`. */
    api.post("/friend-requests", wrap(async (req, res) => {
        const userId = typeof req.body?.userId === "string" ? req.body.userId : "";
        if (!userId) throw new HttpError(400, "userId is required");
        const message = typeof req.body?.message === "string" ? req.body.message.trim().slice(0, MAX_FRIEND_MESSAGE) : "";
        await session.sendFriendRequest(userId, message);
        res.status(204).end();
    }));

    /** Takes back a request I sent. */
    api.delete("/friend-requests/:userId", wrap(async (req, res) => {
        await session.undoFriendRequest(req.params.userId);
        res.status(204).end();
    }));

    api.get("/users/find", wrap(async (req, res) => {
        const phone = typeof req.query.phone === "string" ? req.query.phone.replace(/[\s.()-]/g, "") : "";
        if (!/^\+?\d{8,15}$/.test(phone)) throw new HttpError(400, "phone must be a phone number");
        res.json(await session.findUser(phone.replace(/^\+/, "")));
    }));

    // ---- Groups ----

    /** `{ "name"?, "members": [userId, …] }` (at least 2 people besides me). */
    api.post("/groups", wrap(async (req, res) => {
        const members = parseIds(req.body?.members, "members");
        const name = typeof req.body?.name === "string" ? req.body.name : "";
        res.status(201).json(await session.createGroup(name, members));
    }));

    api.post("/groups/:id/members", wrap(async (req, res) => {
        res.json(await session.addGroupMembers(req.params.id, parseIds(req.body?.members, "members")));
    }));

    api.delete("/groups/:id/members/:userId", wrap(async (req, res) => {
        await session.removeGroupMember(req.params.id, req.params.userId);
        res.status(204).end();
    }));

    api.put("/groups/:id/name", wrap(async (req, res) => {
        if (typeof req.body?.name !== "string") throw new HttpError(400, "name must be a string");
        await session.renameGroup(req.params.id, req.body.name);
        res.status(204).end();
    }));

    /** `{ "silent"?: true }`: only the group's admins learn that I left. */
    api.post("/groups/:id/leave", wrap(async (req, res) => {
        await session.leaveGroup(req.params.id, req.body?.silent === true);
        res.status(204).end();
    }));

    app.use("/api", api);
    app.use((err: unknown, _req: Request, res: Response, _next: NextFunction) => {
        const status = err instanceof HttpError ? err.status : 500;
        const message = err instanceof Error ? err.message : String(err);
        if (status === 500) console.error(err);
        res.status(status).json({ error: message });
    });

    const server = http.createServer(app);
    attachWebSocket(server, config, session);
    return server;
}

/** Pushes session status, message changes, typing and thread-list changes to connected clients. */
function attachWebSocket(server: http.Server, config: Config, session: ZaloSession): void {
    const wss = new WebSocketServer({ noServer: true });

    server.on("upgrade", (req, socket, head) => {
        const url = new URL(req.url ?? "/", "http://localhost");
        if (url.pathname !== "/ws" || !isAuthorized(config, req.headers.authorization, url.searchParams.get("token"))) {
            socket.write("HTTP/1.1 401 Unauthorized\r\n\r\n");
            socket.destroy();
            return;
        }
        wss.handleUpgrade(req, socket, head, (ws) => wss.emit("connection", ws, req));
    });

    const broadcast = (payload: unknown) => {
        const data = JSON.stringify(payload);
        for (const client of wss.clients) {
            if (client.readyState === client.OPEN) client.send(data);
        }
    };

    session.on("status", (status) => broadcast({ type: "status", status }));
    // `notify`: whether a phone should show it (not mine, not muted, not the chat that is open, not old history).
    session.on("message", (message) => {
        const notify = session.shouldNotify(message);
        broadcast({
            type: "message",
            message,
            notify,
            ...(notify ? { threadName: session.threadName(message.threadKind, message.threadId) } : {}),
        });
    });
    session.on("read", (read) => broadcast({ type: "read", ...read }));
    session.on("message_updated", (message) => broadcast({ type: "message_updated", message }));
    session.on("message_deleted", (deleted) => broadcast({ type: "message_deleted", ...deleted }));
    session.on("typing", (typing) => broadcast({ type: "typing", ...typing }));
    session.on("threads", () => broadcast({ type: "threads" }));

    wss.on("connection", (ws: WebSocket) => {
        ws.send(JSON.stringify({ type: "status", status: session.getStatus() }));
    });
}

/** Express 4 does not forward rejected promises to the error handler on its own. */
function wrap(handler: (req: Request, res: Response) => Promise<void>) {
    return (req: Request, res: Response, next: NextFunction) => {
        handler(req, res).catch(next);
    };
}

function isAuthorized(config: Config, header: string | undefined, queryToken: unknown): boolean {
    if (!config.accessToken) return true;
    const presented = header?.startsWith("Bearer ") ? header.slice(7) : typeof queryToken === "string" ? queryToken : "";
    const a = Buffer.from(presented);
    const b = Buffer.from(config.accessToken);
    return a.length === b.length && crypto.timingSafeEqual(a, b);
}

function positiveInt(value: unknown): number | undefined {
    const n = typeof value === "string" ? Number(value) : NaN;
    return Number.isInteger(n) && n > 0 ? n : undefined;
}

function parseIds(input: unknown, field: string): string[] {
    if (!Array.isArray(input) || input.length === 0 || !input.every((v) => typeof v === "string" && v.length > 0)) {
        throw new HttpError(400, `${field} must be a non-empty array of user ids`);
    }
    return input as string[];
}

function parseKind(kind: string): ThreadKind {
    if (kind === "user" || kind === "group") return kind;
    throw new HttpError(400, `Unknown thread kind: ${kind}`);
}

export function parseMentions(input: unknown, text: string): MentionInfo[] | undefined {
    if (input === undefined || input === null) return undefined;
    if (!Array.isArray(input)) throw new HttpError(400, "mentions must be an array");
    return input.map((m: unknown) => {
        const { uid, pos, len } = (m ?? {}) as Record<string, unknown>;
        if (typeof uid !== "string" || !Number.isInteger(pos) || !Number.isInteger(len)) {
            throw new HttpError(400, "each mention needs uid, pos and len");
        }
        const p = pos as number;
        const l = len as number;
        if (p < 0 || l <= 0 || p + l > text.length) throw new HttpError(400, "mention is outside the text");
        return { uid, pos: p, len: l };
    });
}
