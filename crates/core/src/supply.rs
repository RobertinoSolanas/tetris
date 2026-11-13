//! Piece supply: fairness (`7-bag`) plus reproducibility (seeded RNG) and a preview queue.
//!
//! The bag rule: deal all seven tetrominoes in random order, then reshuffle. The property players
//! rely on is the wait bound — a piece can never be more than 13 places away — which
//! `max_gap_is_bounded` asserts over many seeds.

use crate::piece::Tetromino;
use crate::rng::{RngExt as _, XorShift64};
use std::collections::VecDeque;

/// Rolls a shuffled bag of all seven tetrominoes and deals them out one at a time.
#[derive(Clone, Debug)]
pub struct SevenBag {
    rng: XorShift64,
    bag: [Tetromino; 7],
    next: usize,
}

impl SevenBag {
    pub fn new(seed: u64) -> Self {
        let mut rng = XorShift64::new(seed);
        let bag = Self::roll(&mut rng);
        Self { rng, bag, next: 0 }
    }

    fn roll(rng: &mut XorShift64) -> [Tetromino; 7] {
        let mut bag = Tetromino::ALL;
        rng.shuffle(&mut bag);
        bag
    }

    fn refill(&mut self) {
        self.bag = Self::roll(&mut self.rng);
        self.next = 0;
    }

    /// Deal the next piece from the bag.
    pub fn deal(&mut self) -> Tetromino {
        if self.next >= self.bag.len() {
            self.refill();
        }
        let piece = self.bag[self.next];
        self.next += 1;
        piece
    }

    /// Peek `n` upcoming pieces without consuming any.
    ///
    /// Future bags are rolled on a *copy* of the generator using exactly the same steps `next()`
    /// would, so `preview(k)[0] == next()` always holds and peeking never shifts the sequence.
    pub fn preview(&self, n: usize) -> Vec<Tetromino> {
        let mut rng = self.rng.clone();
        let mut bag = self.bag;
        let mut idx = self.next;
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            if idx >= bag.len() {
                bag = Self::roll(&mut rng);
                idx = 0;
            }
            out.push(bag[idx]);
            idx += 1;
        }
        out
    }
}

/// What the session uses: a bag behind a queue, so "next 5" is cheap to render.
#[derive(Clone, Debug)]
pub struct PieceSupply {
    bag: SevenBag,
    queue: VecDeque<Tetromino>,
    seed: u64,
    dealt: u64,
}

/// How many pieces the queue keeps buffered (≥ the number shown in the HUD).
const LOOKAHEAD: usize = 7;

impl PieceSupply {
    pub fn new(seed: u64) -> Self {
        let mut this = Self {
            bag: SevenBag::new(seed),
            queue: VecDeque::new(),
            seed,
            dealt: 0,
        };
        this.fill(LOOKAHEAD);
        this
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Pieces dealt so far (statistics, not gameplay).
    pub fn dealt(&self) -> u64 {
        self.dealt
    }

    fn fill(&mut self, n: usize) {
        while self.queue.len() < n {
            self.queue.push_back(self.bag.deal());
        }
    }

    /// Deal the next piece to the caller.
    pub fn deal(&mut self) -> Tetromino {
        self.fill(1);
        let piece = self.queue.pop_front().expect("queue was just filled");
        self.dealt += 1;
        self.fill(LOOKAHEAD);
        piece
    }

    /// Upcoming pieces, nearest first. Does not consume.
    pub fn preview(&self, n: usize) -> Vec<Tetromino> {
        self.queue.iter().copied().take(n).collect()
    }

    /// How many pieces are buffered internally (at least `LOOKAHEAD`).
    pub fn buffered(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn counts(pieces: &[Tetromino]) -> HashMap<Tetromino, usize> {
        let mut m = HashMap::new();
        for p in pieces {
            *m.entry(*p).or_insert(0) += 1;
        }
        m
    }

    #[test]
    fn every_bag_contains_each_piece_once() {
        for seed in [0u64, 1, 7, 99, 12345, u64::MAX] {
            let mut bag = SevenBag::new(seed);
            let dealt: Vec<Tetromino> = (0..7).map(|_| bag.deal()).collect();
            let c = counts(&dealt);
            for kind in Tetromino::ALL {
                assert_eq!(
                    c.get(&kind).copied().unwrap_or(0),
                    1,
                    "seed {seed}: {dealt:?} is not a permutation"
                );
            }
        }
    }

    #[test]
    fn sequence_is_deterministic_per_seed() {
        let xs: Vec<Tetromino> = {
            let mut b = SevenBag::new(42);
            (0..200).map(|_| b.deal()).collect()
        };
        let ys: Vec<Tetromino> = {
            let mut b = SevenBag::new(42);
            (0..200).map(|_| b.deal()).collect()
        };
        assert_eq!(xs, ys);

        let zs: Vec<Tetromino> = {
            let mut b = SevenBag::new(43);
            (0..200).map(|_| b.deal()).collect()
        };
        assert_ne!(xs, zs, "different seeds must give different sequences");
    }

    #[test]
    fn sequence_is_balanced_over_many_bags() {
        let mut bag = SevenBag::new(0xC0FFEE);
        let dealt: Vec<Tetromino> = (0..7 * 200).map(|_| bag.deal()).collect();
        let c = counts(&dealt);
        for kind in Tetromino::ALL {
            assert_eq!(c[&kind], 200);
        }
    }

    #[test]
    fn first_two_pieces_of_a_run_differ_for_most_seeds() {
        // Bag fairness allows any repeat distance up to 13; the point of this test is that the
        // shuffle is not degenerate (e.g. always identical first two pieces across seeds).
        let mut same_first_two = 0;
        let mut reference: Option<[Tetromino; 2]> = None;
        for seed in 0..200u64 {
            let p = SevenBag::new(seed).preview(2);
            if reference == Some([p[0], p[1]]) {
                same_first_two += 1;
            }
            reference = Some([p[0], p[1]]);
        }
        assert!(same_first_two < 40, "shuffle looks degenerate");
    }

    #[test]
    fn max_gap_is_bounded() {
        // Worst case: last of bag N, then first of bag N+2 → index distance ≤ 13.
        for seed in [1u64, 2, 3, 999, 1_000_000] {
            let mut bag = SevenBag::new(seed);
            let seq: Vec<Tetromino> = (0..7 * 500).map(|_| bag.deal()).collect();
            for kind in Tetromino::ALL {
                let mut last: Option<usize> = None;
                for (i, p) in seq.iter().enumerate() {
                    if *p == kind {
                        if let Some(l) = last {
                            assert!(i - l <= 13, "seed {seed}: {kind} gap {}", i - l);
                        }
                        last = Some(i);
                    }
                }
            }
        }
    }

    #[test]
    fn preview_does_not_consume_the_sequence() {
        let mut peeker = SevenBag::new(5);
        let mut plain = SevenBag::new(5);
        for i in 0..40 {
            let p = peeker.preview(12); // deliberately crosses bag boundaries
            assert_eq!(p.len(), 12, "preview at {i}");
            assert_eq!(p[0], peeker.clone().deal(), "preview head at {i}");
            assert_eq!(peeker.deal(), plain.deal(), "sequence diverged at {i}");
        }
    }

    #[test]
    fn supply_preview_matches_next() {
        let mut s = PieceSupply::new(7);
        for i in 0..30 {
            let shown = s.preview(5);
            assert_eq!(shown.len(), 5, "iteration {i}");
            let next = s.deal();
            assert_eq!(shown[0], next, "iteration {i}");
        }
        assert_eq!(s.dealt(), 30);
        assert_eq!(s.seed(), 7);
        assert!(s.buffered() >= 5);
    }

    #[test]
    fn supply_is_deterministic() {
        let a: Vec<Tetromino> = {
            let mut s = PieceSupply::new(11);
            (0..60).map(|_| s.deal()).collect()
        };
        let b: Vec<Tetromino> = {
            let mut s = PieceSupply::new(11);
            (0..60).map(|_| s.deal()).collect()
        };
        assert_eq!(a, b);
    }

    #[test]
    fn preview_counts_are_clamped_not_panicking() {
        let bag = SevenBag::new(3);
        assert!(bag.preview(0).is_empty());
        assert_eq!(bag.preview(21).len(), 21);

        let mut s = PieceSupply::new(3);
        assert!(s.preview(0).is_empty());
        assert_eq!(s.preview(4).len(), 4);
        assert_eq!(s.preview(100).len(), s.buffered(), "clamped to the queue");
        let _ = s.deal();
        assert_eq!(s.preview(100).len(), s.buffered());
    }
}
