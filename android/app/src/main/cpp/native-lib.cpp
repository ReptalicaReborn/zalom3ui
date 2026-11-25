// JNI bridge that runs Node.js (nodejs-mobile) inside the app's ":node" process.
#include <android/log.h>
#include <jni.h>
#include <pthread.h>
#include <unistd.h>

#include <atomic>
#include <cerrno>
#include <cstdio>
#include <cstring>
#include <new>
#include <string>
#include <vector>

#include "node.h"

namespace {

const char *kTag = "ZalooNode";
int stdoutPipe[2] = {-1, -1};
int stderrPipe[2] = {-1, -1};
std::atomic<bool> started{false};

struct PipeArgs {
    int fd;
    int priority;
};

// Node writes to stdout/stderr, which Android discards; forward both to logcat.
void *forwardToLogcat(void *raw) {
    auto *args = static_cast<PipeArgs *>(raw);
    if (!args) return nullptr;
    char buf[2048];
    for (;;) {
        ssize_t n = read(args->fd, buf, sizeof buf - 1);
        if (n < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (n == 0) break;
        if (buf[n - 1] == '\n') n--;
        buf[n] = '\0';
        __android_log_write(args->priority, kTag, buf);
    }
    close(args->fd);
    delete args;
    return nullptr;
}

void startForwarding(int (&fds)[2], int target, int priority) {
    if (pipe(fds) != 0) return;
    if (dup2(fds[1], target) < 0) {
        close(fds[0]);
        close(fds[1]);
        fds[0] = -1;
        fds[1] = -1;
        return;
    }
    if (fds[1] != target) {
        close(fds[1]);
    }
    fds[1] = -1;
    auto *args = new (std::nothrow) PipeArgs{fds[0], priority};
    if (!args) {
        close(fds[0]);
        fds[0] = -1;
        return;
    }
    pthread_t thread;
    if (pthread_create(&thread, nullptr, forwardToLogcat, args) == 0) {
        pthread_detach(thread);
    } else {
        delete args;
        close(fds[0]);
        fds[0] = -1;
    }
}

}  // namespace

extern "C" JNIEXPORT jint JNICALL
Java_com_zalom3ui_app_server_NodeRuntime_startNode(JNIEnv *env, jobject /* this */, jobjectArray arguments) {
    if (arguments == nullptr) return -1;

    bool expected = false;
    if (!started.compare_exchange_strong(expected, true)) return -1;

    jsize argc = 0;
    char *block = nullptr;
    char **argv = nullptr;
    try {
        argc = env->GetArrayLength(arguments);
        if (env->ExceptionCheck() || argc < 0) {
            started.store(false);
            return -1;
        }
        std::vector<std::string> args;
        args.reserve(static_cast<size_t>(argc));
        size_t total = 0;
        for (jsize i = 0; i < argc; i++) {
            auto str = static_cast<jstring>(env->GetObjectArrayElement(arguments, i));
            if (env->ExceptionCheck() || str == nullptr) {
                if (str != nullptr) env->DeleteLocalRef(str);
                started.store(false);
                return -1;
            }
            const char *chars = env->GetStringUTFChars(str, nullptr);
            if (env->ExceptionCheck() || chars == nullptr) {
                if (chars != nullptr) env->ReleaseStringUTFChars(str, chars);
                env->DeleteLocalRef(str);
                started.store(false);
                return -1;
            }
            try {
                args.emplace_back(chars);
            } catch (...) {
                env->ReleaseStringUTFChars(str, chars);
                env->DeleteLocalRef(str);
                throw;
            }
            env->ReleaseStringUTFChars(str, chars);
            env->DeleteLocalRef(str);
            total += args.back().size() + 1;
        }

        // libuv expects all argv strings in one contiguous block. Never freed: Node owns it for the process lifetime.
        block = new (std::nothrow) char[total > 0 ? total : 1];
        argv = new (std::nothrow) char *[static_cast<size_t>(argc) + 1];
        if (!block || !argv) {
            delete[] block;
            delete[] argv;
            started.store(false);
            return -1;
        }
        char *cursor = block;
        for (jsize i = 0; i < argc; i++) {
            std::memcpy(cursor, args[i].c_str(), args[i].size() + 1);
            argv[i] = cursor;
            cursor += args[i].size() + 1;
        }
        argv[argc] = nullptr;
    } catch (...) {
        delete[] block;
        delete[] argv;
        started.store(false);
        return -1;
    }

    setvbuf(stdout, nullptr, _IOLBF, 0);
    setvbuf(stderr, nullptr, _IONBF, 0);
    startForwarding(stdoutPipe, STDOUT_FILENO, ANDROID_LOG_INFO);
    startForwarding(stderrPipe, STDERR_FILENO, ANDROID_LOG_ERROR);

    try {
        return static_cast<jint>(node::Start(argc, argv));
    } catch (...) {
        return -1;
    }
}
