// JNI wrapper around libopus for com.zalom3ui.app.call.OpusEncoder / OpusDecoder.
// Handles are raw pointers passed to Kotlin as jlong; errors become IllegalStateException.
#include <jni.h>
#include <stdint.h>
#include <stdio.h>
#include <opus.h>

#define MAX_PACKET 1275

static void throw_opus(JNIEnv *env, const char *what, int err) {
    char msg[160];
    snprintf(msg, sizeof msg, "%s: %s", what, opus_strerror(err));
    jclass cls = (*env)->FindClass(env, "java/lang/IllegalStateException");
    if (cls) (*env)->ThrowNew(env, cls, msg);
}

JNIEXPORT jlong JNICALL
Java_com_zalom3ui_app_call_OpusEncoder_nativeCreate(JNIEnv *env, jobject self, jint rate, jint channels, jint bitrate) {
    int err = 0;
    OpusEncoder *enc = opus_encoder_create(rate, channels, OPUS_APPLICATION_VOIP, &err);
    if (err != OPUS_OK || !enc) { throw_opus(env, "opus_encoder_create", err); return 0; }
    // FEC lets the decoder rebuild a lost frame from the next packet; 10 % is the loss we plan for.
    opus_encoder_ctl(enc, OPUS_SET_BITRATE(bitrate));
    opus_encoder_ctl(enc, OPUS_SET_INBAND_FEC(1));
    opus_encoder_ctl(enc, OPUS_SET_PACKET_LOSS_PERC(10));
    opus_encoder_ctl(enc, OPUS_SET_DTX(0));
    opus_encoder_ctl(enc, OPUS_SET_COMPLEXITY(5));
    return (jlong)(intptr_t)enc;
}

JNIEXPORT jbyteArray JNICALL
Java_com_zalom3ui_app_call_OpusEncoder_nativeEncode(JNIEnv *env, jobject self, jlong handle, jshortArray pcm, jint frameSize) {
    OpusEncoder *enc = (OpusEncoder *)(intptr_t)handle;
    unsigned char out[MAX_PACKET];
    jshort *in = (*env)->GetShortArrayElements(env, pcm, NULL);
    if (!in) return NULL;
    int n = opus_encode(enc, in, frameSize, out, MAX_PACKET);
    (*env)->ReleaseShortArrayElements(env, pcm, in, JNI_ABORT);
    if (n < 0) { throw_opus(env, "opus_encode", n); return NULL; }
    jbyteArray res = (*env)->NewByteArray(env, n);
    if (res) (*env)->SetByteArrayRegion(env, res, 0, n, (const jbyte *)out);
    return res;
}

JNIEXPORT void JNICALL
Java_com_zalom3ui_app_call_OpusEncoder_nativeDestroy(JNIEnv *env, jobject self, jlong handle) {
    opus_encoder_destroy((OpusEncoder *)(intptr_t)handle);
}

JNIEXPORT jlong JNICALL
Java_com_zalom3ui_app_call_OpusDecoder_nativeCreate(JNIEnv *env, jobject self, jint rate, jint channels) {
    int err = 0;
    OpusDecoder *dec = opus_decoder_create(rate, channels, &err);
    if (err != OPUS_OK || !dec) { throw_opus(env, "opus_decoder_create", err); return 0; }
    return (jlong)(intptr_t)dec;
}

// packet == null means the packet was lost: opus_decode with NULL data runs concealment.
JNIEXPORT jshortArray JNICALL
Java_com_zalom3ui_app_call_OpusDecoder_nativeDecode(JNIEnv *env, jobject self, jlong handle, jbyteArray packet, jint frameSize, jint channels) {
    OpusDecoder *dec = (OpusDecoder *)(intptr_t)handle;
    opus_int16 out[5760 * 2]; // 120 ms at 48 kHz stereo, the libopus maximum
    if (frameSize <= 0 || channels <= 0 || frameSize * channels > (jint)(sizeof out / sizeof out[0])) {
        throw_opus(env, "opus_decode", OPUS_BAD_ARG);
        return NULL;
    }
    int n;
    if (packet) {
        jsize len = (*env)->GetArrayLength(env, packet);
        jbyte *data = (*env)->GetByteArrayElements(env, packet, NULL);
        if (!data) return NULL;
        n = opus_decode(dec, (const unsigned char *)data, len, out, frameSize, 0);
        (*env)->ReleaseByteArrayElements(env, packet, data, JNI_ABORT);
    } else {
        n = opus_decode(dec, NULL, 0, out, frameSize, 0);
    }
    if (n < 0) { throw_opus(env, "opus_decode", n); return NULL; }
    jshortArray res = (*env)->NewShortArray(env, n * channels);
    if (res) (*env)->SetShortArrayRegion(env, res, 0, n * channels, out);
    return res;
}

JNIEXPORT void JNICALL
Java_com_zalom3ui_app_call_OpusDecoder_nativeDestroy(JNIEnv *env, jobject self, jlong handle) {
    opus_decoder_destroy((OpusDecoder *)(intptr_t)handle);
}
