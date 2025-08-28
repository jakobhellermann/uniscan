mod rescan;
mod utils;
mod widgets;

use anyhow::Result;
use masonry::properties::types::Length;
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

use crate::rescan::ScanSettings;
use crate::widgets::margin;

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

    results: Option<(Vec<serde_json::Value>, usize)>,
    error: Result<()>,

    sender: Option<UnboundedSender<ScanSettings>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            query_raw: "".into(),
            script_filter: ScriptFilter::new("GeoRock"),
            script_filter_raw: "GeoRock".into(),
            results: None,
            error: Result::Ok(()),
            sender: None,
        }
    }
}

impl App {
    fn set_query(&mut self, query: String) {
        self.query_raw = query;
        self.reload();
    }
    fn set_script_filter(&mut self, script_filter: String) {
        self.script_filter_raw = script_filter;
        let new_filter = ScriptFilter::new(&self.script_filter_raw);
        if new_filter != self.script_filter {
            self.script_filter = new_filter;
            self.reload();
        }
    }

    fn reload(&self) {
        let query = match self.query_raw.as_str() {
            "" => ".".into(),
            other => other.to_owned(),
        };

        let _ = self.sender.as_ref().unwrap().send(ScanSettings {
            query,
            script: self.script_filter.clone(),
            limit: 1000,
        });
    }

    fn results(&self) -> &[serde_json::Value] {
        match self.results {
            Some((ref values, _)) => values.as_slice(),
            None => &[],
        }
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
                    let missing = match state.results {
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

        fork(
            flex_col((
                search,
                self.error
                    .as_ref()
                    .err()
                    .map(|e| label(format!("{:?}", e)).color(COLOR_ERROR))
                    .unwrap_or_else(|| label("")),
                self.results
                    .as_ref()
                    .map(|(_, count)| label(format!("Found {} results", count))),
                sized_box(content).expand_height().flex(1.0),
                flex_row((
                    button("Back", |_: &mut App| {}).background_color(BUTTON_COLOR),
                    button("Export", |_: &mut App| {})
                        .background_color(BUTTON_COLOR)
                        .disabled_background(Background::Color(BUTTON_DISABLED_COLOR))
                        .disabled(self.results.as_ref().is_none_or(|x| x.1 == 0)),
                ))
                .main_axis_alignment(MainAxisAlignment::End),
            ))
            .cross_axis_alignment(CrossAxisAlignment::Fill)
            .padding(8.)
            .background_color(BACKGROUND_COLOR),
            worker(
                rescan::worker,
                |state: &mut App, sender| {
                    state.sender = Some(sender);
                    state.reload();
                },
                |state, res: Result<rescan::Answer>| match res {
                    Ok(res) => {
                        state.error = Ok(());
                        state.results = Some(res);
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
