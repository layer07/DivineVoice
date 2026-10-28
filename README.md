<p align="center">
  <img src="ui/mascot.webp" width="120" alt="the DivineVoice mascot, a pink pixel devil-cat, mid-jump">
</p>

<h1 align="center">DIVINE<i>//</i>VOICE</h1>

<p align="center"><b>Hold the shortcut. Talk. Let go. Text appears where your cursor is.</b></p>

<p align="center">
  Local push-to-talk dictation for Windows · NVIDIA Parakeet on your CPU or GPU ·
  no cloud, no account, no telemetry, no Electron
</p>

```
you:   [holds Ctrl+Space] "deploy it to staging and page me if it catches fire" [lets go]
~150 ms later, in whatever window had focus:
       deploy it to staging and page me if it catches fire
```

> **Cave edition:** Finger hold key. Mouth make words. Finger let go. Words appear.
> No cloud see words. CPU already awake. GPU want fight? Race. Loser go home.

## Why it exists

- **Fast.** On an i9-14900K, a 3 s clip is typed out about 100 ms after you let
  go. 53 s of audio transcribes in 1.6 s (34× real-time) on 8 threads. The Perf
  page measures your own numbers, live.
- **Multilingual.** Parakeet v3 auto-detects 25 European languages. Speak
  English, switch to Portuguese, and it keeps up.
- **Crosses boundaries other dictation tools can't.** With the optional
  Interception driver it types real HID scancodes. Those survive Parsec and a
  hardware KVM, where clipboard paste would hand the remote box yesterday's clipboard.
- **Hard to break.** A crashed transcriber respawns. A stalled hook can't take
  your keyboard hostage. A wedged clipboard falls back to typing. A typo in your
  config gets logged, not "fixed" by overwriting your file.

## Quick start

1. **Get the binary.** Build it (see below), or grab `divinevoice.exe` from Releases.
2. **Run it.** On first run there's no model yet, so Settings opens on the
   **Models** page. Click **Download** on *Parakeet Ultra* (~2.5 GB, one time;
   English only and short on disk? *Parakeet Unified · int8*, 660 MB). The
   download resumes if your Wi-Fi drops, and the model is selected automatically
   when it finishes. Headless box? `divinevoice fetch parakeet-ultra`.
3. **Talk.** Focus any text box, hold **Ctrl+Space**, talk, let go.

The tray icon is a status light: idle → **recording** → transcribing → typing.
Left-click it for Settings, right-click for the menu. The Settings window is
frameless and a fixed size. Its **—** and **✕** buttons both send it to the
tray, so the app keeps listening. **Quit lives in the tray menu, and only there.**
`divinevoice --settings` opens Settings straight away.

## Settings

| Page | What's in it |
|---|---|
| **Core** | Live status · **rebind the shortcut** (click *Rebind*, press any chord; Esc cancels) · Hold vs Toggle · **engine: one-click CPU ⇄ CUDA ⇄ DirectML, and the language** · output method |
| **Models** | Download, switch and delete models. Accuracy/speed bars, resumable downloads with live progress, or point at your own ONNX folder. **ⓘ on every model** opens its papers: who made it, when, size, architecture, license, languages, published WER, what our race measured, and a plain *pick it if / skip it if* |
| **Audio** | Pin a mic by name · min/max clip length |
| **Perf** | The last 64 clips, timed from hotkey release to text delivered. p50/p95 latency, × real-time, words/s, your speaking pace in wpm, cold-start tax. A per-run bar chart with a stage breakdown (resample / queue / ASR / deliver). Numbers only, never your words |
| **Benchmark** | Pick a clip, pick **one model or all installed**, pick **CPU, CUDA and/or DirectML**, hit **▶ Run**: the same recorded tape runs through every model × device, your active model first. Its verdict is revealed word by word in a terminal with a shimmer, with substitutions, misses and insertions marked, plus accuracy, WER, ASR time, × real-time and a letter grade. The rest race onto a leaderboard with a device chip and a *3.4× vs CPU* line on GPU rows; a GPU that can't come up gets a failed row saying why, never CPU numbers in a GPU costume. Click a row to see what that model heard. Optional play-along through the speakers, synth sound effects. Clips live in [`bench/`](bench/README.md) (baked in) or `%APPDATA%\DivineVoice\bench` (yours, no rebuild). Prefer your own voice? *Read it yourself* arms the hotkey against the clip's script |
| **History** | Opt-in list of recent transcripts, with copy and delete |
| **Advanced** | **Device (CPU / CUDA / DirectML)** · ONNX threads · typing delays · Interception skip list · fbdirector bus · log level |

Two things live outside the pages. The **swatch button on the titlebar** is
the theme picker: 40 themes from the ElectricWizard set, each shown as its
accent / surface / text stripes. Hover one and the whole app repaints live;
move off and it snaps back; click to keep it. **Right-click** anywhere gives
our own menu instead of a browser's: device switch, model switch, *Benchmark
this setup*, copy the last transcript, logs, minimise / close to tray. Text
fields keep the normal cut/copy/paste menu. Esc minimises to the tray.

Every change saves to `config.toml` and applies live. A new hotkey works on the
next keypress, and a new model loads in the background.

### Models

| Model | Size | Languages | Engine | What it is |
|---|---|---|---|---|
| **Parakeet Ultra** *(default)* | 2.5 GB | 25 European | TDT | [Moondream's post-trained v3](https://huggingface.co/moondream/parakeet-ultra) |
| Parakeet Unified · int8 | 660 MB | English | RNNT | [NVIDIA parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b) (2026), quantized |
| Parakeet Unified | 2.5 GB | English | RNNT | Same, full precision |
| Parakeet v3 | 2.5 GB | 25 European | TDT | Stock [NVIDIA v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3). The reference |
| Cohere Transcribe · int8 | 3.1 GB | 14, **must be set** | AED | [Cohere's 2B model](https://huggingface.co/CohereLabs/cohere-transcribe-03-2026), #1 on the Open ASR leaderboard at release (5.42%). Apache-2.0 |
| Nemotron 3.5 | 2.6 GB | 40 locales | RNNT·S | [NVIDIA's multilingual streaming model](https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b): Japanese, Chinese, Korean, Arabic, Hindi, Turkish… |
| Nemotron Streaming · English | 2.5 GB | English | RNNT·S | [NVIDIA's low-latency English model](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b), 530k hours of training |

Our own race: a 53 s English clip, 8 threads on an i9-14900K, warm p50.

| | v3 | **Ultra** | Ultra int8 (cut) | Unified | Unified int8 |
|---|---|---|---|---|---|
| time | 1593 ms | **1567 ms** | 1379 ms | 1730 ms | 1474 ms |
| word errors | 1 | **0** | 4 | 1 | 1 |

The only int8 Ultra export we could find lost words to its quantization, so the
catalogue ships Ultra in full precision. Unified's int8 build lost nothing.

**Ultra** is v3 post-trained by Moondream. It has the same architecture,
tokenizer and languages, and Moondream reports fewer errors everywhere: English
6.26 → 5.80 % WER on the Open ASR leaderboard sets, the 25-language FLEURS set
11.62 → 9.55 %, Portuguese 4.65 → 3.96 %. **Unified** is NVIDIA's newest English
model: 5.91 % offline WER vs 6.04 for v2 and 6.34 for v3. It's an RNNT, so it
runs on a second engine path. Both engines come from
[parakeet-rs](https://github.com/altunenes/parakeet-rs), and DivineVoice picks
the right one from the files in the folder.

**Language.** Parakeet TDT models detect the language per clip on their own,
and English-only models don't care. **Cohere can't detect at all**: set Core ▸
language, or it transcribes as English. Nemotron 3.5 detects, but is more
accurate when you pin it. Clips longer than 35 s are cut at the quietest
moment for Cohere (it was trained on ≤35 s) and glued back together.

Skipped on purpose: an int8 Nemotron that's under a non-compete license
(PolyForm Shield), and Cohere fp32 (7.6 GB; int8 is the sane download).

We don't host weights. Each model downloads from the ONNX export credited on
its card (hover the blurb). Any other Parakeet TDT or Unified ONNX folder works
too: Models ▸ *bring your own*.

```
divinevoice models                    # list the catalogue, sizes, what's installed
divinevoice fetch parakeet-ultra      # download one without opening the UI
divinevoice race --clip 01_altermann --models all --devices cpu,directml   # model × device, JSON lines, pipe to jq
```

`fetch` and the Models page use the same downloader. Each file streams to a
`.part` file, resumes with HTTP Range, and is checked against the exact size
HuggingFace reports.

### Output modes

| Mode | How | Reaches | Characters |
|---|---|---|---|
| **Paste** *(default)* | clipboard + Ctrl+V, then your clipboard is restored | this PC | everything |
| **Paste plain** | clipboard + Ctrl+Shift+V | terminals that think Ctrl+V means something else | everything |
| **Unicode** | `SendInput` + `KEYEVENTF_UNICODE`, char by char | this PC, clipboard untouched | everything |
| **Type** | set-1 scancodes through the Interception driver | this PC · Parsec · hardware KVM | US-QWERTY ASCII |

Paste is local only. Over a KVM it sets **this** machine's clipboard and then
presses Ctrl+V on the **other** one, which pastes whatever that box had copied.
Only real keystrokes carry the actual text across.

If Paste can't get the clipboard, or Type can't find Interception, the text
goes out through Unicode instead. A transcript is never silently dropped.

#### Optional: Interception (for Type mode)

[Interception](https://github.com/oblitum/Interception) is a kernel keyboard and
mouse filter driver. Keys sent through it can't be told apart from real ones.

1. Run `install-interception.exe /install` as admin, then reboot.
2. Put `interception.dll` (from `library\x64\`) next to `divinevoice.exe`.

It isn't bundled. It's a third-party kernel driver with its own license, and
installing one should be your call. Without it, everything works except Type,
which falls back to Unicode. The DLL is loaded at runtime, so a missing DLL
can't stop the app from starting.

## Config

`%APPDATA%\DivineVoice\config.toml` is created on first run (Settings ▸
Advanced ▸ *config.toml*). Any key you leave out takes its default. If the file
has a syntax error, DivineVoice logs it, runs on defaults, and **leaves your
file alone**.

```toml
hotkey          = "Ctrl+Space"  # any chord: "Ctrl+Shift+F13", "RAlt", "Pause"…
hotkey_mode     = "hold"        # hold (walkie-talkie) | toggle (tap on, tap off)
model_dir       = 'C:\path\to\model'
inject_mode     = "paste"       # paste | paste_plain | unicode | type
device          = "cpu"         # cpu | directml (any DX12 GPU) | cuda (NVIDIA). A GPU that won't come up falls back to CPU, and says why
language        = "auto"        # or "en", "pt", "ja"… Cohere needs it; Nemotron 3.5 likes it; Parakeet ignores it
theme           = "default"     # any id from ui/themes.js: lain, matrix, phosphor-amber, light-sakura…
# input_device  = "QuadCast"    # pin a mic by name substring; omit = Windows default
min_ms          = 250           # shorter clips = accidental tap, discarded
max_ms          = 120000        # hard cap per clip
paste_settle_ms = 80            # how long Ctrl+V gets before your clipboard is restored
type_delay_ms   = 5             # gap between chars in Type/Unicode. 0 = flat out
intra_threads   = 8             # ONNX Runtime threads. Default: cores/2, max 8
skip_hwids      = []            # Interception hw-ids Type must never use (your KVM's own HID gadget)
# fbdirector    = "127.0.0.1:17879"  # optional input-sharing daemon handshake
log_level       = "info"        # "debug" also logs transcript text
history         = false         # keep recent transcripts in history.json
history_max     = 100
perf_max        = 64            # runs kept on the Perf page
```

A hotkey that would make a key unusable (plain `A`, plain `Space`) is refused,
both in the UI and in the file.

Logs go to `%APPDATA%\DivineVoice\divinevoice.YYYY-MM-DD.log`, rotated daily
and kept for 7 days. Transcripts are **not** logged unless you set
`log_level = "debug"`. A plaintext file of everything you've ever said is a bad default.

## CPU or GPU

**CPU is the default** because it always works, and a fat desktop CPU is already
25-40× real-time. Push-to-talk is one short clip, then a minute of nothing; a
GPU spends that minute clocking down and pays a ramp plus host↔device copies on
every press. That's why the first version shipped CPU-only, and it was right
for the CUDA build we had then.

Then DirectML on an RTX 3080 did this to a 40 s clip (`race`, warm median):

| | CPU (i9, 8 threads) | DirectML (RTX 3080) | |
|---|---|---|---|
| Parakeet v3 | 976 ms · 41× | **288 ms · 138×** | 3.4× faster |
| Parakeet Ultra | ~1600 ms · 25× | 1096 ms · 36× | 1.5× faster |

Same model, same words, same WER. Speed is the only thing that changes.

Not every model likes a GPU, though. From racing all seven on both built-in
clips (warm median, 8 threads vs the 3080):

| | WER (two clips) | CPU | DirectML |
|---|---|---|---|
| Parakeet v3 | 3.4% · 2.6% | 40× | **123–145×** |
| Parakeet Ultra | 3.4% · 2.6% | 17–39× | 31–32× |
| Unified · int8 | 3.4% · 3.8% | 45× | 25–36× |
| Nemotron 3.5 | **1.7% · 2.6%** | 7–8× | 4–5× (slower) |
| Nemotron EN | 1.7% · 3.8% | 8× | slower |
| Cohere Transcribe | **1.7% · 2.6%** | 2× | refused: its attention op breaks DirectML |

Known-bad model × device pairs are refused up front with the reason, and the
device buttons grey out for the model you have loaded. Every model's ⓘ shows
these numbers. So now
you pick (Advanced ▸ Engine), and you **measure instead of guess**: Benchmark ▸
choose models ▸ toggle CPU + DirectML ▸ Run.

| Device | Works on | Needs |
|---|---|---|
| `cpu` | everything | nothing |
| `directml` | any DirectX 12 GPU: NVIDIA, AMD, Intel | nothing: `DirectML.dll` ships with Windows 10 1903+ |
| `cuda` | NVIDIA | driver **580+** (CUDA 13), **cuDNN 9**, and the `onnxruntime_providers_*.dll` files next to the exe |

Honesty rules, enforced in code:

- **GPUs are probed for real at startup.** ONNX Runtime's default is to quietly
  run on the CPU when a GPU provider fails to load, and then you're
  "benchmarking the GPU" on the CPU. DivineVoice builds a tiny session with
  `error_on_failure` on each GPU. Unavailable devices are greyed out with the
  reason in English: *NVIDIA driver too old for this CUDA build*, not
  `CUDA failure 35: (null)`.
- **Benchmarks load strict.** A GPU that can't come up is a failed row.
- **Dictation loads forgiving.** If the GPU can't come up, it runs on CPU and
  the status bar says why. If the GPU dies **mid-clip** (VRAM blowout), the
  same clip is re-run on CPU, so your sentence survives.
- **VRAM is shared fairly.** A GPU race *parks* the dictation model (unloads
  it, status `parked`) so two 2.5 GB models don't fight over your card, then
  reloads it when the race ends. That holds even if the race thread panics.

The other half of the cold start: ONNX Runtime does its real setup on the
*first inference*, not at load time. So DivineVoice runs one second of silence
through the model right after loading it, and your first sentence of the day
skips that cost. The Perf page's *cold tax* tile shows whether that worked.

The other half of the cold start: ONNX Runtime does its real setup on the
*first inference*, not at load time. So DivineVoice runs one second of silence
through the model right after loading it, and your first sentence of the day
skips that cost. The Perf page's *cold tax* tile shows whether that worked.

Measure your own machine:

```
divinevoice bench some-speech.mp3 --thread-scan 1,2,4,6,8,12,16
divinevoice bench some-speech.mp3 --model parakeet-unified --text   # any model, full transcript
divinevoice bench --clip 02_leftmost_child --device directml         # same, on the GPU
```

That prints load, cold and warm timings (p50/p95), real-time factor and scaling
efficiency per thread count. Put the knee in `intra_threads`. `--model` takes a
catalogue id or a folder, so you can race models on your own voice. `--device`
takes `cpu`, `cuda` or `directml`.

## Build

Windows 10/11 x64, stable Rust 1.85+.

```powershell
cargo build --release      # → target\release\divinevoice.exe
cargo test                 # pure cores: hotkey, binding, dsp, keymap, perf, ipc…
```

ONNX Runtime is linked statically by `ort` (via `parakeet-rs`), so the exe
runs alone on CPU and DirectML. CUDA is the exception: its provider lives in
`onnxruntime_providers_cuda.dll` + `onnxruntime_providers_shared.dll`, which
must sit next to the exe. In a dev build those land in `target\release` as
**symlinks** into ort's download cache, so a release copies the real files.
`cargo build --release --no-default-features` builds CPU-only (smaller, no GPU
code at all). The other optional DLL is `interception.dll`.

The settings UI is plain HTML/CSS/JS in `ui/`, baked into the exe with
`include_bytes!` and served over a custom `dv://` scheme to WebView2, which
ships with Windows 10/11. There's no npm, no bundler and no `node_modules`. Edit
the files and run `cargo build`. Colors come from `ui/theme.css` (corpo-doom),
and `app.css` uses only those tokens.

> Building from Git Bash? Its coreutils `link` can shadow MSVC's `link.exe`.
> Build from PowerShell or a VS developer prompt.

## How it works

```
 hotkey ──Cmd──► audio ──Clip──► asr ──Transcript──► inject ──► perf.json
 LL hook         cpal/WASAPI     Parakeet TDT/RNNT    paste · unicode · HID
                 downmix in cb   (CPU or GPU, ORT,
                 resample once    warm-up, hot-swap,
                 on release       parks for GPU races)

 every Clip/Transcript carries a Stopwatch started at hotkey release

 ui (main thread): tao loop · tray · frameless WebView2 settings ⇄ ipc
 watchdog · bus (optional) · download (on demand)
```

| File | Job |
|---|---|
| `main.rs` | Wiring, thread supervision, watchdog, logging, single instance |
| `hotkey.rs` | `WH_KEYBOARD_LL` hook. Hold/toggle and "learn a new chord" are pure, unit-tested `step()` functions |
| `binding.rs` | `"Ctrl+Shift+F13"` ⇄ (mods, VK). Parse, format, sanity checks. Pure |
| `audio.rs` | Mic capture, generic over every cpal sample format |
| `dsp.rs` | `downmix`, `resample` (boxcar decimator / linear interpolator), `split_at_quiet` (long clips cut between words). Pure |
| `asr.rs` | Model load (TDT or Unified engine) on CPU/CUDA/DirectML, honest GPU probe, warm-up, hot swap, parking, GPU→CPU rescue, inference |
| `models.rs` | Catalogue with each model's papers (facts for the ⓘ panel), engine detection, resumable HuggingFace downloader |
| `perf.rs` | Per-clip stopwatch, 64-entry ring, pure p50/p95/xRT/wps/wpm stats |
| `wer.rs` | The judge: word-level Levenshtein → ok/sub/del/ins + WER. Pure |
| `race.rs` | Replays one clip or take through the chosen models × devices, active first, one in RAM at a time |
| `clips.rs` | Benchmark clips (baked from `bench/` by `build.rs`, or yours from `%APPDATA%`) + the audio decoder |
| `history.rs` | Opt-in transcript history |
| `inject.rs` | The four output modes and the fallback ladder |
| `keymap.rs` | char → set-1 scancode as a keyboard-shaped table. Pure |
| `interception.rs` | `LoadLibrary` FFI to interception.dll (four symbols, no import lib) |
| `state.rs` | Pipeline mode byte (CAS transitions), model status, download state |
| `ui.rs` | Event loop, tray wiring, frameless on-demand WebView2 window, embedded `dv://` assets |
| `ipc.rs` | Every command the page can send. Config writes are a serde round-trip, so type checks come free |
| `tray.rs` | Tray icon and menu |
| `bus.rs` | Optional fbdirector handshake |
| `bench.rs` | The benchmark subcommand |
| `ui/` | `index.html`, `theme.css` (corpo-doom tokens), `themes.js` (40 baked skins), `app.css`, `app.js` (no framework, pure views), `mascot.webp` |

House rules:
- The realtime callback does nothing clever.
- Every decision that can be a pure function is one, and has tests.
- Nothing in the keyboard path can block your keyboard.

## Known limits

- Windows only. It's Win32 all the way down: LL hooks, SendInput, WebView2, tray.
- Hotkeys are keyboard only, no mouse side buttons yet. The LL keyboard hook can't see them.
- Downloads are checked by exact byte size, not by hash.
- GPU devices are probed once at startup. Updated your driver? Restart DivineVoice.
- Big models on small cards: Ultra in fp32 wants ~3 GB of free VRAM on DirectML.
  If your browser and game already ate it, you'll see *out of GPU memory*:
  that's the card, not the model. Use v3 / Unified int8, or CPU.
- Type mode is US-QWERTY ASCII. Anything else goes through Unicode or Paste.
- Unicode/Paste can't type into elevated (admin) windows unless DivineVoice is
  elevated too. That's UIPI, Windows working as designed.
- Paste restores text clipboards only. If you had an image copied, it's gone.

## License & credits

**GPL-2.0-or-later.** See [LICENSE](LICENSE). Every source file carries the SPDX tag.

- Speech model: [NVIDIA Parakeet TDT 0.6B](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3), CC-BY-4.0,
  via [parakeet-rs](https://github.com/altunenes/parakeet-rs) and istupakov's ONNX exports.
- HID driver: [Interception](https://github.com/oblitum/Interception) by oblitum. Optional, not distributed here.

---

<p align="center">
  <b>Magnus Leatti</b> (layer07) · <a href="https://kernelriot.com">kernelriot.com</a> · <a href="https://github.com/layer07">github.com/layer07</a>
</p>

```
        ██▓    ▄▄▄     ▓██   ██▓▓█████  ██▀███
       ▓██▒   ▒████▄    ▒██  ██▒▓█   ▀ ▓██ ▒ ██▒
       ▒██░   ▒██  ▀█▄   ▒██ ██░▒███   ▓██ ░▄█ ▒
       ▒██░   ░██▄▄▄▄██  ░ ▐██▓░▒▓█ ▄ ▒██▀▀█▄
       ░██████▒▓█   ▓██▒ ░ ██▒▓░░▒████▒░██▓ ▒██▒
       ░ ▒░▓  ░▒▒   ▓▒█░  ██▒▒▒ ░░ ▒░ ░░ ▒▓ ░▒▓░
       ░ ░ ▒  ░ ▒   ▒▒ ░▓██ ░▒░  ░ ░  ░  ░▒ ░ ▒░
         ░ ░    ░   ▒   ▒ ▒ ░░     ░     ░░   ░
           ░  ░     ░  ░░ ░        ░  ░   ░
```
