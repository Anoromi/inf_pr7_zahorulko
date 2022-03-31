use core::panic;
use std::{
    io::Error,
    marker::PhantomData,
    sync::{atomic::AtomicU32, Arc},
};

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::{
    fs::{self, File},
    io::BufWriter,
};

use crate::reader::{
    CharInterpretation, CharType, Reader, ReaderResult, WordOption, WordProvider, XmlWordProvider,
};

use save::u8::{read_char, CommU8Provider, U8Provider};

// struct Position {
//     inside : String
// }

#[derive(PartialEq, Eq)]
enum Position {
    Inside,
    Outside,
}

pub struct RepeatedXmlReader<Provider: U8Provider + Send, Interpreter: CharInterpretation + Send> {
    reader: Provider,
    word_provider: XmlWordProvider,
    position: Position,
    attribute_order: Arc<Vec<String>>,
    attribute_index: usize,
    interpreter: PhantomData<Interpreter>,
}

impl<Provider: U8Provider + Send, Interpreter: CharInterpretation + Send>
    RepeatedXmlReader<Provider, Interpreter>
{
    pub async fn new(reader: Provider, attribute_order: Arc<Vec<String>>) -> Result<Self, Error> {
        if attribute_order.len() == 0 {
            panic!("attribute_order.len() is 0")
        }
        Ok(Self {
            reader,
            word_provider: XmlWordProvider::new(),
            position: Position::Outside,
            attribute_order,
            attribute_index: 0,
            interpreter: PhantomData::<Interpreter>,
        })
    }

    pub async fn divide_write(
        &mut self,
        resdir: String,
        skips: u16,
        mut index: Arc<AtomicU32>,
    ) -> Option<()> {
        let skips = skips as u64 * self.zones_len() as u64;
        async fn wr(resdir: &String, index: &mut Arc<AtomicU32>) -> Option<BufWriter<File>> {
            let index = index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let name = format!("{}\\{}.xml", resdir.clone(), index);
            fs::remove_file(&name).await;
            println!("{}", name);
            dbg!(&name);
            Some(BufWriter::new(File::create(name).await.unwrap()))
        }

        let mut cur_file = wr(&resdir, &mut index).await?;
        let mut skip = skips;
        // cur_file
        //     .write(format!("<{}>\n", self.zone()).as_bytes())
        //     .await
        //     .ok()?;
        let mut has_next = true;
        while let Some(s) = self.next_word().await {
            if skip == 0 {
                println!("Zero");
                skip = skips;
                cur_file.flush().await.unwrap();
                cur_file = wr(&resdir, &mut index).await?;
            }
            if has_next {
                cur_file
                    .write(format!("<{}>\n", self.zone()).as_bytes())
                    .await
                    .ok()?;
                    has_next = false;
            }
            match s {
                ReaderResult::Word(w) => {
                    cur_file.write(w.as_bytes()).await.ok()?;
                    cur_file.write(" ".as_bytes()).await.ok()?;
                }
                ReaderResult::AttributeEnd => {
                    println!("AttributeEndP {} {skip}", &self.zone());
                    cur_file
                        .write(format!("\n<{}/>\n", self.zone()).as_bytes())
                        .await
                        .ok()?;
                    self.transform_zone().await;
                    skip -= 1;
                    has_next = true;
                    // if ((skips - skip) as usize) % self.zones_len() == 0 {
                }
            }
        }
        cur_file.flush().await.unwrap();

        Some(())
        // loop {}
    }
}

#[async_trait]
impl<
        Provider: U8Provider + std::marker::Send,
        Interpreter: CharInterpretation + std::marker::Send,
    > Reader for RepeatedXmlReader<Provider, Interpreter>
{
    type UProvider = Provider;
    type Interpreter = Interpreter;
    async fn next_word(&mut self) -> Option<ReaderResult> {
        let current_attribute = self.attribute_order[self.attribute_index].as_str();
        loop {
            while Position::Outside == self.position {
                if read_char(&mut self.reader).await? == '<' {
                    let c = read_char(&mut self.reader).await?;
                    if c == '/' {
                        while read_char(&mut self.reader).await? != '>' {}
                    } else {
                        let str = if c == ' ' { None } else { Some(c.to_string()) };

                        let str = self
                            .word_provider
                            .next_word::<Interpreter, Provider>(&mut self.reader, str)
                            .await?;
                        match str {
                            WordOption::Word(str) => {
                                if str == current_attribute {
                                    if self.word_provider.consume() != Some('>') {
                                        while read_char(&mut self.reader).await? != '>' {}
                                    }
                                    self.position = Position::Inside;
                                    break;
                                }
                            }
                            WordOption::Empty => {}
                        }
                    }
                }
            }

            let next = if let Some(v) = self.word_provider.consume() {
                v
            } else {
                read_char(&mut self.reader).await?
            };

            let next = Interpreter::interpret_character(next);
            match next {
                CharType::Letter(next) => {
                    let mut str = String::new();
                    for i in next {
                        str.push(i);
                    }
                    if let WordOption::Word(w) = self
                        .word_provider
                        .next_word::<Interpreter, Provider>(&mut self.reader, Some(str))
                        .await?
                    {
                        return Some(ReaderResult::Word(w));
                    };
                }
                CharType::Ordinary(next) => {
                    let mut str = String::new();
                    str.push(next);
                    if let WordOption::Word(w) = self
                        .word_provider
                        .next_word::<Interpreter, Provider>(&mut self.reader, Some(str))
                        .await?
                    {
                        return Some(ReaderResult::Word(w));
                    }
                }
                CharType::Delimiter(d) => {
                    if d == '<'
                        && read_char(&mut self.reader).await? == '/'
                        && self
                            .word_provider
                            .next_word::<Interpreter, Provider>(&mut self.reader, None)
                            .await?
                            .contains(current_attribute)
                    {
                        self.position = Position::Outside;
                        return Some(ReaderResult::AttributeEnd);
                    }
                }
                CharType::EOF => return None,
            }
        }
    }
}

#[async_trait]
impl<Provider: U8Provider + Send, Interpreter: CharInterpretation + Send> ZoneRepeatedReader
    for RepeatedXmlReader<Provider, Interpreter>
{
    async fn transform_zone(&mut self) {
        self.attribute_index += 1;
        self.attribute_index %= self.attribute_order.len();
    }

    fn zone(&self) -> &'_ str {
        self.attribute_order[self.attribute_index].as_str()
    }

    fn zones_len(&self) -> usize {
        self.attribute_order.len()
    }
}

#[async_trait]
pub trait ZoneRepeatedReader: Reader {
    async fn transform_zone(&mut self);

    fn zone(&self) -> &'_ str;

    fn zones_len(&self) -> usize;
}

#[cfg(test)]
mod tst {
    use std::{
        io::Error,
        sync::{atomic::AtomicU32, Arc},
    };

    use save::u8::CommU8Provider;
    use tokio::{
        fs::File,
        io::BufReader,
        task::{self, JoinHandle},
    };

    use crate::reader::{CommCharInterpreter, Reader, ReaderResult};

    use super::{RepeatedXmlReader, ZoneRepeatedReader};

    #[tokio::test]
    async fn reader_test() -> Result<(), Error> {
        let mut xml = RepeatedXmlReader::<_, CommCharInterpreter>::new(
            CommU8Provider::new(BufReader::new(File::open(r#".\test\ha.xml"#).await?)),
            Arc::new(vec!["title".to_string(), "text".to_string()]),
        )
        .await?;
        while let Some(kar) = xml.next_word().await {
            match kar {
                ReaderResult::Word(w) => println!("{w}",),
                ReaderResult::AttributeEnd => {
                    println!("AttributeEnd {}", &xml.zone());
                    xml.transform_zone().await;
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn gra() -> Result<(), Error> {
        let file = File::open(r#".\test\ha.xml"#).await?;
        let index = Arc::new(AtomicU32::new(0));
        let mut xml = RepeatedXmlReader::<_, CommCharInterpreter>::new(
            CommU8Provider::new(BufReader::with_capacity(1024 * 1024, file)),
            Arc::new(vec!["title".to_string(), "text".to_string()]),
        )
        .await
        .unwrap();
        xml.divide_write(".\\tvex".to_string(), 1, index).await;

        println!();
        Ok(())
    }
}
