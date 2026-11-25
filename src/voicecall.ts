import type { API } from "zca-js";
import type { CallControl, IncomingCall } from "./calls.js";

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
    /** The call id exactly as Zalo sent it (number or text): it is sent back the same way. */
    rawCallId: unknown;
    /** Zalo's call type as sent (`callType`), echoed when declining. */
    callType: unknown;
    /** The request's full data, for answering later (media servers, codec, session). */
    data: Record<string, unknown>;
    receivedAt: number;
};

export function ringingCallOf(incoming: IncomingCall, control: CallControl, now = Date.now()): RingingCall {
    const d = control.data;
    return {
        ...incoming,
        rawCallId: d.callId ?? d.call_id ?? incoming.callId,
        callType: d.callType ?? d.type ?? (incoming.video ? 1 : 0),
        data: d,
        receivedAt: now,
    };
}

/**
 * Declines a ringing call. Zalo PC has no separate "decline" request: its call program sends signal 405, which the
 * renderer turns into `/api/voicecall/cancel` with the other side's id, both when a caller gives up and when the
 * person being called declines.
 */
export function declineCall(send: VoiceCallSender, call: RingingCall, imei: string): Promise<unknown> {
    return send("cancel", { callerId: call.callerId, callId: call.rawCallId, callType: call.callType, status: 0, imei });
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
