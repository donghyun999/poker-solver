use kuhn::{CARD_NAMES, Trainer};
use std::time::Instant;

fn main() {
    let iterations: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);

    let start = Instant::now();
    let mut t = Trainer::new();
    t.train(iterations);
    let elapsed = start.elapsed();

    println!("반복 {iterations}회 ({:.2}초)\n", elapsed.as_secs_f64());

    // (기록, 상황 설명, 액션 이름)
    let rows = [
        ("", "P1 첫 액션", ["체크", "벳"]),
        ("b", "P2, P1 벳에 대응", ["폴드", "콜"]),
        ("p", "P2, P1 체크 후", ["체크", "벳"]),
        ("pb", "P1, P2 벳에 대응", ["폴드", "콜"]),
    ];
    for (h, situation, acts) in rows {
        println!("[{situation}]");
        for card in CARD_NAMES {
            let s = t.avg(&format!("{card}{h}"));
            println!(
                "  {card}   {} {:6.1}%   {} {:6.1}%",
                acts[0],
                s[0] * 100.0,
                acts[1],
                s[1] * 100.0
            );
        }
    }

    let j_bet = t.avg("J")[1];
    let k_bet = t.avg("K")[1];
    println!("\n[검증]");
    println!("P1 게임 가치    : {:+.4}   (이론 -0.0556)", t.game_value());
    println!("P1 Q 먼저 벳    : {:.1}%     (이론 0%)", t.avg("Q")[1] * 100.0);
    if j_bet > 0.0 {
        println!("P1 K 벳 / J 벳  : {:.2}      (이론 3.00)", k_bet / j_bet);
    }
}
