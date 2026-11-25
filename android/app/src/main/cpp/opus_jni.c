// JNI wrapper around libopus for com.zalom3ui.app.call.OpusEncoder / OpusDecoder.
// Handles are raw pointers passed to Kotlin as jlong; errors become IllegalStateException.
#include <jni.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <opus.h>

#define MAX_PACKET 1275

typedef struct {
    OpusEncoder *enc;
    int channels;
} EncWrap;

typedef struct {
    OpusDecoder *dec;
    int channels;
} DecWrap;

static void throw_opus(JNIEnv *env, const char *what, int err) {
    if ((*env)->ExceptionCheck(env)) return;
    char msg[160];
    snprintf(msg, sizeof msg, "%s: %s", what, opus_strerror(err));
    jclass cls = (*env)->FindClass(env, "java/lang/IllegalStateException");
    if (cls) {
        (*env)->ThrowNew(env, cls, msg);
        (*env)->DeleteLocalRef(env, cls);
    }
}

static int configure_encoder(OpusEncoder *enc, jint bitrate, const char **failed_ctl) {
    int err;
    // FEC lets the decoder rebuild a lost frame from the next packet; 10 % is the loss we plan for.
    if ((err = opus_encoder_ctl(enc, OPUS_SET_BITRATE(bitrate))) != OPUS_OK) {
        *failed_ctl = "opus_encoder_ctl(OPUS_SET_BITRATE)";
        return err;
    }
    if ((err = opus_encoder_ctl(enc, OPUS_SET_INBAND_FEC(1))) != OPUS_OK) {
        *failed_ctl = "opus_encoder_ctl(OPUS_SET_INBAND_FEC)";
        return err;
    }
    if ((err = opus_encoder_ctl(enc, OPUS_SET_PACKET_LOSS_PERC(10))) != OPUS_OK) {
        *failed_ctl = "opus_encoder_ctl(OPUS_SET_PACKET_LOSS_PERC)";
        return err;
    }
    if ((err = opus_encoder_ctl(enc, OPUS_SET_DTX(0))) != OPUS_OK) {
        *failed_ctl = "opus_encoder_ctl(OPUS_SET_DTX)";
        return err;
    }
    if ((err = opus_encoder_ctl(enc, OPUS_SET_COMPLEXITY(5))) != OPUS_OK) {
        *failed_ctl = "opus_encoder_ctl(OPUS_SET_COMPLEXITY)";
        return err;
    }
    return OPUS_OK;
}

JNIEXPORT jlong JNICALL
Java_com_zalom3ui_app_call_OpusEncoder_nativeCreate(JNIEnv *env, jobject self, jint rate, jint channels, jint bitrate) {
    (void)self;
    int err = 0;
    OpusEncoder *enc = opus_encoder_create(rate, channels, OPUS_APPLICATION_VOIP, &err);
    if (err != OPUS_OK || !enc) { throw_opus(env, "opus_encoder_create", err); return 0; }
    const char *failed_ctl = NULL;
    err = configure_encoder(enc, bitrate, &failed_ctl);
    if (err != OPUS_OK) {
        opus_encoder_destroy(enc);
        throw_opus(env, failed_ctl, err);
        return 0;
    }
    EncWrap *wrap = (EncWrap *)malloc(sizeof(EncWrap));
    if (!wrap) {
        opus_encoder_destroy(enc);
        throw_opus(env, "opus_encoder_create", OPUS_ALLOC_FAIL);
        return 0;
    }
    wrap->enc = enc;
    wrap->channels = channels;
    return (jlong)(intptr_t)wrap;
}

JNIEXPORT jbyteArray JNICALL
Java_com_zalom3ui_app_call_OpusEncoder_nativeEncode(JNIEnv *env, jobject self, jlong handle, jshortArray pcm, jint frameSize) {
    (void)self;
    if (handle == 0 || !pcm) {
        throw_opus(env, "opus_encode", OPUS_BAD_ARG);
        return NULL;
    }
    EncWrap *wrap = (EncWrap *)(intptr_t)handle;
    int encChannels = wrap->channels;
    if (!wrap->enc || encChannels <= 0 || frameSize <= 0) {
        throw_opus(env, "opus_encode", OPUS_BAD_ARG);
        return NULL;
    }
    jsize pcmLen = (*env)->GetArrayLength(env, pcm);
    if (frameSize > pcmLen / encChannels) {
        throw_opus(env, "opus_encode", OPUS_BAD_ARG);
        return NULL;
    }
    unsigned char out[MAX_PACKET];
    jshort *in = (*env)->GetShortArrayElements(env, pcm, NULL);
    if (!in) return NULL;
    int n = opus_encode(wrap->enc, in, frameSize, out, MAX_PACKET);
    (*env)->ReleaseShortArrayElements(env, pcm, in, JNI_ABORT);
    if (n < 0) { throw_opus(env, "opus_encode", n); return NULL; }
    jbyteArray res = (*env)->NewByteArray(env, n);
    if (!res) return NULL;
    (*env)->SetByteArrayRegion(env, res, 0, n, (const jbyte *)out);
    return res;
}

JNIEXPORT void JNICALL
Java_com_zalom3ui_app_call_OpusEncoder_nativeDestroy(JNIEnv *env, jobject self, jlong handle) {
    (void)env;
    (void)self;
    EncWrap *wrap = (EncWrap *)(intptr_t)handle;
    if (!wrap) return;
    if (wrap->enc) opus_encoder_destroy(wrap->enc);
    free(wrap);
}

JNIEXPORT jlong JNICALL
Java_com_zalom3ui_app_call_OpusDecoder_nativeCreate(JNIEnv *env, jobject self, jint rate, jint channels) {
    (void)self;
    int err = 0;
    OpusDecoder *dec = opus_decoder_create(rate, channels, &err);
    if (err != OPUS_OK || !dec) { throw_opus(env, "opus_decoder_create", err); return 0; }
    DecWrap *wrap = (DecWrap *)malloc(sizeof(DecWrap));
    if (!wrap) {
        opus_decoder_destroy(dec);
        throw_opus(env, "opus_decoder_create", OPUS_ALLOC_FAIL);
        return 0;
    }
    wrap->dec = dec;
    wrap->channels = channels;
    return (jlong)(intptr_t)wrap;
}

// packet == null means the packet was lost: opus_decode with NULL data runs concealment.
JNIEXPORT jshortArray JNICALL
Java_com_zalom3ui_app_call_OpusDecoder_nativeDecode(JNIEnv *env, jobject self, jlong handle, jbyteArray packet, jint frameSize, jint channels) {
    (void)self;
    if (handle == 0) {
        throw_opus(env, "opus_decode", OPUS_BAD_ARG);
        return NULL;
    }
    DecWrap *wrap = (DecWrap *)(intptr_t)handle;
    int decChannels = wrap->channels;
    opus_int16 out[5760 * 2]; // 120 ms at 48 kHz stereo, the libopus maximum
    const int maxSamples = (int)(sizeof out / sizeof out[0]);
    if (!wrap->dec || decChannels <= 0 || channels != decChannels || frameSize <= 0 || frameSize > maxSamples / decChannels) {
        throw_opus(env, "opus_decode", OPUS_BAD_ARG);
        return NULL;
    }
    int n;
    if (packet) {
        jsize len = (*env)->GetArrayLength(env, packet);
        jbyte *data = (*env)->GetByteArrayElements(env, packet, NULL);
        if (!data) return NULL;
        n = opus_decode(wrap->dec, (const unsigned char *)data, len, out, frameSize, 0);
        (*env)->ReleaseByteArrayElements(env, packet, data, JNI_ABORT);
    } else {
        n = opus_decode(wrap->dec, NULL, 0, out, frameSize, 0);
    }
    if (n < 0) { throw_opus(env, "opus_decode", n); return NULL; }
    if (n > maxSamples / decChannels) {
        throw_opus(env, "opus_decode", OPUS_INTERNAL_ERROR);
        return NULL;
    }
    jshortArray res = (*env)->NewShortArray(env, n * decChannels);
    if (!res) return NULL;
    (*env)->SetShortArrayRegion(env, res, 0, n * decChannels, out);
    return res;
}

JNIEXPORT void JNICALL
Java_com_zalom3ui_app_call_OpusDecoder_nativeDestroy(JNIEnv *env, jobject self, jlong handle) {
    (void)env;
    (void)self;
    DecWrap *wrap = (DecWrap *)(intptr_t)handle;
    if (!wrap) return;
    if (wrap->dec) opus_decoder_destroy(wrap->dec);
    free(wrap);
}
