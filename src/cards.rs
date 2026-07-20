//! Cards, ranks, suits and a standard 52-card deck — a self-contained model (no crates).
//!
//! A rank is `2..=14` (`J=11`, `Q=12`, `K=13`, `A=14`); a suit is `0..=3` (spades, hearts,
//! diamonds, clubs). The wire form is a two-char code like `"As"` (ace of spades), `"Td"` (ten
//! of diamonds) — rank char + lowercase suit letter `s`/`h`/`d`/`c`. The view prettifies the
//! suit letter into a coloured pip; the engine stays ASCII so tests can assert on the JSON.

/// A single playing card. `Copy` so hands are cheap to pass by value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Card {
    /// `2..=14` (`J=11`, `Q=12`, `K=13`, `A=14`).
    pub rank: u8,
    /// `0..=3`: `0`=spades, `1`=hearts, `2`=diamonds, `3`=clubs.
    pub suit: u8,
}

/// The rank character for a rank value (`10` renders as `T`).
pub fn rank_char(rank: u8) -> char {
    match rank {
        2..=9 => (b'0' + rank) as char,
        10 => 'T',
        11 => 'J',
        12 => 'Q',
        13 => 'K',
        14 => 'A',
        _ => '?',
    }
}

/// The lowercase suit letter for a suit value.
pub fn suit_char(suit: u8) -> char {
    match suit {
        0 => 's',
        1 => 'h',
        2 => 'd',
        3 => 'c',
        _ => '?',
    }
}

impl Card {
    /// The two-char wire code, e.g. `"As"`, `"Td"`, `"9c"`.
    pub fn to_code(self) -> String {
        format!("{}{}", rank_char(self.rank), suit_char(self.suit))
    }

    /// Parse a two-char code back into a card — a test helper to build fixed hands readably.
    /// Case-insensitive on the rank, lowercase suit letters. Returns `None` on anything invalid.
    #[cfg(test)]
    pub fn from_code(code: &str) -> Option<Card> {
        let mut chars = code.chars();
        let r = chars.next()?;
        let s = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let rank = match r.to_ascii_uppercase() {
            '2'..='9' => (r as u8) - b'0',
            'T' => 10,
            'J' => 11,
            'Q' => 12,
            'K' => 13,
            'A' => 14,
            _ => return None,
        };
        let suit = match s {
            's' => 0,
            'h' => 1,
            'd' => 2,
            'c' => 3,
            _ => return None,
        };
        Some(Card { rank, suit })
    }
}

/// A fresh, ordered 52-card deck (rank-major within each suit). Callers shuffle it with the
/// match's seeded RNG before dealing — never in this order.
pub fn fresh_deck() -> Vec<Card> {
    let mut deck = Vec::with_capacity(52);
    for suit in 0..4 {
        for rank in 2..=14 {
            deck.push(Card { rank, suit });
        }
    }
    deck
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for &code in &["As", "Kh", "Qd", "Jc", "Ts", "9h", "2c"] {
            let c = Card::from_code(code).unwrap();
            assert_eq!(c.to_code(), code);
        }
    }

    #[test]
    fn ten_renders_as_t() {
        assert_eq!(Card { rank: 10, suit: 0 }.to_code(), "Ts");
    }

    #[test]
    fn from_code_rejects_junk() {
        assert!(Card::from_code("Xx").is_none());
        assert!(Card::from_code("A").is_none());
        assert!(Card::from_code("Ass").is_none());
        assert!(Card::from_code("1s").is_none());
    }

    #[test]
    fn fresh_deck_is_a_full_unique_52() {
        let deck = fresh_deck();
        assert_eq!(deck.len(), 52);
        let mut seen = std::collections::HashSet::new();
        for c in &deck {
            assert!((2..=14).contains(&c.rank));
            assert!((0..=3).contains(&c.suit));
            assert!(seen.insert(c.to_code()), "duplicate card {}", c.to_code());
        }
        assert_eq!(seen.len(), 52);
    }
}
