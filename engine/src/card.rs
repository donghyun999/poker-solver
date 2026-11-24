//! 카드와 콤보(핸드 2장) 표현.
//!
//! - `Card` = 0~51. `rank = c / 4` (0=2 … 12=A), `suit = c % 4` (c,d,h,s)
//! - 콤보 = 카드 2장. 0~1325 인덱스로 표현 (`combo_index`)
//! - 보드 등 카드 집합은 `u64` 비트마스크 (비트 c = 카드 c)

use std::sync::LazyLock;

pub type Card = u8;

pub const NUM_CARDS: usize = 52;
pub const NUM_COMBOS: usize = 1326;

const RANK_CHARS: &[u8; 13] = b"23456789TJQKA";
const SUIT_CHARS: &[u8; 4] = b"cdhs";

#[inline]
pub fn rank(c: Card) -> u8 {
    c >> 2
}

#[inline]
pub fn suit(c: Card) -> u8 {
    c & 3
}

#[inline]
pub fn make_card(rank: u8, suit: u8) -> Card {
    rank * 4 + suit
}

#[inline]
pub fn card_bit(c: Card) -> u64 {
    1u64 << c
}

pub fn parse_rank(ch: char) -> Option<u8> {
    let up = ch.to_ascii_uppercase() as u8;
    RANK_CHARS.iter().position(|&r| r == up).map(|i| i as u8)
}

pub fn parse_suit(ch: char) -> Option<u8> {
    let low = ch.to_ascii_lowercase() as u8;
    SUIT_CHARS.iter().position(|&s| s == low).map(|i| i as u8)
}

pub fn rank_char(r: u8) -> char {
    RANK_CHARS[r as usize] as char
}

/// "As" → 카드
pub fn parse_card(s: &str) -> Result<Card, String> {
    let mut it = s.chars();
    match (it.next(), it.next(), it.next()) {
        (Some(r), Some(su), None) => match (parse_rank(r), parse_suit(su)) {
            (Some(r), Some(su)) => Ok(make_card(r, su)),
            _ => Err(format!("잘못된 카드: '{s}'")),
        },
        _ => Err(format!("잘못된 카드: '{s}'")),
    }
}

/// "Kh Qd 7c", "KhQd7c", "Kh,Qd,7c" 모두 허용. 중복 카드는 에러.
pub fn parse_cards(s: &str) -> Result<Vec<Card>, String> {
    let compact: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if compact.len() % 2 != 0 {
        return Err(format!("잘못된 카드 목록: '{s}'"));
    }
    let mut cards = Vec::new();
    let mut seen = 0u64;
    for i in (0..compact.len()).step_by(2) {
        let c = parse_card(&compact[i..i + 2])?;
        if seen & card_bit(c) != 0 {
            return Err(format!("중복 카드: '{}'", card_to_string(c)));
        }
        seen |= card_bit(c);
        cards.push(c);
    }
    Ok(cards)
}

pub fn card_to_string(c: Card) -> String {
    format!("{}{}", rank_char(rank(c)), SUIT_CHARS[suit(c) as usize] as char)
}

pub fn cards_to_string(cards: &[Card]) -> String {
    cards.iter().map(|&c| card_to_string(c)).collect::<Vec<_>>().join(" ")
}

pub fn mask_of(cards: &[Card]) -> u64 {
    cards.iter().fold(0, |m, &c| m | card_bit(c))
}

/// 카드 2장 → 0~1325. 순서 무관.
#[inline]
pub fn combo_index(a: Card, b: Card) -> usize {
    let (lo, hi) = if a < b { (a as usize, b as usize) } else { (b as usize, a as usize) };
    hi * (hi - 1) / 2 + lo
}

/// 인덱스 → (낮은 카드, 높은 카드)
static COMBOS: LazyLock<Vec<(Card, Card)>> = LazyLock::new(|| {
    let mut v = Vec::with_capacity(NUM_COMBOS);
    for hi in 1..NUM_CARDS as u8 {
        for lo in 0..hi {
            v.push((lo, hi));
        }
    }
    v
});

#[inline]
pub fn combo_cards(idx: usize) -> (Card, Card) {
    COMBOS[idx]
}

/// 콤보 → 핸드 클래스 이름. 예: (As, Kh) → "AKo", (Ad, Ac) → "AA"
pub fn hand_class(a: Card, b: Card) -> String {
    let (hi, lo) = if rank(a) >= rank(b) { (a, b) } else { (b, a) };
    let (rh, rl) = (rank(hi), rank(lo));
    if rh == rl {
        format!("{}{}", rank_char(rh), rank_char(rl))
    } else {
        let s = if suit(hi) == suit(lo) { 's' } else { 'o' };
        format!("{}{}{}", rank_char(rh), rank_char(rl), s)
    }
}

/// 핸드 클래스 정렬 키 (작을수록 먼저): AA, AKs, AKo, AQs, … 순
pub fn hand_class_order(a: Card, b: Card) -> u32 {
    let (rh, rl) = if rank(a) >= rank(b) { (rank(a), rank(b)) } else { (rank(b), rank(a)) };
    let kind = if rh == rl { 0 } else if suit(a) == suit(b) { 1 } else { 2 };
    (12 - rh as u32) * 100 + (12 - rl as u32) * 3 + kind
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_index_roundtrip() {
        for i in 0..NUM_COMBOS {
            let (a, b) = combo_cards(i);
            assert!(a < b);
            assert_eq!(combo_index(a, b), i);
            assert_eq!(combo_index(b, a), i);
        }
    }

    #[test]
    fn parse_and_print() {
        let c = parse_cards("Kh Qd 7c").unwrap();
        assert_eq!(cards_to_string(&c), "Kh Qd 7c");
        assert_eq!(parse_cards("KhQd7c").unwrap(), c);
        assert!(parse_cards("Kh Kh").is_err());
        assert!(parse_cards("Kx").is_err());
        assert_eq!(rank(parse_card("As").unwrap()), 12);
        assert_eq!(rank(parse_card("2c").unwrap()), 0);
    }

    #[test]
    fn hand_classes() {
        let c = |s| parse_card(s).unwrap();
        assert_eq!(hand_class(c("Ks"), c("As")), "AKs");
        assert_eq!(hand_class(c("Kd"), c("As")), "AKo");
        assert_eq!(hand_class(c("7d"), c("7s")), "77");
    }
}
