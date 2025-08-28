mod rescan;
mod utils;
mod widgets;

use anyhow::Result;
use masonry::peniko::color::AlphaColor;
use masonry::properties::types::Length;
use uniscan::{JsonValue, ScriptFilter};
use winit::error::EventLoopError;
use xilem::core::fork;
use xilem::style::{Padding, Style};
use xilem::tokio::sync::mpsc::UnboundedSender;
use xilem::view::{
    FlexExt, button, flex_col, flex_row, label, prose, sized_box, text_input, virtual_scroll,
    worker,
};
use xilem::{EventLoop, WidgetView, WindowOptions, Xilem};

use crate::rescan::ScanSettings;
use crate::widgets::margin;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct App {
    query_raw: String,
    script_filter_raw: String,
    script_filter: ScriptFilter,

    results: Option<(Vec<JsonValue>, usize)>,
    status: Result<()>,

    sender: Option<UnboundedSender<ScanSettings>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            query_raw: "".into(),
            script_filter: ScriptFilter::new("GeoRock"),
            script_filter_raw: "GeoRock".into(),
            results: None,
            status: Result::Ok(()),
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

    fn results(&self) -> &[JsonValue] {
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
                        .background_color(AlphaColor::from_rgb8(43, 69, 86))
                        .padding(4.),
                    Padding::bottom(8.),
                )
                .boxed()
            },
        );

        fork(
            flex_col((
                search,
                label(format!("{}", self.query_raw)),
                label(match self.status {
                    Err(ref e) => format!("{:?}", e),
                    Ok(()) => "".into(),
                }),
                label(match self.results {
                    Some((_, count)) => format!("Found {} results", count),
                    None => "".into(),
                }),
                sized_box(content).expand_height().flex(1.0),
                flex_row((
                    button("increment", |_: &mut App| {}),
                    button("dec", |_: &mut App| {}),
                )),
            ))
            .padding(8.),
            worker(
                rescan::worker,
                |state: &mut App, sender| {
                    state.sender = Some(sender);
                    state.reload();
                },
                |state, res: Result<rescan::Answer>| match res {
                    Ok(res) => {
                        state.status = Ok(());
                        state.results = Some(res);
                    }
                    Err(e) => {
                        state.status = Err(e);
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
