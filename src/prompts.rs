//! Built-in prompts for growing a conversation toward the context limit.
//!
//! A conversation is seeded with one open-ended question, then grown by cycling
//! through a small set of follow-up nudges. The accumulating history is what
//! actually stresses the context window — each turn sends a longer, unique
//! prompt, so there is no fixed prefix for a server-side cache to fully reuse.

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::seq::IndexedRandom;

/// Per-conversation prompt state: picks a system prompt and seed at construction,
/// then vends follow-ups without immediate repeats.
pub struct PromptSampler {
    rng: SmallRng,
    system: &'static str,
    seed: &'static str,
    last: Option<usize>,
}

impl PromptSampler {
    /// Create a sampler. When `seed` is `Some`, the RNG is deterministic so the
    /// same prompt sequence is produced across runs for fair comparison.
    pub fn new(seed: Option<u64>) -> Self {
        let mut rng = match seed {
            Some(s) => SmallRng::seed_from_u64(s),
            None => SmallRng::from_os_rng(),
        };
        let system = SYSTEM_PROMPTS.choose(&mut rng).copied().unwrap_or("");
        let seed = SEEDS.choose(&mut rng).copied().unwrap_or(DEFAULT_SEED);
        Self {
            rng,
            system,
            seed,
            last: None,
        }
    }

    /// System prompt for this conversation (chosen at construction).
    pub fn system(&self) -> &'static str {
        self.system
    }

    /// First user turn for this conversation.
    pub fn seed(&self) -> &'static str {
        self.seed
    }

    /// Next follow-up user turn — never the same index twice in a row so the
    /// model keeps producing fresh output as the conversation grows.
    pub fn next_followup(&mut self) -> &'static str {
        let len = FOLLOWUPS.len();
        if len == 1 {
            return FOLLOWUPS[0];
        }
        loop {
            let idx = self.rng.random_range(0..len);
            if Some(idx) != self.last {
                self.last = Some(idx);
                return FOLLOWUPS[idx];
            }
        }
    }
}

const DEFAULT_SEED: &str =
    "Explain how a modern CPU executes instructions, starting from fetch and decode.";

/// System prompts that elicit verbose answers, growing the conversation quickly.
pub static SYSTEM_PROMPTS: &[&str] = &[
    "You are a senior engineer. Answer thoroughly: give a conceptual explanation, a complete commented code example, common pitfalls, and production considerations. Never give a short answer when a detailed one is possible.",
    "You are a technical educator writing a textbook chapter. Structure every answer as: introduction, core concepts with definitions, worked examples with full code, and a summary. Be exhaustive.",
    "You are a rigorous engineer who thinks step by step. Before answering, write out your reasoning: what you know, what you must figure out, and your approach. Then give the full solution.",
    "You are a staff engineer mentoring a junior. Explain everything deeply enough that they could debug it in production at 3am, including reasoning behind every decision.",
];

/// Open-ended seed questions that invite long, detailed responses.
pub static SEEDS: &[&str] = &[
    "I want to build a production-grade web scraper in Python. What architecture should I use, and can you show a complete async implementation?",
    "Design a URL shortening service that handles 100,000 redirects per second globally with sub-10ms latency. Walk me through the full system.",
    "Explain the Raft consensus algorithm in enough depth that I could implement it in my own distributed key-value store.",
    "Our PostgreSQL database is slow under load — queries that took 10ms now take 3 seconds at peak. How do I systematically diagnose and fix this?",
    "I'm learning Rust from C++ and want to build a high-performance async TCP server. Explain the ownership model and how to structure the project.",
    "Walk me through building an end-to-end ML pipeline to predict customer churn, from raw data to a production model.",
];

/// Follow-up nudges cycled to grow the conversation within the same thread.
pub static FOLLOWUPS: &[&str] = &[
    "Continue — go deeper and add concrete, runnable code for the next part.",
    "What are the main failure modes here, and how do I handle each one?",
    "Now walk me through testing this thoroughly, with example test cases.",
    "How does this behave under high load, and how do I optimize it?",
    "Expand on the trade-offs of alternative approaches and when to prefer each.",
    "Add observability: what should I log, measure, and alert on, with examples?",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn followups_avoid_immediate_repeats() {
        let mut s = PromptSampler::new(Some(42));
        let mut prev = s.next_followup();
        for _ in 0..50 {
            let next = s.next_followup();
            assert_ne!(prev, next, "follow-up repeated immediately");
            prev = next;
        }
    }

    #[test]
    fn seed_is_deterministic_for_fixed_seed() {
        let a = PromptSampler::new(Some(7));
        let b = PromptSampler::new(Some(7));
        assert_eq!(a.seed(), b.seed());
        assert_eq!(a.system(), b.system());
    }
}
