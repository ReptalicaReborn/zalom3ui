import assert from "node:assert/strict";
import test from "node:test";
import { extractDroppedMessages, messageFrameShape } from "../src/oamsg.js";
import { extractOtpCode, normalizeMessage, parseAttachment, type RawMessage } from "../src/store.js";

function frameOf(cmd: number, subCmd: number, payload: unknown): Buffer {
    const header = Buffer.alloc(4);
    header[0] = 1;
    header.writeUInt16LE(cmd, 1);
    header[3] = subCmd;
    const envelope = JSON.stringify({ encrypt: 0, data: JSON.stringify(payload) });
    return Buffer.concat([header, Buffer.from(envelope, "utf8")]);
}

test("extractDroppedMessages captures OA pageMsgs from cmd 501 without duplicating regular msgs", () => {
    const frame = frameOf(501, 0, {
        data: {
            msgs: [
                { msgId: "100", cliMsgId: "100", msgType: "webchat", uidFrom: "friend1", idTo: "0", dName: "Friend", ts: "1700000001000", content: "hi" },
            ],
            pageMsgs: [
                {
                    msgId: "200",
                    cliMsgId: "200",
                    msgType: "chat.webcontent.v2",
                    uidFrom: "oa_the1",
                    idTo: "0",
                    dName: "The 1",
                    ts: "1700000002000",
                    content: {
                        title: "Đổi mã PIN",
                        description: "Bạn đang yêu cầu cài đặt lại mã PIN tài khoản Giang với mã tài khoản 0378795621 tại The 1.",
                        href: "https://the1.vn/reset-pin",
                        params: JSON.stringify({
                            actions: [{ actionTitle: "ĐẶT LẠI MÃ PIN", actionUrl: "https://the1.vn/reset-pin" }],
                        }),
                    },
                },
            ],
        },
    });

    const out = extractDroppedMessages(frame, undefined, "my_uid");
    assert.equal(out.length, 1);
    assert.equal(out[0].threadId, "oa_the1");
    assert.equal(out[0].data.dName, "The 1");

    const normalized = normalizeMessage(out[0]);
    assert.equal(normalized.attachment?.kind, "link");
    assert.equal(normalized.attachment?.title, "Đổi mã PIN");
    assert.equal(normalized.attachment?.actionLabel, "ĐẶT LẠI MÃ PIN");
    assert.equal(normalized.attachment?.href, "https://the1.vn/reset-pin");
    assert.equal(normalized.attachment?.otp, undefined, "10-digit account ID 0378795621 must not be treated as OTP");
});

test("parseAttachment handles chat.webcontent.v2 ZNS OTP cards and chat.list.action arrays", () => {
    const zns = parseAttachment("chat.webcontent.v2", {
        thumb: "https://cdn.example/logo.png",
        zinstant_msg: JSON.stringify({
            customMsg: {
                msg: { vi: "Mã xác thực OTP\nMã OTP của bạn là 849201. Không chia sẻ cho bất kỳ ai." },
            },
            actions: [{ actionTitle: "Sao chép mã", actionData: JSON.stringify({ code: "849201" }) }],
        }),
    });
    assert.equal(zns?.kind, "link");
    assert.equal(zns?.title, "Mã xác thực OTP");
    assert.equal(zns?.otp, "849201");
    assert.equal(zns?.actionLabel, "Sao chép mã");

    const listMsg: RawMessage = {
        type: 0,
        threadId: "oa_shop",
        isSelf: false,
        data: {
            msgId: "300",
            cliMsgId: "300",
            msgType: "chat.list.action",
            uidFrom: "oa_shop",
            idTo: "my_uid",
            dName: "Shop OA",
            ts: "1700000003000",
            content: [
                { type: "l.a.header.full", title: "Xác nhận đơn hàng", description: "Đơn #1234 đang giao", thumb: "https://cdn.example/box.jpg" },
                { type: "l.a.child.full", title: "Theo dõi đơn", href: "https://shop.example/track/1234" },
            ],
        },
    };
    const norm = normalizeMessage(listMsg);
    assert.equal(norm.attachment?.kind, "link");
    assert.equal(norm.attachment?.title, "Xác nhận đơn hàng");
    assert.equal(norm.attachment?.actionLabel, "Theo dõi đơn");
    assert.equal(norm.attachment?.href, "https://shop.example/track/1234");
});

test("extractOtpCode extracts 4-8 digit OTPs and ignores 10-digit account/phone numbers", () => {
    assert.equal(extractOtpCode("Mã xác thực OTP của bạn là: 482910 (hiệu lực 2 phút)"), "482910");
    assert.equal(extractOtpCode("Your verification code is 9912"), "9912");
    assert.equal(extractOtpCode("739201 là mã xác nhận giao dịch"), "739201");
    assert.equal(extractOtpCode("Bạn đang yêu cầu cài đặt lại mã PIN tài khoản Giang với mã tài khoản 0378795621 tại The 1."), undefined);
    assert.equal(extractOtpCode("Hẹn gặp lúc 14:30 nhé"), undefined);
});

test("messageFrameShape logs where unusual messages arrive, never their ids, text or codes", () => {
    const otpFrame = frameOf(501, 0, {
        data: {
            msgs: [{ msgId: "1", msgType: "webchat", uidFrom: "friend1", idTo: "0", ts: "1", content: "hi" }],
            znsMsgs: [
                {
                    msgId: "2",
                    msgType: "chat.webcontent.v2",
                    uidFrom: "oa_bank",
                    idTo: "0",
                    ts: "2",
                    content: { title: "Mã OTP", description: "Mã xác thực của bạn là 482913", params: "{}" },
                },
            ],
            delivereds: [{ msgId: "9", seenUids: ["x"] }],
        },
    });
    const shape = messageFrameShape(otpFrame, undefined);
    assert.ok(shape);
    assert.equal(shape.cmd, 501);
    assert.deepEqual(shape.keys, ["msgs", "znsMsgs", "delivereds"]);
    assert.deepEqual(shape.lists, [
        { name: "msgs", count: 1, msgTypes: ["webchat"], content: ["text"] },
        { name: "znsMsgs", count: 1, msgTypes: ["chat.webcontent.v2"], content: ["description,params,title"] },
    ]);
    const logged = JSON.stringify(shape);
    for (const secret of ["482913", "oa_bank", "friend1", "Mã OTP", "hi\""]) assert.ok(!logged.includes(secret), secret);

    // Plain text in the lists zca-js reads is the normal case: nothing to log.
    const plain = frameOf(521, 0, { data: { groupMsgs: [{ msgId: "3", msgType: "webchat", uidFrom: "a", idTo: "g", ts: "3", content: "yo" }] } });
    assert.equal(messageFrameShape(plain, undefined), null);
    // Not a message frame.
    assert.equal(messageFrameShape(frameOf(601, 0, { data: { controls: [] } }), undefined), null);
});
