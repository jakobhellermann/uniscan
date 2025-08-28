mod utils;
mod widgets;
mod workers;

use anyhow::Result;
use masonry::properties::types::Length;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use uniscan::ScriptFilter;
use winit::error::EventLoopError;
use xilem::core::fork;
use xilem::style::{Background, Padding, Style};
use xilem::tokio::sync::mpsc::UnboundedSender;
use xilem::view::{
    CrossAxisAlignment, FlexExt, MainAxisAlignment, button, flex_col, flex_row, label, prose,
    sized_box, text_input, virtual_scroll, worker,
};
use xilem::{Color, EventLoop, WidgetView, WindowOptions, Xilem};

use widgets::{NumberInputState, margin, number_input};

use crate::utils::time;
use crate::workers::{generic, rescan};

pub const COLOR_ERROR: Color = Color::from_rgb8(255, 51, 51);
pub const BACKGROUND_COLOR: Color = Color::from_rgb8(18, 18, 20);
pub const HIGHLIGHT_COLOR: Color = Color::from_rgb8(36, 36, 40);
pub const BUTTON_COLOR: Color = Color::from_rgb8(60, 90, 140);
pub const BUTTON_DISABLED_COLOR: Color = Color::from_rgb8(55, 55, 60);

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct App {
    query_raw: String,
    script_filter_raw: String,
    script_filter: ScriptFilter,
    limit: NumberInputState<usize>,

    results_all: Option<(Vec<serde_json::Value>, usize)>,
    results_filtered: Option<Vec<serde_json::Value>>,
    error: Result<()>,

    sender_rescan: Option<UnboundedSender<rescan::Request>>,
    sender_generic: Option<UnboundedSender<generic::Request>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            query_raw: "".into(),
            script_filter: ScriptFilter::new("GeoRock"),
            script_filter_raw: "GeoRock".into(),
            limit: NumberInputState::new(500),
            results_all: None,
            results_filtered: None,
            error: Result::Ok(()),

            sender_rescan: None,
            sender_generic: None,
        }
    }
}

impl App {
    fn set_query(&mut self, query: String) {
        self.query_raw = query;

        self.reload_query();
    }
    fn set_script_filter(&mut self, script_filter: String) {
        self.script_filter_raw = script_filter;
        let new_filter = ScriptFilter::new(&self.script_filter_raw);
        if new_filter != self.script_filter {
            self.script_filter = new_filter;
            self.reload();
        }
    }

    fn real_query(&self) -> String {
        match self.query_raw.as_str() {
            "" => ".".into(),
            other => other.to_owned(),
        }
    }

    fn reload(&self) {
        self.send_rescan_command(rescan::Request {
            query: self.real_query(),
            script: self.script_filter.clone(),
            limit: self.limit.last_valid,
        });
    }

    // TODO async
    fn reload_query(&mut self) {
        match uniscan::query::QueryRunner::new(&self.real_query()) {
            Ok(runner) => {
                self.error = Ok(());
                if let Some((results, _)) = &self.results_all {
                    time("filter", || {
                        let result = results
                            .par_iter()
                            .try_fold(Vec::new, |mut acc, item| -> Result<_> {
                                let mapped = runner.exec_jaq(From::from(item))?;
                                acc.extend(mapped.into_iter().map(serde_json::Value::from));
                                Ok(acc)
                            })
                            .try_reduce(Vec::new, |mut a, b| {
                                a.extend(b);
                                Ok(a)
                            });
                        match result {
                            Ok(data) => {
                                self.results_filtered = Some(data);
                                self.error = Ok(());
                            }
                            Err(err) => {
                                self.error = Err(err);
                            }
                        }
                    });
                }
            }
            Err(e) => {
                self.error = Err(e);
            }
        }
    }

    fn results(&self) -> &[serde_json::Value] {
        self.results_filtered.as_deref().unwrap_or_default()
    }
    fn result_count(&self) -> Option<usize> {
        self.results_all.as_ref().map(|(_, count)| *count)
    }

    pub fn export(&mut self) -> Result<()> {
        let results = self.results();

        let formatted = serde_json::to_string_pretty(&results)?;
        // TODO: tempfile
        let path = "/tmp/out.json";
        std::fs::write(path, &formatted)?;
        opener::open(path)?;

        Ok(())
    }

    fn send_rescan_command(&self, cmd: rescan::Request) {
        let _ = self.sender_rescan.as_ref().unwrap().send(cmd);
    }
    fn send_command(&self, cmd: generic::Request) {
        let _ = self.sender_generic.as_ref().unwrap().send(cmd);
    }

    pub fn save(&mut self) -> Result<()> {
        let results = self.results();
        let formatted = serde_json::to_string_pretty(&results)?;
        self.send_command(generic::Request::Save(formatted));

        Ok(())
    }

    fn ui(&mut self) -> impl WidgetView<App> + use<> {
        let search = flex_row((
            text_input(self.query_raw.clone(), App::set_query)
                .placeholder(".m_GameObject | deref | .m_Name")
                .flex(1.),
            sized_box(text_input(
                self.script_filter_raw.clone(),
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
                    let missing = match state.results_all {
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

        let can_export = self.results_all.as_ref().is_some_and(|x| x.1 != 0);

        fork(
            flex_col((
                search,
                self.error
                    .as_ref()
                    .err()
                    .map(|e| label(format!("{:?}", e)).color(COLOR_ERROR).boxed())
                    .unwrap_or_else(|| label("").boxed()),
                self.result_count()
                    .map(|count| label(format!("Found {} results", count))),
                sized_box(content).expand_height().flex(1.0),
                flex_row((
                    label("Limit:"),
                    sized_box(number_input(
                        self.limit.clone(),
                        |state: &mut App, limit| {
                            let changed = limit.last_valid != state.limit.last_valid;
                            state.limit = limit;
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
            .padding(8.)
            .background_color(BACKGROUND_COLOR),
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
                        state.reload_query();
                    },
                    |state, res: Result<rescan::Response>| match res {
                        Ok(res) => {
                            state.error = Ok(());
                            state.results_all = Some(res);
                            state.reload_query();
                        }
                        Err(e) => {
                            state.error = Err(e);
                        }
                    },
                ),
            ),
        )
    }

    fn set_error<T>(&mut self, result: Result<T>) {
        self.error = result.map(drop);
    }
}

fn main() -> Result<(), EventLoopError> {
    let app = App::default();

    let app = Xilem::new_simple(app, App::ui, WindowOptions::new("uniscan"));
    app.run_in(EventLoop::with_user_event())?;
    Ok(())
}
