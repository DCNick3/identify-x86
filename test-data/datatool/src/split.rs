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
