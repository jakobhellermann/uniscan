use std::path::PathBuf;

use gpui::{Size, *};
use gpui_component::button::Button;
use gpui_component::label::Label;
use gpui_component::table::{Column, Table, TableDelegate, TableEvent};
use gpui_component::*;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

pub struct SelectScreen {
    table: Entity<Table<MyTableDelegate>>,
    selected: Option<usize>,
}

impl SelectScreen {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let app = cx.new(|cx| Self::new(window, cx));

        app
    }
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> SelectScreen {
        let delegate = MyTableDelegate::new();
        let table = cx.new(|cx| Table::new(delegate, window, cx).col_movable(false));

        cx.subscribe_in(&table, window, |view, _, event, _, cx| match *event {
            TableEvent::SelectRow(row) => {
                view.selected = Some(row);
                cx.notify();
            }
            _ => {}
        })
        .detach();

        cx.spawn(async |this, cx| {
            let _ = this.update(cx, |t, cx| {
                t.table.update(cx, |table, cx| {
                    table.delegate_mut().data = logic::find_games().unwrap();
                    cx.notify();
                });
            });
        })
        .detach();

        SelectScreen {
            table,
            selected: None,
        }
    }
}
impl Render for SelectScreen {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .p_4()
            .h_full()
            .child(Label::new("Select a unity game").text_xl())
            .child(
                div()
                    .flex_1()
                    .paddings(8.)
                    .w_full()
                    .h_full()
                    .child(self.table.clone()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("open").label("Open").disabled(true))
                    .child(
                        Button::new("Open another")
                            .label("Select")
                            .disabled(self.selected.is_none()),
                    ),
            )
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
                    let view = SelectScreen::view(window, cx);
                    cx.new(|cx| Root::new(view.into(), window, cx))
                },
            )?;

            Ok(())
        })
        .detach();
    });
}

struct MyTableDelegate {
    data: Vec<logic::SteamGame>,
    columns: Vec<Column>,
}

impl MyTableDelegate {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            columns: vec![
                Column::new("id", "App ID").width(100.),
                Column::new("name", "Name").width(200.).sortable(),
            ],
        }
    }
}

impl TableDelegate for MyTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.data.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_td(
        &self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<Table<Self>>,
    ) -> impl IntoElement {
        let row = &self.data[row_ix];
        let col = &self.columns[col_ix];

        match col.key.as_ref() {
            "id" => row.app_id.to_string().into_any_element(),
            "name" => row.game.name.clone().into_any_element(),
            _ => unreachable!(),
        }
    }
}

mod logic {
    use std::path::PathBuf;

    use anyhow::Result;
    use rabex_env::game_files::GameFiles;

    pub struct UnityGame {
        pub name: String,
        pub path: PathBuf,
    }

    pub struct SteamGame {
        pub game: UnityGame,
        pub app_id: u32,
    }

    pub fn find_games() -> Result<Vec<SteamGame>> {
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
}
