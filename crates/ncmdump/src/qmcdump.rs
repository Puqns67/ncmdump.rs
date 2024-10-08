use std::io::{Read, Seek, SeekFrom, Write};

use crate::error::Result;

const BUFFER_SIZE: usize = 8192;
const KEY: [u8; 256] = [
    0x77, 0x48, 0x32, 0x73, 0xDE, 0xF2, 0xC0, 0xC8, 0x95, 0xEC, 0x30, 0xB2, 0x51, 0xC3, 0xE1, 0xA0,
    0x9E, 0xE6, 0x9D, 0xCF, 0xFA, 0x7F, 0x14, 0xD1, 0xCE, 0xB8, 0xDC, 0xC3, 0x4A, 0x67, 0x93, 0xD6,
    0x28, 0xC2, 0x91, 0x70, 0xCA, 0x8D, 0xA2, 0xA4, 0xF0, 0x08, 0x61, 0x90, 0x7E, 0x6F, 0xA2, 0xE0,
    0xEB, 0xAE, 0x3E, 0xB6, 0x67, 0xC7, 0x92, 0xF4, 0x91, 0xB5, 0xF6, 0x6C, 0x5E, 0x84, 0x40, 0xF7,
    0xF3, 0x1B, 0x02, 0x7F, 0xD5, 0xAB, 0x41, 0x89, 0x28, 0xF4, 0x25, 0xCC, 0x52, 0x11, 0xAD, 0x43,
    0x68, 0xA6, 0x41, 0x8B, 0x84, 0xB5, 0xFF, 0x2C, 0x92, 0x4A, 0x26, 0xD8, 0x47, 0x6A, 0x7C, 0x95,
    0x61, 0xCC, 0xE6, 0xCB, 0xBB, 0x3F, 0x47, 0x58, 0x89, 0x75, 0xC3, 0x75, 0xA1, 0xD9, 0xAF, 0xCC,
    0x08, 0x73, 0x17, 0xDC, 0xAA, 0x9A, 0xA2, 0x16, 0x41, 0xD8, 0xA2, 0x06, 0xC6, 0x8B, 0xFC, 0x66,
    0x34, 0x9F, 0xCF, 0x18, 0x23, 0xA0, 0x0A, 0x74, 0xE7, 0x2B, 0x27, 0x70, 0x92, 0xE9, 0xAF, 0x37,
    0xE6, 0x8C, 0xA7, 0xBC, 0x62, 0x65, 0x9C, 0xC2, 0x08, 0xC9, 0x88, 0xB3, 0xF3, 0x43, 0xAC, 0x74,
    0x2C, 0x0F, 0xD4, 0xAF, 0xA1, 0xC3, 0x01, 0x64, 0x95, 0x4E, 0x48, 0x9F, 0xF4, 0x35, 0x78, 0x95,
    0x7A, 0x39, 0xD6, 0x6A, 0xA0, 0x6D, 0x40, 0xE8, 0x4F, 0xA8, 0xEF, 0x11, 0x1D, 0xF3, 0x1B, 0x3F,
    0x3F, 0x07, 0xDD, 0x6F, 0x5B, 0x19, 0x30, 0x19, 0xFB, 0xEF, 0x0E, 0x37, 0xF0, 0x0E, 0xCD, 0x16,
    0x49, 0xFE, 0x53, 0x47, 0x13, 0x1A, 0xBD, 0xA4, 0xF1, 0x40, 0x19, 0x60, 0x0E, 0xED, 0x68, 0x09,
    0x06, 0x5F, 0x4D, 0xCF, 0x3D, 0x1A, 0xFE, 0x20, 0x77, 0xE4, 0xD9, 0xDA, 0xF9, 0xA4, 0x2B, 0x76,
    0x1C, 0x71, 0xDB, 0x00, 0xBC, 0xFD, 0x0C, 0x6C, 0xA5, 0x47, 0xF7, 0xF6, 0x00, 0x79, 0x4A, 0x11,
];

/// The qmc file dump wrapper.
pub struct QmcDump<S>
where
    S: Read,
{
    reader: S,
    cursor: u64,
}

impl<S> QmcDump<S>
where
    S: Read,
{
    fn map_l(value: u64) -> u8 {
        let v = if value > 0x7FFF {
            value % 0x7FFF
        } else {
            value
        } as usize;
        let index = (v * v + 80923) % 256;
        KEY[index]
    }

    fn encrypt(offset: u64, buffer: &mut [u8]) {
        for (index, byte) in buffer.iter_mut().enumerate() {
            *byte ^= Self::map_l(offset + index as u64);
        }
    }

    /// Create QmcDump from reader.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use std::fs::File;
    /// #
    /// # use ncmdump::QmcDump;
    /// #
    /// let file = File::open("res/test.qmcflac").expect("Can't open file");
    /// let _ = QmcDump::from_reader(file).unwrap();
    /// ```
    pub fn from_reader(reader: S) -> Result<Self> {
        Ok(Self { reader, cursor: 0 })
    }

    /// Get the music data from qmcdump.
    ///
    /// # Example:
    ///
    /// ```rust
    /// use std::fs::File;
    /// use std::io::Write;
    /// use std::path::Path;
    ///
    /// use anyhow::Result;
    /// use ncmdump::QmcDump;
    ///
    /// fn main() -> Result<()> {
    ///     let file = File::open("res/test.qmcflac")?;
    ///     let mut qmc = QmcDump::from_reader(file)?;
    ///     let music = qmc.get_data()?;
    ///
    ///     let mut target = File::options()
    ///         .create(true)
    ///         .write(true)
    ///         .open("res/test.flac")?;
    ///     target.write_all(&music)?;
    ///     Ok(())
    /// }
    /// ```
    pub fn get_data(&mut self) -> std::io::Result<Vec<u8>> {
        let mut buffer = [0; BUFFER_SIZE];
        let mut output = Vec::new();
        while let Ok(size) = self.read(&mut buffer) {
            if size == 0 {
                break;
            }
            output.write_all(&buffer[..size])?;
        }
        Ok(output)
    }
}

impl<R> Read for QmcDump<R>
where
    R: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let size = self.reader.read(buf)?;
        Self::encrypt(self.cursor, buf);
        self.cursor += size as u64;
        Ok(size)
    }
}

impl<R> Seek for QmcDump<R>
where
    R: Read + Seek,
{
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.cursor = self.reader.seek(pos)?;
        Ok(self.cursor)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Cursor;

    use anyhow::Result;

    use super::*;

    #[test]
    fn test_qmcdump_map_ok() {
        let dest = QmcDump::<File>::map_l(0x99);
        assert_eq!(dest, 146);

        let dest = QmcDump::<File>::map_l(0x8FFF);
        assert_eq!(dest, 195);
    }

    #[test]
    fn test_qmcdump_encrypt_ok() {
        let mut data = [0x00, 0x01, 0x02, 0x03];
        QmcDump::<File>::encrypt(0, &mut data);
        assert_eq!(data, [0xC3, 0x4B, 0xD4, 0xC9]);

        let mut data = [0x00, 0x01, 0x02, 0x03];
        QmcDump::<File>::encrypt(0x7fff, &mut data);
        assert_eq!(data, [0x4A, 0x4B, 0xD4, 0xC9]);
    }

    #[test]
    fn test_encrypt_head_ok() -> Result<()> {
        // fLaC
        let mut input = [0xA5, 0x06, 0xB7, 0x89];
        QmcDump::<File>::encrypt(0, &mut input);
        assert_eq!(input, [0x66, 0x4C, 0x61, 0x43]);

        // ID3
        let mut input = [0x8A, 0x0E, 0xE5];
        QmcDump::<File>::encrypt(0, &mut input);
        assert_eq!(input, [0x49, 0x44, 0x33]);
        Ok(())
    }

    #[test]
    fn test_qmcdump_ok() -> Result<()> {
        let input = File::open("res/test.qmcflac")?;
        let mut output = File::options()
            .create(true)
            .truncate(true)
            .write(true)
            .open("res/test.flac")?;
        let mut qmc = QmcDump::from_reader(input)?;
        let data = qmc.get_data()?;
        output.write_all(&data)?;
        Ok(())
    }

    #[test]
    fn test_qmcdump_read_ok() -> Result<()> {
        let input = Cursor::new([0x00, 0x01, 0x02, 0x03]);
        let mut qmc = QmcDump::from_reader(input)?;
        let mut buf = [0; 4];
        let size = qmc.read(&mut buf)?;
        assert_eq!(size, 4);
        assert_eq!(buf, [0xC3, 0x4B, 0xD4, 0xC9]);
        Ok(())
    }

    #[test]
    fn test_qmcdump_multi_read_ok() -> Result<()> {
        let input = Cursor::new([0x00, 0x01, 0x02, 0x03]);
        let mut qmc = QmcDump::from_reader(input)?;
        let mut buf = [0; 2];
        let size = qmc.read(&mut buf)?;
        assert_eq!(size, 2);
        assert_eq!(buf, [0xC3, 0x4B]);

        let size = qmc.read(&mut buf)?;
        assert_eq!(size, 2);
        assert_eq!(buf, [0xD4, 0xC9]);
        Ok(())
    }
}
