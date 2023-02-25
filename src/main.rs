use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use glob::glob;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use thiserror::Error;

use ncmdump::{Ncmdump, QmcDump};

enum FileType {
    Ncm,
    Qmc,
    Other,
}

#[derive(Clone, Debug, Error)]
enum Error {
    #[error("Can't resolve the path")]
    PathError,
    #[error("Invalid file format")]
    FormatError,
}

#[derive(Debug, Parser)]
#[command(name = "ncmdump", bin_name = "ncmdump", about, version)]
struct Command {
    /// Specified the targets to convert.
    #[arg(value_name = "TARGETS")]
    matchers: Vec<String>,

    /// Specified the output directory.
    /// Default it's the same directory with input file.
    #[arg(short = 'o', long = "output")]
    output: Option<String>,

    /// Verbosely list files processing.
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Recursively find files that need to be converted.
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,
}

struct Wrapper {
    name: String,
    format: FileType,
    path: PathBuf,
    size: u64,
}

impl Wrapper {
    fn from_path(path: PathBuf) -> Result<Self> {
        let mut file = File::open(&path)?;
        let mut head = [0; 8];
        let format = if file.read(&mut head)? == 8 {
            match head[..] {
                [0x43, 0x54, 0x45, 0x4E, 0x46, 0x44, 0x41, 0x4D] => FileType::Ncm,
                [0xA5, 0x06, 0xB7, 0x89, _, _, _, _] => FileType::Qmc,
                [0x8A, 0x0E, 0xE5, _, _, _, _, _] => FileType::Qmc,
                _ => FileType::Other,
            }
        } else {
            FileType::Other
        };

        Ok(Wrapper {
            name: path.file_name().unwrap().to_str().unwrap().to_string(),
            format,
            path,
            size: file.metadata().unwrap().len(),
        })
    }
}

struct NcmdumpCli {
    command: Command,
}

impl NcmdumpCli {
    fn from_command(command: Command) -> Self {
        NcmdumpCli { command }
    }

    fn get_output(
        &self,
        file_path: &Path,
        format: &str,
        output: &Option<String>,
    ) -> Result<PathBuf> {
        let parent = match output {
            None => file_path.parent().ok_or(Error::PathError)?,
            Some(p) => Path::new(p),
        };
        let file_name = file_path.file_stem().ok_or(Error::PathError)?;
        let path = parent.join(file_name).with_extension(format);
        Ok(path)
    }

    fn get_subfile(&self, dir: PathBuf) -> Result<Vec<PathBuf>> {
        let mut result = Vec::new();
        if dir.is_dir() {
            for entry in dir.read_dir()? {
                let path = entry?.path();
                if path.is_file() {
                    result.push(path);
                } else if path.is_dir() && self.command.recursive {
                    result.extend(self.get_subfile(path)?);
                }
            }
        }
        Ok(result)
    }

    fn get_paths(&self) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for matcher in &self.command.matchers {
            for entry in glob(matcher.as_str())? {
                match entry {
                    Ok(path) => {
                        if path.is_file() {
                            paths.push(path);
                        } else if path.is_dir() {
                            paths.extend(self.get_subfile(path)?);
                        }
                    }
                    Err(e) => println!("{:?}", e),
                }
            }
        }
        Ok(paths)
    }

    fn dump(&self, item: &Wrapper, progress: &ProgressBar) -> Result<()> {
        let mut data = Vec::new();
        let file = File::open(&item.path)?;
        let mut buffer = [0; 1024];
        match item.format {
            FileType::Ncm => {
                let mut dump = Ncmdump::from_reader(file)?;
                while let Ok(size) = dump.read(&mut buffer) {
                    if size == 0 {
                        break;
                    }
                    data.write_all(&buffer[..size])?;
                    progress.inc(size as u64);
                }
            }
            FileType::Qmc => {
                let mut dump = QmcDump::from_reader(file)?;
                while let Ok(size) = dump.read(&mut buffer) {
                    if size == 0 {
                        break;
                    }
                    data.write_all(&buffer[..size])?;
                    progress.inc(size as u64);
                }
            }
            FileType::Other => return Ok(()),
        };
        let ext = match data[..4] {
            [0x66, 0x4C, 0x61, 0x43] => Ok("flac"),
            [0x49, 0x44, 0x33, _] => Ok("mp3"),
            _ => Err(Error::FormatError),
        }?;
        let output_file = self.get_output(&item.path, ext, &self.command.output)?;
        let mut target = File::options().create(true).write(true).open(output_file)?;
        target.write_all(&data)?;
        Ok(())
    }

    fn get_info(&self, paths: Vec<PathBuf>, progress: &ProgressBar) -> Vec<Wrapper> {
        let mut result = Vec::new();
        for path in paths {
            progress.set_message(path.file_name().unwrap().to_str().unwrap().to_string());
            let item = match Wrapper::from_path(path) {
                Ok(x) => x,
                Err(_) => {
                    progress.inc(1);
                    continue;
                }
            };
            match item.format {
                FileType::Other => {}
                _ => result.push(item),
            }
            progress.inc(1)
        }
        progress.finish();
        result
    }

    fn start(&self) -> Result<()> {
        let progress_style_run = ProgressStyle::with_template(
            "[{elapsed_precise:.blue}] [{bar:40.cyan/blue}] {pos:>10!.cyan}/{len:<10!.blue} | {percent:>3!}% | {msg}",
        )?
        .progress_chars("=>-");
        let progress_style_dump = ProgressStyle::with_template(
            "[{elapsed_precise:.blue}] [{bar:40.cyan/blue}] {bytes:>10!.cyan}/{total_bytes:<10!.blue} | {percent:>3!}% | {bytes_per_sec}",
        )?.progress_chars("=>-");

        let multi_progress = MultiProgress::new();
        let paths = self.get_paths()?;
        let progress_info = multi_progress
            .add(ProgressBar::new(paths.len() as u64))
            .with_style(progress_style_run.clone());
        let items = self.get_info(paths, &progress_info);

        if items.is_empty() {
        } else if items.len() == 1 {
            let item = items.get(0).unwrap();
            let progress =
                multi_progress.add(ProgressBar::new(item.size).with_style(progress_style_dump));
            self.dump(item, &progress)?;
            if self.command.verbose {
                progress.println(format!("Converting file {}\t complete!", item.name));
            }
            progress.finish();
        } else {
            let progress_run = multi_progress
                .add(ProgressBar::new(items.len() as u64).with_style(progress_style_run));
            let progress_dump =
                multi_progress.add(ProgressBar::new(1).with_style(progress_style_dump));
            for item in items {
                progress_run.set_message(item.name.clone());
                progress_dump.reset();
                progress_dump.set_length(item.size);
                match self.dump(&item, &progress_dump) {
                    Ok(_) => {
                        if self.command.verbose {
                            multi_progress
                                .println(format!("Converting file {}\t complete!", item.name))?;
                        }
                        progress_run.inc(1);
                        progress_dump.finish();
                    }
                    Err(e) => println!("{:?}", e),
                }
            }
            progress_run.finish();
        }

        Ok(())
    }
}

fn main() -> Result<()> {
    let app = NcmdumpCli::from_command(Command::parse());
    app.start()
}
