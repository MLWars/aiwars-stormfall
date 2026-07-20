//! A self-contained poker hand evaluator (no crates): score the best 5-card hand out of the
//! 5–7 cards a player can make, as a totally-ordered [`HandRank`] you can compare directly.
//!
//! A `HandRank` is a category (`0`=high card … `8`=straight flush) plus up to five tiebreak
//! ranks in descending priority. Because the derived `Ord` compares `cat` first and then the
//! `tie` array lexicographically, `a > b` iff hand `a` beats hand `b`, and `a == b` is a genuine
//! tie (a split pot). The tiebreak layout per category:
//!
//! * straight flush  → `[high]`                     (wheel A-2-3-4-5 scores high = 5)
//! * four of a kind  → `[quad, kicker]`
//! * full house      → `[trips, pair]`
//! * flush           → `[c1, c2, c3, c4, c5]`       (top five flush-suit ranks)
//! * straight        → `[high]`                     (wheel scores high = 5)
//! * three of a kind → `[trips, k1, k2]`
//! * two pair        → `[hi_pair, lo_pair, kicker]`
//! * one pair        → `[pair, k1, k2, k3]`
//! * high card       → `[c1, c2, c3, c4, c5]`

use crate::cards::Card;

/// Hand categories, high (best) to low. Kept as named constants so the code reads like poker.
pub const HIGH_CARD: u8 = 0;
pub const PAIR: u8 = 1;
pub const TWO_PAIR: u8 = 2;
pub const TRIPS: u8 = 3;
pub const STRAIGHT: u8 = 4;
pub const FLUSH: u8 = 5;
pub const FULL_HOUSE: u8 = 6;
pub const QUADS: u8 = 7;
pub const STRAIGHT_FLUSH: u8 = 8;

/// A comparable hand score: category first, then tiebreak ranks in descending priority. The
/// derived `Ord` makes `>` mean "beats" and `==` mean "splits".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct HandRank {
    pub cat: u8,
    pub tie: [u8; 5],
}

impl HandRank {
    fn new(cat: u8, tie: &[u8]) -> HandRank {
        let mut t = [0u8; 5];
        for (i, &r) in tie.iter().take(5).enumerate() {
            t[i] = r;
        }
        HandRank { cat, tie: t }
    }
}

/// The best straight's high card among the present ranks, or `None`. `present[r]` must be set
/// for every rank held; the caller also mirrors an ace into `present[1]` so the wheel
/// (A-2-3-4-5, high = 5) is found. Scans from the top so the *highest* straight wins.
fn straight_high(present: &[bool; 15]) -> Option<u8> {
    for top in (5..=14u8).rev() {
        if (0..5).all(|k| present[(top - k) as usize]) {
            return Some(top);
        }
    }
    None
}

/// Human name for a category — for the action log / view ("a flush", "two pair", …).
pub fn category_name(cat: u8) -> &'static str {
    match cat {
        STRAIGHT_FLUSH => "a straight flush",
        QUADS => "four of a kind",
        FULL_HOUSE => "a full house",
        FLUSH => "a flush",
        STRAIGHT => "a straight",
        TRIPS => "three of a kind",
        TWO_PAIR => "two pair",
        PAIR => "a pair",
        _ => "high card",
    }
}

/// Score the best 5-card hand from `cards` (5, 6 or 7 cards — a showdown always passes 7:
/// two hole cards plus the five-card board). Never allocates a combination set; it reads the
/// rank/suit histograms directly, which is both faster and easier to get right.
pub fn evaluate(cards: &[Card]) -> HandRank {
    // Rank histogram (index by rank, 2..=14) and the ranks held in each suit.
    let mut count = [0u8; 15];
    let mut by_suit: [Vec<u8>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for c in cards {
        count[c.rank as usize] += 1;
        by_suit[c.suit as usize].push(c.rank);
    }

    // --- straight flush: a straight inside the flush suit (covers the steel wheel). ---
    let flush_suit = (0..4).find(|&s| by_suit[s].len() >= 5);
    if let Some(fs) = flush_suit {
        let mut present = [false; 15];
        for &r in &by_suit[fs] {
            present[r as usize] = true;
            if r == 14 {
                present[1] = true; // ace plays low for the wheel
            }
        }
        if let Some(high) = straight_high(&present) {
            return HandRank::new(STRAIGHT_FLUSH, &[high]);
        }
    }

    // Ranks bucketed by their multiplicity, each list already in descending rank order
    // (we walk ranks high→low).
    let mut quads = Vec::new();
    let mut trips = Vec::new();
    let mut pairs = Vec::new();
    for r in (2..=14u8).rev() {
        match count[r as usize] {
            4 => quads.push(r),
            3 => trips.push(r),
            2 => pairs.push(r),
            _ => {}
        }
    }
    let highest = |exclude: &[u8]| -> Option<u8> {
        (2..=14u8)
            .rev()
            .find(|&r| count[r as usize] > 0 && !exclude.contains(&r))
    };

    // --- four of a kind ---
    if let Some(&q) = quads.first() {
        let kicker = highest(&[q]).unwrap_or(0);
        return HandRank::new(QUADS, &[q, kicker]);
    }

    // --- full house (trips + a lower trips-as-pair or a pair) ---
    if let Some(&t) = trips.first() {
        let pair = trips.iter().skip(1).chain(pairs.iter()).copied().max();
        if let Some(p) = pair {
            return HandRank::new(FULL_HOUSE, &[t, p]);
        }
    }

    // --- flush (top five cards of the flush suit) ---
    if let Some(fs) = flush_suit {
        let mut fr = by_suit[fs].clone();
        fr.sort_unstable_by(|a, b| b.cmp(a));
        fr.truncate(5);
        return HandRank::new(FLUSH, &fr);
    }

    // --- straight ---
    let mut present = [false; 15];
    for r in 2..=14u8 {
        if count[r as usize] > 0 {
            present[r as usize] = true;
            if r == 14 {
                present[1] = true;
            }
        }
    }
    if let Some(high) = straight_high(&present) {
        return HandRank::new(STRAIGHT, &[high]);
    }

    // --- three of a kind ---
    if let Some(&t) = trips.first() {
        let mut tie = vec![t];
        tie.extend(
            (2..=14u8)
                .rev()
                .filter(|&r| r != t && count[r as usize] > 0)
                .take(2),
        );
        return HandRank::new(TRIPS, &tie);
    }

    // --- two pair ---
    if pairs.len() >= 2 {
        let (hi, lo) = (pairs[0], pairs[1]);
        let kicker = highest(&[hi, lo]).unwrap_or(0);
        return HandRank::new(TWO_PAIR, &[hi, lo, kicker]);
    }

    // --- one pair ---
    if let Some(&p) = pairs.first() {
        let mut tie = vec![p];
        tie.extend(
            (2..=14u8)
                .rev()
                .filter(|&r| r != p && count[r as usize] > 0)
                .take(3),
        );
        return HandRank::new(PAIR, &tie);
    }

    // --- high card ---
    let tie: Vec<u8> = (2..=14u8)
        .rev()
        .filter(|&r| count[r as usize] > 0)
        .take(5)
        .collect();
    HandRank::new(HIGH_CARD, &tie)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::Card;

    /// Evaluate a hand written as space-separated card codes ("As Kd Qc …").
    fn eval(hand: &str) -> HandRank {
        let cards: Vec<Card> = hand
            .split_whitespace()
            .map(|c| Card::from_code(c).unwrap())
            .collect();
        evaluate(&cards)
    }

    #[test]
    fn categories_are_detected() {
        assert_eq!(eval("As Ks Qs Js Ts 2d 3c").cat, STRAIGHT_FLUSH);
        assert_eq!(eval("9h 9s 9d 9c Kh 2d 3c").cat, QUADS);
        assert_eq!(eval("9h 9s 9d Kc Kh 2d 3c").cat, FULL_HOUSE);
        assert_eq!(eval("As Ks Qs 2s 7s 9d 3c").cat, FLUSH);
        assert_eq!(eval("9h 8s 7d 6c 5h 2d 3c").cat, STRAIGHT);
        assert_eq!(eval("9h 9s 9d Kc Qh 2d 3c").cat, TRIPS);
        assert_eq!(eval("9h 9s Kd Kc Qh 2d 3c").cat, TWO_PAIR);
        assert_eq!(eval("9h 9s Kd Qc Jh 2d 3c").cat, PAIR);
        assert_eq!(eval("9h 7s Kd Qc Jh 2d 3c").cat, HIGH_CARD);
    }

    #[test]
    fn wheel_straight_uses_five_as_high_and_loses_to_six_high() {
        let wheel = eval("Ah 2d 3c 4s 5h Kd Qc"); // A-2-3-4-5
        assert_eq!(wheel.cat, STRAIGHT);
        assert_eq!(wheel.tie[0], 5, "the wheel is a five-high straight");
        let six_high = eval("2h 3d 4c 5s 6h Kd Qc");
        assert!(six_high > wheel, "6-high straight beats the wheel");
    }

    #[test]
    fn wheel_straight_flush_is_a_straight_flush_scored_five_high() {
        let steel_wheel = eval("Ah 2h 3h 4h 5h Kd Qc");
        assert_eq!(steel_wheel.cat, STRAIGHT_FLUSH);
        assert_eq!(steel_wheel.tie[0], 5);
        // A six-high straight flush beats the steel wheel.
        let six_high_sf = eval("2h 3h 4h 5h 6h Kd Qc");
        assert!(six_high_sf > steel_wheel);
    }

    #[test]
    fn broadway_straight_flush_beats_lower_straight_flush() {
        let royal = eval("Ts Js Qs Ks As 2d 3c");
        let lower = eval("9h Th Jh Qh Kh 2d 3c");
        assert_eq!(royal.cat, STRAIGHT_FLUSH);
        assert_eq!(lower.cat, STRAIGHT_FLUSH);
        assert!(royal > lower);
    }

    #[test]
    fn flush_beats_straight() {
        let flush = eval("As Ks 9s 5s 2s 6d 7h");
        let straight = eval("9h 8s 7d 6c 5h 2d 3c");
        assert!(flush > straight);
    }

    #[test]
    fn full_house_beats_flush_beats_straight_ordering() {
        let boat = eval("9h 9s 9d Kc Kh 2s 3s");
        let flush = eval("As Ks 9s 5s 2s 6d 7h");
        assert!(boat > flush);
    }

    #[test]
    fn quads_beat_full_house() {
        let quads = eval("9h 9s 9d 9c Kh Kd 3c");
        let boat = eval("9h 9s 9d Kc Kh 2d 3c");
        assert!(quads > boat);
    }

    #[test]
    fn four_of_a_kind_kicker_breaks_ties() {
        let with_ace = eval("9h 9s 9d 9c Ah 2d 3c");
        let with_king = eval("9h 9s 9d 9c Kh 2d 3c");
        assert_eq!(with_ace.cat, QUADS);
        assert!(with_ace > with_king, "ace kicker beats king kicker");
    }

    #[test]
    fn full_house_ranks_trips_before_pair() {
        // 999-22 beats 888-KK: the trips rank dominates the pair rank.
        let nines_full = eval("9h 9s 9d 2c 2h 4d 5c");
        let eights_full = eval("8h 8s 8d Kc Kh 4d 5c");
        assert!(nines_full > eights_full);
    }

    #[test]
    fn two_trips_makes_the_best_full_house() {
        // With two sets, the higher is the trips and the lower plays as the pair.
        let hr = eval("9h 9s 9d 7c 7h 7s 2c");
        assert_eq!(hr.cat, FULL_HOUSE);
        assert_eq!(hr.tie[0], 9);
        assert_eq!(hr.tie[1], 7);
    }

    #[test]
    fn two_pair_uses_top_two_pairs_and_a_kicker() {
        // Three pairs present (K,9,4) → the best two pair is KK99 with a 4 kicker... but the
        // fifth card is the higher of the remaining ranks: kicker is the top leftover card.
        let hr = eval("Kh Ks 9h 9s 4d 4c Qh");
        assert_eq!(hr.cat, TWO_PAIR);
        assert_eq!(hr.tie[0], 13); // kings
        assert_eq!(hr.tie[1], 9); // nines
        assert_eq!(hr.tie[2], 12, "kicker is the queen, not the third pair");
    }

    #[test]
    fn pair_kickers_break_ties() {
        let ace_kicker = eval("9h 9s Ad Kc Qh 2s 3c");
        let jack_kicker = eval("9h 9s Jd Kc Qh 2s 3c");
        assert!(ace_kicker > jack_kicker);
    }

    #[test]
    fn board_plays_for_both_is_a_tie() {
        // The board is a wheel-to-broadway... here a straight on the board; both players'
        // hole cards are irrelevant, so the two 7-card hands score identically.
        let board = "Ah Kd Qc Js Ts"; // broadway straight
        let a = eval(&format!("{board} 2h 3d"));
        let b = eval(&format!("{board} 4c 5s"));
        assert_eq!(a, b, "when the board plays, the hands tie");
        assert_eq!(a.cat, STRAIGHT);
    }

    #[test]
    fn high_card_compares_all_five_kickers() {
        let a = eval("Ah Kd Qc Js 9h 2d 3c"); // A K Q J 9
        let b = eval("Ah Kd Qc Js 8h 2d 3c"); // A K Q J 8
        assert!(a > b, "only the last kicker differs");
    }
}
