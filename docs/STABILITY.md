# Stability fixes and verification

This document describes the initial render/diagnostics patch. The later
JNI initialization changes and their compatibility limits are documented in
[JNI_CRASH.md](JNI_CRASH.md); that follow-up supersedes the JNI `Send`/`Sync`
and JNI recovery concerns listed below.

## Scope

These changes address observable code-level crash risks and frame-copy overhead.
They do not establish the cause of a particular user's crash, do not claim an
FPS improvement measured in the game, and do not modify anti-cheat evasion,
DLL hiding, attestation handling, or combat/movement behavior.

### Changes

- Render readers share one immutable `Arc<RenderBuffer>` instead of cloning all
  commands and strings on every display frame. Commands and text indices are
  published together. The reader never waits for the producer and releases the
  publication lock before OpenGL calls. The producer still copies once per
  submitted frame; this is not a zero-allocation renderer.
- The native overlay declines unknown function prologues rather than blindly
  copying 14 bytes. It declines a pre-existing foreign hook, checks initial
  `VirtualProtect` failure, frees unused allocation on that failure, and uses
  explicitly unaligned stores for encoded instruction operands.
- The diagnostic runner reports export addresses without decoding arbitrary
  bytes as a pointer and dereferencing it. Loaded diagnostic DLL handles are
  released.
- Logging no longer assumes `D:\project\rustme\dump` exists. Directory creation
  and file opening happen once. Concurrent logging is best effort: a message
  may be dropped while another logger holds the file lock. File writes remain
  synchronous; this is not an asynchronous logger.

### Finding the new log

The logger tries these directories in order, falling back if opening fails:

1. `VIBE_LOG_DIR`, if set to a nonempty directory path.
2. `%LOCALAPPDATA%\VibeClient\logs`.
3. The OS temporary directory, under `VibeClient\logs`.

The filename is `client.log`. The original hard-coded log directory is no
longer used. An unsupported prologue is logged as:

```text
[RENDER] Unsupported wglSwapBuffers prologue; overlay not installed
```

That means the overlay was intentionally not installed, not that it was fixed
for that Windows build. Do not remove the safety check to force installation.

## Validation performed

- `cargo check --locked --all-targets --target x86_64-pc-windows-gnu` passed
  using rustc 1.98.1 on Linux. This checks compilation, not Windows linking
  or execution.
- Six standalone frame-exchange tests passed: empty state, shared storage,
  old-frame lifetime, nonblocking reads, poisoned-lock handling, and consistent
  frame contents during concurrent publication.
- Three standalone logging tests passed: path ordering, empty-setting fallback,
  and appending multiple messages through one file handle.
- `git diff --check` passed.

To repeat the platform-independent tests without loading the native client:

```sh
rustc --edition=2021 --test src/engine/frame_exchange.rs -o frame-tests
./frame-tests
rustc --edition=2021 --test src/engine/diagnostics.rs -o log-tests
./log-tests
```

On Windows, use `.exe` output filenames and execute them normally.

## Remaining risks / required follow-up

The full DLL was not linked or run against RustMe in this environment. The
following are known inspection concerns, not established causes of the reported
crash, and are not resolved by this patch:

- Concurrent inline patch installation can race with an executing game thread.
- Native mutable globals and manual exception/stack recovery need a separate
  lifecycle/thread-safety review.
- `JNIEnv` is thread-local, but `JniBridge` declares unsafe `Send` and `Sync`.
- Starting a Rust thread inside `DllMain` remains a loader-lock risk.
- JNI class/field compatibility with the user's installed game is unverified.
- The fixed-function OpenGL overlay requires a compatible context.

Before concluding the crash is resolved, collect the Windows version, GPU and
driver version, game/runtime build, exact reproduction steps, `client.log`,
game `latest.log`, and the JVM `hs_err_pid*.log` if one was produced. Redact
account names, tokens, and other private information before sharing logs.
Compare the same scene and settings before/after to measure frame-time changes.

## Research references

- Rust `Arc`: cloning shares the allocation rather than cloning its contents:
  https://doc.rust-lang.org/std/sync/struct.Arc.html
- Rust `Mutex`: locking, try-locking, and poisoning:
  https://doc.rust-lang.org/std/sync/struct.Mutex.html
- Microsoft DLL best practices: loader-lock restrictions and thread creation
  inside `DllMain`:
  https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-best-practices
- Oracle JNI functions: local reference frames and failure behavior:
  https://docs.oracle.com/javase/8/docs/technotes/guides/jni/spec/functions.html
- JNI threading/reference lifecycle guidance:
  https://developer.android.com/ndk/guides/jni-tips

The existing snapshot reader already pushes/pops a local reference frame;
this patch does not claim to have added one or established a local-reference leak.