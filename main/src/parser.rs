use std::{
    collections::{BTreeMap},
    fmt::Debug,
    io::{Error},
    sync::{atomic::AtomicUsize, Arc},
};

use crate::{reader::*};
use async_trait::async_trait;


use futures::future::join_all;

use tokio::{
    fs::{self, File},
    sync::Mutex,
    task::{self, JoinHandle},
};
use crate::indexed::Segments;

pub trait Term: Ord + Debug {
    fn combine(&mut self, other: Self);

    fn get_use_count(&self) -> u64;
}

#[async_trait]
pub trait TermProvider {
    type Term: Term;

    async fn next_term(&mut self) -> Option<Self::Term>;
}

#[async_trait]
pub trait TermSaver {
    type Provider: TermProvider<Term = Self::Term>;
    type Term: Term;

    async fn save(path: &String, unique: BTreeMap<String, Self::Term>);

    async fn provider(path: &String) -> Self::Provider;
}
#[derive(PartialEq, Eq)]
pub enum ParserCallback {
    Full,
    FileEnd,
    ZoneEnd
}
#[async_trait]
pub trait Parser: Send {
    type Term: Term;
    type Provider: TermProvider<Term = Self::Term>;
    type Reader: Reader + Send;
    type Segments : Segments + Send;

    async fn parse(&mut self, reader: &mut Self::Reader, ind: usize) -> ParserCallback;

    async fn provider_from_file(file: &String) -> Result<Self::Provider, Error>;

    async fn flush_to(&mut self, file: &String) -> Result<(), Error>;
}

#[async_trait]
pub trait Merger: Send {
    type Parser: Parser;

    async fn merge(
        &mut self,
        input_file: Arc<Vec<String>>,
        buffer_files: Arc<Mutex<Vec<String>>>,
        destination: String,
    ) -> Result<(), Error>;
}

#[async_trait]
pub trait ParserBuilder: Send {
    type Parser: Parser;
    fn build(&mut self) -> Self::Parser;
    async fn reader_from_file(&mut self, file: File) -> <Self::Parser as Parser>::Reader;
}

pub struct ParseController<P: Parser, M: Merger<Parser = P>, Pb: ParserBuilder<Parser = P>> {
    files: Vec<String>,
    destination: String,
    buffer_directory: String,
    tasks_count: u16,
    builder: Pb,
    merger: M,
}

impl<P: Parser, M: Merger<Parser = P>, Pb: 'static + ParserBuilder<Parser = P>>
    ParseController<P, M, Pb>
{
    pub fn new(
        files: Vec<String>,
        destination: String,
        buffer_directory: String,
        tasks_count: u16,
        builder: Pb,
        merger: M,
    ) -> Self {
        Self {
            files,
            destination,
            buffer_directory,
            tasks_count,
            builder,
            merger,
        }
    }

    async fn invert(mut self) -> Result<(), Error> {
        let mut tasks = Vec::<JoinHandle<()>>::new();
        let files = Arc::new(self.files);
        match fs::create_dir(self.buffer_directory.clone()).await {
            Ok(_) => {
                log::info!("Directory created for parser");
            }
            Err(w) => {
                log::info!("{}", w);
            }
        };
        let buffer_directory = Arc::new(self.buffer_directory);
        let file_index = Arc::new(AtomicUsize::new(0));
        let output_index: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
        let output_files = Arc::new(Mutex::new(Vec::<String>::new()));
        let builder = Arc::new(Mutex::new(self.builder));
        for _ in 0..self.tasks_count {
            let files = files.clone();
            let buffer_directory = buffer_directory.clone();
            let file_index = file_index.clone();
            let output_index = output_index.clone();
            let output_files = output_files.clone();
            let builder = builder.clone();

            tasks.push(task::spawn(async move {
                let mut parser = builder.lock().await.build();
                loop {
                    let next_index = file_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if next_index >= files.len() {
                        let flush_index =
                            output_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let path = format!("{}\\{}", buffer_directory, flush_index);
                        output_files.lock().await.push(path.clone());
                        parser.flush_to(&path).await.unwrap();
                        break;
                    }
                    let file = File::from_std(std::fs::File::open(&files[next_index]).unwrap());
                    let mut reader = builder.lock().await.reader_from_file(file).await;

                    while let ParserCallback::Full = parser.parse(&mut reader, next_index).await {
                        let flush_index =
                            output_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let path = format!("{}\\{}", buffer_directory, flush_index);
                        output_files.lock().await.push(path.clone());
                        parser.flush_to(&path).await.unwrap();
                    }
                }
                let flush_index = output_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let path = format!("{}\\{}", buffer_directory, flush_index);
                output_files.lock().await.push(path.clone());
                parser.flush_to(&path).await.unwrap();
                ()
            }));
        }
        join_all(tasks).await;
        self.merger
            .merge(files, output_files, self.destination)
            .await?;
        Ok(())
    }

    pub async fn create_dictionary(self) -> Result<(), Error> {
        self.invert().await
    }
}

pub async fn remove_buffer(files: &Arc<Mutex<Vec<String>>>) {
    let files = files.lock().await;
    for v in files.iter() {
        if let Err(v) = fs::remove_dir_all(v).await {
            log::error!("{} at remove_buffer", v);
        }
    }
}