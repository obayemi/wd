//! # wd - A fast directory switcher
//!
//! `wd` (warp directory) is a command-line tool that helps you quickly navigate to frequently used directories.
//! It uses fuzzy string matching to find directories based on partial names and maintains a database of visited paths.
//!
//! ## Features
//! - Fuzzy directory matching using Damerau-Levenshtein distance
//! - Automatic path history tracking
//! - Case-insensitive matching
//! - Configurable confidence threshold
//! - Ability to forget paths

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

/// Database content structure that stores the list of visited paths
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct DBContent {
    paths: Vec<PathBuf>,
}

impl DBContent {
    /// Creates a new empty database content
    const fn new() -> Self {
        Self { paths: vec![] }
    }
}

/// Database wrapper that handles file operations and path management
#[derive(Debug, Clone)]
struct DB {
    file_path: String,
    content: DBContent,
}

impl DB {
    /// Opens a database from the specified path or the default location.
    /// If the file doesn't exist, creates an empty database.
    /// If the file is corrupted, starts with an empty database and prints a warning.
    fn open(db_path: Option<&str>) -> Result<Self, IOError> {
        let file_path = db_path.map_or_else(Self::default_db_path, std::string::ToString::to_string);

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(&file_path).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Warning: Failed to create database directory {}: {}", parent.display(), e);
            }
        }

        match File::open(file_path.clone()) {
            Ok(file) => {
                if let Ok(content) = serde_json::from_reader(BufReader::new(file)) {
                    Ok(Self { file_path, content })
                } else {
                    // If JSON is corrupted, start with empty database
                    eprintln!(
                        "Warning: Database file is corrupted, starting with empty database"
                    );
                    Ok(Self {
                        file_path,
                        content: DBContent::new(),
                    })
                }
            }
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

    /// Returns a slice of all stored paths
    fn paths(&self) -> &[PathBuf] {
        &self.content.paths
    }

    /// Writes the database content to disk
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

    /// Moves a path to the front of the list (most recently used).
    /// If the path doesn't exist in the database, it's added.
    fn bump(&mut self, path: &Path) -> &mut Self {
        let abspath: PathBuf = path.to_path_buf();
        self.content.paths.retain(|p| p != &abspath);
        self.content.paths.insert(0, abspath);
        self
    }

    /// Removes a path from the database
    fn forget(&mut self, path: &Path) -> &mut Self {
        self.content.paths.retain(|p| p != path);
        self
    }

    /// Returns the default database path (e.g., ~/.local/share/wd/wddb)
    fn default_db_path() -> String {
        let mut a = data_dir().unwrap_or_else(|| "/tmp/".into());
        a.push("wd/wddb");
        a.to_string_lossy().into()
    }
}

/// Result of a path completion attempt
struct CompleteResult {
    /// Confidence score (0.0 to 1.0)
    confidence: f64,
    /// The matched path
    path: PathBuf,
}

impl CompleteResult {
    const fn new(confidence: f64, path: PathBuf) -> Self {
        Self { confidence, path }
    }
}

/// Calculates a weight factor based on the index position.
/// More recent paths (lower indices) get higher weights.
fn weight(index: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    {
        1.2 - (0.4 / (1. + (index as f64 / -2.).exp()))
    }
}

/// Calculates the similarity distance between a path and a query string.
/// Uses normalized Damerau-Levenshtein distance.
/// Returns the best match among full path, basename, and case-insensitive basename.
fn dist(path: &Path, query: &str) -> eyre::Result<f64> {
    let path_str = path.to_str().ok_or_eyre("couldn't turn path to str")?;
    let basename = path.file_name().and_then(|s| s.to_str());

    let full_dist = normalized_damerau_levenshtein(path_str, query);
    let base_dist = basename
        .map_or(0., |n| normalized_damerau_levenshtein(n, query));
    let base_icase_dist = basename
        .map_or(0., |n| {
            normalized_damerau_levenshtein(&n.to_ascii_lowercase(), &query.to_ascii_lowercase())
        });

    Ok(full_dist.max(base_dist).max(base_icase_dist * 0.9))
}

/// Available subcommands for the wd tool
#[derive(Debug, Clone, Subcommand)]
pub enum Action {
    /// Complete a directory path using fuzzy matching
    Complete {
        /// The search query (partial directory name)
        input: String,

        /// Minimum confidence threshold for matches (0.0 to 1.0)
        #[clap(short = 'c', long = "confidence", default_value = "0.4")]
        confidence: f64,

        /// Number of results to return (if not specified, returns best match)
        #[clap(short = 'l', long = "list")]
        list: Option<usize>,
    },
    /// Remove a path from the database
    Forget {
        /// Path to forget (defaults to current directory)
        input: Option<String>,
    },
    /// Output shell setup functions and aliases
    Init {
        /// Shell type (bash, zsh, fish)
        #[clap(value_enum)]
        shell: ShellType,
    },
}

/// Supported shell types for initialization
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
}

/// Command line options for the wd tool
#[derive(Parser)]
#[clap(version=env!("CARGO_PKG_VERSION"), author = "obayemi")]
struct Opts {
    /// Path to the database file (defaults to ~/.local/share/wd/wddb)
    #[clap(long = "db")]
    db_path: Option<String>,

    /// Enable debug output
    #[clap(short = 'd', long = "debug")]
    debug: bool,

    /// The action to perform
    #[command(subcommand)]
    action: Action,
}

impl Opts {
    /// Performs directory completion based on the input query.
    /// Returns a list of matching paths sorted by relevance.
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
            db.bump(&input_path.canonicalize()?)
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
                db.bump(&item.path).write()?;
            }
        }
        if self.debug {
            #[allow(clippy::cast_precision_loss)]
            {
                println!("time: {:.2} ms", now.elapsed().as_micros() as f64 / 1000.);
            }
        }
        Ok(matches)
    }

    /// Removes a path from the database.
    /// If no input is provided, removes the current directory.
    fn forget(&self, input: Option<&str>) -> eyre::Result<()> {
        let mut db = DB::open(self.db_path.as_deref()).wrap_err("error loading wd db")?;

        let path = input.map_or_else(|| Path::new("."), Path::new);
        db.forget(&path.canonicalize().wrap_err("foo")?).write()?;

        db.write().wrap_err("error writing wd db")?;
        Ok(())
    }

    /// Outputs shell setup functions and aliases for the specified shell
    fn init(&self, shell: &ShellType) {
        match shell {
            ShellType::Bash | ShellType::Zsh => {
                println!(r#"function wd () {{
  local target
  target=$("${{WDBIN:-"wdbin"}}" complete "$@")
  if [ $? -eq 0 ]; then
    builtin cd "$target"
  fi
}}

function cd() {{
  # Handle special cases first
  if [ $# -eq 0 ]; then
    # cd with no arguments goes to HOME
    builtin cd "$HOME"
  elif [ "$1" = "-" ]; then
    # cd - goes to previous directory
    builtin cd -
  elif [ "$1" = "~" ]; then
    # cd ~ goes to HOME
    builtin cd "$HOME"
  else
    # Use wd for all other cases
    wd "$@"
  fi
}}"#);
            }
            ShellType::Fish => {
                println!(r#"# Add these functions to your Fish config (~/.config/fish/config.fish)
# or run: wdbin init fish >> ~/.config/fish/config.fish

function wd
  set target (wdbin complete $argv)

  if test "$status" -eq 0
    builtin cd "$target"
  end
end

function cd --description 'Change directory'
    set -l MAX_DIR_HIST 25

    if test (count $argv) -gt 1
        printf "%s\n" (_ "Too many args for cd command")
        return 1
    end

    # Skip history in subshells.
    if status --is-command-substitution
        builtin cd $argv
        return $status
    end

    # Avoid set completions.
    set -l previous $PWD

    if test "$argv" = "-"
        if test "$__fish_cd_direction" = "next"
            nextd
        else
            prevd
        end
        return $status
    end

    # allow explicit "cd ." if the mount-point became stale in the meantime
    if test "$argv" = "."
        cd "$PWD"
        return $status
    end

    if test (count $argv) -eq 0
      cd $HOME
      return $status
    end

    wd $argv
    set -l cd_status $status

    if test $cd_status -eq 0 -a "$PWD" != "$previous"
        set -q dirprev
        or set -l dirprev
        set -q dirprev[$MAX_DIR_HIST]
        and set -e dirprev[1]

        # If dirprev, dirnext, __fish_cd_direction
        # are set as universal variables, honor their scope.

        set -U -q dirprev
        and set -U -a dirprev $previous
        or set -g -a dirprev $previous

        set -U -q dirnext
        and set -U -e dirnext
        or set -e dirnext

        set -U -q __fish_cd_direction
        and set -U __fish_cd_direction prev
        or set -g __fish_cd_direction prev
    end

    return $cd_status
end"#);
            }
        }
    }
}

fn main() -> eyre::Result<()> {
    let opts: Opts = Opts::parse();

    match &opts.action {
        Action::Complete {
            input,
            confidence,
            list,
        } => {
            let matches = opts.complete(input, *confidence, *list)?;
            if matches.is_empty() {
                eprint!("no match found for {input}");
                std::process::exit(1);
            }
            for p in matches {
                if opts.debug {
                    println!("[{:.2}] {}", p.confidence, p.path.display());
                } else {
                    println!("{}", p.path.display());
                }
            }
        }
        Action::Forget { input } => {
            opts.forget(input.as_deref())?;
        }
        Action::Init { shell } => {
            opts.init(shell);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_db_open_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("nonexistent.db");

        let db = DB::open(Some(db_path.to_str().unwrap())).unwrap();
        assert_eq!(db.paths().len(), 0);
    }

    #[test]
    fn test_db_open_corrupted_file() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("corrupted.db");

        // Write corrupted JSON
        fs::write(&db_path, "{ invalid json }").unwrap();

        let db = DB::open(Some(db_path.to_str().unwrap())).unwrap();
        assert_eq!(db.paths().len(), 0);
    }

    #[test]
    fn test_db_open_various_corrupted_files() {
        let temp_dir = TempDir::new().unwrap();

        // Test various types of corrupted JSON
        let test_cases = vec![
            ("truncated.db", "{ \"paths\": ["),
            ("invalid_syntax.db", "not json at all"),
            ("wrong_structure.db", "{ \"wrong_field\": 123 }"),
            ("null.db", "null"),
            ("empty.db", ""),
        ];

        for (filename, content) in test_cases {
            let db_path = temp_dir.path().join(filename);
            fs::write(&db_path, content).unwrap();

            let db = DB::open(Some(db_path.to_str().unwrap())).unwrap();
            assert_eq!(db.paths().len(), 0, "Failed for {filename}");
        }
    }

    #[test]
    fn test_db_write_and_read() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let mut db = DB::open(Some(db_path.to_str().unwrap())).unwrap();
        db.bump(Path::new("/test/path1"));
        db.bump(Path::new("/test/path2"));
        db.write().unwrap();

        let db2 = DB::open(Some(db_path.to_str().unwrap())).unwrap();
        assert_eq!(db2.paths().len(), 2);
        assert_eq!(db2.paths()[0], PathBuf::from("/test/path2"));
        assert_eq!(db2.paths()[1], PathBuf::from("/test/path1"));
    }

    #[test]
    fn test_bump_reorders_paths() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let mut db = DB::open(Some(db_path.to_str().unwrap())).unwrap();
        db.bump(Path::new("/path1"));
        db.bump(Path::new("/path2"));
        db.bump(Path::new("/path3"));

        // Bump path1 again, should move to front
        db.bump(Path::new("/path1"));

        assert_eq!(db.paths()[0], PathBuf::from("/path1"));
        assert_eq!(db.paths()[1], PathBuf::from("/path3"));
        assert_eq!(db.paths()[2], PathBuf::from("/path2"));
    }

    #[test]
    fn test_forget_removes_path() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let mut db = DB::open(Some(db_path.to_str().unwrap())).unwrap();
        db.bump(Path::new("/path1"));
        db.bump(Path::new("/path2"));
        db.bump(Path::new("/path3"));

        db.forget(Path::new("/path2"));

        assert_eq!(db.paths().len(), 2);
        assert_eq!(db.paths()[0], PathBuf::from("/path3"));
        assert_eq!(db.paths()[1], PathBuf::from("/path1"));
    }

    #[test]
    fn test_weight_function() {
        assert!(weight(0) > weight(1));
        assert!(weight(1) > weight(2));
        assert!(weight(10) > weight(100));
    }

    #[test]
    fn test_dist_function() {
        let path = Path::new("/home/user/documents/project");

        // Exact match
        assert!((dist(path, "/home/user/documents/project").unwrap() - 1.0).abs() < f64::EPSILON);

        // Partial match
        assert!(dist(path, "project").unwrap() > 0.5);

        // Case insensitive basename match
        assert!(dist(path, "PROJECT").unwrap() > 0.5);
    }

    #[test]
    fn test_complete_with_existing_directory() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("test_dir");
        fs::create_dir(&test_dir).unwrap();

        let opts = Opts {
            db_path: Some(
                temp_dir
                    .path()
                    .join("test.db")
                    .to_str()
                    .unwrap()
                    .to_string(),
            ),
            debug: false,
            action: Action::Complete {
                input: test_dir.to_str().unwrap().to_string(),
                confidence: 0.4,
                list: None,
            },
        };

        let results = opts
            .complete(test_dir.to_str().unwrap(), 0.4, None)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_complete_with_pattern_matching() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let mut db = DB::open(Some(db_path.to_str().unwrap())).unwrap();
        db.bump(Path::new("/home/user/projects/rust-app"));
        db.bump(Path::new("/home/user/projects/python-app"));
        db.bump(Path::new("/home/user/documents/notes"));
        db.write().unwrap();

        let opts = Opts {
            db_path: Some(db_path.to_str().unwrap().to_string()),
            debug: false,
            action: Action::Complete {
                input: "rust".to_string(),
                confidence: 0.4,
                list: Some(2),
            },
        };

        let results = opts.complete("rust", 0.4, Some(2)).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].path.to_str().unwrap().contains("rust"));
    }

    #[test]
    fn test_default_db_path() {
        let path = DB::default_db_path();
        assert!(path.contains("wd/wddb"));
    }

    #[test]
    fn test_database_directory_creation() {
        let temp_dir = TempDir::new().unwrap();
        let nested_db_path = temp_dir.path().join("nested/dirs/test.db");
        
        // Ensure the nested directory doesn't exist initially
        assert!(!nested_db_path.parent().unwrap().exists());
        
        // Opening the database should create the directory
        let db = DB::open(Some(nested_db_path.to_str().unwrap())).unwrap();
        
        // Verify the parent directory was created
        assert!(nested_db_path.parent().unwrap().exists());
        
        // Verify we can write to the database
        db.write().unwrap();
        assert!(nested_db_path.exists());
    }

    #[test]
    fn test_database_directory_creation_with_existing_dir() {
        let temp_dir = TempDir::new().unwrap();
        let existing_dir = temp_dir.path().join("existing");
        fs::create_dir(&existing_dir).unwrap();
        let db_path = existing_dir.join("test.db");
        
        // Opening should work even if directory already exists
        let db = DB::open(Some(db_path.to_str().unwrap())).unwrap();
        db.write().unwrap();
        assert!(db_path.exists());
    }
}
