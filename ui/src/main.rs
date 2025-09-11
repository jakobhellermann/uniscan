#![windows_subsystem = "windows"]
mod utils;
mod widgets;
mod workers;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use masonry::properties::types::Length;
use rabex::typetree::NullTypeTreeProvider;
use rabex_env::Environment;
use rabex_env::game_files::GameFiles;
use uniscan::{ScanResults, ScriptFilter, UniScan};
use winit::error::EventLoopError;
use xilem::core::one_of::OneOf2;
use xilem::core::{NoElement, ViewSequence, fork};
use xilem::style::{Background, Padding, Style};
use xilem::tokio::sync::mpsc::UnboundedSender;
use xilem::view::{
    CrossAxisAlignment, FlexExt, MainAxisAlignment, button, flex_col, flex_row, label, portal,
    prose, sized_box, text_input, virtual_scroll, worker, worker_raw,
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

struct UnityGame {
    name: String,
    path: PathBuf,
}
struct SteamGame {
    game: UnityGame,
    app_id: u32,
}

struct GameSelect {
    steam_games: Vec<SteamGame>,
    custom_selection: Option<UnityGame>,

    game_selection: SelectedGame,
}

struct Main {
    query_raw: String,
    script_filter_raw: String,
    script_filter: ScriptFilter,
    limit: NumberInputState<usize>,
    results: Option<ScanResults>,
}

struct App {
    view: View,

    gameselect: GameSelect,
    main: Main,

    // Shared
    error: Result<()>,
    sender_rescan: Option<UnboundedSender<rescan::Request>>,
    sender_generic: Option<UnboundedSender<generic::Request>>,
    uniscan: Arc<Mutex<Option<UniScan>>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            view: View::GameSelect,

            gameselect: GameSelect {
                steam_games: Vec::new(),
                custom_selection: None,
                game_selection: SelectedGame::None,
            },
            main: Main {
                query_raw: "".into(),
                script_filter: ScriptFilter::new(""),
                script_filter_raw: String::new(),
                limit: NumberInputState::new(500),
                results: None,
            },

            error: Result::Ok(()),
            sender_rescan: None,
            sender_generic: None,
            uniscan: Default::default(),
        }
    }
}

enum SelectedGame {
    None,
    Steam(usize),
    Custom,
}

// Shared
impl App {
    fn go_to_main(&mut self, selection: SelectedGame) {
        self.gameselect.game_selection = selection;

        self.view = View::Main;
        self.clear_error();
        self.set_error_with(|app| {
            utils::time("game init", || {
                let uniscan = UniScan::new(&app.selected_game().path, ".")?;
                let env = Arc::clone(&uniscan.env);
                *app.uniscan.lock().unwrap() = Some(uniscan);
                app.send_command(generic::Request::GetStats(env));
                Ok(())
            })
        });
    }
    fn go_to_gameselect(&mut self) {
        self.view = View::GameSelect;

        self.main.results = None;
        self.set_script_filter(String::new());
        self.set_query(String::new());
        self.clear_error();

        self.gameselect.game_selection = SelectedGame::None;

        self.uniscan.clear_poison();
        *self.uniscan.lock().unwrap() = None;
        // TODO: cancel tasks?
    }

    fn clear_error(&mut self) {
        self.error = Ok(());
    }
    fn set_error(&mut self, err: anyhow::Error) {
        self.error = Err(err);
    }
    fn set_error_with<T>(&mut self, f: impl Fn(&mut App) -> Result<T>) {
        let result = f(self);
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
impl App {
    pub fn selected_game(&self) -> &UnityGame {
        match self.gameselect.game_selection {
            SelectedGame::None => unreachable!(),
            SelectedGame::Steam(i) => &self.gameselect.steam_games[i].game,
            SelectedGame::Custom => self.gameselect.custom_selection.as_ref().unwrap(),
        }
    }
    pub fn gameselect_open_custom(&mut self) {
        self.send_command(generic::Request::OpenGame);
    }
}

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

        self.send_rescan_command(rescan::Request::Scan {
            query,
            script: self.main.script_filter.clone(),
            limit: self.main.limit.last_valid,
        });
    }

    fn results(&self) -> &[serde_json::Value] {
        match self.main.results {
            Some(ref scan) => scan.items.as_slice(),
            None => &[],
        }
    }

    fn export(&mut self) -> Result<()> {
        let results = self.results();

        let formatted = serde_json::to_string_pretty(&results)?;
        let game = &self.selected_game().name;
        let dir = std::env::temp_dir().join("uniscan").join(game);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("out.json");
        std::fs::write(&path, &formatted)?;
        opener::open(&path)?;

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
        let content = flex_col(content)
            .padding(8.)
            .background_color(BACKGROUND_COLOR);
        fork(content, App::workers(Arc::clone(&self.uniscan)))
    }

    fn ui_gameselect(&mut self) -> impl WidgetView<App> + use<> {
        let header = label("Select a unity game").text_size(26.);

        flex_col((
            flex_row(header),
            self.error_ui(),
            portal({
                let items = self
                    .gameselect
                    .steam_games
                    .iter()
                    .enumerate()
                    .map(|(i, game)| {
                        flex_row((
                            sized_box(
                                button("Open", move |state: &mut App| {
                                    state.go_to_main(SelectedGame::Steam(i))
                                })
                                .padding(4.),
                            ),
                            sized_box(label(game.app_id.to_string())).width(Length::px(60.)),
                            label(game.game.name.as_str()),
                        ))
                    })
                    .collect::<Vec<_>>();
                let items_empty = items.is_empty();
                sized_box(
                    flex_col((
                        items,
                        items_empty.then(|| label("No steam games detected.")),
                        self.gameselect.custom_selection.as_ref().map(|game| {
                            flex_row((
                                sized_box(
                                    button("Open", move |state: &mut App| {
                                        state.go_to_main(SelectedGame::Custom)
                                    })
                                    .padding(4.),
                                ),
                                label(game.name.as_str()),
                            ))
                        }),
                        button("Open another", App::gameselect_open_custom).padding(4.),
                    ))
                    .cross_axis_alignment(CrossAxisAlignment::Start),
                )
                .expand()
            })
            .flex(1.),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Fill)
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
                        Some(ref scan) => scan.count.saturating_sub(state.main.limit.last_valid),
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

        let can_export = self
            .main
            .results
            .as_ref()
            .is_some_and(|scan| scan.count != 0);

        flex_col((
            search,
            self.error_ui(),
            self.main.results.as_ref().map(|scan| {
                let text = format!("Found {} results ({})", scan.count, scan.query_count);
                label(text)
            }),
            sized_box(content).expand_height().flex(1.0),
            flex_row((
                button("Back", App::go_to_gameselect),
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
                )),
            ))
            .main_axis_alignment(MainAxisAlignment::SpaceBetween),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Fill)
    }

    fn error_ui(&mut self) -> impl WidgetView<App> + use<> {
        self.error
            .as_ref()
            .err()
            .map(|e| {
                prose(format!("{:?}", e))
                    .line_break_mode(masonry::properties::LineBreaking::WordWrap)
                    .text_color(COLOR_ERROR)
                    .boxed()
            })
            .unwrap_or_else(|| label("").boxed())
    }

    fn workers(
        uniscan: Arc<Mutex<Option<UniScan>>>,
    ) -> impl ViewSequence<App, (), ViewCtx, NoElement> {
        // let x = Arc::clone(&self.uniscan);
        (
            worker(
                workers::generic::worker,
                |state: &mut App, sender| state.sender_generic = Some(sender),
                |state: &mut App, res: Result<generic::Response>| match res {
                    Ok(res) => match res {
                        generic::Response::Noop => {}
                        generic::Response::OpenAnotherGame(path) => {
                            let Some(path) = path else {
                                return;
                            };
                            state.set_error_with(|app| {
                                app.gameselect.custom_selection =
                                    Some(unity_game_from_path(&path)?);
                                Ok(())
                            });
                        }
                        generic::Response::Stats(stats) => {
                            if state.main.script_filter_raw.is_empty() {
                                state.set_script_filter(stats.most_used_script);
                            }
                        }
                    },
                    Err(err) => state.set_error(err),
                },
            ),
            worker_raw(
                move |a, b| workers::rescan::worker(uniscan.clone(), a, b),
                |state: &mut App, sender| state.sender_rescan = Some(sender),
                |state, res: Result<rescan::Response>| match res {
                    Ok(res) => match res {
                        rescan::Response::ScanFinished(scan) => {
                            state.clear_error();
                            state.main.results = Some(scan);
                        }
                        rescan::Response::Error(err) => state.set_error(err),
                    },
                    Err(err) => state.set_error(err),
                },
            ),
        )
    }
}

fn main() -> Result<(), EventLoopError> {
    let mut app = App::default();

    app.set_error_with(|app| Ok(app.gameselect.steam_games = find_games()?));

    let app = Xilem::new_simple(app, App::ui, WindowOptions::new("uniscan"));
    app.run_in(EventLoop::with_user_event())?;
    Ok(())
}

fn find_games() -> Result<Vec<SteamGame>> {
    let steam_dir = steamlocate::SteamDir::locate()?;

    let mut results = Vec::new();
    for lib in steam_dir.libraries()? {
        let lib = lib?;
        for app in lib.apps() {
            let app = app?;

            let Ok(path) = GameFiles::probe_dir(&lib.resolve_app_dir(&app)) else {
                continue;
            };

            let game = UnityGame {
                name: app.name.clone().unwrap_or_else(|| app.install_dir.clone()),
                path,
            };
            results.push(SteamGame {
                game,
                app_id: app.app_id,
            })
        }
    }

    Ok(results)
}

fn unity_game_from_path(path: &Path) -> Result<UnityGame> {
    let env = Environment::new_in(path, NullTypeTreeProvider)?;
    let name = env.app_info()?.name;

    Ok(UnityGame {
        name,
        path: env.resolver.game_dir,
    })
}
