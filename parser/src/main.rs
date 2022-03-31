extern crate core;

use std::sync::Arc;
use sysinfo::SystemExt;


use crate::indexed::{IndexedBuilder, IndexMerger, IndexParser};

pub mod indexed;
pub mod list;
pub mod parser;
pub mod reader;

pub mod rep_reader;
pub mod listmap;
pub mod save;
pub mod segment;

static mut SYSTEM: Option<sysinfo::System> = None;

pub fn get_system() -> &'static sysinfo::System {
    unsafe { SYSTEM.as_ref().unwrap() }
}

#[tokio::main]
async fn main() {
    use std::fs::{self};

    use chrono::Local;
    use log4rs::{
        append::file::FileAppender,
        config::{Appender, Root},
        Config,
    };

    use crate::parser::ParseController;

    unsafe {
        SYSTEM = Some(sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::new().with_memory().with_cpu(),
        ));
    }

    let mut files = fs::read_dir("../gex").unwrap();
    let mut files_vec = Vec::<String>::new();
    let mut files_size = 0u64;
    while let Some(w) = files.next() {
        let w = w.unwrap();
        files_vec.push(w.path().to_str().unwrap().to_string());
        files_size += w.metadata().unwrap().len();
    }

    files_vec.sort_unstable();

    let log_pipe = FileAppender::builder()
        .append(false)
        .build("info.txt")
        .unwrap();
    let config = Config::builder()
        .appender(Appender::builder().build("pipe", Box::new(log_pipe)))
        .build(
            Root::builder()
                .appender("pipe")
                .build(log::LevelFilter::Info),
        )
        .unwrap();

    let _handle = log4rs::init_config(config).unwrap();

    let destination = "../res".to_string();
    let buffer = ".\\buffer".to_string();

    log::info!("Files' overall size {} kb", files_size / 1024);
    log::info!("{}", Local::now().format("Start at %H:%M:%S").to_string());

    match ParseController::<IndexParser, _, _>::new(
        files_vec,
        destination,
        buffer,
        12,
        IndexedBuilder::new(100000, 6, Arc::new(vec!["title".to_string(), "text".to_string()])),
        IndexMerger::new(100),
    )
    .create_dictionary()
    .await {
        Ok(_) => {},
        Err(e) => println!("{e}"),
    }

    log::info!("{}", Local::now().format("End at %H:%M:%S").to_string());
    // let mut index = IndexTermProvider::new(CommU8Provider::new(tokio::io::BufReader::new(
    //     tokio::fs::File::open(".\\res\\dictionary.txt").await.unwrap(),
    // )))
    // .await;
    // let mut s_index = IndexTermProvider::new(CommU8Provider::new(tokio::io::BufReader::new(
    //     tokio::fs::File::open(".\\res\\dictionary.txt").await.unwrap(),
    // )))
    // .await;
    // for i in 0..400 {
    //     dbg!(i);
    //     dbg!(index.next_term().await);
    // }

    // let mut count = 0;
    // while let Some(kar) = index.next_term().await {
    //     let s_kar = s_index.next_term().await;
    //     dbg!(count, &s_kar);
    //     match s_kar {
    //         Some(s_kar) => {
    //             if kar != s_kar {

    //                 dbg!(count, kar);
    //             }
    //         },
    //         None => {
    //             dbg!(count, kar);
    //         },
    //     }
    //     // if kar.use_count > 1 {
    //         // dbg!(kar);
    //     // }
    //     count +=1;
    // }
    // println!("{}", count);
    // use reader::gra;

    // gra().await.unwrap();
}