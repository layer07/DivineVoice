# macOS port plan

## Goal

Run DivineVoice as a native macOS menu-bar app on Apple Silicon and Intel Macs running macOS 13 or later. It must record from the microphone, transcribe locally, and deliver text to the app that had focus when recording began.

The Windows release must keep its current behavior.

## Scope

The shared Rust code already covers audio buffering, transcription, model management, configuration, history, benchmarks, and the embedded settings UI. The port replaces the Windows-specific parts in `src/hotkey.rs`, `src/inject.rs`, `src/autostart.rs`, `src/tray.rs`, `src/ui.rs`, and the Windows-only parts of `src/bus.rs`.

The first macOS release supports CPU inference, paste delivery, Unicode delivery, tray controls, settings, and optional autostart. CUDA, DirectML, the Interception driver, and `Type` delivery remain Windows-only.

## Design

Add one `platform` module with a small interface for operating-system behavior. Its adapters own hotkey capture, text delivery, autostart, permission checks, and platform-specific tray/window work. The recording, transcription, and state modules call this interface instead of calling Windows APIs directly.

This seam keeps permissions and native event handling in one place. Shared code should not need to know whether it is running on Windows or macOS.

## Prerequisites

1. Install Rust with `brew install rust`.
2. Confirm `cargo`, `rustc`, and Xcode command-line tools are available in a new terminal.
3. Keep at least 25 GB free for the Rust toolchain, build artifacts, and one model.

Check:

```sh
cargo --version
rustc --version
xcodebuild -version
df -h .
```

## Phase 1: make platform code explicit

1. Add compile-time platform selection and a `platform` module.
2. Move Windows implementations behind the module without changing their behavior.
3. Add a macOS placeholder adapter that reports unsupported operations clearly.
4. Update Cargo dependencies only where the macOS adapter needs them.

Checks:

```sh
cargo fmt --check
cargo test
cargo build
```

Acceptance: the existing Windows build still compiles, and a macOS build reaches the app startup path without importing Windows-only symbols.

## Phase 2: native macOS app shell

1. Make the Tao/Wry tray and settings window compile on macOS.
2. Replace WebView2-only assumptions with the WebKit-backed macOS implementation.
3. Show platform and permission status in Settings.
4. Keep all UI content local in the embedded `dv://` protocol.

Checks:

```sh
cargo build
cargo run -- --settings
```

Acceptance: the app appears in the menu bar; Settings opens, closes to the menu bar, and reopens without a crash.

## Phase 3: microphone permissions and recording

1. Request and report Microphone permission.
2. Verify CPAL discovers macOS input devices and opens the selected device.
3. Keep the current recording limits and error handling.

Manual check:

1. Deny Microphone permission and start recording.
2. Confirm the app shows a clear error and stays running.
3. Grant permission in System Settings, record a short clip, and confirm the audio pipeline receives samples.

Acceptance: permission state is correct before and after granting access, and a short recording reaches the shared transcription queue.

## Phase 4: global hotkey

1. Implement a macOS Core Graphics event tap for the configured hotkey.
2. Request and report Input Monitoring permission.
3. Pass only the configured hotkey to the recorder. Do not persist or log ordinary keystrokes.
4. Preserve hold and toggle modes.

Tests:

1. Unit-test chord recognition through the shared hotkey state machine.
2. On a real Mac, bind `Ctrl+Space`, hold it, and confirm one start and one stop event.
3. Deny Input Monitoring permission and confirm the app reports the problem without swallowing keys.

Acceptance: the configured chord starts and stops one recording; unrelated input behaves normally.

## Phase 5: text delivery

1. Implement paste delivery with `NSPasteboard` and a synthetic Command+V event.
2. Restore text clipboard contents after the configured settle time.
3. Implement Unicode delivery with synthetic Core Graphics keyboard events.
4. Mark `Type` as unavailable on macOS rather than loading the Windows Interception driver.
5. Request and report Accessibility permission.

Manual check:

1. Put known text on the clipboard.
2. Dictate into TextEdit and check that the transcript appears.
3. Confirm the original clipboard text returns.
4. Repeat in Terminal using paste-plain mode.
5. Deny Accessibility permission and confirm delivery fails clearly instead of pretending to succeed.

Acceptance: paste and Unicode delivery work in TextEdit, clipboard restoration works, and permission failures are visible.

## Phase 6: models and CPU transcription

1. Build and run the existing CPU ONNX path on Apple Silicon.
2. Download one small model first, `parakeet-unified-int8`.
3. Test the default model after the basic path works.
4. Confirm downloads happen only after the user requests them.

Checks:

```sh
cargo run -- models
cargo run -- fetch parakeet-unified-int8
```

Acceptance: a downloaded model loads, a recorded phrase transcribes locally, and its text is delivered to TextEdit.

## Phase 7: autostart

1. Add a macOS autostart adapter using a per-user LaunchAgent plist.
2. Create or remove only DivineVoice's own plist when the setting changes.
3. Show the actual registered state in Settings.

Manual check:

1. Enable autostart and inspect the created plist.
2. Disable it and confirm the plist is gone.
3. Log out and back in to confirm only one instance starts.

Acceptance: enabling and disabling autostart changes no unrelated LaunchAgent files.

## Phase 8: release validation

1. Build release artifacts for Apple Silicon and Intel.
2. Test on a clean macOS user account.
3. Add Developer ID signing and notarization before public distribution.
4. Document the three required permissions: Microphone, Input Monitoring, and Accessibility.

Checks:

```sh
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
```

Acceptance: a clean Mac can install the signed app, grant permissions, download a model, dictate into a standard app, and pass Gatekeeper.

## Out of scope for the first port

- Windows GPU backends: CUDA and DirectML.
- Interception driver support and physical-HID `Type` delivery.
- A Windows-to-macOS feature rewrite unrelated to the platform port.
- Notarization without an Apple Developer account and signing credentials.
