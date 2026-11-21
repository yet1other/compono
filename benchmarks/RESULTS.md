# Where the published numbers come from

The tables in `decompose-comparison/README.md` and `ffi-boundary/README.md` are the
medians in these raw files, which are kept as the runs wrote them:

| Table | Raw output |
|---|---|
| Compono and Decompose, this Mac, JVM and Kotlin/Native | `decompose-comparison/out/compono-host.txt`, `decompose-jvm.txt`, `decompose-macos.txt` |
| Compono and Decompose, iOS simulator | `decompose-comparison/out/compono-ios.txt`, `decompose-ios.txt` |
| Compono and Decompose, Android emulator | `decompose-comparison/out/compono-android.txt`, `decompose-android.txt` |
| FFI boundary, Android emulator | `ffi-boundary/out/android-bolt.log`, `android-uni.log` |
| FFI boundary, iOS simulator | not kept: `run-apple.sh` printed it without writing a file. It now writes `out/apple-bolt.log` and `out/apple-uni.log` |

Each line is a scenario, then the median, lowest and highest nanoseconds over 31
rounds. The files are dated 2026-11-10 and were written on an Apple M4 with 24 GB.
The macOS version of that run was not recorded; `ffi-boundary/README.md` names the
simulator and emulator. `run.sh` runs everything again and records the machine and date in
`machine.txt`.

The Rust builds use `decompose-comparison/rust/Cargo.lock` and
`ffi-boundary/Cargo.lock`, kept in the repository. Decompose 3.6.1 and the Kotlin
toolchain are pinned in the Gradle build files.
