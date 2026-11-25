cmake_minimum_required(VERSION 3.22.1)
project(nodebridge C CXX)

# Set by app/build.gradle.kts to the extracted nodejs-mobile release.
set(NODE_MOBILE_DIR "" CACHE PATH "Extracted nodejs-mobile release")

add_library(node SHARED IMPORTED)
set_target_properties(node PROPERTIES IMPORTED_LOCATION ${NODE_MOBILE_DIR}/bin/${ANDROID_ABI}/libnode.so)

add_library(nodebridge SHARED native-lib.cpp)
target_include_directories(nodebridge PRIVATE ${NODE_MOBILE_DIR}/include/node)
find_library(log-lib log)
# AGP packages imported libraries that targets link against, so libnode.so ships with the APK.
target_link_libraries(nodebridge node ${log-lib})

# libopus, built from the pinned release tarball (static, then wrapped by JNI in libzaloopus.so).
include(FetchContent)
set(OPUS_BUILD_SHARED_LIBRARY OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_PROGRAMS OFF CACHE BOOL "" FORCE)
set(OPUS_INSTALL_PKG_CONFIG_MODULE OFF CACHE BOOL "" FORCE)
set(OPUS_INSTALL_CMAKE_CONFIG_MODULE OFF CACHE BOOL "" FORCE)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
# Opus at -O0 is too slow for real-time audio, so keep the C code optimised in debug builds too.
set(CMAKE_C_FLAGS_DEBUG "-O2 -g")
FetchContent_Declare(opus
    URL https://github.com/xiph/opus/releases/download/v1.5.2/opus-1.5.2.tar.gz
    URL_HASH SHA256=65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1)
FetchContent_MakeAvailable(opus)

add_library(zaloopus SHARED opus_jni.c)
target_link_libraries(zaloopus opus)
