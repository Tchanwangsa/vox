//! The look shared by both windows: colours, the two typefaces, and the few
//! controls the app needs.

pub mod icons;
pub mod main_view;
pub mod pill;

use gpui::{
    div, prelude::*, px, rgb, App, BoxShadow, ClickEvent, Div, FontWeight, Hsla, SharedString,
    Stateful, Window, WindowAppearance,
};

/// Transcripts and headlines: what was said reads as prose.
pub const SERIF: &str = "Iowan Old Style";

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Hsla,
    pub surface: Hsla,
    pub sunken: Hsla,
    pub glass: Hsla,
    pub glass_edge: Hsla,
    pub solid_fg: Hsla,
    pub ink: Hsla,
    pub soft: Hsla,
    pub faint: Hsla,
    pub line: Hsla,
    pub accent: Hsla,
    pub notice: Hsla,
    pub live: Hsla,
    pub warn: Hsla,
    pub good: Hsla,
    pub dark: bool,
}

impl Theme {
    pub fn of(window: &Window) -> Self {
        let hex = |value: u32| -> Hsla { rgb(value).into() };
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self {
                bg: hex(0x14161d).opacity(0.92),
                surface: hex(0x1d202b).opacity(0.90),
                sunken: hex(0x0d0f15).opacity(0.80),
                glass: hex(0x1e2230),
                glass_edge: gpui::white().opacity(0.14),
                solid_fg: hex(0x14161d),
                ink: hex(0xeceef4),
                soft: hex(0xa7aebd),
                faint: hex(0x767e91),
                line: gpui::white().opacity(0.10),
                accent: hex(0x8c9bff),
                notice: hex(0x2d2740).opacity(0.90),
                live: hex(0xff6b57),
                warn: hex(0xf5b041),
                good: hex(0x4cc79a),
                dark: true,
            },
            _ => Self {
                bg: hex(0xf4f5f8).opacity(0.96),
                surface: gpui::white().opacity(0.96),
                sunken: hex(0xe7e9f0).opacity(0.90),
                glass: gpui::white(),
                glass_edge: hex(0x171b26).opacity(0.10),
                solid_fg: gpui::white(),
                ink: hex(0x171b26),
                soft: hex(0x555d6e),
                faint: hex(0x838b9b),
                line: hex(0x171b26).opacity(0.09),
                accent: hex(0x2e46e8),
                notice: hex(0xeceafb),
                live: hex(0xe8452f),
                warn: hex(0xc97a00),
                good: hex(0x17906a),
                dark: false,
            },
        }
    }

    /// The soft lift under raised surfaces.
    pub fn lift(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: gpui::black().opacity(if self.dark { 0.35 } else { 0.08 }),
            offset: gpui::point(px(0.), px(1.)),
            blur_radius: px(4.),
            spread_radius: px(0.),
        }]
    }

    /// Layered elevation and outer hairline rim for translucent glass popovers.
    pub fn glass_shadow(&self) -> Vec<BoxShadow> {
        vec![
            BoxShadow {
                color: gpui::black().opacity(if self.dark { 0.52 } else { 0.14 }),
                offset: gpui::point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
            },
            BoxShadow {
                color: gpui::black().opacity(if self.dark { 0.28 } else { 0.06 }),
                offset: gpui::point(px(0.), px(2.)),
                blur_radius: px(8.),
                spread_radius: px(0.),
            },
            BoxShadow {
                color: gpui::black().opacity(if self.dark { 0.38 } else { 0.07 }),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(1.),
            },
        ]
    }
}

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type Hover = Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

pub fn click(f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Click {
    Box::new(f)
}

/// One key of a shortcut.
pub fn keycap(t: &Theme, label: &str, size: f32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(size * 1.55))
        .min_w(px(size * 1.55))
        .px(px(size * 0.42))
        .rounded(px(size * 0.36))
        .bg(t.surface)
        .border_1()
        .border_b_2()
        .border_color(t.line)
        .text_size(px(size))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.ink)
        .child(SharedString::from(label.to_string()))
}

pub fn keycaps(t: &Theme, keys: &[&str], size: f32) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(size * 0.28))
        .children(keys.iter().map(|key| keycap(t, key, size)))
}

const HOVER_DURATION: f32 = 0.15;

struct HoverProgress {
    from: [f32; 2],
    hovered: bool,
    pressed: bool,
    changed: std::time::Instant,
}

impl HoverProgress {
    fn sample(&self, now: std::time::Instant) -> [f32; 2] {
        let elapsed = now.duration_since(self.changed).as_secs_f32();
        let target = [self.hovered as u8 as f32, self.pressed as u8 as f32];
        if elapsed >= HOVER_DURATION {
            return target;
        }
        let progress = gpui::ease_out_quint()(elapsed / HOVER_DURATION);
        std::array::from_fn(|i| self.from[i] + (target[i] - self.from[i]) * progress)
    }

    fn update(&mut self, hovered: bool, pressed: bool) -> bool {
        if (self.hovered, self.pressed) == (hovered, pressed) {
            return false;
        }
        let now = std::time::Instant::now();
        self.from = self.sample(now);
        self.changed = now;
        self.hovered = hovered;
        self.pressed = pressed;
        true
    }
}

#[derive(IntoElement)]
pub(super) struct HoverTransition<F: FnOnce(f32, f32) -> Stateful<Div> + 'static> {
    id: gpui::ElementId,
    press: bool,
    build: F,
    on_hover: Option<Hover>,
}

pub(super) fn hover_transition<F: FnOnce(f32, f32) -> Stateful<Div> + 'static>(
    id: impl Into<gpui::ElementId>,
    press: bool,
    build: F,
) -> HoverTransition<F> {
    HoverTransition { id: id.into(), press, build, on_hover: None }
}

impl<F: FnOnce(f32, f32) -> Stateful<Div> + 'static> HoverTransition<F> {
    pub(super) fn on_hover(mut self, listener: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Box::new(listener));
        self
    }
}

impl<F: FnOnce(f32, f32) -> Stateful<Div> + 'static> RenderOnce for HoverTransition<F> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = std::time::Instant::now();
        let state = window.use_keyed_state(self.id, cx, |_, _| HoverProgress {
            from: [0.; 2],
            hovered: false,
            pressed: false,
            changed: now,
        });
        let progress = state.read(cx);
        let [hover, press] = progress.sample(now);
        if [hover, press] != [progress.hovered as u8 as f32, progress.pressed as u8 as f32] {
            window.request_animation_frame();
        }

        let hover_state = state.clone();
        let on_hover = self.on_hover;
        (self.build)(hover, press)
            .on_hover(move |hovered, window, cx| {
                hover_state.update(cx, |state, cx| {
                    if state.update(*hovered, state.pressed) {
                        cx.notify();
                    }
                });
                if let Some(listener) = &on_hover {
                    listener(hovered, window, cx);
                }
            })
            .when(self.press, |element| {
                let press_state = state.clone();
                let outside_state = state.clone();
                let move_state = state.clone();
                element
                    .on_mouse_move(move |_, _, cx| {
                        if !cx.has_active_drag() {
                            move_state.update(cx, |state, cx| {
                                if !state.pressed && state.update(true, false) {
                                    cx.notify();
                                }
                            });
                        }
                    })
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        press_state.update(cx, |state, cx| {
                            if state.update(state.hovered, true) {
                                cx.notify();
                            }
                        });
                    })
                    .on_mouse_up_out(gpui::MouseButton::Left, move |_, _, cx| {
                        outside_state.update(cx, |state, cx| {
                            if state.update(false, false) {
                                cx.notify();
                            }
                        });
                    })
                    .on_mouse_up(gpui::MouseButton::Left, move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            if state.update(state.hovered, false) {
                                cx.notify();
                            }
                        });
                    })
            })
    }
}

pub(super) fn hover_color(from: Hsla, to: Hsla, progress: f32) -> Hsla {
    if progress == 0. {
        return from;
    }
    if progress == 1. {
        return to;
    }
    let (from, to) = (gpui::Rgba::from(from), gpui::Rgba::from(to));
    let mix = |a, b| a + (b - a) * progress;
    gpui::Rgba {
        r: mix(from.r, to.r),
        g: mix(from.g, to.g),
        b: mix(from.b, to.b),
        a: mix(from.a, to.a),
    }
    .into()
}

/// A button. `solid` is the one action a surface is asking for.
pub fn button(t: &Theme, id: &'static str, label: &str, solid: bool, on_click: Click) -> gpui::AnyElement {
    action_button(t, id, label, None, solid, Some(on_click))
}

pub fn button_with_icon(
    t: &Theme, id: &'static str, label: &str, icon: icons::Icon, solid: bool, on_click: Click,
) -> gpui::AnyElement {
    action_button(t, id, label, Some(icon), solid, Some(on_click))
}

pub fn action_button(
    t: &Theme, id: &'static str, label: &str, icon: Option<icons::Icon>, solid: bool,
    on_click: Option<Click>,
) -> gpui::AnyElement {
    let theme = *t;
    let (bg, fg, border) = if solid {
        (t.ink, t.solid_fg, t.ink)
    } else {
        (t.surface, t.ink, t.line)
    };
    let button = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .flex_shrink_0()
        .px(px(12.))
        .py(px(5.))
        .rounded(px(8.))
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .children(icon.map(|icon| icon.render(13., fg)))
        .child(SharedString::from(label.to_string()));
    match on_click {
        Some(on_click) => hover_transition(id, true, move |hover, press| {
            let button = if solid {
                let opacity = 1. - 0.18 * hover;
                button.opacity(opacity + (0.65 - opacity) * press)
            } else {
                let hovered = hover_color(bg, theme.sunken, hover);
                button.bg(hover_color(hovered, theme.line, press))
            };
            button.cursor_pointer().on_click(on_click)
        }).into_any_element(),
        None => button.opacity(0.65).into_any_element(),
    }
}

pub fn toggle(t: &Theme, id: &'static str, on: bool, on_click: Click) -> impl IntoElement {
    let track = if on { t.accent } else { t.sunken };
    let border = if on { t.accent } else { t.line };
    let ink = t.ink;
    let knob = div().size(px(16.)).rounded_full().bg(gpui::white()).shadow(t.lift());
    let toggle = div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .w(px(38.))
        .h(px(22.))
        .p(px(2.))
        .rounded_full()
        .border_1()
        .when(on, |d| d.justify_end())
        .cursor_pointer()
        .child(knob)
        .on_click(on_click);
    hover_transition(id, true, move |hover, press| {
        let tint = 0.12 * hover + 0.16 * press;
        toggle.bg(hover_color(track, ink, tint)).border_color(hover_color(border, ink, tint))
    })
}

/// A filled dot when ready, a ring when not.
pub fn status_dot(t: &Theme, ready: bool) -> Div {
    div()
        .size(px(8.))
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(if ready { t.good } else { t.faint })
        .when(ready, |d| d.bg(t.good))
}
