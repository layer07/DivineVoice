# bench/: benchmark clips

Every clip here is baked into the exe at build time and shows up as a tab on
**Settings ▸ Benchmark**. You don't need to touch any code: drop the files in
and run `cargo build`.

A clip is two files with the same name:

| file | what |
|---|---|
| `<name>.mp3` | the audio. `.wav`, `.flac`, `.ogg` and `.m4a` work too |
| `<name>.txt` | **exactly** what the voice says. This is the answer key the judge grades against |

The tab label comes from the file name: `ill-met-by-silicon` becomes *Ill Met By Silicon*.

## Writing the script (`.txt`)

The judge ignores case, punctuation, hyphens and curly quotes. It spells out
digits on both sides and merges split compounds ("run book" = "runbook"), the
same way the Open ASR leaderboard does. Two rules are still on you:

- **The text must match the audio word for word.** If the TTS says "O of log n",
  the script says "O of log n", not "O(log n)".
- **No stage directions** or anything else the voice doesn't say.

Aim for 80–150 words (30–60 s). A clip with no `.txt` is skipped, with a build warning.

## Your own clips, no rebuild

The same pairs dropped in `%APPDATA%\DivineVoice\bench\` appear at runtime. If
one has the same name as a built-in clip, yours wins.

## Licensing

Everything in this folder ships in a GPL-2.0-or-later repo. Only add audio whose
license allows that: your own voice, Apache/MIT/CC-BY TTS output (Kokoro), or a
paid service whose terms allow redistribution. **Never** commercial recordings.

## From the terminal

```
divinevoice bench --clip <name> --model parakeet-ultra    # one model, graded, timed
divinevoice race  --clip <name>                           # every installed model, JSON lines
```
