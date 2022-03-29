use std::io::{Error, SeekFrom};

use async_trait::async_trait;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, BufReader},
};

#[async_trait]
pub trait U8Provider: Sized {
    type Reader;

    fn reader(&mut self) -> &mut Self::Reader;
    async fn next_u8(&mut self) -> Option<u8>;

    async fn take<const SIZE: usize>(&mut self) -> Option<[u8; SIZE]>;

    async fn from_path(path: &String) -> Result<Self, Error>;
}
#[async_trait]
pub trait MovableU8Provider: U8Provider {
    async fn seek(&mut self, from: SeekFrom) -> Result<(), Error>;
}

pub struct CommU8Provider {
    buf: [u8; 1],
    reader: BufReader<File>,
}

impl CommU8Provider {
    pub fn new(reader: BufReader<File>) -> Self {
        Self { buf: [0], reader }
    }

    #[inline(always)]
    pub async fn next_u8(&mut self) -> Option<u8> {
        if let Ok(out) = self.reader.read_exact(&mut self.buf).await {
            if out != 0 {
                return Some(self.buf[0]);
            }
        }
        None
    }

    #[inline(always)]
    pub async fn take<const SIZE: usize>(&mut self) -> Option<[u8; SIZE]> {
        let mut res = [0u8; SIZE];
        if self.reader.read_exact(&mut res).await.is_err() {
            return None;
        }
        Some(res)
    }
}
#[async_trait]
impl U8Provider for CommU8Provider {
    type Reader = BufReader<File>;

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    #[inline(always)]
    async fn next_u8(&mut self) -> Option<u8> {
        if let Ok(out) = self.reader.read_exact(&mut self.buf).await {
            if out != 0 {
                return Some(self.buf[0]);
            }
        }
        None
    }

    #[inline(always)]
    async fn take<const SIZE: usize>(&mut self) -> Option<[u8; SIZE]> {
        let mut res = [0u8; SIZE];
        if self.reader.read_exact(&mut res).await.is_err() {
            return None;
        }
        Some(res)
    }

    async fn from_path(path: &String) -> Result<Self, Error> {
        Ok(CommU8Provider::new(BufReader::new(File::open(path).await?)))
    }
}

#[async_trait]
impl MovableU8Provider for CommU8Provider {
    async fn seek(&mut self, from: SeekFrom) -> Result<(), Error> {
        self.reader.seek(from).await?;
        Ok(())
    }
}

pub async fn read_char(reader: &mut impl U8Provider) -> Option<char> {
    let char_buf: u32;
    if let Some(r) = reader.next_u8().await {
        if r >= 0b11110000 {
            let res = reader.take::<3>().await;
            match res {
                Some(res) => {
                    char_buf = ((r & 0b111) as u32) << 18
                        | ((res[0] & 0b111111) as u32) << 12
                        | ((res[1] & 0b111111) as u32) << 6
                        | ((res[2] & 0b111111) as u32);
                }
                None => return None,
            }
        } else if r >= 0b11100000 {
            let res = reader.take::<2>().await;
            match res {
                Some(res) => {
                    char_buf = ((r & 0b1111) as u32) << 12
                        | ((res[0] & 0b111111) as u32) << 6
                        | ((res[1] & 0b111111) as u32);
                }
                None => return None,
            }
        } else if r >= 0b11000000 {
            let res = reader.take::<1>().await;
            match res {
                Some(res) => {
                    char_buf = ((res[0] & 0b111111) as u32) | (((r & 0b11111) as u32) << 6)
                }
                None => return None,
            }
        } else {
            char_buf = r as u32;
        }
        char::from_u32(char_buf)
    } else {
        None
    }
}

pub async fn read_line(reader: &mut impl U8Provider) -> Option<String> {
    let mut str = String::new();
    while let Some(c) = read_char(reader).await {
        if c == '\n' || c == '\0' {
            break;
        } else if c != '\r' {
            str.push(c);
        }
    }
    if !str.is_empty() {
        return Some(str);
    } else {
        None
    }
}

pub async fn read_to_space(reader: &mut impl U8Provider) -> Option<String> {
    let mut str = String::new();
    while let Some(c) = read_char(reader).await {
        if c == ' ' {
            break;
        } else if c == '\0' {
            break;
        }
        str.push(c);
    }
    if !str.is_empty() {
        return Some(str);
    } else {
        None
    }
}

pub async fn read_char_reader(reader: &mut BufReader<File>) -> Result<char, Error> {
    let char_buf: u32;
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf).await?;
    if buf[0] >= 0b11110000 {
        let res = take_u8::<3>(reader).await?;
        char_buf = ((buf[0] & 0b111) as u32) << 18
                    | ((res[0] & 0b111111) as u32) << 12
                    | ((res[1] & 0b111111) as u32) << 6
                    | ((res[2] & 0b111111) as u32);
    } else if buf[0] >= 0b11100000 {
        let res = take_u8::<2>(reader).await?;
        char_buf = ((buf[0] & 0b1111) as u32) << 12
                    | ((res[0] & 0b111111) as u32) << 6
                    | ((res[1] & 0b111111) as u32);
    } else if buf[0] >= 0b11000000 {
        let res = take_u8::<1>(reader).await?;
        char_buf = ((res[0] & 0b111111) as u32) | (((buf[0] & 0b11111) as u32) << 6);
    } else {
        char_buf = buf[0] as u32;
    }
    char::from_u32(char_buf).ok_or(Error::new(std::io::ErrorKind::InvalidData, "char doesn't follow utf standard"))
}


#[inline(always)]
async fn take_u8<const SIZE: usize>(reader : &mut BufReader<File>) -> Result<[u8; SIZE], Error> {
    let mut res = [0u8; SIZE];
    reader.read_exact(&mut res).await?;
    Ok(res)
}