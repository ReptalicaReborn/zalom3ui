//! Seam between [`crate::session::ZaloSession`] and the Zalo protocol client.
//!
//! The session talks to Zalo through [`ZaloApi`] (every client method it uses, plus starting the
//! WebSocket listener) and logs in through [`ZaloLogin`]. Production wires both to
//! `zalo_proto::ZaloClient` / `login_qr` / `login_cookie` ([`ProtoApi`], [`ProtoLogin`]); tests use
//! the recording fake in [`crate::testing`].
//!
//! The method list lives once in [`with_api_methods!`], which expands into the trait, the
//! production delegation, and the fake's implementation.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use zalo_proto::{
    AddPollOptionsInput, AvatarSize, CreateGroupOptions, CreatePollInput, Credentials,
    DeleteMessageTarget, DestType, ForwardMessagePayload, LastMessageInfo, ListenerHandle,
    LoginQrEvent, MuteAction, MuteDuration, NoisedIdsReply, ReactionTarget, Result as ProtoResult,
    SeenMessageInput, SendMessageOptions, SendMessageResponse, StickerParam, ThreadType,
    UndoTarget, UploadResult, UploadSource, WsEvent, ZaloClient, ZaloHttp,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Controls a running chat-socket listener (what the session needs from `ListenerHandle`).
pub trait ListenerControl: Send + Sync {
    /// Asks Zalo to replay old messages; `last_id` is the `lastId` of the request (`None`: null).
    fn request_old_messages(&self, thread_type: ThreadType, last_id: Option<String>);
    fn request_old_reactions(&self, thread_type: ThreadType);
    /// Gracefully stops the listener (WebSocket close code 1000).
    fn stop(&self);
}

/// A started listener: its event stream and its control handle.
pub struct ListenerStart {
    pub events: mpsc::UnboundedReceiver<WsEvent>,
    pub control: Arc<dyn ListenerControl>,
}

/// [`ListenerControl`] over the real `zalo_proto` listener handle.
pub struct ProtoListenerControl(pub ListenerHandle);

impl ListenerControl for ProtoListenerControl {
    fn request_old_messages(&self, thread_type: ThreadType, last_id: Option<String>) {
        let _ = self.0.request_old_messages(thread_type, last_id);
    }

    fn request_old_reactions(&self, thread_type: ThreadType) {
        let _ = self.0.request_old_reactions(thread_type, None);
    }

    fn stop(&self) {
        self.0.stop();
    }
}

/// Expands `$callback! { <method list> }` with every client method `ZaloSession` uses.
///
/// Each entry is `name(arg: type, ...) -> ok_type;` and becomes
/// `fn name<'a>(&'a self, ...) -> BoxFuture<'a, zalo_proto::Result<ok_type>>`; reference
/// arguments are written with the shared lifetime `'a`.
#[macro_export]
#[doc(hidden)]
macro_rules! with_api_methods {
    ($callback:ident) => {
        $callback! {
            // account
            fetch_account_info() -> Value;
            // conversations
            add_unread_mark(thread_id: &'a str, thread_type: ThreadType) -> Value;
            remove_unread_mark(thread_id: &'a str, thread_type: ThreadType) -> Value;
            set_pinned_conversations(pinned: bool, thread_ids: &'a [String], thread_type: ThreadType) -> Value;
            get_pin_conversations() -> Value;
            set_mute(action: MuteAction, duration: MuteDuration, thread_id: &'a str, thread_type: ThreadType) -> Value;
            get_mute() -> Value;
            delete_chat(last_message: &'a LastMessageInfo, thread_id: &'a str, thread_type: ThreadType) -> Value;
            // friends
            get_all_friends(count: Option<u32>, page: Option<u32>, avatar_size: Option<AvatarSize>) -> Value;
            get_user_info(user_ids: &'a [String], avatar_size: Option<AvatarSize>) -> Value;
            last_online(uid: &'a str) -> Value;
            find_user(phone_number: &'a str, avatar_size: Option<AvatarSize>) -> Value;
            get_friend_recommendations() -> Value;
            get_sent_friend_request() -> Value;
            accept_friend_request(friend_id: &'a str) -> Value;
            reject_friend_request(friend_id: &'a str) -> Value;
            send_friend_request(msg: &'a str, user_id: &'a str) -> Value;
            undo_friend_request(friend_id: &'a str) -> Value;
            // groups
            get_all_groups() -> Value;
            get_group_info(group_ids: &'a [String]) -> Value;
            get_group_members_info(member_ids: &'a [String]) -> Value;
            create_group(options: &'a CreateGroupOptions) -> Value;
            add_user_to_group(member_ids: &'a [String], group_id: &'a str) -> Value;
            remove_user_from_group(member_ids: &'a [String], group_id: &'a str) -> Value;
            change_group_name(name: &'a str, group_id: &'a str) -> Value;
            leave_group(group_id: &'a str, silent: bool) -> Value;
            // messaging
            upload_attachment(sources: &'a [UploadSource], thread_id: &'a str, thread_type: ThreadType) -> Vec<UploadResult>;
            send_message(options: SendMessageOptions, thread_id: &'a str, thread_type: ThreadType) -> SendMessageResponse;
            send_voice(voice_url: &'a str, ttl: Option<i64>, thread_id: &'a str, thread_type: ThreadType) -> Value;
            send_sticker(sticker: &'a StickerParam, thread_id: &'a str, thread_type: ThreadType) -> Value;
            get_stickers(keyword: &'a str) -> Vec<i64>;
            get_stickers_detail(sticker_ids: &'a [i64]) -> Vec<Value>;
            add_reaction(icon: &'a str, target: &'a ReactionTarget) -> Value;
            undo(target: &'a UndoTarget, thread_id: &'a str, thread_type: ThreadType) -> Value;
            delete_message(target: &'a DeleteMessageTarget, only_me: bool) -> Value;
            forward_message(payload: &'a ForwardMessagePayload, thread_ids: &'a [String], thread_type: ThreadType) -> Value;
            send_typing_event(thread_id: &'a str, thread_type: ThreadType, dest_type: Option<DestType>) -> Value;
            send_seen_event(messages: &'a [SeenMessageInput], thread_type: ThreadType) -> Value;
            get_group_chat_history(group_id: &'a str, count: Option<u32>) -> Value;
            // polls
            create_poll(input: &'a CreatePollInput, group_id: &'a str) -> Value;
            get_poll_detail(poll_id: i64) -> Value;
            vote_poll(poll_id: i64, option_ids: &'a [i64]) -> Value;
            add_poll_options(input: &'a AddPollOptionsInput) -> Value;
            lock_poll(poll_id: i64) -> Value;
            // profile service
            get_znoise(user_ids: &'a [String], group_ids: &'a [String]) -> NoisedIdsReply;
        }
    };
}

macro_rules! define_api_trait {
    ($( $name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty ; )*) => {
        /// Everything `ZaloSession` needs from a logged-in Zalo client.
        pub trait ZaloApi: Send + Sync {
            /// The signed-in account's user id.
            fn own_id(&self) -> String;

            /// The concrete protocol client, when this API is backed by one. The call and
            /// history-sync managers need it (`VoiceCallApi`, `get_file_service`, ...); fakes
            /// return `None`.
            fn concrete(&self) -> Option<Arc<ZaloClient>>;

            /// Starts the chat-socket listener (self-listen on, retry-on-close on).
            fn start_listener(&self) -> ProtoResult<ListenerStart>;

            $( fn $name<'a>(&'a self $(, $arg: $ty)*) -> BoxFuture<'a, ProtoResult<$ret>>; )*
        }

        impl ZaloApi for ProtoApi {
            fn own_id(&self) -> String {
                self.client.own_id().to_string()
            }

            fn concrete(&self) -> Option<Arc<ZaloClient>> {
                Some(Arc::clone(&self.client))
            }

            fn start_listener(&self) -> ProtoResult<ListenerStart> {
                let (events, handle) = self.client.listener(true).start(true)?;
                Ok(ListenerStart {
                    events,
                    control: Arc::new(ProtoListenerControl(handle)),
                })
            }

            $( fn $name<'a>(&'a self $(, $arg: $ty)*) -> BoxFuture<'a, ProtoResult<$ret>> {
                Box::pin(self.client.$name($($arg),*))
            } )*
        }
    };
}

/// Production [`ZaloApi`]: delegates to a `zalo_proto::ZaloClient`.
pub struct ProtoApi {
    client: Arc<ZaloClient>,
}

impl ProtoApi {
    pub fn new(client: Arc<ZaloClient>) -> Self {
        Self { client }
    }
}

with_api_methods!(define_api_trait);

/// Logs in to Zalo, from saved credentials or a QR code.
pub trait ZaloLogin: Send + Sync {
    /// Logs in with saved credentials.
    fn login_cookie(
        &self,
        credentials: Credentials,
    ) -> BoxFuture<'_, ProtoResult<Arc<dyn ZaloApi>>>;

    /// Runs a QR-code login, reporting progress on `event_tx` until `abort_rx` fires or it ends.
    fn login_qr(
        &self,
        event_tx: mpsc::UnboundedSender<LoginQrEvent>,
        abort_rx: oneshot::Receiver<()>,
    ) -> BoxFuture<'_, ProtoResult<(Credentials, Arc<dyn ZaloApi>)>>;
}

/// Production [`ZaloLogin`]: `zalo_proto::login_cookie` / `login_qr`.
pub struct ProtoLogin;

impl ZaloLogin for ProtoLogin {
    fn login_cookie(
        &self,
        credentials: Credentials,
    ) -> BoxFuture<'_, ProtoResult<Arc<dyn ZaloApi>>> {
        Box::pin(async move {
            let client = zalo_proto::login_cookie(credentials).await?;
            Ok(Arc::new(ProtoApi::new(Arc::new(client))) as Arc<dyn ZaloApi>)
        })
    }

    fn login_qr(
        &self,
        event_tx: mpsc::UnboundedSender<LoginQrEvent>,
        abort_rx: oneshot::Receiver<()>,
    ) -> BoxFuture<'_, ProtoResult<(Credentials, Arc<dyn ZaloApi>)>> {
        Box::pin(async move {
            let http = ZaloHttp::new()?;
            let (creds, client) = zalo_proto::login_qr(http, None, event_tx, abort_rx).await?;
            Ok((
                creds,
                Arc::new(ProtoApi::new(Arc::new(client))) as Arc<dyn ZaloApi>,
            ))
        })
    }
}
