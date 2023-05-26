use bevy::{
    app::{App, CoreSet, Plugin},
    ecs::{schedule::IntoSystemConfig, system::Resource},
    log,
    utils::tracing::Metadata,
};
use std::{collections::VecDeque, io::LineWriter, panic};
use tracing_log::LogTracer;
use tracing_subscriber::{fmt::MakeWriter, prelude::*, registry::Registry, EnvFilter};

const MAX_ENTRIES_COUNT: usize = 10_000;

struct MakeChannelWriter {
    tx: tokio::sync::mpsc::UnboundedSender<LogEntry>,
}

impl<'a> MakeWriter<'a> for MakeChannelWriter {
    type Writer = LineWriter<ChannelWriter>;

    fn make_writer(&'a self) -> Self::Writer {
        LineWriter::new(ChannelWriter::new(self.tx.clone(), log::Level::INFO))
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        LineWriter::new(ChannelWriter::new(self.tx.clone(), *meta.level()))
    }
}

struct ChannelWriter {
    tx: tokio::sync::mpsc::UnboundedSender<LogEntry>,
    level: log::Level,
}

pub struct LogEntry {
    pub message: String,
    pub level: log::Level,
    pub timestamp: String,
}

impl ChannelWriter {
    pub fn new(tx: tokio::sync::mpsc::UnboundedSender<LogEntry>, level: log::Level) -> Self {
        Self { tx, level }
    }
}

impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let s = String::from_utf8_lossy(&buf).to_string();
        let (timestamp, message) = s.split_once(self.level.as_str()).unwrap_or(("", &s));
        self.tx
            .send(LogEntry {
                message: message.to_owned(),
                level: self.level,
                timestamp: timestamp.to_owned(),
            })
            .map_err(|_err| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "channel receiver dropped")
            })?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Resource)]
pub struct LogsReceiver {
    rx: tokio::sync::mpsc::UnboundedReceiver<LogEntry>,
    buf: VecDeque<LogEntry>,
}

impl LogsReceiver {
    fn new(rx: tokio::sync::mpsc::UnboundedReceiver<LogEntry>, max_entries_count: usize) -> Self {
        Self {
            rx,
            buf: VecDeque::with_capacity(max_entries_count),
        }
    }

    pub fn receive(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(log_string) => {
                    if self.buf.len() == self.buf.capacity() {
                        self.buf.pop_front();
                    }
                    self.buf.push_back(log_string);
                }
                Err(_err) => break,
            }
        }
    }

    pub fn entires(&self) -> &VecDeque<LogEntry> {
        &self.buf
    }
}

/// A plugin that sets up `puffin` and configures it as a `tracing-subscriber`
/// layer. It also adds a in-memory subscriber to display logs in Egui.
///
/// Note that this plugin can't be used with Bevy's default `LogPlugin`.
pub struct MuddleTracePlugin;

impl Plugin for MuddleTracePlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "profiler")]
        app.add_system(bevy_puffin::new_frame_system.in_base_set(CoreSet::First));

        {
            let old_handler = panic::take_hook();
            panic::set_hook(Box::new(move |infos| {
                println!("{}", tracing_error::SpanTrace::capture());
                old_handler(infos);
            }));
        }

        let finished_subscriber;
        let filter_layer = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new("info,wgpu=error"))
            .unwrap();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        app.insert_resource(LogsReceiver::new(rx, MAX_ENTRIES_COUNT));
        let subscriber = Registry::default().with(filter_layer);

        #[cfg(feature = "profiler")]
        let subscriber = subscriber.with(bevy_puffin::PuffinLayer::new());

        let subscriber = subscriber
            .with(
                tracing_subscriber::fmt::Layer::default()
                    .with_ansi(false)
                    .with_writer(MakeChannelWriter { tx }),
            )
            .with(tracing_error::ErrorLayer::default());

        #[cfg(not(target_arch = "wasm32"))]
        {
            let fmt_layer = tracing_subscriber::fmt::Layer::default();
            finished_subscriber = subscriber.with(fmt_layer);
        }

        #[cfg(target_arch = "wasm32")]
        {
            console_error_panic_hook::set_once();
            finished_subscriber = subscriber.with(tracing_wasm::WASMLayer::new(
                tracing_wasm::WASMLayerConfig::default(),
            ));
        }

        let logger_already_set = LogTracer::init().is_err();
        let subscriber_already_set =
            bevy::utils::tracing::subscriber::set_global_default(finished_subscriber).is_err();

        match (logger_already_set, subscriber_already_set) {
            (true, true) => log::warn!(
                "Could not set global logger and tracing subscriber for bevy_puffin as they are already set. Consider disabling LogPlugin or re-ordering plugin initialization."
            ),
            (true, _) => log::warn!("Could not set global logger as it is already set. Consider disabling LogPlugin."),
            (_, true) => log::warn!("Could not set global tracing subscriber as it is already set. Consider disabling LogPlugin."),
            _ => (),
        }
    }
}
