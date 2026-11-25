//! A clip in, the text to type out. Each avenue hears and tidies with its own
//! pair of models; with the fallback on, the other avenue answers a step the
//! chosen one cannot.

pub mod cohere;
pub mod download;
pub mod groq;
pub mod groq_usage;
pub mod models;
pub mod s1;
mod speech;
pub mod whisper;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread::{self, JoinHandle};

use crate::config::{data_dir, Avenue, Family, LocalModel, Settings};
use groq_usage::{GroqUsage, Reading};

/// Which step a running dictation is on, for the pill.
pub const HEARING: u8 = 0;
pub const TIDYING: u8 = 1;
pub const LOADING_SPEECH: u8 = 2;
pub const LOADING_TIDY: u8 = 3;

enum Speech {
    Whisper(whisper::Request),
    Cohere(speech::Process),
}

impl Speech {
    fn start(model: LocalModel, local: &Local, language: &str) -> Result<Self, String> {
        match model.family() {
            Family::Cohere => cohere::start(model, language).map(Self::Cohere),
            Family::Whisper => {
                let helper = local.whisper_cli.as_deref().ok_or("Whisper helper missing. Check Engine.")?;
                whisper::start(helper, &whisper_path(model), local.vad.as_deref(), language)
                    .map(Self::Whisper)
            }
        }
    }

    fn hear(self, wav: &[u8], stage: &Arc<AtomicU8>) -> Result<String, String> {
        match self {
            Self::Whisper(request) => request.hear(wav, stage),
            Self::Cohere(process) => cohere::hear(process, wav, stage),
        }
    }
}

pub struct Preload {
    model: LocalModel,
    language: String,
    worker: JoinHandle<Result<Speech, String>>,
}

impl Preload {
    pub fn start(model: LocalModel, local: Local, language: String) -> Self {
        let requested_language = language.clone();
        let worker = thread::spawn(move || Speech::start(model, &local, &language));
        Self { model, language: requested_language, worker }
    }
}

const VAD_FILE: &str = "ggml-silero-v6.2.0.bin";
const S1_FILE: &str = "s1-mini-q4_k_m.gguf";

/// What the local avenue has to work with on this Mac.
#[derive(Debug, Clone, Default)]
pub struct Local {
    pub whisper_cli: Option<PathBuf>,
    pub vad: Option<PathBuf>,
    pub llama_server: Option<PathBuf>,
    pub s1_model: Option<PathBuf>,
    /// The speech variants on disk: whatever ggml files are present and the
    /// Cohere variants installed and verified.
    pub downloaded: Vec<LocalModel>,
}

impl Local {
    /// Helpers beside the executable, in Vox's `bin/`, or from Homebrew;
    /// models in Vox's `models/`.
    pub fn discover() -> Self {
        let models = data_dir().join("models");
        Self {
            whisper_cli: helper("whisper-cli"),
            vad: Some(models.join("whisper").join(VAD_FILE)).filter(|p| p.is_file()),
            llama_server: helper("llama-server"),
            s1_model: Some(models.join("s1-mini").join(S1_FILE)).filter(|p| p.is_file()),
            downloaded: models::CATALOGUE
                .iter()
                .map(|spec| spec.model)
                .filter(|&model| match model.family() {
                    Family::Whisper => whisper_path(model).is_file(),
                    Family::Cohere => cohere::ready(model),
                })
                .collect(),
        }
    }

    pub fn can_hear(&self, model: LocalModel) -> bool {
        self.downloaded.contains(&model)
            && (model.family() == Family::Cohere || self.whisper_cli.is_some())
    }

    /// The variant that answers a dictation. The one chosen, once it is ready;
    /// until then the last one that was ready, then the default if it is there,
    /// else the best on disk.
    pub fn active(&self, chosen: LocalModel, last_ready: Option<LocalModel>) -> Option<LocalModel> {
        [chosen]
            .into_iter()
            .chain(last_ready)
            .chain([LocalModel::default()])
            .chain(models::CATALOGUE.iter().rev().map(|spec| spec.model))
            .find(|&model| self.can_hear(model))
    }

    /// Why nothing can hear, in terms of what was chosen.
    fn unavailable(&self, chosen: LocalModel) -> &'static str {
        match chosen.family() {
            Family::Cohere => "Set up Cohere in Engine",
            Family::Whisper if self.whisper_cli.is_none() => "Whisper helper missing. Check Engine.",
            Family::Whisper => "Whisper model missing. Check Engine.",
        }
    }

    pub fn can_tidy(&self) -> bool {
        self.s1().is_some()
    }

    pub fn s1(&self) -> Option<s1::Launch> {
        Some(s1::Launch {
            helper: self.llama_server.clone()?,
            model: self.s1_model.clone()?,
        })
    }
}

/// Where a Whisper variant's ggml file is kept.
pub fn whisper_path(model: LocalModel) -> PathBuf {
    data_dir().join("models").join("whisper").join(download::whisper_asset(model).file)
}

/// Remove one downloaded speech model from this Mac. Symlinks are unlinked
/// without touching their target.
pub fn remove_model(model: LocalModel) -> Result<(), String> {
    remove_model_in(&data_dir(), model)
}

fn remove_model_in(data_dir: &Path, model: LocalModel) -> Result<(), String> {
    match model.family() {
        Family::Whisper => {
            let path = data_dir.join("models").join("whisper").join(download::whisper_asset(model).file);
            if path.exists() || path.is_symlink() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("couldn't remove Whisper model: {e}"))?;
            }
            Ok(())
        }
        Family::Cohere => cohere::remove_in(&data_dir.join("models").join("cohere"), model),
    }
}

fn helper(name: &str) -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)));
    beside
        .into_iter()
        .chain([
            data_dir().join("bin").join(name),
            Path::new("/opt/homebrew/bin").join(name),
            Path::new("/usr/local/bin").join(name),
        ])
        .find(|path| path.is_file())
}

/// A dictation, finished.
#[derive(Debug)]
pub struct Outcome {
    /// What to type.
    pub text: String,
    /// What was heard, before tidying.
    pub heard: String,
    /// Who heard it.
    pub avenue: Avenue,
    /// Which speech model actually supplied the transcript, including fallback.
    pub heard_model: String,
    /// Who tidied it; `None` when it is typed as heard.
    pub tidied_by: Option<Avenue>,
    /// Something the speaker should know although the text is good, such as
    /// tidying having failed.
    pub note: Option<String>,
}

pub struct Dictation {
    pub outcome: Result<Outcome, String>,
    /// A cloud step was refused because Groq no longer accepts the key.
    pub groq_key_rejected: bool,
    /// Rate-limit readings captured from Groq steps (including `429`s, even
    /// if the dictation fell back to this Mac or failed).
    pub groq_usage: GroqUsage,
}

struct GroqSession {
    key: Option<Result<String, groq::Error>>,
    rejected: bool,
    usage: GroqUsage,
}

impl GroqSession {
    fn request<T>(&mut self, send: impl FnOnce(&str) -> Result<T, groq::Error>) -> Result<T, String> {
        let key = self.key.get_or_insert_with(|| {
            crate::keys::groq().ok_or_else(|| groq::Error::Other("Add a Groq key in Engine".into()))
        });
        let response = match key {
            Ok(key) => send(key),
            Err(error) => Err(error.clone()),
        };
        match response {
            Ok(value) => Ok(value),
            Err(groq::Error::RejectedKey) => {
                self.key = Some(Err(groq::Error::RejectedKey));
                self.rejected = true;
                Err(groq::REJECTED_KEY.to_string())
            }
            Err(groq::Error::RateLimited { message, reading }) => {
                self.usage.record_reading(reading);
                Err(message)
            }
            Err(error) => Err(error.message().to_string()),
        }
    }
}

fn hear(
    avenue: Avenue,
    settings: &Settings,
    local: &Local,
    wav: &[u8],
    cloud: &mut GroqSession,
    stage: &Arc<AtomicU8>,
    preload: &mut Option<Preload>,
) -> Result<(String, Option<Reading>), String> {
    match avenue {
        Avenue::Cloud => {
            let language = Some(settings.language.as_str()).filter(|l| *l != "auto");
            cloud.request(|key| groq::hear(key, wav, language))
        }
        Avenue::Local => {
            let model = local
                .active(settings.local_model, settings.last_ready_model)
                .ok_or_else(|| local.unavailable(settings.local_model))?;
            let speech = match preload.take() {
                Some(preload) if preload.model == model && preload.language == settings.language => {
                    preload.worker.join().map_err(|_| "Couldn't start speech model")??
                }
                _ => Speech::start(model, local, &settings.language)?,
            };
            speech.hear(wav, stage).map(|text| (text, None))
        }
    }
}

fn tidy(
    avenue: Avenue,
    settings: &Settings,
    local: &Local,
    heard: &str,
    cloud: &mut GroqSession,
    stage: &AtomicU8,
) -> Result<(String, Option<Reading>), String> {
    match avenue {
        Avenue::Cloud => cloud.request(|key| groq::tidy(key, heard)),
        Avenue::Local => {
            let launch = local.s1().ok_or("S1-mini is not on this Mac")?;
            s1::tidy(&launch, heard, &settings.language, stage).map(|text| (text, None))
        }
    }
}

/// The avenues to try for one step: the chosen one, then the other when the
/// fallback is on.
fn order(first: Avenue, fallback: bool) -> Vec<Avenue> {
    if fallback {
        vec![first, first.other()]
    } else {
        vec![first]
    }
}

/// Hear the clip, then tidy what was heard. A dictation is never lost to a
/// failed tidy: the text is typed as heard, with a note saying so.
pub fn run(
    settings: &Settings,
    local: &Local,
    wav: &[u8],
    stage: &Arc<AtomicU8>,
    groq_key_rejected: bool,
    preload: Option<Preload>,
) -> Dictation {
    let key = groq_key_rejected.then_some(Err(groq::Error::RejectedKey));
    let mut cloud = GroqSession { key, rejected: false, usage: GroqUsage::default() };
    let outcome = run_steps(settings, local, wav, stage, &mut cloud, preload);
    Dictation { outcome, groq_key_rejected: cloud.rejected, groq_usage: cloud.usage }
}

fn run_steps(
    settings: &Settings,
    local: &Local,
    wav: &[u8],
    stage: &Arc<AtomicU8>,
    cloud: &mut GroqSession,
    mut preload: Option<Preload>,
) -> Result<Outcome, String> {
    stage.store(HEARING, Ordering::Relaxed);
    let mut refusals = Vec::new();
    let mut answer = None;
    for avenue in order(settings.avenue, settings.fallback) {
        match hear(avenue, settings, local, wav, cloud, stage, &mut preload) {
            Ok((text, reading)) => {
                if let Some(reading) = reading {
                    cloud.usage.record_reading(reading);
                }
                answer = Some((avenue, text));
                break;
            }
            Err(e) => refusals.push(e),
        }
    }
    drop(preload);
    // The chosen avenue's reason is the one worth reading.
    let (avenue, heard) = answer.ok_or_else(|| refusals.remove(0))?;
    let heard_model = match (avenue, local.active(settings.local_model, settings.last_ready_model)) {
        (Avenue::Cloud, _) => groq::HEAR_MODEL.into(),
        (Avenue::Local, Some(model)) if model.family() == Family::Cohere => cohere::id(model).into(),
        (Avenue::Local, Some(model)) => whisper_path(model)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into()),
        (Avenue::Local, None) => "unknown".into(),
    };
    let heard = heard.trim().to_string();
    if heard.is_empty() {
        return Err("No speech heard. Try speaking longer.".into());
    }
    if !settings.tidy {
        return Ok(Outcome {
            text: heard.clone(),
            heard,
            avenue,
            heard_model,
            tidied_by: None,
            note: None,
        });
    }

    let mut failure = None;
    for tidier in order(avenue, settings.fallback) {
        stage.store(TIDYING, Ordering::Relaxed);
        match tidy(tidier, settings, local, &heard, cloud, stage) {
            Ok((text, reading)) if text.trim().is_empty() => {
                if let Some(reading) = reading {
                    cloud.usage.record_reading(reading);
                }
                return Err("Only filler heard".into());
            }
            Ok((text, reading)) => {
                if let Some(reading) = reading {
                    cloud.usage.record_reading(reading);
                }
                return Ok(Outcome {
                    text,
                    heard,
                    avenue,
                    heard_model,
                    tidied_by: Some(tidier),
                    note: None,
                });
            }
            Err(e) => {
                failure.get_or_insert(e);
            }
        }
    }
    Ok(Outcome {
        text: heard.clone(),
        heard,
        avenue,
        heard_model,
        tidied_by: None,
        note: failure.map(|e| {
            eprintln!("[vox] Transcript cleanup: {e}");
            "as heard".into()
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groq_is_skipped_for_later_steps_after_a_key_rejection() {
        let mut cloud = GroqSession {
            key: Some(Ok("gsk_test".into())),
            rejected: false,
            usage: GroqUsage::default(),
        };
        let result = cloud.request::<()>(|_| Err(groq::Error::Other("temporary".into())));
        assert_eq!(result, Err("temporary".into()));
        assert!(!cloud.rejected);

        let result = cloud.request::<()>(|_| Err(groq::Error::RejectedKey));
        assert_eq!(result, Err(groq::REJECTED_KEY.into()));
        assert!(cloud.rejected);
        assert!(cloud.request::<()>(|_| panic!("should not call Groq again")).is_err());
    }

    #[test]
    fn rate_limit_errors_record_their_reading_on_the_session() {
        let mut cloud = GroqSession {
            key: Some(Ok("gsk_test".into())),
            rejected: false,
            usage: GroqUsage::default(),
        };
        let reading = Reading::from_rate_limit(
            groq_usage::Role::Cleanup,
            groq::TIDY_MODEL,
            &[("retry-after".into(), "45".into())],
            "Rate limit reached on tokens per day (TPD): Limit 500000, Used 500000.",
            10_000,
        );
        let message = reading.rate_limit_message();
        let result = cloud.request::<()>(|_| {
            Err(groq::Error::RateLimited { message: message.clone(), reading: reading.clone() })
        });
        assert_eq!(result, Err(message));
        assert!(!cloud.rejected);
        let row = cloud.usage.row_for(groq_usage::Role::Cleanup, 10_000).unwrap();
        assert!(row.rate_limited);
        assert_eq!(row.metric, Some(groq_usage::Metric::TokensPerDay));
    }

    fn local(downloaded: &[LocalModel]) -> Local {
        Local {
            whisper_cli: Some(PathBuf::from("whisper-cli")),
            downloaded: downloaded.to_vec(),
            ..Local::default()
        }
    }

    #[test]
    fn the_chosen_variant_answers_once_it_is_ready() {
        let local = local(&[LocalModel::WhisperLargeV3TurboQ5, LocalModel::Cohere4bit]);
        assert_eq!(local.active(LocalModel::Cohere4bit, None), Some(LocalModel::Cohere4bit));
    }

    #[test]
    fn the_last_ready_variant_answers_while_the_chosen_one_downloads() {
        let local = local(&[LocalModel::WhisperMedium, LocalModel::WhisperLargeV3TurboQ5]);
        let active = local.active(LocalModel::Cohere8bit, Some(LocalModel::WhisperMedium));
        assert_eq!(active, Some(LocalModel::WhisperMedium));
    }

    #[test]
    fn with_no_last_ready_the_default_then_the_best_on_disk_answers() {
        let both = local(&[LocalModel::WhisperSmall, LocalModel::WhisperLargeV3TurboQ5]);
        assert_eq!(both.active(LocalModel::Cohere4bit, None), Some(LocalModel::WhisperLargeV3TurboQ5));
        let others = local(&[LocalModel::WhisperSmall, LocalModel::WhisperLargeV3]);
        assert_eq!(others.active(LocalModel::Cohere4bit, None), Some(LocalModel::WhisperLargeV3));
        assert_eq!(local(&[]).active(LocalModel::Cohere4bit, None), None);
    }

    #[test]
    fn a_whisper_file_without_its_helper_cannot_hear() {
        let mut local = local(&[LocalModel::WhisperLargeV3TurboQ5]);
        local.whisper_cli = None;
        assert_eq!(local.active(LocalModel::WhisperLargeV3TurboQ5, None), None);
    }

    #[test]
    fn removing_models_deletes_weights_and_unlinks_symlinks_safely() {
        let root = std::env::temp_dir().join(format!("vox-remove-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let whisper_dir = root.join("models/whisper");
        let cohere_dir = root.join("models/cohere");
        std::fs::create_dir_all(&whisper_dir).unwrap();
        std::fs::create_dir_all(cohere_dir.join("weights")).unwrap();
        std::fs::create_dir_all(cohere_dir.join("runtime/bin")).unwrap();

        // Plain Whisper file is deleted.
        let tiny = whisper_dir.join(download::whisper_asset(LocalModel::WhisperTiny).file);
        std::fs::write(&tiny, b"weights").unwrap();
        remove_model_in(&root, LocalModel::WhisperTiny).unwrap();
        assert!(!tiny.exists());

        // Symlinked Whisper file is unlinked without removing its target.
        let external = root.join("external-model.bin");
        std::fs::write(&external, b"external-copy").unwrap();
        let turbo = whisper_dir.join(download::whisper_asset(LocalModel::WhisperLargeV3TurboQ5).file);
        std::os::unix::fs::symlink(&external, &turbo).unwrap();
        remove_model_in(&root, LocalModel::WhisperLargeV3TurboQ5).unwrap();
        assert!(!turbo.exists() && !turbo.is_symlink());
        assert!(external.exists());

        // Cohere variant weights + stamp are deleted while the shared runtime remains.
        std::fs::write(cohere_dir.join("ready"), cohere::id(LocalModel::Cohere4bit)).unwrap();
        std::fs::write(cohere_dir.join("weights/model.safetensors"), b"cohere").unwrap();
        std::fs::write(cohere_dir.join("runtime/bin/python"), b"py").unwrap();
        remove_model_in(&root, LocalModel::Cohere4bit).unwrap();
        assert!(!cohere_dir.join("ready").exists());
        assert!(!cohere_dir.join("weights").exists());
        assert!(cohere_dir.join("runtime/bin/python").exists());

        let _ = std::fs::remove_dir_all(&root);
    }
}
