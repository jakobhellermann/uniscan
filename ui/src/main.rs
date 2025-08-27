#![windows_subsystem = "windows"]
mod utils;
use iced::alignment::Vertical;
use iced::widget::{Scrollable, button, column, container, row, text, text_editor, text_input};
use iced::{Element, Length, Task, Theme};
use iced_highlighter::{Highlighter, Settings};
use rfd::FileHandle;
use std::fmt::Debug;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uniscan::UniScan;

type Column<'a> = iced::widget::Column<'a, Message>;

pub fn main() -> iced::Result {
    let init = || {
        initial_path()
            .map(Message::Selected)
            .map(Task::done)
            .unwrap_or_else(Task::none)
    };
    iced::application(
        move || (State::default(), init()),
        State::update,
        State::view,
    )
    .theme(State::theme)
    .title("uniscan")
    .window_size((800., 600.))
    .run()
}

fn initial_path() -> Option<PathBuf> {
    Some(PathBuf::from(
        "/home/jakob/.local/share/Steam/steamapps/common/Hollow Knight/hollow_knight_Data",
    ))
    // None
}

struct Selection {
    uniscan: UniScan,
    results: Vec<(text_editor::Content, serde_json::Value)>,
}

struct State {
    script_filter: String,
    query: String,

    selection: Option<Selection>,

    error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            script_filter: String::new(),
            query: String::new(),
            selection: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    OpenPicker,
    Selected(PathBuf),

    SetQuery(String),
    SetScriptFilter(String),
    SetResults(Vec<serde_json::Value>),
    Error(String),

    Noop,
}

impl State {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenPicker => Task::future(async {
                let file = rfd::AsyncFileDialog::new()
                    .set_title("Open a text file...")
                    .pick_file()
                    .await;

                file.map(FileHandle::into)
            })
            .and_then(|path| Task::done(Message::Selected(path))),
            Message::Selected(path) => match UniScan::new(&path, ".") {
                Ok(uniscan) => {
                    self.selection = Some(Selection {
                        uniscan,
                        results: Vec::new(),
                    });
                    Task::done(Message::SetQuery("".into())) // reload
                }
                Err(e) => Task::done(Message::Error(e.to_string())),
            },
            Message::SetScriptFilter(script_filter) => {
                self.script_filter = script_filter;
                self.reload()
            }
            Message::SetQuery(query) => {
                self.query = query;
                self.reload()
            }
            Message::SetResults(results) => {
                let results = results
                    .into_iter()
                    .map(|value| {
                        let pretty = serde_json::to_string_pretty(&value)
                            .unwrap_or_else(|_| "Failed to serialize".into());
                        let content = text_editor::Content::with_text(&pretty);
                        (content, value)
                    })
                    .collect();
                self.selection.as_mut().unwrap().results = results;
                Task::none()
            }
            Message::Error(error) => {
                self.error = Some(error);
                Task::none()
            }
            Message::Noop => Task::none(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        match &self.selection {
            Some(selection) => {
                let column = selection
                    .results
                    .iter()
                    .map(|(content, _)| {
                        let text = text_editor(content)
                            .highlight_with::<Highlighter>(
                                Settings {
                                    theme: iced_highlighter::Theme::Base16Eighties,
                                    token: "json".into(),
                                },
                                |a, _| a.to_format(),
                            )
                            .on_action(|_| Message::Noop);
                        Element::from(text)
                    })
                    .collect::<Column>()
                    .spacing(8);
                let scrollable = Scrollable::new(column)
                    .width(Length::Fill)
                    .height(Length::Fill);

                let filename = selection.uniscan.env.resolver.game_dir.file_name().unwrap();
                let mut display_filename = filename.display().to_string();
                display_filename.truncate(40);

                let controls = row![container(
                    button(text(format!("{display_filename}..."))).on_press(Message::OpenPicker)
                ),]
                .spacing(8)
                .align_y(Vertical::Center);

                let query_input = text_input("jq query", &self.query).on_input(Message::SetQuery);
                let script_filter_input = text_input("Class Name", &self.script_filter)
                    .width(Length::Fixed(160.))
                    .on_input(Message::SetScriptFilter);

                let results_text = text(match self.script_filter.is_empty() {
                    true => String::new(),
                    false => format!("Found {} items", selection.results.len()),
                });

                Element::from(
                    column![
                        row![query_input, script_filter_input].spacing(4),
                        results_text,
                        scrollable,
                        self.error.as_deref().map(text).unwrap_or_else(|| text("")),
                        controls
                    ]
                    .spacing(12)
                    .padding(12),
                )
            }
            None => Column::new()
                .push(
                    container(button("Open Assetbundle").on_press(Message::OpenPicker))
                        .center(Length::Fill),
                )
                .push_maybe(self.error.as_deref().map(text))
                .into(),
        }
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn reload(&mut self) -> Task<Message> {
        let selection = self
            .selection
            .as_mut()
            .expect("setting query without selection");

        let actual_query = match self.query.is_empty() {
            true => ".",
            false => &self.query,
        };

        match time("Parse query", || {
            selection.uniscan.query.set_query(actual_query)
        }) {
            Ok(()) => set_error(
                time("Scan", || selection.uniscan.scan_all(&self.script_filter)),
                Message::SetResults,
            ),
            Err(_) => Task::none(),
        }
    }
}

fn set_error<T>(result: Result<T, anyhow::Error>, f: impl FnOnce(T) -> Message) -> Task<Message> {
    match result {
        Ok(val) => Task::done(f(val)),
        Err(e) => Task::done(Message::Error(format!("{:?}", e))),
    }
}

const MIN_LOG_DURATION: Duration = Duration::from_millis(1);

fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let res = f();
    if start.elapsed() > MIN_LOG_DURATION {
        eprintln!("{name}: {:?}", start.elapsed());
    }
    res
}
