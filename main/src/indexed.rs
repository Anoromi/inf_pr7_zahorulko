
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, BTreeMap},
    io::{Error, SeekFrom},
    marker::{Send, PhantomData},
    mem::size_of,
    sync::Arc, fmt::Debug,
};
use std::future::Future;

use async_trait::async_trait;
use chrono::Local;
use futures::future::join_all;
use modular_bitfield::{
    bitfield,
    prelude::{B1, B6}, Specifier,
};
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader, BufWriter},
    sync::Mutex,
    task::{self, JoinHandle},
};

use mcr::VariableSaveD;
use save::save::VariableSave;
use save::u8::{CommU8Provider, read_char, read_char_reader, read_line, read_to_space, U8Provider};
use save::writer::{CountedWriter, variable_load, variable_save_usize};

use crate::{
    adreader::RepeatedXmlReader,
    list::SortedLinkedList,
    listmap::SortedLinkedMap,
    parser::{
        Merger, Parser, ParserBuilder, ParserCallback, remove_buffer, Term, TermProvider, TermSaver,
    }, reader::{CommCharInterpreter, Reader, XmlReader},
};
use crate::adreader::ZoneRepeatedReader;
use crate::reader::ReaderResult;

#[derive(Debug)]
pub struct IndexedTerm<S : Segments> {
    pub term: String,
    pub use_count: u64,
    pub indexes: SortedLinkedMap<usize, UsageData<S>>,
}

impl<S : Segments> Ord for IndexedTerm<S> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.term.cmp(&other.term)
    }
}

impl<S : Segments> PartialOrd for IndexedTerm<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<S : Segments> Eq for IndexedTerm<S> {}

impl<S : Segments> PartialEq for IndexedTerm<S> {
    fn eq(&self, other: &Self) -> bool {
        self.term == other.term
    }
}

impl<S : Segments> IndexedTerm<S> {
    pub fn new(term: String) -> Self {
        Self {
            term,
            use_count: 0,
            indexes: SortedLinkedMap::new(),
        }
    }
}

impl<S : Segments> Term for IndexedTerm<S> {
    fn combine(&mut self, other: Self) {
        self.use_count += other.use_count;
        self.indexes.or(other.indexes, |_, _| {});
    }

    fn get_use_count(&self) -> u64 {
        self.use_count
    }
}

pub struct IndexedTermSaver {}

// #[async_trait]
// impl TermSaver for IndexedTermSaver {
//     type Provider = IndexTermProvider<CommU8Provider>;
//     type Term = IndexedTerm;

//     async fn save(
//         writer: &String,
//         unique: <<Self as TermSaver>::Provider as TermProvider>::Term,
//     ) {
//         async fn line(writer: &mut BufWriter<File>) {
//             writer.write("\n".as_bytes()).await.unwrap();
//         }

//         writer.write_all(unique.term.as_bytes()).await.unwrap();
//         line(writer).await;

//         writer
//             .write_all(unique.use_count.to_string().as_bytes())
//             .await
//             .unwrap();
//         line(writer).await;

//         writer
//             .write(unique.indexes.len().to_string().as_bytes())
//             .await
//             .unwrap();
//         line(writer).await;

//         for i in unique
//             .indexes
//             .iter()
//             .collect::<Vec<usize>>()
//             .into_iter()
//             .rev()
//         {
//             writer.write(i.to_string().as_bytes()).await.unwrap();
//             writer.write(" ".as_bytes()).await.unwrap();
//         }
//     }

//     async fn provider(file: BufReader<File>) -> Self::Provider {
//         IndexTermProvider::new(CommU8Provider::new(file)).await
//     }

// }

pub struct IndexParser {
    b_tree: BTreeMap<String, IndexedTerm<<IndexParser as Parser>::Segments>>,
    tree_max_size: usize,
    lexical_max_size: u8,
}

impl IndexParser {
    pub fn new(tree_max_size: usize, lexical_max_size: u8) -> Self {
        Self {
            b_tree: BTreeMap::new(),
            tree_max_size,
            lexical_max_size,
        }
    }
}

pub trait Segments : Default + VariableSave + Debug + Send {
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
        reader.read(&mut out.bytes).await;
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
            "text" => {
                CommonSegments::set_text
            }
            "title" => {
                CommonSegments::set_title
            }
            _ => panic!("Unexpected value {}", value)
        }
    }
}

#[derive(VariableSaveD, Debug)]
pub struct UsageData<S : Segments> {
    use_count: usize,
    segments: S,
}

impl<S : Segments> UsageData<S> {
    fn new() -> Self {
        Self {
            use_count: 0,
            segments: S::default(),
        }
    }

    /// Get a mutable reference to the word usage's use count.
    pub fn use_count_mut(&mut self) -> &mut usize {
        &mut self.use_count
    }

    /// Get a mutable reference to the word usage's segments.
    pub fn segments_mut(&mut self) -> &mut S {
        &mut self.segments
    }
}

#[async_trait]
impl Parser for IndexParser {
    type Term = IndexedTerm<Self::Segments>;
    type Reader = RepeatedXmlReader<CommU8Provider, CommCharInterpreter>;
    type Provider = IndexTermProvider<Self::Segments>;
    type Segments = CommonSegments;

    async fn parse(&mut self, reader: &mut Self::Reader, ind: usize) -> ParserCallback {

        // while self.b_tree.len() < self.tree_max_size {
        //     let word = reader.next_word().await;

        //     match word {
        //         Some(word) => match self.b_tree.get_mut(&word) {
        //             Some(term) => {
        //                 term.indexes.push(ind);
        //                 term.use_count += 1;
        //             }
        //             None => {
        //                 let mut term = IndexedTerm::new(word.clone());
        //                 term.indexes.push(ind);
        //                 term.use_count += 1;
        //                 self.b_tree.insert(word, term);
        //             }
        //         },
        //         None => {
        //             return ParserCallback::FileEnd;
        //         }
        //     }
        // }
        // ParserCallback::Full
        let mut current_index = reader.zones_len();

        // let mut current_applier =
        while self.b_tree.len() < self.tree_max_size {
            match reader.next_word().await {
                None => break,
                Some(v) => {
                    match v {
                        ReaderResult::Word(w) => {}
                        ReaderResult::AttributeEnd => {
                            reader.transform_zone().await;
                            current_index -= 1;
                        }
                    }
                }
            }
        }
        todo!()
    }

    async fn provider_from_file(file: &String) -> Result<Self::Provider, Error> {
        IndexTermProvider::new(file).await
    }

    async fn flush_to(&mut self, file: &String) -> Result<(), Error> {
        match fs::create_dir(&file).await {
            Ok(_) => {}
            Err(_) => {}
        }
        let mut merger = IndexMergeSaver::new(file.clone(), self.lexical_max_size).await?;
        let tree = std::mem::replace(&mut self.b_tree, BTreeMap::new());
        for v in tree.into_iter() {
            merger.push(v.1).await?;
        }
        merger.finish().await?;
        Ok(())
    }
}

pub struct IndexMerger {
    lexical_max_size: u8,
}

impl IndexMerger {
    pub fn new(lexical_max_size: u8) -> Self {
        Self { lexical_max_size }
    }
}

#[async_trait]
impl Merger for IndexMerger {
    type Parser = IndexParser;

    async fn merge(
        &mut self,
        input_file: Arc<Vec<String>>,
        buffer_files: Arc<Mutex<Vec<String>>>,
        destination: String,
    ) -> Result<(), Error> {
        match fs::create_dir(destination.clone()).await {
            Ok(_) => {
                log::info!("Directory created for parser");
            }
            Err(w) => {
                log::info!("{}", w);
            }
        }

        log::info!(
            "Merge starts at {}",
            Local::now().format("%H:%M:%S").to_string()
        );

        write_input_files(format!("{}\\files.txt", destination.clone()), input_file).await;

        let mut providers = Vec::<Arc<Mutex<<IndexParser as Parser>::Provider>>>::new();
        let mut tasks = Vec::<JoinHandle<()>>::new();
        async fn line(writer: &mut BufWriter<File>) {
            writer.write("\n".as_bytes()).await.unwrap();
        }
        for v in buffer_files.lock().await.iter() {
            providers.push(Arc::new(Mutex::new(
                IndexParser::provider_from_file(&v).await?,
            )));
        }

        let p_q = Arc::new(Mutex::new(BinaryHeap::<(
            Reverse<<IndexParser as Parser>::Term>,
            usize,
        )>::new()));

        // dbg!("Nya");

        for (v, i) in providers.iter_mut().enumerate() {
            if let Some(term) = i.lock().await.next_term().await {
                p_q.lock().await.push((Reverse(term), v));
            }
        }

        // let mut writer = BufWriter::with_capacity(
        //     1024 * 1024 * 50,
        //     File::create(format!("{}\\dictionary.txt", destination.clone()))
        //         .await
        //         .unwrap(),
        // );

        let mut saver = IndexMergeSaver::new(destination.clone(), self.lexical_max_size).await?;

        let mut values = Vec::<usize>::new();
        let mut lexeme_count = 0u64;
        let mut term_count = 0u64;
        let mut tstind = 0;
        loop {
            let mut q = p_q.lock().await;
            if let Some(mut next) = q.pop() {
                // dbg!("Hya");
                values.push(next.1);
                while let Some(v) = q.peek() {
                    // dbg!("Kya");
                    if v.0 == next.0 {
                        let v = q.pop().unwrap();
                        values.push(v.1);
                        next.0.0.combine(v.0.0);
                    } else {
                        break;
                    }
                }
                // dbg!("Bya");
                // dbg!("Hya");
                drop(q);

                // values.iter().map(|v| {
                //     task::spawn(async move {
                //         let provider = providers[*v].lock().await;
                //     })
                // });
                for v in values.iter() {
                    let v = *v;
                    let provider = providers[v].clone();
                    let p_q = p_q.clone();
                    tasks.push(task::spawn(async move {
                        let next = provider.lock().await.next_term().await;
                        // dbg!(&next);
                        if let Some(next) = next {
                            p_q.lock().await.push((Reverse(next), v))
                        }
                        // if let Some(pr)
                    }))
                }
                // dbg!("Hya");
                join_all(tasks).await;
                // dbg!("Hya");
                tasks = Vec::new();

                // for i in values.iter() {
                //     let next = providers[*i].lock().await.next_term().await;
                //     if let Some(provider) = next {
                //         p_q.push((Reverse(provider), *i));
                //     }
                // }
                lexeme_count += next.0.0.get_use_count();
                term_count += 1;
                saver.push(next.0.0).await?;
                values.clear();
                tstind += 1;
                // if tstind % 1 == 0 {
                //     dbg!(tstind);
                // }
            } else {
                break;
            }
        }
        saver.finish().await?;

        remove_buffer(&buffer_files).await;

        let mut info_writer = BufWriter::new(
            File::create(format!("{}\\info.txt", destination))
                .await
                .unwrap(),
        );
        info_writer
            .write_all(lexeme_count.to_string().as_bytes())
            .await
            .unwrap();
        line(&mut info_writer).await;

        info_writer
            .write_all(term_count.to_string().as_bytes())
            .await
            .unwrap();
        line(&mut info_writer).await;
        info_writer.flush().await.unwrap();
        Ok(())
    }
}

async fn write_input_files(path: String, input_files: Arc<Vec<String>>) {
    let mut file = BufWriter::new(File::create(path).await.unwrap());
    for v in input_files.iter() {
        file.write_all(v.clone().as_bytes()).await.unwrap();
        file.write_all("\n".as_bytes()).await.unwrap();
    }
    file.flush().await.unwrap();
}

pub struct IndexedBuilder {
    tree_max_size: usize,
    lexical_max_size: u8,
    attributes: Arc<Vec<String>>,
}

impl IndexedBuilder {
    pub fn new(tree_max_size: usize, lexical_max_size: u8, attributes: Arc<Vec<String>>) -> Self {
        Self {
            tree_max_size,
            lexical_max_size,
            attributes,
        }
    }
}

#[async_trait]
impl ParserBuilder for IndexedBuilder {
    type Parser = IndexParser;

    fn build(&mut self) -> Self::Parser {
        IndexParser::new(self.tree_max_size, self.lexical_max_size)
    }

    async fn reader_from_file(&mut self, file: File) -> <Self::Parser as Parser>::Reader {
        RepeatedXmlReader::<_, CommCharInterpreter>::new(CommU8Provider::new(BufReader::new(file)), self.attributes.clone())
            .await
            .unwrap()
    }
}

struct Dictionary {
    pointer_part: BufReader<File>,
    lexical_part: BufReader<File>,
    index_part: BufReader<File>,
}

impl<> Dictionary {
    async fn new(directory: &String) -> Result<Self, Error> {
        Ok(Self {
            pointer_part: BufReader::new(File::open(&format!("{directory}/dictionary.txt")).await?),
            lexical_part: BufReader::new(
                File::open(&format!("{directory}/lexical_part.txt")).await?,
            ),
            index_part: BufReader::new(File::open(&format!("{directory}/index_part.txt")).await?),
        })
    }

    async fn get_term(&mut self, cursor: IndexedCursor) -> Result<IndexedTerm<S>, Error> {
        self.lexical_part
            .seek(SeekFrom::Start(cursor.lexical_pointer as u64))
            .await?;
        let mut start = String::new();
        let mut index = variable_load(&mut self.lexical_part).await?;
        while index > 0 {
            let next_char = read_char_reader(&mut self.lexical_part).await?;
            index -= next_char.len_utf8();
            start.push(next_char);
        }

        for _ in 0..cursor.lexical_index {
            let skip = variable_load(&mut self.lexical_part).await?;
            self.lexical_part
                .seek(SeekFrom::Current(skip as i64))
                .await?;
        }
        let mut index = variable_load(&mut self.lexical_part).await?;
        while index > 0 {
            let next_char = read_char_reader(&mut self.lexical_part).await?;
            index -= next_char.len_utf8();
            start.push(next_char);
        }

        self.index_part
            .seek(SeekFrom::Start(cursor.indexes_pointer as u64))
            .await?;
        let list = SortedLinkedMap::<usize, UsageData<S>>::variable_load(&mut self.index_part).await?;

        Ok(IndexedTerm {
            term: start,
            use_count: cursor.use_count as u64,
            indexes: list,
        })
    }
}

pub struct IndexTermProvider<S : Segments> {
    dictionary: Dictionary,
    first_part: String,
    first_part_pointer: Option<usize>,
    remaining_size: usize,
    segment_date : PhantomData<S>
}

impl<S : Segments> IndexTermProvider<S> {
    pub async fn new(directory: &String) -> Result<Self, Error> {
        let mut dictionary = Dictionary::new(directory).await?;
        let remaining_size = dictionary.pointer_part.read_u64().await? as usize;
        Ok(Self {
            dictionary,
            first_part: String::new(),
            first_part_pointer: None,
            remaining_size,
            segment_date: PhantomData::<S>
        })
    }
}

#[async_trait]
impl<S : Segments> TermProvider for IndexTermProvider<S> {
    type Term = IndexedTerm<S>;

    async fn next_term(&mut self) -> Option<Self::Term> {
        // if let Some(st) = read_line(&mut self.reader).await {
        //     let use_count = read_line(&mut self.reader)
        //         .await
        //         .unwrap()
        //         .parse::<u64>()
        //         .unwrap();

        //     let index = read_line(&mut self.reader)
        //         .await
        //         .unwrap()
        //         .parse::<usize>()
        //         .unwrap();
        //     let mut list = SortedLinkedList::<usize>::new();
        //     for _ in 0..index {
        //         let next = read_to_space(&mut self.reader).await.unwrap();
        //         list.push(next.parse::<usize>().unwrap());
        //     }

        //     Some(IndexedTerm {
        //         term: st,
        //         use_count,
        //         indexes: list,
        //     })
        // } else {
        //     None
        // }
        // IndexedCursor::load(self.)
        // let load = variable_load(&mut self.pointer_part).await.ok()?;
        if self.remaining_size == 0 {
            return None;
        }
        let next = IndexedCursor::load(&mut self.dictionary.pointer_part)
            .await
            .ok()?;
        // dbg!(&next);
        if self.first_part_pointer.is_none()
            || self.first_part_pointer.unwrap() != next.lexical_pointer
        {
            self.first_part.clear();
            self.first_part_pointer = Some(next.lexical_pointer as usize);
            // dbg!("var");
            let mut skip = variable_load(&mut self.dictionary.lexical_part)
                .await
                .ok()?;
            // dbg!(skip);
            while skip > 0 {
                let next_char = read_char_reader(&mut self.dictionary.lexical_part)
                    .await
                    .ok()?;
                skip -= next_char.len_utf8();
                self.first_part.push(next_char);
            }
        }
        let mut term = String::new();
        term.push_str(&self.first_part);

        // dbg!("var");
        let mut skip = variable_load(&mut self.dictionary.lexical_part)
            .await
            .ok()?;
        // dbg!("S", skip);
        while skip > 0 {
            let next_char = read_char_reader(&mut self.dictionary.lexical_part)
                .await
                .ok()?;
            skip -= next_char.len_utf8();
            term.push(next_char);
        }

        // dbg!("list");
        let indexes = SortedLinkedMap::<usize, UsageData>::variable_load(&mut self.dictionary.index_part)
            .await
            .ok()?;
        // dbg!("list end");
        self.remaining_size -= 1;
        Some(IndexedTerm {
            term,
            use_count: next.use_count as u64,
            indexes,
        })
    }
}

struct IndexMergeSaver<S: Segments> {
    directory: String,
    pointer_part: BufWriter<File>,
    lexical_part: CountedWriter,
    index_part: CountedWriter,
    buffer_items: Vec<IndexedTerm<S>>,
    current_substr_size: u16,
    max_part_size: u8,
    current_directory_size: u64,
}

impl<S : Segments> IndexMergeSaver<S> {
    async fn new(directory: String, max_size: u8) -> Result<Self, Error> {
        let mut pointer_part = BufWriter::with_capacity(
            1024 * 1024 * 5,
            File::create(format!("{}/dictionary.txt", &directory)).await?,
        );
        pointer_part.write_u64(0).await?;
        Ok(Self {
            pointer_part,
            lexical_part: CountedWriter::new(BufWriter::new(
                File::create(format!("{}/lexical_part.txt", &directory)).await?,
            )),
            index_part: CountedWriter::new(BufWriter::new(
                File::create(format!("{}/index_part.txt", &directory)).await?,
            )),
            directory: directory,
            buffer_items: Vec::with_capacity(max_size.into()),
            current_substr_size: 0,
            max_part_size: max_size,
            current_directory_size: 0,
        })
    }

    async fn flush(&mut self) -> Result<(), Error> {
        if self.buffer_items.len() == 0 {
            return Ok(());
        }
        let items = std::mem::take(&mut self.buffer_items);
        let lexical_pointer = self.lexical_part.passed();
        self.lexical_part
            .push_variable_u64(self.current_substr_size as u64)
            .await?;
        {
            let first_part = &items[0].term.as_str()[..self.current_substr_size as usize];
            self.lexical_part.push(first_part.as_bytes()).await?;
        }
        for (i, mut v) in items.into_iter().enumerate() {
            IndexedCursor::new(
                lexical_pointer as usize,
                i as u8,
                self.index_part.passed() as usize,
                v.use_count as usize,
            )
                .save(&mut self.pointer_part)
                .await?;
            self.index_part.push_variable(&mut v.indexes).await?;
            // self.index_part.push_sorted_indexes(v.indexes).await?;
            let other_part = &v.term.as_str()[self.current_substr_size as usize..];
            self.lexical_part
                .push_variable_u64(other_part.len() as u64)
                .await?;
            self.lexical_part.push(other_part.as_bytes()).await?;
        }
        Ok(())
    }

    async fn finish(&mut self) -> Result<(), Error> {
        self.flush().await?;
        self.index_part.flush().await?;
        self.lexical_part.flush().await?;
        self.pointer_part.flush().await?;
        self.pointer_part.seek(SeekFrom::Start(0)).await?;
        self.pointer_part
            .write_u64(self.current_directory_size)
            .await?;
        self.pointer_part.flush().await?;
        Ok(())
    }

    async fn push(&mut self, term: IndexedTerm<S>) -> Result<(), Error> {
        if self.buffer_items.len() == self.max_part_size as usize {
            self.flush().await?;
            self.current_substr_size = 0;
        } else if self.buffer_items.len() > 0 {
            let last = self.buffer_items.last().unwrap();
            let size = count_same(&last.term, &term.term) as u16;
            if size > self.current_substr_size {
                let last = self.buffer_items.pop().unwrap();
                self.flush().await?;
                self.current_substr_size = size;
                self.buffer_items.push(last);
            } else if self.buffer_items.len() * (self.current_substr_size as usize)
                < ((self.buffer_items.len() + 1) * (size as usize))
            {
                self.current_substr_size = size;
            } else {
                self.flush().await?;
                self.current_substr_size = 0;
            }
        } else {
            self.current_substr_size = 0;
        }
        self.buffer_items.push(term);
        self.current_directory_size += 1;
        Ok(())
    }
}

fn count_same(f: &String, s: &String) -> usize {
    let mut fc = f.chars();
    let mut sc = s.chars();
    let mut i = 0;
    while let (Some(f), Some(s)) = (fc.next(), sc.next()) {
        if f != s {
            break;
        }
        i += f.len_utf8();
    }
    i
}

#[tokio::test]
async fn vartst() {
    let mut b = BufWriter::new(File::create("tst/vartst.txt").await.unwrap());
    variable_save_usize(255, &mut b).await.unwrap();
    b.flush().await.unwrap();
}

#[derive(Debug)]
struct IndexedCursor {
    lexical_pointer: usize,
    lexical_index: u8,
    indexes_pointer: usize,
    use_count: usize,
}

impl IndexedCursor {
    fn new(
        lexical_pointer: usize,
        lexical_index: u8,
        indexes_pointer: usize,
        use_count: usize,
    ) -> Self {
        Self {
            lexical_pointer,
            lexical_index,
            indexes_pointer,
            use_count,
        }
    }

    async fn save(self, writer: &mut BufWriter<File>) -> Result<(), Error> {
        writer.write_u64(self.lexical_pointer as u64).await?;
        writer.write_u8(self.lexical_index).await?;
        writer.write_u64(self.indexes_pointer as u64).await?;
        writer.write_u64(self.use_count as u64).await?;
        Ok(())
    }

    async fn load(reader: &mut BufReader<File>) -> Result<IndexedCursor, Error> {
        Ok(Self {
            lexical_pointer: reader.read_u64().await? as usize,
            lexical_index: reader.read_u8().await?,
            indexes_pointer: reader.read_u64().await? as usize,
            use_count: reader.read_u64().await? as usize,
        })
    }
}

// #[tokio::test]
// async fn merger_tst() -> Result<(), Error> {
//     let mut merger = IndexMergeSaver::new("res".to_string(), 6).await?;
//     let mut next = IndexedTerm::new("Tяrаt".to_string());
//     next.indexes.push(4);
//     merger.push(next).await?;
//     next = IndexedTerm::new("Tяrk".to_string());
//     next.indexes.push(1);
//     merger.push(next).await?;
//     merger.finish().await?;
//     Ok(())
// }

// #[tokio::test]
// async fn ano_merger_tst() -> Result<(), Error> {
//     let mut merger = IndexMergeSaver::new("res".to_string(), 6).await?;
//     // let mut v = vec![]
//     let mut next = IndexedTerm::new("'''garfield".to_string());
//     next.indexes.push(4);
//     merger.push(next).await?;
//     let mut next = IndexedTerm::new("'''len".to_string());
//     next.indexes.push(4);
//     merger.push(next).await?;
//     let mut next = IndexedTerm::new("'''hack".to_string());
//     next.indexes.push(4);
//     merger.push(next).await?;
//     next = IndexedTerm::new("Tяrk".to_string());
//     next.indexes.push(1);
//     merger.push(next).await?;
//     dbg!(merger.current_directory_size);
//     merger.finish().await?;
//     Ok(())
// }

#[tokio::test]
async fn loader_tst() -> Result<(), Error> {
    // let mut reader = BufReader::new(File::open("./res/dictionary.txt").await?);
    // dbg!(IndexedCursor::load(&mut reader).await?);
    // dbg!(IndexedCursor::load(&mut reader).await?);
    // dbg!(IndexedCursor::load(&mut reader).await?);
    // dbg!(IndexedCursor::load(&mut reader).await?);

    let mut reader = IndexTermProvider::new(&"./res".to_string()).await?;
    dbg!(reader.next_term().await);
    let n = reader.next_term().await.unwrap();
    println!("{}", n.indexes.len());
    for i in n.indexes.iter() {
        println!("{}", i.0);
    }
    // dbg!(reader.next_term().await);
    // dbg!(reader.next_term().await);
    // dbg!(reader.next_term().await);
    // dbg!(reader.next_term().await);
    Ok(())
}

#[tokio::test]
async fn loader_tst_buff() -> Result<(), Error> {
    // let mut reader = BufReader::new(File::open("./res/dictionary.txt").await?);
    // dbg!(IndexedCursor::load(&mut reader).await?);
    // dbg!(IndexedCursor::load(&mut reader).await?);
    // dbg!(IndexedCursor::load(&mut reader).await?);
    // dbg!(IndexedCursor::load(&mut reader).await?);

    let mut reader = IndexTermProvider::new(&"./buffer/11".to_string()).await?;
    dbg!(reader.remaining_size);
    dbg!(reader.next_term().await);
    dbg!(reader.next_term().await);
    dbg!(reader.next_term().await);
    dbg!(reader.remaining_size);
    dbg!(reader.next_term().await);
    dbg!(reader.next_term().await);
    dbg!(reader.next_term().await);
    dbg!(reader.next_term().await);
    dbg!(reader.next_term().await);
    Ok(())
}

#[tokio::test]
async fn reader_tst() -> Result<(), Error> {
    let mut wr = CountedWriter::new(BufWriter::new(File::create("./res/tar.txt").await?));
    wr.push_u64(3).await?;
    wr.push_u64(5).await?;
    wr.push_u64(6).await?;
    wr.flush().await?;
    wr.goto(0).await?;
    wr.push_u64(10).await?;
    wr.flush().await?;
    Ok(())
}

const fn tra() {
    let b = 2;
    // let kra = f"{b}";
}
