use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::thread;

use anyhow::Result;
use clap::Parser;

use ncmdump::utils::FileType;
use ncmdump::{NcmDump, QmcDump};

use crate::command::Command;
use crate::errors::Error;
use crate::metadata::{FlacMetadata, Metadata, Mp3Metadata};
use crate::provider::{DataProvider, FileProvider};
use crate::state::State;

mod command;
mod errors;
mod metadata;
mod provider;
mod state;
mod utils;

/// The global program
#[derive(Clone)]
struct Program {
    command: Arc<Command>,
    state: Arc<State>,
}

impl Program {
    /// Create new command progress.
    fn new(command: Command) -> Result<Self> {
        let state = State::try_from(&command)?;
        Ok(Self {
            command: Arc::new(command),
            state: Arc::new(state),
        })
    }

    fn dump<P>(&self, provider: &P) -> Result<()>
    where
        P: DataProvider,
    {
        let source = File::open(provider.get_path())?;
        let result = match provider.get_format() {
            FileType::Ncm => self.dump_data(provider, NcmDump::from_reader(source)?),
            FileType::Qmc => self.dump_data(provider, QmcDump::from_reader(source)?),
            FileType::Other => Err(Error::Format.into()),
        };
        if let Err(ref e) = result {
            self.state
                .println(format!("[Warning] {e}: {:?}", provider.get_path()))?;
        }
        Ok(())
    }

    fn dump_data<R, P>(&self, provider: &P, mut source: R) -> Result<()>
    where
        R: Read,
        P: DataProvider,
    {
        let progress = self.state.create_progress(provider)?;
        let mut data: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let mut buffer = [0; 1024];
        let mut ext_buffer = [0; 4];

        // Get file extensions early and return quickly if formatted incorrectly
        let ext = match source.read(&mut ext_buffer) {
            Ok(4) => match ext_buffer {
                [0x66, 0x4C, 0x61, 0x43] => Ok("flac"),
                [0x49, 0x44, 0x33, _] => Ok("mp3"),
                _ => Err(Error::Format),
            },
            Ok(_) => Err(Error::Format),
            Err(e) => return Err(e.into()),
        }?;

        // Get output file path
        let path = provider.get_path();
        let parent = match &self.command.output {
            None => path.parent().ok_or(Error::Path(format!(
                "Can't get output dir for target: {:?}",
                provider.get_path()
            )))?,
            Some(p) => Path::new(p),
        };
        let target_path = parent.join(provider.get_name()).with_extension(ext);

        // Open / Create file
        let mut option = OpenOptions::new();
        option.truncate(true).write(true);
        let mut target = match (target_path.exists(), self.command.overwrite) {
            (false, _) => option.create(true).open(target_path),
            (true, true) => option.open(target_path),
            (true, false) => return Err(Error::Exists.into()),
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
                    self.state.inc(size as u64);
                    if let Some(p) = &progress {
                        p.inc(size as u64);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }

        let data = data.into_inner();

        match provider.get_format() {
            FileType::Ncm => {
                let file = File::open(provider.get_path())?;
                let mut dump = NcmDump::from_reader(file)?;
                let image = dump.get_image()?;
                let info = dump.get_info()?;
                if ext == "mp3" {
                    let buffer = Mp3Metadata::new(&info, &image, &data).inject_metadata(data)?;
                    target.write_all(&buffer)?;
                } else if ext == "flac" {
                    let buffer = FlacMetadata::new(&info, &image, &data).inject_metadata(data)?;
                    target.write_all(&buffer)?;
                }
            }
            FileType::Qmc => target.write_all(&data)?,
            FileType::Other => return Err(Error::Format.into()),
        };

        // Finish progress bar
        if let Some(p) = &progress {
            p.finish();
        }

        Ok(())
    }

    fn start(&self) -> Result<()> {
        let mut tasks = Vec::new();
        let (tx, rx) = crossbeam_channel::unbounded();

        let items = self.command.items()?;
        let state = self.state.clone();
        tasks.push(thread::spawn(move || {
            for path in items {
                let provider = FileProvider::new(path)?;
                state.inc_length(provider.get_size());
                tx.send(provider)?;
            }
            anyhow::Ok(())
        }));

        for _ in 1..=self.command.worker {
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
        Ok(())
    }
}

fn main() -> Result<()> {
    let command = Command::parse();
    command.invalid()?;

    let program = Program::new(command)?;
    program.start()
}
