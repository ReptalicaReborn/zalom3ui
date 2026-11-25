import type { API } from "zca-js";
import { jsonObject, type CallControl, type IncomingCall } from "./calls.js";

/**
 * Zalo's call signalling: GET requests to the voice call service, `/api/voicecall/<endpoint>?params=<AES(json)>`, the
 * same encryption zca-js uses for every other API (docs/calls.md lists the endpoints and their parameters).
 */

/** Sends one voice call request and returns Zalo's answer (`data`). */
export type VoiceCallSender = (endpoint: string, params: Record<string, unknown>) => Promise<unknown>;

/** Voice call service used when the login answer names none (Zalo PC's default). */
export const DEFAULT_VOICE_CALL_DOMAIN = "https://voicecall-wpa.chat.zalo.me";

/** The voice call service of a login's service map (`voice_call`: a list of URLs, or one). */
export function voiceCallDomain(serviceMap: unknown): string {
    const entry = (serviceMap as { voice_call?: unknown } | null | undefined)?.voice_call;
    const first = Array.isArray(entry) ? entry[0] : entry;
    return typeof first === "string" && /^https?:\/\//.test(first) ? first.replace(/\/+$/, "") : DEFAULT_VOICE_CALL_DOMAIN;
}

/** A call ringing on this account, with what answering or declining it needs. */
export type RingingCall = IncomingCall & {
    /** The call id as Zalo PC's call program sends it back: a number (it fits in 32 bits). */
    rawCallId: number | string;
    /** Zalo PC's call type for an incoming call: 1 voice, 3 video. */
    callType: number;
    /** The request's full data, for answering later (media servers, codec, session). */
    data: Record<string, unknown>;
    receivedAt: number;
};

export function ringingCallOf(incoming: IncomingCall, control: CallControl, now = Date.now()): RingingCall {
    const d = control.data;
    const id = Number(d.callId ?? d.call_id ?? incoming.callId);
    return {
        ...incoming,
        rawCallId: Number.isSafeInteger(id) ? id : incoming.callId,
        // ZCallInfo::_updateCallInfo(ZIncomingCallInfo): video ? 3 : 1.
        callType: incoming.video ? 3 : 1,
        data: d,
        receivedAt: now,
    };
}

/** Status of an `answer` (Zalo PC's call program, `ZCallInfo`): picked up, busy, declined. */
export const AnswerStatus = { Accept: 0, Busy: 1, Decline: 3 } as const;

/**
 * Declines a ringing call. Zalo PC's call program sends signal 402 with status 3 when the incoming call is declined or
 * its window closed (`ZCallInfo::UiAnswerCallEvent` / `UiCloseEvent`), which the renderer turns into
 * `/api/voicecall/answer`: the caller's noised id, the call id as a number, the status, the codec list, the media
 * server and the session. (`cancel` is the caller giving up; sent by the person called, Zalo accepts it but the caller
 * keeps ringing.) The other devices then get the `answer` control with status 3, as when declining in the official app.
 */
export function declineCall(send: VoiceCallSender, call: RingingCall): Promise<unknown> {
    const d = call.data;
    const params = jsonObject(d.params);
    const server = text(d.rtpAddress) || text(params?.rtpIP);
    return send("answer", {
        callerId: call.callerId,
        callId: call.rawCallId,
        status: AnswerStatus.Decline,
        codec: text(d.codec),
        rtcpAddress: text(d.rtcpAddress) || text(params?.rtcpIP) || server,
        rtpAddress: server,
        session_id: text(d.session) || text(params?.sessId),
    });
}

function text(v: unknown): string {
    return typeof v === "string" ? v : typeof v === "number" ? String(v) : "";
}

/**
 * Tells the caller this device is ringing (signal 407, `/api/voicecall/ringring`). Zalo PC's call program sends it as
 * soon as a call arrives.
 */
export function ringRing(send: VoiceCallSender, call: RingingCall, imei: string): Promise<unknown> {
    return send("ringring", { callerId: call.callerId, callId: call.rawCallId, status: 0, imei });
}

/** A [VoiceCallSender] on a logged-in zca-js session. */
export function voiceCallSender(api: API): VoiceCallSender {
    const name = "zalooVoiceCall";
    if (!(name in api)) {
        api.custom<unknown, { endpoint: string; params: Record<string, unknown> }>(name, async ({ utils, props }) => {
            const base = `${voiceCallDomain((api as unknown as { zpwServiceMap?: unknown }).zpwServiceMap)}/api/voicecall/${props.endpoint}`;
            const params = utils.encodeAES(JSON.stringify(props.params));
            if (!params) throw new Error("Could not encrypt the call request");
            const response = await utils.request(utils.makeURL(base, { params }), { method: "GET" });
            return utils.resolve(response);
        });
    }
    const call = (api as unknown as Record<string, (p: { endpoint: string; params: Record<string, unknown> }) => Promise<unknown>>)[name];
    return (endpoint, params) => call({ endpoint, params });
}
