use crate::disassembly::DisassemblyResult;
use crate::model::{Label, SupersetSample};
use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub struct EvaluationResult {
    pub true_positives: BTreeSet<u32>,
    pub false_positives: BTreeSet<u32>,
    pub false_negatives: BTreeSet<u32>,
}

impl EvaluationResult {
    pub fn summary(&self) -> EvaluationResultSummary {
        let true_positives = self.true_positives.len();
        let false_positives = self.false_positives.len();
        let false_negatives = self.false_negatives.len();
        let precision = true_positives as f64 / (true_positives + false_positives) as f64;
        let recall = true_positives as f64 / (true_positives + false_negatives) as f64;
        let f1 = 2.0 * precision * recall / (precision + recall);
        EvaluationResultSummary {
            true_positives,
            false_positives,
            false_negatives,
            precision,
            recall,
            f1,
        }
    }
}

#[derive(Debug)]
pub struct EvaluationResultSummary {
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

pub fn evaluate_result(superset: &SupersetSample, result: &DisassemblyResult) -> EvaluationResult {
    let result = &result.predicted_instructions;
    let mut true_result = BTreeSet::new();

    for &(address, _, label) in superset.superset.iter() {
        match label.unwrap() {
            Label::Code => {
                true_result.insert(address);
            }
            Label::NotCode => {}
        }
    }

    let mut evaluation_result = EvaluationResult::default();

    for address in result.iter() {
        if true_result.contains(address) {
            evaluation_result.true_positives.insert(*address);
        } else {
            evaluation_result.false_positives.insert(*address);
        }
    }

    for address in true_result.iter() {
        if !result.contains(address) {
            evaluation_result.false_negatives.insert(*address);
        }
    }

    evaluation_result
}
