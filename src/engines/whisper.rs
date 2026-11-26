//! Hearing on this Mac: whisper.cpp through its `whisper-cli` helper, on the
//! GPU through Metal. Silero VAD, when its file is there, skips the silence a
//! clip opens with, where Whisper otherwise invents "Thank you."

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use super::speech::Process;

static CLIP: AtomicU64 = AtomicU64::new(0);

pub(super) struct Request {
    process: Process,
    output: TranscriptFile,
}

struct TranscriptFile(PathBuf);

impl Drop for TranscriptFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

pub(super) fn start(
    helper: &Path,
    model: &Path,
    vad: Option<&Path>,
    language: &str,
) -> Result<Request, String> {
    let stem = std::env::temp_dir().join(format!(
        "vox-{}-{}",
        std::process::id(),
        CLIP.fetch_add(1, Ordering::Relaxed)
    ));
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    let mut command = Command::new(helper);
    // whisper-cli initializes its context before reading the WAV from stdin.
    command
        .arg("-m")
        .arg(model)
        .args([
            "-f",
            "-",
            "-l",
            language,
            "-t",
            &threads.to_string(),
            "-oj",
            "-sns",
            "-of",
        ])
        .arg(&stem)
        .stdout(Stdio::null());
    if let Some(vad) = vad {
        command.arg("--vad").arg("-vm").arg(vad);
    }
    let process = Process::start(&mut command, |line| {
        line.starts_with(b"whisper_init_state: compute buffer (decode)")
            || line.starts_with(b"main: processing ")
    })
    .map_err(|e| transcription_error(e.to_string()))?;
    let output = TranscriptFile(stem.with_extension("json"));
    Ok(Request { process, output })
}

impl Request {
    pub(super) fn hear(
        self,
        wav: &[u8],
        stage: &Arc<AtomicU8>,
        cancel: &AtomicBool,
    ) -> Result<String, String> {
        let output = self.process.transcribe(wav, stage, cancel).map_err(|e| {
            if cancel.load(Ordering::Relaxed) {
                "Cancelled".into()
            } else {
                transcription_error(e.to_string())
            }
        })?;
        // whisper-cli can exit successfully without a JSON file on bad input.
        if output.status.success()
            && let Ok(reply) = std::fs::read(&self.output.0)
        {
            return parse(&String::from_utf8_lossy(&reply)).map_err(transcription_error);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines = || {
            stderr
                .lines()
                .rev()
                .map(str::trim)
                .filter(|l| !l.is_empty())
        };
        let detail = lines()
            .find(|l| l.to_lowercase().contains("error"))
            .or_else(|| lines().next())
            .unwrap_or("whisper-cli wrote no transcript");
        Err(transcription_error(detail.into()))
    }
}

fn transcription_error(detail: String) -> String {
    eprintln!("[vox] Whisper: {detail}");
    "Whisper couldn't transcribe. Try again.".into()
}

/// `-oj`'s `transcription[].text`, joined. whisper-cli escapes only `"` and
/// `\`, so other control characters are blanked first.
fn parse(body: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Item {
        #[serde(default)]
        text: String,
    }
    #[derive(serde::Deserialize)]
    struct Reply {
        #[serde(default)]
        transcription: Vec<Item>,
    }
    let clean = body.replace(|c: char| c.is_control() && c != '\n', " ");
    let reply: Reply = serde_json::from_str(&clean)
        .map_err(|e| format!("whisper-cli's reply was not a transcript: {e}"))?;
    Ok(reply
        .transcription
        .iter()
        .map(|item| item.text.trim())
        .filter(|text| !non_speech(text))
        .collect::<Vec<_>>()
        .join(" "))
}

/// Empty, or only markers such as `[BLANK_AUDIO]`, `(silence)`, `*laughs*`
/// or `♪`, which nobody said.
fn non_speech(text: &str) -> bool {
    let mut depth = 0i32;
    let mut starred = false;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            '*' => starred = !starred,
            '♪' | '♫' => {}
            c if c.is_whitespace() || c.is_ascii_punctuation() => {}
            _ if depth > 0 || starred => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_not_speech() {
        let body = r#"{"transcription":[{"text":" [BLANK_AUDIO]"},{"text":" Hello there."},{"text":" (silence)"},{"text":" How are you?"}]}"#;
        assert_eq!(parse(body).unwrap(), "Hello there. How are you?");
    }
}
