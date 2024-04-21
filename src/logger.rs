use log::LevelFilter;
use log4rs::{
    append::{
        console::{ConsoleAppender, Target},
        file::FileAppender,
    },
    config::{Appender, Root},
    encode::pattern::PatternEncoder,
    Config,
};

use crate::env;

pub fn init_log() {
    let mut log_file_path = env::ROOT_DIR.clone();
    log_file_path.push("process-compose.log");
    let log_pattern = Box::new(PatternEncoder::new(
        "{d(%Y-%m-%d %H:%M:%S)} {f} {L} {l} - {m}\n",
    ));
    let console = ConsoleAppender::builder()
        .encoder(log_pattern.clone())
        .target(Target::Stdout)
        .build();
    println!("log file:{:?}", log_file_path);
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
                .build(LevelFilter::Info),
        )
        .unwrap();
    let _handle = log4rs::init_config(config).expect("log init failed!!");
}
