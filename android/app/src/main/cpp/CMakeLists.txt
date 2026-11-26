cmake_minimum_required(VERSION 3.22.1)
project(zaloopus C)

# libopus, built from the pinned release tarball (static, then wrapped by JNI in libzaloopus.so).
include(FetchContent)
set(OPUS_BUILD_SHARED_LIBRARY OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_PROGRAMS OFF CACHE BOOL "" FORCE)
set(OPUS_INSTALL_PKG_CONFIG_MODULE OFF CACHE BOOL "" FORCE)
set(OPUS_INSTALL_CMAKE_CONFIG_MODULE OFF CACHE BOOL "" FORCE)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
# Opus at -O0 is too slow for real-time audio, so keep the C code optimised in debug builds too.
string(APPEND CMAKE_C_FLAGS_DEBUG " -O2 -g")
FetchContent_Declare(opus
    URL https://github.com/xiph/opus/releases/download/v1.5.2/opus-1.5.2.tar.gz
    URL_HASH SHA256=65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1)
FetchContent_MakeAvailable(opus)

add_library(zaloopus SHARED opus_jni.c)
target_link_libraries(zaloopus opus)
