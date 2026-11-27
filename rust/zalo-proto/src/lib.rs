//! Pure-Rust Zalo Web protocol client (port of used `zca-js` v2.2.0 surface + OA/call/sync frame decoding).

pub mod api;
pub mod cookies;
pub mod crypto;
pub mod error;
pub mod framelog;
pub mod http;
pub mod listener;
pub mod login;
pub mod time_util;
pub mod types;

pub use api::ctrlpoll::{
    controls_in, poll_cursor, read_poll_answer, server_list, ControlPollEvent, ControlPoller,
    ControlPollerHandle, PollAnswer,
};
pub use api::messaging::reaction_type_id;
pub use api::polls::{build_poll_view, poll_id_of_group_event};
pub use api::voicecall::{
    incoming_call_of, media_params_of, new_call_id, our_extend_data, outgoing_call_of,
    outgoing_media_of, redact_for_trace, ringing_call_of, voice_call_domain, AnswerStatus,
    IncomingCall, MediaParams, OfferedServer, OutgoingMedia, RingingCall, VoiceCallApi,
    ANSWER_CODEC, CALL_ENDING_ACTS, DEFAULT_VOICE_CALL_DOMAIN, RINGING_MAX_AGE_MS,
};
pub use api::{parse_noised_ids_reply, NoisedIdsReply, ZaloClient};
pub use cookies::{is_zalo_cookie_host, SavedCookie, ZaloCookieJar};
pub use crypto::{
    decode_aes, decode_aes_utf8_key, decode_event_payload, decode_event_payload_with_cipher,
    encode_aes, encode_aes_utf8_key, generate_zalo_uuid, get_sign_key, inflate_any, md5_hex,
    parse_json_keeping_bigints, AesOutputEncoding, EventCipher, ParamsEncryptor,
};
pub use error::{ProtoError, Result};
pub use http::{handle_zalo_response, ZaloHttp};
pub use listener::{
    decode_frame_header, dispatch_frame, dispatch_frame_traced, encode_frame, CloseReason, ListenerHandle, ZaloListener,
};
pub use login::{login_cookie, login_cookie_with_http, login_qr, DEFAULT_USER_AGENT};
pub use types::{
    AddPollOptionsInput, AvatarSize, CreateGroupOptions, CreatePollInput, Credentials,
    DeleteMessageTarget, DestType, FileDonePayload, ForwardMessagePayload, ForwardReference,
    LastMessageInfo, LoginQrEvent, MentionInput, MuteAction, MuteDuration, PollOptionView,
    PollView, PollViewContext, PollVoter, QuoteInput, RawWsMessage, ReactionTarget,
    SeenMessageInput, SendMessageOptions, SendMessageResponse, StickerParam, ThreadType,
    UndoTarget, UploadProgress, UploadProgressSink, UploadResult, UploadSource, WsEvent, ZaloContext, DEFAULT_API_TYPE,
    DEFAULT_API_VERSION,
};
