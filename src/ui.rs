use std::{future::Future, time::Duration};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind};
use futures::{FutureExt, StreamExt};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect}, style::{Color, Style, Stylize}, widgets::{Block, Borders, List, ListItem, ListState, Paragraph}, DefaultTerminal, Frame
};
use anyhow::{Error, Result};
use tokio::{sync::mpsc::{channel, Receiver, Sender}, task::JoinHandle, time::sleep};

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

enum Modal {
    Hidden,
    Visible(i64)
}

#[derive()]
pub struct App {
    // Is the application running?
    // gee i sure hope so
    running: bool,
    event_stream: EventStream,
    client: Logs,
    data_chan: Chan<Data>,
    error_chan: Chan<Error>,
    fut: Option<JoinHandle<()>>,
    modal: Modal,
    status: Status,
    query: String,
    rows: DataList,
    ctx_rows: Vec<String>
}


type Log = (i64, String);

struct DataList {
    pub items: Vec<Log>,
    pub state: ListState
}

enum Data {
    Logs(Vec<(i64, String)>),
    Context(Vec<String>)
}

enum Status {
    Loading,
    Loaded,
    Failed(Error),
}

type LoadFn = Box<dyn FnOnce(Logs, Sender<Data>, Sender<Error>) -> Box<dyn Future<Output = ()> + Send + 'static> + Send + 'static>;


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
        let fut = None;
        let modal = Modal::Hidden;
        let ctx_rows = Vec::new();

        let mut res = Self { ctx_rows, modal, fut, running, event_stream, client, data_chan, error_chan, status, rows, query };
        res.load_more();

        res
    }



    // Wrap the async function, spawn it and send it's results back via channels.
    fn load<F, Fut>(&mut self, act: F)
        where F: FnOnce(Logs, Sender<Data>, Sender<Error>) -> Fut + 'static + Send,
              Fut: Future<Output = ()> + Send + 'static
    {
        self.status = Status::Loading;
        if let Some(f) = self.fut.take() {
            f.abort();
        }

        let data_tx = self.data_chan.tx.clone();
        let err_tx = self.error_chan.tx.clone();
        let logs = self.client.clone();
        self.fut = Some(tokio::spawn(async move {
            sleep(Duration::from_millis(33)).await;
            act(logs, data_tx, err_tx).await;
        }))
    }

    fn load_more(&mut self) {
        let q = self.query.clone();
        self.load(|mut logs, data_tx, err_tx| async move  {
            logs.set_query(q);
            let result = logs.get_more_logs().await;
            let wrapped = Data::Logs(result);
            if let Err(e) = data_tx.send(wrapped).await {
                err_tx.send(anyhow::anyhow!(e)).await.expect("Failed to send error, something's fucked!");
            }
        });
    }

    fn load_context(&mut self, ts: i64) {
        self.load(move |mut logs, data_tx, err_tx| async move {
            let result = logs.find_context(ts).await;
            let wrapped = Data::Context(result);
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

    fn draw_modal(&self, f: &mut Frame) {

    }

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

        match self.modal {
            Modal::Visible(_) => {
                self.draw_modal(f);
            }
            Modal::Hidden => {}
        }

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
                    },
                    Some(Data::Context(ctx)) => {
                        self.ctx_rows = ctx;
                        self.rows.state.select_last();
                    },
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
            (_, KeyCode::Esc) => {
                match self.modal {
                    Modal::Hidden => self.quit(),
                    Modal::Visible(_) => {
                        self.modal = Modal::Hidden;
                    }
                }
                self.quit();
            },
            (_, KeyCode::Down) => self.rows.state.select_next(),
            (_, KeyCode::Up) => self.rows.state.select_previous(),
            (_, KeyCode::Char(a)) => {
                if matches!(self.modal, Modal::Hidden) {
                    self.query.push(a);
                    self.load_more();
                }
            },
            (_, KeyCode::Backspace) => {
                if matches!(self.modal, Modal::Hidden) {
                    let _ = self.query.pop();
                    self.load_more();
                }
            },
            (_, KeyCode::Enter) => {
                let selected = self.rows.state.selected();
                if let Some(selected) = selected {
                    let ts = self.rows.items[selected].0;
                    self.modal = Modal::Visible(ts);
                    self.load_context(ts);
                }
            },
            // Add other key handlers here.
            _ => {}
        }
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }
}
