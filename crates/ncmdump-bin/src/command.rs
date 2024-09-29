use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use walkdir::WalkDir;

use crate::errors::Error;

#[derive(Clone, Debug, Default, Parser)]
#[command(name = "ncmdump", bin_name = "ncmdump", about, version)]
pub(crate) struct Command {
    /// Specified the files or dirs to convert.
    #[arg(value_name = "TARGETS")]
    pub(crate) targets: Vec<PathBuf>,

    /// Specified the output directory.
    /// Default it's the same directory with input file.
    #[arg(short = 'o', long = "output")]
    pub(crate) output: Option<String>,

    /// Force to overwrite file if file already exists.
    /// By default, if the file already exists, it will be skipped.
    #[arg(short = 'O', long)]
    pub(crate) overwrite: bool,

    /// Include file recursively
    #[arg(short, long)]
    pub(crate) recursive: bool,

    /// Verbosely list files processing.
    #[arg(short = 'v', long = "verbose")]
    pub(crate) verbose: bool,

    /// The process work count.
    /// It should more than 0 and less than 9.
    #[arg(short = 'w', long = "worker", default_value = "1")]
    pub(crate) worker: usize,
}

impl Command {
    pub(crate) fn invalid(&self) -> Result<()> {
        // Check argument worker
        if self.worker < 1 || self.worker > 8 {
            return Err(Error::Worker.into());
        }

        // Check argument matchers
        if self.targets.is_empty() {
            return Err(Error::NoTarget.into());
        }

        Ok(())
    }

    pub(crate) fn items(&self) -> Result<Vec<PathBuf>> {
        let mut result = Vec::new();
        for target in &self.targets {
            let target = target.to_path_buf();

            if !target.exists() {
                continue;
            }

            if target.is_file() {
                result.push(target.to_path_buf());
            } else if target.is_dir() {
                result.append(
                    &mut WalkDir::new(target)
                        .min_depth(1)
                        .max_depth({
                            match self.recursive {
                                true => 8,
                                false => 1,
                            }
                        })
                        .follow_links(true)
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_file())
                        .map(|e| e.into_path())
                        .collect(),
                );
            } else {
                return Err(Error::Path(String::from("Unsupport target type")).into());
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;

    use crate::errors::Error;
    use crate::Command;

    #[test]
    fn test_empty_input_files_err() -> Result<()> {
        let command = Command {
            targets: vec![],
            worker: 1,
            ..Default::default()
        };
        let result = command.invalid();
        assert!(result.is_err_and(|err| err
            .downcast_ref::<Error>()
            .map(|err| *err == Error::NoTarget)
            .unwrap_or(false)));
        Ok(())
    }

    #[test]
    fn test_invalid_worker_ok() -> Result<()> {
        let works = [1, 2, 3, 4, 5, 6, 7, 8];
        for worker in works {
            let command = Command {
                targets: vec![PathBuf::new()],
                worker,
                ..Default::default()
            };
            let result = command.invalid();
            assert!(result.is_ok());
        }
        Ok(())
    }

    #[test]
    fn test_invalid_worker_err() -> Result<()> {
        let works = [0, 9, 10, 15, 100, 199];
        for worker in works {
            let command = Command {
                targets: vec![PathBuf::new()],
                worker,
                ..Default::default()
            };
            let result = command.invalid();
            assert!(result.is_err_and(|err| err
                .downcast_ref::<Error>()
                .map(|err| *err == Error::Worker)
                .unwrap_or(false)));
        }
        Ok(())
    }
}
