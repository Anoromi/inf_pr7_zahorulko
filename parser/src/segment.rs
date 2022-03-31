use std::io::Error;

use async_trait::async_trait;
use modular_bitfield::{
    bitfield,
    prelude::{B1, B6},
    Specifier,
};
use save::save::VariableSave;
use std::fmt::Debug;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
};

pub trait Segments: Default + VariableSave + Debug + Send + Sync {
    fn selector_for(value: &'_ str) -> fn(&mut Self, u8) -> ();
}

#[bitfield]
#[derive(Debug)]
pub struct CommonSegments {
    title: B1,
    text: B1,
    nothing: B6,
}

#[async_trait]
impl VariableSave for CommonSegments {
    async fn variable_save(&mut self, writer: &mut BufWriter<File>) -> Result<usize, Error> {
        writer.write(&self.bytes).await?;
        Ok(self.bytes.len())
    }

    async fn variable_load(reader: &mut BufReader<File>) -> Result<Self, Error> {
        let mut out = CommonSegments::new();
        reader.read(&mut out.bytes).await?;
        Ok(out)
    }
}

impl Default for CommonSegments {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Segments for CommonSegments {
    #[inline]
    fn selector_for(value: &'_ str) -> fn(&mut CommonSegments, <B1 as Specifier>::InOut) -> () {
        match value {
            "text" => CommonSegments::set_text,
            "title" => CommonSegments::set_title,
            _ => panic!("Unexpected value {}", value),
        }
    }
}

pub trait SegmentSelector: Sync + Send {
    type Segments: Segments;
    fn applier_for(&self, value: &str) -> fn(&mut Self::Segments) -> ();
}

pub struct CommonSegmentSelector {}

impl CommonSegmentSelector {
    pub fn new() -> Self {
        Self {}
    }
}

impl SegmentSelector for CommonSegmentSelector {
    type Segments = CommonSegments;

    fn applier_for(&self, value: &str) -> fn(&mut Self::Segments) -> () {
        match value {
            "text" => |v| v.set_text(1),
            "title" => |v| v.set_title(1),
            _ => panic!("Unexpected value {}", value),
        }
    }
}
