// Based off of the great SimpleLogger crate: https://crates.io/crates/simple_logger
use colored::*;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex};
use strip_ansi_escapes::strip;
use termsize::Size;

use crate::constants;

pub struct TreeLogger {
    default_level: LevelFilter,
    threads_enabled: bool,
    colors_enabled: bool,
    use_stderr: bool,
    filter_fn: fn(&LoggingEvent) -> bool,
    data: LoggingData,
}

#[derive(Debug, Default, Clone)]
struct LoggingData {
    // Maps thread ids to logging data
    internal_data: Arc<Mutex<FxHashMap<String, InternalLoggingData>>>,
}

#[derive(Debug, Default, Clone)]
struct InternalLoggingData {
    indentation: usize,
    next_id: usize,
    events: Vec<LoggingEvent>,
}

#[derive(Debug, Clone)]
pub struct LoggingEvent {
    pub id: Option<usize>,
    pub indentation: usize,
    pub elapsed: Option<u128>,
    pub level: Level,
    pub target: String,
    pub args: String,
    pub thread: String,
    pub quiet: bool,
}

impl LoggingEvent {
    fn get_args(&self) -> String {
        use ansi_term::Colour::{Cyan, Red};
        match self.elapsed {
            Some(elapsed) => {
                if elapsed > 100 {
                    format!("{}: {}", self.args, Red.paint(format!("{elapsed}ms")))
                } else {
                    format!("{}: {}", self.args, Cyan.paint(format!("{elapsed}ms")))
                }
            }
            None => self.args.clone(),
        }
    }
}

impl LoggingData {
    fn get_name(&self) -> String {
        let thread = std::thread::current();
        thread.name().unwrap_or("default").to_string()
    }

    fn increment(&self) {
        let mut data = self.internal_data.lock().unwrap();
        let data = data.entry(self.get_name()).or_default();
        data.indentation += 1;
    }

    fn decrement(&self) {
        let mut data = self.internal_data.lock().unwrap();
        let data = data.entry(self.get_name()).or_default();
        data.indentation -= 1;
    }

    fn push_record(&self, record: &Record, should_log_thread: bool) {
        let id = if let Some(id_value) = record.key_values().get(constants::ID.into()) {
            if let Ok(id) = id_value.to_string().parse::<usize>() {
                Some(id)
            } else {
                None
            }
        } else {
            None
        };

        let quiet = if let Some(quiet) = record.key_values().get(constants::QUIET.into()) {
            match quiet.to_string().parse::<usize>() {
                Ok(quiet) => quiet == 1,
                Err(_) => false,
            }
        } else {
            false
        };

        self.push(LoggingEvent {
            id,
            quiet,
            level: record.level(),
            target: if !record.target().is_empty() {
                record.target()
            } else {
                record.module_path().unwrap_or_default()
            }
            .to_string(),

            args: record.args().to_string(),
            indentation: 0,
            elapsed: None,
            thread: if should_log_thread {
                let thread = std::thread::current();

                match thread.name() {
                    Some(name) => {
                        if name == "main" {
                            "".into()
                        } else {
                            format!(" @{name}")
                        }
                    }
                    None => "".into(),
                }
            } else {
                "".into()
            },
        });
    }

    fn push(&self, mut event: LoggingEvent) -> usize {
        let mut data = self.internal_data.lock().unwrap();
        let data = data.entry(self.get_name()).or_default();
        event.indentation = data.indentation;

        // TODO: do I need ID anymore?
        let id = data.next_id;
        data.next_id += 1;

        data.events.push(event);
        id
    }

    fn get_data_to_log(&self) -> Option<Vec<LoggingEvent>> {
        let mut data = self.internal_data.lock().unwrap();
        let data = data.entry(self.get_name()).or_default();
        if data.indentation == 0 {
            let mut rv = Vec::new();
            std::mem::swap(&mut data.events, &mut rv);
            return Some(rv);
        }
        None
    }

    fn set_time(&self, id: usize, ms: u128) {
        let mut data = self.internal_data.lock().unwrap();
        let data = data.entry(self.get_name()).or_default();
        for record in &mut data.events {
            if let Some(record_id) = record.id {
                if record_id == id {
                    record.elapsed = Some(ms);
                    return;
                }
            }
        }
        eprintln!("Couldn't set time!");
    }
}

impl Default for TreeLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeLogger {
    /// Initializes the global logger with a CustomLogger instance with
    /// default log level set to `Level::Trace`.
    ///
    /// ```no_run
    /// use tree_logger::TreeLogger;
    /// TreeLogger::new().with_colors(true).with_threads(true).init().unwrap();
    /// log::warn!("This is an example message.");
    /// ```
    ///
    /// [`init`]: #method.init
    #[must_use = "You must call init() to begin logging"]
    pub fn new() -> TreeLogger {
        TreeLogger {
            default_level: LevelFilter::Trace,
            threads_enabled: false,
            colors_enabled: false,
            use_stderr: false,
            filter_fn: |_| true,
            data: LoggingData::default(),
        }
    }

    pub fn init(self) -> Result<(), SetLoggerError> {
        log::set_max_level(self.max_level());
        log::set_boxed_logger(Box::new(self))
    }

    #[must_use = "You must call init() to begin logging"]
    pub fn with_filter_fn(mut self, filter_fn: fn(&LoggingEvent) -> bool) -> TreeLogger {
        self.filter_fn = filter_fn;
        self
    }

    #[must_use = "You must call init() to begin logging"]
    pub fn with_level(mut self, level: LevelFilter) -> TreeLogger {
        self.default_level = level;
        self
    }

    #[must_use = "You must call init() to begin logging"]
    pub fn with_threads(mut self, enable_threads: bool) -> TreeLogger {
        self.threads_enabled = enable_threads;
        self
    }

    /// Control whether messages are colored or not.
    #[must_use = "You must call init() to begin logging"]
    pub fn with_colors(mut self, enable_colors: bool) -> TreeLogger {
        self.colors_enabled = enable_colors;
        self
    }

    pub fn max_level(&self) -> LevelFilter {
        self.default_level
    }

    fn get_level_string(&self, level: Level) -> String {
        let level_string = format!("{:<5}", level.to_string());
        if self.colors_enabled {
            match level {
                Level::Error => level_string.red(),
                Level::Warn => level_string.yellow(),
                Level::Info => level_string.cyan(),
                Level::Debug => level_string.purple(),
                Level::Trace => level_string.normal(),
            }
            .to_string()
        } else {
            level_string
        }
    }

    fn print_data(&self, data: Vec<LoggingEvent>) {
        if data.len() == 0 {
            return;
        }

        if !(self.filter_fn)(&data[0]) {
            return;
        }

        if data.len() == 1 && data[0].quiet && data[0].elapsed.unwrap_or(u128::MAX) == 0 {
            return;
        }

        let terminal_width = termsize::get().unwrap_or(Size { rows: 0, cols: 0 }).cols as usize;
        for record in data.iter().filter(|e| (self.filter_fn)(e)) {
            let left = format!(
                "{} {:indent$}{}",
                self.get_level_string(record.level),
                " ",
                record.get_args(),
                indent = record.indentation.checked_sub(1).unwrap_or_default() * 2,
            );

            let right = format!("[{}{}]", record.target, record.thread);

            let width = String::from_utf8(strip(format!("{left}{right}").as_bytes()))
                .unwrap_or_default()
                .len();
            let message = if terminal_width > 0 && width + 5 < terminal_width {
                format!(
                    "{}{:padding$}{}",
                    left,
                    " ",
                    right,
                    padding = terminal_width - width
                )
            } else {
                left
            };

            if self.use_stderr {
                eprintln!("{}", message);
            } else {
                println!("{}", message);
            }
        }
    }
}

impl Log for TreeLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level().to_level_filter() <= self.default_level
    }

    fn log(&self, record: &Record) {
        if record
            .key_values()
            .get(constants::INCREMENT.into())
            .is_some()
        {
            self.data.increment();
        } else if record
            .key_values()
            .get(constants::DECREMENT.into())
            .is_some()
        {
            self.data.decrement();
        } else if record
            .key_values()
            .get(constants::SET_TIME.into())
            .is_some()
        {
            if let Some(time_value) = record.key_values().get(constants::TIME.into()) {
                if let Ok(time) = time_value.to_string().parse::<u128>() {
                    if let Some(id_value) = record.key_values().get(constants::ID.into()) {
                        if let Ok(id) = id_value.to_string().parse::<usize>() {
                            self.data.set_time(id, time);
                        }
                    }
                }
            }
        } else {
            if !self.enabled(record.metadata()) {
                return;
            }

            self.data.push_record(record, self.threads_enabled);
        }

        if let Some(data) = self.data.get_data_to_log() {
            self.print_data(data);
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod test {
    // use super::*;

    // TODO: how to test?
    #[test]
    fn test_module_levels_denylist() {}
}
