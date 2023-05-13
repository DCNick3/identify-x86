use float_ord::FloatOrd;
use memory_image::MemoryImage;
use rustc_hash::FxHashMap;

// it's actually just a multi-set of N-grams
pub struct NGramIndex<const N: usize> {
    ngrams: FxHashMap<[u8; N], usize>,
    total_count: usize,
}

impl<const N: usize> NGramIndex<N> {
    pub fn new(program: &MemoryImage) -> Self {
        let mut ngrams = FxHashMap::default();
        let mut total_count = 0;

        for item in program.iter() {
            for window in item.data.windows(N) {
                let ngram: [u8; N] = window.try_into().unwrap();
                total_count += 1;
                *ngrams.entry(ngram).or_insert(0) += 1;
            }
        }

        NGramIndex {
            ngrams,
            total_count,
        }
    }

    pub fn similarity(&self, other: &NGramIndex<N>) -> f64 {
        // compute Jacard similarity: |A ∩ B| / |A ∪ B|
        let mut intersection = 0;
        for (ngram, count) in &self.ngrams {
            if let Some(other_count) = other.ngrams.get(ngram) {
                intersection += count.min(other_count);
            }
        }

        let union = self.total_count + other.total_count - intersection;

        intersection as f64 / union as f64
    }

    pub fn len(&self) -> usize {
        self.ngrams.len()
    }
}

pub type Index = usize;

struct SplitGroup {
    target_fraction: f64,
    current_size: u64,
    items: Vec<Index>,
}

pub struct SplitBuilder {
    current_size: u64,
    groups: Vec<SplitGroup>,
}

#[derive(Debug)]
pub struct SplitGroupResult {
    pub target_fraction: f64,
    pub actual_fraction: f64,
    pub items: Vec<Index>,
}

impl SplitBuilder {
    pub fn new(groups: &[f64]) -> Self {
        let groups = groups
            .iter()
            .map(|&target_fraction| SplitGroup {
                target_fraction,
                current_size: 0,
                items: Vec::new(),
            })
            .collect();

        SplitBuilder {
            current_size: 0,
            groups,
        }
    }

    pub fn push_component(&mut self, items: impl Iterator<Item = Index>, group_size: u64) {
        let target_group = self
            .groups
            .iter()
            .map(|group| {
                if group.current_size == 0 {
                    // negative - we are empty
                    // the larger the target fraction, the larger the loss (to prioritize larger groups in the beginning)
                    -1.0 - group.target_fraction
                } else {
                    let current_fraction = group.current_size as f64 / self.current_size as f64;
                    current_fraction - group.target_fraction
                }
            })
            .enumerate()
            .min_by_key(|&(_, v)| FloatOrd(v))
            .unwrap()
            .0;

        self.groups[target_group].current_size += group_size;
        self.current_size += group_size;

        self.groups[target_group].items.extend(items);
    }

    pub fn build(self) -> Vec<SplitGroupResult> {
        let total_size = self.current_size;

        self.groups
            .into_iter()
            .map(|group| SplitGroupResult {
                target_fraction: group.target_fraction,
                actual_fraction: group.current_size as f64 / total_size as f64,
                items: group.items,
            })
            .collect()
    }
}
