<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/assets/banner-dark.svg">
  <img src=".github/assets/banner-light.svg" alt="Vox" width="720">
</picture>

<p>Hold a shortcut, speak, and type cleaned-up words into whichever app is in front.</p>

<p>
  <a href="https://github.com/Tchanwangsa/vox/releases/latest"><img alt="Download for macOS" src="https://img.shields.io/github/v/release/Tchanwangsa/vox?label=Download%20for%20macOS&style=for-the-badge&logo=apple&logoColor=white&labelColor=14161d&color=e05341"></a>&nbsp;
  <a href="#install"><img alt="Install guide" src="https://img.shields.io/badge/Install_Guide-14161d?style=for-the-badge"></a>
</p>

<p>
  <a href="https://www.rust-lang.org/"><img alt="Rust" src="https://img.shields.io/badge/rust-000000?style=for-the-badge&logo=rust&logoColor=white"></a>&nbsp;
  <a href="https://www.gpui.rs/"><img alt="GPUI" src="https://img.shields.io/badge/GPUI-084CCF?style=for-the-badge"></a>&nbsp;
  <img alt="macOS 13+" src="https://img.shields.io/badge/macOS_13+-000000?style=for-the-badge&logo=apple&logoColor=white">&nbsp;
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-000000?style=for-the-badge"></a>
</p>

</div>

## About

A macOS dictation app in Rust and [GPUI](https://www.gpui.rs), with local and Groq transcription. A floating pill shows live audio levels while you speak, transcribes the clip, cleans up fillers and punctuation, and pastes the result into the active app.

macOS only for now — Windows support may come later.

## What it does

- **Push-to-talk or hands-free**: hold `⌃⌥Space` to talk and release to finish, or tap it to dictate hands-free and press again when done. `Esc` cancels. Pick a preset or record your own shortcut in **Settings**.
- **Local or cloud transcription**: run offline on your Mac with Whisper or Cohere Transcribe, or use Groq for fast cloud transcription. Turn on automatic fallback to let the other engine cover if one fails.
- **Transcript cleanup**: removes fillers, false starts, and self-corrections and fixes punctuation. If cleanup fails, the raw transcript is typed as heard rather than lost.
- **History, stats, and recordings**: browse recent dictations on **Home**, track speaking time and words by day and app on **Stats**, and optionally save 16 kHz audio clips with a JSONL manifest under `recordings/`.

## Engines

| Avenue | Transcribe | Clean up |
| --- | --- | --- |
| **On this Mac** | OpenAI Whisper (`whisper.cpp`) or Cohere Transcribe (MLX, Apple silicon & macOS 14+) | S1-mini by Superwhisper (`llama-server`) |
| **Groq** | `whisper-large-v3-turbo` | `qwen/qwen3.8-27b` |

Open **Engine** to switch avenues, download and manage local Whisper, Cohere, and S1-mini models (with RAM-based recommendations), or paste a [Groq Console](https://console.groq.com/keys) API key. Keys are verified and stored in the macOS login keychain (`GROQ_API_KEY` overrides it when set).

## Install

### Download (macOS Apple Silicon)

Download the latest `.dmg` or `.zip` from [GitHub Releases](https://github.com/Tchanwangsa/vox/releases/latest), drag **Vox** into **Applications**, and open it.

> [!NOTE]
> Prebuilt releases are ad-hoc signed (not notarized with an Apple Developer ID yet). If macOS blocks the app on first launch, remove the quarantine attribute once and reopen it:
>
> ```sh
> xattr -cr /Applications/Vox.app
> ```

On first use macOS prompts for Microphone access; allow **System Settings → Privacy & Security → Accessibility** to paste into other apps (until then, dictations are copied to your clipboard).

For optional local models, install `whisper-cpp` and `llama.cpp` via [Homebrew](https://brew.sh/) (`brew install whisper-cpp llama.cpp`), or [uv](https://docs.astral.sh/uv/getting-started/installation/) for local Cohere Transcribe.

### Build from source

#### Prerequisites

- **macOS 13+** (Apple silicon and macOS 14+ if using local Cohere Transcribe)
- **[Rust](https://www.rust-lang.org/) 1.85+** (edition 2024; GPUI builds with `runtime_shaders`, so Xcode's Metal toolchain isn't needed)
- **For local Whisper / S1-mini** *(optional)*: `whisper-cpp` and `llama.cpp` via [Homebrew](https://brew.sh/) (`brew install whisper-cpp llama.cpp`)
- **For local Cohere Transcribe** *(optional)*: [uv](https://docs.astral.sh/uv/getting-started/installation/) — Vox installs the MLX runtime and weights when you pick a variant in **Engine**
- **For Groq** *(optional)*: an API key from [Groq Console](https://console.groq.com/keys)

#### Build and launch

```sh
brew install whisper-cpp llama.cpp   # optional: helpers for local Whisper and S1-mini
scripts/bundle.sh --open             # build target/Vox.app and launch it
```

Run Vox as the app bundle rather than `cargo run` so macOS grants Microphone and Accessibility permissions to Vox instead of your terminal. On first use macOS prompts for Microphone access; allow **System Settings → Privacy & Security → Accessibility** to paste into other apps (until then, dictations are copied to your clipboard).

### Developing

```sh
cargo test
cargo run                            # signs the debug binary so Keychain trusts rebuilds
VOX_DATA_DIR=/tmp/vox cargo run      # isolated data dir (also skips Keychain)
vox --hear clip.wav whisper          # run one 16 kHz WAV through the pipeline without UI
```

App data, models, history, stats, and recordings live in `~/Library/Application Support/com.tchan.vox/`.

| Path | Contents |
| --- | --- |
| `src/state.rs`, `src/audio.rs` | Dictation state machine and 16 kHz microphone capture |
| `src/engines/` | Whisper, Cohere, S1-mini, and Groq pipelines, model downloads, and fallback |
| `src/hotkey.rs`, `src/paste.rs`, `src/keys.rs` | Global shortcuts, pasting into the front app, and Keychain |
| `src/ui/` | Main window (Home, Stats, Engine, Settings) and floating pill |
| `scripts/` | App bundle, code signing, release packaging, and icon generation |

## License & third-party notices

Vox's source code is released under the [MIT License](LICENSE) — feel free to use, copy, modify, and redistribute it.

Model weights are **not** bundled in this repository or in prebuilt releases; Vox downloads them on demand, and each model, runtime, and brand asset remains governed by its own license and terms:

- **OpenAI Whisper** (`whisper.cpp` GGML weights from [`ggerganov/whisper.cpp`](https://huggingface.co/ggerganov/whisper.cpp)) — MIT License
- **Silero VAD** ([`ggml-org/whisper-vad`](https://huggingface.co/ggml-org/whisper-vad)) — MIT License
- **Cohere Transcribe** ([`CohereLabs/cohere-transcribe-03-2026`](https://huggingface.co/CohereLabs/cohere-transcribe-03-2026), MLX weights from [`lyzgeorge/cohere-transcribe-03-2026-mlx-4bit`](https://huggingface.co/lyzgeorge/cohere-transcribe-03-2026-mlx-4bit), [`aufklarer/Cohere-Transcribe-2B-MLX-8bit`](https://huggingface.co/aufklarer/Cohere-Transcribe-2B-MLX-8bit), and [`aufklarer/Cohere-Transcribe-2B-MLX-FP16`](https://huggingface.co/aufklarer/Cohere-Transcribe-2B-MLX-FP16)) — Apache 2.0 License
- **"S1-mini" by "Superwhisper"** ([`superwhisper/s1-mini-GGUF`](https://huggingface.co/superwhisper/s1-mini-GGUF)) — Apache 2.0 License with the Superwhisper naming clause (Vox downloads its `LICENSE` and `NOTICE` files alongside the weights)
- **Groq API** (`whisper-large-v3-turbo`, `qwen/qwen3.8-27b`) — subject to [Groq's Terms of Use](https://groq.com/terms-of-use/) and each model's license (MIT / Apache 2.0)
- **Frameworks & runtimes** — [GPUI](https://www.gpui.rs/) (Apache 2.0), [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (MIT), [llama.cpp](https://github.com/ggml-org/llama.cpp) (MIT), [MLX](https://github.com/ml-explore/mlx) & [mlx-audio](https://github.com/Blaizzy/mlx-audio) (MIT)
- **Icons & brand marks** — see [`assets/icons/README.md`](assets/icons/README.md) (Feather MIT, Lucide ISC, Lobe Icons MIT; third-party logos remain the property of their respective owners)
