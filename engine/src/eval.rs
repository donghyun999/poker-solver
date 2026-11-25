//! 핸드 평가기: 카드 5~7장 → 강도(u32, 클수록 강함).
//!
//! 강도 = (족보 << 16) | 세부값
//! - 족보: 0 하이카드 … 8 스트레이트 플러시
//! - 세부값: 같은 족보끼리 비교용. 랭크를 4비트씩 붙이거나(페어·트립 등),
//!   13비트 랭크 마스크(하이카드·플러시)를 그대로 사용 → 정수 비교로 승패 판정
//!
//! 처음엔 단순하고 맞는 구현. 솔버에서는 보드(리버)마다 콤보별로 한 번씩만
//! 계산해서 저장하므로 여기 속도는 크게 중요하지 않다.

use crate::card::{Card, rank, suit};

pub const HIGH_CARD: u32 = 0;
pub const PAIR: u32 = 1;
pub const TWO_PAIR: u32 = 2;
pub const TRIPS: u32 = 3;
pub const STRAIGHT: u32 = 4;
pub const FLUSH: u32 = 5;
pub const FULL_HOUSE: u32 = 6;
pub const QUADS: u32 = 7;
pub const STRAIGHT_FLUSH: u32 = 8;

pub const CATEGORY_NAMES: [&str; 9] = [
    "하이카드", "원페어", "투페어", "트립스", "스트레이트", "플러시", "풀하우스", "포카드", "스트레이트 플러시",
];

#[inline]
fn make(cat: u32, value: u32) -> u32 {
    (cat << 16) | value
}

pub fn category(strength: u32) -> u32 {
    strength >> 16
}

/// 13비트 랭크 마스크에서 스트레이트 최고 랭크(+1). 없으면 None.
/// A-2-3-4-5(휠)는 가장 낮은 스트레이트.
#[inline]
fn straight_high(mask: u16) -> Option<u32> {
    // 비트0 = 로우 에이스, 비트1~13 = 2~A
    let m = ((mask as u32) << 1) | ((mask as u32 >> 12) & 1);
    (4..=13).rev().find(|&hi| (m >> (hi - 4)) & 0x1F == 0x1F)
}

/// 높은 비트부터 n개만 남김
#[inline]
fn top_bits(mut m: u16, n: u32) -> u16 {
    while m.count_ones() > n {
        m &= m - 1; // 가장 낮은 비트 제거
    }
    m
}

/// 높은 랭크부터 n개를 4비트씩 붙임
#[inline]
fn pack_top(mask: u16, n: u32) -> u32 {
    let mut v = 0;
    let mut cnt = 0;
    for r in (0..13).rev() {
        if cnt == n {
            break;
        }
        if mask & (1 << r) != 0 {
            v = (v << 4) | r as u32;
            cnt += 1;
        }
    }
    v
}

pub fn evaluate(cards: &[Card]) -> u32 {
    let mut suit_masks = [0u16; 4];
    let mut counts = [0u8; 13];
    for &c in cards {
        suit_masks[suit(c) as usize] |= 1 << rank(c);
        counts[rank(c) as usize] += 1;
    }
    let rank_mask = suit_masks[0] | suit_masks[1] | suit_masks[2] | suit_masks[3];

    // 플러시 / 스트레이트 플러시
    let mut flush = None;
    for &sm in &suit_masks {
        if sm.count_ones() >= 5 {
            if let Some(hi) = straight_high(sm) {
                return make(STRAIGHT_FLUSH, hi);
            }
            flush = Some(top_bits(sm, 5) as u32);
        }
    }

    // 같은 랭크 개수별로 분류 (높은 랭크부터)
    let (mut quad, mut trips, mut pairs) = (None, [0u32; 2], [0u32; 3]);
    let (mut nt, mut np) = (0, 0);
    for r in (0..13u32).rev() {
        match counts[r as usize] {
            4 => quad = Some(r),
            3 if nt < 2 => {
                trips[nt] = r;
                nt += 1;
            }
            2 if np < 3 => {
                pairs[np] = r;
                np += 1;
            }
            _ => {}
        }
    }

    if let Some(q) = quad {
        let kick = pack_top(rank_mask & !(1 << q), 1);
        return make(QUADS, (q << 4) | kick);
    }
    if nt >= 1 && (nt >= 2 || np >= 1) {
        let t = trips[0];
        // 트립이 2개면 낮은 트립도 페어 후보
        let p = match (nt >= 2, np > 0) {
            (true, true) => trips[1].max(pairs[0]),
            (true, false) => trips[1],
            _ => pairs[0],
        };
        return make(FULL_HOUSE, (t << 4) | p);
    }
    if let Some(f) = flush {
        return make(FLUSH, f);
    }
    if let Some(hi) = straight_high(rank_mask) {
        return make(STRAIGHT, hi);
    }
    if nt == 1 {
        let t = trips[0];
        let kick = pack_top(rank_mask & !(1 << t), 2);
        return make(TRIPS, (t << 8) | kick);
    }
    if np >= 2 {
        let (p1, p2) = (pairs[0], pairs[1]);
        let kick = pack_top(rank_mask & !(1 << p1) & !(1 << p2), 1);
        return make(TWO_PAIR, (p1 << 8) | (p2 << 4) | kick);
    }
    if np == 1 {
        let p = pairs[0];
        let kick = pack_top(rank_mask & !(1 << p), 3);
        return make(PAIR, (p << 12) | kick);
    }
    make(HIGH_CARD, top_bits(rank_mask, 5) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::parse_cards;

    fn ev(s: &str) -> u32 {
        evaluate(&parse_cards(s).unwrap())
    }

    #[test]
    fn categories() {
        assert_eq!(category(ev("As Ks Qs Js Ts 2c 3d")), STRAIGHT_FLUSH);
        assert_eq!(category(ev("As Ad Ah Ac Ks 2c 3d")), QUADS);
        assert_eq!(category(ev("As Ad Ah Kc Ks 2c 3d")), FULL_HOUSE);
        assert_eq!(category(ev("As Ad Ah Kc Ks Kd 3d")), FULL_HOUSE);
        assert_eq!(category(ev("As 9s 7s 4s 2s Kc Kd")), FLUSH);
        assert_eq!(category(ev("As 2d 3h 4c 5s Kc 9d")), STRAIGHT);
        assert_eq!(category(ev("7s 7d 7h 4c 5s Kc 9d")), TRIPS);
        assert_eq!(category(ev("7s 7d 4h 4c 5s Kc 9d")), TWO_PAIR);
        assert_eq!(category(ev("7s 7d 4h 2c 5s Kc 9d")), PAIR);
        assert_eq!(category(ev("7s Td 4h 2c 5s Kc 9d")), HIGH_CARD);
    }

    #[test]
    fn ordering() {
        // 휠 < 6하이 스트레이트
        assert!(ev("As 2d 3h 4c 5s") < ev("2d 3h 4c 5s 6s"));
        // 브로드웨이 > K하이 스트레이트
        assert!(ev("Ts Jd Qh Kc As") > ev("9s Td Jh Qc Ks"));
        // 키커
        assert!(ev("As Ad Kh 7c 5s 3d 2c") > ev("As Ad Qh 7c 5s 3d 2c"));
        // 투페어: 높은 페어 우선, 그다음 낮은 페어, 그다음 키커
        assert!(ev("Ks Kd 2h 2c As") > ev("Qs Qd Jh Jc As"));
        assert!(ev("Ks Kd 3h 3c 4s") > ev("Ks Kd 2h 2c As"));
        // 3페어 → 상위 2페어 + 남은 최고 카드
        assert_eq!(ev("Ks Kd Qh Qc 2s 2d 3c"), ev("Ks Kd Qh Qc 3s"));
        assert!(ev("Ks Kd Qh Qc 2s 2d 3c") < ev("Ks Kd Qh Qc 4s 2d 2c"));
        // 풀하우스: 트립 2개면 높은 트립 + 낮은 트립을 페어로
        assert!(ev("As Ad Ah Kc Ks Kd 3d") > ev("As Ad Ah Qc Qs Qd Kd"));
        // 같은 핸드는 동점
        assert_eq!(ev("As Kd 9h 7c 5s"), ev("Ad Ks 9c 7h 5d"));
        // 플러시 > 스트레이트
        assert!(ev("2s 4s 6s 8s 9s") > ev("Ts Jd Qh Kc As"));
    }

    /// 7장 조합 1억 3천만 개 전수 검사. 족보별 개수가 알려진 값과 일치해야 함.
    /// 테스트 프로필이 최적화 빌드라 몇 초면 끝나서 CI에서 매번 돈다.
    #[test]
    fn exhaustive_7_card_counts() {
        let mut counts = [0u64; 9];
        let mut c = [0u8; 7];
        for a in 0..46u8 {
            c[0] = a;
            for b in a + 1..47 {
                c[1] = b;
                for d in b + 1..48 {
                    c[2] = d;
                    for e in d + 1..49 {
                        c[3] = e;
                        for f in e + 1..50 {
                            c[4] = f;
                            for g in f + 1..51 {
                                c[5] = g;
                                for h in g + 1..52 {
                                    c[6] = h;
                                    counts[category(evaluate(&c)) as usize] += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        let expected = [
            23_294_460, // 하이카드
            58_627_800, // 원페어
            31_433_400, // 투페어
            6_461_620,  // 트립스
            6_180_020,  // 스트레이트
            4_047_644,  // 플러시
            3_473_184,  // 풀하우스
            224_848,    // 포카드
            41_584,     // 스트레이트 플러시 (로열 포함)
        ];
        assert_eq!(counts, expected);
    }
}
