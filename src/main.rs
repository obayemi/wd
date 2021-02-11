use clap::Clap;
use dirs::data_dir;
use serde::{Deserialize, Serialize};
use serde_json;
use std::f64::consts::E;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Error as IOError, ErrorKind};
use std::path::Path;
use std::process;
use std::time::Instant;
use strsim::normalized_damerau_levenshtein;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DB {
    paths: Vec<String>,
}

fn default_db_path() -> String {
    let mut a = data_dir().unwrap_or_else(|| "/tmp/".into());
    a.push("wd/wddb");
    //println!("{}", a.display());
    a.to_string_lossy().into()
}

impl DB {
    fn open(db_path: &str) -> Result<Self, IOError> {
        match File::open(db_path) {
            Ok(file) => Ok(serde_json::from_reader(BufReader::new(file)).unwrap()),
            Err(e) => {
                if e.kind() == ErrorKind::NotFound {
                    Ok(DB { paths: vec![] })
                } else {
                    Err(e)
                }
            }
        }
    }
    fn write(&self, db_path: &str) -> Result<(), IOError> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(db_path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, self).unwrap();
        Ok(())
    }
    fn bump<P: AsRef<Path>>(&self, path: &P) -> Self {
        let abspath = path.as_ref().canonicalize().unwrap();
        let abspath_str = abspath.to_string_lossy();
        if abspath_str == "/" {
            return self.clone();
        }
        let mut new = DB {
            paths: self
                .paths
                .iter()
                .filter(|p| p.as_ref() != abspath_str)
                .cloned()
                .collect(),
        };
        new.paths.insert(0, abspath_str.to_string());
        new
    }
}

#[derive(Clap)]
#[clap(version = "1.1", author = "obayemi")]
struct Opts {
    input: String,

    #[clap(short = 'c', long = "confidence", default_value = "0.4")]
    confidence: f64,

    #[clap(short = 'l', long = "list")]
    list: Option<usize>,

    #[clap(long = "db")]
    db_path: Option<String>,

    #[clap(short = 'd', long = "debug")]
    debug: bool,
}

fn weight(index: usize) -> f64 {
    1.2 - (0.4 / (1. + (E.powf(index as f64 / -2.))))
}

fn dist(path: &Path, query: &str) -> f64 {
    let path_str = path.to_str().unwrap();
    let basename = path.file_name().unwrap().to_str().unwrap();

    let full_dist = normalized_damerau_levenshtein(path_str, query);
    let base_dist = normalized_damerau_levenshtein(basename, query);
    let base_icase_dist =
        normalized_damerau_levenshtein(&basename.to_ascii_lowercase(), &query.to_ascii_lowercase());

    full_dist.max(base_dist).max(base_icase_dist * 0.9)
}

fn main() {
    let now = Instant::now();

    let opts: Opts = Opts::parse();

    let db_path = &opts
        .db_path
        .as_ref()
        .cloned()
        .unwrap_or_else(default_db_path);
    let db = DB::open(&db_path).expect("error loading wd db");

    let input_path = Path::new(&opts.input);
    if input_path.is_dir() {
        if opts.debug {
            println!("input is concrete path");
        }
        println!("{}", input_path.display());
        db.bump(&input_path)
            .write(&db_path)
            .expect("failed to write to db");
        return;
    }

    let mut paths: Vec<(f64, &Path)> = db
        .paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let path = Path::new(p);
            (dist(&path, &opts.input) * weight(i), path)
        })
        .filter(|(weight, _)| weight > &opts.confidence)
        .collect();

    if paths.is_empty() {
        if opts.debug {
            eprintln!("no results");
        }
        process::exit(1);
    }

    paths.sort_by(|(weight1, _), (weight2, _)| weight2.partial_cmp(weight1).unwrap());
    let its = paths.iter().take(opts.list.unwrap_or(1));
    its.for_each(|(weight, path)| {
        if opts.debug {
            println!("[{:.2}] {}", weight, path.to_str().unwrap())
        } else {
            println!("{}", path.to_str().unwrap())
        }
        if opts.list == None {
            db.bump(path)
                .write(&db_path)
                .expect("failed to write to db");
        }
    });
    if opts.debug {
        println!("time: {:.2} ms", now.elapsed().as_micros() as f64 / 1000.)
    }
}
