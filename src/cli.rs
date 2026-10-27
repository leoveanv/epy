use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Parser;

use crate::book::LibraryEntry;
use crate::book::{Book, BookLocation};
use crate::configuration::{AppPaths, Configuration, PathEnvironment};
use crate::storage::Store;
use crate::{Error, Result};

#[derive(Parser)]
#[command(
    name = "ers",
    version,
    about = "Read ebooks in the terminal",
    disable_version_flag = true
)]
pub struct Arguments {
    #[arg(short = 'r', long, help = "Print reading history")]
    pub history: bool,
    #[arg(short = 'd', long, help = "Dump ebook text to standard output")]
    pub dump: bool,
    #[arg(short = 'v', long, action = clap::ArgAction::Version)]
    pub version: Option<bool>,
    #[arg(value_name = "PATH | # | PATTERN | URL", num_args = 0.., allow_negative_numbers = true)]
    pub ebook: Vec<String>,
}

pub struct RuntimeEnvironment {
    pub paths: PathEnvironment,
}

impl RuntimeEnvironment {
    pub fn capture() -> Self {
        Self {
            paths: PathEnvironment {
                home: std::env::var_os("HOME").map(PathBuf::from),
                config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
                app_data: std::env::var_os("APPDATA").map(PathBuf::from),
            },
        }
    }
}

pub fn run(arguments: Arguments, environment: RuntimeEnvironment) -> Result<()> {
    if arguments.dump
        && arguments.ebook.len() == 1
        && (Path::new(&arguments.ebook[0]).is_file() || arguments.ebook[0].contains("://"))
    {
        return dump_book(
            BookLocation::parse(&arguments.ebook[0])?,
            &mut std::io::stdout().lock(),
        );
    }
    let paths = AppPaths::discover(&environment.paths)?;
    std::fs::create_dir_all(&paths.directory)?;
    let store = Store::open(&paths.database)?;
    let history = store.history()?;
    if arguments.history {
        return print_history(&history, &mut std::io::stdout().lock());
    }
    let path = resolve_target(&arguments.ebook, &history)?;
    drop(store);
    if arguments.dump {
        return dump_book(path, &mut std::io::stdout().lock());
    }
    let configuration = Configuration::load(&paths.configuration)?;
    crate::terminal::read_book(path, paths.database, configuration)
}

pub fn resolve_target(arguments: &[String], history: &[LibraryEntry]) -> Result<BookLocation> {
    if arguments.is_empty() {
        return history
            .first()
            .map(|entry| BookLocation::parse(&entry.filepath))
            .ok_or_else(|| Error::Compatibility("Found no last read ebook file.".into()))?;
    }
    if arguments.len() == 1 {
        if Path::new(&arguments[0]).is_file() {
            return Ok(BookLocation::local(&arguments[0]));
        }
        if arguments[0].contains("://") {
            return BookLocation::parse(&arguments[0]);
        }
        if let Ok(number) = arguments[0].parse::<i64>() {
            return history_index(number, history.len())
                .and_then(|index| history.get(index))
                .map(|entry| BookLocation::parse(&entry.filepath))
                .ok_or_else(|| Error::Compatibility(format!("#{number} file not found.")))?;
        }
    }
    let pattern = arguments.join(" ").to_lowercase();
    if pattern.is_empty() {
        return Err(Error::Compatibility(
            "Provide a nonempty ebook path or history pattern.".into(),
        ));
    }
    let mut best = None;
    let mut best_score = 0;
    for entry in history {
        let title = entry.title.as_deref().unwrap_or("None");
        let author = entry.author.as_deref().unwrap_or("None");
        let score = sequence_match_count(&format!("{title} - {author}").to_lowercase(), &pattern);
        if best.is_none() || score > best_score {
            best = Some(entry);
            best_score = score;
        }
    }
    best.filter(|_| best_score * 2 >= pattern.chars().count())
        .map(|entry| BookLocation::parse(&entry.filepath))
        .ok_or_else(|| Error::Compatibility("Found no matching ebook from history.".into()))?
}

fn history_index(number: i64, length: usize) -> Option<usize> {
    if number > 0 {
        usize::try_from(number - 1).ok()
    } else {
        usize::try_from(number.unsigned_abs().checked_add(1)?)
            .ok()
            .and_then(|distance| length.checked_sub(distance))
    }
}

fn print_history(history: &[LibraryEntry], output: &mut impl Write) -> Result<()> {
    if history.is_empty() {
        writeln!(output, "No Reading History.")?;
        return Ok(());
    }
    writeln!(output, "Reading History:")?;
    for (index, entry) in history.iter().enumerate() {
        let progress = entry.reading_progress.map_or_else(
            || "N/A".into(),
            |progress| format!("{}%", (progress * 100.0) as usize),
        );
        writeln!(
            output,
            "{} {:>4} {}: {} - {} ({})",
            index + 1,
            progress,
            entry.last_read,
            entry.title.as_deref().unwrap_or(""),
            entry.author.as_deref().unwrap_or(""),
            entry.filepath
        )?;
    }
    Ok(())
}

fn dump_book(location: BookLocation, output: &mut impl Write) -> Result<()> {
    let mut book = Book::open(location)?;
    for content_id in book.spine_contents().to_vec() {
        for paragraph in
            crate::document::text::parse_document(&content_id, &book.raw_text(&content_id)?)?
                .paragraphs
        {
            writeln!(output, "{}\n", paragraph.raw_text)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct MatchRegion {
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
}

struct MatchingBlock {
    left_start: usize,
    right_start: usize,
    size: usize,
}

fn longest_match(
    left: &[char],
    right: &[char],
    indices: &BTreeMap<char, Vec<usize>>,
    region: MatchRegion,
) -> MatchingBlock {
    let mut best = MatchingBlock {
        left_start: region.left_start,
        right_start: region.right_start,
        size: 0,
    };
    let mut previous = BTreeMap::new();
    for (index, character) in left
        .iter()
        .enumerate()
        .take(region.left_end)
        .skip(region.left_start)
    {
        let mut current = BTreeMap::new();
        for right_index in indices
            .get(character)
            .into_iter()
            .flatten()
            .copied()
            .filter(|index| *index >= region.right_start && *index < region.right_end)
        {
            let size = right_index
                .checked_sub(1)
                .and_then(|index| previous.get(&index).copied())
                .unwrap_or(0)
                + 1;
            current.insert(right_index, size);
            if size > best.size {
                best = MatchingBlock {
                    left_start: index + 1 - size,
                    right_start: right_index + 1 - size,
                    size,
                };
            }
        }
        previous = current;
    }
    while best.left_start > region.left_start
        && best.right_start > region.right_start
        && left[best.left_start - 1] == right[best.right_start - 1]
    {
        best.left_start -= 1;
        best.right_start -= 1;
        best.size += 1;
    }
    while best.left_start + best.size < region.left_end
        && best.right_start + best.size < region.right_end
        && left[best.left_start + best.size] == right[best.right_start + best.size]
    {
        best.size += 1;
    }
    best
}

fn sequence_match_count(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut indices = BTreeMap::<char, Vec<usize>>::new();
    for (index, character) in right.iter().enumerate() {
        indices.entry(*character).or_default().push(index);
    }
    if right.len() >= 200 {
        indices.retain(|_, indices| indices.len() <= right.len() / 100 + 1);
    }
    let mut regions = vec![MatchRegion {
        left_start: 0,
        left_end: left.len(),
        right_start: 0,
        right_end: right.len(),
    }];
    let mut count = 0;
    while let Some(region) = regions.pop() {
        let best = longest_match(&left, &right, &indices, region);
        if best.size == 0 {
            continue;
        }
        count += best.size;
        if region.left_start < best.left_start && region.right_start < best.right_start {
            regions.push(MatchRegion {
                left_end: best.left_start,
                right_end: best.right_start,
                ..region
            });
        }
        if best.left_start + best.size < region.left_end
            && best.right_start + best.size < region.right_end
        {
            regions.push(MatchRegion {
                left_start: best.left_start + best.size,
                right_start: best.right_start + best.size,
                ..region
            });
        }
    }
    count
}
