// Token from ?token=... is remembered so it only needs to be typed once per device.
const TOKEN_KEY = "watchhrept-token";
const urlToken = new URLSearchParams(location.search).get("token");
if (urlToken) {
    try { localStorage.setItem(TOKEN_KEY, urlToken); } catch {}
    history.replaceState(null, "", location.pathname);
}
let token = urlToken;
try { token ??= localStorage.getItem(TOKEN_KEY); } catch {}

const $ = (id) => document.getElementById(id);
const state = {
    status: null,
    threads: [],
    current: null, // { kind, id }
    messages: [],
};

async function api(method, path, body) {
    const res = await fetch(`/api${path}`, {
        method,
        headers: {
            ...(body ? { "Content-Type": "application/json" } : {}),
            ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        body: body ? JSON.stringify(body) : undefined,
    });
    if (res.status === 204) return null;
    const data = await res.json().catch(() => ({}));
    if (!res.ok) {
        const err = new Error(data.error || `HTTP ${res.status}`);
        err.status = res.status;
        throw err;
    }
    return data;
}

// ---- Rendering ----

function renderStatus() {
    const s = state.status;
    if (!s) return;
    const loggedIn = s.state === "ready" || s.state === "disconnected";
    $("login").hidden = loggedIn;
    $("app").hidden = !loggedIn;

    if (!loggedIn) {
        $("qr").hidden = !s.qrImage;
        if (s.qrImage) $("qr").src = s.qrImage;
        $("login-btn").hidden = s.state !== "logged_out" && !s.retrying;
        $("login-hint").textContent =
            s.state === "awaiting_qr_scan"
                ? s.scannedBy
                    ? `Scanned by ${s.scannedBy}. Confirm on your phone.`
                    : "Open Zalo on your phone and scan this code."
                : s.state === "logging_in"
                  ? "Logging in…"
                  : "";
        $("login-error").textContent = s.error || "";
        return;
    }

    $("me").textContent = s.ownName || s.ownId || "";
    $("banner").hidden = s.state !== "disconnected";
    $("banner-text").textContent = s.error || "Disconnected";
}

function avatarEl(thread) {
    const avatarUrl = safeUrl(thread.avatar);
    if (avatarUrl) {
        const img = document.createElement("img");
        img.className = "avatar";
        img.src = avatarUrl;
        img.alt = "";
        img.loading = "lazy";
        img.referrerPolicy = "no-referrer";
        return img;
    }
    const div = document.createElement("div");
    div.className = "avatar";
    div.textContent = (thread.name || "?").trim().charAt(0).toUpperCase();
    return div;
}

function sortThreads() {
    state.threads.sort(
        (a, b) =>
            Number(!!b.pinned) - Number(!!a.pinned) ||
            (b.lastMessage?.ts ?? 0) - (a.lastMessage?.ts ?? 0) ||
            a.name.localeCompare(b.name),
    );
}

function renderThreads() {
    const query = $("search").value.trim().toLowerCase();
    const list = $("threads");
    list.replaceChildren();
    for (const t of state.threads) {
        if (query && !t.name.toLowerCase().includes(query)) continue;
        // Without a search, hide contacts we have never talked to.
        if (!query && !t.lastMessage) continue;

        const li = document.createElement("li");
        li.classList.toggle("active", isCurrent(t.kind, t.id));
        li.classList.toggle("has-unread", t.unread > 0);
        li.append(avatarEl(t));

        const name = document.createElement("span");
        name.className = "name";
        name.textContent = (t.kind === "group" ? "👥 " : "") + t.name;
        li.append(name);

        const badge = document.createElement("span");
        if (t.unread > 0) {
            badge.className = "unread";
            badge.textContent = t.unread > 99 ? "99+" : String(t.unread);
        }
        li.append(badge);

        const preview = document.createElement("span");
        preview.className = "preview";
        if (t.lastMessage) {
            const who = t.lastMessage.isSelf ? "You: " : t.kind === "group" ? `${t.lastMessage.fromName}: ` : "";
            preview.textContent = who + t.lastMessage.text;
        }
        li.append(preview);

        li.addEventListener("click", () => {
            openThread(t.kind, t.id).catch((err) => alert(err.message));
        });
        list.append(li);
    }
}

function renderMessages() {
    const list = $("messages");
    const nearBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 80;
    list.replaceChildren(...state.messages.map(messageEl));
    if (nearBottom || list.dataset.thread !== currentKey()) list.scrollTop = list.scrollHeight;
    list.dataset.thread = currentKey();
}

function messageEl(m) {
    const li = document.createElement("li");
    li.classList.toggle("mine", m.isSelf);

    if (m.threadKind === "group" && !m.isSelf) {
        const from = document.createElement("span");
        from.className = "from";
        from.textContent = m.fromName;
        li.append(from);
    }

    if (m.quote) {
        const quote = document.createElement("span");
        quote.className = "quote";
        quote.textContent = `↩ ${m.quote.fromName}: ${m.quote.text}`;
        li.append(quote);
    }
    li.classList.toggle("recalled", !!m.recalled);
    li.classList.toggle("mentions-me", !!m.mentionsMe);

    const href = safeUrl(m.attachment?.href);
    const photoSrc = safeUrl(m.attachment?.thumb) || href;
    if (m.msgType === "chat.photo" && photoSrc) {
        const img = document.createElement("img");
        img.src = photoSrc;
        img.alt = "Photo";
        img.loading = "lazy";
        img.referrerPolicy = "no-referrer";
        li.append(img);
    } else if (href) {
        const a = document.createElement("a");
        a.href = href;
        a.target = "_blank";
        a.rel = "noopener noreferrer";
        a.textContent = m.text;
        li.append(a);
    } else {
        li.append(document.createTextNode(m.text));
    }

    if (m.reactions?.length) {
        const reactions = document.createElement("span");
        reactions.className = "reactions";
        reactions.textContent = m.reactions.map((r) => (r.count > 1 ? `${r.emoji}${r.count}` : r.emoji)).join(" ");
        li.append(reactions);
    }

    const time = document.createElement("span");
    time.className = "time";
    time.textContent = new Date(m.ts).toLocaleString([], { hour: "2-digit", minute: "2-digit", day: "numeric", month: "short" });
    li.append(time);
    return li;
}

function safeUrl(url) {
    if (typeof url !== "string" || !url) return null;
    try {
        const u = new URL(url);
        return u.protocol === "https:" || u.protocol === "http:" ? u.href : null;
    } catch {
        return null;
    }
}

// ---- Data ----

const currentKey = () => (state.current ? `${state.current.kind}:${state.current.id}` : "");
const isCurrent = (kind, id) => state.current?.kind === kind && state.current?.id === id;
let openSeq = 0;

function mergeMessages(fetched) {
    const byId = new Map(fetched.map((m) => [m.id, m]));
    for (const m of state.messages) {
        if (!byId.has(m.id)) byId.set(m.id, m);
    }
    return [...byId.values()].sort((a, b) => a.ts - b.ts);
}

async function loadThreads() {
    state.threads = await api("GET", "/threads");
    renderThreads();
}

async function openThread(kind, id) {
    const seq = ++openSeq;
    state.current = { kind, id };
    state.messages = [];
    $("app").classList.add("chat-open");
    const thread = state.threads.find((t) => t.kind === kind && t.id === id);
    $("chat-title").textContent = thread?.name || id;
    renderThreads();
    renderMessages();

    const data = await api("GET", `/threads/${kind}/${encodeURIComponent(id)}/messages?limit=200`);
    if (seq !== openSeq || !isCurrent(kind, id)) return;
    state.messages = mergeMessages(data.messages);
    if (data.thread) $("chat-title").textContent = data.thread.name;
    renderMessages();
    await markRead().catch(() => {});
    $("text").focus();
}

async function refreshCurrentThread() {
    if (!state.current) return;
    const { kind, id } = state.current;
    const seq = openSeq;
    const data = await api("GET", `/threads/${kind}/${encodeURIComponent(id)}/messages?limit=200`);
    if (seq !== openSeq || !isCurrent(kind, id)) return;
    state.messages = mergeMessages(data.messages);
    if (data.thread) $("chat-title").textContent = data.thread.name;
    renderMessages();
}

async function markRead() {
    if (!state.current) return;
    const { kind, id } = state.current;
    await api("POST", `/threads/${kind}/${encodeURIComponent(id)}/read`);
    const t = state.threads.find((t) => t.kind === kind && t.id === id);
    if (t) t.unread = 0;
    renderThreads();
}

function onIncoming(m, notify, threadName) {
    let t = state.threads.find((t) => t.kind === m.threadKind && t.id === m.threadId);
    if (!t) {
        // A thread we have not seen yet: refresh to get its name.
        void loadThreads().catch(() => {});
    } else if (!t.lastMessage || m.ts >= t.lastMessage.ts) {
        t.lastMessage = m;
        if (!m.isSelf && !(isCurrent(m.threadKind, m.threadId) && document.hasFocus())) t.unread++;
        sortThreads();
        renderThreads();
    }

    if (isCurrent(m.threadKind, m.threadId) && !state.messages.some((x) => x.id === m.id)) {
        state.messages.push(m);
        state.messages.sort((a, b) => a.ts - b.ts);
        renderMessages();
        if (document.hasFocus()) void markRead().catch(() => {});
    } else if (notify && !m.isSelf && document.hidden && "Notification" in window && Notification.permission === "granted") {
        new Notification(threadName || m.fromName || "Zalo", { body: m.text, tag: `${m.threadKind}:${m.threadId}` });
    }
}

function onUpdated(m) {
    if (isCurrent(m.threadKind, m.threadId)) {
        state.messages = state.messages.map((x) => (x.id === m.id ? m : x));
        renderMessages();
    }
    const t = state.threads.find((t) => t.lastMessage?.id === m.id);
    if (t) {
        t.lastMessage = m;
        renderThreads();
    }
}

// ---- Live updates ----

const WS_BACKOFF_MIN_MS = 1000;
const WS_BACKOFF_MAX_MS = 30000;
let wsBackoffMs = WS_BACKOFF_MIN_MS;
let wsTimer = null;
let activeWs = null;
let authStopped = false;
let hasConnectedOnce = false;

function showUnauthorized(message) {
    $("login").hidden = false;
    $("app").hidden = true;
    $("login-btn").hidden = true;
    $("login-error").textContent = message || "Open this page with ?token=YOUR_ACCESS_TOKEN";
}

function scheduleReconnect() {
    if (authStopped || wsTimer) return;
    const delay = wsBackoffMs;
    wsBackoffMs = Math.min(wsBackoffMs * 2, WS_BACKOFF_MAX_MS);
    wsTimer = setTimeout(() => {
        wsTimer = null;
        connect();
    }, delay);
}

function connect() {
    if (authStopped) return;
    if (activeWs && (activeWs.readyState === WebSocket.OPEN || activeWs.readyState === WebSocket.CONNECTING)) return;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${proto}://${location.host}/ws${token ? `?token=${encodeURIComponent(token)}` : ""}`);
    activeWs = ws;
    let opened = false;

    ws.addEventListener("open", () => {
        opened = true;
        wsBackoffMs = WS_BACKOFF_MIN_MS;
        if (hasConnectedOnce && state.status && ["ready", "disconnected"].includes(state.status.state)) {
            void loadThreads().catch(() => {});
            void refreshCurrentThread().catch(() => {});
        }
        hasConnectedOnce = true;
    });

    ws.addEventListener("message", (ev) => {
        let event;
        try {
            event = JSON.parse(ev.data);
        } catch {
            return;
        }
        if (event.type === "status") {
            const wasLoggedIn = state.status && ["ready", "disconnected"].includes(state.status.state);
            state.status = event.status;
            renderStatus();
            if (event.status.state === "ready" && !wasLoggedIn) void loadThreads().catch(() => {});
        } else if (event.type === "message") {
            onIncoming(event.message, event.notify, event.threadName);
        } else if (event.type === "read") {
            const t = state.threads.find((t) => t.kind === event.threadKind && t.id === event.threadId);
            if (t && t.unread !== 0) {
                t.unread = 0;
                renderThreads();
            }
        } else if (event.type === "message_updated") {
            onUpdated(event.message);
        } else if (event.type === "message_deleted") {
            if (isCurrent(event.threadKind, event.threadId)) {
                state.messages = state.messages.filter((m) => m.id !== event.id);
                renderMessages();
            }
            void loadThreads().catch(() => {});
        } else if (event.type === "threads") {
            void loadThreads().catch(() => {});
        }
    });

    ws.addEventListener("close", () => {
        if (activeWs === ws) activeWs = null;
        if (authStopped) return;
        if (!opened) {
            api("GET", "/status")
                .then((s) => {
                    state.status = s;
                    renderStatus();
                    scheduleReconnect();
                })
                .catch((err) => {
                    if (err.status === 401) {
                        authStopped = true;
                        showUnauthorized();
                    } else {
                        scheduleReconnect();
                    }
                });
            return;
        }
        scheduleReconnect();
    });
}

// ---- Wiring ----

$("login-btn").addEventListener("click", async () => {
    $("login-error").textContent = "";
    try {
        state.status = await api("POST", "/login");
        authStopped = false;
        renderStatus();
        connect();
        if ("Notification" in window && Notification.permission === "default") void Notification.requestPermission();
    } catch (err) {
        $("login-error").textContent = err.message;
    }
});

$("logout-btn").addEventListener("click", async () => {
    if (!confirm("Log out and delete the saved session and message cache on the server?")) return;
    try {
        state.status = await api("POST", "/logout");
        state.threads = [];
        state.current = null;
        state.messages = [];
        renderStatus();
    } catch (err) {
        alert(err.message);
    }
});

$("reconnect-btn").addEventListener("click", () => api("POST", "/reconnect").catch((err) => alert(err.message)));

$("back-btn").addEventListener("click", () => {
    openSeq++;
    state.current = null;
    $("app").classList.remove("chat-open");
    renderThreads();
});

$("search").addEventListener("input", renderThreads);

$("composer").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const text = $("text").value.trim();
    if (!text || !state.current) return;
    const { kind, id } = state.current;
    $("text").value = "";
    try {
        await api("POST", `/threads/${kind}/${encodeURIComponent(id)}/messages`, { text });
    } catch (err) {
        if (!$("text").value) $("text").value = text;
        alert(`Could not send: ${err.message}`);
    }
});

$("text").addEventListener("keydown", (ev) => {
    if (ev.key === "Enter" && !ev.shiftKey && !ev.isComposing) {
        ev.preventDefault();
        $("composer").requestSubmit();
    }
});

window.addEventListener("focus", () => void markRead().catch(() => {}));

api("GET", "/status")
    .then((s) => {
        state.status = s;
        renderStatus();
        if (s.state === "ready" || s.state === "disconnected") void loadThreads().catch(() => {});
        connect();
    })
    .catch((err) => {
        if (err.status === 401 || err.message === "Unauthorized") {
            authStopped = true;
            showUnauthorized();
        } else {
            showUnauthorized(err.message);
            scheduleReconnect();
        }
    });
