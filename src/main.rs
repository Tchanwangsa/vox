//! Vox: hold a shortcut, speak, and the words are typed into the app in
//! front. Hearing and tidying run on this Mac or on Groq.

mod apps;
mod audio;
mod config;
mod engines;
mod hotkey;
mod keys;
mod paste;
mod recordings;
mod state;
mod stats;
mod ui;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{
    actions, point, prelude::*, px, size, App, Application, Bounds, Global, KeyBinding, Menu,
    MenuItem, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowOptions,
};

use config::{Avenue, LocalModel, Settings};
use state::{Handle, Vox};
use ui::main_view::MainView;

actions!(vox, [Quit, CloseWindow]);

struct Main(Option<WindowHandle<MainView>>);

impl Global for Main {}

fn open_main(cx: &mut App) {
    let vox = state::vox(cx);
    // `VOX_HEIGHT` opens a development run tall enough to see a whole page.
    let height = std::env::var("VOX_HEIGHT").ok().and_then(|h| h.parse().ok()).unwrap_or(680.);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(880.), px(height)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Vox".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(18.), px(18.))),
        }),
        window_min_size: Some(size(px(700.), px(480.))),
        window_background: WindowBackgroundAppearance::Blurred,
        ..Default::default()
    };
    let handle = cx
        .open_window(options, |_, cx| cx.new(|cx| MainView::new(vox, cx)))
        .ok();
    cx.set_global(Main(handle));
    cx.activate(true);
}

/// Bring the main window forward, opening it again if it was closed.
fn show_main(cx: &mut App) {
    let open = cx.try_global::<Main>().and_then(|main| main.0);
    let shown = open.is_some_and(|handle| {
        handle.update(cx, |_, window, _| window.activate_window()).is_ok()
    });
    if !shown {
        open_main(cx);
    }
}

fn show_engine(cx: &mut App) {
    show_main(cx);
    if let Some(handle) = cx.try_global::<Main>().and_then(|main| main.0) {
        handle.update(cx, |view, _, cx| view.show_engine(cx)).ok();
    }
}

fn run_app() {
    let app = Application::new().with_assets(ui::icons::Icons);
    // Closing the window leaves Vox listening for the shortcut; the Dock
    // icon brings the window back.
    app.on_reopen(show_main);
    app.run(|cx: &mut App| {
        let settings = Settings::load();
        let (hotkeys, events) = match hotkey::Hotkeys::new(settings.shortcut) {
            Ok((hotkeys, events)) => (Ok(hotkeys), Some(events)),
            Err(e) => (Err(e), None),
        };
        let vox = cx.new(|cx| Vox::new(settings, hotkeys, cx));
        cx.set_global(Handle(vox.clone()));

        if let Some(mut events) = events {
            let vox = vox.clone();
            cx.spawn(async move |cx| {
                while let Some(event) = events.next().await {
                    vox.update(cx, |vox, cx| vox.on_hotkey(event, cx)).ok();
                }
            })
            .detach();
        }

        // Accessibility access is granted in System Settings, outside the app.
        cx.spawn(async move |cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            if vox.update(cx, |vox, cx| vox.check_typing(cx)).is_err() {
                break;
            }
        })
        .detach();

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx.active_window() {
                window.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Vox".into(),
            items: vec![
                MenuItem::action("Close Window", CloseWindow),
                MenuItem::separator(),
                MenuItem::action("Quit Vox", Quit),
            ],
        }]);
        cx.on_app_quit(|_| async { engines::s1::stop() }).detach();

        open_main(cx);
        if std::env::var_os("VOX_DEMO").is_some() {
            ui::pill::show(cx);
        }
    });
}

/// A speech variant by its settings name: `whisper`, `cohere`, `whisper_medium`, `cohere_8bit`.
fn variant(name: &str) -> Option<LocalModel> {
    serde_json::from_value(name.into()).ok()
}

/// `vox --hear clip.wav [cloud|local|whisper|cohere|<variant>] [--as-heard]`: run one clip through
/// the pipeline and print the result, without the app.
fn hear_file(args: &[String]) -> Result<(), String> {
    let wav = std::fs::read(&args[0]).map_err(|e| format!("{}: {e}", args[0]))?;
    let mut settings = Settings { fallback: false, ..Settings::default() };
    for arg in &args[1..] {
        match arg.as_str() {
            "cloud" => settings.avenue = Avenue::Cloud,
            "local" => settings.avenue = Avenue::Local,
            "--as-heard" => settings.tidy = false,
            other => {
                settings.avenue = Avenue::Local;
                settings.local_model = variant(other).ok_or(format!("unknown argument {other}"))?;
            }
        }
    }
    let local = engines::Local::discover();
    let started = std::time::Instant::now();
    let stage = Arc::new(AtomicU8::new(engines::HEARING));
    let cancel = AtomicBool::new(false);
    let outcome = engines::run(&settings, &local, &wav, &stage, &cancel, false, None).outcome;
    engines::s1::stop();
    let outcome = outcome?;
    eprintln!("heard by {} on {} in {} ms", outcome.heard_model, outcome.avenue.label(), started.elapsed().as_millis());
    eprintln!("heard: {}", outcome.heard);
    if let Some(note) = outcome.note {
        eprintln!("note:  {note}");
    }
    println!("{}", outcome.text);
    Ok(())
}

fn install_model(name: &str) -> Result<(), String> {
    use engines::download::Model;
    let model = match name {
        "s1" => Model::S1,
        _ => Model::Speech(variant(name).ok_or("Choose s1, whisper, cohere, or a variant such as cohere_8bit")?),
    };
    model.install(&AtomicBool::new(false), |stage| eprintln!("{}", stage.message(model)))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match args.first().map(String::as_str) {
        Some("--hear") if args.len() > 1 => Some(hear_file(&args[1..])),
        Some("--install-cohere") => Some(install_model("cohere")),
        Some("--install-model") if args.len() == 2 => Some(install_model(&args[1])),
        _ => None,
    };
    if let Some(result) = command {
        if let Err(e) = result {
            eprintln!("vox: {e}");
            std::process::exit(1);
        }
        return;
    }
    run_app();
}
