use std::path::PathBuf;

mod screens;

use gpui::{Size, *};
use gpui_component::label::Label;
use gpui_component::*;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

use crate::screens::select::SelectScreen;

pub struct GlobalState {
    screen: Screen,
}
impl Global for GlobalState {}

#[derive(Debug)]
enum Screen {
    Select,
    Game,
}

pub struct Main {
    select: Entity<SelectScreen>,
}
impl Main {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<GlobalState>(|_, cx| cx.notify())
            .detach();

        let select = cx.new(|cx| SelectScreen::new(window, cx));

        Main { select }
    }
}

impl Render for Main {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let global = cx.global::<GlobalState>();

        match global.screen {
            Screen::Select => self.select.clone().into_any_element(),
            Screen::Game => div().child(Label::new("test")).into_any_element(),
        }
    }
}

fn init(cx: &mut App) {
    let theme_name = "Default Dark";
    if let Err(err) = ThemeRegistry::watch_dir(
        PathBuf::from("/home/jakob/dev/rust/contrib/gpui-component/themes"),
        cx,
        move |cx| {
            let Some(theme) = ThemeRegistry::global(cx).themes().get(theme_name).cloned() else {
                tracing::error!("Theme {} doesn't exist", theme_name);
                return;
            };
            Theme::global_mut(cx).apply_config(&theme);
        },
    ) {
        tracing::error!("Failed to watch themes directory: {}", err);
    }

    cx.set_global(GlobalState {
        screen: Screen::Select,
    });
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let app = Application::new();

    app.run(move |cx| {
        gpui_component::init(cx);
        init(cx);

        let bounds = Bounds::centered(None, Size::new(600u32.into(), 400u32.into()), cx);

        cx.spawn(async move |cx| -> Result<_> {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| Main::new(window, cx));
                    cx.new(|cx| Root::new(view.into(), window, cx))
                },
            )?;

            Ok(())
        })
        .detach_and_log_err(cx);
    });
}
