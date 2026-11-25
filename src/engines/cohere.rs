use std::fs;
use std::io::{BufRead as _, BufReader, Read as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use super::download::{CANCELLED, Progress, check_cancelled, lock};
use super::speech::Process;
use crate::config::{LocalModel, data_dir};

/// One variant's weights: where they come from and where they live. The 4-bit
/// keeps the folder and stamp of the first release, so an install made then
/// is still ready.
struct Weights {
    model: LocalModel,
    /// The name recordings give the model; the ready stamp holds it.
    id: &'static str,
    /// Under `models/cohere/`.
    dir: &'static str,
    stamp: &'static str,
    repo: &'static str,
    /// Pinned, so what is downloaded is what was tested.
    revision: &'static str,
}

/// All three load with `mlx_audio.stt.load(strict=True)` at [`RUNTIME`]'s version.
const VARIANTS: [Weights; 3] = [
    Weights {
        model: LocalModel::Cohere4bit,
        id: "cohere-transcribe-03-2026-mlx-4bit",
        dir: "weights",
        stamp: "ready",
        repo: "lyzgeorge/cohere-transcribe-03-2026-mlx-4bit",
        revision: "f2f6d89b1a300c51f48ae1ecb8523dbb58636cb0",
    },
    Weights {
        model: LocalModel::Cohere8bit,
        id: "cohere-transcribe-03-2026-mlx-8bit",
        dir: "weights-8bit",
        stamp: "ready-8bit",
        repo: "aufklarer/Cohere-Transcribe-2B-MLX-8bit",
        revision: "924ad09b5bc13619c16248d2c6b625b430e9e192",
    },
    Weights {
        model: LocalModel::CohereFp16,
        id: "cohere-transcribe-03-2026-mlx-fp16",
        dir: "weights-fp16",
        stamp: "ready-fp16",
        repo: "aufklarer/Cohere-Transcribe-2B-MLX-FP16",
        revision: "081d436a09f944de89e63acc862362db97d792ff",
    },
];

fn weights(model: LocalModel) -> &'static Weights {
    VARIANTS.iter().find(|w| w.model == model).expect("a Cohere variant")
}

/// The model's name in a recording.
pub fn id(model: LocalModel) -> &'static str {
    weights(model).id
}

pub const REQUIREMENTS: &str = "Cohere needs Apple silicon and macOS 14+";
const RUNTIME: &str = "mlx-audio[stt]==0.6.8";
const SCRIPT: &str = include_str!("cohere.py");
const LANGUAGES: [&str; 14] = [
    "en", "de", "fr", "it", "es", "pt", "el", "nl", "pl", "vi", "zh", "ar", "ja", "ko",
];

pub fn supported() -> bool {
    static SUPPORTED: LazyLock<bool> = LazyLock::new(|| {
        cfg!(target_arch = "aarch64")
            && Command::new("/usr/bin/sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| {
                    let version = String::from_utf8(output.stdout).ok()?;
                    version.trim().split('.').next()?.parse::<u32>().ok()
                })
                .is_some_and(|major| major >= 14)
    });
    *SUPPORTED
}

pub fn installer() -> Option<PathBuf> {
    let local_bin = dirs::home_dir().map(|home| home.join(".local/bin/uv"));
    local_bin
        .into_iter()
        .chain(super::helper("uv"))
        .chain(std::env::var_os("PATH").into_iter().flat_map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join("uv"))
                .collect::<Vec<_>>()
        }))
        .find(|path| path.is_file())
}

fn root() -> PathBuf {
    data_dir().join("models/cohere")
}

fn python(root: &Path) -> PathBuf {
    root.join("runtime/bin/python")
}

/// Whether `model`'s weights are installed and were verified, and its runtime is there.
pub fn ready(model: LocalModel) -> bool {
    let root = root();
    let w = weights(model);
    let dir = root.join(w.dir);
    supported()
        && fs::read_to_string(root.join(w.stamp)).is_ok_and(|stamp| stamp == w.id)
        && python(&root).is_file()
        && dir.join("model.safetensors").is_file()
        && dir.join("config.json").is_file()
        && dir.join("tokenizer.model").is_file()
        && dir.join("tokenizer_config.json").is_file()
        && dir.join("special_tokens_map.json").is_file()
}

/// Remove one variant's weights and ready stamp, leaving the shared Python runtime.
pub(super) fn remove_in(root: &Path, model: LocalModel) -> Result<(), String> {
    let w = weights(model);
    let stamp = root.join(w.stamp);
    let dir = root.join(w.dir);
    if stamp.exists() || stamp.is_symlink() {
        fs::remove_file(&stamp).map_err(|e| format!("couldn't remove Cohere stamp: {e}"))?;
    }
    if dir.is_symlink() {
        fs::remove_file(&dir).map_err(|e| format!("couldn't remove Cohere weights link: {e}"))?;
    } else if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("couldn't remove Cohere weights: {e}"))?;
    }
    Ok(())
}

pub(super) fn install(
    model: LocalModel,
    cancel: &AtomicBool,
    mut report: impl FnMut(Progress) + Send,
) -> Result<(), String> {
    if !supported() {
        return Err(REQUIREMENTS.into());
    }
    let root = root();
    let _lock = lock(&root)?;
    if ready(model) {
        return Ok(());
    }
    let uv = installer().ok_or("Install uv, then recheck")?;
    let runtime = root.join("runtime");
    if !python(&root).is_file() {
        setup_command(
            Command::new(&uv)
                .args(["venv", "--python", "3.12"])
                .arg(runtime),
            cancel,
        )?;
    }
    setup_command(
        Command::new(&uv)
            .args(["pip", "install", "--python"])
            .arg(python(&root))
            .args([RUNTIME, "mlx==0.32.5", "huggingface-hub==2.4.0"]),
        cancel,
    )?;
    let w = weights(model);
    run_setup(
        Command::new(python(&root))
            .args(["-I", "-c", SCRIPT, "download"])
            .arg(root.join(w.dir))
            .args([w.repo, w.revision])
            .env("HF_HUB_DISABLE_PROGRESS_BARS", "1"),
        cancel,
        |line| {
            if let Ok(percent) = line.parse::<u8>() {
                report(Progress::Downloading(percent.min(99)));
            }
        },
    )?;
    report(Progress::Verifying);
    setup_command(
        Command::new(python(&root))
            .args(["-I", "-c", SCRIPT, "verify"])
            .arg(root.join(w.dir)),
        cancel,
    )?;
    check_cancelled(cancel)?;
    fs::write(root.join(w.stamp), w.id).map_err(setup_io)
}

fn run_setup(
    command: &mut Command,
    cancel: &AtomicBool,
    mut progress: impl FnMut(String) + Send,
) -> Result<(), String> {
    check_cancelled(cancel)?;
    let mut child = command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(setup_io)?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");

    std::thread::scope(|scope| {
        scope.spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                progress(line);
            }
        });
        let errors = scope.spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).ok();
            bytes
        });
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                // uv can launch subprocesses that inherit its pipes. Stop the
                // whole group before waiting for the output readers to finish.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                child.wait().map_err(setup_io)?;
                return Err(CANCELLED.into());
            }
            if let Some(status) = child.try_wait().map_err(setup_io)? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        check_cancelled(cancel)?;
        if !status.success() {
            let errors = errors.join().expect("setup stderr reader");
            return Err(setup_io(String::from_utf8_lossy(&errors)));
        }
        Ok(())
    })
}

fn setup_io(error: impl std::fmt::Display) -> String {
    eprintln!("[vox] Cohere setup: {error}");
    "Cohere setup failed. Try again.".into()
}

fn setup_command(command: &mut Command, cancel: &AtomicBool) -> Result<(), String> {
    run_setup(command, cancel, |_| {})
}

pub(super) fn start(model: LocalModel, language: &str) -> Result<Process, String> {
    if !ready(model) {
        return Err("Set up Cohere in Engine".into());
    }
    // Cohere's decoder needs an explicit language.
    let language = if language == "auto" { "en" } else { language };
    if !LANGUAGES.contains(&language) {
        return Err("Cohere doesn't support this language".into());
    }
    let root = root();
    let mut command = Command::new(python(&root));
    command.args(["-I", "-c", SCRIPT, "hear"])
        .arg(root.join(weights(model).dir))
        .arg(language)
        .stdout(Stdio::piped());
    Process::start(&mut command, |line| line == b"VOX_MODEL_READY\n").map_err(runtime_error)
}

fn runtime_error(error: std::io::Error) -> String {
    eprintln!("[vox] Cohere runtime: {error}");
    "Cohere couldn't transcribe. Try again.".into()
}

pub(super) fn hear(process: Process, wav: &[u8], stage: &Arc<AtomicU8>) -> Result<String, String> {
    let output = process.transcribe(wav, stage).map_err(runtime_error)?;
    if output.status.success() {
        let last_line = String::from_utf8_lossy(&output.stdout);
        if let Some(text) = last_line
            .lines()
            .last()
            .and_then(|line| serde_json::from_str(line).ok())
        {
            return Ok(text);
        }
    }
    eprintln!(
        "[vox] Cohere runtime: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Err("Cohere couldn't transcribe. Try again.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn cancelling_setup_stops_children_that_hold_its_output_pipes() {
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        let result = run_setup(
            Command::new("/bin/sh").args(["-c", "/bin/sleep 4 & echo started; wait"]),
            &cancel,
            |line| {
                assert_eq!(line, "started");
                cancel.store(true, Ordering::Relaxed);
            },
        );

        assert_eq!(result.as_ref().err().map(String::as_str), Some(CANCELLED));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
