//! The main window: Home (the shortcut and what was dictated), Stats (how
//! much and where), Engine (the two avenues) and Settings.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    anchored, AnyElement, canvas, deferred, div, ease_out_quint, point, prelude::*,
    pulsating_between, px, relative, Animation, AnimationExt as _, App, Bounds, ClickEvent,
    ClipboardItem, Context, Corner, Div, Entity, FocusHandle, FontWeight, KeyDownEvent, Keystroke,
    ModifiersChangedEvent, MouseButton, PathBuilder, Pixels, SharedString, Stateful, Subscription,
    Window,
};

use super::icons::Icon;
use super::{action_button, button, button_with_icon, click, hover_color, hover_transition, keycaps, status_dot, toggle, Theme, SERIF};
use crate::audio::MicPermission;
use crate::config::{Avenue, Entry, Family, LocalModel, Shortcut, ShortcutModifiers};
use crate::engines::download::{Model, Progress};
use crate::engines::models::{self, Fit, Spec};
use crate::engines::{cohere};
use crate::engines::groq_usage::{grouped_u64, now_ms, Role, UsageRow};
use crate::paste;
use crate::state::{DownloadState, GroqKey, Phase, Vox};
use crate::stats::{self, local_noon, DAY};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Home,
    Stats,
    Engine,
    Settings,
}

/// How Stats draws the words of each day.
#[derive(Clone, Copy, PartialEq)]
enum Activity {
    Chart,
    Heatmap,
}

pub struct MainView {
    vox: Entity<Vox>,
    tab: Tab,
    activity: Activity,
    /// The day under the pointer in the activity view, by its local noon.
    hovered: Option<u64>,
    /// The day clicked in the activity view to filter Stats, by its local noon.
    selected: Option<u64>,
    /// The entry whose Copy was just pressed, by its time.
    copied: Option<u64>,
    /// The page column's inner width, set at the start of each render.
    width: f32,
    /// The window's height, likewise: the menu is no taller than the room under its trigger.
    height: f32,
    /// Whether the Transcribe model menu is open.
    model_menu: bool,
    /// Which provider tab is shown in the model menu; unset, the chosen model's.
    menu_family: Option<Family>,
    /// Whether the menu's "More models" section is expanded; unset, it is when
    /// one is in play.
    smaller_models: Option<bool>,
    /// Focus for the open menu, so Esc reaches it.
    menu_focus: FocusHandle,
    /// Where the menu's trigger was last painted: what the menu hangs from,
    /// and what a click outside the menu does not count as outside.
    trigger_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Whether Settings is currently capturing a custom shortcut.
    recording_shortcut: bool,
    /// Modifiers held right now while recording a custom shortcut.
    recorded_modifiers: ShortcutModifiers,
    /// Inline reason a recorded key combination couldn't be used.
    record_error: Option<String>,
    /// Focus handle for the custom shortcut recorder row.
    recorder_focus: FocusHandle,
    /// Intercepts all keystrokes while recording so app shortcuts don't fire.
    record_interceptor: Option<Subscription>,
    /// Cancels recording if the window loses focus.
    record_activation: Option<Subscription>,
    _release: Subscription,
    _watch: Subscription,
}

/// The page column's widest, and the space either side of it.
///
/// GPUI wraps text at the width its last measurement was given, which in a
/// flex row is not always the width it ends up with. So every block of
/// wrapping text beside something else is given its width outright, worked
/// out from the column's.
const COLUMN: f32 = 760.;
const GUTTER: f32 = 56.;

/// How many dictations Home lists.
const SHOWN: usize = 80;

/// A comfortable typing speed, for the time-saved estimate.
const TYPING_WPM: f32 = 40.;

impl MainView {
    pub fn new(vox: Entity<Vox>, cx: &mut Context<Self>) -> Self {
        // `VOX_MENU=open` opens the Transcribe menu once the first frame has
        // placed its trigger.
        if std::env::var("VOX_MENU").as_deref() == Ok("open") {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(400)).await;
                this.update(cx, |this, cx| {
                    this.model_menu = true;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        let _release = cx.on_release(|this: &mut Self, cx| {
            if this.recording_shortcut {
                this.vox.update(cx, |vox, cx| vox.set_shortcut_recording(false, cx));
            }
        });
        Self {
            _watch: cx.observe(&vox, |_, _, cx| cx.notify()),
            _release,
            vox,
            // `VOX_TAB` opens a development run on another page.
            tab: match std::env::var("VOX_TAB").as_deref() {
                Ok("engine") => Tab::Engine,
                Ok("stats") => Tab::Stats,
                Ok("settings") => Tab::Settings,
                _ => Tab::Home,
            },
            activity: Activity::Chart,
            hovered: None,
            selected: None,
            copied: None,
            width: COLUMN - 2. * GUTTER,
            height: 680.,
            model_menu: false,
            menu_family: None,
            smaller_models: None,
            menu_focus: cx.focus_handle(),
            trigger_bounds: Rc::default(),
            recording_shortcut: false,
            recorded_modifiers: ShortcutModifiers::NONE,
            record_error: None,
            recorder_focus: cx.focus_handle(),
            record_interceptor: None,
            record_activation: None,
        }
    }

    /// A click that changes the app's state.
    fn act(
        &self,
        f: impl Fn(&mut Vox, &mut Context<Vox>) + 'static,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let vox = self.vox.clone();
        move |_, _, cx| vox.update(cx, |vox, cx| f(vox, cx))
    }

    fn go(&self, tab: Tab, cx: &Context<Self>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _, _, cx| {
            this.stop_recording_shortcut(cx);
            this.tab = tab;
            cx.notify();
        })
    }

    pub fn show_engine(&mut self, cx: &mut Context<Self>) {
        self.stop_recording_shortcut(cx);
        self.tab = Tab::Engine;
        cx.notify();
    }

    fn start_recording_shortcut(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording_shortcut {
            window.focus(&self.recorder_focus);
            return;
        }
        self.recording_shortcut = true;
        self.recorded_modifiers = ShortcutModifiers::NONE;
        self.record_error = None;
        self.vox.update(cx, |vox, cx| vox.set_shortcut_recording(true, cx));
        window.focus(&self.recorder_focus);

        let view = cx.weak_entity();
        self.record_interceptor = Some(cx.intercept_keystrokes(move |event, _, cx| {
            cx.stop_propagation();
            view.update(cx, |this, cx| this.on_record_keystroke(&event.keystroke, cx)).ok();
        }));
        self.record_activation = Some(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.stop_recording_shortcut(cx);
            }
        }));
        cx.notify();
    }

    fn stop_recording_shortcut(&mut self, cx: &mut Context<Self>) {
        if !self.recording_shortcut {
            return;
        }
        self.recording_shortcut = false;
        self.recorded_modifiers = ShortcutModifiers::NONE;
        self.record_error = None;
        self.record_interceptor = None;
        self.record_activation = None;
        self.vox.update(cx, |vox, cx| vox.set_shortcut_recording(false, cx));
        cx.notify();
    }

    fn on_record_keystroke(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        if !self.recording_shortcut {
            return;
        }
        let modifiers = ShortcutModifiers {
            control: keystroke.modifiers.control,
            alt: keystroke.modifiers.alt,
            shift: keystroke.modifiers.shift,
            command: keystroke.modifiers.platform,
        };
        if keystroke.key == "escape" && modifiers.is_empty() {
            self.stop_recording_shortcut(cx);
            return;
        }
        match Shortcut::from_gpui_keystroke(modifiers, &keystroke.key) {
            Ok(shortcut) => {
                let result = self.vox.update(cx, |vox, cx| vox.apply_recorded_shortcut(shortcut, cx));
                match result {
                    Ok(()) => {
                        self.recording_shortcut = false;
                        self.recorded_modifiers = ShortcutModifiers::NONE;
                        self.record_error = None;
                        self.record_interceptor = None;
                        self.record_activation = None;
                        cx.notify();
                    }
                    Err(message) => {
                        if self.record_error.as_deref() != Some(message.as_str()) {
                            self.record_error = Some(message);
                            cx.notify();
                        }
                    }
                }
            }
            Err(error) => {
                let message = error.message();
                if self.record_error.as_deref() != Some(message) {
                    self.record_error = Some(message.into());
                    cx.notify();
                }
            }
        }
    }

    fn key_button(
        &self,
        t: &Theme,
        label: &'static str,
        solid: bool,
        then_use: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let vox = self.vox.read(cx);
        let (label, icon) = if vox.checking_groq_key {
            ("Checking…", Icon::Key)
        } else if vox.key_notice.is_some() {
            ("Try again", Icon::Close)
        } else {
            (label, Icon::Key)
        };
        action_button(
            t,
            "groq-key-action",
            label,
            Some(icon),
            solid,
            (!vox.checking_groq_key).then(|| click(self.act(move |vox, cx| vox.paste_key(then_use, cx)))),
        )
    }

    fn key_toast(&self, t: &Theme, message: String) -> impl IntoElement {
        div()
            .absolute()
            .bottom(px(20.))
            .right(px(24.))
            .w(px(344.))
            .flex()
            .items_start()
            .gap(px(10.))
            .px(px(14.))
            .py(px(12.))
            .rounded(px(12.))
            .bg(t.glass)
            .border_1()
            .border_color(t.glass_edge)
            .shadow(t.glass_shadow())
            .occlude()
            .child(Icon::Alert.render(16., t.live))
            .child(
                div()
                    .w(px(256.))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Couldn’t save Groq key"))
                    .child(div().text_size(px(12.)).line_height(relative(1.4)).text_color(t.soft).child(message)),
            )
            .child(
                {
                    let dismiss = self.act(Vox::dismiss_key_notice);
                    let sunken = t.sunken;
                    let icon = Icon::Close.render(13., t.soft);
                    hover_transition("dismiss-groq-key-error", true, move |hover, press| {
                        div()
                            .id("dismiss-groq-key-error")
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .bg(sunken.opacity(hover))
                            .opacity(1. - 0.18 * press)
                            .child(icon)
                            .on_click(dismiss)
                    })
                },
            )
            .with_animation(
                "groq-key-error",
                Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
                |toast, delta| toast.opacity(delta),
            )
    }

    fn top_bar(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let tab = |tab: Tab, label: &'static str, icon: Icon, cx: &Context<Self>| {
            let active = self.tab == tab;
            let ink = if active { t.ink } else { t.soft };
            let theme = *t;
            let go = click(self.go(tab, cx));
            hover_transition(label, true, move |hover, press| {
                let ink = hover_color(ink, theme.ink, hover);
                let fill = if active {
                    hover_color(theme.sunken, theme.line, 0.5 * hover)
                } else {
                    theme.sunken.opacity(0.6 * hover)
                };
                div()
                    .id(label)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(11.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ink)
                    .bg(fill)
                    .opacity(1. - 0.18 * press)
                    .cursor_pointer()
                    .child(icon.render(14., ink))
                    .child(label)
                    .on_click(go)
            })
        };
        let listening = matches!(self.vox.read(cx).phase, Phase::Listening { .. });
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .h(px(52.))
            // Clear of the traffic lights.
            .pl(px(92.))
            .pr(px(20.))
            .gap(px(2.))
            .child(tab(Tab::Home, "Home", Icon::Home, cx))
            .child(tab(Tab::Stats, "Stats", Icon::BarChart, cx))
            .child(tab(Tab::Engine, "Engine", Icon::Layers, cx))
            .child(tab(Tab::Settings, "Settings", Icon::Settings, cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()),
            )
            .when(listening, |bar| {
                bar.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .text_size(px(12.))
                        .text_color(t.soft)
                        .child(div().size(px(7.)).rounded_full().bg(t.live))
                        .child("Listening"),
                )
            })
    }

    fn home(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let vox = self.vox.read(cx);
        let keys = vox.settings.shortcut.keys();

        let headline = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(11.))
            .font_family(SERIF)
            .text_size(px(34.))
            .line_height(relative(1.25))
            .child("Hold")
            .child(keycaps(t, &keys, 17.).font_family(".SystemUIFont"))
            .child("and speak.");

        let lede = div()
            .max_w(px(540.))
            .text_size(px(14.))
            .line_height(relative(1.55))
            .text_color(t.soft)
            .child("Tap for hands-free. Press again or Enter to finish. Esc cancels.");

        let mut page = column().child(div().flex().flex_col().gap(px(14.)).child(headline).child(lede));

        page = page.child(self.avenue_switch(t, cx));

        for notice in self.notices(t, cx) {
            page = page.child(notice);
        }
        page.children(self.pace_card(t, cx)).child(self.dictations(t, cx))
    }

    /// The two avenues as one switch, with what the chosen one runs.
    fn avenue_switch(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let vox = self.vox.read(cx);
        let chosen = vox.settings.avenue;
        let option = |avenue: Avenue, id: &'static str, label: &'static str| {
            let active = chosen == avenue;
            let theme = *t;
            let ready = status_dot(t, vox.ready(avenue));
            let select = self.act(move |vox, cx| vox.set(|s| s.avenue = avenue, cx));
            hover_transition(id, true, move |hover, press| {
                let ink = hover_color(theme.soft, theme.ink, if active { 1. } else { hover });
                let fill = if active {
                    hover_color(theme.surface, theme.line, 0.3 * hover)
                } else {
                    theme.surface.opacity(0.55 * hover)
                };
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(5.))
                    .rounded(px(7.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ink)
                    .bg(fill)
                    .when(active, |d| d.shadow(theme.lift()))
                    .opacity(1. - 0.18 * press)
                    .cursor_pointer()
                    .child(avenue_icon(avenue).render(15., ink))
                    .child(label)
                    .child(ready)
                    .on_click(select)
            })
        };
        let (speech, speech_icon, cleanup, cleanup_icon) = match chosen {
            Avenue::Local => {
                // What answers, which is not the chosen one until it is ready.
                let model = vox.active_model().unwrap_or(vox.settings.local_model);
                (
                    format!("{} · {}", model.family().label(), models::spec(model).variant),
                    family_icon(model.family()),
                    "S1-mini",
                    Icon::Superwhisper,
                )
            }
            Avenue::Cloud => (Family::Whisper.label().to_string(), Icon::OpenAi, "Qwen", Icon::Qwen),
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .p(px(3.))
                    .rounded(px(10.))
                    .bg(t.sunken)
                    .child(option(Avenue::Local, "use-local", "On this Mac"))
                    .child(option(Avenue::Cloud, "use-cloud", "Groq")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .whitespace_nowrap()
                    .text_size(px(13.))
                    .text_color(t.soft)
                    .child(speech_icon.render(14., t.soft))
                    .child(speech)
                    .when(vox.settings.tidy, |models| {
                        models.child("+").child(cleanup_icon.render(16., t.soft)).child(cleanup)
                    }),
            )
    }

    /// Speaking pace, with the week beside it and the way to Stats.
    fn pace_card(&self, t: &Theme, cx: &Context<Self>) -> Option<Div> {
        let vox = self.vox.read(cx);
        let wpm = vox.stats.wpm()?;
        let (words, spoken) = vox.history.week();
        let saved = (words as f32 / TYPING_WPM - spoken / 60.).round();
        let week = div()
            .flex()
            .items_baseline()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(12.))
            .text_color(t.soft);
        let week = match (words, saved >= 1.) {
            (0, _) => week.child("Nothing dictated this week yet"),
            (_, true) => week
                .child(strong(t, format!("{} {}", grouped(words), if words == 1 { "word" } else { "words" })))
                .child(" this week, about ")
                .child(strong(t, format!("{saved} min")))
                .child(" of typing saved"),
            (_, false) => week
                .child(strong(t, format!("{} {}", grouped(words), if words == 1 { "word" } else { "words" })))
                .child(" this week"),
        };
        let pace = div()
            .flex()
            .items_baseline()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(13.))
            .text_color(t.ink)
            .child("Speaking pace, ")
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(format!("{:.1}× faster", wpm / TYPING_WPM)),
            )
            .child(" than typing");
        const NUMBER: f32 = 112.;
        const STATS_BUTTON: f32 = 116.;
        let detail = self.width - 2. * 18. - 2. - NUMBER - STATS_BUTTON - 2. * 18.;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(18.))
                .px(px(18.))
                .py(px(14.))
                .rounded(px(12.))
                .bg(t.surface)
                .border_1()
                .border_color(t.line)
                .child(
                    div()
                        .w(px(NUMBER))
                        .flex()
                        .items_baseline()
                        .gap(px(6.))
                        .child(
                            div()
                                .font_family(SERIF)
                                .text_size(px(32.))
                                .line_height(relative(1.1))
                                .child(format!("{}", wpm.round() as usize)),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.soft)
                                .child("WPM"),
                        ),
                )
                .child(
                    div()
                        .w(px(detail))
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .pl(px(18.))
                        .border_l_1()
                        .border_color(t.line)
                        .child(pace)
                        .child(week),
                )
                .child(
                    div().w(px(STATS_BUTTON)).flex().justify_end().child(button_with_icon(
                        t,
                        "see-stats",
                        "See stats",
                        Icon::BarChart,
                        false,
                        click(self.go(Tab::Stats, cx)),
                    )),
                ),
        )
    }

    /// What stands between the speaker and a working dictation, most
    /// blocking first.
    fn notices(&self, t: &Theme, cx: &Context<Self>) -> Vec<Div> {
        let vox = self.vox.read(cx);
        let mut out = Vec::new();
        if let Some(why) = &vox.shortcut_notice {
            out.push(notice(
                t,
                self.width,
                why.clone(),
                Some(button(t, "fix-shortcut", "Choose a shortcut", false, click(self.go(Tab::Settings, cx)))),
            ));
        }
        let (chosen, other) = (vox.settings.avenue, vox.settings.avenue.other());
        if !vox.ready(chosen) && vox.groq != GroqKey::Loading {
            let said = if vox.settings.fallback && vox.ready(other) {
                format!("{} unavailable. Using {}.", chosen.label(), other.label())
            } else {
                format!("{} needs setup", chosen.label())
            };
            out.push(notice(
                t,
                self.width,
                said,
                Some(button(t, "fix-engine", "Set it up", false, click(self.go(Tab::Engine, cx)))),
            ));
        }
        if vox.settings.type_into_app && !vox.can_type {
            out.push(notice(
                t,
                self.width,
                "Copied until you allow Accessibility".into(),
                Some(button(
                    t,
                    "fix-typing",
                    "Open Settings",
                    false,
                    click(|_, _, _| paste::open_accessibility_settings()),
                )),
            ));
        }
        out
    }

    fn dictations(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let vox = self.vox.read(cx);
        let entries = &vox.history.entries;
        if entries.is_empty() {
            return div().pt(px(4.)).child(
                div()
                    .max_w(px(460.))
                    .font_family(SERIF)
                    .text_size(px(17.))
                    .line_height(relative(1.55))
                    .text_color(t.faint)
                    .child("Your dictations will appear here."),
            );
        }

        let today = day_of(crate::config::now());
        let mut list = div().flex().flex_col();
        let mut last_day = None;
        for entry in entries.iter().rev().take(SHOWN) {
            let day = day_of(entry.at);
            if last_day != Some(day.key) {
                let first = last_day.is_none();
                last_day = Some(day.key);
                list = list.child(
                    div()
                        .pt(px(if first { 0. } else { 26. }))
                        .pb(px(8.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(day.heading(&today)),
                );
            }
            list = list.child(self.entry_row(t, entry, &day, cx));
        }
        div().pt(px(4.)).child(list)
    }

    fn entry_row(&self, t: &Theme, entry: &Entry, day: &Day, cx: &Context<Self>) -> impl IntoElement {
        let at = entry.at;
        let text = entry.text.clone();
        let copied = self.copied == Some(at);
        let copy = cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            this.copied = Some(at);
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(1_400)).await;
                this.update(cx, |this, cx| {
                    if this.copied == Some(at) {
                        this.copied = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        });
        let id = SharedString::from(format!("entry-{at}"));
        let theme = *t;
        let width = self.width;
        let text = entry.text.clone();
        let time = format!("{}:{:02}", day.hour, day.minute);
        hover_transition(id.clone(), false, move |reveal, _| {
            div()
                .id(id)
                .flex()
                .items_start()
                .gap(px(18.))
                .py(px(10.))
                .border_t_1()
                .border_color(theme.line)
                .child(
                    div()
                        .w(px(44.))
                        .flex_shrink_0()
                        .pt(px(4.))
                        .text_size(px(12.))
                        .text_color(theme.faint)
                        .child(time),
                )
                .child(
                    div()
                        .w(px(width - 44. - 66. - 2. * 18.))
                        .font_family(SERIF)
                        .text_size(px(16.))
                        .line_height(relative(1.5))
                        .child(text),
                )
                .child(
                    hover_transition("copy-dictation", true, move |hover, press| {
                        let (icon, label, ink) = if copied {
                            (Icon::Check, "Copied", theme.good)
                        } else {
                            (Icon::Copy, "Copy", theme.accent)
                        };
                        let tint = hover_color(theme.sunken.opacity(hover), theme.line, press);
                        div()
                            .id("copy-dictation")
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(4.))
                            .w(px(66.))
                            .flex_shrink_0()
                            .px(px(5.))
                            .py(px(3.))
                            .rounded(px(5.))
                            .text_size(px(12.))
                            .whitespace_nowrap()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ink)
                            .bg(tint)
                            .opacity(if copied { 1. } else { reveal })
                            .cursor_pointer()
                            .child(icon.render(12., ink))
                            .child(label)
                            .on_click(copy)
                    }),
                )
        })
    }

    fn engine(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let vox = self.vox.read(cx);
        let chosen = vox.settings.avenue;
        let width = self.width;
        // Inside a panel: its padding and border off the column.
        let inner = width - 2. * 20. - 2.;

        let panel = |avenue: Avenue, title: &'static str, about: &'static str, body: Div| {
            let active = chosen == avenue;
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .p(px(20.))
                .rounded(px(14.))
                .bg(t.surface)
                .border_1()
                .border_color(if active { t.ink } else { t.line })
                .shadow(t.lift())
                .child(
                    div()
                        .flex()
                        .items_start()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(
                                    div().flex().items_center().gap(px(8.))
                                        .text_size(px(16.)).font_weight(FontWeight::SEMIBOLD)
                                        .child(avenue_icon(avenue).render(17., t.ink))
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .w(px(inner - 110.))
                                        .text_size(px(13.))
                                        .line_height(relative(1.5))
                                        .text_color(t.soft)
                                        .child(about),
                                ),
                        )
                        .child(if avenue == Avenue::Cloud && vox.groq == GroqKey::Missing {
                            self.key_button(t, "Paste key", true, !active, cx).into_any_element()
                        } else if avenue == Avenue::Cloud && matches!(vox.groq, GroqKey::Rejected(_)) {
                            self.key_button(t, "Replace key", true, false, cx).into_any_element()
                        } else if avenue == Avenue::Cloud && vox.groq == GroqKey::Loading {
                            div().into_any_element()
                        } else if active {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .pt(px(3.))
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.accent)
                                .child(Icon::Check.render(13., t.accent))
                                .child("In use")
                                .into_any_element()
                        } else {
                            button(
                                t,
                                if avenue == Avenue::Local { "pick-local" } else { "pick-cloud" },
                                "Use this",
                                false,
                                click(self.act(move |vox, cx| vox.set(|s| s.avenue = avenue, cx))),
                            )
                            .into_any_element()
                        }),
                )
                .child(body)
        };
        let local = &vox.local;
        let selected = vox.settings.local_model;
        let mut local_body = div()
            .flex()
            .flex_col()
            .gap(px(11.))
            .py(px(12.))
            .border_y_1()
            .border_color(t.line)
            .child(self.transcribe_row(t, cx, inner))
            // Align with the Transcribe dot, which sits before its chevron.
            .child(step(t, "Clean up", "S1-mini by Superwhisper", Icon::Superwhisper, status_dot(t, local.can_tidy())).pr(px(32.)));

        let whisper_selected = selected.family() == Family::Whisper;
        let missing_whisper = whisper_selected && local.whisper_cli.is_none();
        let missing_llama = local.llama_server.is_none();
        let helper_hint = match (missing_whisper, missing_llama) {
            (true, true) => Some("Install whisper-cpp and llama.cpp with Homebrew"),
            (true, false) => Some("Install whisper-cpp with Homebrew"),
            (false, true) => Some("Install llama.cpp with Homebrew"),
            (false, false) => None,
        };
        if !local.can_tidy() || (whisper_selected && vox.active_model().is_none()) {
            local_body = local_body.child(
                div().flex().items_center().justify_between()
                    .children(helper_hint.map(|hint| small(t, inner - 102., hint.into())))
                    .child(button(t, "look-again", "Recheck", false, click(self.act(|vox, cx| vox.look_again(cx))))),
            );
        }

        let saved = matches!(vox.groq, GroqKey::Saved(_));
        let cloud_status = || {
            div()
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .child(if saved { Icon::Check.render(14., t.good) } else { status_dot(t, false) })
        };
        let now = now_ms();
        let transcribe_usage = saved
            .then(|| vox.groq_usage.row_for(Role::Transcription, now))
            .flatten();
        let cleanup_usage = saved
            .then(|| vox.groq_usage.row_for(Role::Cleanup, now))
            .flatten();
        let mut cloud_steps = div()
            .flex()
            .flex_col()
            .gap(px(11.))
            .py(px(12.))
            .border_y_1()
            .border_color(t.line)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .child(step(t, "Transcribe", "OpenAI Whisper · Large v3 Turbo", Icon::OpenAi, cloud_status()))
                    .children(transcribe_usage.as_ref().map(|row| groq_usage_bar(t, inner, row, now))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .child(step(t, "Clean up", "Qwen 3.8 27B", Icon::Qwen, cloud_status()))
                    .children(cleanup_usage.as_ref().map(|row| groq_usage_bar(t, inner, row, now))),
            );
        if saved && transcribe_usage.is_none() && cleanup_usage.is_none() {
            cloud_steps = cloud_steps.child(small(
                t,
                inner,
                "Usage appears here after your first Groq dictation.".into(),
            ));
        }
        let remove = || button(t, "remove-key", "Remove", false, click(self.act(Vox::remove_key)));
        let cloud_body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(cloud_steps)
            .child(match &vox.groq {
                GroqKey::Loading => small(t, inner, "Checking Keychain…".into()),
                GroqKey::Missing => small(t, inner, "Copy a key from console.groq.com/keys".into()),
                GroqKey::Saved(masked) => div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().text_size(px(13.)).text_color(t.soft).child(format!("Groq key {masked}")))
                    .child(self.key_button(t, "Replace key", false, false, cx))
                    .children((!vox.checking_groq_key).then(remove)),
                GroqKey::Rejected(masked) => div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(inner - 86.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(format!("Key {masked} rejected")))
                            .child(small(t, inner - 86., "It may have expired or been revoked. Copy a new key from console.groq.com/keys.".into())),
                    )
                    .children((!vox.checking_groq_key).then(remove)),
            });

        let (tidy, fallback) = (vox.settings.tidy, vox.settings.fallback);
        column()
            .child(heading(t, "Engine", ""))
            .children(self.model_downloads(t, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(panel(
                        Avenue::Local,
                        "On this Mac",
                        "Runs offline on your Mac.",
                        local_body,
                    ))
                    .child(panel(
                        Avenue::Cloud,
                        "Groq",
                        "Sends audio to Groq.",
                        cloud_body,
                    )),
            )
            .child(group(t, vec![
                row(
                    t,
                    width,
                    "Clean up transcripts",
                    "Removes fillers and fixes punctuation.",
                    TOGGLE,
                    toggle(t, "tidy", tidy, click(self.act(move |vox, cx| vox.set(|s| s.tidy = !tidy, cx)))),
                ),
                row(
                    t,
                    width,
                    "Automatic fallback",
                    "Switches between this Mac and Groq if a step fails.",
                    TOGGLE,
                    toggle(t, "fallback", fallback, click(self.act(move |vox, cx| vox.set(|s| s.fallback = !fallback, cx)))),
                ),
            ]))
    }

    fn stats(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let vox = self.vox.read(cx);
        let s = &vox.stats;
        let width = self.width;
        let now = crate::config::now();

        let focused = self.selected;
        let focused_day = focused.and_then(|at| s.day(at));
        let (wpm, words, dictations, spoken_secs) = match focused {
            Some(_) => focused_day.map_or((None, 0, 0, 0.), |d| (d.wpm(), d.words, d.dictations, d.secs)),
            None => (s.wpm(), s.words(), s.dictations(), s.spoken_secs()),
        };
        let saved_min = (words as f32 / TYPING_WPM - spoken_secs / 60.).max(0.);
        let streak = s.streak(now);
        let gap = 12.;
        let tile_w = (width - 3. * gap) / 4.;
        let tile = |icon: Icon, icon_color: gpui::Hsla, label: &'static str, value: String, unit: &'static str, about: Div| {
            div()
                .w(px(tile_w))
                .flex()
                .flex_col()
                .gap(px(4.))
                .px(px(16.))
                .py(px(14.))
                .rounded(px(12.))
                .bg(t.surface)
                .border_1()
                .border_color(t.line)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.soft)
                        .child(icon.render(13., icon_color))
                        .child(label),
                )
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(5.))
                        .child(div().font_family(SERIF).text_size(px(28.)).line_height(relative(1.15)).child(value))
                        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.soft).child(unit)),
                )
                .child(about)
        };
        let about = || {
            div()
                .flex()
                .items_baseline()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(12.))
                .text_color(t.faint)
        };
        let (saved_value, saved_unit) = if saved_min >= 90. {
            (format!("{:.1}", saved_min / 60.), "h")
        } else {
            (format!("{}", saved_min.round() as usize), "min")
        };
        let flame_color = if streak > 0 { t.warn } else { t.soft };
        let tiles = div()
            .flex()
            .gap(px(gap))
            .child(tile(
                Icon::Zap,
                t.soft,
                "Pace",
                wpm.map_or("–".into(), |w| format!("{}", w.round() as usize)),
                "WPM",
                match wpm {
                    Some(w) => about()
                        .child(strong(t, format!("{:.1}×", w / TYPING_WPM)))
                        .child(" typing"),
                    None => about().child("While speaking"),
                },
            ))
            .child(tile(
                Icon::MessageSquare,
                t.soft,
                "Words",
                grouped(words),
                "",
                match dictations {
                    1 => about().child(strong(t, "1")).child(" dictation"),
                    n => about().child(strong(t, grouped(n))).child(" dictations"),
                },
            ))
            .child(tile(
                Icon::Clock,
                t.soft,
                "Time saved",
                saved_value,
                saved_unit,
                about()
                    .child("vs ")
                    .child(strong(t, format!("{TYPING_WPM} WPM"))),
            ))
            .child(tile(
                Icon::Flame,
                flame_color,
                "Streak",
                streak.to_string(),
                if streak == 1 { "day" } else { "days" },
                about().child("In a row"),
            ));

        let switch = |view: Activity, id: &'static str, label: &'static str, icon: Icon| {
            let active = self.activity == view;
            let theme = *t;
            let select = cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.activity = view;
                this.hovered = None;
                cx.notify();
            });
            hover_transition(id, true, move |hover, press| {
                let ink = hover_color(theme.soft, theme.ink, if active { 1. } else { hover });
                let fill = if active {
                    hover_color(theme.surface, theme.line, 0.3 * hover)
                } else {
                    theme.surface.opacity(0.55 * hover)
                };
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(7.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ink)
                    .bg(fill)
                    .when(active, |d| d.shadow(theme.lift()))
                    .opacity(1. - 0.18 * press)
                    .cursor_pointer()
                    .child(icon.render(13., ink))
                    .child(label)
                    .on_click(select)
            })
        };
        let activity_head = div()
            .flex()
            .items_center()
            .justify_between()
            .mb(px(-14.))
            .child(
                div().flex().items_center().gap(px(6.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD)
                    .child(Icon::BarChart.render(14., t.soft))
                    .child("Activity"),
            )
            .child(
                div()
                    .flex()
                    .p(px(3.))
                    .rounded(px(9.))
                    .bg(t.sunken)
                    .child(switch(Activity::Chart, "view-chart", "Chart", Icon::BarChart))
                    .child(switch(Activity::Heatmap, "view-heatmap", "Heatmap", Icon::Grid)),
            );

        // Inside the activity card: its padding and border off the column.
        let inner = width - 2. * 18. - 2.;
        let (graph, span) = match self.activity {
            Activity::Chart => self.chart(t, inner, now, cx),
            Activity::Heatmap => self.heatmap(t, inner, now, cx),
        };
        let caption_row = div()
            .flex()
            .items_baseline()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(13.))
            .text_color(t.soft);
        let caption_day = self.hovered.or(focused);
        let caption = match caption_day.map(|at| (at, s.day(at))) {
            Some((at, day)) => {
                let (words, dictations) = day.map_or((0, 0), |d| (d.words, d.dictations));
                caption_row
                    .child(div().text_color(t.ink).child(long_date(at)))
                    .child(div().text_color(t.faint).child(" · "))
                    .child(strong(t, grouped(words)))
                    .child(if words == 1 { " word" } else { " words" })
                    .child(div().text_color(t.faint).child(" · "))
                    .child(strong(t, grouped(dictations)))
                    .child(if dictations == 1 { " dictation" } else { " dictations" })
            }
            None => {
                let total: usize = (0..span)
                    .filter_map(|i| s.day(local_noon(now).saturating_sub(i * DAY)))
                    .map(|d| d.words)
                    .sum();
                let caption_row = caption_row
                    .child(strong(t, grouped(total)))
                    .child(if total == 1 { " word" } else { " words" });
                match self.activity {
                    Activity::Chart => caption_row.child(format!(" in the last {span} days")),
                    Activity::Heatmap => caption_row.child(format!(
                        " since {}",
                        short_date(local_noon(now).saturating_sub((span - 1) * DAY)),
                    )),
                }
            }
        };
        let clear_button = |id: &'static str, label: String| {
            let theme = *t;
            let clear = cx.listener(|this, _: &ClickEvent, _, cx| {
                this.selected = None;
                cx.notify();
            });
            hover_transition(id, true, move |hover, press| {
                let ink = hover_color(theme.soft, theme.ink, hover);
                let fill = hover_color(theme.sunken, theme.line, 0.55 * hover);
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .flex_shrink_0()
                    .px(px(9.))
                    .py(px(3.))
                    .rounded(px(7.))
                    .bg(fill)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ink)
                    .opacity(1. - 0.18 * press)
                    .cursor_pointer()
                    .child(label.clone())
                    .child(Icon::Close.render(11., ink))
                    .on_click(clear)
            })
        };
        let activity = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(18.))
            .rounded(px(12.))
            .bg(t.surface)
            .border_1()
            .border_color(t.line)
            .child(caption)
            .child(graph);

        let apps = match focused {
            Some(_) => focused_day.map_or_else(Vec::new, |d| d.apps()),
            None => s.apps(),
        };
        let top = apps.first().map_or(1, |(_, w)| *w).max(1);
        let app_total: usize = apps.iter().map(|(_, w)| w).sum::<usize>().max(1);
        const NAME: f32 = 150.;
        const COUNT: f32 = 96.;
        const SHARE: f32 = 40.;
        let bar_w = width - 2. - 2. * 18. - NAME - COUNT - SHARE - 3. * 14.;
        let rows: Vec<Div> = if apps.is_empty() {
            let empty = match focused {
                Some(at) if words == 0 => format!("Nothing dictated on {}.", long_date(at)),
                Some(at) => format!("No app breakdown recorded for {}.", long_date(at)),
                None => "Where you dictate shows up here, starting with your next dictation.".into(),
            };
            vec![div()
                .px(px(18.))
                .py(px(14.))
                .text_size(px(13.))
                .text_color(t.soft)
                .child(empty)]
        } else {
            apps.iter()
                .take(8)
                .map(|(app, words)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .px(px(18.))
                        .py(px(11.))
                        .child(
                            div()
                                .w(px(NAME))
                                .flex()
                                .items_center()
                                .gap(px(9.))
                                .child(app_badge(t, app))
                                .child(div().w(px(NAME - 31.)).truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(app.clone())),
                        )
                        .child(
                            div()
                                .w(px(bar_w))
                                .h(px(6.))
                                .rounded_full()
                                .bg(t.sunken)
                                .child(
                                    div()
                                        .h_full()
                                        .w(px((bar_w * *words as f32 / top as f32).max(6.)))
                                        .rounded_full()
                                        .bg(t.accent),
                                ),
                        )
                        .child(
                            div()
                                .w(px(COUNT))
                                .flex()
                                .items_baseline()
                                .justify_end()
                                .whitespace_nowrap()
                                .text_size(px(12.))
                                .text_color(t.soft)
                                .child(strong(t, grouped(*words)))
                                .child(if *words == 1 { " word" } else { " words" }),
                        )
                        .child(
                            div()
                                .w(px(SHARE))
                                .flex()
                                .justify_end()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("{}%", (100. * *words as f32 / app_total as f32).round() as usize)),
                        )
                })
                .collect()
        };

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(heading(t, "Stats", ""))
            .children(focused.map(|at| clear_button("clear-day-header", long_date(at))));
        let apps_head = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .mb(px(-14.))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(Icon::Layers.render(14., t.soft))
            .child("Apps")
            .children(focused.map(|at| {
                div()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(t.soft)
                    .child(format!("· {}", short_date(at)))
            }));

        column()
            .child(header)
            .child(tiles)
            .child(activity_head)
            .child(activity)
            .child(apps_head)
            .child(group(t, rows))
    }

    /// Words a day over the last month, as bars. Returns the days shown.
    fn chart(&self, t: &Theme, inner: f32, now: u64, cx: &Context<Self>) -> (Div, u64) {
        const DAYS: u64 = 30;
        const HEIGHT: f32 = 150.;
        let s = &self.vox.read(cx).stats;
        let today = local_noon(now);
        let days: Vec<(u64, usize)> = (0..DAYS)
            .rev()
            .map(|i| {
                let at = today.saturating_sub(i * DAY);
                (at, s.day(at).map_or(0, |d| d.words))
            })
            .collect();
        let max = days.iter().map(|(_, w)| *w).max().unwrap_or(0).max(1);
        let gap = 4.;
        let bar_w = (inner - gap * (DAYS - 1) as f32) / DAYS as f32;
        let mut bars = div().flex().items_end().gap(px(gap)).h(px(HEIGHT)).border_b_1().border_color(t.line);
        let theme = *t;
        let selected_key = self.selected.map(stats::date_key);
        let any_selected = selected_key.is_some();
        let accent_hover = accent_deep(t, 0.25);
        let accent_selected = accent_deep(t, 0.35);
        for (i, &(at, words)) in days.iter().enumerate() {
            let h = if words == 0 { 2. } else { (HEIGHT - 4.) * words as f32 / max as f32 }.max(2.);
            let id = ("bar", i);
            let is_selected = selected_key.as_deref() == Some(stats::date_key(at).as_str());
            let select = self.select_day(at, cx);
            bars = bars.child(
                hover_transition(id, true, move |hover, press| {
                    let color = if is_selected {
                        if words == 0 {
                            theme.accent.opacity(0.65)
                        } else {
                            accent_selected
                        }
                    } else if words == 0 {
                        hover_color(theme.line, theme.faint, 0.45 * hover)
                    } else if any_selected {
                        hover_color(theme.accent.opacity(0.32), theme.accent, hover)
                    } else {
                        hover_color(theme.accent, accent_hover, hover)
                    };
                    div()
                        .id(id)
                        .w(px(bar_w))
                        .h_full()
                        .flex()
                        .flex_col()
                        .justify_end()
                        .cursor_pointer()
                        .opacity(1. - 0.14 * press)
                        .child(div().w_full().h(px(h)).rounded_t(px(3.)).bg(color))
                        .on_click(select)
                })
                .on_hover(self.hover(at, cx)),
            );
        }
        let label = |at: u64| div().text_size(px(11.)).text_color(t.faint).child(short_date(at));
        let axis = div()
            .flex()
            .justify_between()
            .child(label(days[0].0))
            .child(label(days[DAYS as usize / 2].0))
            .child(div().text_size(px(11.)).text_color(t.faint).child("Today"));
        (div().flex().flex_col().gap(px(6.)).child(bars).child(axis), DAYS)
    }

    /// Words a day as a grid of weeks, darker for busier days. Returns the
    /// days shown.
    fn heatmap(&self, t: &Theme, inner: f32, now: u64, cx: &Context<Self>) -> (Div, u64) {
        const GAP: f32 = 3.;
        const LABELS: f32 = 28.;
        let s = &self.vox.read(cx).stats;
        let today = local_noon(now);
        let weekday = stats::local(today).tm_wday as u64;
        let room = inner - LABELS;
        let weeks = ((room / (12. + GAP)).floor() as u64).clamp(8, 53);
        let cell = (room / weeks as f32 - GAP).floor();
        // Days back from today, for column `week` and row `day` (Sunday first).
        let back = |week: u64, day: u64| ((weeks - 1 - week) * 7 + weekday).checked_sub(day);
        let span = (weeks - 1) * 7 + weekday + 1;
        let max = (0..span)
            .filter_map(|i| s.day(today.saturating_sub(i * DAY)))
            .map(|d| d.words)
            .max()
            .unwrap_or(0)
            .max(1);
        let shade = |words: usize| -> gpui::Hsla {
            if words == 0 {
                return t.sunken;
            }
            let level = ((4. * words as f32 / max as f32).ceil() as usize).clamp(1, 4);
            t.accent.opacity([0.3, 0.5, 0.75, 1.][level - 1])
        };

        let mut months = div().flex().gap(px(GAP)).pl(px(LABELS));
        let mut last_month = None;
        for week in 0..weeks {
            // The month this column starts, named where it changes.
            let first = back(week, 0).map_or(today, |b| today.saturating_sub(b * DAY));
            let tm = stats::local(first);
            let name = (last_month != Some(tm.tm_mon) && (week > 0 || tm.tm_mday <= 7))
                .then(|| MONTHS[tm.tm_mon as usize % 12][..3].to_string());
            last_month = Some(tm.tm_mon);
            months = months.child(
                div()
                    .w(px(cell))
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(11.))
                    .text_color(t.faint)
                    .child(name.unwrap_or_default()),
            );
        }

        let mut grid = div().flex().gap(px(GAP));
        grid = grid.child(
            div().w(px(LABELS - GAP)).flex().flex_col().gap(px(GAP)).children(
                ["", "Mon", "", "Wed", "", "Fri", ""].into_iter().map(|day| {
                    div().h(px(cell)).flex().items_center().text_size(px(10.)).text_color(t.faint).child(day)
                }),
            ),
        );
        let theme = *t;
        let selected_key = self.selected.map(stats::date_key);
        let any_selected = selected_key.is_some();
        let accent_hover = accent_deep(t, 0.25);
        let accent_selected = accent_deep(t, 0.35);
        for week in 0..weeks {
            let mut column = div().flex().flex_col().gap(px(GAP));
            for day in 0..7 {
                let square = div().size(px(cell)).rounded(px(2.5));
                let Some(b) = back(week, day) else {
                    column = column.child(square);
                    continue;
                };
                let at = today.saturating_sub(b * DAY);
                let words = s.day(at).map_or(0, |d| d.words);
                let fill = shade(words);
                let is_selected = selected_key.as_deref() == Some(stats::date_key(at).as_str());
                let select = self.select_day(at, cx);
                let id = ("day", (week * 7 + day) as usize);
                column = column.child(
                    hover_transition(id, true, move |hover, press| {
                        let bg = if is_selected {
                            if words == 0 {
                                theme.sunken
                            } else {
                                hover_color(fill, accent_selected, 0.25)
                            }
                        } else if words == 0 {
                            hover_color(theme.sunken, theme.line, hover)
                        } else {
                            hover_color(fill, accent_hover, 0.35 * hover)
                        };
                        let opacity = if any_selected && !is_selected {
                            0.45 + 0.55 * hover
                        } else {
                            1.
                        };
                        square
                            .id(id)
                            .bg(bg)
                            .cursor_pointer()
                            .opacity(opacity * (1. - 0.14 * press))
                            .when(is_selected, |s| s.border_2().border_color(accent_selected))
                            .when(!is_selected, |s| s.border_1().border_color(accent_hover.opacity(0.75 * hover)))
                            .on_click(select)
                    })
                    .on_hover(self.hover(at, cx)),
                );
            }
            grid = grid.child(column);
        }

        let legend = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(GAP))
            .text_size(px(11.))
            .text_color(t.faint)
            .child(div().pr(px(3.)).child("Less"))
            .children([0, 1, 2, 3, 4].map(|level| {
                let words = if level == 0 { 0 } else { (max * level).div_ceil(4) };
                div().size(px(cell)).rounded(px(2.5)).bg(shade(words))
            }))
            .child(div().pl(px(3.)).child("More"));
        (
            div().flex().flex_col().gap(px(6.)).child(months).child(grid).child(legend),
            span,
        )
    }

    /// Show a day in the caption while the pointer is over it.
    fn hover(&self, at: u64, cx: &Context<Self>) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
        cx.listener(move |this, over: &bool, _, cx| {
            if *over {
                this.hovered = Some(at);
            } else if this.hovered == Some(at) {
                this.hovered = None;
            } else {
                return;
            }
            cx.notify();
        })
    }

    /// Toggle focusing Stats on a single day.
    fn select_day(&self, at: u64, cx: &Context<Self>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _: &ClickEvent, _, cx| {
            let same = this
                .selected
                .is_some_and(|s| stats::date_key(s) == stats::date_key(at));
            this.selected = if same { None } else { Some(at) };
            cx.notify();
        })
    }

    fn close_model_menu(&mut self) {
        self.model_menu = false;
        self.menu_family = None;
        self.smaller_models = None;
    }

    /// The Transcribe step: a trigger showing the chosen model, and under it,
    /// while open, the menu of every model this Mac could run.
    fn transcribe_row(&self, t: &Theme, cx: &Context<Self>, inner: f32) -> Div {
        let vox = self.vox.read(cx);
        let selected = vox.settings.local_model;
        let (family, spec) = (selected.family(), models::spec(selected));
        let open = self.model_menu;
        let bounds = self.trigger_bounds.clone();
        // The label, then the trigger drawn back over its own padding so that
        // the icon and name sit where the Clean up row's do.
        const LABEL: f32 = 74.;
        const OUTSET: f32 = 10.;
        const TRIGGER: f32 = 28.;
        let wide = inner - LABEL;
        let trigger = div()
            .id("model-trigger")
            .relative()
            .flex()
            .items_center()
            .gap(px(10.))
            .size_full()
            .px(px(10.))
            .rounded(px(8.))
            .cursor_pointer()
            .child(family_icon(family).render(16., t.ink))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(family.label())
                    .child(
                        div()
                            .text_color(t.soft)
                            .font_weight(FontWeight::NORMAL)
                            .child(format!(" · {}", spec.variant)),
                    ),
            )
            .child(status_dot(t, vox.local.can_hear(selected)))
            .child(if open { Icon::ChevronUp } else { Icon::ChevronDown }.render(12., t.soft))
            .child(
                canvas(move |at, _, _| bounds.set(at), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                if this.model_menu {
                    this.close_model_menu();
                } else {
                    this.model_menu = true;
                    this.menu_family = None;
                    this.smaller_models = None;
                    window.focus(&this.menu_focus);
                }
                cx.notify();
            }));
        let theme = *t;
        let trigger = hover_transition("model-trigger", true, move |hover, press| {
            let fill = if open {
                hover_color(theme.sunken, theme.line, 0.5 * hover)
            } else {
                theme.sunken.opacity(hover)
            };
            trigger.bg(fill).opacity(1. - 0.18 * press)
        });
        let menu_at = {
            let at = self.trigger_bounds.get();
            point(at.left(), at.bottom() + px(4.))
        };
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(step_label(t, "Transcribe"))
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .w(px(wide))
                    .h(px(TRIGGER))
                    .ml(px(-OUTSET))
                    .child(trigger)
                    .when(open, |d| {
                        d.child(deferred(
                            anchored()
                                .anchor(Corner::TopLeft)
                                .position(menu_at)
                                .child(self.model_menu(t, cx, wide, self.height - f32::from(menu_at.y) - 16.)),
                        ))
                    }),
            )
    }

    /// Top segmented provider tabs, the active provider's models below, and
    /// this Mac's hardware summary pinned along the bottom.
    fn model_menu(&self, t: &Theme, cx: &Context<Self>, width: f32, room: f32) -> Stateful<Div> {
        let vox = self.vox.read(cx);
        let selected = vox.settings.local_model;
        let active_family = self.menu_family.unwrap_or_else(|| selected.family());
        let cohere_supported = cohere::supported();

        const CHROME_HEIGHT: f32 = 82.;
        let max_list_h = (room - CHROME_HEIGHT).clamp(160., 520.);
        let min_list_h = 196.0_f32.min(max_list_h);

        let mut tabs = div()
            .flex()
            .items_center()
            .gap(px(3.))
            .p(px(3.))
            .rounded(px(9.))
            .bg(t.sunken);
        for (i, family) in Family::ALL.into_iter().enumerate() {
            let active = active_family == family;
            let has_selected = selected.family() == family;
            let supported = family != Family::Cohere || cohere_supported;
            let theme = *t;
            let id = ("model-provider", i);
            let pick = cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.menu_family = Some(family);
                cx.notify();
            });
            tabs = tabs.child(hover_transition(id, true, move |hover, press| {
                let ink = if active {
                    theme.ink
                } else {
                    hover_color(theme.soft, theme.ink, hover)
                };
                let fill = if active {
                    hover_color(theme.surface, theme.line, 0.25 * hover)
                } else {
                    theme.surface.opacity(0.55 * hover)
                };
                let opacity = if supported {
                    1.
                } else if active {
                    0.75
                } else {
                    0.5
                };
                div()
                    .id(id)
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .text_size(px(12.5))
                    .font_weight(if active { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                    .text_color(ink)
                    .bg(fill)
                    .when(active, |d| d.shadow(theme.lift()))
                    .opacity(opacity * (1. - 0.16 * press))
                    .cursor_pointer()
                    .child(family_icon(family).render(15., ink))
                    .child(div().truncate().child(family.label()))
                    .children(has_selected.then(|| {
                        div().size(px(5.)).flex_shrink_0().rounded_full().bg(theme.accent)
                    }))
                    .on_click(pick)
            }));
        }
        let header = div().flex_shrink_0().px(px(8.)).pt(px(8.)).pb(px(2.)).child(tabs);

        let mut list = div()
            .id("model-menu-list")
            .min_h(px(min_list_h))
            .max_h(px(max_list_h))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(1.))
            .p(px(6.));

        match active_family {
            Family::Whisper => {
                let whisper = models::fits_here(Family::Whisper);
                let of = |model: LocalModel| {
                    whisper
                        .iter()
                        .find(|(spec, _)| spec.model == model)
                        .copied()
                        .expect("in its family")
                };
                let (main, smaller): (Vec<_>, Vec<_>) = [
                    LocalModel::WhisperLargeV3TurboQ5,
                    LocalModel::WhisperLargeV3Turbo,
                    LocalModel::WhisperLargeV3,
                    LocalModel::WhisperMedium,
                    LocalModel::WhisperSmall,
                    LocalModel::WhisperBase,
                    LocalModel::WhisperTiny,
                ]
                .into_iter()
                .map(of)
                .enumerate()
                .partition(|(i, _)| *i < 3);
                let smaller: Vec<_> = smaller.into_iter().map(|(_, row)| row).collect();
                let in_play = smaller.iter().any(|(spec, fit)| {
                    spec.model == selected
                        || vox.local.downloaded.contains(&spec.model)
                        || *fit == Fit::Recommended
                });
                let show_smaller = self.smaller_models.unwrap_or(in_play);

                for (_, (spec, fit)) in main {
                    list = list.child(self.option_row(t, cx, spec, fit, true));
                }
                let more_label = format!("More models ({})", smaller.len());
                let theme = *t;
                let toggle_more = cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.smaller_models = Some(!show_smaller);
                    cx.notify();
                });
                list = list.child(hover_transition("smaller-models", true, move |hover, press| {
                    let ink = hover_color(theme.soft, theme.ink, hover);
                    div()
                        .id("smaller-models")
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py(px(6.))
                        .mt(px(2.))
                        .rounded(px(7.))
                        .bg(theme.sunken.opacity(0.75 * hover))
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ink)
                        .opacity(1. - 0.18 * press)
                        .cursor_pointer()
                        .child(
                            div()
                                .size(px(14.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    if show_smaller { Icon::ChevronDown } else { Icon::ChevronRight }
                                        .render(11., ink),
                                ),
                        )
                        .child(more_label.clone())
                        .on_click(toggle_more)
                }));
                if show_smaller {
                    for (spec, fit) in smaller {
                        list = list.child(self.option_row(t, cx, spec, fit, true));
                    }
                }
            }
            Family::Cohere => {
                if !cohere_supported {
                    list = list.child(
                        div()
                            .px(px(10.))
                            .pt(px(4.))
                            .pb(px(6.))
                            .text_size(px(11.5))
                            .text_color(t.faint)
                            .child(cohere::REQUIREMENTS),
                    );
                }
                for (spec, fit) in models::fits_here(Family::Cohere) {
                    list = list.child(self.option_row(t, cx, spec, fit, cohere_supported));
                }
            }
        }

        let footer = div()
            .flex_shrink_0()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(t.line)
            .text_size(px(11.5))
            .text_color(t.faint)
            .child(models::machine_line());

        div()
            .id("model-menu")
            .track_focus(&self.menu_focus)
            .occlude()
            .w(px(width))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(12.))
            .bg(t.glass)
            .border_1()
            .border_color(t.glass_edge)
            .shadow(t.glass_shadow())
            .text_size(px(13.))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close_model_menu();
                    cx.notify();
                }
            }))
            .on_mouse_down_out({
                let trigger = self.trigger_bounds.clone();
                cx.listener(move |this, _, window, cx| {
                    // The trigger's own click toggles; closing here as well
                    // would have it reopen the menu.
                    if !trigger.get().contains(&window.mouse_position()) {
                        this.close_model_menu();
                        cx.notify();
                    }
                })
            })
            .child(header)
            .child(list)
            .child(footer)
    }

    /// One variant in the menu. Hovering a downloaded row swaps its ready dot
    /// for a trash button to remove its weights from this Mac.
    fn option_row(
        &self,
        t: &Theme,
        cx: &Context<Self>,
        spec: &Spec,
        fit: Fit,
        enabled: bool,
    ) -> AnyElement {
        let vox = self.vox.read(cx);
        let model = spec.model;
        let chosen = vox.settings.local_model == model;
        let downloaded = vox.local.downloaded.contains(&model);
        let too_large = fit == Fit::TooLarge;
        let note = if too_large {
            "Not enough memory".to_string()
        } else {
            format!("{} · {} memory", spec.about, models::memory_size(spec.ram_bytes))
        };
        let download_size = (!downloaded).then(|| models::download_size(spec.bytes));
        let id = ("model-option", model as usize);
        let delete_id = ("delete-model", model as usize);
        let select = cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.close_model_menu();
            this.vox.update(cx, |vox, cx| vox.choose_model(model, cx));
            cx.notify();
        });
        let delete_vox = self.vox.clone();
        let variant = spec.variant;
        let recommended = fit == Fit::Recommended;
        let theme = *t;

        let build_row = move |hover: f32, press: f32| {
            let side = if downloaded {
                let delete_vox = delete_vox.clone();
                let delete_button = (hover > 0.02).then(|| {
                    hover_transition(delete_id, true, move |btn_hover, btn_press| {
                        let alpha = hover * (1. - 0.18 * btn_press);
                        let icon_ink = hover_color(theme.soft, theme.live, btn_hover).opacity(alpha);
                        div()
                            .id(delete_id)
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .bg(theme.live.opacity(0.14 * btn_hover * alpha))
                            .cursor_pointer()
                            .child(Icon::Trash.render(13., icon_ink))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                delete_vox.update(cx, |vox, cx| vox.delete_model(model, cx));
                            })
                    })
                });
                let ready_dot = (hover < 0.98).then(|| {
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(1. - hover)
                        .child(status_dot(&theme, true))
                });
                div()
                    .relative()
                    .size(px(24.))
                    .mr(px(-4.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(ready_dot)
                    .children(delete_button)
                    .into_any_element()
            } else {
                let side_ink = if enabled {
                    hover_color(theme.faint, theme.ink, hover)
                } else {
                    theme.faint
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(11.5))
                    .text_color(side_ink)
                    .child(Icon::Download.render(12., side_ink))
                    .child(download_size.clone().unwrap_or_default())
                    .into_any_element()
            };

            let fill = if enabled {
                if chosen {
                    hover_color(theme.sunken, theme.line, 0.5 * hover)
                } else {
                    theme.sunken.opacity(hover)
                }
            } else if chosen {
                theme.sunken
            } else {
                gpui::transparent_black()
            };
            let opacity = if !enabled || too_large { 0.5 } else { 1. };

            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(7.))
                .bg(fill)
                .opacity(opacity * if enabled { 1. - 0.18 * press } else { 1. })
                .when(enabled, |d| d.cursor_pointer().on_click(select))
                .child(
                    div()
                        .size(px(14.))
                        .flex_shrink_0()
                        .children(chosen.then(|| Icon::Check.render(14., theme.accent))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(variant)
                                .children(recommended.then(|| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(3.))
                                        .px(px(6.))
                                        .py(px(1.))
                                        .rounded_full()
                                        .bg(theme.notice)
                                        .text_size(px(11.))
                                        .text_color(theme.accent)
                                        .child(Icon::Star.render(9., theme.accent))
                                        .child("Recommended")
                                })),
                        )
                        .child(div().text_size(px(11.5)).text_color(theme.faint).child(note.clone())),
                )
                .child(side)
        };

        if enabled {
            hover_transition(id, true, build_row).into_any_element()
        } else {
            build_row(0., 0.).into_any_element()
        }
    }

    fn model_downloads(&self, t: &Theme, cx: &Context<Self>) -> Option<Div> {
        let vox = self.vox.read(cx);
        let selected = vox.settings.local_model;
        let speech = Model::Speech(selected);
        let failed = models::CATALOGUE
            .iter()
            .map(|spec| Model::Speech(spec.model))
            .chain([Model::S1])
            .filter(|model| matches!(vox.downloads.get(model), Some(DownloadState::Failed(_))));
        let available = [
            (!vox.local.downloaded.contains(&selected)).then_some(speech),
            vox.local.s1_model.is_none().then_some(Model::S1),
        ]
        .into_iter()
        .flatten()
        .filter(|model| !vox.downloads.contains_key(model));
        let mut models = vox.download_queue.iter().copied().chain(failed).chain(available).peekable();
        models.peek()?;

        let fallback = matches!(
            vox.downloads.get(&speech),
            Some(DownloadState::Working(_) | DownloadState::Queued)
        )
        .then(|| vox.active_model().filter(|model| *model != selected))
        .flatten();
        let mut panel = div()
            .flex()
            .flex_col()
            .px(px(14.))
            .py(px(8.))
            .rounded(px(12.))
            .border_1()
            .border_color(t.line)
            .bg(t.surface)
            .child(
                div().py(px(4.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD)
                    .child("Model downloads"),
            );
        panel = panel.children(models.map(|model| self.model_download_row(t, cx, model)));
        if let Some(model) = fallback {
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .border_t_1()
                    .border_color(t.line)
                    .pt(px(8.))
                    .pb(px(3.))
                    .text_size(px(12.))
                    .text_color(t.soft)
                    .child(Icon::Info.render(13., t.soft))
                    .child("Transcribing with")
                    .child(family_icon(model.family()).render(13., t.ink))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.ink)
                            .child(models::spec(model).name()),
                    )
                    .child("for now."),
            );
        }
        Some(panel)
    }

    fn model_download_row(&self, t: &Theme, cx: &Context<Self>, model: Model) -> Stateful<Div> {
        let vox = self.vox.read(cx);
        let status = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap(px(8.))
            .whitespace_nowrap()
            .text_size(px(12.))
            .text_color(t.soft);
        let cancel = || button(
            t, "cancel-model", "Cancel", false,
            click(self.act(move |vox, cx| vox.cancel_download(model, cx))),
        );
        let download = |label| button_with_icon(
            t, "download-model", label, Icon::Download, false,
            click(self.act(move |vox, cx| vox.download_model(model, cx))),
        );
        let status = match vox.downloads.get(&model) {
            Some(DownloadState::Queued) => status.child("Queued").child(cancel()),
            Some(DownloadState::Working(progress)) => {
                let (label, fraction) = download_indicator(*progress);
                status.child(progress_ring(t, fraction)).child(label).child(cancel())
            }
            Some(DownloadState::Cancelling) => status.child("Cancelling…"),
            Some(DownloadState::Failed(error)) => status
                .child(error.trim_end_matches(". Try again.").to_string())
                .child(download("Retry"))
                .child(button(
                    t, "dismiss-download", "Dismiss", false,
                    click(self.act(move |vox, cx| vox.dismiss_download_failure(model, cx))),
                )),
            None => match model {
                Model::Speech(variant) if variant.family() == Family::Cohere && !cohere::supported() => {
                    status.child(cohere::REQUIREMENTS)
                }
                Model::Speech(variant) if variant.family() == Family::Cohere && cohere::installer().is_none() => {
                    status.child("Install uv, then recheck").child(button(
                        t, "recheck-download", "Recheck", false,
                        click(self.act(|vox, cx| vox.look_again(cx))),
                    ))
                }
                Model::Speech(variant) => status
                    .child(models::download_size(models::spec(variant).bytes))
                    .child(download("Download")),
                Model::S1 => status.child("484 MB").child(download("Download")),
            },
        };
        let label = SharedString::from(model.label());
        let icon = match model {
            Model::Speech(variant) => family_icon(variant.family()),
            Model::S1 => Icon::Superwhisper,
        };
        div()
            .id(label.clone())
            .flex()
            .items_center()
            .gap(px(8.))
            .min_h(px(42.))
            .py(px(4.))
            .child(icon.render(14., t.ink))
            .child(div().flex_1().min_w(px(0.)).truncate().text_size(px(13.)).child(label))
            .child(status)
    }

    fn settings(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let vox = self.vox.read(cx);
        let s = &vox.settings;
        let width = self.width;

        let mut shortcuts: Vec<Div> = Shortcut::PRESETS
            .into_iter()
            .enumerate()
            .map(|(i, shortcut)| {
                let chosen = !self.recording_shortcut && s.shortcut == shortcut;
                let theme = *t;
                let id = ("shortcut", i);
                let select = cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.stop_recording_shortcut(cx);
                    this.vox.update(cx, |vox, cx| vox.set_shortcut(shortcut, cx));
                });
                let keys = shortcut.keys();
                div().child(hover_transition(id, true, move |hover, press| {
                    let tint = hover_color(theme.sunken.opacity(hover), theme.line, press);
                    div()
                        .id(id)
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .px(px(18.))
                        .py(px(12.))
                        .bg(tint)
                        .cursor_pointer()
                        .child(
                            div()
                                .size(px(16.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(if chosen { theme.accent } else { theme.faint })
                                .when(chosen, |d| d.child(div().size(px(8.)).rounded_full().bg(theme.accent))),
                        )
                        .child(keycaps(&theme, &keys, 12.))
                        .on_click(select)
                }))
            })
            .collect();

        let custom_shortcut = if !s.shortcut.is_preset() {
            Some(s.shortcut)
        } else {
            s.custom_shortcut
        };
        let custom_row = if self.recording_shortcut {
            let held = self.recorded_modifiers.symbols();
            let cancel = click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.stop_recording_shortcut(cx);
            }));
            let prompt = if !held.is_empty() {
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(keycaps(t, &held, 12.))
                    .child(div().text_size(px(13.)).text_color(t.soft).child("+ key…"))
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.ink)
                            .child("Press a shortcut…"),
                    )
                    .child(div().text_size(px(12.5)).text_color(t.faint).child("Esc to cancel"))
            };
            div().child(
                div()
                    .id("shortcut-custom")
                    .track_focus(&self.recorder_focus)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(14.))
                    .px(px(18.))
                    .py(px(10.))
                    .bg(t.notice.opacity(0.65))
                    .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                        let mods = ShortcutModifiers {
                            control: event.modifiers.control,
                            alt: event.modifiers.alt,
                            shift: event.modifiers.shift,
                            command: event.modifiers.platform,
                        };
                        if this.recorded_modifiers != mods {
                            this.recorded_modifiers = mods;
                            if !mods.is_empty() {
                                this.record_error = None;
                            }
                            cx.notify();
                        }
                    }))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.stop_recording_shortcut(cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .child(
                                div()
                                    .size(px(16.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        div()
                                            .size(px(8.))
                                            .rounded_full()
                                            .bg(t.live)
                                            .with_animation(
                                                "recording-shortcut",
                                                Animation::new(Duration::from_millis(900))
                                                    .repeat()
                                                    .with_easing(pulsating_between(0.35, 1.)),
                                                |dot, delta| dot.opacity(delta),
                                            ),
                                    ),
                            )
                            .child(prompt),
                    )
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(button(t, "cancel-record-shortcut", "Cancel", false, cancel)),
                    ),
            )
        } else {
            let chosen = custom_shortcut.is_some_and(|custom| s.shortcut == custom);
            let theme = *t;
            let focus_handle = self.recorder_focus.clone();
            let start_record = click(cx.listener(|this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.start_recording_shortcut(window, cx);
            }));
            let row_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
                if let Some(custom) = custom_shortcut {
                    if !chosen {
                        this.vox.update(cx, |vox, cx| vox.set_shortcut(custom, cx));
                    } else {
                        this.start_recording_shortcut(window, cx);
                    }
                } else {
                    this.start_recording_shortcut(window, cx);
                }
            });
            let button_label = if custom_shortcut.is_some() { "Change" } else { "Record" };
            let keys = custom_shortcut.map(Shortcut::keys);
            div().child(hover_transition("shortcut-custom", true, move |hover, press| {
                let tint = hover_color(theme.sunken.opacity(hover), theme.line, press);
                let content = if let Some(keys) = &keys {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(keycaps(&theme, keys, 12.))
                        .child(div().text_size(px(12.5)).text_color(theme.faint).child("Custom"))
                } else {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.ink)
                                .child("Custom"),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(theme.faint)
                                .child("Record your own"),
                        )
                };
                div()
                    .id("shortcut-custom")
                    .track_focus(&focus_handle)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(14.))
                    .px(px(18.))
                    .py(px(10.))
                    .bg(tint)
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .child(
                                div()
                                    .size(px(16.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(if chosen { theme.accent } else { theme.faint })
                                    .when(chosen, |d| {
                                        d.child(div().size(px(8.)).rounded_full().bg(theme.accent))
                                    }),
                            )
                            .child(content),
                    )
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(button(&theme, "record-shortcut", button_label, false, start_record)),
                    )
                    .on_click(row_click)
            }))
        };
        shortcuts.push(custom_row);

        let (typing, keep) = (s.type_into_app, s.keep_clipboard);
        let (can_type, mic_permission) = (vox.can_type, vox.mic_permission);
        let (keeping, kept) = (s.keep_recordings, vox.kept);
        let count = vox.history.entries.len();
        let mut page = column()
            .child(heading(t, "Settings", ""))
            .child(section(t, Icon::Key, "Shortcut", "Hold to talk, or tap for hands-free."))
            .child(group(t, shortcuts).overflow_hidden());
        if let Some(why) = self.record_error.as_ref().or(vox.shortcut_notice.as_ref()) {
            page = page.child(notice(t, width, why.clone(), None));
        }
        page.child(section(t, Icon::Clipboard, "Typing", ""))
            .child(group(t, vec![
                row(
                    t,
                    width,
                    "Paste into active app",
                    "When off, copies text to the clipboard.",
                    TOGGLE,
                    toggle(t, "typing", typing, click(self.act(move |vox, cx| vox.set(|s| s.type_into_app = !typing, cx)))),
                ),
                row(
                    t,
                    width,
                    "Restore clipboard",
                    "Restores your clipboard after pasting.",
                    TOGGLE,
                    toggle(t, "keep", keep, click(self.act(move |vox, cx| vox.set(|s| s.keep_clipboard = !keep, cx)))),
                ),
            ]))
            .child(section(t, Icon::Settings, "Permissions", ""))
            .child(group(t, vec![
                row(
                    t,
                    width,
                    "Microphone",
                    match mic_permission {
                        MicPermission::Allowed => "Allowed",
                        MicPermission::Denied => "Blocked in System Settings.",
                        MicPermission::NotDetermined => "macOS asks the first time you dictate.",
                    },
                    ACTION,
                    match mic_permission {
                        MicPermission::Allowed => div().id("mic-ok").child(status_dot(t, true)).into_any_element(),
                        MicPermission::Denied => button(
                            t,
                            "mic-settings",
                            "Allow in Settings",
                            false,
                            click(|_, _, _| paste::open_microphone_settings()),
                        ),
                        MicPermission::NotDetermined => button(
                            t,
                            "mic-settings",
                            "Open Settings",
                            false,
                            click(|_, _, _| paste::open_microphone_settings()),
                        ),
                    },
                ),
                row(
                    t,
                    width,
                    "Accessibility",
                    if can_type {
                        "Allowed"
                    } else {
                        "Enable Vox to paste into other apps."
                    },
                    ACTION,
                    if can_type {
                        div().id("ax-ok").child(status_dot(t, true)).into_any_element()
                    } else {
                        button(t, "ax-settings", "Open Settings", false, click(|_, _, _| paste::open_accessibility_settings()))
                    },
                ),
            ]))
            .child(section(t, Icon::Mic, "Recordings", ""))
            .child(group(t, vec![
                row(
                    t,
                    width,
                    "Save recordings locally",
                    "Keeps audio and transcripts on this Mac.",
                    TOGGLE,
                    toggle(t, "keep-recordings", keeping, click(self.act(move |vox, cx| vox.set(|s| s.keep_recordings = !keeping, cx)))),
                ),
                row(
                    t,
                    width,
                    match kept.clips {
                        0 => "No recordings kept yet".to_string(),
                        1 => "1 recording kept".to_string(),
                        n => format!("{} recordings kept", grouped(n)),
                    },
                    if kept.clips == 0 {
                        "Saved as WAV files.".to_string()
                    } else {
                        format!("{:.1} MB on disk.", kept.bytes as f64 / 1e6)
                    },
                    ACTION,
                    button(t, "show-recordings", "Show in Finder", false, click(|_, _, _| crate::recordings::reveal())),
                ),
            ]))
            .child(section(t, Icon::Layers, "History", ""))
            .child(group(t, vec![row(
                t,
                width,
                "Dictation history",
                if count == 0 { "None yet." } else { "Keeps your latest 300 dictations." },
                ACTION,
                button(t, "clear", "Clear history", false, click(self.act(|vox, cx| vox.clear_history(cx)))),
            )]))
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        self.width = (f32::from(window.viewport_size().width).min(COLUMN) - 2. * GUTTER).max(320.);
        self.height = f32::from(window.viewport_size().height);
        let page = match self.tab {
            Tab::Home => self.home(&t, cx),
            Tab::Stats => self.stats(&t, cx),
            Tab::Engine => self.engine(&t, cx),
            Tab::Settings => self.settings(&t, cx),
        };
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.ink)
            .text_size(px(14.))
            .child(self.top_bar(&t, cx))
            .child(
                div()
                    .id("page")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .child(page),
            )
            .children(self.vox.read(cx).key_notice.clone().map(|message| self.key_toast(&t, message)))
    }
}

/// The page's single column.
fn column() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(26.))
        .w_full()
        .max_w(px(COLUMN))
        .px(px(GUTTER))
        .pt(px(26.))
        .pb(px(56.))
}

fn heading(t: &Theme, title: &'static str, about: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(div().font_family(SERIF).text_size(px(30.)).line_height(relative(1.25)).child(title))
        .when(!about.is_empty(), |d| {
            d.child(
                div()
                    .max_w(px(540.))
                    .text_size(px(14.))
                    .line_height(relative(1.55))
                    .text_color(t.soft)
                    .child(about),
            )
        })
}

fn section(t: &Theme, icon: Icon, title: &'static str, about: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .mb(px(-14.))
        .child(div().flex().items_center().gap(px(6.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(icon.render(14., t.soft)).child(title))
        .when(!about.is_empty(), |d| {
            d.child(div().text_size(px(13.)).text_color(t.soft).child(about))
        })
}

/// A surface holding rows of settings, with a rule between each.
fn group(t: &Theme, rows: Vec<Div>) -> Div {
    let mut group = div()
        .flex()
        .flex_col()
        .rounded(px(12.))
        .bg(t.surface)
        .border_1()
        .border_color(t.line)
        .shadow(t.lift());
    for (i, row) in rows.into_iter().enumerate() {
        group = group.child(row.when(i > 0, |row| row.border_t_1().border_color(t.line)));
    }
    group
}

/// A setting: what it is, what it does, and its control in a slot of
/// `control_width` at the right. `width` is the column's.
fn row(
    t: &Theme,
    width: f32,
    title: impl Into<SharedString>,
    about: impl Into<SharedString>,
    control_width: f32,
    control: impl IntoElement,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(20.))
        .px(px(18.))
        .py(px(13.))
        .child(
            div()
                .w(px(width - 2. * 18. - 2. - 20. - control_width))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(title.into()))
                .child(div().text_size(px(13.)).line_height(relative(1.45)).text_color(t.soft).child(about.into())),
        )
        .child(div().w(px(control_width)).flex().justify_end().child(control))
}

/// One step of an avenue: what does it, and whether it can.
fn step(t: &Theme, does: &'static str, with: &'static str, icon: Icon, status: Div) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(step_label(t, does))
        .child(icon.render(16., t.ink))
        .child(div().flex_1().min_w(px(0.)).text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(with))
        .child(status)
}

fn step_label(t: &Theme, label: &'static str) -> Div {
    div().w(px(74.)).flex_shrink_0().text_size(px(13.)).text_color(t.soft).child(label)
}

fn download_indicator(progress: Progress) -> (String, f32) {
    match progress {
        Progress::Preparing => ("Preparing…".into(), 0.),
        Progress::Downloading(percent) => (format!("{percent}%"), f32::from(percent) / 100.),
        Progress::Verifying => ("Verifying…".into(), 1.),
    }
}

fn progress_ring(t: &Theme, fraction: f32) -> impl IntoElement {
    let (track, accent) = (t.line, t.accent);
    canvas(|_, _, _| {}, move |bounds, _, window, _| {
        let center = bounds.center();
        let radius = px(6.);
        let radii = point(radius, radius);
        let start = point(center.x, center.y - radius);
        for (sweep, color) in [(1., track), (fraction, accent)] {
            if sweep == 0. {
                continue;
            }
            let mut path = PathBuilder::stroke(px(2.));
            path.move_to(start);
            if sweep == 1. {
                let bottom = point(center.x, center.y + radius);
                path.arc_to(radii, px(0.), false, true, bottom);
                path.arc_to(radii, px(0.), false, true, start);
                path.close();
            } else {
                let angle = std::f32::consts::TAU * sweep;
                let end = point(center.x + radius * angle.sin(), center.y - radius * angle.cos());
                path.arc_to(radii, px(0.), sweep > 0.5, true, end);
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
    })
    .size(px(14.))
    .flex_shrink_0()
}

fn groq_usage_bar(t: &Theme, inner: f32, row: &UsageRow, now: u64) -> Div {
    let indent = 84.;
    let bar_width = (inner - indent).max(160.);
    let fill_color = if row.exhausted() {
        t.live
    } else if row.warn {
        t.warn
    } else {
        t.accent
    };
    let count_color = if row.exhausted() {
        t.live
    } else if row.warn {
        t.warn
    } else {
        t.ink
    };
    let fill_width = if row.rate_limited && row.limit == 0 {
        bar_width
    } else if row.used == 0 {
        0.0
    } else {
        (bar_width * row.used_fraction()).max(4.0).min(bar_width)
    };
    let countdown = row.countdown(now);
    let left_text = if row.limit > 0 {
        match row.metric {
            Some(metric) => format!(
                "{} of {} {}",
                grouped_u64(row.used),
                grouped_u64(row.limit),
                metric.unit_label()
            ),
            None => format!("{} of {} used", grouped_u64(row.used), grouped_u64(row.limit)),
        }
    } else {
        match row.metric {
            Some(metric) => format!("Limit reached ({})", metric.unit_label()),
            None => "Rate limit reached".into(),
        }
    };
    let right = if row.rate_limited {
        div()
            .flex()
            .items_baseline()
            .whitespace_nowrap()
            .child(div().font_weight(FontWeight::MEDIUM).text_color(count_color).child("Rate limited"))
            .children(countdown.map(|text| {
                div().text_color(count_color).child(format!(" · {text}"))
            }))
    } else {
        div()
            .flex()
            .items_baseline()
            .whitespace_nowrap()
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(count_color)
                    .child(grouped_u64(row.remaining)),
            )
            .child(div().text_color(t.soft).child(" left"))
            .children(countdown.map(|text| {
                div()
                    .text_color(if row.warn { count_color } else { t.faint })
                    .child(format!(" · {text}"))
            }))
    };

    div()
        .pl(px(indent))
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .w(px(bar_width))
                .h(px(5.))
                .rounded_full()
                .bg(t.sunken)
                .child(div().w(px(fill_width)).h_full().rounded_full().bg(fill_color)),
        )
        .child(
            div()
                .w(px(bar_width))
                .flex()
                .items_baseline()
                .justify_between()
                .gap(px(8.))
                .text_size(px(12.))
                .child(div().text_color(t.soft).whitespace_nowrap().child(left_text))
                .child(right),
        )
}

/// Emphasised value inside a stat line.
fn strong(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(text.into())
}

/// A deeper shade of the accent colour for hovered or selected chart items.
fn accent_deep(t: &Theme, amount: f32) -> gpui::Hsla {
    let target: gpui::Hsla = if t.dark { gpui::rgb(0x4a5ce8).into() } else { gpui::black().into() };
    hover_color(t.accent, target, amount)
}

fn small(t: &Theme, width: f32, text: String) -> Div {
    div()
        .w(px(width))
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(t.faint)
        .child(text)
}

/// Something to act on. `width` is the column's; the action sits in a slot
/// of [`ACTION`] at the right.
fn notice(t: &Theme, width: f32, text: String, action: Option<AnyElement>) -> Div {
    notice_with(t, width, div().truncate().child(text), action)
}

/// A [`notice`] whose message is built by the caller.
fn notice_with(t: &Theme, width: f32, message: Div, action: Option<AnyElement>) -> Div {
    let text_width = width - 32. - 30. - if action.is_some() { ACTION + 12. } else { 0. };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(12.))
        .rounded(px(12.))
        .bg(t.notice)
        .border_1()
        .border_color(t.accent.opacity(0.18))
        .child(Icon::Alert.render(16., t.accent))
        .child(message.w(px(text_width)).text_size(px(13.)))
        .children(action.map(|action| div().w(px(ACTION)).flex().justify_end().child(action)))
}

/// Room for the widest button a row or notice ends with.
const ACTION: f32 = 172.;
const TOGGLE: f32 = 38.;

fn avenue_icon(avenue: Avenue) -> Icon {
    match avenue {
        Avenue::Cloud => Icon::Groq,
        Avenue::Local => Icon::Monitor,
    }
}

fn family_icon(family: Family) -> Icon {
    match family {
        Family::Whisper => Icon::OpenAi,
        Family::Cohere => Icon::Cohere,
    }
}

/// 12345 as "12,345".
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A moment in this Mac's time zone.
struct Day {
    /// Year and day of the year: the same for two moments on one day.
    key: (i32, i32),
    month: usize,
    date: i32,
    hour: i32,
    minute: i32,
}

fn day_of(at: u64) -> Day {
    let tm = stats::local(at);
    Day {
        key: (tm.tm_year, tm.tm_yday),
        month: tm.tm_mon as usize,
        date: tm.tm_mday,
        hour: tm.tm_hour,
        minute: tm.tm_min,
    }
}

impl Day {
    fn heading(&self, today: &Day) -> String {
        if self.key == today.key {
            "Today".into()
        } else if self.key == (today.key.0, today.key.1 - 1) {
            "Yesterday".into()
        } else {
            format!("{} {}", self.date, MONTHS[self.month % 12])
        }
    }
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August",
    "September", "October", "November", "December",
];

/// "26 Nov".
fn short_date(at: u64) -> String {
    let tm = stats::local(at);
    format!("{} {}", tm.tm_mday, &MONTHS[tm.tm_mon as usize % 12][..3])
}

/// "Wednesday, 26 November".
fn long_date(at: u64) -> String {
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let tm = stats::local(at);
    format!("{}, {} {}", DAYS[tm.tm_wday as usize % 7], tm.tm_mday, MONTHS[tm.tm_mon as usize % 12])
}

/// An app's icon, or its initial on a soft square until the icon is kept.
fn app_badge(t: &Theme, app: &str) -> Div {
    if let Some(icon) = crate::apps::icon(app) {
        return div().size(px(22.)).flex_shrink_0().child(gpui::img(icon).size_full());
    }
    let initial = app.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    div()
        .size(px(22.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .bg(t.sunken)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.soft)
        .child(initial)
}
