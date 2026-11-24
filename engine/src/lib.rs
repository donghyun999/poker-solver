//! 솔버 엔진. UI에 의존하지 않는 순수 라이브러리.
//! 설계: docs/DESIGN.md

pub mod card;
pub mod eval;
pub mod range;

/// 엔진 버전. 미리 계산한 결과 파일에 함께 기록한다.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
