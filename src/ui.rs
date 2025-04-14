use std::{future::Future, sync::Arc, time::Duration};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind};
use futures::{FutureExt, StreamExt};
use ratatui::{
    layout::{Constraint, Direction, Flex, Layout, Rect}, style::{Color, Style, Stylize}, widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph}, DefaultTerminal, Frame
};
use anyhow::{Error, Result};
use tokio::{sync::{mpsc::{channel, Receiver, Sender}, Mutex}, task::JoinHandle, time::sleep};

use crate::cw::Logs;

// Mnemonic for Rust's most annoying type
type Z<T> = Arc<tokio::sync::Mutex<T>>;

// Entrypoint for the UI.
pub async fn run(logs: Logs) -> Result<()> {
    let terminal = ratatui::init();
    App::new(logs).run(terminal).await?;
    ratatui::restore();

    Ok(())
}

// Helper to hold both sides of a channel. We need to retain the receiver, and hand out clones of
// the sender.
struct Chan<T> {
    rx: Receiver<T>,
    tx: Sender<T>,
}

impl<T> From<(Sender<T>, Receiver<T>)> for Chan<T> {
    fn from((tx, rx): (Sender<T>, Receiver<T>)) -> Self {
        Self { rx, tx }
    }
}

enum Mode {
    Searching(SearchState),
    Viewing(ViewState)
}

type Log = (i64, String);

#[derive(Default)]
struct SearchState {
    query: String,
    items: Vec<Log>,
    state: ListState
}

impl SearchState {
    fn draw(&mut self, area: Rect, f: &mut Frame) {
        let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Fill(1), Constraint::Length(3)].as_ref())
                .split(area);
        let list_items: Vec<ListItem> = self.items
            .iter()
            .map(|item| ListItem::from(item.1.clone()))
            .collect();

        let list = List::new(list_items)
            .block(Block::default()
            .borders(Borders::ALL)
            .title("Results"))
            .highlight_style(Style::new().bg(Color::DarkGray));
        f.render_stateful_widget(list, chunks[0], &mut self.state);

        let input_paragraph = Paragraph::new(self.query.clone()) // ARGH
            .block(Block::default().borders(Borders::ALL).title("Query"));
        f.render_widget(input_paragraph, chunks[1]);
    }
}

#[derive(Default)]
struct ViewState {
    prev: SearchState,
    items: Vec<String>,
    state: ListState,
    selected_ts: i64
}

impl ViewState {
    fn draw(&mut self, f: &mut Frame) {
        let area = popup_area(f.area(), 95, 95);
        // TODO: Format as date
        let block = Block::bordered().title(format!("Viewing context: {} +/- 1000", self.selected_ts));
        let list = List::new(self.items.iter().map(|row| {ListItem::new(row.to_string())}))
            .highlight_style(Style::new().bg(Color::DarkGray))
            .block(block);
        f.render_widget(Clear, area);
        f.render_stateful_widget(list, area, &mut self.state);
    }
}

// Data channel messages
enum Data {
    Logs(Vec<(i64, String)>),
    Context(Vec<String>)
}

// Made a proper mess of this state... Some enums could probably better cover the load states.
pub struct App {
    // Is the application running?
    // gee i sure hope so
    running: bool,
    event_stream: EventStream,
    client: Z<Logs>,
    data_chan: Chan<Data>,
    error_chan: Chan<Error>,
    fut: Option<JoinHandle<()>>,
    mode: Mode,
    status: Status
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
        let fut = None;
        let client = Arc::new(Mutex::new(client));
        let mode = Mode::Searching(SearchState::default());

        let mut res = Self { mode, fut, running, event_stream, client, data_chan, error_chan, status };
        res.load_more("".into());

        res
    }

    // Wrap the async function, spawn it and send it's results back via channels.
    // The sig is nasty, but it's saying we need a func that takes in the client, the channels and returns Promise<void> (teehee)
    // Tried to simplify it, it would have to be defined as a trait, therefore a "Functor", which adds too much more complexity;
    // or via dynamic dispatch, but then we summon the pin demons.
    fn load<F, Fut>(&mut self, act: F)
        where F: FnOnce(Z<Logs>) -> Fut + 'static + Send,
              Fut: Future<Output = Result<Data>> + Send + 'static
    {
        self.status = Status::Loading;
        // Naive debouncing, we tack on a sleep to all load requests,
        // subsequent ones get cancelled. This does add some delay, but consideirng every API call
        // is billed, probably preferable than dispatching every keystroke.
        if let Some(f) = self.fut.take() {
            // Is this the correct way to cancel a task? Seems ok if we have no "cleanup"
            // Notably, dropping the JoinHandle does not cancel the task!
            f.abort();
        }

        // Channels are easy and all, but I feel it's a bit messy. idk if the Actor pattern
        // is worthwhile here...Might be a good exercise if you want to try and "improve" the
        // code quality lmao.
        let data_tx = self.data_chan.tx.clone();
        let err_tx = self.error_chan.tx.clone();
        let logs = Arc::clone(&self.client);
        self.fut = Some(tokio::spawn(async move {
            sleep(Duration::from_millis(33)).await;
            let res = act(logs).await;
            match res {
                Ok(d) => { data_tx.send(d).await.expect("Shitter's full"); },
                Err(e) => { err_tx.send(e).await.expect("Shitter's full"); }
            };
        }))
    }

    fn load_more(&mut self, q: String) {
        self.load(|logs| async move  {
            let mut logs = logs.lock().await;
            logs.set_query(q);
            let result = logs.get_more_logs().await?;
            Ok(Data::Logs(result))
        });
    }

    fn load_context(&mut self, ts: i64) {
        self.load(move |logs| async move {
            let mut logs = logs.lock().await;
            let result = logs.find_context(ts).await?;
            Ok(Data::Context(result))
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

    fn draw(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Fill(1), Constraint::Length(1)].as_ref())
                .split(f.area());
        match &mut self.mode {
            Mode::Searching(ref mut s) => {
                s.draw(chunks[0], f);
            },
            Mode::Viewing(ref mut v) => {
                v.prev.draw(chunks[0], f);
                v.draw(f);
            }
        }
        let s = match self.status {
            Status::Loaded => " Done".into(),
            Status::Loading => " Loading...".into(),
            Status::Failed(ref e) => format!(" Error: {}", e),
        };
        let status_line = Paragraph::new(s)
            .fg(Color::White)
            .bg(Color::Blue);
        f.render_widget(status_line, chunks[1]);
    }

    /// Reads the crossterm events and updates the state of [`App`].
    async fn handle_crossterm_events(&mut self) -> Result<()> {
        // 'Fuse' means as in like blowing a fuse.
        // It means after the Future returns None, do not poll it again.
        // Streams should be fused, if they are used in a select! macro.
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
            // See, the fuse here is probably unnecesssary, as the channel will not close.
            err = self.error_chan.rx.recv().fuse() => {
                if let Some(err) = err {
                    self.status = Status::Failed(err);
                }
            }
            data = self.data_chan.rx.recv().fuse() => {
                match (&mut self.mode, data) {
                    (Mode::Searching(ref mut s), Some(Data::Logs(logs))) => {
                        s.items = logs;
                        s.state.select_first();
                    },
                    (Mode::Viewing(ref mut v), Some(Data::Context(ctx))) => {
                        v.items = ctx;
                        v.state.select_last();
                    },
                    _ => {}
                }
                self.status = Status::Loaded;
            }
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (&mut self.mode, key.code) {
            (Mode::Searching(_), KeyCode::Esc) => {
                self.quit();
            },
            (Mode::Viewing(vs), KeyCode::Esc) => {
                let vs = std::mem::take(vs);
                let ViewState { prev, items: _, state: _, selected_ts: _ } = vs;
                self.mode = Mode::Searching(prev);
            },
            (Mode::Searching(s), KeyCode::Down) => s.state.select_next(),
            (Mode::Searching(s), KeyCode::Up) => s.state.select_previous(),
            (Mode::Searching(s), KeyCode::Left) => s.state.select_first(),
            (Mode::Searching(s), KeyCode::Right) => s.state.select_last(),
            (Mode::Viewing(v), KeyCode::Down) => v.state.select_next(),
            (Mode::Viewing(v), KeyCode::Up) => v.state.select_previous(),
            (Mode::Viewing(v), KeyCode::Right) => v.state.select_last(),
            (Mode::Viewing(v), KeyCode::Left) => v.state.select_first(),
            (Mode::Searching(s), KeyCode::Char(a)) if a.is_alphanumeric() => {
                s.query.push(a);
                let q = s.query.clone();
                self.load_more(q);
            },
            (Mode::Searching(s), KeyCode::Backspace) => {
                let _ = s.query.pop();
                let q = s.query.clone();
                self.load_more(q);
            },
            (Mode::Searching(s), KeyCode::Enter) => {
                let selected = s.state.selected();
                if let Some(prev) = selected {
                    let selected_ts = s.items[prev].0;
                    let prev = std::mem::take(s);
                    let vs = ViewState { prev, selected_ts, ..Default::default()};
                    self.mode = Mode::Viewing(vs);
                    self.load_context(selected_ts);
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

fn popup_area(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
