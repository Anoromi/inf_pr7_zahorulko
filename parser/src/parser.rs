use std::{
    collections::BTreeMap,
    fmt::Debug,
    io::Error,
    sync::{atomic::AtomicUsize, Arc},
};

use crate::{
    reader::*,
    segment::{CommonSegmentSelector, SegmentSelector},
};
use async_trait::async_trait;

use futures::future::join_all;
use sysinfo::DiskExt;

use crate::segment::Segments;
use tokio::{
    fs::{self, File},
    sync::Mutex,
    task::{self, JoinHandle},
};

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
    ZoneEnd,
}
#[async_trait]
pub trait Parser: Send {
    type Term: Term;
    type Provider: TermProvider<Term = Self::Term>;
    type Reader: Reader + Send;
    type Segments: Segments;
    type SegmentSelector: SegmentSelector;

    async fn parse(&mut self, reader: &mut Self::Reader, ind: usize) -> ParserCallback;

    async fn provider_from_file(file: &String) -> Result<Self::Provider, Error>;

    async fn flush_to(&mut self, file: &String) -> Result<(), Error>;
}

#[async_trait]
pub trait Merger: Send {
    type Parser: Parser;

    async fn merge(
        &mut self,
        input_file: Arc<Mutex<IndexPositions>>,
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

macro_rules! clone_all {
    ($($values : ident), *) => {
        $(let $values = $values.clone(); )*
    };
}
macro_rules! clone_mut_all {
    {$($values : ident), *} => {
        $(let mut $values = $values.clone(); )*
    };
}

pub struct IndexPositions {
    pub names: Vec<(String, usize)>,
    pub ids: Vec<(usize, usize)>,
}

impl IndexPositions {
    fn new<'a>(names: Vec<String>) -> Self {
        Self {
            names: names.into_iter().map(|v| (v, 0)).collect::<Vec<_>>(),
            ids: vec![],
        }
    }

    fn put(&mut self, name_index: usize) -> usize {
        self.ids.push((name_index, self.names[name_index].1));
        self.names[name_index].1 += 1;
        self.ids.len() - 1
    }
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
        let files = Arc::new(Mutex::new(IndexPositions::new(self.files)));
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
            clone_all![
                files,
                buffer_directory,
                file_index,
                output_index,
                output_files,
                builder
            ];
            tasks.push(task::spawn(async move {
                let mut parser = builder.lock().await.build();
                let (mut current_file_index, mut current_output, files_count) = {
                    let next_file = file_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let mut files = files.lock().await;
                    if files.names.len() >= next_file {
                        return;
                    }
                    (next_file, files.put(next_file), files.names.len())
                };
                while current_file_index < files_count {
                    // let next_file = file_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    // let mut flush_index = {
                    //     let files = files.lock().await;
                    //     if next_file >= files.names.len() {
                    //         break;
                    //     }
                    //     let flush_index = files.put(next_file);
                    //     let path = format!("{}\\{}", buffer_directory, flush_index);
                    //     output_files.lock().await.push(path.clone());
                    //     parser.flush_to(&path).await.unwrap();
                    // };
                    // let file = File::from_std(
                    //     std::fs::File::open(&files.lock().await.names[next_file].0).unwrap(),
                    // );
                    // let mut reader = builder.lock().await.reader_from_file(file).await;
                    // while {
                    //     match parser.parse(&mut reader, next_file).await {
                    //         ParserCallback::Full => false,
                    //         ParserCallback::FileEnd => true,
                    //         ParserCallback::ZoneEnd => todo!(),
                    //     }
                    // } {}
                    // while let ParserCallback::Full = parser.parse(&mut reader, next_file).await {
                    //     let flush_index =
                    //         output_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    //     let path = format!("{}\\{}", buffer_directory, flush_index);
                    //     output_files.lock().await.push(path.clone());
                    //     parser.flush_to(&path).await.unwrap();
                    // }

                    let file = File::from_std(
                        std::fs::File::open(&files.lock().await.names[current_file_index].0)
                            .unwrap(),
                    );
                    let mut reader = builder.lock().await.reader_from_file(file).await;
                    while {
                        match parser.parse(&mut reader, current_output).await {
                            ParserCallback::Full => {
                                let flush_index =
                                    output_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                let path = format!("{buffer_directory}\\{flush_index}");
                                parser.flush_to(&path).await.unwrap();
                                output_files.lock().await.push(path);
                                true
                            }
                            ParserCallback::FileEnd => {
                                current_file_index =
                                    file_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                current_output = files.lock().await.put(current_file_index);
                                false
                            }
                            ParserCallback::ZoneEnd => {
                                current_output = files.lock().await.put(current_file_index);
                                true
                            }
                        }
                    } {}
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
