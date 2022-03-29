use std::{
    io::{Error, SeekFrom},
    mem::size_of,
};

use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader, BufWriter},
};

use crate::save::VariableSave;

pub struct CountedWriter {
    writer: BufWriter<File>,
    passed: u64,
}

impl CountedWriter {
    pub fn new(writer: BufWriter<File>) -> Self {
        Self { writer, passed: 0 }
    }

    #[inline(always)]
    pub async fn push(&mut self, buffer: &[u8]) -> Result<(), Error> {
        self.writer.write_all(buffer).await?;
        self.passed += buffer.len() as u64;
        Ok(())
    }

    pub async fn push_variable_u64(&mut self, value: u64) -> Result<(), Error> {
        self.passed += variable_save_u64(value, &mut self.writer).await? as u64;
        Ok(())
    }

    pub async fn push_u64(&mut self, value: u64) -> Result<(), Error> {
        self.writer.write_u64(value).await?;
        self.passed += size_of::<u64>() as u64;
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<(), Error> {
        self.writer.flush().await.map(|_| ())
    }

    pub async fn goto(&mut self, index: u64) -> Result<(), Error> {
        self.flush().await?;
        Ok(())
    }

    pub async fn push_variable(&mut self, save : &mut impl VariableSave) -> Result<(), Error>{
        self.passed += save.variable_save(&mut self.writer).await? as u64;
        Ok(())
    }

    /// Get the counted writer's passed.
    pub fn passed(&self) -> u64 {
        self.passed
    }
}

pub async fn variable_save_usize(mut v: usize, writer: &mut BufWriter<File>) -> Result<u8, Error> {
    let mut next = v >> 7;
    let mut write_slice = [0u8; 1];
    let mut writes = 0u8;
    while next > 0 {
        write_slice[0] = (v & 0b111_1111) as u8;
        writer.write_all(&write_slice).await?;
        v = next;
        next >>= 7;
        writes += 1;
    }
    write_slice[0] = (v & 0b111_1111) as u8 | (1 << 7);
    writer.write_all(&write_slice).await?;
    writes += 1;
    Ok(writes)
}

pub async fn variable_save_u64(mut v: u64, writer: &mut BufWriter<File>) -> Result<u8, Error> {
    let mut next = v >> 7;
    let mut write_slice = [0u8; 1];
    let mut writes = 0u8;
    while next > 0 {
        write_slice[0] = (v & 0b111_1111) as u8;
        writer.write_all(&write_slice).await?;
        v = next;
        next >>= 7;
        writes += 1;
    }
    write_slice[0] = (v & 0b111_1111) as u8 | (1 << 7);
    writer.write_all(&write_slice).await?;
    writes += 1;
    Ok(writes)
}

pub async fn variable_load(reader: &mut BufReader<File>) -> Result<usize, Error> {
    let mut v = 0usize;
    let mut shift = 0;
    let mut read_slice = [0u8; 1];
    reader.read(&mut read_slice).await?;
    loop {
        // dbg!(read_slice[0]);
        if read_slice[0] & 0b1000_0000 != 0 {
            break;
        }
        v += (read_slice[0] as usize) << shift;
        reader.read(&mut read_slice).await?;
        shift += 7;
    }
    v += (read_slice[0] as usize & 0b111_1111) << shift;
    Ok(v)
}

// pub async fn variable_load_u8_provider(reader: &mut impl U8Provider) -> Option<usize> {
//     let mut v = 0usize;
//     let mut shift = 0;
//     let mut next = reader.next_u8().await?;
//     loop {
//         if next & 0b111_1111 != 0 {
//             break;
//         }
//         v += (next as usize) << shift;
//         next = reader.next_u8().await?;
//         shift += 7;
//     }
//     v += (next as usize & 0b111_1111) << shift;
//     Some(v)
// }
