use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;
use std::thread;

use anyhow::Result;
use clap::Parser;
use glob::glob;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use ncmdump::{tag::Tag, utils::FileType};
use ncmdump::{Ncmdump, Qmcdump};

mod errors;
mod provider;

use errors::{DumpError, Error};
use provider::{DataProvider, FileProvider};

const PBCHARS: &str = "=> ";
const PBSTYLE_SINGLE: &str =
    "{wide_msg:!} {bytes} {bytes_per_sec} {eta} [{bar:36.cyan}] {percent:>3}%";
const PBSTYLE_TOTAL: &str =
    "{spinner} [{elapsed_precise}] [{wide_bar:.cyan}] {bytes}/{total_bytes} {bytes_per_sec}";

#[derive(Clone, Debug, Default, Parser)]
#[command(name = "ncmdump", bin_name = "ncmdump", about, version)]
struct Command {
    /// Specified the files to convert.
    #[arg(value_name = "FILES")]
    matchers: Vec<String>,

    /// Specified the output directory.
    /// Default it's the same directory with input file.
    #[arg(short, long)]
    output: Option<String>,

    /// Verbosely list files processing.
    #[arg(short, long)]
    verbose: bool,

    /// The process work count.
    /// It should more than 0 and less than 9.
    #[arg(short, long, default_value = "1")]
    worker: usize,

    /// Force to overwrite file.
    #[arg(short, long)]
    force: bool,
}

/// The global program
#[derive(Clone)]
struct Program {
    command: Arc<Command>,
    total: ProgressBar,
    group: MultiProgress,
}

impl Program {
    /// Create a new command progress.
    fn new(command: Command) -> Result<Self> {
        let group = MultiProgress::new();
        let total = ProgressBar::new(0)
            .with_style(ProgressStyle::with_template(PBSTYLE_TOTAL)?.progress_chars(PBCHARS));
        Ok(Self {
            command: Arc::new(command),
            total: group.add(total),
            group,
        })
    }

    /// Create a new progress.
    fn new_progress<P>(&self, provider: &P) -> Result<Option<ProgressBar>>
    where
        P: DataProvider,
    {
        if !self.command.verbose {
            return Ok(None);
        }
        let progress = ProgressBar::new(provider.get_size())
            .with_message(provider.get_name())
            .with_style(ProgressStyle::with_template(PBSTYLE_SINGLE)?.progress_chars(PBCHARS));
        Ok(Some(self.group.insert_from_back(1, progress)))
    }

    fn finish(&self) {
        self.total.finish();
    }

    fn dump<P>(&self, provider: &P) -> Result<()>
    where
        P: DataProvider,
    {
        let source = File::open(provider.get_path())?;
        let result = match provider.get_format() {
            FileType::Ncm => self.dump_data(Ncmdump::from_reader(source)?, provider),
            FileType::Qmc => self.dump_data(Qmcdump::from_reader(source)?, provider),
            FileType::Other => Err(DumpError::Format.into()),
        };
        match result {
            Ok(()) => Ok(()),
            Err(ref e) => {
                if let Some(e) = e.downcast_ref::<DumpError>() {
                    if self.command.verbose {
                        self.group
                            .println(format!("[Warning] {e}: {}", provider.get_name()))?;
                    }
                    return Ok(());
                }
                result
            }
        }
    }

    fn dump_data<R, P>(&self, mut source: R, provider: &P) -> Result<()>
    where
        R: Read + Tag,
        P: DataProvider,
    {
        let progress = self.new_progress(provider)?;
        let mut data: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let mut buffer = [0; 1024];
        let mut ext_buffer = [0; 4];

        // Get file extensions early and return quickly if formatted incorrectly
        let ext = match source.read(&mut ext_buffer) {
            Ok(4) => match ext_buffer {
                [0x66, 0x4C, 0x61, 0x43] => Ok("flac"),
                [0x49, 0x44, 0x33, _] => Ok("mp3"),
                _ => Err(DumpError::Format),
            },
            Ok(_) => Err(DumpError::Format),
            Err(e) => return Err(e.into()),
        }?;

        // Get output file path
        let path = provider.get_path();
        let parent = match &self.command.output {
            None => path.parent().ok_or(Error::Path)?,
            Some(p) => Path::new(p),
        };
        let file_name = path.file_stem().ok_or(Error::Path)?;
        let target_path = parent.join(file_name).with_extension(ext);

        // Open / Create file
        let mut option = OpenOptions::new();
        option.truncate(true).write(true);
        let mut target = match (target_path.exists(), self.command.force) {
            (false, _) => option.create(true).open(target_path),
            (true, true) => option.open(target_path),
            (true, false) => return Err(DumpError::Exists.into()),
        }?;

        // Don't lose these 4 bits
        data.write_all(&ext_buffer)?;

        // Read data
        loop {
            // Read data from dumper
            match source.read(&mut buffer) {
                Ok(size) => {
                    // Break the loop if the size of data read is zero
                    if size == 0 {
                        break;
                    }

                    // Write data from buffer
                    data.write_all(&buffer[..size])?;

                    // Update progress bar
                    self.total.inc(size as u64);
                    if let Some(p) = &progress {
                        p.inc(size as u64);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }

        // Write tag
        if let Ok(t) = source.get_tag() {
            data.rewind()?;
            t.write_to_file(&mut data, id3::Version::Id3v24)?;
        }

        target.write_all(&data.into_inner())?;

        // Finish progress bar
        if let Some(p) = &progress {
            p.finish();
        }

        Ok(())
    }

    fn start(&self) -> Result<()> {
        // Check argument worker
        let worker = match self.command.worker {
            1..=8 => Ok(self.command.worker),
            _ => Err(Error::Worker),
        }?;

        // Check argument matchers
        if self.command.matchers.is_empty() {
            return Err(Error::NoFile.into());
        }

        let mut tasks = Vec::new();
        let (tx, rx) = crossbeam_channel::unbounded();

        {
            let state = self.clone();
            let task = thread::spawn(move || {
                for matcher in &state.command.matchers {
                    for entry in glob(matcher)? {
                        let path = entry.map_err(|_| Error::Path)?;
                        if !path.is_file() {
                            continue;
                        }
                        let p = FileProvider::new(path).map_err(|_| Error::Path)?;
                        let len = state.total.length().unwrap_or(0);
                        state.total.set_length(len + p.get_size());
                        tx.send(p)?;
                    }
                }
                anyhow::Ok(())
            });
            tasks.push(task);
        }

        for _ in 1..=worker {
            let rx = rx.clone();
            let state = self.clone();
            let task = thread::spawn(move || {
                while let Ok(w) = rx.recv() {
                    state.dump(&w)?;
                }
                anyhow::Ok(())
            });
            tasks.push(task);
        }
        for task in tasks {
            task.join().unwrap()?;
        }
        self.finish();
        Ok(())
    }
}

fn main() -> Result<()> {
    Program::new(Command::parse())?.start()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::{Command, Program};

    #[test]
    fn test_empty_input_files_err() -> Result<()> {
        let command = Command {
            matchers: vec![],
            worker: 1,
            ..Default::default()
        };
        let result = Program::new(command)?.start();
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_invalid_worker_err() -> Result<()> {
        let works = [0, 9, 10, 15, 100, 199];
        for worker in works {
            let command = Command {
                matchers: vec![],
                worker,
                ..Default::default()
            };
            let result = Program::new(command)?.start();
            assert!(result.is_err());
        }
        Ok(())
    }

    #[test]
    fn test_worker_ok() -> Result<()> {
        for worker in 1..=8 {
            let command = Command {
                matchers: vec!["./test/test.ncm".into()],
                worker,
                ..Default::default()
            };
            let result = Program::new(command)?.start();
            assert!(result.is_ok());
        }
        Ok(())
    }
}
