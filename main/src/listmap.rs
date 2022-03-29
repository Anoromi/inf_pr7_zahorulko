use std::{
    collections::LinkedList,
    io::Error,
    mem,
    ops::{Index, IndexMut},
    ptr::NonNull,
};

use async_trait::async_trait;
use save::save::VariableSave;
use tokio::{
    fs::File,
    io::{BufReader, BufWriter},
};

use save::writer::{variable_load, variable_save_usize};

#[derive(Debug)]
struct Value<T, G>(T, G, Option<Box<Value<T, G>>>);
#[derive(Debug)]
pub struct SortedLinkedMap<T: Ord, G> {
    start: Option<Box<Value<T, G>>>,
    size: usize,
}

impl<T: Ord, G> SortedLinkedMap<T, G> {
    pub fn new() -> Self {
        Self {
            start: None,
            size: 0,
        }
    }
    fn addFirst(&mut self, key: T, value: G) {
        self.start = Some(Box::new(Value(key, value, None)));
        self.size += 1;
    }

    pub fn push(&mut self, key: T, value: G) {
        let current = &mut self.start;
        match current {
            None => {
                self.addFirst(key, value);
                return;
            }
            Some(current) => {
                let mut current = (*current).as_mut();
                while current.2.is_some() {
                    if key <= current.0 {
                        break;
                    }
                    current = current.2.as_mut().unwrap().as_mut();
                }
                match &mut current.2 {
                    Some(_) if key < current.0 => {
                        let k = mem::replace(&mut current.0, key);
                        let v = mem::replace(&mut current.1, value);
                        let w = mem::replace(&mut current.2, Some(Box::new(Value(k, v, None))));
                        current.2.as_mut().unwrap().2 = w;
                        self.size += 1;
                    }
                    Some(_) => {}
                    None => {
                        if key < current.0 {
                            let k = mem::replace(&mut current.0, key);
                            let v = mem::replace(&mut current.1, value);
                            let w = mem::replace(&mut current.2, Some(Box::new(Value(k, v, None))));
                            current.2.as_mut().unwrap().2 = w;
                        } else if key > current.0 {
                            current.2 = Some(Box::new(Value(key, value, None)));
                        }
                        self.size += 1;
                    }
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn iter(mut self) -> LinkedMapIterator<T, G> {
        LinkedMapIterator {
            current: self.start.take(),
        }
    }

    pub fn iter_mut(&mut self) -> RefLinkedMapIterator<'_, T, G> {
        RefLinkedMapIterator {
            current: self.start.as_deref_mut(),
        }
    }

    pub fn or(&mut self, mut oth: SortedLinkedMap<T, G>, map: fn(&mut G, &mut G) -> ()) {
        if self.len() == 0 {}
        let mut fc = self.start.as_mut().unwrap();
        let mut sc = oth.start.take();
        'outer: while sc.is_some() {
            while sc.is_some() && fc.as_ref().0 > sc.as_ref().unwrap().0 {
                let mfc = fc;
                let msc = sc.unwrap();
                let key = mem::replace(&mut mfc.0, msc.0);
                let value = mem::replace(&mut mfc.1, msc.1);
                let next = mfc.2.take();
                mfc.2 = Some(Box::new(Value(key, value, next)));

                sc = msc.2;
                self.size += 1;
                match mfc.2 {
                    Some(ref mut v) => fc = v,
                    None => {
                        fc = mfc;
                        break 'outer;
                    }
                }
            }
            if sc.is_some() && fc.as_ref().0 == sc.as_ref().unwrap().0 {
                map(&mut fc.1, &mut sc.as_mut().unwrap().1);
                sc = sc.unwrap().2;
            }

            if let Some(ref mut k) = fc.2 {
                fc = k;
            } else {
                break;
            }
        }
        while sc.is_some() {
            let usc = sc.unwrap();
            fc.2 = Some(Box::new(Value(usc.0, usc.1, None)));
            sc = usc.2;
            self.size += 1;
        }
    }

    pub fn element_at(&self, index: T) -> Option<&G> {
        let mut next = &self.start;
        while let Some(v) = next {
            match v.0.cmp(&index) {
                std::cmp::Ordering::Less => {
                    next = &v.2;
                }
                std::cmp::Ordering::Equal => {
                    return Some(&v.1);
                }
                std::cmp::Ordering::Greater => {
                    return None;
                }
            }
        }
        None
    }

    pub fn element_at_mut(&mut self, index: T) -> Option<&mut G> {
        let mut next = &mut self.start;
        while let Some(v) = next {
            match v.0.cmp(&index) {
                std::cmp::Ordering::Less => {
                    next = &mut v.2;
                }
                std::cmp::Ordering::Equal => {
                    return Some(&mut v.1);
                }
                std::cmp::Ordering::Greater => {
                    return None;
                }
            }
        }
        None
    }
}

impl<T: Ord, G> Index<T> for SortedLinkedMap<T, G> {
    type Output = G;

    fn index(&self, index: T) -> &Self::Output {
        self.element_at(index).unwrap()
    }
}

impl<T: Ord, G> IndexMut<T> for SortedLinkedMap<T, G> {
    fn index_mut(&mut self, index: T) -> &mut Self::Output {
        self.element_at_mut(index).unwrap()
    }
}

impl<T: Ord, G> Drop for SortedLinkedMap<T, G> {
    fn drop(&mut self) {
        let mut c = self.start.take();
        while let Some(mut n) = c {
            c = n.2.take();
        }
    }
}

pub struct RefLinkedMapIterator<'a, T, G> {
    current: Option<&'a mut Value<T, G>>,
}
impl<'a, T: Ord, G> Iterator for RefLinkedMapIterator<'a, T, G> {
    type Item = (&'a mut T, &'a mut G);

    fn next(&mut self) -> Option<Self::Item> {
        self.current.take().map(|node| {
            self.current = node.2.as_deref_mut();
            (&mut node.0, &mut node.1)
        })
    }
}

pub struct LinkedMapIterator<T, G> {
    current: Option<Box<Value<T, G>>>,
}
impl<T: Ord, G> Iterator for LinkedMapIterator<T, G> {
    type Item = (T, G);

    fn next(&mut self) -> Option<Self::Item> {
        let c = self.current.take();
        match c {
            Some(c) => {
                self.current = c.2;
                Some((c.0, c.1))
            }
            None => None,
        }
    }
}

#[async_trait]
impl<S: VariableSave + Send + Sync> VariableSave for SortedLinkedMap<usize, S> {
    async fn variable_save(&mut self, writer: &mut BufWriter<File>) -> Result<usize, Error> {
        let mut passed = variable_save_usize(self.len(), writer).await? as usize;
        let mut iter = self.iter_mut();
        let (mut v, mut vs) = iter.next().unwrap();
        passed += variable_save_usize(*v, writer).await? as usize;
        for (i, s) in iter {
            passed += variable_save_usize((*i) - (*v), writer).await? as usize;
            passed += vs.variable_save(writer).await?;
            v = i;
            vs = s;
        }
        Ok(passed)
    }

    async fn variable_load(
        reader: &mut BufReader<File>,
    ) -> Result<SortedLinkedMap<usize, S>, Error> {
        let mut list = SortedLinkedMap::<usize, S>::new();

        let size = variable_load(reader).await?;
        if size > 0 {
            let mut previous = variable_load(reader).await?;
            list.push(previous, S::variable_load(reader).await?);
            for _ in 0..size - 1 {
                previous += variable_load(reader).await?;
                list.push(previous, S::variable_load(reader).await?);
            }
        }
        Ok(list)
    }
}
