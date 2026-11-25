// Polls: turning Zalo's poll detail into what the app shows, and spotting poll updates in group events.
// The view rules (hidden results before voting, expired = closed) follow Zalos' poll block.
import type { PollDetail } from "zca-js";

export type PollVoter = { id: string; name: string };
export type PollOptionView = {
    id: number;
    text: string;
    votes: number;
    voted: boolean;
    /** Empty for anonymous polls and while results are hidden. */
    voters: PollVoter[];
};

/** What `GET /api/polls/:id` returns. */
export type PollView = {
    pollId: number;
    question: string;
    creatorId: string;
    creatorName: string;
    /** Closed by the creator or past its deadline: no more voting. */
    closed: boolean;
    expired: boolean;
    /** Epoch ms of the deadline, 0 for none. */
    expiresAt: number;
    createdAt: number;
    allowMultiple: boolean;
    allowAddOptions: boolean;
    anonymous: boolean;
    /** The poll hides counts until you vote (and it is still open): options carry no counts or voters. */
    resultsHidden: boolean;
    /** Number of votes cast (a person choosing two options counts twice), as Zalo reports it. */
    totalVotes: number;
    /** People who voted at all. */
    voterCount: number;
    myVotes: number[];
    canVote: boolean;
    canAddOptions: boolean;
    /** Creator or group admin: may close the poll. */
    canClose: boolean;
    options: PollOptionView[];
};

export type PollViewContext = {
    ownId: string | null;
    isAdmin: boolean;
    now: number;
    /** User id -> display name, for voters and the creator. */
    names?: Map<string, string>;
};

export function buildPollView(d: PollDetail, ctx: PollViewContext): PollView {
    const expiresAt = Number(d.expired_time) > 0 ? Number(d.expired_time) : 0;
    const expired = expiresAt > 0 && expiresAt <= ctx.now;
    const closed = Boolean(d.closed) || expired;
    const options = Array.isArray(d.options) ? d.options : [];
    const myVotes = options.filter((o) => o.voted).map((o) => Number(o.option_id));
    const hasVoted = myVotes.length > 0 || Boolean(d.joined);
    const resultsHidden = Boolean(d.is_hide_vote_preview) && !hasVoted && !closed;
    const anonymous = Boolean(d.is_anonymous);
    const nameOf = (id: string) => ctx.names?.get(id) ?? id;
    const everyone = new Set<string>();
    const views = options.map<PollOptionView>((o) => {
        const voters = (o.voters ?? []).map(String);
        voters.forEach((v) => everyone.add(v));
        return {
            id: Number(o.option_id),
            text: o.content,
            votes: resultsHidden ? 0 : Number(o.votes) || 0,
            voted: Boolean(o.voted),
            voters: anonymous || resultsHidden ? [] : voters.map((id) => ({ id, name: nameOf(id) })),
        };
    });
    const creatorId = String(d.creator ?? "");
    const isCreator = !!ctx.ownId && creatorId === ctx.ownId;
    return {
        pollId: Number(d.poll_id),
        question: d.question,
        creatorId,
        creatorName: nameOf(creatorId),
        closed,
        expired,
        expiresAt,
        createdAt: Number(d.created_time) || 0,
        allowMultiple: Boolean(d.allow_multi_choices),
        allowAddOptions: Boolean(d.allow_add_new_option),
        anonymous,
        resultsHidden,
        totalVotes: resultsHidden ? 0 : Number(d.num_vote) || 0,
        voterCount: resultsHidden ? 0 : everyone.size,
        myVotes,
        canVote: !closed,
        canAddOptions: !closed && Boolean(d.allow_add_new_option),
        canClose: !closed && (isCreator || ctx.isAdmin),
        options: views,
    };
}

/**
 * The poll a group event is about, if any. Poll changes arrive as board events whose topic is a poll
 * (`type` 3) and whose `params` is a JSON string (or object) holding `pollId`.
 */
export function pollIdOfGroupEvent(e: { type?: unknown; data?: unknown }): number | null {
    const topic = (e.data as { groupTopic?: { type?: unknown; params?: unknown } } | undefined)?.groupTopic;
    if (!topic || Number(topic.type) !== 3) return null;
    let params = topic.params;
    if (typeof params === "string") {
        try {
            params = JSON.parse(params);
        } catch {
            return null;
        }
    }
    const id = Number((params as { pollId?: unknown; poll_id?: unknown } | null)?.pollId ?? (params as { poll_id?: unknown } | null)?.poll_id);
    return Number.isFinite(id) && id > 0 ? id : null;
}

export type CreatePollInput = {
    question: string;
    options: string[];
    allowMultiple: boolean;
    allowAddOptions: boolean;
    anonymous: boolean;
    hideVotePreview: boolean;
    /** Epoch ms, 0 for none. */
    expiresAt: number;
};
