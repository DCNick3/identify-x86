use std::collections::BTreeMap;
use std::mem;

/// Represents a half-interval [start, end)
///
/// Invariant: start <= end
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Interval<V: num::Integer + Copy> {
    start: V,
    end: V,
}

impl<V: num::Integer + Copy> Interval<V> {
    pub fn from_start_and_end(start: V, end: V) -> Self {
        assert!(start <= end);
        Interval { start, end }
    }

    pub fn from_start_and_len(start: V, len: V) -> Self {
        assert!(len >= V::zero());
        Interval {
            start,
            end: start + len,
        }
    }

    pub fn start(&self) -> V {
        self.start
    }

    pub fn end(&self) -> V {
        self.end
    }

    pub fn len(&self) -> V {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.len() == V::zero()
    }

    pub fn intersection(&self, other: Self) -> Self {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if end < start {
            Self::from_start_and_len(V::zero(), V::zero())
        } else {
            Self { start, end }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntervalSetNode {
    Start,
    End,
}

/// A set of intervals.
///
/// Invariant: no two intervals in the set are intersecting.
/// Stores the actual intervals as a balanced binary tree of start and end points.
/// See https://stackoverflow.com/a/1983402/14747973 or idk
#[derive(Debug, Clone)]
pub struct IntervalSet<V: num::Integer + Copy> {
    intervals: BTreeMap<V, IntervalSetNode>,
}

impl<V: num::Integer + Copy> IntervalSet<V> {
    pub fn new() -> Self {
        IntervalSet {
            intervals: BTreeMap::new(),
        }
    }

    /// Add an interval to the set, merging it with any intersecting intervals.
    pub fn push(&mut self, interval: Interval<V>) {
        // insert start point
        // if there is an end point at the same position - delete it, merging the intervals
        let start_entry = self.intervals.entry(interval.start);
        if *start_entry.or_insert(IntervalSetNode::Start) == IntervalSetNode::End {
            self.intervals.remove_entry(&interval.start);
        }

        // insert end point
        // if there is an start point at the same position - delete it, merging the intervals
        let end_entry = self.intervals.entry(interval.end);
        if *end_entry.or_insert(IntervalSetNode::End) == IntervalSetNode::Start {
            self.intervals.remove_entry(&interval.end);
        }

        // iterate over the inner points and remove them
        // we have to collect them first because rust doesn't allow us to modify the map while iterating
        // we could implement our own tree structure to avoid this, but it's not worth it
        let rm_keys = self
            .intervals
            .range(interval.start.add(V::one())..interval.end)
            .map(|(k, _)| *k)
            .collect::<smallvec::SmallVec<[V; 8]>>();

        for k in rm_keys {
            self.intervals.remove(&k);
        }
    }

    /// Shifts all intervals by the given offset.
    pub fn shift(&mut self, offset: V) {
        let old_intervals = mem::take(&mut self.intervals);
        for (k, v) in old_intervals {
            self.intervals.insert(k + offset, v);
        }
    }

    /// Pushes all the intervals returned by the given iterator.
    pub fn extend(&mut self, other: impl IntoIterator<Item = Interval<V>>) {
        for interval in other.into_iter() {
            self.push(interval);
        }
    }

    pub fn iter(&self) -> IntervalSetIter<'_, V> {
        IntervalSetIter {
            inner: self.intervals.iter(),
        }
    }
}

pub struct IntervalSetIter<'a, V: num::Integer + Copy> {
    inner: std::collections::btree_map::Iter<'a, V, IntervalSetNode>,
}

impl<'a, V: num::Integer + Copy> Iterator for IntervalSetIter<'a, V> {
    type Item = Interval<V>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            None => None,
            Some((&start_pos, &start_val)) => {
                assert_eq!(start_val, IntervalSetNode::Start);
                let (&end_pos, &end_val) = self.inner.next().expect("BUG: end point not found");
                assert_eq!(end_val, IntervalSetNode::End);

                debug_assert!(start_pos <= end_pos);
                Some(Interval {
                    start: start_pos,
                    end: end_pos,
                })
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::model::interval_set::Interval;

    #[test]
    pub fn test_interval() {
        use super::Interval;

        let interval = Interval::from_start_and_end(1, 3);
        assert_eq!(interval.start(), 1);
        assert_eq!(interval.end(), 3);
        assert_eq!(interval.len(), 2);

        let interval = Interval::from_start_and_len(1, 2);
        assert_eq!(interval.start(), 1);
        assert_eq!(interval.end(), 3);
        assert_eq!(interval.len(), 2);

        let interval = Interval::from_start_and_len(3, 2);
        assert_eq!(interval.start(), 3);
        assert_eq!(interval.end(), 5);
        assert_eq!(interval.len(), 2);

        let interval = Interval::from_start_and_end(1, 1);
        assert_eq!(interval.start(), 1);
        assert_eq!(interval.end(), 1);
        assert_eq!(interval.len(), 0);

        let interval = Interval::from_start_and_len(1, 0);
        assert_eq!(interval.start(), 1);
        assert_eq!(interval.end(), 1);
        assert_eq!(interval.len(), 0);
    }

    #[test]
    pub fn test_interval_set() {
        use super::{Interval, IntervalSet};

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(1, 2));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Interval::from_start_and_len(1, 2)]
        );

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(1, 2));
        set.push(Interval::from_start_and_len(1, 3));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Interval::from_start_and_len(1, 3)]
        );

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(1, 2));
        set.push(Interval::from_start_and_len(3, 3));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Interval::from_start_and_len(1, 5)]
        );

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(3, 3));
        set.push(Interval::from_start_and_len(1, 2));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Interval::from_start_and_len(1, 5)]
        );

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(2, 1));
        set.push(Interval::from_start_and_len(1, 3));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Interval::from_start_and_len(1, 3)]
        );
    }
}
