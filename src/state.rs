//! The one piece of app state: what a dictation is doing right now, and
//! everything the windows show. The shortcut drives it; both windows watch it.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{StreamExt as _, channel::mpsc};
use gpui::{App, AppContext as _, ClipboardItem, Context, Entity, Global};

use crate::audio::{self, Levels, MicPermission, Recorder};
use crate::config::{Avenue, Entry, Family, History, LocalModel, Settings, Shortcut};
use crate::engines::download::{Model, Progress, CANCELLED};
use crate::engines::groq_usage::{now_ms, DisplaySig, GroqUsage};
use crate::engines::{self, cohere, groq, Dictation, Local, Outcome};
use crate::hotkey::{Hotkeys, Press};
use crate::recordings::{self, Kept, Record};
use crate::stats::Stats;
use crate::{apps, keys, paste, ui};

/// A press shorter than this is a tap: the dictation keeps listening until
/// the shortcut is pressed again.
const TAP: Duration = Duration::from_millis(320);

/// A recording shorter than this was a slip of the hand.
const TOO_SHORT: f32 = 0.35;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PillAction {
    OpenMicSettings,
    OpenAccessibilitySettings,
    OpenGroqSettings,
}

pub enum Phase {
    Idle,
    Listening { hands_free: bool },
    Working,
    Done {
        words: usize,
        typed: bool,
        needs_accessibility: bool,
        note: Option<String>,
        groq_key_rejected: bool,
        groq: GroqUsage,
    },
    Failed {
        message: String,
        action: Option<PillAction>,
    },
}

#[derive(Clone, PartialEq)]
pub enum GroqKey {
    /// The keychain has not answered yet.
    Loading,
    Missing,
    /// Saved, shown masked.
    Saved(String),
    /// Saved, but Groq refused it during dictation.
    Rejected(String),
}

pub enum DownloadState {
    Queued,
    Working(Progress),
    Cancelling,
    Failed(String),
}

pub struct Vox {
    pub settings: Settings,
    pub history: History,
    /// Totals by day, for the Stats page.
    pub stats: Stats,
    pub local: Local,
    pub downloads: HashMap<Model, DownloadState>,
    /// The installer owns the front entry until it exits and releases its lock.
    pub download_queue: VecDeque<Model>,
    download_cancel: Option<Arc<AtomicBool>>,
    pub phase: Phase,
    pub levels: Levels,
    /// Progress for this dictation, written by the pipeline and model helpers.
    pub stage: Arc<AtomicU8>,
    pub groq: GroqKey,
    pub checking_groq_key: bool,
    /// Lets a result from an older key finish without updating the new one's status.
    groq_key_revision: u64,
    /// Latest rate-limit readings reported by Groq, persisted for the Engine tab.
    pub groq_usage: GroqUsage,
    /// Last displayed usage signature, to repaint only when counts or timers tick.
    last_usage_sig: DisplaySig,
    /// A temporary toast for an unsuccessful paste.
    pub key_notice: Option<String>,
    key_attempt: u64,
    /// Whether macOS lets Vox paste into other apps.
    pub can_type: bool,
    /// Whether macOS lets Vox read the microphone.
    pub mic_permission: MicPermission,
    /// Why the shortcut could not be registered, when it could not.
    pub shortcut_notice: Option<String>,
    /// The recordings kept so far.
    pub kept: Kept,
    pub listening_since: Instant,
    /// The app in front when this dictation began.
    target_app: Option<apps::Front>,
    recorder: Option<Recorder>,
    preload: Option<engines::Preload>,
    pressed_at: Instant,
    hotkeys: Option<Hotkeys>,
    /// Counts dictations, so a cancelled one's late result is dropped.
    run: u64,
}

/// Where the rest of the app finds the state.
pub struct Handle(pub Entity<Vox>);

impl Global for Handle {}

impl Vox {
    pub fn new(settings: Settings, hotkeys: Result<Hotkeys, String>, cx: &mut Context<Self>) -> Self {
        const HELD: &str = "Shortcut in use. Choose another.";
        let (hotkeys, shortcut_notice) = match hotkeys {
            Ok(hotkeys) if hotkeys.taken() => (Some(hotkeys), None),
            Ok(hotkeys) => (Some(hotkeys), Some(HELD.to_string())),
            Err(e) => {
                eprintln!("[vox] Shortcut registration: {e}");
                (None, Some("Couldn't register the shortcut".into()))
            }
        };
        // `VOX_DEMO` opens a development run with the pill in one state.
        let demo = std::env::var("VOX_DEMO").ok();
        let demo_groq = match demo.as_deref() {
            Some("done-groq") => Some(GroqUsage::demo(false)),
            Some("done-groq-low") => Some(GroqUsage::demo_low_cleanup()),
            Some("done-groq-low-both") => Some(GroqUsage::demo(true)),
            _ => None,
        };
        let initial_groq = if demo_groq.is_some() {
            GroqKey::Saved("gsk_…demo".into())
        } else {
            // Read the keychain off the main thread.
            cx.spawn(async move |this, cx| {
                let key = cx.background_spawn(async { keys::groq() }).await;
                this.update(cx, |this, cx| {
                    this.groq = key.map_or(GroqKey::Missing, |k| GroqKey::Saved(keys::masked(&k)));
                    cx.notify();
                })
                .ok();
            })
            .detach();
            GroqKey::Loading
        };
        let phase = match demo.as_deref() {
            Some("listening") => Phase::Listening { hands_free: false },
            Some("hands-free" | "hands-free-hover-cancel" | "hands-free-hover-finish") => {
                Phase::Listening { hands_free: true }
            }
            Some("working" | "loading-speech" | "loading-tidy") => Phase::Working,
            Some("done") => Phase::Done {
                words: 23,
                typed: true,
                needs_accessibility: false,
                note: None,
                groq_key_rejected: false,
                groq: GroqUsage::default(),
            },
            Some("done-groq" | "done-groq-low" | "done-groq-low-both") => Phase::Done {
                words: 23,
                typed: true,
                needs_accessibility: false,
                note: None,
                groq_key_rejected: false,
                groq: demo_groq.clone().unwrap_or_default(),
            },
            Some("copied") => Phase::Done {
                words: 23,
                typed: false,
                needs_accessibility: true,
                note: None,
                groq_key_rejected: false,
                groq: GroqUsage::default(),
            },
            Some("failed") => Phase::Failed {
                message: "Allow microphone access".into(),
                action: Some(PillAction::OpenMicSettings),
            },
            Some("silence") => Phase::Failed {
                message: "No audio detected — check your mic".into(),
                action: None,
            },
            _ => Phase::Idle,
        };
        let history = History::load();
        let stats = Stats::load(&history.entries);
        // Icons for apps counted before Vox kept them.
        let missing: Vec<_> = stats
            .apps()
            .into_iter()
            .filter(|(app, _)| apps::icon(app).is_none())
            .map(|(app, _)| (app, None))
            .collect();
        Self::keep_icons(missing, cx);
        let groq_usage = demo_groq.unwrap_or_else(GroqUsage::load);
        let last_usage_sig = groq_usage.display_signature(now_ms());
        Self {
            settings,
            stats,
            history,
            local: Local::discover(),
            downloads: HashMap::new(),
            download_queue: VecDeque::new(),
            download_cancel: None,
            phase,
            levels: if demo.is_some() { Levels::demo() } else { Levels::default() },
            stage: Arc::new(AtomicU8::new(match demo.as_deref() {
                Some("loading-speech") => engines::LOADING_SPEECH,
                Some("loading-tidy") => engines::LOADING_TIDY,
                _ => engines::HEARING,
            })),
            groq: initial_groq,
            checking_groq_key: false,
            groq_key_revision: 0,
            groq_usage,
            last_usage_sig,
            key_notice: None,
            key_attempt: 0,
            can_type: paste::allowed(),
            mic_permission: audio::microphone_permission(),
            shortcut_notice,
            kept: Kept::count(),
            listening_since: Instant::now(),
            target_app: None,
            recorder: None,
            preload: None,
            pressed_at: Instant::now(),
            hotkeys,
            run: 0,
        }
    }

    /// Whether `avenue` can hear right now.
    pub fn ready(&self, avenue: Avenue) -> bool {
        match avenue {
            Avenue::Cloud => matches!(self.groq, GroqKey::Saved(_)),
            Avenue::Local => self.active_model().is_some(),
        }
    }

    /// The speech variant that would answer a dictation now.
    pub fn active_model(&self) -> Option<LocalModel> {
        self.local.active(self.settings.local_model, self.settings.last_ready_model)
    }

    /// The shortcut, as the raw hot key event that arrived.
    pub fn on_hotkey(&mut self, event: (u32, bool), cx: &mut Context<Self>) {
        let Some(press) = self.hotkeys.as_ref().and_then(|h| h.read(event)) else {
            return;
        };
        match (press, &self.phase) {
            (Press::Down | Press::Finish, Phase::Listening { hands_free: true }) => self.finish(cx),
            (Press::Down, Phase::Listening { .. } | Phase::Working) => {}
            (Press::Down, _) => self.listen(cx),
            (Press::Up, Phase::Listening { hands_free: false }) => {
                if self.pressed_at.elapsed() < TAP {
                    self.phase = Phase::Listening { hands_free: true };
                    self.enter(true);
                    cx.notify();
                } else {
                    self.finish(cx);
                }
            }
            (Press::Cancel, Phase::Listening { .. } | Phase::Working) => self.cancel(cx),
            _ => {}
        }
    }

    pub fn finish_dictation(&mut self, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Listening { hands_free: true }) {
            self.finish(cx);
        }
    }

    pub fn cancel_dictation(&mut self, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Listening { .. } | Phase::Working) {
            self.cancel(cx);
        }
    }

    fn listen(&mut self, cx: &mut Context<Self>) {
        self.run += 1;
        if self.settings.avenue == Avenue::Cloud
            && matches!(self.groq, GroqKey::Rejected(_))
            && !(self.settings.fallback && self.ready(Avenue::Local))
        {
            return self.fail_with_action(groq::REJECTED_KEY.into(), PillAction::OpenGroqSettings, cx);
        }
        self.pressed_at = Instant::now();
        self.listening_since = Instant::now();
        self.mic_permission = audio::microphone_permission();
        if self.mic_permission == MicPermission::Denied {
            return self.fail_with_action(
                "Allow microphone access".into(),
                PillAction::OpenMicSettings,
                cx,
            );
        }
        // Vox's own window takes no dictation, so it is no destination.
        self.target_app = apps::frontmost().filter(|app| app.name != "Vox" && app.name != "vox");
        match Recorder::start() {
            Ok(recorder) => {
                self.levels = recorder.levels();
                self.recorder = Some(recorder);
                self.phase = Phase::Listening { hands_free: false };
                self.escape(true);
                self.warm(cx);
            }
            Err(e) => {
                eprintln!("[vox] Microphone: {e}");
                self.mic_permission = audio::microphone_permission();
                if self.mic_permission == MicPermission::Denied {
                    return self.fail_with_action(
                        "Allow microphone access".into(),
                        PillAction::OpenMicSettings,
                        cx,
                    );
                }
                return self.fail("Couldn't open microphone".into(), cx);
            }
        }
        cx.defer(ui::pill::show);
        cx.notify();
    }

    /// Start the local helpers while the speaker talks.
    fn warm(&mut self, cx: &mut Context<Self>) {
        let s = &self.settings;
        let first = if self.ready(s.avenue) || !s.fallback { s.avenue } else { s.avenue.other() };
        if first == Avenue::Local && let Some(model) = self.active_model() {
            self.preload = Some(engines::Preload::start(
                model,
                self.local.clone(),
                s.language.clone(),
            ));
        }
        if let (true, Avenue::Local, Some(launch)) = (s.tidy, first, self.local.s1()) {
            cx.background_spawn(async move { engines::s1::warm(&launch) }).detach();
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.enter(false);
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        let clip = match recorder.finish() {
            Ok(clip) => clip,
            Err(e) => {
                eprintln!("[vox] Microphone: {e}");
                return self.fail("Couldn't record audio. Try again.".into(), cx);
            }
        };
        if clip.seconds < TOO_SHORT {
            return self.rest(cx);
        }
        if !clip.heard_anything {
            self.mic_permission = audio::microphone_permission();
            if self.mic_permission == MicPermission::Denied {
                return self.fail_with_action(
                    "Allow microphone access".into(),
                    PillAction::OpenMicSettings,
                    cx,
                );
            }
            return self.fail("No audio detected — check your mic".into(), cx);
        }
        self.stage = Arc::new(AtomicU8::new(engines::HEARING));
        self.phase = Phase::Working;
        cx.notify();

        let (settings, local, stage, run) =
            (self.settings.clone(), self.local.clone(), self.stage.clone(), self.run);
        let preload = self.preload.take();
        let key_revision = self.groq_key_revision;
        let key_rejected = matches!(self.groq, GroqKey::Rejected(_));
        cx.spawn(async move |this, cx| {
            let seconds = clip.seconds;
            let (dictation, wav) = cx
                .background_spawn(async move {
                    let dictation =
                        engines::run(&settings, &local, &clip.wav, &stage, key_rejected, preload);
                    (dictation, clip.wav)
                })
                .await;
            this.update(cx, |this, cx| this.deliver(run, key_revision, dictation, wav, seconds, cx))
                .ok();
        })
        .detach();
    }

    fn deliver(
        &mut self,
        run: u64,
        key_revision: u64,
        dictation: Dictation,
        wav: Vec<u8>,
        spoken_secs: f32,
        cx: &mut Context<Self>,
    ) {
        if run != self.run {
            return;
        }
        let same_key = key_revision == self.groq_key_revision;
        let groq_key_rejected = same_key && dictation.groq_key_rejected;
        if groq_key_rejected && let GroqKey::Saved(masked) = &self.groq {
            self.groq = GroqKey::Rejected(masked.clone());
            self.groq_usage.clear();
            self.last_usage_sig = [None, None];
        }
        let now = now_ms();
        let groq = if same_key && !groq_key_rejected {
            let warnings = self.groq_usage.merge_and_take_warnings(&dictation.groq_usage, now);
            self.last_usage_sig = self.groq_usage.display_signature(now);
            warnings
        } else {
            GroqUsage::default()
        };
        let outcome = match dictation.outcome {
            Ok(outcome) => outcome,
            Err(e) if groq_key_rejected => {
                return self.fail_with_action(e, PillAction::OpenGroqSettings, cx);
            }
            Err(e) => return self.fail(e, cx),
        };
        if self.settings.keep_recordings {
            self.keep(wav, &outcome, spoken_secs, cx);
        }
        self.can_type = paste::allowed();
        let typed = self.settings.type_into_app && self.can_type;
        let needs_accessibility = self.settings.type_into_app && !self.can_type;
        let before = cx.read_from_clipboard();
        cx.write_to_clipboard(ClipboardItem::new_string(outcome.text.clone()));
        if typed {
            let restore = before.filter(|_| self.settings.keep_clipboard);
            cx.spawn(async move |_, cx| {
                // The pasteboard needs a beat before the app in front reads it,
                // and that app needs one before its clipboard is put back.
                cx.background_executor().timer(Duration::from_millis(70)).await;
                paste::press_paste().ok();
                if let Some(item) = restore {
                    cx.background_executor().timer(Duration::from_millis(700)).await;
                    cx.update(|cx| cx.write_to_clipboard(item)).ok();
                }
            })
            .detach();
        }
        let target = self.target_app.take();
        let entry = Entry {
            at: now / 1000,
            text: outcome.text,
            heard: outcome.heard,
            avenue: outcome.avenue,
            spoken_secs,
            app: target.as_ref().map(|app| app.name.clone()),
        };
        let words = entry.words();
        self.stats.add(&entry);
        if let Some(app) = target {
            Self::keep_icons(vec![(app.name, app.bundle)], cx);
        }
        self.history.push(entry);
        let has_groq_bar = !groq.is_empty_at(now);
        let linger = if groq_key_rejected {
            6_000
        } else if needs_accessibility {
            5_000
        } else if outcome.note.is_some() || !typed || has_groq_bar {
            3_200
        } else {
            1_300
        };
        self.phase = Phase::Done {
            words,
            typed,
            needs_accessibility,
            note: outcome.note,
            groq_key_rejected,
            groq,
        };
        self.escape(false);
        self.settle(Duration::from_millis(linger), cx);
        cx.notify();
    }

    /// Keep each app's icon for Stats, off the main thread, and show any
    /// that were new.
    fn keep_icons(wanted: Vec<(String, Option<String>)>, cx: &mut Context<Self>) {
        if wanted.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let kept = cx
                .background_spawn(async move {
                    wanted
                        .iter()
                        .filter(|(app, bundle)| apps::keep_icon(app, bundle.as_deref()))
                        .count()
                })
                .await;
            if kept > 0 {
                this.update(cx, |_, cx| cx.notify()).ok();
            }
        })
        .detach();
    }

    /// Keep the clip and its result. A cancelled dictation never gets here,
    /// and one that cannot be saved is still typed.
    fn keep(&mut self, wav: Vec<u8>, outcome: &Outcome, seconds: f32, cx: &mut Context<Self>) {
        let record = Record::new(outcome, seconds, &self.settings.language, &self.local);
        cx.spawn(async move |this, cx| {
            let bytes = wav.len();
            let kept = cx.background_spawn(async move { recordings::keep(&wav, &record) }).await;
            if let Err(e) = kept {
                return eprintln!("[vox] {e}");
            }
            this.update(cx, |this, cx| {
                this.kept.add(bytes);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.recorder = None;
        self.run += 1;
        self.rest(cx);
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.recorder = None;
        self.discard_preload(cx);
        self.phase = Phase::Failed { message, action: None };
        self.escape(false);
        self.enter(false);
        cx.defer(ui::pill::show);
        self.settle(Duration::from_millis(4_500), cx);
        cx.notify();
    }

    fn fail_with_action(&mut self, message: String, action: PillAction, cx: &mut Context<Self>) {
        self.recorder = None;
        self.discard_preload(cx);
        self.phase = Phase::Failed { message, action: Some(action) };
        self.escape(false);
        self.enter(false);
        cx.defer(ui::pill::show);
        self.settle(Duration::from_millis(6_000), cx);
        cx.notify();
    }

    /// Run an interactive pill button action and dismiss the pill.
    pub fn run_pill_action(&mut self, action: PillAction, cx: &mut Context<Self>) {
        match action {
            PillAction::OpenMicSettings => paste::open_microphone_settings(),
            PillAction::OpenAccessibilitySettings => paste::open_accessibility_settings(),
            PillAction::OpenGroqSettings => cx.defer(crate::show_engine),
        }
        self.rest(cx);
    }

    fn discard_preload(&mut self, cx: &mut Context<Self>) {
        if let Some(preload) = self.preload.take() {
            cx.background_spawn(async move { drop(preload) }).detach();
        }
    }

    /// Back to idle now.
    fn rest(&mut self, cx: &mut Context<Self>) {
        self.discard_preload(cx);
        self.phase = Phase::Idle;
        self.escape(false);
        self.enter(false);
        cx.defer(ui::pill::hide);
        cx.notify();
    }

    /// Back to idle after the pill has shown its last word, unless another
    /// dictation started meanwhile.
    fn settle(&mut self, after: Duration, cx: &mut Context<Self>) {
        let run = self.run;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            this.update(cx, |this, cx| {
                if this.run == run
                    && matches!(this.phase, Phase::Done { .. } | Phase::Failed { .. })
                {
                    this.rest(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn escape(&mut self, on: bool) {
        if let Some(hotkeys) = self.hotkeys.as_mut() {
            hotkeys.listen_for_escape(on);
        }
    }

    fn enter(&mut self, on: bool) {
        if let Some(hotkeys) = self.hotkeys.as_mut() {
            hotkeys.listen_for_enter(on);
        }
    }

    /// Change the settings and keep them.
    pub fn set(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        change(&mut self.settings);
        self.settings.save();
        cx.notify();
    }

    pub fn set_shortcut(&mut self, shortcut: Shortcut, cx: &mut Context<Self>) {
        let Some(hotkeys) = self.hotkeys.as_mut() else {
            return;
        };
        match hotkeys.set_shortcut(shortcut) {
            Ok(()) => {
                self.shortcut_notice = None;
                self.set(
                    |s| {
                        s.shortcut = shortcut;
                        if !shortcut.is_preset() {
                            s.custom_shortcut = Some(shortcut);
                        }
                    },
                    cx,
                );
            }
            Err(_) => {
                hotkeys.set_paused(false);
                self.shortcut_notice =
                    Some("Shortcut in use. Kept your previous one.".into());
                cx.notify();
            }
        }
    }

    /// Pause or resume the global talk shortcut while the user records a new one.
    pub fn set_shortcut_recording(&mut self, recording: bool, cx: &mut Context<Self>) {
        if recording && matches!(self.phase, Phase::Listening { .. }) {
            self.cancel(cx);
        }
        let Some(hotkeys) = self.hotkeys.as_mut() else {
            return;
        };
        if hotkeys.is_paused() == recording {
            return;
        }
        hotkeys.set_paused(recording);
        if !recording {
            self.shortcut_notice =
                (!hotkeys.taken()).then(|| "Shortcut in use. Choose another.".into());
            cx.notify();
        }
    }

    /// Try to register a newly recorded shortcut. Leaves the global shortcut
    /// paused on failure so the user can immediately try another combination.
    pub fn apply_recorded_shortcut(
        &mut self,
        shortcut: Shortcut,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(hotkeys) = self.hotkeys.as_mut() else {
            return Err("Couldn't register the shortcut".into());
        };
        match hotkeys.set_shortcut(shortcut) {
            Ok(()) => {
                self.shortcut_notice = None;
                self.set(
                    |s| {
                        s.shortcut = shortcut;
                        if !shortcut.is_preset() {
                            s.custom_shortcut = Some(shortcut);
                        }
                    },
                    cx,
                );
                Ok(())
            }
            Err(_) => Err(format!("{} is in use by another app. Try another.", shortcut.display())),
        }
    }

    /// Look again for what changes outside the app: Accessibility access and
    /// the local helpers and models.
    pub fn look_again(&mut self, cx: &mut Context<Self>) {
        self.local = Local::discover();
        self.kept = Kept::count();
        self.can_type = paste::allowed();
        self.mic_permission = audio::microphone_permission();
        cx.notify();
    }

    /// Choose a speech variant. One that is not on this Mac starts downloading
    /// at once; the variant that was ready answers until it is.
    pub fn choose_model(&mut self, model: LocalModel, cx: &mut Context<Self>) {
        if model.family() == Family::Cohere && !cohere::supported() {
            return;
        }
        let previous = self.settings.local_model;
        let was_ready = self.local.can_hear(previous);
        self.set(
            |s| {
                if was_ready && previous != model {
                    s.last_ready_model = Some(previous);
                }
                s.local_model = model;
            },
            cx,
        );
        // Without uv, Cohere cannot be fetched; Engine says to install it.
        let fetchable = model.family() == Family::Whisper || cohere::installer().is_some();
        if !self.local.downloaded.contains(&model) && fetchable {
            self.download_model(Model::Speech(model), cx);
        }
    }

    pub fn download_model(&mut self, model: Model, cx: &mut Context<Self>) {
        if self.download_queue.contains(&model) {
            return;
        }
        self.download_queue.push_back(model);
        self.downloads.insert(model, DownloadState::Queued);
        self.start_next_download(cx);
        cx.notify();
    }

    fn start_next_download(&mut self, cx: &mut Context<Self>) {
        if self.download_cancel.is_some() {
            return;
        }
        let Some(&model) = self.download_queue.front() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.download_cancel = Some(cancel.clone());
        self.downloads.insert(model, DownloadState::Working(Progress::Preparing));
        cx.spawn(async move |this, cx| {
            let (updates, mut progress) = mpsc::unbounded();
            let task = cx.background_spawn(async move {
                model.install(&cancel, |stage| {
                    updates.unbounded_send(stage).ok();
                })
            });
            while let Some(stage) = progress.next().await {
                this.update(cx, |this, cx| {
                    if let Some(DownloadState::Working(progress)) = this.downloads.get_mut(&model) {
                        *progress = stage;
                        cx.notify();
                    }
                })
                .ok();
            }
            let result = task.await;
            this.update(cx, |this, cx| {
                this.downloads.remove(&model);
                this.download_queue.pop_front();
                this.download_cancel = None;
                if let Err(error) = result && error != CANCELLED {
                    this.downloads.insert(model, DownloadState::Failed(error));
                }
                this.look_again(cx);
                this.start_next_download(cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel_download(&mut self, model: Model, cx: &mut Context<Self>) {
        match self.downloads.get(&model) {
            Some(DownloadState::Working(_)) => {
                if let Some(cancel) = &self.download_cancel {
                    cancel.store(true, Ordering::Relaxed);
                    self.downloads.insert(model, DownloadState::Cancelling);
                }
            }
            Some(DownloadState::Queued) => {
                self.download_queue.retain(|queued| *queued != model);
                self.downloads.remove(&model);
            }
            _ => return,
        }
        cx.notify();
    }

    pub fn dismiss_download_failure(&mut self, model: Model, cx: &mut Context<Self>) {
        if matches!(self.downloads.get(&model), Some(DownloadState::Failed(_))) {
            self.downloads.remove(&model);
            cx.notify();
        }
    }

    /// Delete one downloaded speech model from this Mac and refresh availability.
    pub fn delete_model(&mut self, model: LocalModel, cx: &mut Context<Self>) {
        if self.downloads.contains_key(&Model::Speech(model)) {
            return;
        }
        self.discard_preload(cx);
        cx.spawn(async move |this, cx| {
            let removed = cx.background_spawn(async move { engines::remove_model(model) }).await;
            if let Err(error) = removed {
                eprintln!("[vox] {error}");
                return;
            }
            this.update(cx, |this, cx| {
                this.local = Local::discover();
                if this.settings.last_ready_model == Some(model) {
                    this.set(|s| s.last_ready_model = None, cx);
                } else {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The periodic check for permissions and live Groq reset countdowns.
    pub fn check_typing(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let allowed = paste::allowed();
        if allowed != self.can_type {
            self.can_type = allowed;
            changed = true;
        }
        let mic = audio::microphone_permission();
        if mic != self.mic_permission {
            self.mic_permission = mic;
            changed = true;
        }
        let sig = self.groq_usage.display_signature(now_ms());
        if sig != self.last_usage_sig {
            self.last_usage_sig = sig;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// Save the Groq key on the clipboard, once Groq has accepted it, and
    /// with `then_use` switch to Groq.
    pub fn paste_key(&mut self, then_use: bool, cx: &mut Context<Self>) {
        if self.checking_groq_key || self.groq == GroqKey::Loading {
            return;
        }
        self.key_attempt += 1;
        let pasted = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .map(|text| text.trim().to_string())
            .unwrap_or_default();
        if !groq::looks_like_key(&pasted) {
            let message = if pasted.is_empty() {
                "Copy a Groq API key first, then try again."
            } else {
                "Clipboard text doesn’t look like a Groq API key. Copy a key and try again."
            };
            return self.show_key_notice(message.into(), cx);
        }
        self.checking_groq_key = true;
        self.key_notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn(async move {
                    groq::check(&pasted).map_err(|error| match error {
                        groq::Error::RejectedKey => {
                            "Groq didn’t accept this key. Copy a current key and try again.".to_string()
                        }
                        groq::Error::RateLimited { message, .. } | groq::Error::Other(message) => {
                            message
                        }
                    })?;
                    keys::set_groq(&pasted)?;
                    Ok::<_, String>(keys::masked(&pasted))
                })
                .await;
            this.update(cx, |this, cx| {
                this.checking_groq_key = false;
                match saved {
                    Ok(masked) => {
                        this.groq_key_revision += 1;
                        this.groq = GroqKey::Saved(masked);
                        this.groq_usage.clear();
                        this.last_usage_sig = [None, None];
                        if then_use {
                            this.set(|s| s.avenue = Avenue::Cloud, cx);
                        } else {
                            cx.notify();
                        }
                    }
                    Err(e) => this.show_key_notice(e, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn show_key_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.key_notice = Some(message);
        let attempt = self.key_attempt;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(7)).await;
            this.update(cx, |this, cx| {
                if this.key_attempt == attempt {
                    this.dismiss_key_notice(cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn dismiss_key_notice(&mut self, cx: &mut Context<Self>) {
        if self.key_notice.take().is_some() {
            cx.notify();
        }
    }

    pub fn remove_key(&mut self, cx: &mut Context<Self>) {
        if self.checking_groq_key {
            return;
        }
        keys::remove_groq();
        self.groq_key_revision += 1;
        self.groq = GroqKey::Missing;
        self.groq_usage.clear();
        self.last_usage_sig = [None, None];
        self.key_notice = None;
        cx.notify();
    }

    pub fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        cx.notify();
    }
}

/// The state, from anywhere on the main thread.
pub fn vox(cx: &App) -> Entity<Vox> {
    cx.global::<Handle>().0.clone()
}
