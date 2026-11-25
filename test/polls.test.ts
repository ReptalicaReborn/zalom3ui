import assert from "node:assert/strict";
import os from "node:os";
import { test } from "node:test";
import type { PollDetail } from "zca-js";
import { buildPollView, pollIdOfGroupEvent } from "../src/polls.js";
import { ZaloSession } from "../src/session.js";
import { ChatStore, normalizeMessage, parseAttachment, type RawMessage } from "../src/store.js";

function detail(extra: Partial<PollDetail> = {}): PollDetail {
    return {
        creator: "alice",
        question: "Lunch?",
        options: [
            { option_id: 1, content: "Pho", votes: 2, voted: true, voters: ["me", "bob"] },
            { option_id: 2, content: "Bun", votes: 1, voted: false, voters: ["carol"] },
        ],
        joined: true,
        closed: false,
        poll_id: 55,
        allow_multi_choices: false,
        allow_add_new_option: true,
        is_anonymous: false,
        poll_type: 0,
        created_time: 1000,
        updated_time: 1000,
        expired_time: 0,
        is_hide_vote_preview: false,
        num_vote: 3,
        ...extra,
    };
}

const ctx = { ownId: "me", isAdmin: false, now: 5000 };

test("a poll message becomes a poll attachment, with the id and question from content or params", () => {
    assert.deepEqual(parseAttachment("group.poll", { poll_id: 55, question: "Lunch?" }), {
        kind: "poll",
        title: "Lunch?",
        poll: { pollId: 55, question: "Lunch?" },
    });
    const viaParams = parseAttachment("group.poll", { action: "vote", params: JSON.stringify({ pollId: "56", question: "Q" }) });
    assert.deepEqual(viaParams?.poll, { pollId: 56, question: "Q", event: "vote" });
    assert.equal(parseAttachment("group.poll", { question: "no id" }), undefined);
});

test("a poll message reads as [Poll] in the chat list", () => {
    const raw: RawMessage = {
        type: 1,
        threadId: "g1",
        isSelf: false,
        data: { msgId: "1", cliMsgId: "c1", msgType: "group.poll", uidFrom: "alice", idTo: "g1", ts: "1000", content: { poll_id: 55, question: "Lunch?" } },
    };
    const msg = normalizeMessage(raw);
    assert.equal(msg.text, "[Poll] Lunch?");
    assert.equal(msg.attachment?.poll?.pollId, 55);
});

test("the poll view shows counts, voters and my votes, and lets the creator or an admin close it", () => {
    const view = buildPollView(detail(), { ...ctx, names: new Map([["bob", "Bob"]]) });
    assert.deepEqual(view.myVotes, [1]);
    assert.equal(view.options[0].voters[1].name, "Bob");
    assert.equal(view.options[0].voters[0].name, "me");
    assert.equal(view.voterCount, 3);
    assert.equal(view.totalVotes, 3);
    assert.equal(view.canClose, false);
    assert.equal(buildPollView(detail(), { ...ctx, isAdmin: true }).canClose, true);
    assert.equal(buildPollView(detail({ creator: "me" }), ctx).canClose, true);
});

test("an anonymous poll lists no voters; hidden results show no counts until you vote", () => {
    const anon = buildPollView(detail({ is_anonymous: true }), ctx);
    assert.equal(anon.options[0].voters.length, 0);
    assert.equal(anon.options[0].votes, 2);
    const hidden = buildPollView(
        detail({ is_hide_vote_preview: true, joined: false, options: detail().options.map((o) => ({ ...o, voted: false })) }),
        ctx,
    );
    assert.equal(hidden.resultsHidden, true);
    assert.equal(hidden.totalVotes, 0);
    assert.ok(hidden.options.every((o) => o.votes === 0 && o.voters.length === 0));
});

test("a poll past its deadline is closed and cannot take votes or new options", () => {
    const view = buildPollView(detail({ expired_time: 4000 }), ctx);
    assert.equal(view.closed, true);
    assert.equal(view.expired, true);
    assert.equal(view.canVote, false);
    assert.equal(view.canAddOptions, false);
    assert.equal(buildPollView(detail({ closed: true }), ctx).canClose, false);
});

test("group events about a poll board entry give the poll id", () => {
    const event = (type: number, params: unknown) => ({ type: "update_board", data: { groupTopic: { type, params } } });
    assert.equal(pollIdOfGroupEvent(event(3, JSON.stringify({ pollId: 77, title: "x" }))), 77);
    assert.equal(pollIdOfGroupEvent(event(3, { pollId: 78 })), 78);
    assert.equal(pollIdOfGroupEvent(event(2, JSON.stringify({ pollId: 77 }))), null);
    assert.equal(pollIdOfGroupEvent(event(3, "not json")), null);
    assert.equal(pollIdOfGroupEvent({ type: "join", data: {} }), null);
});

function setup(responses: Record<string, unknown | ((...args: unknown[]) => unknown)> = {}) {
    const store = new ChatStore();
    store.ownId = "me";
    const session = new ZaloSession(store, os.tmpdir());
    const calls: { method: string; args: unknown[] }[] = [];
    const api = new Proxy(
        {},
        {
            get: (_t, method: string) =>
                async (...args: unknown[]) => {
                    calls.push({ method, args });
                    const answer = responses[method];
                    return typeof answer === "function" ? answer(...args) : (answer ?? { status: 0 });
                },
        },
    );
    const internals = session as unknown as { api: unknown; status: { state: string; ownId: string } };
    internals.api = api;
    internals.status.state = "ready";
    internals.status.ownId = "me";
    const polls: number[] = [];
    session.on("poll", (p) => polls.push(p.pollId));
    return { session, calls, polls };
}

test("creating a poll passes the options to Zalo and announces it", async () => {
    const { session, calls, polls } = setup({ createPoll: detail() });
    const view = await session.createPoll("g1", {
        question: "Lunch?",
        options: ["Pho", "Bun"],
        allowMultiple: true,
        allowAddOptions: false,
        anonymous: false,
        hideVotePreview: false,
        expiresAt: 9999,
    });
    assert.equal(view.pollId, 55);
    const [opts, groupId] = calls[0].args as [Record<string, unknown>, string];
    assert.equal(groupId, "g1");
    assert.deepEqual(opts.options, ["Pho", "Bun"]);
    assert.equal(opts.allowMultiChoices, true);
    assert.equal(opts.expiredTime, 9999);
    assert.deepEqual(polls, [55]);
});

test("voting sends the whole choice, removing a vote sends an empty list, and bad votes are refused", async () => {
    const { session, calls, polls } = setup({ getPollDetail: detail() });
    await session.votePoll(55, [2]);
    assert.deepEqual(calls.find((c) => c.method === "votePoll")?.args, [55, [2]]);
    assert.deepEqual(polls, [55]);
    await session.votePoll(55, []);
    assert.deepEqual(calls.filter((c) => c.method === "votePoll")[1].args, [55, []]);
    await assert.rejects(session.votePoll(55, [9]), /Unknown option/);
    await assert.rejects(session.votePoll(55, [1, 2]), /one choice/);
});

test("a closed poll refuses votes", async () => {
    const { session } = setup({ getPollDetail: detail({ closed: true }) });
    await assert.rejects(session.votePoll(55, [1]), /closed/);
});

test("adding options keeps what I voted for; polls that don't allow it refuse non-creators", async () => {
    const { session, calls } = setup({ getPollDetail: detail() });
    await session.addPollOptions(55, ["Com"]);
    const payload = calls.find((c) => c.method === "addPollOptions")?.args[0];
    assert.deepEqual(payload, { pollId: 55, options: [{ voted: false, content: "Com" }], votedOptionIds: [1] });
    const locked = setup({ getPollDetail: detail({ allow_add_new_option: false }) });
    await assert.rejects(locked.session.addPollOptions(55, ["Com"]), /new options/);
});

test("only the creator or a group admin closes a poll", async () => {
    const other = setup({ getPollDetail: detail() });
    await assert.rejects(other.session.lockPoll(55), /creator or a group admin/);
    const own = setup({ getPollDetail: detail({ creator: "me" }) });
    await own.session.lockPoll(55);
    assert.ok(own.calls.some((c) => c.method === "lockPoll"));
    assert.deepEqual(own.polls, [55]);
});

test("a poll vote message announces the poll", () => {
    const { session, polls } = setup();
    const ingest = (m: RawMessage) => (session as unknown as { ingest(m: unknown): void }).ingest(m);
    ingest({
        type: 1,
        threadId: "g1",
        isSelf: false,
        data: { msgId: "2", cliMsgId: "c2", msgType: "group.poll", uidFrom: "bob", idTo: "g1", ts: "2000", content: { action: "vote", poll_id: 55, question: "Lunch?" } },
    });
    assert.deepEqual(polls, [55]);
});
