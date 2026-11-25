"""Download pinned Cohere weights, verify them, or transcribe a 16 kHz WAV from stdin."""
import io
import json
import os
from pathlib import Path
import sys
import wave

action, weights, *args = sys.argv[1:]
weights = Path(weights)

if action == "download":
    from huggingface_hub import hf_hub_download, snapshot_download
    from tqdm.auto import tqdm

    class WeightsProgress(tqdm):
        def display(self, msg=None, pos=None):
            # Xet reports separate compressed transfer and reconstructed file bytes.
            if self.total and not self.desc.endswith(": downloading bytes"):
                percent = min(99, int(100 * self.n / self.total))
                if percent > 0 and percent != getattr(self, "reported_percent", None):
                    self.reported_percent = percent
                    print(percent, flush=True)

    repo, revision = args

    snapshot_download(
        repo,
        revision=revision,
        allow_patterns=[
            "config.json",
            "special_tokens_map.json",
            "tokenizer.model",
            "tokenizer_config.json",
            "LICENSE",
            "ATTRIBUTION.md",
        ],
        local_dir=weights,
        token=False,
    )
    hf_hub_download(
        repo,
        "model.safetensors",
        revision=revision,
        local_dir=weights,
        token=False,
        tqdm_class=WeightsProgress,
    )
else:
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"

    from mlx_audio.stt import load

    model = load(weights, strict=True)
    if action != "hear":
        sys.exit(0)

    print("VOX_MODEL_READY", file=sys.stderr, flush=True)
    audio_bytes = sys.stdin.buffer.read()

    import numpy as np

    with wave.open(io.BytesIO(audio_bytes)) as wav:
        if (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) != (1, 2, 16000):
            raise ValueError("Expected mono 16 kHz PCM WAV")
        audio = np.frombuffer(wav.readframes(wav.getnframes()), dtype="<i2")
    result = model.generate(audio.astype(np.float32) / 32768, language=args[0])
    print(json.dumps(result.text))
