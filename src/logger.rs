use std::{str::FromStr, sync::Mutex};

use lazy_static::lazy_static;
use log::LevelFilter;
use log4rs::{
    append::{
        console::{ConsoleAppender, Target},
        file::FileAppender,
    },
    config::{Appender, Root},
    encode::pattern::PatternEncoder,
    Config, Handle,
};

use crate::env;

lazy_static! {
    static ref LOG_HANDLE: Mutex<Option<Handle>> = Mutex::new(None);
}

pub fn init_log(log_level: &str) {
    let handle = log4rs::init_config(create_config(log_level)).expect("log init failed!!");
    LOG_HANDLE.lock().unwrap().replace(handle);
}

pub fn change_log_level(log_level: &str) {
    let config = create_config(log_level);
    let mut handle = LOG_HANDLE.lock().unwrap();
    if handle.is_some() {
        handle.as_mut().unwrap().set_config(config);
    }
}

fn create_config(log_level: &str) -> Config {
    let mut level = LevelFilter::Info;
    if !log_level.is_empty() {
        level = LevelFilter::from_str(log_level).unwrap();
    }
    let mut log_file_path = env::ROOT_DIR.clone();
    log_file_path.push("process-compose.log");
    let log_pattern = Box::new(PatternEncoder::new(
        "{d(%Y-%m-%d %H:%M:%S)} {f} {L} {l} - {m}\n",
    ));
    let console = ConsoleAppender::builder()
        .encoder(log_pattern.clone())
        .target(Target::Stdout)
        .build();
    // Logging to log file.
    let logfile = FileAppender::builder()
        // Pattern: https://docs.rs/log4rs/*/log4rs/encode/pattern/index.html
        .encoder(log_pattern)
        .build(log_file_path)
        .unwrap();
    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .appender(Appender::builder().build("console", Box::new(console)))
        .build(
            Root::builder()
                .appender("logfile")
                .appender("console")
                .build(level),
        )
        .unwrap();
    config
}
