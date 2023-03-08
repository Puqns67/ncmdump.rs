use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use glob::glob;
use id3::frame::{Picture, PictureType};
use id3::{Tag, TagLike, Version};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use thiserror::Error;

#[cfg(feature = "ncmdump")]
use ncmdump::Ncmdump;
#[cfg(feature = "qmcdump")]
use ncmdump::QmcDump;

const PROGRESS_STYLE_RUN: &str = "[{elapsed_precise:.blue}] [{bar:40.cyan}] {pos:>10!.cyan}/{len:<10!.blue} | {percent:>3!}% | {msg}";
const PROGRESS_STYLE_DUMP: &str = "[{elapsed_precise:.blue}] [{bar:40.cyan}] {bytes:>10!.cyan}/{total_bytes:<10!.blue} | {percent:>3!}% | {bytes_per_sec}";
const PROGRESS_STYLE_BAR: &str = "=> ";
const MAX_RECURSIVE_DEPEH: u8 = 8;

enum FileType {
    #[cfg(feature = "ncmdump")]
    Ncm,
    #[cfg(feature = "qmcdump")]
    Qmc,
    Other,
}

#[derive(Clone, Debug, Error)]
enum Errors {
    #[error("Can't resolve the path")]
    InvalidPath,
    #[error("Invalid file format")]
    InvalidFormat,
    #[error("No file can be converted")]
    NoFileError,
}

#[derive(Debug, Parser)]
#[command(name = "ncmdump", bin_name = "ncmdump", about, version)]
struct Command {
    /// Specified the files to convert.
    #[arg(value_name = "FILE_MATCHERS")]
    matchers: Vec<String>,

    /// Specified the output directory.
    /// Default it's the same directory with input file.
    #[arg(short = 'o', long = "output")]
    output: Option<String>,

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
                #[cfg(feature = "ncmdump")]
                [0x43, 0x54, 0x45, 0x4E, 0x46, 0x44, 0x41, 0x4D] => FileType::Ncm,
                #[cfg(feature = "qmcdump")]
                [0xA5, 0x06, 0xB7, 0x89, _, _, _, _] => FileType::Qmc,
                #[cfg(feature = "qmcdump")]
                [0x8A, 0x0E, 0xE5, _, _, _, _, _] => FileType::Qmc,
                _ => FileType::Other,
            }
        } else {
            FileType::Other
        };

        Ok(Self {
            name: path.file_name().unwrap().to_str().unwrap().to_string(),
            format,
            path,
            size: file.metadata().unwrap().len(),
        })
    }
}

struct NcmdumpCli {
    command: Command,
    progress: MultiProgress,
}

impl NcmdumpCli {
    fn from_command(command: Command) -> Self {
        Self {
            command,
            progress: MultiProgress::new(),
        }
    }

    fn get_output(
        &self,
        file_path: &Path,
        format: &str,
        output: &Option<String>,
    ) -> Result<PathBuf> {
        let parent = match output {
            None => file_path.parent().ok_or(Errors::InvalidPath)?,
            Some(p) => Path::new(p),
        };
        let file_name = file_path.file_stem().ok_or(Errors::InvalidPath)?;
        let path = parent.join(file_name).with_extension(format);
        Ok(path)
    }

    fn get_subfile(&self, dir: PathBuf, depth: u8) -> Result<Vec<PathBuf>> {
        let mut result = Vec::new();
        if dir.is_dir() {
            for entry in dir.read_dir()? {
                let path = entry?.path();
                if path.is_file() {
                    result.push(path);
                } else if path.is_dir() && self.command.recursive {
                    if depth < MAX_RECURSIVE_DEPEH {
                        result.extend(self.get_subfile(path, depth + 1)?);
                    } else {
                        self.progress
                            .println("Folder nesting layers are too deep, skipping")?;
                    }
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
                            paths.extend(self.get_subfile(path, 0)?);
                        }
                    }
                    Err(e) => println!("{:?}", e),
                }
            }
        }
        Ok(paths)
    }

    fn get_info(&self, paths: Vec<PathBuf>) -> Vec<Wrapper> {
        let mut result = Vec::new();
        for path in paths {
            if let Ok(item) = Wrapper::from_path(path) {
                match item.format {
                    FileType::Other => {}
                    _ => result.push(item),
                }
            };
        }
        result
    }

    fn get_data(&self, mut dump: impl Read, progress: &ProgressBar) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let mut buffer = [0; 1024];
        while let Ok(size) = dump.read(&mut buffer) {
            if size == 0 {
                break;
            }
            data.write_all(&buffer[..size])?;
            progress.inc(size as u64);
        }
        progress.finish();
        Ok(data)
    }

    fn dump(&self, item: &Wrapper, progress: &ProgressBar) -> Result<()> {
        let file = File::open(&item.path)?;
        let data = match item.format {
            #[cfg(feature = "ncmdump")]
            FileType::Ncm => self.get_data(Ncmdump::from_reader(file)?, progress),
            #[cfg(feature = "qmcdump")]
            FileType::Qmc => self.get_data(QmcDump::from_reader(file)?, progress),
            FileType::Other => Err(Errors::InvalidFormat.into()),
        }?;
        let ext = match data[..4] {
            [0x66, 0x4C, 0x61, 0x43] => Ok("flac"),
            [0x49, 0x44, 0x33, _] => Ok("mp3"),
            _ => Err(Errors::InvalidFormat),
        }?;
        let output_file = self.get_output(&item.path, ext, &self.command.output)?;
        let mut target = File::options().create(true).write(true).open(output_file)?;
        target.write_all(&data)?;
        if let FileType::Ncm = item.format {
            let mut reader = Ncmdump::from_reader(File::open(&item.path)?)?;
            let mut tag = Tag::new();
            if let Ok(info) = reader.get_info() {
                tag.set_title(info.name);
                tag.set_artist(
                    info.artist
                        .iter()
                        .map(|(i, _)| i.to_owned())
                        .collect::<Vec<String>>()
                        .join(","),
                );
                tag.set_album(info.album);
                tag.set_duration(info.duration as u32);
            }
            if let Ok(image) = reader.get_image() {
                tag.add_frame(Picture {
                    mime_type: "image/png".to_string(),
                    picture_type: PictureType::Other,
                    description: "some other image".to_string(),
                    data: image,
                });
            }
            tag.write_to(&mut target, Version::Id3v24)?;
        }
        Ok(())
    }

    fn start(&self) -> Result<()> {
        if self.command.matchers.is_empty() {
            return Err(Errors::NoFileError.into());
        }
        let paths = self.get_paths()?;
        let items = self.get_info(paths);
        if items.is_empty() {
            return Err(Errors::NoFileError.into());
        }

        let progress_style_run = ProgressStyle::with_template(PROGRESS_STYLE_RUN)?;
        let progress_style_dump = ProgressStyle::with_template(PROGRESS_STYLE_DUMP)?;

        let progress_run = self
            .progress
            .add(ProgressBar::new(items.len() as u64).with_style(progress_style_run));

        for item in items {
            let current = self.progress.insert(
                0,
                ProgressBar::new(item.size).with_style(progress_style_dump.clone()),
            );
            current.set_message(item.name.clone());
            match self.dump(&item, &current) {
                Ok(_) => progress_run.inc(1),
                Err(e) => println!("{:?}", e),
            }
        }
        progress_run.finish();

        Ok(())
    }
}

fn main() -> Result<()> {
    NcmdumpCli::from_command(Command::parse()).start()
}
