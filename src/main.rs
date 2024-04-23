#![deny(clippy::all)]
// #![warn(clippy::pedantic)]
#![warn(clippy::nursery)]

use clap::{Parser, Subcommand};
use dirs::data_dir;
use eyre::{Context, OptionExt};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Error as IOError, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::Instant;
use strsim::normalized_damerau_levenshtein;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct DBContent {
    paths: Vec<PathBuf>,
}
impl DBContent {
    const fn new() -> Self {
        Self { paths: vec![] }
    }
}
#[derive(Debug, Clone)]
struct DB {
    file_path: String,
    content: DBContent,
}

impl DB {
    fn open(db_path: Option<&str>) -> Result<Self, IOError> {
        let file_path = db_path
            .map(|p| p.to_string())
            .unwrap_or_else(Self::default_db_path);

        match File::open(file_path.clone()) {
            Ok(file) => Ok(Self {
                file_path,
                content: serde_json::from_reader(BufReader::new(file))?,
            }),
            Err(e) => {
                if e.kind() == ErrorKind::NotFound {
                    Ok(Self {
                        file_path,
                        content: DBContent::new(),
                    })
                } else {
                    Err(e)
                }
            }
        }
    }

    fn paths(&self) -> &[PathBuf] {
        &self.content.paths
    }

    fn write(&self) -> Result<(), IOError> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.file_path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, &self.content)?;
        Ok(())
    }

    fn bump(&mut self, path: PathBuf) -> Result<&mut Self, IOError> {
        let abspath: PathBuf = (*path).into();
        self.content.paths.retain(|p| p != &abspath);
        self.content.paths.insert(0, abspath);
        Ok(self)
    }

    fn forget(&mut self, path: PathBuf) -> Result<&mut Self, IOError> {
        self.content.paths.retain(|p| p != &path);
        Ok(self)
    }

    fn default_db_path() -> String {
        let mut a = data_dir().unwrap_or_else(|| "/tmp/".into());
        a.push("wd/wddb");
        a.to_string_lossy().into()
    }
}

struct CompleteResult {
    confidence: f64,
    path: PathBuf,
}

impl CompleteResult {
    const fn new(confidence: f64, path: PathBuf) -> Self {
        Self { confidence, path }
    }
}

fn weight(index: usize) -> f64 {
    1.2 - (0.4 / (1. + (index as f64 / -2.).exp()))
}

fn dist(path: &Path, query: &str) -> eyre::Result<f64> {
    let path_str = path.to_str().ok_or_eyre("couldn't turn path to str")?;
    let basename = path.file_name().and_then(|s| s.to_str());

    let full_dist = normalized_damerau_levenshtein(path_str, query);
    let base_dist = basename
        .map(|n| normalized_damerau_levenshtein(n, query))
        .unwrap_or(0.);
    let base_icase_dist = basename
        .map(|n| {
            normalized_damerau_levenshtein(&n.to_ascii_lowercase(), &query.to_ascii_lowercase())
        })
        .unwrap_or(0.);

    Ok(full_dist.max(base_dist).max(base_icase_dist * 0.9))
}

#[derive(Debug, Clone, Subcommand)]
pub enum Action {
    Complete {
        input: String,

        #[clap(short = 'c', long = "confidence", default_value = "0.4")]
        confidence: f64,

        #[clap(short = 'l', long = "list")]
        list: Option<usize>,
    },
    Forget {
        input: Option<String>,
    },
    // TODO: Init,
}

#[derive(Parser)]
#[clap(version=env!("CARGO_PKG_VERSION"), author = "obayemi")]
struct Opts {
    #[clap(long = "db")]
    db_path: Option<String>,

    #[clap(short = 'd', long = "debug")]
    debug: bool,

    #[command(subcommand)]
    action: Action,
}

impl Opts {
    fn complete(
        &self,
        input: &str,
        min_confidence: f64,
        list: Option<usize>,
    ) -> eyre::Result<Vec<CompleteResult>> {
        let mut db = DB::open(self.db_path.as_deref()).wrap_err("error loading wd db")?;

        let now = Instant::now();
        let input_path = Path::new(input);
        if input_path.is_dir() {
            if self.debug {
                println!("input is concrete path");
            }
            db.bump(input_path.canonicalize()?)?
                .write()
                .expect("failed to write to db");
            return Ok(vec![CompleteResult::new(1.0, input_path.canonicalize()?)]);
        }

        let mut paths: Vec<(f64, &PathBuf)> = db
            .paths()
            .iter()
            .enumerate()
            .map(|(i, path)| (dist(path, input).unwrap() * weight(i), path))
            .filter(|(confidence, _)| *confidence > min_confidence)
            .collect();

        if paths.is_empty() {
            return Ok(vec![]);
        }

        paths.sort_by(|(weight1, _), (weight2, _)| weight2.partial_cmp(weight1).unwrap());
        let matches: Vec<_> = paths
            .into_iter()
            .map(|(confidence, path)| CompleteResult::new(confidence, path.clone()))
            .take(list.unwrap_or(1))
            .collect();

        if list.is_none() {
            if let Some(item) = matches.first() {
                db.bump(item.path.clone())?.write()?;
            }
        }
        if self.debug {
            println!("time: {:.2} ms", now.elapsed().as_micros() as f64 / 1000.)
        }
        Ok(matches)
    }

    fn forget(&self, input: Option<&str>) -> eyre::Result<()> {
        let mut db = DB::open(self.db_path.as_deref()).wrap_err("error loading wd db")?;

        let path = input.map(Path::new).unwrap_or_else(|| Path::new("."));
        db.forget(path.canonicalize().wrap_err("foo")?)?.write()?;

        db.write().wrap_err("error writing wd db")?;
        Ok(())
    }
}

fn main() {
    let opts: Opts = Opts::parse();

    match &opts.action {
        Action::Complete {
            input,
            confidence,
            list,
        } => {
            for p in opts.complete(input, *confidence, *list).unwrap() {
                if opts.debug {
                    println!("[{:.2}] {}", p.confidence, p.path.display());
                } else {
                    println!("{}", p.path.display());
                }
            }
        }
        Action::Forget { input } => {
            opts.forget(input.as_deref()).unwrap();
        }
    }
}
