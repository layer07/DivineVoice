# tests/fixtures

`voice_note.ogg`: the first 6 s of `bench/01_altermann.mp3`, re-encoded the way
WhatsApp sends voice notes (Ogg Opus, mono, 16 kHz, 16 kbps, VoIP mode):

```
ffmpeg -i bench/01_altermann.mp3 -t 6 -ac 1 -ar 16000 -c:a libopus -b:a 16k -application voip -vbr on voice_note.ogg
```

Derived from an OmniVoice clip, so it carries the same **CC-BY-NC-4.0** terms as
`bench/` (see `bench/README.md`). Not baked into the exe: only the tests read it.
