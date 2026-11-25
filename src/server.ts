import crypto from "node:crypto";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import express, { type NextFunction, type Request, type Response } from "express";
import { WebSocketServer, type WebSocket } from "ws";
import type { Config } from "./config.js";
import { HttpError } from "./errors.js";
import type { CallEvent, MuteOption, UploadFile, ZaloSession } from "./session.js";
import { REACTIONS, type MentionInfo, type ThreadKind } from "./store.js";

const PUBLIC_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "public");
const MAX_TEXT_LENGTH = 4000;
const MUTE_OPTIONS = new Set<MuteOption>(["1h", "4h", "8am", "forever"]);
/** Zalo itself takes larger files; this is what a phone comfortably holds in memory while it uploads. */
const MAX_UPLOAD_BYTES = 100 * 1024 * 1024;
const MAX_FRIEND_MESSAGE = 150;
const MAX_FORWARD_TARGETS = 20;
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "::1", "[::1]"]);
const STATIC_CSP =
    "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: https: http:; connect-src 'self' ws: wss:; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

export function createServer(config: Config, session: ZaloSession): http.Server {
    const app = express();
    app.disable("x-powered-by");

    app.use((req, res, next) => {
        res.setHeader("X-Content-Type-Options", "nosniff");
        res.setHeader("X-Frame-Options", "DENY");
        res.setHeader("Referrer-Policy", "no-referrer");
        if (req.path !== "/api" && !req.path.startsWith("/api/")) {
            res.setHeader("Content-Security-Policy", STATIC_CSP);
        }
        if (!isValidHost(config, req.headers.host)) {
            res.status(403).json({ error: "Forbidden" });
            return;
        }
        next();
    });

    app.use(express.static(PUBLIC_DIR));

    const api = express.Router();
    api.use((req, res, next) => {
        const isMutation = req.method !== "GET" && req.method !== "HEAD" && req.method !== "OPTIONS";
        if (!isAllowedOrigin(req.headers, isMutation)) {
            res.status(403).json({ error: "Forbidden" });
            return;
        }
        if (!isAuthorized(config, req.headers.authorization, req.query.token)) {
            res.status(401).json({ error: "Unauthorized" });
            return;
        }
        next();
    });

    /**
     * One photo or file as the raw request body; `name` is its file name (the extension decides photo or file),
     * `width` / `height` the pixel size of a photo. Mounted before express.json() so .json uploads arrive as Buffers.
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

    api.use(express.json({ limit: "64kb" }));

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
            const lastOnline = await session.lastOnline(id).catch((err) => {
                if (err instanceof HttpError) throw err;
                return null;
            });
            res.json({ lastOnline });
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

    /** The chat a client shows (or `{}` / `{ kind: null, id: null }` for none). Clients repeat it while it stays open. */
    api.put("/presence", (req, res) => {
        const { kind, id } = (req.body ?? {}) as { kind?: unknown; id?: unknown };
        if ((kind === undefined || kind === null) && (id === undefined || id === null)) {
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
        let before: number | undefined;
        if (req.query.before !== undefined) {
            before = typeof req.query.before === "string" && req.query.before.trim() !== "" ? Number(req.query.before) : NaN;
            if (!Number.isFinite(before)) throw new HttpError(400, "before must be a finite number");
        }
        let limit: number | undefined;
        if (req.query.limit !== undefined) {
            limit = positiveInt(req.query.limit);
            if (limit === undefined) throw new HttpError(400, "limit must be a positive integer");
        }

        // Groups have a history endpoint, so fill a thin cache from the server on first open.
        if (kind === "group" && before === undefined && session.store.getMessages(kind, id).length < 20) {
            await session.fetchGroupHistory(id).catch((err) => console.warn("Group history failed:", err));
        }
        res.json({ thread: session.store.getThread(kind, id) ?? null, messages: session.store.getMessages(kind, id, { before, limit }) });
    }));

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
        const rawQuoteId = req.body?.quoteId;
        const quoteId = rawQuoteId === undefined || rawQuoteId === null || rawQuoteId === "" ? undefined : String(rawQuoteId);
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
        const seen = new Set<string>();
        const parsed: { kind: ThreadKind; id: string }[] = [];
        for (const t of targets) {
            const { kind, id } = (t ?? {}) as { kind?: unknown; id?: unknown };
            if (typeof id !== "string" || !id) throw new HttpError(400, "each target needs an id");
            const k = parseKind(String(kind));
            const key = `${k}:${id}`;
            if (seen.has(key)) continue;
            seen.add(key);
            parsed.push({ kind: k, id });
            if (parsed.length > MAX_FORWARD_TARGETS) {
                throw new HttpError(400, `Cannot forward to more than ${MAX_FORWARD_TARGETS} chats at once`);
            }
        }
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

    /** Requests I sent and nobody answered yet (to take back). */
    api.get("/friend-requests/sent", wrap(async (_req, res) => {
        res.json(await session.sentFriendRequests());
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

    // ---- Polls (group chats) ----

    /**
     * `{ question, options: string[], allowMultiple?, allowAddOptions?, anonymous?, hideVotePreview?, expiresAt? }`
     * (`expiresAt` epoch ms, absent or 0 for none). Answers with the poll view; the poll message itself arrives over /ws.
     */
    api.post("/groups/:id/polls", wrap(async (req, res) => {
        const { question, options, expiresAt } = req.body ?? {};
        if (typeof question !== "string" || !question.trim()) throw new HttpError(400, "question is required");
        if (question.trim().length > MAX_POLL_TEXT) throw new HttpError(400, `question is limited to ${MAX_POLL_TEXT} characters`);
        const cleaned = cleanOptions(options);
        if (cleaned.length < 2) throw new HttpError(400, "A poll needs at least 2 different options");
        if (cleaned.length > MAX_POLL_OPTIONS) throw new HttpError(400, `A poll takes at most ${MAX_POLL_OPTIONS} options`);
        let deadline = 0;
        if (expiresAt !== undefined && expiresAt !== null && expiresAt !== 0) {
            if (typeof expiresAt !== "number" || !Number.isFinite(expiresAt) || expiresAt <= Date.now()) {
                throw new HttpError(400, "expiresAt must be a time in the future (epoch ms)");
            }
            deadline = Math.floor(expiresAt);
        }
        const flag = (v: unknown) => v === true;
        const poll = await session.createPoll(req.params.id, {
            question: question.trim(),
            options: cleaned,
            allowMultiple: flag(req.body.allowMultiple),
            allowAddOptions: flag(req.body.allowAddOptions),
            anonymous: flag(req.body.anonymous),
            hideVotePreview: flag(req.body.hideVotePreview),
            expiresAt: deadline,
        });
        res.status(201).json(poll);
    }));

    api.get("/polls/:pollId", wrap(async (req, res) => {
        res.json(await session.pollDetail(parsePollId(req.params.pollId)));
    }));

    /** `{ optionIds: number[] }`: the options this account now chooses; an empty list removes its vote. */
    api.post("/polls/:pollId/vote", wrap(async (req, res) => {
        const ids = req.body?.optionIds;
        if (!Array.isArray(ids) || !ids.every((i) => Number.isInteger(i) && i >= 0) || ids.length > MAX_POLL_OPTIONS) {
            throw new HttpError(400, "optionIds must be an array of option ids");
        }
        res.json(await session.votePoll(parsePollId(req.params.pollId), [...new Set<number>(ids)]));
    }));

    /** `{ options: string[] }` */
    api.post("/polls/:pollId/options", wrap(async (req, res) => {
        const cleaned = cleanOptions(req.body?.options);
        if (cleaned.length === 0) throw new HttpError(400, "options must hold at least one option");
        if (cleaned.length > MAX_POLL_OPTIONS) throw new HttpError(400, `At most ${MAX_POLL_OPTIONS} options at a time`);
        res.json(await session.addPollOptions(parsePollId(req.params.pollId), cleaned));
    }));

    api.post("/polls/:pollId/lock", wrap(async (req, res) => {
        res.json(await session.lockPoll(parsePollId(req.params.pollId)));
    }));

    // ---- Calls (docs/calls.md) ----

    /** Declines a call ringing on this account. */
    api.post("/calls/:callId/decline", wrap(async (req, res) => {
        await session.declineCall(req.params.callId);
        res.status(204).end();
    }));

    /** Picks up a ringing call; its audio then flows over the /ws/call-audio socket. */
    api.post("/calls/:callId/answer", wrap(async (req, res) => {
        await session.answerCall(req.params.callId);
        res.status(204).end();
    }));

    /** Picks up a ringing call for 30 s without sound, to record the media connection in the call log. */
    api.post("/calls/:callId/answer-test", wrap(async (req, res) => {
        await session.answerCallTest(req.params.callId);
        res.status(204).end();
    }));

    /** `{ "userId": "123", "video": true | false }`: places a 1:1 call; progress arrives as call events. Answers the call. */
    api.post("/calls/start", wrap(async (req, res) => {
        const userId = typeof req.body?.userId === "string" ? req.body.userId : "";
        res.json(await session.startCall(userId, req.body?.video === true));
    }));

    api.post("/calls/hangup", (_req, res) => {
        session.hangUpCall();
        res.status(204).end();
    });

    /** `{ "muted": true | false }`: tells the other side (the app silences the microphone itself). */
    api.post("/calls/mute", (req, res) => {
        session.setCallMuted(req.body?.muted === true);
        res.status(204).end();
    });

    /** `{ "on": true | false }`: tells the other side our camera is on / off; the app's video is forwarded only while on. */
    api.post("/calls/camera", (req, res) => {
        session.setCallCamera(req.body?.on === true);
        res.status(204).end();
    });

    api.get("/calls/current", (_req, res) => {
        const call = session.currentCall();
        if (!call) {
            res.status(404).json({ error: "No call in progress" });
            return;
        }
        res.json(call);
    });

    // ---- Call research (docs/calls.md) ----

    /** The call log: one JSON line per call event Zalo sent. Holds call ids and server addresses, no message text. */
    api.get("/debug/call-log", (_req, res) => {
        res.type("text/plain").send(session.callLog());
    });

    api.delete("/debug/call-log", (_req, res) => {
        session.clearCallLog();
        res.status(204).end();
    });

    app.use("/api", api);
    app.use((err: unknown, _req: Request, res: Response, _next: NextFunction) => {
        if (err instanceof HttpError) {
            res.status(err.status).json({ error: err.message });
            return;
        }
        const statusCode =
            typeof err === "object" && err !== null
                ? Number((err as { status?: unknown; statusCode?: unknown }).status ?? (err as { statusCode?: unknown }).statusCode)
                : NaN;
        if (Number.isInteger(statusCode) && statusCode >= 400 && statusCode < 500) {
            const message = err instanceof Error && err.message ? err.message : "Bad request";
            res.status(statusCode).json({ error: message });
            return;
        }
        console.error(err);
        res.status(500).json({ error: "Internal server error" });
    });

    const server = http.createServer(app);
    attachWebSocket(server, config, session);
    return server;
}

const WS_HEARTBEAT_MS = 30_000;
const MAX_MEDIA_BUFFERED_BYTES = 2 * 1024 * 1024;

/** Pushes session status, message changes, typing and thread-list changes to connected clients. */
function attachWebSocket(server: http.Server, config: Config, session: ZaloSession): void {
    const wss = new WebSocketServer({ noServer: true, maxPayload: 64 * 1024 });
    /** The call's audio, both ways, as binary messages (docs/call-audio.md §3). */
    const audioWss = new WebSocketServer({ noServer: true, maxPayload: MAX_VIDEO_FRAME + 1024 });
    const alive = new WeakSet<WebSocket>();

    server.on("upgrade", (req, socket, head) => {
        socket.on("error", () => {});
        let url: URL;
        try {
            url = new URL(req.url ?? "/", "http://localhost");
        } catch {
            socket.write("HTTP/1.1 400 Bad Request\r\n\r\n");
            socket.destroy();
            return;
        }
        if (!isValidHost(config, req.headers.host) || !isAllowedOrigin(req.headers, true)) {
            socket.write("HTTP/1.1 403 Forbidden\r\n\r\n");
            socket.destroy();
            return;
        }
        if (!isAuthorized(config, req.headers.authorization, url.searchParams.get("token"))) {
            socket.write("HTTP/1.1 401 Unauthorized\r\n\r\n");
            socket.destroy();
            return;
        }
        const target = url.pathname === "/ws" ? wss : url.pathname === "/ws/call-audio" ? audioWss : null;
        if (!target) {
            socket.write("HTTP/1.1 404 Not Found\r\n\r\n");
            socket.destroy();
            return;
        }
        target.handleUpgrade(req, socket, head, (ws) => target.emit("connection", ws, req));
    });

    const sendToApp = (frame: Buffer) => {
        if (audioWss.clients.size === 0) return;
        for (const client of audioWss.clients) {
            if (client.readyState === client.OPEN && client.bufferedAmount <= MAX_MEDIA_BUFFERED_BYTES) {
                client.send(frame, () => {});
            }
        }
    };

    const onCallAudio = ({ seq, timestamp, opus }: { seq: number; timestamp: number; opus: Buffer }) => {
        if (audioWss.clients.size === 0) return;
        sendToApp(encodeAudioToApp(seq, timestamp, opus));
    };
    const onCallAudioClose = (call: CallEvent) => {
        if (call.type === "ended" && !session.currentCall()) {
            for (const client of audioWss.clients) client.close(1000, "call ended");
        }
    };
    const onCallVideo = ({ key, timestamp, data }: { key: boolean; timestamp: number; data: Buffer }) =>
        sendToApp(encodeVideoToApp(key, timestamp, data));
    const onCallKeyFrame = () => sendToApp(KEY_FRAME_REQUEST);

    audioWss.on("connection", (ws: WebSocket) => {
        ws.on("error", () => {});
        alive.add(ws);
        ws.on("pong", () => alive.add(ws));
        if (!session.currentCall()) {
            ws.close(1008, "no call in progress");
            return;
        }
        for (const existing of audioWss.clients) {
            if (existing !== ws && (existing.readyState === existing.OPEN || existing.readyState === existing.CONNECTING)) {
                existing.close(1000, "replaced");
            }
        }
        ws.on("message", (data: Buffer, isBinary: boolean) => {
            if (!isBinary) return;
            const opus = decodeAudioFromApp(data);
            if (opus) {
                session.sendCallAudio(opus);
                return;
            }
            const video = decodeVideoFromApp(data);
            if (video) session.sendCallVideo(video);
            else if (isKeyFrameWanted(data)) session.requestCallKeyFrame();
        });
    });

    const broadcast = (payload: unknown) => {
        const data = JSON.stringify(payload);
        for (const client of wss.clients) {
            if (client.readyState === client.OPEN) client.send(data, () => {});
        }
    };

    const onStatus = (status: ReturnType<ZaloSession["getStatus"]>) => broadcast({ type: "status", status });
    // `notify`: whether a phone should show it (not mine, not muted, not the chat that is open, not old history).
    const onMessage = (message: Parameters<ZaloSession["shouldNotify"]>[0]) => {
        const notify = session.shouldNotify(message);
        broadcast({
            type: "message",
            message,
            notify,
            ...(notify ? { threadName: session.threadName(message.threadKind, message.threadId) } : {}),
        });
    };
    const onRead = (read: { threadKind: ThreadKind; threadId: string }) => broadcast({ type: "read", ...read });
    const onCallBroadcast = (call: CallEvent) => broadcast({ type: "call", call });
    const onMessageUpdated = (message: Parameters<ZaloSession["shouldNotify"]>[0]) => broadcast({ type: "message_updated", message });
    const onMessageDeleted = (deleted: { threadKind: ThreadKind; threadId: string; id: string }) =>
        broadcast({ type: "message_deleted", ...deleted });
    const onTyping = (typing: { threadKind: ThreadKind; threadId: string; uid: string; name: string }) =>
        broadcast({ type: "typing", ...typing });
    const onThreads = () => broadcast({ type: "threads" });
    const onPoll = (poll: { pollId: number }) => broadcast({ type: "poll", pollId: poll.pollId });

    session.on("call_audio", onCallAudio);
    session.on("call", onCallAudioClose);
    session.on("call_video", onCallVideo);
    session.on("call_key_frame", onCallKeyFrame);
    session.on("status", onStatus);
    session.on("message", onMessage);
    session.on("read", onRead);
    session.on("call", onCallBroadcast);
    session.on("message_updated", onMessageUpdated);
    session.on("message_deleted", onMessageDeleted);
    session.on("typing", onTyping);
    session.on("threads", onThreads);
    session.on("poll", onPoll);

    wss.on("connection", (ws: WebSocket) => {
        ws.on("error", () => {});
        alive.add(ws);
        ws.on("pong", () => alive.add(ws));
        ws.send(JSON.stringify({ type: "status", status: session.getStatus() }), () => {});
    });

    const heartbeat = setInterval(() => {
        for (const wsServer of [wss, audioWss]) {
            for (const client of wsServer.clients) {
                if (client.readyState !== client.OPEN) continue;
                if (!alive.has(client)) {
                    client.terminate();
                    continue;
                }
                alive.delete(client);
                try {
                    client.ping();
                } catch {
                    client.terminate();
                }
            }
        }
    }, WS_HEARTBEAT_MS);
    heartbeat.unref();

    server.on("close", () => {
        clearInterval(heartbeat);
        session.off("call_audio", onCallAudio);
        session.off("call", onCallAudioClose);
        session.off("call_video", onCallVideo);
        session.off("call_key_frame", onCallKeyFrame);
        session.off("status", onStatus);
        session.off("message", onMessage);
        session.off("read", onRead);
        session.off("call", onCallBroadcast);
        session.off("message_updated", onMessageUpdated);
        session.off("message_deleted", onMessageDeleted);
        session.off("typing", onTyping);
        session.off("threads", onThreads);
        session.off("poll", onPoll);
        wss.close();
        audioWss.close();
    });
}

/** Server → app audio message: `01`, u16 RTP sequence, u32 RTP timestamp (big-endian), Opus packet. */
export function encodeAudioToApp(seq: number, timestamp: number, opus: Buffer): Buffer {
    const head = Buffer.alloc(7);
    head[0] = 1;
    head.writeUInt16BE(seq & 0xffff, 1);
    head.writeUInt32BE(timestamp >>> 0, 3);
    return Buffer.concat([head, opus]);
}

/** App → server audio message: `02` + one Opus frame; anything else is ignored. */
export function decodeAudioFromApp(data: Buffer): Buffer | null {
    return data.length > 1 && data.length <= 1276 && data[0] === 2 ? data.subarray(1) : null;
}

/** Server → app video message: `03`, u8 flags (bit 0 = key frame), u32 RTP timestamp (90 kHz), H.264 access unit (Annex B). */
export function encodeVideoToApp(key: boolean, timestamp: number, data: Buffer): Buffer {
    const head = Buffer.alloc(6);
    head[0] = 3;
    head[1] = key ? 1 : 0;
    head.writeUInt32BE(timestamp >>> 0, 2);
    return Buffer.concat([head, data]);
}

/** Server → app: the other side asked for a key frame. */
export const KEY_FRAME_REQUEST = Buffer.from([5]);

/** Biggest access unit accepted from the app (a 640x480 key frame is a few tens of kilobytes). */
const MAX_VIDEO_FRAME = 512 * 1024;

/** App → server video message: `04`, u8 flags (bit 0 = key frame), access unit; anything else is ignored. */
export function decodeVideoFromApp(data: Buffer): { key: boolean; data: Buffer } | null {
    if (data.length < 3 || data.length > MAX_VIDEO_FRAME + 2 || data[0] !== 4) return null;
    return { key: (data[1] & 1) !== 0, data: data.subarray(2) };
}

/** App → server: its decoder needs a key frame (`06`). */
export function isKeyFrameWanted(data: Buffer): boolean {
    return data.length >= 1 && data[0] === 6;
}

/** Express 4 does not forward rejected promises to the error handler on its own. */
const MAX_POLL_TEXT = 200;
const MAX_POLL_OPTIONS = 30;

/** Trimmed, non-empty, de-duplicated option texts; anything that isn't a string array gives an empty list. */
function cleanOptions(value: unknown): string[] {
    if (!Array.isArray(value)) return [];
    const seen = new Set<string>();
    for (const v of value) {
        if (typeof v !== "string") continue;
        const text = v.trim().slice(0, MAX_POLL_TEXT);
        if (text) seen.add(text);
    }
    return [...seen];
}

function parsePollId(value: string): number {
    const id = Number(value);
    if (!Number.isSafeInteger(id) || id <= 0) throw new HttpError(400, "Invalid poll id");
    return id;
}

function wrap(handler: (req: Request, res: Response) => Promise<void>) {
    return (req: Request, res: Response, next: NextFunction) => {
        handler(req, res).catch(next);
    };
}

function isValidHost(config: Config, hostHeader: string | undefined): boolean {
    if (!LOOPBACK_HOSTS.has(config.host)) return true;
    if (!hostHeader || /[\s/@\\]/.test(hostHeader)) return false;
    try {
        const u = new URL(`http://${hostHeader}`);
        return LOOPBACK_HOSTS.has(u.hostname.toLowerCase());
    } catch {
        return false;
    }
}

function originMatchesHost(origin: string, hostHeader: string | undefined): boolean {
    if (!hostHeader || /[\s/@\\]/.test(hostHeader)) return false;
    try {
        const originUrl = new URL(origin);
        if (originUrl.protocol !== "http:" && originUrl.protocol !== "https:") return false;
        const hostUrl = new URL(`${originUrl.protocol}//${hostHeader}`);
        return originUrl.host.toLowerCase() === hostUrl.host.toLowerCase();
    } catch {
        return false;
    }
}

function isAllowedOrigin(headers: http.IncomingHttpHeaders, isMutationOrWs: boolean): boolean {
    const rawSecFetchSite = headers["sec-fetch-site"];
    const secFetchSite = typeof rawSecFetchSite === "string" ? rawSecFetchSite.trim().toLowerCase() : "";
    const rawOrigin = headers.origin;
    const origin = typeof rawOrigin === "string" ? rawOrigin.trim() : undefined;

    if (secFetchSite === "cross-site") return false;
    if (origin !== undefined && !originMatchesHost(origin, headers.host)) return false;
    if (isMutationOrWs && secFetchSite === "same-site" && (!origin || !originMatchesHost(origin, headers.host))) {
        return false;
    }
    return true;
}

function isAuthorized(config: Config, header: string | undefined, queryToken: unknown): boolean {
    if (!config.accessToken) return true;
    const presented = header?.startsWith("Bearer ") ? header.slice(7) : typeof queryToken === "string" ? queryToken : "";
    const a = Buffer.from(presented);
    const b = Buffer.from(config.accessToken);
    return a.length === b.length && crypto.timingSafeEqual(a, b);
}

function positiveInt(value: unknown): number | undefined {
    const n = typeof value === "string" && value.trim() !== "" ? Number(value) : NaN;
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
        if (typeof uid !== "string" || uid.length === 0 || !Number.isInteger(pos) || !Number.isInteger(len)) {
            throw new HttpError(400, "each mention needs uid, pos and len");
        }
        const p = pos as number;
        const l = len as number;
        if (p < 0 || l <= 0 || p + l > text.length) throw new HttpError(400, "mention is outside the text");
        return { uid, pos: p, len: l };
    });
}
