use std::{
    char::ToLowercase,
    io::Error,
    marker::PhantomData,
    sync::{atomic::AtomicU32, Arc},
};

use async_trait::async_trait;
use egui::TextBuffer;

use futures::future::join_all;
use save::u8::{U8Provider, read_char, CommU8Provider};
use tokio::{
    fs::{self, File},
    io::{AsyncSeekExt, AsyncWriteExt, BufReader, BufWriter},
    task::{self, JoinHandle},
};


pub enum CharType {
    Letter(ToLowercase),
    Ordinary(char),
    Delimiter(char),
    EOF,
}

trait Between {
    fn is_between(&self, l: &Self, h: &Self) -> bool;
}

impl<T: Ord> Between for T {
    #[inline(always)]
    fn is_between(&self, l: &Self, h: &Self) -> bool {
        l <= self && self >= h
    }
}

pub trait CharInterpretation {
    fn interpret_character(c: char) -> CharType;
}

pub struct CommCharInterpreter;

impl CharInterpretation for CommCharInterpreter {
    #[inline(always)]
    fn interpret_character(c: char) -> CharType {
        if c.is_alphabetic() {
            CharType::Letter(c.to_lowercase())
        } else if c.is_whitespace()
            || c.is_ascii_digit()
            || matches!(
                c,
                ',' | '.'
                    | ';'
                    | '('
                    | ')'
                    | '"'
                    | '|'
                    | '\\'
                    | '/'
                    | '='
                    | '-'
                    | '+'
                    | '*'
                    | '<'
                    | '>'
                    | '{'
                    | '}'
                    | '['
                    | ']'
                    | ':'
                    | '!'
                    | '?'
                    | '，'
                    | '；'
                    | '。'
                    | '、'
            )
        {
            CharType::Delimiter(c)
        } else if c == '\0' {
            CharType::EOF
        } else {
            CharType::Ordinary(c)
        }
    }
}

pub enum WordOption {
    Word(String),
    Empty,
}
impl WordOption {
    #[inline(always)]
    pub fn contains(&self, str: &str) -> bool {
        match self {
            WordOption::Word(w) => w == str,
            WordOption::Empty => false,
        }
    }
}

#[async_trait]
pub trait WordProvider {
    async fn next_word<Interpreter, Reader>(
        &mut self,
        reader: &mut Reader,
        mut start: Option<String>,
    ) -> Option<WordOption>
    where
        Interpreter: CharInterpretation,
        Reader: U8Provider + std::marker::Send;
}

pub struct XmlWordProvider {
    previous: Option<char>,
}

impl XmlWordProvider {
    pub fn new() -> Self {
        Self { previous: None }
    }

    pub fn consume(&mut self) -> Option<char> {
        let res = self.previous;
        self.previous = None;
        res
    }
}

#[async_trait]
impl WordProvider for XmlWordProvider {
    #[inline(always)]
    async fn next_word<Interpreter, Reader>(
        &mut self,
        reader: &mut Reader,
        start: Option<String>,
    ) -> Option<WordOption>
    where
        Interpreter: CharInterpretation,
        Reader: U8Provider + std::marker::Send,
    {
        fn passable<Interpreter: CharInterpretation>(str: &String) -> bool {
            !str.is_empty()
                && str.char_indices().any(|(_, c)| {
                    if let CharType::Letter(_) = Interpreter::interpret_character(c) {
                        true
                    } else {
                        false
                    }
                })
        }
        const AMP: &'static str = "&amp";
        const APOS: &'static str = "&apos";
        const GT: &'static str = "&gt";
        const LT: &'static str = "&lt";
        const QUOT: &'static str = "&quot";
        let mut start = {
            match start {
                Some(str) => str,
                None => String::new(),
            }
        };
        while let Some(c) = read_char(reader).await {
            match Interpreter::interpret_character(c) {
                CharType::Letter(chars) => {
                    start.reserve(chars.len());
                    for i in chars {
                        start.push(i);
                    }
                }
                CharType::Ordinary(c) => {
                    start.push(c);
                }
                CharType::Delimiter(c) => {
                    if c == '<' {
                        self.previous = Some('<');
                        return Some(WordOption::Empty);
                    }
                    if c == ';' {
                        self.previous = Some(c);
                        if start.ends_with(APOS) {
                            start.delete_char_range(start.len() - 5..start.len());
                            start.push('\'');
                        } else if start.ends_with(AMP) {
                            start.delete_char_range(start.len() - 4..start.len());
                            if passable::<Interpreter>(&start) {
                                break;
                            }
                        } else if start.ends_with(GT) {
                            start.delete_char_range(start.len() - 3..start.len());
                            if passable::<Interpreter>(&start) {
                                break;
                            }
                        } else if start.ends_with(LT) {
                            start.delete_char_range(start.len() - 3..start.len());
                            if passable::<Interpreter>(&start) {
                                break;
                            }
                        } else if start.ends_with(QUOT) {
                            start.delete_char_range(start.len() - 5..start.len());
                            if passable::<Interpreter>(&start) {
                                break;
                            }
                        } else {
                            if passable::<Interpreter>(&start) {
                                break;
                            }
                        }
                    } else if passable::<Interpreter>(&start) {
                        self.previous = Some(c);
                        break;
                    }
                    start.clear();
                }
                CharType::EOF => {
                    self.previous = Some('\0');
                    break;
                }
            }
        }
        if passable::<Interpreter>(&start) {
            Some(WordOption::Word(start))
        } else {
            None
        }
    }
}

#[derive(PartialEq, Eq)]
enum XmlPosition {
    InsideText,
    OutsideText,
}

pub struct XmlReader<Provider: U8Provider + Send, Interpreter: CharInterpretation> {
    reader: Provider,
    word_provider: XmlWordProvider,
    position: XmlPosition,
    interpreter: PhantomData<Interpreter>,
}

impl<Provider: U8Provider + Send, Interpreter: CharInterpretation>
    XmlReader<Provider, Interpreter>
{
    pub async fn new(reader: Provider) -> Result<Self, Error> {
        Ok(Self {
            reader,
            word_provider: XmlWordProvider::new(),
            position: XmlPosition::OutsideText,
            interpreter: PhantomData::<Interpreter>,
        })
    }

    async fn divide_write(
        &mut self,
        resdir: String,
        skips: u16,
        mut index: Arc<AtomicU32>,
    ) -> Option<()> {
        async fn wr(resdir: &String, index: &mut Arc<AtomicU32>) -> Option<BufWriter<File>> {
            let index = index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let name = format!("{}\\{}.xml", resdir.clone(), index);
            fs::remove_file(name.clone()).await.unwrap();
            println!("{}", name);
            Some(BufWriter::new(File::create(name).await.unwrap()))
        }

        const TEXT: &'static str = "text";
        let mut cur_file = wr(&resdir, &mut index).await?;
        let mut skip = skips;
        loop {
            while XmlPosition::OutsideText == self.position {
                loop {
                    let next = read_char(&mut self.reader).await?;
                    if next == '<' {
                        let c = read_char(&mut self.reader).await?;
                        if c == '/' {
                            while read_char(&mut self.reader).await? != '>' {}
                        } else {
                            let str = if c == ' ' { None } else { Some(c.to_string()) };

                            let str = self
                                .word_provider
                                .next_word::<Interpreter, Provider>(&mut self.reader, str)
                                .await?;
                            if str.contains(TEXT) {
                                if self.word_provider.consume() != Some('>') {
                                    while read_char(&mut self.reader).await? != '>' {}
                                }
                                self.position = XmlPosition::InsideText;
                                cur_file.write("<text>".as_bytes()).await.ok()?;
                                break;
                            }
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
                        cur_file.write(w.as_bytes()).await.ok()?;
                        cur_file.write(" ".as_bytes()).await.ok()?;
                    }
                }
                CharType::Ordinary(next) => {
                    let mut str = String::new();
                    str.push(next);
                    if let WordOption::Word(w) = self
                        .word_provider
                        .next_word::<Interpreter, Provider>(&mut self.reader, Some(str))
                        .await?
                    {
                        cur_file.write(w.as_bytes()).await.ok()?;
                        cur_file.write(" ".as_bytes()).await.ok()?;
                    }
                }
                CharType::Delimiter(d) => {
                    if d == '<'
                        && read_char(&mut self.reader).await? == '/'
                        && self
                            .word_provider
                            .next_word::<Interpreter, Provider>(&mut self.reader, None)
                            .await?
                            .contains(TEXT)
                    {
                        cur_file.write("\n</text>\n".as_bytes()).await.ok()?;
                        if self.position == XmlPosition::InsideText {
                            if skip == 0 {
                                cur_file.flush().await.ok()?;
                                cur_file = wr(&resdir, &mut index).await?;
                                skip = skips;
                            } else {
                                skip -= 1;
                            }
                        }
                        self.position = XmlPosition::OutsideText;
                        // wr(&resdir, &mut index).await?;
                    }
                }
                CharType::EOF => return None,
            }
        }
    }
}

pub enum ReaderResult {
    Word(String),
    AttributeEnd,
}

#[async_trait]
pub trait Reader {
    type UProvider: U8Provider + Send;
    type Interpreter: CharInterpretation;

    async fn next_word(&mut self) -> Option<ReaderResult>;
}

#[async_trait]
impl<
        Provider: U8Provider + std::marker::Send,
        Interpreter: CharInterpretation + std::marker::Send,
    > Reader for XmlReader<Provider, Interpreter>
{
    type UProvider = Provider;
    type Interpreter = Interpreter;

    async fn next_word(&mut self) -> Option<ReaderResult> {
        const TEXT: &'static str = "text";
        loop {
            while XmlPosition::OutsideText == self.position {
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
                                if str == TEXT {
                                    if self.word_provider.consume() != Some('>') {
                                        while read_char(&mut self.reader).await? != '>' {}
                                    }
                                    self.position = XmlPosition::InsideText;
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
                            .contains(TEXT)
                    {
                        self.position = XmlPosition::OutsideText;
                        return Some(ReaderResult::AttributeEnd);
                    }
                }
                CharType::EOF => return None,
            }
        }
    }
}

#[tokio::test]
async fn nya() -> Result<(), Error> {
    let mut xml = XmlReader::<CommU8Provider, CommCharInterpreter>::new(CommU8Provider::new(BufReader::new(
        File::open(r#"C:\Dataset\enwiki-20211101-pages-articles1.xml-p1p41242\enwiki-20211101-pages-articles1.xml-p1p41242"#).await?,
        // File::open(r#".\inp\5.xml"#).await?,
    )))
    .await?;
    xml.divide_write(".\\inp".to_string(), 100, Arc::new(AtomicU32::new(1163)))
        .await;

    println!();
    Ok(())
}

pub async fn gra() -> Result<(), Error> {
    let mut files = vec![
        r#"C:\Dataset\2\file.xml"#,
        r#"C:\Dataset\3\file.xml"#,
        r#"C:\Dataset\4\file.xml"#,
        r#"C:\Dataset\5\file.xml"#,
        r#"C:\Dataset\6\file.xml"#,
        r#"C:\Dataset\7\file.xml"#,
        r#"C:\Dataset\9\file.xml"#,
        r#"C:\Dataset\10\file.xml"#,
        r#"C:\Dataset\11\file.xml"#,
        r#"C:\Dataset\12\file.xml"#,
        r#"C:\Dataset\13\file.xml"#,
        r#"C:\Dataset\14\file.xml"#,
    ];
    let index = Arc::new(AtomicU32::new(0));
    let mut tasks = Vec::<JoinHandle<()>>::new();
    for _ in 0..files.len() {
        let file = files.pop().unwrap();
        let index = index.clone();
        tasks.push(task::spawn(async move {
            let mut xml = XmlReader::<_, CommCharInterpreter>::new(CommU8Provider::new(
                BufReader::with_capacity(1024 * 1024, File::open(file).await.unwrap()),
            ))
            .await
            .unwrap();
            xml.divide_write(".\\tvex".to_string(), 1000, index).await;
        }));
    }
    join_all(tasks).await;
    println!();
    Ok(())
}

#[tokio::test]
async fn reader_test() -> Result<(), Error> {
    let mut xml = XmlReader::<_, CommCharInterpreter>::new(CommU8Provider::new(BufReader::new(
        File::open(r#".\test\ha.xml"#).await?,
    )))
    .await?;
    while let Some(kar) = xml.next_word().await {
        match kar {
            ReaderResult::Word(w) => println!("{w}",),
            ReaderResult::AttributeEnd => println!("AttributeEnd"),
        }
    }
    Ok(())
}

pub trait FromU8Provider {
    fn from_file<Provider: U8Provider>(provider: Provider) -> Self;
}

#[tokio::test]
async fn interpret_test() {
    let a = "2009";
    for i in a.char_indices() {
        println!("{}", i.1);
    }
    println!(
        "{}",
        a.char_indices().any(|(_, c)| {
            if let CharType::Letter(_) = CommCharInterpreter::interpret_character(c) {
                true
            } else {
                false
            }
        })
    )
}

// #[tokio::test]
// async fn to_space() {
//     let mut reader = CommU8Provider::new(BufReader::new(File::open("../test/s.txt").await.unwrap()));
//     use crate::u8::read_to_space;
//     while let Some(w) = read_to_space(&mut reader).await {
//         println!("{}", w);
//     }
// }
