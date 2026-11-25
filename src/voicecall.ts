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
    return typeof first === "string" && /^https:\/\//.test(first) ? first.replace(/\/+$/, "") : DEFAULT_VOICE_CALL_DOMAIN;
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
        // Zalo PC's renderer adds these whenever the call program hands it extendData, which it always does here; without
        // them Zalo ends the call but never tells the caller or the other devices.
        extendData: JSON.stringify(ourExtendData(call)),
        subCommand: 3,
    });
}

/** The codec Zalỏ answers with: Opus 16 kHz mono, payload type 112, 20 ms frames (one of the caller's offers). */
export const ANSWER_CODEC = '[{"dynamicFptime":0,"frmPtime":20,"name":"opus/16000/1","payload":112}]';

/**
 * Picks up (signal 402, status 0, Zalo PC's `ZCallInfo::_trySendAcceptSignal`): sent once the media server accepted us,
 * with the codec we'll send, that media server and our extendData.
 */
export function acceptCall(send: VoiceCallSender, call: RingingCall, server: string): Promise<unknown> {
    const d = call.data;
    const params = jsonObject(d.params);
    return send("answer", {
        callerId: call.callerId,
        callId: call.rawCallId,
        status: AnswerStatus.Accept,
        codec: ANSWER_CODEC,
        rtcpAddress: server,
        rtpAddress: server,
        session_id: text(d.session) || text(params?.sessId),
        extendData: JSON.stringify(ourExtendData(call)),
        subCommand: 3,
    });
}

/**
 * Tells the other side our camera / microphone state (signal 12006, `/api/voicecall/conf`; the Zalos project's engine
 * does the same): `isOnCam` 1 on / 0 off, `isOnMic` 1 muted / 0 unmuted, -1 for "unchanged".
 */
export function camMicState(send: VoiceCallSender, call: RingingCall, state: { camOn?: boolean; muted?: boolean }): Promise<unknown> {
    return send("conf", {
        calleeId: call.callerId,
        callId: call.rawCallId,
        isOnCam: state.camOn === undefined ? -1 : state.camOn ? 1 : 0,
        isOnMic: state.muted === undefined ? -1 : state.muted ? 1 : 0,
        data: "",
    });
}

/** Hangs up a call that was picked up (signal 409, `/api/voicecall/endcall`). */
export function hangUp(send: VoiceCallSender, call: RingingCall, imei: string): Promise<unknown> {
    return send("endcall", { uidTo: call.callerId, callId: call.rawCallId, status: 3, imei });
}

const MIN_SRTP_SESSION_BYTES = 30;

/** What joining the call's media server needs, from the request: 32-bit zrtc ids, call id, session and server. */
export function mediaParamsOf(
    call: RingingCall,
): { server: string; userId: number; partnerId: number; callId: number; session: string; srtpKey?: string } | null {
    const d = call.data;
    const params = jsonObject(d.params);
    const zrtcConfig = jsonObject(params?.zrtc_config) ?? jsonObject(d.zrtc_config);
    const server = text(d.rtpAddress) || text(params?.rtpIP);
    const userId = Number(params?.userId ?? d.uidTo);
    const partnerId = Number(d.uidFrom);
    const callId = Number(call.rawCallId);
    const rawSrtpKey = text(zrtcConfig?.srtpKey);
    const srtpKey =
        rawSrtpKey && (Buffer.from(rawSrtpKey, "utf8").length >= MIN_SRTP_SESSION_BYTES || Buffer.from(rawSrtpKey, "base64").length >= MIN_SRTP_SESSION_BYTES)
            ? rawSrtpKey
            : undefined;
    const session = text(d.session) || text(params?.sessId) || srtpKey || "";
    if (
        !server ||
        !session ||
        Buffer.from(session, "utf8").length < MIN_SRTP_SESSION_BYTES ||
        Buffer.from(session, "latin1").length < MIN_SRTP_SESSION_BYTES ||
        ![userId, partnerId, callId].every((n) => Number.isInteger(n) && n > 0 && n <= 0xffffffff)
    ) {
        return null;
    }
    return { server, userId, partnerId, callId, session, ...(srtpKey ? { srtpKey } : {}) };
}

/** Zalo PC's call platform (`zrtc::Utility::getPlatform`); phones send 1. */
export const ZRTC_PLATFORM_PC = 2;

/**
 * Our side's `extendData`, with the fields Zalo PC's `CallController::getExtendData` writes: the call type (a video call
 * stays one even answered without a camera, as Zalo PC's plain Accept does), platform, packet / congestion-control
 * modes, only the SRTP mode we actually implement (`srtpMode: 1`, `srtcp: 0`), no P2P candidates, the call's media
 * server, and the video codec list.
 */
export function ourExtendData(call: RingingCall): Record<string, unknown> {
    const params = jsonObject(call.data.params);
    const theirs = jsonObject(params?.extendData) ?? {};
    const num = (k: string, fallback: number) => (typeof theirs[k] === "number" ? (theirs[k] as number) : fallback);
    const servers = Array.isArray(theirs.serverAddr) ? theirs.serverAddr.slice(0, 1) : [];
    return {
        p2p: [],
        callType: call.video ? 1 : 0,
        fecTP: 0,
        gccAudio: num("gccAudio", 1),
        gccEarlyCall: 0,
        gccMode: num("gccMode", 1),
        gccSVLR: num("gccSVLR", 1),
        maxFT: num("maxFT", 60),
        newZrtc: 1,
        numServers: 0,
        packetMode: num("packetMode", 2),
        platform: ZRTC_PLATFORM_PC,
        sP2P: 0,
        select2side: 1,
        spTcp: 1,
        srtcp: 0,
        srtpMode: 1,
        tpType: 0,
        supportCallBusy: 1,
        supportHevcDecode: 0,
        serverAddr: servers,
        // Both official apps always list their video codec, voice calls included. Answering a video call without it
        // crashed the caller's Android app; H.264 only (no HEVC), like supportHevcDecode says.
        video: { codec: [{ name: "h264", payload: 97 }] },
    };
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
