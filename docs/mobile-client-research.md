# Research: identifying Zalỏ as Zalo PC or the mobile app

> Status: **research only.** No protocol code here, nothing built. This records what a client-identity
> change would involve, what is unknown, the risks, and what inputs are needed to go further. It exists
> because OTP / Official-Account messages do not reach Zalỏ (see the socket log in *Settings → Developer*),
> and because the owner asked whether imitating the mobile app would also bring push notifications.

## The short answers

- **Does this give real push notifications? No.** Real Zalo push is Zalo's servers → Google FCM, using a
  registration tied to the official app's signing and Zalo's server-side allowlist. A third-party client
  cannot register as that FCM client without Zalo's app-signing key and server cooperation. Imitating the
  mobile app does **not** change that. On Android, the realistic options for "push" are, best first:
  1. the **notification bridge** already in the app (reads the official Zalo app's notifications; instant,
     reliable, needs that app installed);
  2. a **held connection** (the foreground service keeps the socket open; live, not push);
  3. FCM impersonation — not realistically achievable, and the most ban-prone part.
- **Would it fix OTP? Maybe, and we don't yet know.** The question is whether Zalo delivers OTP / OA / ZNS
  messages to non-mobile sessions at all. The socket log answers that on a real phone before any rework.

## Where Zalỏ sits today

Zalỏ is **not** a pure "Zalo Web" client. It already logs in through Zalo's **PC** QR flow and then
identifies as a web session. Concretely (`rust/zalo-proto/src/login.rs`, `types.rs`, `http.rs`):

| Field | Current value | Meaning |
|---|---|---|
| `zpw_type` | `30` | The client-type discriminator on every API call |
| `zpw_ver` | `685` | Client version |
| `computer_name` | `"Web"` | Reported at login |
| QR login endpoint | `id.zalo.me/account?continue=…/pc`, PC API headers (`QR_API_HEADERS_PC`) | The PC login path |
| User-Agent | a desktop Firefox string | Desktop browser |
| Device identity | `imei` (a generated UUID) + `first_launch_time`, hashed into the request signature (`ParamsEncryptor`, `crypto.rs`) | Per-install id |

So there are three identity levels, not two:

1. **Web** — what chat.zalo.me runs. Zalỏ is already past this.
2. **PC (Zalo PC desktop)** — same wire protocol family as web (same encryption, same `zpw_*` params, the
   history-sync code already has `[PC]`-confirmed paths), but a different `zpw_type`, version, `computer_name`,
   User-Agent, and possibly a few extra login fields. **This is a small, cheap change**, mostly constants.
3. **Mobile (the Android/iOS app)** — a different protocol: its own login (phone number + SMS code + device
   registration, not QR), its own request signing and keys built into the app, and FCM push. **This is a large,
   fragile rewrite.**

## Experiment A: present as Zalo PC (cheap, low-risk)

**Hypothesis:** OA/OTP/ZNS messages may be pushed to Zalo PC sessions but not to web sessions. Unproven.

**What changes:** a developer switch that sends `zpw_type` = the PC value (not 30), the PC `zpw_ver`,
`computer_name` = a desktop-style name, the PC User-Agent, and whatever extra login fields Zalo PC sends. It
does **not** touch the phone's login (PC is a companion session, like web), so the notification bridge keeps
working. New QR login required when toggled.

**Risk:** low. Same protocol family; it's a companion session; reversible by toggling off. Worst case Zalo
rejects the session and you log in again as web.

### What public reverse-engineering says (unverified, sources conflict)

Read from third-party RE repos (`qnguyen3/zalo-swift-re`, `ctdoteam1110/ctdozlite`, `williamcachamwri/zalo-tg`),
not confirmed against Zalo. The values **drift with each Zalo PC release**, so pin them from a current source
before using them:

| Field | Value(s) seen | Source |
|---|---|---|
| `zpw_type` (Zalo PC) | **23** (Zalo PC 26.10.20, captured 2026‑11‑08); **30** in three older notes | qnguyen3 vs ctdozlite / williamcachamwri |
| `zpw_ver` (Zalo PC) | **694** (current), 693, 671 | version drift |
| `computer_name` | the machine's real hostname (`os.hostname()`), fallback literal `ZaloPCClient` — **not** a fixed string | williamcachamwri, qnguyen3 |
| User-Agent | `ZaloPC-darwin-23v694` (Electron sets the whole UA), or an Electron `ZaloPC/…` string | qnguyen3 / williamcachamwri |
| PC QR login | `GET /api/login/reqqr` with `type=30`, `client_version`, `imei`, `computer_name`; confirm URL carries `src=pc` | ctdozlite, williamcachamwri |

So the PC lever is real and roughly known: a different `zpw_type`/`zpw_ver`, a desktop User-Agent, a hostname
`computer_name`, and the `src=pc` QR confirm. **But** — the decisive point — **none of these sources document
whether Zalo PC receives OA / OTP / ZNS messages differently from a web session.** That is exactly the unknown
Experiment A is testing, and nothing public answers it. So Experiment A stays a gamble, justified only once the
socket log shows OTPs never reach the web session.

No source describes the **mobile** app's `zpw_type`, login, signing or UA at all; Experiment B remains
unsourced and would need the APK.

## Experiment B: emulate the mobile app (expensive, higher-risk) — fork only

**What it would require, none of it public:**

- **Login:** phone number + SMS OTP + device registration. Replaces the QR flow entirely. Zalo issues a device
  token the client must store and present.
- **Request signing:** the mobile app signs requests with keys compiled into the app binary. These must be
  extracted from the APK and may rotate with app updates.
- **Protocol surface:** message formats, the socket framing and the API endpoints differ from web/PC in places;
  each has to be mapped from captured traffic.
- **FCM:** even with all the above, push still needs Zalo's app-signing identity — out of reach (see top).

**Hard constraints:**

- **One phone per account.** Logging in as "the mobile app" signs the official Zalo app out of this phone.
  That **kills the notification bridge**, which is today's best notifications and today's OTP fallback. So
  mobile emulation and the bridge cannot both run on the same phone/account. This largely defeats the stated
  goal (OTP + push) unless mobile emulation delivers OTP *and* a working live connection by itself.
- **Fragility:** signing keys and protocol details change with Zalo app releases; expect breakage.
- **Ban risk:** a fake phone is easier for Zalo to detect than a companion session, and the account is what's
  at stake. Use a throwaway account only.
- **Effort:** weeks, with the APK and captured traffic in hand; impossible to do blind.

**Therefore:** Experiment B is worth starting only as an isolated fork, as research, and only if Experiment A
and the OTP fixes both fail to deliver OTP. It should never share a branch with the shippable app.

## Recommended order

1. **Ship the OTP work in flight** (socket diagnostic log + the notification-bridge OTP rescue). On a real
   phone, the socket log says whether OTPs reach a web session at all. If they do, this is solved with no
   identity change.
2. **If OTPs never arrive over the socket:** try **Experiment A (PC identity)**. Small, reversible, keeps the
   bridge. Re-read the socket log.
3. **Only if A also fails:** open a separate fork for **Experiment B**, scoped from the inputs below. Treat it
   as research on a throwaway account.

## Inputs needed from the owner

I can't fetch Zalo's app or desktop client in this environment, and real testing needs an account you can risk.
To move past this writeup:

**For Experiment A (PC identity):**
- One capture of a **Zalo PC desktop** login + a few API calls (HAR or a proxy log such as mitmproxy/Fiddler),
  with the `zpw_type`, `zpw_ver`, `computer_name`, User-Agent and login form fields visible. Redact cookies and
  any `params=` / signature values — I need the field **names and the type/version numbers**, not your secrets.
- Or: a pointer to the current Zalo PC web bundle, from which those constants can be read.

**For Experiment B (mobile), only if we get there:**
- The Zalo **Android APK** (version noted), for the login flow and signing.
- A capture of a **mobile app** login + message receive (again, structure not secrets).
- Confirmation you have a **throwaway account** to test with, separate from your main one.

**A decision from you at each step:** proceed to the next experiment or stop. Each step down this list trades
more risk and effort for a smaller chance of success.

## Why this lives here and not in the app

Nothing in this document is wired into Zalỏ. Experiment A, if pursued, would be a developer-only toggle.
Experiment B would be a separate fork on a throwaway account. The shippable app keeps logging in as it does now.
