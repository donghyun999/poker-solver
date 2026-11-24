//! 레인지: 1326개 콤보 각각의 가중치(0~1).
//!
//! 지원 표기 (쉼표로 구분, `:가중치` 선택)
//! - `AA`, `AKs`, `AKo`, `AK`(수딧+오프수트 16콤보)
//! - `TT+` (TT~AA), `A2s+` (A2s~AKs), `K9o+`, `Q9+`
//! - `22-55`, `A2s-A5s`, `KTo-KQo`
//! - 특정 콤보 `AsKs`
//! - 가중치 `AA:0.5`, `AKs:0.25`

use crate::card::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    pub weights: Vec<f32>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Pair,
    Suited,
    Offsuit,
    Both,
}

impl Default for Range {
    fn default() -> Self {
        Self::new()
    }
}

impl Range {
    pub fn new() -> Self {
        Self { weights: vec![0.0; NUM_COMBOS] }
    }

    pub fn get(&self, a: Card, b: Card) -> f32 {
        self.weights[combo_index(a, b)]
    }

    pub fn set(&mut self, a: Card, b: Card, w: f32) {
        self.weights[combo_index(a, b)] = w;
    }

    /// 가중치 합 = 콤보 수
    pub fn num_combos(&self) -> f32 {
        self.weights.iter().sum()
    }

    pub fn parse(s: &str) -> Result<Range, String> {
        let mut range = Range::new();
        for token in s.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            parse_token(&mut range, token).map_err(|e| format!("'{token}': {e}"))?;
        }
        Ok(range)
    }

    fn add_class(&mut self, r1: u8, r2: u8, kind: Kind, w: f32) {
        for s1 in 0..4 {
            for s2 in 0..4 {
                let (a, b) = (make_card(r1, s1), make_card(r2, s2));
                if a == b {
                    continue;
                }
                let ok = match kind {
                    Kind::Pair => r1 == r2 && s1 < s2,
                    Kind::Suited => s1 == s2,
                    Kind::Offsuit => s1 != s2,
                    Kind::Both => true,
                };
                if ok {
                    self.set(a, b, w);
                }
            }
        }
    }
}

/// "AKs" 같은 클래스 표기 → (높은 랭크, 낮은 랭크, 종류)
fn parse_class(s: &str) -> Result<(u8, u8, Kind), String> {
    let ch: Vec<char> = s.chars().collect();
    if ch.len() < 2 || ch.len() > 3 {
        return Err("핸드 표기 오류".into());
    }
    let r1 = parse_rank(ch[0]).ok_or("랭크 오류")?;
    let r2 = parse_rank(ch[1]).ok_or("랭크 오류")?;
    let (hi, lo) = if r1 >= r2 { (r1, r2) } else { (r2, r1) };
    let kind = match (hi == lo, ch.get(2).map(|c| c.to_ascii_lowercase())) {
        (true, None) => Kind::Pair,
        (true, Some(_)) => return Err("페어에는 s/o를 붙일 수 없음".into()),
        (false, None) => Kind::Both,
        (false, Some('s')) => Kind::Suited,
        (false, Some('o')) => Kind::Offsuit,
        _ => return Err("s/o 표기 오류".into()),
    };
    Ok((hi, lo, kind))
}

fn parse_token(range: &mut Range, token: &str) -> Result<(), String> {
    let (body, w) = match token.split_once(':') {
        Some((b, w)) => {
            let w: f32 = w.trim().parse().map_err(|_| "가중치 오류")?;
            if !(0.0..=1.0).contains(&w) {
                return Err("가중치는 0~1".into());
            }
            (b.trim(), w)
        }
        None => (token, 1.0),
    };

    // 특정 콤보: AsKs
    if body.len() == 4 && body.chars().nth(1).and_then(parse_suit).is_some() {
        let cards = parse_cards(body)?;
        range.set(cards[0], cards[1], w);
        return Ok(());
    }

    // 구간: 22-55, A2s-A5s
    if let Some((a, b)) = body.split_once('-') {
        let (h1, l1, k1) = parse_class(a.trim())?;
        let (h2, l2, k2) = parse_class(b.trim())?;
        if k1 != k2 {
            return Err("구간 양쪽 종류가 다름".into());
        }
        if k1 == Kind::Pair {
            for r in h1.min(h2)..=h1.max(h2) {
                range.add_class(r, r, Kind::Pair, w);
            }
        } else {
            if h1 != h2 {
                return Err("구간은 첫 랭크가 같아야 함 (예: A2s-A5s)".into());
            }
            for r in l1.min(l2)..=l1.max(l2) {
                range.add_class(h1, r, k1, w);
            }
        }
        return Ok(());
    }

    // 플러스: TT+, A2s+
    if let Some(base) = body.strip_suffix('+') {
        let (hi, lo, kind) = parse_class(base)?;
        if kind == Kind::Pair {
            for r in hi..=12 {
                range.add_class(r, r, Kind::Pair, w);
            }
        } else {
            for r in lo..hi {
                range.add_class(hi, r, kind, w);
            }
        }
        return Ok(());
    }

    let (hi, lo, kind) = parse_class(body)?;
    range.add_class(hi, lo, kind, w);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> f32 {
        Range::parse(s).unwrap().num_combos()
    }

    #[test]
    fn combo_counts() {
        assert_eq!(n("AA"), 6.0);
        assert_eq!(n("AKs"), 4.0);
        assert_eq!(n("AKo"), 12.0);
        assert_eq!(n("AK"), 16.0);
        assert_eq!(n("KA"), 16.0);
        assert_eq!(n("TT+"), 30.0);
        assert_eq!(n("A2s+"), 48.0);
        assert_eq!(n("K9o+"), 48.0);
        assert_eq!(n("22-44"), 18.0);
        assert_eq!(n("A5s-A2s"), 16.0);
        assert_eq!(n("AsKs"), 1.0);
        assert_eq!(n("AA:0.5"), 3.0);
        assert_eq!(n("AA, KK, AKs"), 16.0);
        assert_eq!(n("22+,A2+,K2+,Q2+,J2+,T2+,92+,82+,72+,62+,52+,42+,32"), 1326.0);
    }

    #[test]
    fn errors() {
        assert!(Range::parse("AAs").is_err());
        assert!(Range::parse("AX").is_err());
        assert!(Range::parse("AA:2").is_err());
        assert!(Range::parse("A2s-K5s").is_err());
    }
}
