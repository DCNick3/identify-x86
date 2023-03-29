use crate::model::SupersetSample;
use anyhow::Result;
use iced_x86::Code;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::io::{BufWriter, Write};
use std::ops::Index;

#[derive(Clone)]
pub struct CodeVocabBuilder {
    freq: HashMap<Code, usize>,
}

impl CodeVocabBuilder {
    pub fn new() -> Self {
        Self {
            freq: HashMap::new(),
        }
    }

    #[inline]
    pub fn add(&mut self, code: Code) {
        *self.freq.entry(code).or_insert(0) += 1;
    }

    pub fn add_sample(&mut self, sample: &SupersetSample) {
        for (_, instr, _) in sample.superset.iter() {
            self.add(instr.code);
        }
    }

    pub fn merge(&mut self, other: Self) {
        for (code, freq) in other.freq.iter() {
            *self.freq.entry(*code).or_insert(0) += freq;
        }
    }

    #[allow(unused)]
    pub fn len(&self) -> usize {
        self.freq.len()
    }

    #[allow(unused)]
    pub fn total_count(&self) -> usize {
        self.freq.values().sum()
    }

    pub fn build_top_k(mut self, k: usize) -> CodeVocab {
        self.freq.remove(&Code::INVALID);
        let codes = self
            .freq
            .into_iter()
            .sorted_unstable_by_key(|(c, f)| (Reverse(*f), *c))
            .map(|(c, _)| c)
            .take(k)
            .collect::<Vec<_>>();
        CodeVocab::new(codes)
    }

    #[allow(unused)]
    pub fn build(self) -> CodeVocab {
        let len = self.len();
        self.build_top_k(len)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CodeVocab {
    codes: Vec<Code>,
    code_to_index: HashMap<Code, usize>,
}

impl CodeVocab {
    const SPECIAL_VALUE_COUNT: usize = 2;
    const INVALID: usize = 0;
    const UNKNOWN: usize = 1;

    pub fn new(codes: Vec<Code>) -> Self {
        Self {
            code_to_index: codes
                .iter()
                .enumerate()
                .map(|(i, c)| (*c, i + Self::SPECIAL_VALUE_COUNT))
                .collect::<HashMap<_, _>>(),
            codes,
        }
    }

    pub fn deserialize_from<R: Read>(buf: R) -> Result<Self> {
        let mut lines = BufReader::new(buf).lines();
        assert_eq!(lines.next().unwrap()?, "INVALID");
        assert_eq!(lines.next().unwrap()?, "UNKNOWN");

        let dict = Code::values()
            .map(|c| (format!("{:?}", c), c))
            .collect::<HashMap<_, _>>();

        let codes = lines
            .map(|l| -> Result<_> {
                let l = l?;
                dict.get(l.as_str())
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("invalid code: {}", l))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::new(codes))
    }

    pub fn serialize_to<W: Write>(&self, buf: W) -> Result<()> {
        let mut buf = BufWriter::new(buf);
        writeln!(buf, "INVALID")?;
        writeln!(buf, "UNKNOWN")?;
        for code in &self.codes {
            writeln!(buf, "{:?}", code)?;
        }
        Ok(())
    }
}

impl Index<usize> for CodeVocab {
    type Output = Code;

    fn index(&self, index: usize) -> &Self::Output {
        if index < Self::SPECIAL_VALUE_COUNT {
            match index {
                Self::INVALID => &Code::INVALID,
                Self::UNKNOWN => panic!("Attempt to get unknown code from the vocab"),
                _ => unreachable!(),
            }
        } else {
            &self.codes[index - Self::SPECIAL_VALUE_COUNT]
        }
    }
}

impl Index<Code> for CodeVocab {
    type Output = usize;

    fn index(&self, code: Code) -> &Self::Output {
        self.code_to_index.get(&code).unwrap_or(&Self::UNKNOWN)
    }
}
