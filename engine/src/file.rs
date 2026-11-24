//! 솔브 결과 저장/불러오기.
//!
//! 형식 (리틀엔디언 바이너리)
//! - 매직 "PKSOLV01", 엔진 버전 문자열
//! - 설정: 보드, 레인지 2개(1326 f32씩), 트리 설정
//! - 반복 횟수, 저장소(F32 또는 C16) 원본 배열
//!
//! 불러올 때는 설정으로 트리를 다시 만들고 저장소만 채운다.
//! 트리 생성 규칙이 바뀌면 결과가 안 맞으므로 **엔진 버전이 다르면 거부**한다.

use crate::range::Range;
use crate::solver::{Game, SolveConfig, Solver, Storage};
use crate::tree::{StreetSizes, TreeConfig};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"PKSOLV01";

struct W<T: Write>(T);

impl<T: Write> W<T> {
    fn bytes(&mut self, b: &[u8]) -> std::io::Result<()> {
        self.0.write_all(b)
    }
    fn u8(&mut self, x: u8) -> std::io::Result<()> {
        self.bytes(&[x])
    }
    fn u32(&mut self, x: u32) -> std::io::Result<()> {
        self.bytes(&x.to_le_bytes())
    }
    fn u64(&mut self, x: u64) -> std::io::Result<()> {
        self.bytes(&x.to_le_bytes())
    }
    fn f64(&mut self, x: f64) -> std::io::Result<()> {
        self.bytes(&x.to_le_bytes())
    }
    fn str(&mut self, s: &str) -> std::io::Result<()> {
        self.u32(s.len() as u32)?;
        self.bytes(s.as_bytes())
    }
    fn f64s(&mut self, v: &[f64]) -> std::io::Result<()> {
        self.u32(v.len() as u32)?;
        v.iter().try_for_each(|&x| self.f64(x))
    }
    /// 숫자 배열을 그대로 기록 (f32/i16/u16, 리틀엔디언 플랫폼 가정)
    fn raw<N: Copy>(&mut self, v: &[N]) -> std::io::Result<()> {
        self.u64(v.len() as u64)?;
        // SAFETY: N은 패딩 없는 기본 숫자 타입
        let b = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) };
        self.bytes(b)
    }
}

struct R<T: Read>(T);

impl<T: Read> R<T> {
    fn bytes(&mut self, n: usize) -> std::io::Result<Vec<u8>> {
        let mut b = vec![0; n];
        self.0.read_exact(&mut b)?;
        Ok(b)
    }
    fn arr<const N: usize>(&mut self) -> std::io::Result<[u8; N]> {
        let mut b = [0; N];
        self.0.read_exact(&mut b)?;
        Ok(b)
    }
    fn u8(&mut self) -> std::io::Result<u8> {
        Ok(self.arr::<1>()?[0])
    }
    fn u32(&mut self) -> std::io::Result<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    fn u64(&mut self) -> std::io::Result<u64> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    fn f64(&mut self) -> std::io::Result<f64> {
        Ok(f64::from_le_bytes(self.arr()?))
    }
    fn str(&mut self) -> std::io::Result<String> {
        let n = self.u32()? as usize;
        Ok(String::from_utf8_lossy(&self.bytes(n)?).into_owned())
    }
    fn f64s(&mut self) -> std::io::Result<Vec<f64>> {
        let n = self.u32()? as usize;
        (0..n).map(|_| self.f64()).collect()
    }
    fn raw_into<N: Copy>(&mut self, out: &mut [N]) -> Result<(), String> {
        let n = self.u64().map_err(io)? as usize;
        if n != out.len() {
            return Err(format!("저장소 크기 불일치: 파일 {n}, 예상 {}", out.len()));
        }
        // SAFETY: N은 기본 숫자 타입이라 어떤 비트 패턴도 유효
        let b = unsafe { std::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut u8, std::mem::size_of_val(out)) };
        self.0.read_exact(b).map_err(io)
    }
}

fn io(e: std::io::Error) -> String {
    e.to_string()
}

impl Solver {
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let f = File::create(path).map_err(io)?;
        let mut w = W(BufWriter::new(f));
        let cfg = &self.game.config;
        (|| -> std::io::Result<()> {
            w.bytes(MAGIC)?;
            w.str(crate::VERSION)?;
            w.u8(cfg.board.len() as u8)?;
            w.bytes(&cfg.board)?;
            for r in &cfg.ranges {
                let v: Vec<f32> = r.weights.clone();
                w.raw(&v)?;
            }
            let t = &cfg.tree;
            w.f64(t.starting_pot)?;
            w.f64(t.effective_stack)?;
            for street in &t.sizes {
                for s in street {
                    w.f64s(&s.bet)?;
                    w.f64s(&s.raise)?;
                }
            }
            w.u32(t.max_raises)?;
            w.u8(t.allow_allin as u8)?;
            w.u32(self.iteration)?;
            match &self.storage {
                Storage::F32 { regrets, strat } => {
                    w.u8(0)?;
                    w.raw(regrets)?;
                    w.raw(strat)?;
                }
                Storage::C16 { regrets, strat, rscale, sscale } => {
                    w.u8(1)?;
                    w.raw(regrets)?;
                    w.raw(strat)?;
                    w.raw(rscale)?;
                    w.raw(sscale)?;
                }
            }
            w.0.flush()
        })()
        .map_err(io)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Solver, String> {
        let f = File::open(path).map_err(io)?;
        let mut r = R(BufReader::new(f));
        if &r.arr::<8>().map_err(io)? != MAGIC {
            return Err("솔버 결과 파일이 아님".into());
        }
        let ver = r.str().map_err(io)?;
        if ver != crate::VERSION {
            return Err(format!("엔진 버전이 다름: 파일 {ver}, 현재 {}. 다시 솔브하세요", crate::VERSION));
        }
        let nb = r.u8().map_err(io)? as usize;
        let board = r.bytes(nb).map_err(io)?;
        let mut ranges = [Range::new(), Range::new()];
        for rg in &mut ranges {
            r.raw_into(&mut rg.weights)?;
        }
        let starting_pot = r.f64().map_err(io)?;
        let effective_stack = r.f64().map_err(io)?;
        let mut sizes: [[StreetSizes; 2]; 3] = Default::default();
        for street in &mut sizes {
            for s in street {
                s.bet = r.f64s().map_err(io)?;
                s.raise = r.f64s().map_err(io)?;
            }
        }
        let max_raises = r.u32().map_err(io)?;
        let allow_allin = r.u8().map_err(io)? != 0;
        let iteration = r.u32().map_err(io)?;
        let tree = TreeConfig { starting_pot, effective_stack, sizes, max_raises, allow_allin };

        let game = Game::new(&SolveConfig { board, ranges, tree })?;
        let compressed = r.u8().map_err(io)? == 1;
        let mut solver = Solver::with_compression(game, compressed);
        solver.iteration = iteration;
        match &mut solver.storage {
            Storage::F32 { regrets, strat } => {
                r.raw_into(regrets)?;
                r.raw_into(strat)?;
            }
            Storage::C16 { regrets, strat, rscale, sscale } => {
                r.raw_into(regrets)?;
                r.raw_into(strat)?;
                r.raw_into(rscale)?;
                r.raw_into(sscale)?;
            }
        }
        Ok(solver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::parse_cards;

    #[test]
    fn save_and_load_roundtrip() {
        let tree = TreeConfig::uniform(10.0, 50.0, &[0.5, 1.0], &[1.0]);
        let cfg = SolveConfig {
            board: parse_cards("Ks 9d 5c 3h").unwrap(),
            ranges: [Range::parse("AA,KK,99,AK,KQ,T9s,A5s").unwrap(), Range::parse("QQ-TT,AQ,KJ,98s,65s").unwrap()],
            tree,
        };
        let dir = std::env::temp_dir();
        for compressed in [false, true] {
            let mut s = Solver::with_compression(Game::new(&cfg).unwrap(), compressed);
            for _ in 0..50 {
                s.iterate();
            }
            let path = dir.join(format!("pksolver_test_{compressed}.bin"));
            s.save(&path).unwrap();
            let l = Solver::load(&path).unwrap();
            std::fs::remove_file(&path).ok();
            assert_eq!(l.iteration, 50);
            assert_eq!(l.is_compressed(), compressed);
            assert_eq!(l.strategy(0), s.strategy(0));
            assert_eq!(l.exploitability(), s.exploitability());
        }
    }
}
