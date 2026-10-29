# Changelog

## 0.3.1 · 2026-10-29

- **Fix: fresh installs couldn't download the default model.** The config
  points at Parakeet Ultra before it exists, and the Models page showed it as
  ACTIVE instead of offering Download. A model is now active only once it's on
  disk; until then nothing is, and the status says "No model yet".

## 0.3.0 · 2026-10-28

**GPU, more models, receipts.**

- **CPU, CUDA or DirectML**, one click on Core (CPU stays the default). GPUs are
  probed for real at startup; one that can't come up is greyed out with the
  reason in English, never silently run on the CPU.
- **DirectML first-shape fix.** DirectML only runs fast on the first input
  length it ever sees. GPU clips are now padded to a fixed 6 s bucket, and the
  GPU is primed while you hold the hotkey. Parakeet v3 on an RTX 3080: 304 →
  92 ms per real phrase, ahead of an i9-14900KF's 125 ms.
- **Three new models:** Cohere Transcribe (2B, fewest errors in our race),
  Nemotron 3.5 (40 locales), Nemotron Streaming English. Seven total.
- **ⓘ on every model:** maker, date, size, license, languages, published WER,
  our measured numbers, *pick it if / skip it if*.
- **Language setting** (Cohere needs one; Nemotron 3.5 likes one).
- **Speed tab:** every model, CPU vs DirectML, measured with real phrase
  lengths and pauses.
- **Benchmark:** one model or all, on any set of devices, CPU-vs-GPU board.
- **Start with Windows** (Core, or the right-click menu).
- **40 themes** on the titlebar: hover previews live, click keeps it.
- **Right-click menu** of our own: device, model, benchmark, last transcript.
- **Fixes:** a fresh install no longer sits at "no model" after its first
  download; re-picking a failed model retries; concurrent settings writes can't
  undo each other; GPU races park the dictation model instead of fighting it
  for VRAM; a GPU that dies mid-clip re-runs your sentence on CPU.
- `bench --device --first --gap --press --json`, `race --models --devices`.

## 0.2.0 · 2026-10-28

**First public cut.** Rewritten from a private tool.

- Push-to-talk dictation: hold, talk, let go, text lands at your cursor.
- NVIDIA Parakeet on the CPU via parakeet-rs / ONNX Runtime; Ultra, Unified
  and v3 in a one-click catalogue with resumable downloads.
- Paste / Paste plain / Unicode / Type (Interception HID scancodes: survives
  Parsec and hardware KVMs).
- Benchmark tab: the same tape through every model, word-by-word graded reveal,
  letter grade, leaderboard. Two built-in clips.
- Perf tab: release → text latency, p50/p95, ×real-time, cold-start tax.
- Frameless settings window, tray-first, no Electron.
