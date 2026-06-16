use log::*;

pub struct MyLog {
    config: Config,
}

pub struct Config {
    level: LevelFilter,
}
impl Config {
    fn new() -> Self {
        Self {
            level: LevelFilter::Info,
        }
    }
}

impl MyLog {
    pub fn new() -> Self {
        Self {
            config: Config::new(),
        }
    }

    pub fn set_level(mut self, level: LevelFilter) -> Self {
        self.config.level = level;
        self
    }

    pub fn init(self) {
        log::set_max_level(log::LevelFilter::Trace);
        match log::set_boxed_logger(Box::new(self)) {
            Ok(()) => (),
            Err(e) => {
                panic!("Unable to initialise logger : {e}")
            }
        }
    }
}

impl Log for MyLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.config.level
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let dt =
                chrono::Local::now().format("%H:%M:%S").to_string();
            let level = record.level().to_string();
            let target = record
                .target()
                .replace("aweb_server::", "")
                .replace("aweb_server", "main");
            let args = record.args().to_string();
            let line = format!("[{} {} {}] {}", dt, level, target, args);

            println!("{line}");
        }
    }
    fn flush(&self) {}
}
