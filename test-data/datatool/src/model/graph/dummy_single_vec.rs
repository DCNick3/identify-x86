use smallvec::{smallvec, SmallVec};

type Item = u32;

/// A Vec type optimized for the case where there is only one u32 element.
#[derive(Clone, Default)]
pub struct SingleVec {
    inner: SmallVec<[Item; 2]>,
}

impl SingleVec {
    pub fn new() -> Self {
        Self { inner: smallvec![] }
    }

    pub fn from_single(value: Item) -> Self {
        Self {
            inner: smallvec![value],
        }
    }

    pub fn from_vec(vec: Vec<Item>) -> Self {
        Self {
            inner: SmallVec::from_vec(vec),
        }
    }

    pub fn push(&mut self, value: Item) {
        self.inner.push(value);
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn contains(&self, value: Item) -> bool {
        self.inner.contains(&value)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn iter(&self) -> Iter {
        Iter {
            inner: self.inner.iter(),
        }
    }
}

pub struct Iter<'a> {
    inner: std::slice::Iter<'a, Item>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied()
    }
}
