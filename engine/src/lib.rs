//! 솔버 엔진. UI에 의존하지 않는 순수 라이브러리.
//! 설계: docs/DESIGN.md

pub mod card;
pub mod eval;
pub mod file;
pub mod range;
pub mod report;
pub mod solver;
pub mod tree;

pub use range::Range;
pub use report::{ClassRow, NodeSummary};
pub use solver::{Game, Progress, SolveConfig, Solver, Step};
pub use tree::{Action, NodeKind, StreetSizes, TreeConfig, parse_sizes, sizes_to_string};

/// 엔진 버전. 미리 계산한 결과 파일에 함께 기록한다.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
