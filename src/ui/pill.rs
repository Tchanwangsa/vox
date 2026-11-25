//! The pill: a small capsule near the bottom of the screen that shows a
//! dictation as it happens. It is a non-activating panel, so the app being
//! typed into keeps the keyboard, and it exists only while there is
//! something to show.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use gpui::{
    canvas, div, ease_out_quint, point, prelude::*, pulsating_between, px, rgb, size, Animation,
    AnimationExt as _, App, Bounds, BoxShadow, ClickEvent, Context, Div, Entity, FontWeight,
    Global, Hsla, Pixels, SharedString, Subscription, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use super::{hover_transition, icons::Icon};
use crate::audio::BARS;
use crate::config::Shortcut;
use crate::engines::groq_usage::{grouped_u64, now_ms, Role, UsageRow};
use crate::engines::{LOADING_SPEECH, LOADING_TIDY, TIDYING};
use crate::state::{vox, Phase, PillAction, Vox};

/// Compact waveform bar count for the hands-free pill (leaves room for × and ✓).
const HANDS_FREE_BARS: usize = 14;

const WIDTH: f32 = 540.;
const HEIGHT: f32 = 196.;
/// How far the pill sits above the bottom of the screen, clear of the Dock.
const ABOVE_BOTTOM: f32 = 70.;

struct Open(Option<WindowHandle<Pill>>);

impl Global for Open {}

fn open(cx: &App) -> Option<WindowHandle<Pill>> {
    cx.try_global::<Open>().and_then(|open| open.0)
}

pub fn show(cx: &mut App) {
    if open(cx).is_some() {
        return;
    }
    let size = size(px(WIDTH), px(HEIGHT));
    let screen = cx.primary_display().map(|d| d.bounds()).unwrap_or_default();
    let origin = point(
        screen.origin.x + (screen.size.width - size.width) / 2.,
        screen.origin.y + screen.size.height - size.height - px(ABOVE_BOTTOM),
    );
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin, size })),
                titlebar: None,
                focus: false,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                float_over(window);
                cx.new(Pill::new)
            },
        )
        .ok();
    cx.set_global(Open(handle));
}

/// GPUI has no switch for any of these. GPUI makes the panel titled, and
/// macOS draws a hairline along a titled window's top edge, which would
/// float above the pill: so the panel is made borderless and shadowless.
/// Ignoring the mouse lets clicks through to whatever the pill's
/// transparent surround covers.
#[allow(unexpected_cfgs)] // objc's `sel_impl!` checks a cfg this crate never sets
fn float_over(window: &Window) {
    use objc::runtime::{Object, NO, YES};
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// `NSWindowStyleMaskNonactivatingPanel`, with no title bar beside it.
    const NONACTIVATING_PANEL: usize = 1 << 7;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRect([f64; 4]);
    unsafe impl objc::Encode for NSRect {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
        }
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    if let RawWindowHandle::AppKit(appkit) = handle.as_raw() {
        let view = appkit.ns_view.as_ptr() as *mut Object;
        unsafe {
            let panel: *mut Object = msg_send![view, window];
            let frame: NSRect = msg_send![panel, frame];
            let _: () = msg_send![panel, setStyleMask: NONACTIVATING_PANEL];
            let _: () = msg_send![panel, setFrame: frame display: YES];
            let _: () = msg_send![panel, setHasShadow: NO];
            let _: () = msg_send![panel, setIgnoresMouseEvents: YES];
        }
    }
}

/// Toggle whether the floating panel passes clicks through or accepts them
/// (only when an actionable button like "Allow in Settings" is visible).
#[allow(unexpected_cfgs)] // objc's `sel_impl!` checks a cfg this crate never sets
fn set_interactive(window: &Window, interactive: bool) {
    use objc::runtime::{Object, NO, YES};
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    if let RawWindowHandle::AppKit(appkit) = handle.as_raw() {
        let view = appkit.ns_view.as_ptr() as *mut Object;
        unsafe {
            let panel: *mut Object = msg_send![view, window];
            let ignore = if interactive { NO } else { YES };
            let _: () = msg_send![panel, setIgnoresMouseEvents: ignore];
        }
    }
}

/// Keep a native macOS `NSVisualEffectView` (dark frosted HUD glass) aligned
/// directly beneath the pill capsule or card so whatever is behind the overlay
/// blurs cleanly through its rounded shape without blurring the empty surround.
#[allow(unexpected_cfgs)]
fn sync_glass_backdrop(window: &Window, bounds: Option<(Bounds<Pixels>, f32, f32)>) {
    use objc::runtime::{Object, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRect {
        origin: NSPoint,
        size: NSSize,
    }
    unsafe impl objc::Encode for NSRect {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
        }
    }

    const MATERIAL_HUD_WINDOW: isize = 13;
    const BLENDING_BEHIND_WINDOW: isize = 0;
    const STATE_ACTIVE: isize = 1;
    const WINDOW_BELOW: isize = -1;

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let metal_view = appkit.ns_view.as_ptr() as *mut Object;
    unsafe {
        let panel: *mut Object = msg_send![metal_view, window];
        if panel.is_null() {
            return;
        }
        let content_view: *mut Object = msg_send![panel, contentView];
        if content_view.is_null() {
            return;
        }
        let mut glass: *mut Object = std::ptr::null_mut();
        let subviews: *mut Object = msg_send![content_view, subviews];
        let count: usize = msg_send![subviews, count];
        let effect_cls = class!(NSVisualEffectView);
        for i in 0..count {
            let subview: *mut Object = msg_send![subviews, objectAtIndex: i];
            let is_effect: BOOL = msg_send![subview, isKindOfClass: effect_cls];
            if is_effect == YES {
                glass = subview;
                break;
            }
        }
        if glass.is_null() {
            let Some(_) = bounds else {
                return;
            };
            let zero = NSRect {
                origin: NSPoint { x: 0., y: 0. },
                size: NSSize { width: 0., height: 0. },
            };
            let alloc: *mut Object = msg_send![effect_cls, alloc];
            glass = msg_send![alloc, initWithFrame: zero];
            let _: () = msg_send![glass, setMaterial: MATERIAL_HUD_WINDOW];
            let _: () = msg_send![glass, setBlendingMode: BLENDING_BEHIND_WINDOW];
            let _: () = msg_send![glass, setState: STATE_ACTIVE];
            let name: *mut Object = msg_send![
                class!(NSString),
                stringWithUTF8String: c"NSAppearanceNameVibrantDark".as_ptr()
            ];
            let appearance: *mut Object = msg_send![class!(NSAppearance), appearanceNamed: name];
            if !appearance.is_null() {
                let _: () = msg_send![glass, setAppearance: appearance];
            }
            let _: () = msg_send![glass, setWantsLayer: YES];
            let layer: *mut Object = msg_send![glass, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setMasksToBounds: YES];
            }
            let _: () = msg_send![
                content_view,
                addSubview: glass
                positioned: WINDOW_BELOW
                relativeTo: metal_view
            ];
            let _: () = msg_send![glass, release];
        }

        let _: () = msg_send![class!(CATransaction), begin];
        let _: () = msg_send![class!(CATransaction), setDisableActions: YES];
        match bounds {
            Some((b, radius, alpha)) if f32::from(b.size.width) > 1. && f32::from(b.size.height) > 1. => {
                let content_bounds: NSRect = msg_send![content_view, bounds];
                let flipped: BOOL = msg_send![content_view, isFlipped];
                let x = f64::from(f32::from(b.origin.x));
                let w = f64::from(f32::from(b.size.width));
                let h = f64::from(f32::from(b.size.height));
                let y = if flipped == YES {
                    f64::from(f32::from(b.origin.y))
                } else {
                    content_bounds.size.height - f64::from(f32::from(b.origin.y)) - h
                };
                let frame = NSRect {
                    origin: NSPoint { x, y },
                    size: NSSize { width: w, height: h },
                };
                let _: () = msg_send![glass, setHidden: NO];
                let _: () = msg_send![glass, setFrame: frame];
                let _: () = msg_send![glass, setAlphaValue: f64::from(alpha)];
                let layer: *mut Object = msg_send![glass, layer];
                if !layer.is_null() {
                    let _: () = msg_send![layer, setCornerRadius: f64::from(radius)];
                    let _: () = msg_send![layer, setMasksToBounds: YES];
                }
            }
            _ => {
                let _: () = msg_send![glass, setHidden: YES];
            }
        }
        let _: () = msg_send![class!(CATransaction), commit];
    }
}

pub fn hide(cx: &mut App) {
    if let Some(handle) = open(cx) {
        handle.update(cx, |_, window, _| window.remove_window()).ok();
    }
    cx.set_global(Open(None));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HandsFreeControl {
    Cancel,
    Finish,
}

pub struct Pill {
    vox: Entity<Vox>,
    born: Instant,
    interactive: bool,
    hovered: Option<HandsFreeControl>,
    _watch: Subscription,
}

impl Pill {
    fn new(cx: &mut Context<Self>) -> Self {
        let vox = vox(cx);
        let hovered = match std::env::var("VOX_DEMO").ok().as_deref() {
            Some("hands-free-hover-cancel") => Some(HandsFreeControl::Cancel),
            Some("hands-free-hover-finish") => Some(HandsFreeControl::Finish),
            _ => None,
        };
        Self {
            _watch: cx.observe(&vox, |_, _, cx| cx.notify()),
            vox,
            born: Instant::now(),
            interactive: false,
            hovered,
        }
    }

    fn set_hovered(
        &mut self,
        control: HandsFreeControl,
        is_hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if is_hovered {
            Some(control)
        } else if self.hovered == Some(control) {
            None
        } else {
            self.hovered
        };
        if next != self.hovered {
            self.hovered = next;
            cx.notify();
        }
    }

    fn hands_free_button(
        &self,
        id: &'static str,
        control: HandsFreeControl,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let vox = self.vox.clone();
        let on_hover = cx.listener(move |this, hovered: &bool, _, cx| {
            this.set_hovered(control, *hovered, cx);
        });
        hover_transition(id, true, move |hover, press| {
            let (bg, icon) = match control {
                HandsFreeControl::Cancel => (
                    white(0.16 + 0.10 * hover + 0.08 * press),
                    Icon::Close.render(13., white(0.92)),
                ),
                HandsFreeControl::Finish => (
                    white(0.95 - 0.10 * hover - 0.12 * press),
                    Icon::Check.render(14., rgb(0x10131b).into()),
                ),
            };
            div()
                .id(id)
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .rounded_full()
                .bg(bg)
                .cursor_pointer()
                .child(icon)
                .on_click(move |_: &ClickEvent, _, cx| {
                    vox.update(cx, |vox, cx| match control {
                        HandsFreeControl::Cancel => vox.cancel_dictation(cx),
                        HandsFreeControl::Finish => vox.finish_dictation(cx),
                    });
                })
        })
        .on_hover(on_hover)
    }

    fn action_button(
        &self,
        id: &'static str,
        text: &'static str,
        action: PillAction,
    ) -> impl IntoElement {
        let vox = self.vox.clone();
        hover_transition(id, true, move |hover, press| {
            let alpha = 0.14 + 0.08 * hover;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(5.))
                .flex_shrink_0()
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .bg(white(alpha + (0.28 - alpha) * press))
                .border_1()
                .border_color(white(0.24))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white(0.95))
                .whitespace_nowrap()
                .cursor_pointer()
                .child(Icon::Settings.render(12., white(0.85)))
                .child(text)
                .on_click(move |_: &ClickEvent, _, cx| {
                    vox.update(cx, |vox, cx| vox.run_pill_action(action, cx));
                })
        })
    }
}

fn white(alpha: f32) -> Hsla {
    gpui::white().opacity(alpha)
}

fn label(text: impl Into<SharedString>) -> Div {
    div()
        .max_w(px(400.))
        .truncate()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white(0.92))
        .child(text.into())
}

fn bars(heights: impl Iterator<Item = (f32, f32)>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .h(px(30.))
        .children(heights.map(|(height, alpha)| {
            div().w(px(3.)).h(px(height)).rounded_full().bg(white(alpha))
        }))
}

fn hands_free_tooltip(control: HandsFreeControl, shortcut: Shortcut) -> impl IntoElement {
    let key = |text: SharedString| {
        div()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(white(0.96))
            .child(text)
    };
    let text = |s: &'static str| div().text_color(white(0.72)).child(s);
    let (id, content) = match control {
        HandsFreeControl::Cancel => (
            "tip-cancel",
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(text("Press"))
                .child(key("Esc".into()))
                .child(text("to cancel")),
        ),
        HandsFreeControl::Finish => {
            let keys = shortcut.keys().join(" ");
            (
                "tip-finish",
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(text("Press"))
                    .child(key(keys.into()))
                    .child(text("or"))
                    .child(key("↵".into()))
                    .child(text("to finish")),
            )
        }
    };
    div()
        .px(px(12.))
        .py(px(5.))
        .rounded_full()
        .bg(Hsla::from(rgb(0x10131b)).opacity(0.92))
        .border_1()
        .border_color(white(0.22))
        .text_size(px(12.))
        .whitespace_nowrap()
        .shadow(vec![BoxShadow {
            color: gpui::black().opacity(0.32),
            offset: point(px(0.), px(4.)),
            blur_radius: px(14.),
            spread_radius: px(0.),
        }])
        .child(content)
        .with_animation(
            id,
            Animation::new(Duration::from_millis(110)).with_easing(ease_out_quint()),
            |tip, delta| tip.relative().top(px((1. - delta) * 4.)).opacity(delta),
        )
}

fn groq_card_row(row: &UsageRow, bar_width: f32, now: u64, show_window: bool) -> Div {
    let good: Hsla = rgb(0x5bd6a6).into();
    let warn: Hsla = rgb(0xf5b041).into();
    let live: Hsla = rgb(0xff6b57).into();

    let fill_color = if row.exhausted() {
        live
    } else if row.warn {
        warn
    } else {
        good
    };
    let count_color = if row.exhausted() {
        live
    } else if row.warn {
        warn
    } else {
        white(0.94)
    };
    let fill_width = if row.used == 0 {
        0.0
    } else {
        (bar_width * row.used_fraction()).max(4.0).min(bar_width)
    };
    let role_icon = match row.role {
        Role::Transcription => Icon::Mic,
        Role::Cleanup => Icon::Qwen,
    };
    let left_suffix = if show_window {
        format!(" left {}", row.window.short_label())
    } else {
        " left".into()
    };
    let reset = if row.warn { row.resets_in(now) } else { None };

    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .text_size(px(11.5))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_color(white(0.75))
                        .font_weight(FontWeight::MEDIUM)
                        .child(role_icon.render(12., white(0.65)))
                        .child(row.role.label()),
                )
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .whitespace_nowrap()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(count_color)
                                .child(grouped_u64(row.remaining)),
                        )
                        .child(div().text_color(white(0.6)).child(left_suffix))
                        .children(reset.map(|reset| {
                            div().text_color(count_color).child(format!(" · {reset}"))
                        })),
                ),
        )
        .child(
            div()
                .w(px(bar_width))
                .h(px(5.))
                .rounded_full()
                .bg(white(0.12))
                .child(div().w(px(fill_width)).h_full().rounded_full().bg(fill_color)),
        )
}

impl Render for Pill {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vox = self.vox.read(cx);
        let needs_clicks = matches!(
            &vox.phase,
            Phase::Listening { hands_free: true }
                | Phase::Failed { action: Some(_), .. }
                | Phase::Done { needs_accessibility: true, .. }
                | Phase::Done { groq_key_rejected: true, .. }
        );
        if needs_clicks != self.interactive {
            self.interactive = needs_clicks;
            set_interactive(window, needs_clicks);
        }
        if !matches!(&vox.phase, Phase::Listening { hands_free: true }) {
            self.hovered = None;
        }
        let live: Hsla = rgb(0xff6b57).into();
        let good: Hsla = rgb(0x5bd6a6).into();
        let glass_fill = Hsla::from(rgb(0x10131b)).opacity(0.18);
        let glass_shadow = || {
            vec![
                BoxShadow {
                    color: gpui::black().opacity(0.34),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(28.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: gpui::black().opacity(0.24),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(1.),
                },
            ]
        };
        let glass_alpha = Rc::new(Cell::new(1.0_f32));
        let backdrop = |radius: f32| {
            let alpha = glass_alpha.clone();
            canvas(
                move |bounds, window, _| {
                    sync_glass_backdrop(window, Some((bounds, radius, alpha.get())));
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
            .size_full()
        };
        let capsule = || {
            div()
                .relative()
                .flex()
                .items_center()
                .gap(px(12.))
                .min_h(px(44.))
                .px(px(16.))
                .rounded(px(22.))
                .bg(glass_fill)
                .border_1()
                .border_color(white(0.28))
                .shadow(glass_shadow())
                .child(backdrop(22.))
        };

        let body = match &vox.phase {
            Phase::Idle => {
                sync_glass_backdrop(window, None);
                div()
            }
            Phase::Listening { hands_free: false } => {
                window.request_animation_frame();
                let secs = vox.listening_since.elapsed().as_secs();
                let levels = vox.levels.recent();
                let wave = bars(levels.iter().enumerate().map(|(i, level)| {
                    // Older readings, to the left, fade out.
                    (3. + level * 26., 0.3 + 0.7 * (i as f32 / BARS as f32))
                }));
                capsule()
                    .child(
                        div().size(px(8.)).rounded_full().bg(live).with_animation(
                            "live",
                            Animation::new(Duration::from_millis(1_100))
                                .repeat()
                                .with_easing(pulsating_between(0.35, 1.)),
                            |dot, delta| dot.opacity(delta),
                        ),
                    )
                    .child(wave)
                    .child(
                        div()
                            .min_w(px(30.))
                            .text_size(px(12.))
                            .text_color(white(0.6))
                            .child(format!("{}:{:02}", secs / 60, secs % 60)),
                    )
            }
            Phase::Listening { hands_free: true } => {
                window.request_animation_frame();
                let secs = vox.listening_since.elapsed().as_secs();
                let levels = vox.levels.recent();
                let shortcut = vox.settings.shortcut;
                let hovered = self.hovered;
                let start = levels.len().saturating_sub(HANDS_FREE_BARS);
                let slice = &levels[start..];
                let count = slice.len().max(1) as f32;
                let wave = bars(slice.iter().enumerate().map(|(i, level)| {
                    (3. + level * 22., 0.35 + 0.65 * (i as f32 / count))
                }));
                let cancel_btn =
                    self.hands_free_button("pill-hf-cancel", HandsFreeControl::Cancel, cx);
                let finish_btn =
                    self.hands_free_button("pill-hf-finish", HandsFreeControl::Finish, cx);
                let pill = div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .min_h(px(44.))
                    .px(px(8.))
                    .rounded(px(22.))
                    .bg(glass_fill)
                    .border_1()
                    .border_color(white(0.28))
                    .shadow(glass_shadow())
                    .child(backdrop(22.))
                    .child(cancel_btn)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(
                                div().size(px(6.)).rounded_full().bg(live).with_animation(
                                    "live-hf",
                                    Animation::new(Duration::from_millis(1_100))
                                        .repeat()
                                        .with_easing(pulsating_between(0.35, 1.)),
                                    |dot, delta| dot.opacity(delta),
                                ),
                            )
                            .child(wave)
                            .child(
                                div()
                                    .min_w(px(28.))
                                    .text_size(px(11.5))
                                    .text_color(white(0.58))
                                    .child(format!("{}:{:02}", secs / 60, secs % 60)),
                            ),
                    )
                    .child(finish_btn);
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .children(hovered.map(|control| hands_free_tooltip(control, shortcut)))
                    .child(pill)
            }
            Phase::Working => {
                window.request_animation_frame();
                let t = self.born.elapsed().as_secs_f32();
                let wave = bars((0..12).map(|i| {
                    let swell = 0.5 + 0.5 * (t * 7. - i as f32 * 0.55).sin();
                    (4. + swell * 12., 0.35 + swell * 0.5)
                }));
                let status = match vox.stage.load(Ordering::Relaxed) {
                    LOADING_SPEECH => "Loading speech model",
                    LOADING_TIDY => "Loading cleanup model",
                    TIDYING => "Cleaning up",
                    _ => "Transcribing",
                };
                capsule().child(wave).child(label(status))
            }
            Phase::Done {
                words,
                typed,
                needs_accessibility,
                note,
                groq_key_rejected,
                groq,
            } => {
                let count = if *words == 1 { "1 word".into() } else { format!("{words} words") };
                let as_heard = if note.is_some() { " as heard" } else { "" };
                let (icon, said) = if *typed {
                    (Icon::Check, format!("Typed {count}{as_heard}"))
                } else {
                    (Icon::Clipboard, format!("Copied to clipboard{as_heard} · ⌘V to paste"))
                };
                let rows: Vec<_> = groq.rows().into_iter().filter(|r| r.warn).collect();
                if rows.is_empty() {
                    let mut pill = capsule().child(icon.render(16., good)).child(label(said));
                    if *groq_key_rejected {
                        pill = pill.child(self.action_button(
                            "pill-fix-groq",
                            "Fix Groq key",
                            PillAction::OpenGroqSettings,
                        ));
                    } else if *needs_accessibility {
                        pill = pill.child(self.action_button(
                            "pill-allow-paste",
                            "Allow pasting",
                            PillAction::OpenAccessibilitySettings,
                        ));
                    }
                    pill
                } else {
                    let now = now_ms();
                    let shared = rows
                        .first()
                        .filter(|first| rows.iter().all(|r| r.window == first.window))
                        .map(|first| first.window);
                    let card_width = 296.0;
                    let bar_width = card_width - 28.0;
                    let groq_label = match shared {
                        Some(window) => format!("Groq · {}", window.short_label()),
                        None => "Groq".into(),
                    };
                    let header_text = if *needs_accessibility {
                        format!("Copied{as_heard} · ⌘V to paste")
                    } else {
                        said
                    };
                    let mut card = div()
                        .relative()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .w(px(card_width))
                        .px(px(14.))
                        .py(px(10.))
                        .rounded(px(15.))
                        .bg(glass_fill)
                        .border_1()
                        .border_color(white(0.28))
                        .shadow(glass_shadow())
                        .child(backdrop(15.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.))
                                        .child(icon.render(15., good))
                                        .child(label(header_text)),
                                )
                                .child(if *needs_accessibility {
                                    self.action_button(
                                        "pill-allow-paste-card",
                                        "Allow pasting",
                                        PillAction::OpenAccessibilitySettings,
                                    )
                                    .into_any_element()
                                } else {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .flex_shrink_0()
                                        .text_size(px(11.5))
                                        .text_color(white(0.55))
                                        .child(Icon::Groq.render(13., white(0.7)))
                                        .child(groq_label.clone())
                                        .into_any_element()
                                }),
                        )
                        .child(div().h(px(1.)).w_full().bg(white(0.10)));
                    if *needs_accessibility {
                        card = card.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .text_size(px(11.))
                                .text_color(white(0.5))
                                .child(Icon::Groq.render(12., white(0.65)))
                                .child(groq_label),
                        );
                    }
                    card.child(div().flex().flex_col().gap(px(8.)).children(rows.iter().map(
                        |row| groq_card_row(row, bar_width, now, shared.is_none()),
                    )))
                }
            }
            Phase::Failed { message, action } => {
                let icon = match action {
                    Some(PillAction::OpenMicSettings) => Icon::Mic,
                    _ => Icon::Alert,
                };
                let mut pill = capsule()
                    .child(icon.render(16., live))
                    .child(label(message.clone()).whitespace_nowrap());
                if let Some(action) = *action {
                    let text = match action {
                        PillAction::OpenMicSettings => "Allow in Settings",
                        PillAction::OpenAccessibilitySettings => "Allow pasting",
                        PillAction::OpenGroqSettings => "Open Engine",
                    };
                    pill = pill.child(self.action_button("pill-error-action", text, action));
                }
                pill
            }
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_end()
            .items_center()
            // Room for the shadow, which the window would otherwise crop.
            .pb(px(48.))
            .child(body.with_animation(
                "arrive",
                Animation::new(Duration::from_millis(180)).with_easing(ease_out_quint()),
                move |body, delta| {
                    glass_alpha.set(delta);
                    body.relative().top(px((1. - delta) * 10.)).opacity(delta)
                },
            ))
    }
}
