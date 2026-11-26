# Replay test

Zalo Web has no history endpoint for 1:1 chats. Each time Zalỏ connects, it asks Zalo to replay the messages it missed. We do not know how much Zalo replays after a long time offline. We also do not know what the `lastId` field does.

This test finds out. The answer decides whether "Stay connected → While open" is safe as the default.

You need your real Zalo account, a second person (or a second account) who can send you messages, and Zalỏ with Zalo running on the phone.

## What the app records

Each time Zalỏ connects, it writes a replay report. The last 50 reports are kept on the phone. A report holds only ids, counts, timestamps and kinds. It holds no message text.

Per connect, the report says:

- how long Zalỏ was offline;
- the newest message we had before the replay, for 1:1 chats and for groups;
- every replay request, and every batch Zalo sent back: how many messages, the oldest and newest time, and how many were already stored or new;
- a verdict for 1:1 chats and one for groups.

The verdicts:

- `covered`: the oldest replayed message is at or before our newest stored one. The replay overlaps what we had, so there is no gap.
- `nothing_missed`: Zalo replayed nothing.
- `possible_gap`: everything replayed is newer than what we had, and the batch looks full. Messages may be missing.
- `unknown`: anything else. For example, nothing was stored yet, or a small batch did not overlap.

A batch "looks full" when it has at least 10 messages and is at least as big as the biggest batch seen so far. The report shows the biggest batch seen. That number is Zalo's page size, if it has one.

## Where to find it

Open Settings. Scroll to Developer.

- **Replay log** shows the reports, newest first. You can copy, share or clear them.
- **Replay mode** picks what Zalỏ sends when it asks for old messages: `null`, `newest` or `page`.

Developer shows only while Zalo runs on this phone and you are logged in.

## Before you start

1. Open Settings → Stay connected. Choose **While open**. Now Zalỏ really disconnects when you close it. (With **Always** it never goes offline, so there is nothing to test.)
2. Open Developer → Replay mode. Make sure it says `null`.
3. Open Developer → Replay log and press Clear.
4. Open a few chats once, so Zalỏ has a newest message stored for 1:1 chats and for groups.

## Round 1: mode `null`

Repeat this for each gap: 1 hour, 8 hours, 24 hours, 3 days.

1. Close Zalỏ. Wait one minute, so it pauses. Swipe it away from recents if you like.
2. Do not open Zalỏ. Do not open Zalo Web either, or you may change what is replayed.
3. While Zalỏ is closed, have someone send you messages:
   - a few 1:1 messages from one person;
   - a few 1:1 messages from a second person, if you can;
   - a few messages in one group.
   For the longer gaps, spread them out. Send more than 50 in total at least once, so we can see if Zalo cuts the replay off.
4. After the gap, open Zalỏ.
5. Wait one minute. Open Settings → Developer → Replay log.
6. Read the newest report. Note the offline time, the verdicts, and the batch sizes.
7. Send the log to yourself (Share) and keep it.

## Round 2: mode `newest`

1. Set Replay mode to `newest`.
2. Repeat the gaps above, at least 1 hour, 8 hours and 24 hours.
3. Read the reports the same way.

In this mode Zalỏ sends `lastId` = the id of the newest message it has. See what Zalo does with it. Does it send everything after that message? Does it send nothing?

## Round 3: mode `page`

1. Set Replay mode to `page`.
2. Repeat the gaps above, at least 8 hours and 24 hours, with more than 50 messages.
3. Read the reports.

In this mode Zalỏ asks once with `null`. While the batch looks full and does not overlap what we have, it asks again from the oldest id of that batch. It stops after 5 requests. Each page is listed in the report.

## How to read the result

Look at the 1:1 verdict first. Groups matter less: Zalỏ can also fetch group history separately.

"While open" is safe as the default if both are true:

- the 1:1 verdict is `covered` or `nothing_missed` after a typical offline gap (8 hours is a good guide), and
- that holds in some mode, not necessarily `null`.

If `possible_gap` shows up after a typical gap in every mode, "While open" loses messages. Keep **Always** as the default.

## Which mode becomes the default

- If `null` already gives `covered` or `nothing_missed`: keep `null`. Nothing changes in the code.
- If `null` leaves a gap and `newest` gives `covered` or `nothing_missed`: make `newest` the default.
- If only `page` closes the gap: make `page` the default. Note how many pages it needed. If it often needs all 5, the gap is probably too big to close.
- If no mode closes the gap after 8 hours: do not change the default.

To change the default replay mode, change `ReplayMode::default()` in `rust/zalo-server/src/replay.rs`.

To change the default "Stay connected" mode for new installs, change `StayConnected.NEW_INSTALL_DEFAULT` in `android/app/src/main/java/com/zalom3ui/app/data/StayConnected.kt`. It is one line. Existing installs keep what they have saved.

## Tips

- Do the test with Wi-Fi and mobile data both working, so a flaky network does not hide the answer.
- Do not log out of Zalo during the test. Logging out clears the stored messages.
- If a report says `unknown`, read the pages. Often the reason line explains it.
- To start over, press Clear in the Replay log.
