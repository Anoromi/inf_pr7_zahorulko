
use std::io::Error;

use async_trait::async_trait;
use tokio::{
    fs::File,
    io::{BufReader, BufWriter},
};
use crate::writer::{variable_save_usize, variable_load};


#[async_trait]
pub trait VariableSave: Sized {
    async fn variable_save(&mut self, writer: &mut BufWriter<File>) -> Result<usize, Error>;
    async fn variable_load(reader: &mut BufReader<File>) -> Result<Self, Error>;
}

#[async_trait]
impl VariableSave for () {
    async fn variable_save(&mut self, _: &mut BufWriter<File>) -> Result<usize, Error> {
        Ok(0)
    }
    async fn variable_load(_: &mut BufReader<File>) -> Result<Self, Error> {
        Ok(())
    }
}

#[async_trait]
impl VariableSave for usize {
    async fn variable_save(&mut self, writer: &mut BufWriter<File>) -> Result<usize, Error> {
        variable_save_usize(*self, writer).await.map(|v| v as usize)
    }
    async fn variable_load(reader: &mut BufReader<File>) -> Result<Self, Error> {
        variable_load(reader).await
    }
}