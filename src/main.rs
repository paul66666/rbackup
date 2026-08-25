use std::fs::File;
use std::io::prelude::*;
use std::io::{stdin, stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use anyhow::Context;
use anyhow::{anyhow, bail};
use chrono::{Duration, Local, NaiveDateTime};
use clap::Parser;
use log::*;
use serde::Deserialize;

use zip::CompressionMethod;
use zip::{AesMode, write::SimpleFileOptions};

use walkdir::{DirEntry, WalkDir};

use crate::tracing::Tracing;

mod tracing;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Show Debug
    #[arg(short, long)]
    debug: bool,

    #[arg(short, long, default_value = "rbackup.toml")]
    config: PathBuf,

    #[arg(long, default_value_t = 40)]
    days: u32,

    #[arg(long, default_value_t = false)]
    full: bool,
}

#[derive(Debug, Deserialize)]
struct Cfg {
    sources: Vec<SourceCfg>,
    output: OutputCfg,
}
#[derive(Debug, Deserialize)]
struct SourceCfg {
    name: String,
    directory: PathBuf,
    skips: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OutputCfg {
    directory: PathBuf,
}

const APP_NAME: &str = match option_env!("CARGO_BIN_NAME") {
    Some(name) => name,
    None => "unknown",
};

struct App {
    _tracing: Tracing,
    args: Args,
    cfg: Cfg,
}

fn main() -> ExitCode {
    match (|| -> anyhow::Result<()> {
        let app = App::new()?;
        app.run()
    })() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

impl App {
    fn new() -> anyhow::Result<App> {
        let tracing = Tracing::new(APP_NAME);
        let args = Args::parse();
        if args.debug {
            tracing.enable_debug();
        } else {
            tracing.disable_debug();
        }

        let cfg = match std::fs::read_to_string(&args.config) {
            Ok(x) => match toml::from_str(&x) {
                Ok(x) => x,
                Err(e) => bail!("Unable to read config file '{:?}' : {e}", args.config),
            },
            Err(e) => bail!("Unable to parse config file '{:?}' : {e}", args.config),
        };

        Ok(App {
            _tracing: tracing,
            args,
            cfg,
        })
    }

    fn run(&self) -> anyhow::Result<()> {
        if !self.args.full {
            let ago: NaiveDateTime = (Local::now() - Duration::days(40)).naive_local();
            info!(
                "About to backup all files newer that {} days ({})",
                self.args.days, ago
            );
        }

        print!("What is the password : ");
        let _ = stdout().flush();
        let mut password = String::new();
        stdin()
            .read_line(&mut password)
            .expect("You did not enter a string");
        password = password.trim().to_string();
        println!("You want to use '{password}'. is that correct? (answer yes)");
        let mut yorn = String::new();
        stdin()
            .read_line(&mut yorn)
            .expect("You did not enter a string");
        yorn = yorn.trim().to_string();
        if yorn != "yes" {
            bail!("Aborting");
        } else {
            info!("Continuing");
        }

        info!("Started.");

        match self.doit(&password) {
            Ok(_) => (),
            Err(e) => error!("doit failed : {e}"),
        }

        info!("Finished");

        Ok(())
    }

    fn doit(&self, password: &str) -> anyhow::Result<()> {
        let today = Local::now();
        let archive_name = format!(
            "rbackup_{}_{}.zip",
            today.format("%Y-%m-%d"),
            if self.args.full { "FULL" } else { "INCR" }
        );

        let mut path = PathBuf::new();
        path.push(&self.cfg.output.directory);
        path.push(&archive_name);

        info!("Output : '{}'", path.display());

        let file = match File::create(&path) {
            Ok(x) => x,
            Err(e) => return Err(anyhow!("Unable to create file '{path:?}' : {e}")),
        };
        let mut zip = zip::ZipWriter::new(file);

        for source in &self.cfg.sources {
            if !Path::new(&source.directory).is_dir() {
                return Err(anyhow!(
                    "A path in sources, '{:?}', is not a directory or not found",
                    source.directory
                ));
            }

            let walkdir = WalkDir::new(&source.directory);
            let it = walkdir.into_iter();

            info!("");
            info!(
                "Backup {} as '{}' : Started",
                source.directory.display(),
                source.name
            );

            match self.zip_dir(&mut it.filter_map(|e| e.ok()), source, &mut zip, password) {
                Ok(_) => (),
                Err(e) => error!("{e}"),
            }

            info!(
                "Backup {} as '{}' : Finished",
                source.directory.display(),
                source.name
            );
            info!("");
        }

        zip.finish()?;

        info!("Done it");

        Ok(())
    }

    fn zip_dir<T>(
        &self,
        it: &mut dyn Iterator<Item = DirEntry>,
        source: &SourceCfg,
        zip: &mut zip::ZipWriter<T>,
        password: &str,
    ) -> anyhow::Result<()>
    where
        T: Write + Seek,
    {
        let source_name = &source.name;
        let prefix = &source.directory;
        let method = CompressionMethod::Deflated;
        let full = self.args.full;
        let days = self.args.days;
        let skips = &source.skips;

        let options = SimpleFileOptions::default()
            .compression_method(method)
            .with_aes_encryption(AesMode::Aes256, password);

        let prefix = Path::new(prefix);
        let now = Local::now();
        for entry in it {
            let path = entry.path();

            if let Some(skips) = skips {
                let p = format!("{}", path.display());
                let mut skip_file = false;
                for skip in skips {
                    if p.contains(skip) {
                        skip_file = true;
                        debug!("Skipped {p}");
                        break;
                    }
                }
                if skip_file {
                    continue;
                }
            }

            if !full {
                let metadata = match path.symlink_metadata() {
                    Ok(x) => x,
                    Err(e) => {
                        error!("{}: symlink_metadata() failed: {e}", path.display());
                        continue;
                    }
                };

                if metadata.is_symlink() {
                    info!("{} : Skipped (symlink)", path.display());
                    continue;
                }

                let mt = match metadata.modified() {
                    Ok(x) => x,
                    Err(e) => bail!(
                        "{}: symlink_metadata.modified() failed: {e}",
                        path.display()
                    ),
                };
                let seconds = mt.duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
                match chrono::DateTime::from_timestamp(seconds as i64, 0) {
                    Some(mt) => {
                        let duration = now.signed_duration_since(mt);
                        if duration.num_days() > days as i64 {
                            debug!(
                                "{} is too old at ({} days)",
                                path.display(),
                                duration.num_days()
                            );
                            continue;
                        }
                    }
                    None => {
                        error!(
                            "Unable to convert {seconds} seconds to DateTime : backing up {} anyway",
                            path.display()
                        );
                    }
                }
            }
            let name = path.strip_prefix(prefix).unwrap();
            let mut name2 = PathBuf::new();
            name2.push(source_name);
            for x in name.components() {
                name2.push(x);
            }
            let name = name2;
            let path_as_string = name
                .to_str()
                .map(str::to_owned)
                .with_context(|| format!("{name:?} Is a Non UTF-8 Path"))?;

            // Write file or directory explicitly
            // Some unzip tools unzip files with directory paths correctly, some do not!
            if path.is_file() {
                if log_enabled!(Level::Debug) {
                    debug!("zip: {name:?} (from {path:?})");
                }

                let mut size: usize = 0;
                zip.start_file_from_path(name, options)?;
                match File::open(path) {
                    Ok(mut f) => {
                        let mut buf = [0; 10_000];
                        loop {
                            let n = f.read(&mut buf[..])?;
                            size += n;
                            if n == 0 {
                                break;
                            }
                            zip.write_all(&buf[0..n])?;
                        }
                    }
                    Err(e) => {
                        error!("Problem with '{path:?} : {e}");
                    }
                }
                info!("{} : ({size} bytes)", path.display());
            } else if !name.as_os_str().is_empty() {
                // Only if not root! Avoids path spec / warning
                // and mapname conversion failed error on unzip
                debug!("zip: {path_as_string:?}");
                zip.add_directory_from_path(path_as_string, options)?;
                info!("{} : directory", path.display());
            }
        }
        Ok(())
    }
}
