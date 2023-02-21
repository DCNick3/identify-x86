#![allow(unstable_name_collisions)]
use sptr::Strict;

type Item = u32;

const SINGLE_BIT: usize = 1;
const MAX_VALUE: usize = 1 << (usize::BITS - 1) - 1;

/// A Vec type optimized for the case where there is only one u32 element.
pub struct SingleVec {
    inner: Option<*mut Vec<Item>>,
}

fn single_into_ptr(value: Item) -> *mut Vec<Item> {
    // dbg_hex!(SINGLE_BIT);
    // dbg_hex!(usize::BITS);
    let raw_ptr = ((value as usize) << 1) | SINGLE_BIT;
    // dbg_hex!(raw_ptr);
    sptr::invalid_mut(raw_ptr)
}
fn ptr_is_single(ptr: *const Vec<Item>) -> bool {
    let raw_ptr = Strict::addr(ptr);
    raw_ptr & SINGLE_BIT != 0
}
fn ptr_into_single(ptr: *const Vec<Item>) -> Item {
    let raw_ptr = Strict::addr(ptr);
    assert_ne!(raw_ptr & SINGLE_BIT, 0);
    ((raw_ptr & !SINGLE_BIT) >> 1) as Item
}

enum ReprRef {
    Empty,
    Single(Item),
    Vec(*const Vec<Item>),
}

enum ReprMut {
    Empty,
    Single(Item),
    Vec(*mut Vec<Item>),
}

impl ReprMut {
    pub fn from_vec(vec: Box<Vec<Item>>) -> Self {
        let ptr = Box::into_raw(vec);
        Self::Vec(ptr)
    }

    pub fn drop(self) {
        match self {
            Self::Empty => {}
            Self::Single(_) => {}
            Self::Vec(ptr) => unsafe { drop(Box::from_raw(ptr)) },
        }
    }
}

impl SingleVec {
    pub fn new() -> Self {
        Self {
            inner: Self::ptr_from_repr_mut(ReprMut::Empty),
        }
    }

    pub fn from_single(value: Item) -> Self {
        Self {
            inner: Self::ptr_from_repr_mut(ReprMut::Single(value)),
        }
    }

    pub fn from_vec(vec: Vec<Item>) -> Self {
        Self {
            inner: Self::ptr_from_repr_mut(ReprMut::from_vec(Box::new(vec))),
        }
    }

    fn repr(&self) -> ReprRef {
        match self.inner {
            None => ReprRef::Empty,
            Some(ptr) => {
                if ptr_is_single(ptr) {
                    ReprRef::Single(ptr_into_single(ptr))
                } else {
                    ReprRef::Vec(ptr)
                }
            }
        }
    }

    /// Safety: don't leak the returned pointer!
    unsafe fn repr_mut(&mut self) -> ReprMut {
        match self.inner {
            None => ReprMut::Empty,
            Some(ptr) => {
                if ptr_is_single(ptr) {
                    ReprMut::Single(ptr_into_single(ptr))
                } else {
                    ReprMut::Vec(ptr)
                }
            }
        }
    }

    fn ptr_from_repr_mut(repr: ReprMut) -> Option<*mut Vec<Item>> {
        match repr {
            ReprMut::Empty => None,
            ReprMut::Single(value) => {
                assert!(value as u64 <= MAX_VALUE as u64);
                Some(single_into_ptr(value))
            }
            ReprMut::Vec(ptr) => {
                assert_eq!(Strict::addr(ptr) & SINGLE_BIT, 0, "unaligned ptr???");
                Some(ptr)
            }
        }
    }

    pub fn push(&mut self, value: Item) {
        let new_repr = match unsafe { self.repr_mut() } {
            ReprMut::Empty => ReprMut::Single(value),
            ReprMut::Single(s) => ReprMut::from_vec(Box::new(vec![s, value])),
            ReprMut::Vec(v) => {
                let mut vec = unsafe { Box::from_raw(v) };
                vec.push(value);
                ReprMut::from_vec(vec)
            }
        };
        self.inner = Self::ptr_from_repr_mut(new_repr);
    }

    pub fn clear(&mut self) {
        unsafe { self.repr_mut() }.drop();
        *self = Self::new();
    }

    pub fn contains(&self, value: Item) -> bool {
        match self.repr() {
            ReprRef::Empty => false,
            ReprRef::Single(s) => s == value,
            ReprRef::Vec(v) => unsafe { &*v }.contains(&value),
        }
    }

    pub fn len(&self) -> usize {
        match self.repr() {
            ReprRef::Empty => 0,
            ReprRef::Single(_) => 1,
            ReprRef::Vec(v) => unsafe { &*v }.len(),
        }
    }

    pub fn iter(&self) -> Iter {
        match self.repr() {
            ReprRef::Empty => Iter::Empty,
            ReprRef::Single(value) => Iter::Single(value),
            ReprRef::Vec(ptr) => Iter::Vec(unsafe { &*ptr }.iter()),
        }
    }
}

impl Default for SingleVec {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SingleVec {
    fn clone(&self) -> Self {
        match self.repr() {
            ReprRef::Empty => Self::new(),
            ReprRef::Single(value) => Self::from_single(value),
            ReprRef::Vec(ptr) => Self::from_vec(unsafe { &*ptr }.clone()),
        }
    }
}

impl Drop for SingleVec {
    fn drop(&mut self) {
        unsafe { self.repr_mut().drop() }
    }
}

pub enum Iter<'a> {
    Empty,
    Single(Item),
    Vec(std::slice::Iter<'a, Item>),
}

impl<'a> Iterator for Iter<'a> {
    type Item = Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Empty => None,
            Self::Single(value) => {
                let value = *value;
                *self = Self::Empty;
                Some(value)
            }
            Self::Vec(iter) => iter.next().copied(),
        }
    }
}
