#!/bin/zsh
# Runs every benchmark and writes the raw output to each benchmark's out/.
# Usage: run.sh <simulator udid>. Needs GRADLE, JAVA_HOME (JDK 21), ANDROID_NDK_HOME,
# ANDROID_SERIAL and boltffi 0.32.1, as the two READMEs say. Every build uses the
# lock files kept beside the benchmarks.
set -eu
cd "${0:A:h}"
udid=$1
limit() { perl -e 'alarm shift; exec @ARGV' "$@"; }
{ sysctl -n machdep.cpu.brand_string; sw_vers; date -u +%Y-%m-%dT%H:%MZ; } > machine.txt

(cd ffi-boundary && limit 1800 sh -c '
  (cd bolt && boltffi pack apple --release --deny-skipped) &&
  (cd bolt && boltffi pack android --release --deny-skipped) &&
  cargo build --release --locked -p uni-bench &&
  cargo build --release --locked -p uni-bench --target aarch64-apple-ios-sim &&
  cargo ndk -t arm64-v8a -P 26 build --release --locked -p uni-bench &&
  cargo run --release --locked -p uni-bench --bin uniffi-bindgen -- generate --library target/release/libuni_bench.dylib --language swift --out-dir uni/dist/swift &&
  cargo run --release --locked -p uni-bench --bin uniffi-bindgen -- generate --library target/release/libuni_bench.dylib --language kotlin --out-dir uni/dist/kotlin')
limit 1200 ffi-boundary/harness/run-apple.sh "$udid"
limit 1800 ffi-boundary/harness/run-android.sh

limit 2400 decompose-comparison/run-host.sh
limit 3600 decompose-comparison/run-devices.sh "$udid"
