# JNI initialization crash follow-up

## Report and confidence

The reported Windows access violation occurred after the old `Attached!` log
and before game-ID resolution completed. The game rendering path had not yet
started. Older JVM crash reports were described as dereferencing a register
whose bytes spelled `java/lan`.

This narrows the investigation to JNI initialization. It does not prove the
exact faulting function or instruction without the complete current dump and
matching runtime symbols. Absence of a new `hs_err` file does not establish why
HotSpot failed to produce one.

Two definite ABI defects were found in the source:

- `GetStringUTFChars` was called with two arguments instead of the required
  `(JNIEnv*, jstring, jboolean* isCopy)` signature.
- `SetFloatField` and `SetDoubleField` were invoked with integer bit-pattern
  arguments instead of actual floating-point arguments.

The pointer reported by the old log was obtained by memory scanning rather than
the standard Invocation API. The dispatcher also decoded intermediate machine
code to select a different function address. Neither heuristic establishes a
valid JavaVM/function signature. They are removed, not enhanced or repaired.

## Changes

- `jni_api.rs` owns explicitly typed `extern "system"` JNI calls through the
  exact function table returned by the JVM.
- String acquisition passes an explicit null `isCopy` argument. Floating-point
  setters pass actual `f32`/`f64` values.
- Object/numeric results are discarded after a Java exception and the pending
  exception is cleared before subsequent calls.
- JVM discovery requires `GetModuleHandleA` and `JNI_GetCreatedJavaVMs` to
  return exactly one VM. PE scanning and JavaVM memory scanning are removed.
- Only `JNI_EDETACHED` triggers attachment. Other `GetEnv` errors stop
  initialization. A second `GetEnv` must return the same non-null environment.
- `JniBridge` no longer declares unsafe `Send`/`Sync`. It checks the owner
  thread before exposing its `JNIEnv`, and detaches only an attachment it owns.
- The JNI exception/stack-recovery gate is removed. The legacy render-only
  handler is installed after successful JNI initialization, not before it.
- Resolution uses known entrypoint loaders, the current thread context loader,
  and the system loader. `Thread.getAllStackTraces` and thread-group enumeration
  are no longer used during initialization.
- Initialization is enclosed in a local-reference frame. Missing game IDs stop
  initialization rather than starting the client with incomplete resolution.
- Bootstrap checkpoints and PID/thread information distinguish the new build
  and narrow the next failure, if any.

## Compatibility boundary

This patch deliberately removes heuristic/bypass access paths. It does not
provide a substitute for them. If the target runtime does not expose the
standard APIs or the required classes through the available loaders, the
client will not start. Expected errors include:

```text
JNI_GetCreatedJavaVMs unavailable; memory-scan fallback disabled
Standard Invocation API did not return exactly one JavaVM
Game IDs unavailable through standard JNI; initialization stopped
```

This is a controlled refusal to initialize, not proof of working gameplay
integration. Do not re-enable the old memory scans or stub decoding to force
startup.

## Verification performed

- Six isolated JNI-dispatch tests passed, covering argument positions, explicit
  string `isCopy`, float/double setters, method argument arrays, exception
  results, and null/invalid-name handling.
- The standalone smoke test passed on Amazon Corretto **21.0.12** on Linux,
  with **`-Xcheck:jni`** enabled and no CheckJNI warnings. It exercises
  `JNI_GetCreatedJavaVMs`, `GetEnv`, `GetVersion`, `FindClass`, exception
  clearing, Java strings, static/instance methods, float/double fields, and a
  worker thread's daemon attach/verify/detach lifecycle.
- Windows-target compilation is checked with:
  `cargo check --locked --all-targets --target x86_64-pc-windows-gnu`.

Run isolated tests and the Linux smoke test:

```sh
rustc --edition=2021 --test src/engine/jni_api.rs -o jni-api-tests
./jni-api-tests
rustc --edition=2021 tools/jni_smoke.rs -o jni-smoke
./jni-smoke /absolute/path/to/lib/server/libjvm.so
```

The smoke test creates its own ordinary JVM. It does not attach to an existing
game or reproduce the modified Windows runtime. Windows DLL linking, gameplay
execution, and the specific reported crash still require verification on the
user's machine.

## Next check on Windows

Rebuild from the new branch and restart the game before testing the new DLL.
The new startup line contains `JNI-safe starting`, PID, and thread information.
Capture everything from that line onward, especially:

```text
[JNI] Attached using standard Invocation API
[JNI] Bootstrap: GetVersion
[JNI] Bootstrap: FindClass(java/lang/Object)
[JNI] Bootstrap: resolve game class/field/method IDs
```

If a crash remains, provide the complete current JVM crash report or native
dump and the complete current initialization log, with private data redacted.
The old `VM recovered via memory scan` message must not occur in this build.

Remaining separate risks include `DllMain` thread startup, inline-hook
installation races, global-reference lifetime, game-field compatibility, and
the legacy render-only stack-recovery handler. This patch does not claim to
resolve those.

## References

- Oracle JNI function signatures:
  https://docs.oracle.com/javase/8/docs/technotes/guides/jni/spec/functions.html
- Oracle Invocation API and `GetEnv` error semantics:
  https://docs.oracle.com/javase/8/docs/technotes/guides/jni/spec/invocation.html