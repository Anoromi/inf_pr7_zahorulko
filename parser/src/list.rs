use std::{io::Error, mem};

use tokio::{
    fs::File,
    io::{AsyncWriteExt, BufReader, BufWriter},
};

use save::writer::{variable_load, variable_save_usize};
#[derive(Debug)]
struct Value<T>(T, Option<Box<Value<T>>>);
#[derive(Debug)]
pub struct SortedLinkedList<T: Ord> {
    start: Option<Box<Value<T>>>,
    size: usize,
}

impl<T: Ord> SortedLinkedList<T> {
    pub fn new() -> Self {
        Self {
            start: None,
            size: 0,
        }
    }
    fn addFirst(&mut self, value: T) {
        self.start = Some(Box::new(Value(value, None)));
        self.size += 1;
    }

    pub fn push(&mut self, value: T) {
        let current = &mut self.start;
        match current {
            None => {
                self.addFirst(value);
                return;
            }
            Some(current) => {
                let mut current = (*current).as_mut();
                while current.1.is_some() {
                    if value <= current.0 {
                        break;
                    }
                    current = current.1.as_mut().unwrap().as_mut();
                }
                match &mut current.1 {
                    Some(_) if value < current.0 => {
                        let v = mem::replace(&mut current.0, value);
                        let w = mem::replace(&mut current.1, Some(Box::new(Value(v, None))));
                        current.1.as_mut().unwrap().1 = w;
                        self.size += 1;
                    }
                    Some(_) => {}
                    None => {
                        if value < current.0 {
                            let v = mem::replace(&mut current.0, value);
                            let w = mem::replace(&mut current.1, Some(Box::new(Value(v, None))));
                            current.1.as_mut().unwrap().1 = w;
                            self.size += 1;
                        } else if value > current.0 {
                            current.1 = Some(Box::new(Value(value, None)));
                            self.size += 1;
                        }
                    }
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn iter(mut self) -> LinkedListIterator<T> {
        LinkedListIterator {
            current: self.start.take(),
        }
    }

    pub fn or(&mut self, mut oth: SortedLinkedList<T>) {
        if self.len() == 0 {}
        let mut fc = self.start.as_mut().unwrap();
        let mut sc = oth.start.take();
        'outer: while sc.is_some() {
            while sc.is_some() && fc.as_ref().0 > sc.as_ref().unwrap().0 {
                let mfc = fc;
                let msc = sc.unwrap();
                let val = mem::replace(&mut mfc.0, msc.0);
                let ne = mfc.1.take();
                mfc.1 = Some(Box::new(Value(val, ne)));

                sc = msc.1;
                self.size += 1;
                match mfc.1 {
                    Some(ref mut v) => fc = v,
                    None => {
                        fc = mfc;
                        break 'outer;
                    }
                }
            }
            if sc.is_some() && fc.as_ref().0 == sc.as_ref().unwrap().0 {
                sc = sc.unwrap().1;
            }

            if let Some(ref mut k) = fc.1 {
                fc = k;
            } else {
                break;
            }
        }
        while sc.is_some() {
            let usc = sc.unwrap();
            fc.1 = Some(Box::new(Value(usc.0, None)));
            sc = usc.1;
            self.size += 1;
        }
    }
}

impl<T: Ord> Drop for SortedLinkedList<T> {
    fn drop(&mut self) {
        let mut c = self.start.take();
        while let Some(mut n) = c {
            c = n.1.take();
        }
    }
}

pub struct LinkedListIterator<T> {
    current: Option<Box<Value<T>>>,
}
impl<T: Ord> Iterator for LinkedListIterator<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let c = self.current.take();
        match c {
            Some(c) => {
                let res = c.0;
                self.current = c.1;
                Some(res)
            }
            None => None,
        }
    }
}

impl SortedLinkedList<usize> {
    pub async fn save(self, writer: &mut BufWriter<File>) -> Result<usize, Error> {
        let mut passed = variable_save_usize(self.len(), writer).await? as usize;
        let mut iter = self.iter();
        let mut v = iter.next().unwrap();
        passed += variable_save_usize(v, writer).await? as usize;
        for i in iter {
            passed += variable_save_usize(i - v, writer).await? as usize;
            v = i;
        }
        Ok(passed)
    }

    pub async fn load(reader: &mut BufReader<File>) -> Result<SortedLinkedList<usize>, Error> {
        let mut list = SortedLinkedList::<usize>::new();
        let size = variable_load(reader).await?;
        if size > 0 {
            let mut previous = variable_load(reader).await?;
            list.push(previous);
            for _ in 0..size - 1 {
                previous += variable_load(reader).await?;
                list.push(previous);
            }
        }
        Ok(list)
    }
}

#[test]
fn lst_tst() {
    let mut f = SortedLinkedList::<i32>::new();

    f.push(6);
    f.push(3);
    f.push(1);
    f.push(3);
    f.push(4);

    let mut s = SortedLinkedList::<i32>::new();

    s.push(2);
    s.push(10);
    s.push(3);

    s.or(f);
    f = s;

    println!("{}\n", f.size);
    // s.iter().for_each(|v| {
    //     println!("{}", v);
    // })
    for i in f.iter().collect::<Vec<i32>>().into_iter().rev() {
        println!("{}", i);
    }
}
#[tokio::test]
async fn write_tst() -> Result<(), Error> {
    let mut buf = BufWriter::new(File::create("tst/tar.txt").await?);

    let mut f = SortedLinkedList::<usize>::new();

    f.push(6);
    f.push(3);
    f.push(1);
    f.push(3);
    f.push(4);

    let mut s = SortedLinkedList::<usize>::new();

    s.push(2);
    s.push(10);
    s.push(3);

    s.or(f);
    // f = s;

    s.save(&mut buf).await?;
    buf.flush().await?;
    Ok(())
}

#[tokio::test]
async fn read_tst() -> Result<(), Error> {
    async fn next(buf: &mut BufReader<File>) -> Result<(), Error> {
        let f = SortedLinkedList::<usize>::load(buf).await?;
        for v in f.iter() {
            println!("{}", v);
        }
        Ok(())
    }
    let mut buf = BufReader::new(File::open("res/index_part.txt").await?);

    for _ in 0..4 {
        next(&mut buf).await?;
        println!();
    }
    Ok(())
}
