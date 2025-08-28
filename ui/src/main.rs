use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use masonry::peniko::color::AlphaColor;
use masonry::properties::types::Length;
use tracing::debug;
use uniscan::{ScriptFilter, UniScan};
use winit::error::EventLoopError;
use xilem::core::fork;
use xilem::style::{Padding, Style};
use xilem::tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use xilem::view::{
    FlexExt, button, flex, flex_row, label, portal, prose, sized_box, text_input, virtual_scroll,
    worker,
};
use xilem::{EventLoop, WidgetView, WindowOptions, Xilem, tokio};

struct App {
    query_raw: String,
    script_filter_raw: String,
    script_filter: ScriptFilter,

    results: Option<Vec<serde_json::Value>>,
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

    pub fn reload(&self) {
        let query = match self.query_raw.as_str() {
            "" => ".".into(),
            other => other.to_owned(),
        };

        let _ = self.sender.as_ref().unwrap().send(ScanSettings {
            query,
            script: self.script_filter.clone(),
        });
    }
}

fn app_logic(data: &mut App) -> impl WidgetView<App> + use<> {
    fork(
        flex((
            flex_row((
                text_input(data.query_raw.clone(), App::set_query)
                    .placeholder(".m_GameObject | deref | .m_Name")
                    .flex(1.),
                sized_box(text_input(
                    data.script_filter_raw.clone(),
                    App::set_script_filter,
                ))
                .width(Length::px(180.)),
            )),
            label(format!("{}", data.query_raw)),
            label(match data.status {
                Err(ref e) => format!("{:?}", e),
                Ok(()) => "".into(),
            }),
            label(match data.results.as_deref() {
                Some(results) => format!("Found {} results", results.len()),
                None => "".into(),
            }),
            sized_box(portal(virtual_scroll(
                0..data.results.as_deref().unwrap_or_default().len() as i64,
                |data: &mut App, index| {
                    let results = data.results.as_deref().unwrap_or_default();
                    let Some(value) = results.get(index as usize) else {
                        return flex(()).boxed();
                    };

                    let val = serde_json::to_string_pretty(&value).unwrap();

                    return sized_box(
                        sized_box(prose(val))
                            .background_color(AlphaColor::from_rgb8(43, 69, 86))
                            .padding(4.),
                    )
                    .padding(Padding::bottom(8.))
                    .boxed();
                },
            )))
            .expand_height()
            .flex(1.0),
            flex_row((
                button("increment", |_: &mut App| {}),
                button("dec", |_: &mut App| {}),
            )),
        ))
        .padding(8.),
        worker(
            |proxy, mut rx: UnboundedReceiver<ScanSettings>| async move {
                let path = "/home/jakob/.local/share/Steam/steamapps/common/Hollow Knight/hollow_knight_Data";
                let uniscan = Arc::new(Mutex::new(UniScan::new(&Path::new(path), ".").unwrap()));

                let mut buffer = Vec::new();
                loop {
                    rx.recv_many(&mut buffer, usize::MAX).await;
                    let Some(scan) = buffer.pop() else {
                        break;
                    };
                    buffer.clear();

                    let uniscan = Arc::clone(&uniscan);

                    let result = tokio::task::spawn_blocking(move || {
                        let mut uniscan = uniscan.lock().unwrap();
                        uniscan.query.set_query(&scan.query)?;
                        time("rescan", || uniscan.scan_all(&scan.script))
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))
                    .flatten();
                    if proxy.message(result).is_err() {
                        eprintln!("Could not send rescan result to UI");
                    }
                }
            },
            |state: &mut App, sender| {
                state.sender = Some(sender);
                state.reload();
            },
            |state, res: Result<Vec<serde_json::Value>>| match res {
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

struct ScanSettings {
    query: String,
    script: ScriptFilter,
}

fn main() -> Result<(), EventLoopError> {
    let app = App::default();

    let app = Xilem::new_simple(app, app_logic, WindowOptions::new("Counter app"));
    app.run_in(EventLoop::with_user_event())?;
    Ok(())
}

const MIN_LOG_DURATION: std::time::Duration = std::time::Duration::from_millis(1);
fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let res = f();
    if start.elapsed() > MIN_LOG_DURATION {
        debug!("{name}: {:?}", start.elapsed());
    }
    res
}
