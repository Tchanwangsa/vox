# Vox

Hold a shortcut, speak, and the words are typed into whichever app is in
front. A dictation app for macOS in Rust and [GPUI](https://www.gpui.rs), in
the spirit of Wispr Flow, with local and Groq transcription.

| Avenue | Hears with | Tidies with |
| --- | --- | --- |
| On this Mac | OpenAI Whisper via whisper.cpp, or Cohere Transcribe via MLX | S1-mini by Superwhisper, through `llama-server` |
| Groq | `whisper-large-v3-turbo` | `qwen/qwen3.8-27b` |

Tidying drops fillers and false starts and fixes punctuation. With the
fallback on, the other avenue answers a step the chosen one cannot, and a
dictation whose tidy fails is typed as heard rather than lost.

## Run it

```sh
scripts/link-oculus.sh        # once: borrow Oculus's helpers and models by symlink
scripts/bundle.sh --open      # build target/Vox.app and launch it
```

Run it as the bundle, not `cargo run`: macOS grants the microphone and
Accessibility to the app that asks, and from a terminal that app is the
terminal.

On first use macOS asks for the microphone. To type into other apps Vox also
needs **System Settings → Privacy & Security → Accessibility**; until then
each dictation is copied to the clipboard instead.

- Hold `⌃⌥Space` to talk, let go to finish.
- Tap it to talk hands-free, press it again to finish.
- `Esc` cancels.

For Groq, copy an API key from [Groq Console](https://console.groq.com/keys) and press **Paste key**
on the Engine page. The key is checked against Groq, kept in the login
keychain, and Vox switches to Groq. `GROQ_API_KEY` in the environment is used when set.

## Local transcription

Open **Engine → On this Mac** and pick a model from the **Transcribe** menu.
It lists OpenAI Whisper (Large v3 Turbo quantised, Large v3 Turbo, Large v3,
and under *Smaller models* Medium, Small, Base, Tiny) and Cohere Transcribe
(4-bit, 8-bit, full precision), each marked as downloaded or with its download
size. One variant per family is marked **Recommended** for this Mac's memory,
by Oculus's rule: the default if it needs at most a quarter of the memory,
else the largest that does. A variant needing more than half of it is dimmed
(still selectable). Choosing one that isn't on this Mac starts its download
at once; the variant that was ready keeps transcribing until it finishes, and
**Cancel** stops the download.

Whisper uses `whisper-cli` and S1-mini uses `llama-server`. Run
`scripts/link-oculus.sh` to reuse Oculus's helpers and local models; any
`ggml-*.bin` in `models/whisper/` counts as downloaded. Engine can download
missing Whisper and S1-mini weights and show their progress.

Cohere Transcribe uses MLX conversions on Apple silicon with macOS 14 or later:
[4-bit](https://huggingface.co/lyzgeorge/cohere-transcribe-03-2026-mlx-4bit) (1.5 GB),
[8-bit](https://huggingface.co/aufklarer/Cohere-Transcribe-2B-MLX-8bit) (2.4 GB) and
[full precision](https://huggingface.co/aufklarer/Cohere-Transcribe-2B-MLX-FP16) (4.1 GB),
each pinned to a revision that loads in the pinned `mlx-audio`. Install
[uv](https://docs.astral.sh/uv/getting-started/installation/) if needed, then
pick a variant in Engine. Vox downloads its weights plus the runtime once.
Cohere runs offline after setup, and uses English when the language setting is
`auto`. Turn off **Automatic fallback** if you want to keep audio on your Mac
when local transcription fails.

## Where things live

`~/Library/Application Support/com.tchan.vox/`

- `bin/whisper-cli`, `bin/llama-server`: the local helpers (also found beside
  the executable or in Homebrew's `bin`)
- `models/whisper/ggml-*.bin`, `models/whisper/ggml-silero-v6.2.0.bin`
- `models/s1-mini/s1-mini-q4_k_m.gguf`
- `models/cohere/` for the Cohere runtime and weights (`weights/` is the 4-bit,
  `weights-8bit/` and `weights-fp16/` the others)
- `settings.json`, `history.json`
- `stats.json`: words, speaking time and dictations per day, and words by
  app, for the Stats page. It keeps every day; the history keeps the latest
  300 dictations
- `app-icons/`: the icons of the apps you dictate into, shown on Stats
- `recordings/`: every dictation, kept to tune the models on later

## Recordings

With **Settings → Save recordings locally** on (the default), each finished
dictation is saved under `recordings/`:

- `audio/<id>.wav`: the clip exactly as the engine heard it, 16 kHz mono
  16-bit PCM
- `manifest.jsonl`: one line a clip, appended and never rewritten

```json
{"id":"1764061200123","at":1764061200,"audio":"audio/1764061200123.wav","seconds":4.2,
 "language":"auto","heard":"um send the q4 report","text":"Send the Q4 report.",
 "heard_by":{"avenue":"local","model":"ggml-large-v3-turbo-q5_0"},
 "tidied_by":{"avenue":"local","model":"s1-mini-q4_k_m"}}
```

`heard` is the transcript and `text` is what was typed, so the pairs
`audio → heard` and `heard → text` are the two things to tune: the hearing
model and the tidying model. Both are the models' own output, not ground
truth; a clip is only training data once someone has corrected it. Cancelled
dictations and ones where nothing was heard are not kept.

## Layout

- `src/state.rs`: the dictation state machine the shortcut drives
- `src/audio.rs`: microphone capture to a 16 kHz WAV, and the waveform levels
- `src/engines/`: `groq.rs`, `whisper.rs`, `cohere.rs`, `s1.rs`, `models.rs` (the model menu's catalogue and fit rule), and the pipeline with its fallback in `mod.rs`
- `src/hotkey.rs`, `src/paste.rs`, `src/keys.rs`: the global shortcut, ⌘V into the app in front, the keychain
- `src/recordings.rs`: the kept clips and their manifest
- `src/ui/`: `main_view.rs` (Home, Engine, Settings) and `pill.rs` (the floating capsule)

## Developing

```sh
cargo test
vox --hear clip.wav whisper            # one 16 kHz WAV through the pipeline, no app
vox --hear clip.wav cohere --as-heard  # or a variant: whisper_medium, cohere_8bit, ...
vox --install-model cohere_8bit        # also accepts whisper, cohere, a variant, or s1
vox --hear clip.wav cloud --as-heard
VOX_TAB=engine VOX_DEMO=listening cargo run   # open on a page, with the pill in a state
VOX_TAB=engine VOX_MENU=open cargo run        # with the Transcribe model menu open
VOX_HEIGHT=1400 cargo run                     # a window tall enough to see a whole page
VOX_DATA_DIR=/tmp/vox cargo run               # a throwaway data directory (also skips Keychain)
```

`cargo run` signs the debug binary with the same Apple Development identity and `com.tchan.vox` identifier as `target/Vox.app` (`scripts/run.sh` → `scripts/sign.sh`), so macOS Keychain trusts it across rebuilds without prompting. Isolated runs (`VOX_DATA_DIR`, `VOX_DEMO`, or `VOX_NO_KEYCHAIN`) never touch the login keychain; pass `GROQ_API_KEY=gsk_demo1234` when mocking a saved key.

GPUI is built with `runtime_shaders`, so Xcode's Metal toolchain is not needed.

## Not yet

- Recording a custom shortcut, or a modifier-only one such as Fn
- Correcting a kept recording's text, which is what turns it into training data
- A menu bar item, launch at login, a personal dictionary
