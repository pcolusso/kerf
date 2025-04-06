use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::{FutureExt, StreamExt};
use ratatui::{
    DefaultTerminal, Frame,
    style::Stylize,
    text::Line,
    widgets::{Block, Paragraph},
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
    rows: Vec<(i64, String)>,
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
        let rows = Vec::new();

        let mut res = Self { running, event_stream, client, data_chan, error_chan, status, rows };
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
    fn draw(&mut self, frame: &mut Frame) {
        let title = Line::from("Ratatui Simple Template")
            .bold()
            .blue()
            .centered();
        let text = "Hello, Ratatui!\n\n\
            Created using https://github.com/ratatui/templates\n\
            Press `Esc`, `Ctrl-C` or `q` to stop running.";
        frame.render_widget(
            Paragraph::new(text)
                .block(Block::bordered().title(title))
                .centered(),
            frame.area(),
        )
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
                        // TODO!
                    }
                    None => {}
                }
                self.status = Status::Loaded;
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                // Sleep for a short duration to avoid busy waiting.
            }
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
            // Add other key handlers here.
            _ => {}
        }
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }
}
