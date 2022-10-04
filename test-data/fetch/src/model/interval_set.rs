use serde::de::SeqAccess;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};
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

impl<V: num::Integer + Copy> From<(V, V)> for Interval<V> {
    fn from((start, end): (V, V)) -> Self {
        Self::from_start_and_end(start, end)
    }
}

impl<V: num::Integer + Copy> Into<(V, V)> for Interval<V> {
    fn into(self) -> (V, V) {
        (self.start, self.end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetNode {
    Start,
    End,
}

/// A set of intervals.
///
/// Invariant: no two intervals in the set are intersecting.
/// Stores the actual intervals as a balanced binary tree of start and end points.
/// See https://stackoverflow.com/a/1983402/14747973 or idk
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalSet<V: num::Integer + Copy> {
    intervals: BTreeMap<V, SetNode>,
}

impl<V: num::Integer + Debug + Copy + Serialize> Serialize for IntervalSet<V> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.intervals.len() / 2))?;
        for e in self.iter() {
            let e: (V, V) = e.into();
            seq.serialize_element(&e)?;
        }
        seq.end()
    }
}

struct IntervalSetVisitor<'de, V: num::Integer + Debug + Copy + Deserialize<'de>> {
    phantom: std::marker::PhantomData<&'de V>,
}

impl<'de, V: num::Integer + Debug + Copy + Deserialize<'de>> serde::de::Visitor<'de>
    for IntervalSetVisitor<'de, V>
{
    type Value = IntervalSet<V>;

    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        formatter.write_str("a sequence of intervals")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut res = IntervalSet::new();

        while let Some(interval) = seq.next_element::<(V, V)>()? {
            res.push(interval.into());
        }

        Ok(res)
    }
}

impl<'de, V: num::Integer + Debug + Copy + Deserialize<'de> + 'de> Deserialize<'de>
    for IntervalSet<V>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(IntervalSetVisitor {
            phantom: std::marker::PhantomData,
        })
    }
}

impl<V: num::Integer + Debug + Copy> IntervalSet<V> {
    pub fn new() -> Self {
        Self {
            intervals: BTreeMap::new(),
        }
    }

    /// Add an interval to the set, merging it with any intersecting intervals.
    pub fn push(&mut self, interval: Interval<V>) {
        // nasty edge case
        if interval.is_empty() {
            return;
        }

        // calculate before doing anything
        let contains_start = self.contains(interval.start);
        let contains_end = self.contains(interval.end);

        // insert start point
        let start_entry = self.intervals.entry(interval.start);
        match start_entry {
            Entry::Vacant(v) => {
                // if the start point does not fall into an existing interval - insert it
                if !contains_start {
                    v.insert(SetNode::Start);
                }
            }
            Entry::Occupied(o) if o.get() == &SetNode::Start => {
                // if the start point is present at the exact same place - do nothing
            }
            Entry::Occupied(_) => {
                // remove the end point if present
                self.intervals.remove(&interval.start);
            }
        }

        // insert end point
        let end_entry = self.intervals.entry(interval.end);
        match end_entry {
            Entry::Vacant(v) => {
                // if the end point does not fall into an existing interval - insert it
                if !contains_end {
                    v.insert(SetNode::End);
                }
            }
            Entry::Occupied(o) if o.get() == &SetNode::End => {
                // if the end point is present at the exact same place - do nothing
            }
            Entry::Occupied(_) => {
                // remove the start point if present
                self.intervals.remove(&interval.end);
            }
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

        self.check_iter();
    }

    /// Remove an interval from the set, splitting other intervals if needed
    pub fn remove(&mut self, interval: Interval<V>) {
        // nasty edge case
        if interval.is_empty() {
            return;
        }

        // this is similar to push, but we reverse some stuff

        // calculate before doing anything
        let contains_start = self.contains(interval.start);
        let contains_end = self.contains(interval.end);

        // insert end point
        let start_entry = self.intervals.entry(interval.start);
        match start_entry {
            Entry::Vacant(v) => {
                // if the end point does not fall into an existing interval - insert it
                if contains_start {
                    v.insert(SetNode::End);
                }
            }
            Entry::Occupied(o) if o.get() == &SetNode::End => {
                // if the end point is present at the exact same place - do nothing
            }
            Entry::Occupied(_) => {
                // remove the start point if present
                self.intervals.remove(&interval.start);
            }
        }

        // insert start point
        let end_entry = self.intervals.entry(interval.end);
        match end_entry {
            Entry::Vacant(v) => {
                // if the start point does not fall into an existing interval - insert it
                if contains_end {
                    v.insert(SetNode::Start);
                }
            }
            Entry::Occupied(o) if o.get() == &SetNode::Start => {
                // if the start point is present at the exact same place - do nothing
            }
            Entry::Occupied(_) => {
                // remove the end point if present
                self.intervals.remove(&interval.end);
            }
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

        self.check_iter();
    }

    pub fn contains(&self, value: V) -> bool {
        matches!(
            self.intervals.range(..value).last(),
            Some((_, SetNode::Start))
        )
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

    /// Checks that the invariants hold.
    #[allow(unused)]
    fn check_iter(&self) {
        self.iter().for_each(|_| {});
    }
}

pub struct IntervalSetIter<'a, V: num::Integer + Copy> {
    inner: std::collections::btree_map::Iter<'a, V, SetNode>,
}

impl<'a, V: num::Integer + Copy> Iterator for IntervalSetIter<'a, V> {
    type Item = Interval<V>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            None => None,
            Some((&start_pos, &start_val)) => {
                assert_eq!(start_val, SetNode::Start);
                let (&end_pos, &end_val) = self.inner.next().expect("BUG: end point not found");
                assert_eq!(end_val, SetNode::End);

                debug_assert!(start_pos <= end_pos);
                Some(Interval {
                    start: start_pos,
                    end: end_pos,
                })
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (lo, hi) = self.inner.size_hint();
        (lo / 2, hi.map(|v| v / 2))
    }
}

#[cfg(test)]
mod test {
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

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(1, 1));
        set.push(Interval::from_start_and_len(1, 0));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Interval::from_start_and_len(1, 1)]
        );

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(2, 1));
        set.push(Interval::from_start_and_len(1, 2));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Interval::from_start_and_len(1, 2)]
        );

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(1, 2));
        set.push(Interval::from_start_and_len(2, 1));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![Interval::from_start_and_len(1, 2)]
        );
    }

    #[test]
    pub fn test_interval_set_serde() {
        use super::{Interval, IntervalSet};
        use serde_json;

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(1, 2));
        set.push(Interval::from_start_and_len(3, 3));
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, "[[1,6]]");
        let set2: IntervalSet<u32> = serde_json::from_str(&json).unwrap();
        assert_eq!(set, set2);

        let mut set = IntervalSet::<u32>::new();
        set.push(Interval::from_start_and_len(4, 5));
        set.push(Interval::from_start_and_len(1, 1));
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, "[[1,2],[4,9]]");
        let set2: IntervalSet<u32> = serde_json::from_str(&json).unwrap();
        assert_eq!(set, set2);
    }
}
