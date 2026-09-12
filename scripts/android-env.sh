#!/bin/sh
# Source this file from project scripts; it intentionally contains no secrets.
set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export REPO_ROOT
export RUSTUP_HOME="$REPO_ROOT/.toolchains/rustup"
export CARGO_HOME="$REPO_ROOT/.toolchains/cargo"
export JAVA_HOME="$REPO_ROOT/.toolchains/jdk-21"
export ANDROID_HOME="$REPO_ROOT/.toolchains/android-sdk"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/26.1.10909125"
export ZIG_GLOBAL_CACHE_DIR="$REPO_ROOT/.toolchains/zig-cache"
export CMAKE_TOOLCHAIN_FILE_aarch64_linux_android="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake"
export CMAKE_GENERATOR_aarch64_linux_android="Unix Makefiles"
export PATH="$REPO_ROOT/scripts/toolchain:$JAVA_HOME/bin:$REPO_ROOT/.toolchains/node-22.12.0/bin:$REPO_ROOT/.toolchains/make/bin:$CARGO_HOME/bin:$ANDROID_HOME/platform-tools:/usr/bin:/bin"
