use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind};
use futures::{FutureExt, StreamExt};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect}, style::{Color, Style, Stylize}, widgets::{Block, Borders, List, ListItem, ListState, Paragraph}, DefaultTerminal, Frame
};
use anyhow::{Error, Result};
use tokio::sync::mpsc::{Receiver, Sender, channel};

use crate::cw::Logs;

pub async fn run(logs: Logs) -> Result<()> {
    let terminal = ratatui::init();
    App::new(logs).run(terminal).await?;
    ratatui::restore();

    Ok(())
}

// TODO: move to utils?
struct Chan<T> {
    rx: Receiver<T>,
    tx: Sender<T>,
}

impl<T> From<(Sender<T>, Receiver<T>)> for Chan<T> {
    fn from((tx, rx): (Sender<T>, Receiver<T>)) -> Self {
        Self { rx, tx }
    }
}

const fn alternate_colors(i: usize) -> Color {
    if i % 2 == 0 {
        Color::DarkGray
    } else {
        Color::Black
    }
}

#[derive()]
pub struct App {
    /// Is the application running?
    running: bool,
    // Event stream.
    event_stream: EventStream,
    client: Logs,
    data_chan: Chan<Data>,
    error_chan: Chan<Error>,
    status: Status,
    query: String,
    rows: DataList
}


type Log = (i64, String);

struct DataList {
    pub items: Vec<Log>,
    pub state: ListState
}

enum Data {
    Logs(Vec<(i64, String)>),
}

enum Status {
    Loading,
    Loaded,
    Failed(Error),
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new(client: Logs) -> Self {
        let event_stream = EventStream::default();
        let running = false;
        let data_chan = channel::<Data>(10).into();
        let error_chan = channel::<Error>(10).into();
        let status = Status::Loading;
        let rows = DataList { items: Vec::new(), state: ListState::default() };
        let query = "".into();

        let mut res = Self { running, event_stream, client, data_chan, error_chan, status, rows, query };
        res.load_more();

        res
    }

    // Wrap the async function, spawn it and send it's results back via channels.
    fn load_more(&mut self) {
        self.status = Status::Loading;
        let data_tx = self.data_chan.tx.clone();
        let err_tx = self.error_chan.tx.clone();
        let mut logs = self.client.clone();
        tokio::spawn(async move {
            let result = logs.get_more_logs().await;
            let wrapped = Data::Logs(result);
            if let Err(e) = data_tx.send(wrapped).await {
                err_tx.send(anyhow::anyhow!(e)).await.expect("Failed to send error, something's fucked!");
            }
        });
    }

    /// Run the application's main loop.
    pub async fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        while self.running {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_crossterm_events().await?;
        }
        Ok(())
    }

    /// Renders the user interface.
    ///
    /// This is where you add new widgets. See the following resources for more information:
    /// - <https://docs.rs/ratatui/latest/ratatui/widgets/index.html>
    /// - <https://github.com/ratatui/ratatui/tree/master/examples>
    fn draw(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Fill(1),
                    Constraint::Length(3),
                    Constraint::Length(1)
                ]
                .as_ref(),
            )
            .split(f.area());

        self.render_list(chunks[0], f);

        let input_paragraph = Paragraph::new(self.query.clone())
            .block(Block::default().borders(Borders::ALL).title("Query"));
        f.render_widget(input_paragraph, chunks[1]);

        let s = match self.status {
            Status::Loaded => " Done".into(),
            Status::Loading => " Loading...".into(),
            Status::Failed(ref e) => format!(" Error: {}", e),
        };
        let status_line = Paragraph::new(s)
            .fg(Color::White)
            .bg(Color::Blue);
        f.render_widget(status_line, chunks[2]);
    }

    fn render_list(&mut self, area: Rect, f: &mut Frame) {
        let list_items: Vec<ListItem> = self.rows.items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let c = alternate_colors(i);
                ListItem::from(item.1.clone())
            })
            .collect();

        let list = List::new(list_items)
            .block(Block::default()
            .borders(Borders::ALL)
            .title("Results"))
            .highlight_style(Style::new().bg(Color::DarkGray));
        f.render_stateful_widget(list, area, &mut self.rows.state);
    }


    /// Reads the crossterm events and updates the state of [`App`].
    async fn handle_crossterm_events(&mut self) -> Result<()> {
        tokio::select! {
            event = self.event_stream.next().fuse() => {
                match event {
                    Some(Ok(evt)) => {
                        match evt {
                            Event::Key(key)
                                if key.kind == KeyEventKind::Press
                                    => self.on_key_event(key),
                            Event::Mouse(_) => {}
                            Event::Resize(_, _) => {}
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            err = self.error_chan.rx.recv() => {
                if let Some(err) = err {
                    self.status = Status::Failed(err);
                }
            }
            data = self.data_chan.rx.recv() => {
                match data {
                    Some(Data::Logs(logs)) => {
                        self.rows.items = logs;
                        self.rows.state.select_first();
                    }
                    None => {}
                }
                self.status = Status::Loaded;
            }
            // _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
            //     // Sleep for a short duration to avoid busy waiting.
            // }
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => self.quit(),
            (_, KeyCode::Down) => self.rows.state.select_next(),
            (_, KeyCode::Up) => self.rows.state.select_previous(),
            (_, KeyCode::Char(a)) => {
                self.query.push(a);
            },
            (_, KeyCode::Backspace) => { let _ = self.query.pop(); },
            // Add other key handlers here.
            _ => {}
        }
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }
}
