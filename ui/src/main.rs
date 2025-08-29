mod utils;
mod widgets;
mod workers;

use anyhow::Result;
use masonry::properties::types::Length;
use uniscan::ScriptFilter;
use winit::error::EventLoopError;
use xilem::core::one_of::OneOf2;
use xilem::core::{NoElement, ViewSequence, fork};
use xilem::style::{Background, Padding, Style};
use xilem::tokio::sync::mpsc::UnboundedSender;
use xilem::view::{
    CrossAxisAlignment, FlexExt, MainAxisAlignment, button, flex_col, flex_row, label, prose,
    sized_box, text_input, virtual_scroll, worker,
};
use xilem::{Color, EventLoop, ViewCtx, WidgetView, WindowOptions, Xilem};

use widgets::{NumberInputState, margin, number_input};

use crate::workers::{generic, rescan};

pub const COLOR_ERROR: Color = Color::from_rgb8(255, 51, 51);
pub const BACKGROUND_COLOR: Color = Color::from_rgb8(18, 18, 20);
pub const HIGHLIGHT_COLOR: Color = Color::from_rgb8(36, 36, 40);
pub const BUTTON_COLOR: Color = Color::from_rgb8(60, 90, 140);
pub const BUTTON_DISABLED_COLOR: Color = Color::from_rgb8(55, 55, 60);

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

enum View {
    GameSelect,
    Main,
}

struct GameSelect {}
struct Main {
    query_raw: String,
    script_filter_raw: String,
    script_filter: ScriptFilter,
    limit: NumberInputState<usize>,
    results: Option<(Vec<serde_json::Value>, usize)>,
}

struct App {
    view: View,

    gameselect: GameSelect,
    main: Main,

    // Shared
    error: Result<()>,
    sender_rescan: Option<UnboundedSender<rescan::Request>>,
    sender_generic: Option<UnboundedSender<generic::Request>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            view: View::GameSelect,

            gameselect: GameSelect {},
            main: Main {
                query_raw: "".into(),
                script_filter: ScriptFilter::new("GeoRock"),
                script_filter_raw: "GeoRock".into(),
                limit: NumberInputState::new(500),
                results: None,
            },

            error: Result::Ok(()),
            sender_rescan: None,
            sender_generic: None,
        }
    }
}

// Shared
impl App {
    fn go_to_main(&mut self) {
        self.view = View::Main;
    }
    fn set_error<T>(&mut self, result: Result<T>) {
        self.error = result.map(drop);
    }
    fn send_rescan_command(&self, cmd: rescan::Request) {
        let _ = self.sender_rescan.as_ref().unwrap().send(cmd);
    }
    fn send_command(&self, cmd: generic::Request) {
        let _ = self.sender_generic.as_ref().unwrap().send(cmd);
    }
}

// View: GameSelect
impl App {}

// View: Main
impl App {
    fn set_query(&mut self, query: String) {
        self.main.query_raw = query;
        self.reload();
    }
    fn set_script_filter(&mut self, script_filter: String) {
        self.main.script_filter_raw = script_filter;
        let new_filter = ScriptFilter::new(&self.main.script_filter_raw);
        if new_filter != self.main.script_filter {
            self.main.script_filter = new_filter;
            self.reload();
        }
    }
    fn reload(&self) {
        let query = match self.main.query_raw.as_str() {
            "" => ".".into(),
            other => other.to_owned(),
        };

        self.send_rescan_command(rescan::Request {
            query,
            script: self.main.script_filter.clone(),
            limit: self.main.limit.last_valid,
        });
    }

    fn results(&self) -> &[serde_json::Value] {
        match self.main.results {
            Some((ref values, _)) => values.as_slice(),
            None => &[],
        }
    }

    fn export(&mut self) -> Result<()> {
        let results = self.results();

        let formatted = serde_json::to_string_pretty(&results)?;
        // TODO: tempfile
        let path = "/tmp/out.json";
        std::fs::write(path, &formatted)?;
        opener::open(path)?;

        Ok(())
    }

    fn save(&mut self) -> Result<()> {
        let results = self.results();
        let formatted = serde_json::to_string_pretty(&results)?;
        self.send_command(generic::Request::Save(formatted));

        Ok(())
    }
}

// UI
impl App {
    fn ui(&mut self) -> impl WidgetView<App> + use<> {
        let content = match self.view {
            View::GameSelect => OneOf2::A(self.ui_gameselect()),
            View::Main => OneOf2::B(self.ui_main()),
        };
        let content = sized_box(content)
            .padding(8.)
            .background_color(BACKGROUND_COLOR);
        fork(content, App::workers())
    }

    fn ui_gameselect(&mut self) -> impl WidgetView<App> + use<> {
        button("next", App::go_to_main)
    }

    fn ui_main(&mut self) -> impl WidgetView<App> + use<> {
        let search = flex_row((
            text_input(self.main.query_raw.clone(), App::set_query)
                .placeholder(".m_GameObject | deref | .m_Name")
                .flex(1.),
            sized_box(text_input(
                self.main.script_filter_raw.clone(),
                App::set_script_filter,
            ))
            .width(Length::px(180.)),
        ));
        let content = virtual_scroll(
            0..self.results().len() as i64 + 1,
            |state: &mut App, index| {
                let index = index as usize;
                let results = state.results();

                if index == results.len() {
                    let missing = match state.main.results {
                        Some((ref data, count)) => count - data.len(),
                        None => 0,
                    };
                    return label(match missing {
                        0 => String::new(),
                        n => format!("... ({n} more)"),
                    })
                    .boxed();
                }

                let Some(value) = results.get(index) else {
                    return flex_col(()).boxed();
                };

                let val = serde_json::to_string_pretty(&value).unwrap();

                margin(
                    sized_box(prose(val))
                        .background_color(HIGHLIGHT_COLOR)
                        .padding(4.),
                    Padding::bottom(8.),
                )
                .boxed()
            },
        );

        let can_export = self.main.results.as_ref().is_some_and(|x| x.1 != 0);

        flex_col((
            search,
            self.error
                .as_ref()
                .err()
                .map(|e| label(format!("{:?}", e)).color(COLOR_ERROR).boxed())
                .unwrap_or_else(|| label("").boxed()),
            self.main
                .results
                .as_ref()
                .map(|(_, count)| label(format!("Found {} results", count))),
            sized_box(content).expand_height().flex(1.0),
            flex_row((
                label("Limit:"),
                sized_box(number_input(
                    self.main.limit.clone(),
                    |state: &mut App, limit| {
                        let changed = limit.last_valid != state.main.limit.last_valid;
                        state.main.limit = limit;
                        if changed {
                            state.reload();
                        }
                    },
                ))
                .width(Length::px(60.)),
                button("Open as JSON", |app: &mut App| app.error = app.export())
                    .disabled(!can_export)
                    .background_color(BUTTON_COLOR)
                    .disabled_background(Background::Color(BUTTON_DISABLED_COLOR)),
                button("Save to file", |app: &mut App| app.error = app.save())
                    .disabled(!can_export)
                    .background_color(BUTTON_COLOR)
                    .disabled_background(Background::Color(BUTTON_DISABLED_COLOR)),
            ))
            .main_axis_alignment(MainAxisAlignment::End),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Fill)
    }

    fn workers() -> impl ViewSequence<App, (), ViewCtx, NoElement> {
        (
            worker(
                workers::generic::worker,
                |state: &mut App, sender| state.sender_generic = Some(sender),
                App::set_error,
            ),
            worker(
                workers::rescan::worker,
                |state: &mut App, sender| {
                    state.sender_rescan = Some(sender);
                    state.reload();
                },
                |state, res: Result<rescan::Response>| match res {
                    Ok(res) => {
                        state.error = Ok(());
                        state.main.results = Some(res);
                    }
                    Err(e) => {
                        state.error = Err(e);
                    }
                },
            ),
        )
    }
}

fn main() -> Result<(), EventLoopError> {
    let app = App::default();

    let app = Xilem::new_simple(app, App::ui, WindowOptions::new("uniscan"));
    app.run_in(EventLoop::with_user_event())?;
    Ok(())
}
