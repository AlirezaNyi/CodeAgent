//! Token budget abstractions.

/// Counts approximate tokens for context budgeting.
pub trait TokenCounter: Send + Sync {
    fn count(&self, text: &str) -> u64;
}

/// Heuristic counter: roughly `chars / 4`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicCounter;

impl TokenCounter for HeuristicCounter {
    fn count(&self, text: &str) -> u64 {
        (text.chars().count() as u64).div_ceil(4).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_basic() {
        let c = HeuristicCounter;
        assert_eq!(c.count("abcd"), 1);
        assert_eq!(c.count("abcdefgh"), 2);
    }
}
