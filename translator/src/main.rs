use std::{
    io::Error,
    sync::{atomic::AtomicU32, Arc},
    vec,
};

use futures::future::join_all;
use parser::{
    reader::{CommCharInterpreter, XmlReader},
    rep_reader::RepeatedXmlReader,
};
use save::u8::CommU8Provider;
use tokio::{
    fs::File,
    io::BufReader,
    task::{self, JoinHandle},
};
#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut files = vec![
        r#"C:\Dataset\2\file.xml"#,
        r#"C:\Dataset\3\file.xml"#,
        r#"C:\Dataset\4\file.xml"#,
        r#"C:\Dataset\5\file.xml"#,
        r#"C:\Dataset\6\file.xml"#,
        // r#"C:\Dataset\7\file.xml"#,
        // r#"C:\Dataset\9\file.xml"#,
        // r#"C:\Dataset\10\file.xml"#,
        // r#"C:\Dataset\11\file.xml"#,
        // r#"C:\Dataset\12\file.xml"#,
        // r#"C:\Dataset\13\file.xml"#,
        // r#"C:\Dataset\14\file.xml"#,
    ];
    let index = Arc::new(AtomicU32::new(0));
    let mut tasks = Vec::<JoinHandle<()>>::new();
    for _ in 0..files.len() {
        let file = files.pop().unwrap();
        let index = index.clone();
        tasks.push(task::spawn(async move {
            let mut xml = RepeatedXmlReader::<_, CommCharInterpreter>::new(
                CommU8Provider::new(BufReader::with_capacity(
                    1024 * 1024,
                    File::open(file).await.unwrap(),
                )),
                Arc::new(vec!["title".to_string(), "text".to_string()]),
            )
            .await
            .unwrap();
            xml.divide_write(".\\gex".to_string(), 1000, index).await;
        }));
    }
    join_all(tasks).await;
    println!();
    Ok(())
}
