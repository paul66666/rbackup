use log::*;
use clap::Parser;
use serde::Deserialize;
use zip::CompressionMethod;
use zip::{AesMode, write::SimpleFileOptions};

use anyhow::{anyhow, bail};
use anyhow::Context;
use std::io::prelude::*;
use std::io::{stdout,stdin};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use chrono::Local;
use walkdir::{DirEntry, WalkDir};

mod my_log;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Show Debug
    #[arg(short, long)]
    debug: bool,

    #[arg(short, long)]
    config: PathBuf,

    #[arg(long, default_value_t = 40)]
    days: u32,

    #[arg(long, default_value_t = false)]
    full: bool,
}

#[derive(Debug, Deserialize)]
struct Config {
    sources: Vec<SourceConfig>,
    output: OutputConfig,
}
#[derive(Debug, Deserialize)]
struct SourceConfig {
    name: String,
    directory: PathBuf,
    skips: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OutputConfig {
    directory: PathBuf,
}

fn main() {
    let args = Args::parse();
    let level = if args.debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    my_log::MyLog::new().set_level(level).init();

    print!("What is the password : ");
    let _ = stdout().flush();
    let mut password = String::new();
    stdin().read_line(&mut password).expect("You did not enter a string");
    password = password.trim().to_string();
    println!("You want to use '{password}'. is that correct? (answer yes)");
    let mut yorn = String::new();
    stdin().read_line(&mut yorn).expect("You did not enter a string");
    yorn = yorn.trim().to_string();
    if yorn != "yes" {
        error!("Aborting");
        return;
    } else {
        info!("Continuing");
    }

    info!("Started.");

    let s = match std::fs::read_to_string(&args.config) {
        Ok(x) => x,
        Err(e) => panic!("Unable to read config file '{:?}' : {e}", &args.config)
    };

    let cfg: Config = match toml::from_str(&s) {
        Ok(x) => x,
        Err(e) => panic!("Unable to create config from file '{:?}' : {e}", &args.config),
    };

    match doit(&cfg, &args, &password) {
        Ok(_) => (),
        Err(e) => error!("doit failed : {e}"),
    }

    info!("Finished");
}

fn doit(cfg: &Config, args: &Args, password: &str) -> anyhow::Result<()> {
    let today = Local::now();
    let archive_name = format!("rbackup_{}_{}.zip",
        today.format("%Y-%m-%d"),
        if args.full { "FULL"} else { "INCR"}
    );

    let mut path = PathBuf::new();
    path.push(&cfg.output.directory);
    path.push(&archive_name);

    info!("Zip Path is '{}'", &path.display());

    let file = match File::create(&path) {
        Ok(x) => x,
        Err(e) => return Err(anyhow!("Unable to create file '{path:?}' : {e}"))
    };
    let mut zip = zip::ZipWriter::new(file);

    for source in &cfg.sources {
        if !Path::new(&source.directory).is_dir() {
            return Err(anyhow!("A path in sources, '{:?}', is not a directory or not found", source.directory));
        }

        let walkdir = WalkDir::new(&source.directory);
        let it = walkdir.into_iter();

        info!("Backup {} Started", source.name);

        match zip_dir(&mut it.filter_map(|e| e.ok()), source, &mut zip, args, password) {
	    Ok(_) => (),
	    Err(e) => error!("{e}"),
	}		

        info!("Backup {} Finished", source.name);
        info!("");
    }

    zip.finish()?;

    info!("Done it");

    Ok(())
}

fn zip_dir<T>(
    it: &mut dyn Iterator<Item = DirEntry>,
    source: &SourceConfig,
    zip: &mut zip::ZipWriter<T>,
    args: &Args,
    password: &str,
) -> anyhow::Result<()>
where
    T: Write + Seek,
{
    let source_name = &source.name;
    let prefix = &source.directory;
    let method = CompressionMethod::Deflated;
    let full = args.full;
    let days = args.days;
    let skips = &source.skips;

    let options = SimpleFileOptions::default()
        .compression_method(method)
        .with_aes_encryption(AesMode::Aes256, password);
        //.unix_permissions(0o755);

    let prefix = Path::new(prefix);
    //let mut buffer = Vec::new();
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
            if skip_file { continue; }
        }
	info!("path={}", path.display());
        if !full {
            let metadata = match path.metadata() {
		Ok(x) => x,
		Err(e) => {
                    error!("{}: metadata() failed: {e}", path.display());
                    continue;
                }
	    };
            let mt = match metadata.modified() {
		Ok(x) => x,
		Err(e) => bail!("{}: metadata.modified() failed: {e}", path.display())
	    };
            let seconds = mt.duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
            match chrono::DateTime::from_timestamp(seconds as i64, 0) {
                Some(mt) => {
                    let duration = now.signed_duration_since(mt);
                    if duration.num_days() > days as i64 {
                        debug!("{} is too old ({} days)", path.display(), duration.num_days());
                        continue;
                    }
                },
                None => {
                    error!("Unable to convert {seconds} seconds to DateTime : backing up {} anyway", path.display());
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
            } else {
                info!("zip: {name:?}");
            }

            zip.start_file_from_path(name, options)?;
            match File::open(path) {
                Ok(mut f) => {
                    let mut buf = [0;10_000];
                    loop {
                        let n = f.read(&mut buf[..])?;
                        if n == 0 { break; }
                        zip.write_all(&buf[0..n])?;
                    }
                    /*
                    f.read_to_end(&mut buffer)?;
                    zip.write_all(&buffer)?;
                    */
                },
                Err(e) => {
                    error!("Problem with '{path:?} : {e}");
                }
            }

            //buffer.clear();
        } else if !name.as_os_str().is_empty() {
            // Only if not root! Avoids path spec / warning
            // and mapname conversion failed error on unzip
            info!("zip: {path_as_string:?}");
            zip.add_directory_from_path(path_as_string, options)?;
        }
    }
    Ok(())
}

