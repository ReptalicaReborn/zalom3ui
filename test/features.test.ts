import assert from "node:assert/strict";
import os from "node:os";
import { test } from "node:test";
import { ThreadType } from "zca-js";
import { ZaloSession } from "../src/session.js";
import { ChatStore, type RawMessage } from "../src/store.js";

type Call = { method: string; args: unknown[] };

/** A ZaloSession wired to a fake zca-js API that records every call and answers from `responses`. */
function setup(responses: Record<string, unknown | ((...args: unknown[]) => unknown)> = {}) {
    const store = new ChatStore();
    store.ownId = "me";
    const session = new ZaloSession(store, os.tmpdir());
    const calls: Call[] = [];
    const defaults: Record<string, unknown> = {
        sendMessage: { message: null, attachment: [{ msgId: 77 }] },
        getAllFriends: [],
        getGroupInfo: { gridInfoMap: {} },
        getStickersDetail: [],
    };
    const api = new Proxy(
        {},
        {
            get: (_t, method: string) =>
                async (...args: unknown[]) => {
                    calls.push({ method, args });
                    const answer = method in responses ? responses[method] : defaults[method];
                    if (answer instanceof Error) throw answer;
                    return typeof answer === "function" ? answer(...args) : (answer ?? { status: 0 });
                },
        },
    );
    const internals = session as unknown as { api: unknown; status: { state: string; ownId: string } };
    internals.api = api;
    internals.status.state = "ready";
    internals.status.ownId = "me";
    const events: { name: string; payload: unknown }[] = [];
    for (const name of ["message", "message_updated", "threads", "read"] as const) {
        session.on(name, (payload?: unknown) => events.push({ name, payload }));
    }
    const ingest = (r: RawMessage) => (session as unknown as { ingest(m: unknown): void }).ingest(r);
    return { store, session, calls, events, ingest };
}

function incoming(id: string, ts: number, extra: Partial<RawMessage["data"]> = {}): RawMessage {
    return {
        type: 0,
        threadId: "100",
        isSelf: extra.uidFrom === "me",
        data: {
            msgId: id,
            cliMsgId: `c${id}`,
            msgType: "webchat",
            uidFrom: "100",
            idTo: "me",
            dName: "Alice",
            ts: String(ts),
            content: `text ${id}`,
            ...extra,
        },
    };
}

const tick = () => new Promise((r) => setImmediate(r));

test("a photo is sent with its pixel size, a file with only its size", async () => {
    const { session, calls } = setup();
    const data = Buffer.from([1, 2, 3, 4]);
    assert.deepEqual(await session.sendAttachment("group", "g1", { data, filename: "a.JPG", width: 640, height: 480 }), ["77"]);
    const [content, threadId, type] = calls[0].args as [{ msg: string; attachments: unknown[] }, string, ThreadType];
    assert.equal(content.msg, "");
    assert.deepEqual(content.attachments, [{ data, filename: "a.JPG", metadata: { totalSize: 4, width: 640, height: 480 } }]);
    assert.equal(threadId, "g1");
    assert.equal(type, ThreadType.Group);

    await session.sendAttachment("user", "100", { data, filename: "notes.pdf" });
    assert.deepEqual((calls[1].args[0] as { attachments: unknown[] }).attachments, [{ data, filename: "notes.pdf", metadata: { totalSize: 4 } }]);
});

test("attachments Zalo can't take here are refused before anything is sent", async () => {
    const { session, calls } = setup();
    const data = Buffer.from([1]);
    await assert.rejects(session.sendAttachment("user", "100", { data, filename: "noext" }), /extension/);
    await assert.rejects(session.sendAttachment("user", "100", { data, filename: "a.gif" }), /GIF/);
    await assert.rejects(session.sendAttachment("user", "100", { data, filename: "a.png" }), /width and height/);
    await assert.rejects(session.sendAttachment("user", "100", { data: Buffer.alloc(0), filename: "a.txt" }), /empty/);
    assert.equal(calls.length, 0);
});

test("sticker search returns images, and a found sticker can be sent", async () => {
    const { session, calls } = setup({
        getStickers: [5, 6],
        getStickersDetail: [
            { id: 5, cateId: 50, type: 7, stickerWebpUrl: "https://s/5.webp", stickerUrl: "https://s/5.png" },
            { id: 6, cateId: 60, type: 7, stickerWebpUrl: null, stickerUrl: "https://s/6.png" },
        ],
    });
    assert.deepEqual(await session.searchStickers("hi"), [
        { id: 5, catId: 50, type: 7, url: "https://s/5.webp" },
        { id: 6, catId: 60, type: 7, url: "https://s/6.png" },
    ]);
    await session.sendSticker("user", "100", 6);
    const send = calls.find((c) => c.method === "sendSticker");
    assert.deepEqual(send?.args, [{ id: 6, cateId: 60, type: 7 }, "100", ThreadType.User]);
});

test("a received sticker gets its image looked up once and the message updated", async () => {
    const { calls, events, ingest, store } = setup({
        getStickersDetail: [{ id: 9, cateId: 1, type: 7, stickerWebpUrl: "https://s/9.webp" }],
    });
    ingest(incoming("1", Date.now(), { msgType: "chat.sticker", content: { id: 9, catId: 1, type: 7 } }));
    ingest(incoming("2", Date.now(), { msgType: "chat.sticker", content: { id: 9, catId: 1, type: 7 } }));
    await tick();
    await tick();
    assert.equal(calls.filter((c) => c.method === "getStickersDetail").length, 1);
    assert.equal(store.getMessage("1")?.attachment?.thumb, "https://s/9.webp");
    assert.equal(store.getMessage("2")?.attachment?.thumb, "https://s/9.webp");
    assert.equal(events.filter((e) => e.name === "message_updated").length, 2);
});

test("notifications: not mine, not muted, not the open chat, not old history", () => {
    const { session, ingest, store } = setup();
    const now = Date.now();
    ingest(incoming("1", now));
    const msg = store.toPublic(store.getMessage("1")!);
    assert.equal(session.shouldNotify(msg, now), true);

    session.setViewing("user", "100", now);
    assert.equal(session.shouldNotify(msg, now), false);
    // A client that stopped saying so no longer counts as looking.
    assert.equal(session.shouldNotify(msg, now + 91_000), true);
    session.setViewing(null, null);

    store.setMuted("user", "100", -1);
    assert.equal(session.shouldNotify(msg, now), false);
    store.setMuted("user", "100", null);

    assert.equal(session.shouldNotify({ ...msg, isSelf: true }, now), false);
    assert.equal(session.shouldNotify({ ...msg, ts: now - 11 * 60_000 }, now), false);
});

test("reading a chat tells clients, so its notification goes away", async () => {
    const { session, events } = setup();
    await session.markRead("group", "g1");
    assert.deepEqual(events.find((e) => e.name === "read")?.payload, { threadKind: "group", threadId: "g1" });
});

test("friend requests: only received ones are listed; accepting reloads friends", async () => {
    const { session, calls } = setup({
        getFriendRecommendations: {
            recommItems: [
                { dataInfo: { userId: "7", displayName: "Bảo", avatar: "a7", recommType: 2, recommTime: 5, recommInfo: { message: "Chào" } } },
                { dataInfo: { userId: "8", displayName: "Gợi ý", avatar: "a8", recommType: 1, recommTime: 6, recommInfo: { message: "" } } },
            ],
        },
        getAllFriends: [{ userId: "7", displayName: "Bảo", zaloName: "Bao", avatar: "a7" }],
    });
    assert.deepEqual(await session.friendRequests(), [{ id: "7", name: "Bảo", avatar: "a7", message: "Chào", time: 5 }]);
    await session.acceptFriendRequest("7");
    assert.deepEqual(calls.find((c) => c.method === "acceptFriendRequest")?.args, ["7"]);
    assert.deepEqual(session.friends(), [{ id: "7", name: "Bảo", avatar: "a7" }]);
});

test("finding someone by phone says whether they are a friend or already asked", async () => {
    const { session } = setup({
        findUser: { uid: "42", display_name: "Chi", zalo_name: "chi", avatar: "a42" },
        getSentFriendRequest: { "42": { userId: "42" } },
    });
    assert.deepEqual(await session.findUser("0901234567"), { id: "42", name: "Chi", avatar: "a42", isFriend: false, requested: true });

    const missing = setup({ findUser: new Error("not found") });
    await assert.rejects(missing.session.findUser("0900000000"), /No Zalo account/);
});

test("groups: create with 2+ people, add members, rename, leave removes the chat", async () => {
    const { session, calls, store } = setup({
        createGroup: { groupId: "555", sucessMembers: ["2", "3"], errorMembers: ["4"], groupType: 0, error_data: {} },
        addUserToGroup: { errorMembers: [], error_data: {} },
    });
    await assert.rejects(session.createGroup("x", ["2", "me"]), /at least 2/);
    assert.deepEqual(await session.createGroup("  Nhóm bạn  ", ["2", "3", "4", "2"]), { groupId: "555", failed: ["4"] });
    assert.deepEqual(calls.find((c) => c.method === "createGroup")?.args, [{ name: "Nhóm bạn", members: ["2", "3", "4"] }]);
    assert.equal(store.getThread("group", "555")?.name, "Nhóm bạn");

    assert.deepEqual(await session.addGroupMembers("555", ["5", "me"]), { failed: [] });
    assert.deepEqual(calls.find((c) => c.method === "addUserToGroup")?.args, [["5"], "555"]);

    await session.renameGroup("555", "Tên mới");
    assert.deepEqual(calls.find((c) => c.method === "changeGroupName")?.args, ["Tên mới", "555"]);
    assert.equal(store.getThread("group", "555")?.name, "Tên mới");

    await session.leaveGroup("555", true);
    assert.deepEqual(calls.find((c) => c.method === "leaveGroup")?.args, ["555", true]);
    assert.equal(store.getThread("group", "555"), undefined);
});

test("removing a member Zalo refuses says why", async () => {
    const { session } = setup({ removeUserFromGroup: { errorMembers: ["5"] } });
    await assert.rejects(session.removeGroupMember("555", "5"), /owner and deputies/);
});
