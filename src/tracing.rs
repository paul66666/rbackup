use time::macros::format_description;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::time::LocalTime;
use tracing_subscriber::reload::Handle;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt, reload};

pub struct Tracing {
    _guard: WorkerGuard,
    filter_handle: Handle<EnvFilter, Registry>,
}

impl Tracing {
    pub fn new(app_name: &str) -> Tracing {
        let file = tracing_appender::rolling::daily("log", app_name);
        let (file, _guard) = tracing_appender::non_blocking(file);
        let (filter_layer, filter_handle) = reload::Layer::new(EnvFilter::new("info"));

        let console_timer = LocalTime::new(format_description!(
            "[hour]:[minute]:[second].[subsecond digits:3]"
        ));
        let file_timer = LocalTime::new(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]|"
        ));

        Registry::default()
            .with(filter_layer)
            .with(fmt::layer().with_timer(console_timer).with_target(false))
            .with(
                fmt::layer()
                    .with_writer(file)
                    .with_ansi(false)
                    .with_timer(file_timer),
            )
            .init();
        Tracing {
            _guard,
            filter_handle,
        }
    }

    pub fn enable_debug(&self) {
        self.filter_handle.reload(EnvFilter::new("debug")).unwrap();
    }
    pub fn disable_debug(&self) {
        self.filter_handle.reload(EnvFilter::new("info")).unwrap();
    }
}
