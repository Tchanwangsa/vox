//! Bundled Feather controls and model marks, available offline in the app.
use std::borrow::Cow;

use gpui::{AssetSource, Div, Hsla, SharedString, Styled as _, div, prelude::*, px, rgb, svg};

#[derive(Clone, Copy)]
pub enum Icon {
    Home,
    Layers,
    Settings,
    Monitor,
    Check,
    Close,
    Alert,
    Info,
    Copy,
    Mic,
    Key,
    Clipboard,
    Download,
    Trash,
    ChevronDown,
    ChevronUp,
    ChevronRight,
    Star,
    BarChart,
    Grid,
    Zap,
    MessageSquare,
    Clock,
    Flame,
    Groq,
    OpenAi,
    Qwen,
    Cohere,
    Superwhisper,
}

impl Icon {
    pub fn render(self, size: f32, ink: Hsla) -> Div {
        let (path, color) = match self {
            Self::Home => ("home.svg", ink),
            Self::Layers => ("layers.svg", ink),
            Self::Settings => ("settings.svg", ink),
            Self::Monitor => ("monitor.svg", ink),
            Self::Check => ("check.svg", ink),
            Self::Close => ("x.svg", ink),
            Self::Alert => ("alert-circle.svg", ink),
            Self::Info => ("info.svg", ink),
            Self::Copy => ("copy.svg", ink),
            Self::Mic => ("mic.svg", ink),
            Self::Key => ("key.svg", ink),
            Self::Clipboard => ("clipboard.svg", ink),
            Self::Download => ("download.svg", ink),
            Self::Trash => ("trash-2.svg", ink),
            Self::ChevronDown => ("chevron-down.svg", ink),
            Self::ChevronUp => ("chevron-up.svg", ink),
            Self::ChevronRight => ("chevron-right.svg", ink),
            Self::Star => ("star.svg", ink),
            Self::BarChart => ("bar-chart-2.svg", ink),
            Self::Grid => ("grid.svg", ink),
            Self::Zap => ("zap.svg", ink),
            Self::MessageSquare => ("message-square.svg", ink),
            Self::Clock => ("clock.svg", ink),
            Self::Flame => ("flame.svg", ink),
            Self::Groq => ("groq.svg", rgb(0xf55036).into()),
            Self::OpenAi => ("openai.svg", ink),
            Self::Qwen => ("qwen.svg", rgb(0x7563df).into()),
            Self::Cohere => ("cohere.svg", ink),
            Self::Superwhisper => ("superwhisper.svg", ink),
        };
        div()
            .size(px(size))
            .flex_shrink_0()
            .child(svg().path(path).size_full().text_color(color))
    }
}

macro_rules! assets {
    ($($path:literal),+ $(,)?) => {
        const FILES: &[(&str, &[u8])] = &[
            $(($path, include_bytes!(concat!("../../assets/icons/", $path)))),+
        ];
    };
}

assets![
    "home.svg",
    "layers.svg",
    "settings.svg",
    "monitor.svg",
    "check.svg",
    "x.svg",
    "alert-circle.svg",
    "info.svg",
    "copy.svg",
    "mic.svg",
    "key.svg",
    "clipboard.svg",
    "download.svg",
    "trash-2.svg",
    "chevron-down.svg",
    "chevron-up.svg",
    "chevron-right.svg",
    "star.svg",
    "bar-chart-2.svg",
    "grid.svg",
    "zap.svg",
    "message-square.svg",
    "clock.svg",
    "flame.svg",
    "groq.svg",
    "openai.svg",
    "qwen.svg",
    "cohere.svg",
    "superwhisper.svg",
];

pub struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, prefix: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, _)| (*name).into())
            .collect())
    }
}
