# Poker Solver (학습 → 데스크탑 앱)

GTO Wizard 같은 포스트플랍 솔버를 데스크탑 앱으로 만드는 프로젝트.
2026-11-24 시작. 아래는 첫날 정리한 내용.

## 0. 바로 실행해 보기

준비: Rust 설치 (https://rustup.rs, Visual Studio C++ Build Tools 필요) + VS Code 확장 `rust-analyzer`

```powershell
git clone https://github.com/donghyun999/poker-solver.git
cd poker-solver

# 테스트 전부 (처음 빌드는 몇 분 걸림)
cargo test

# 리버 이론 토이 게임 — 너트 100% 벳, 블러프 37.5%, 콜 50%가 나와야 정상
cargo run --release -p cli -- --board "Kh Qd 7c 3s 2h" --oop "77,54s" --ip "88" --bet 100 --raise "" --no-allin --max-raises 0

# 위 결과에서 OOP 벳(1번 액션) 후 IP의 대응 보기
cargo run --release -p cli -- --board "Kh Qd 7c 3s 2h" --oop "77,54s" --ip "88" --bet 100 --raise "" --no-allin --max-raises 0 --line 1

# 실전 턴 스팟 (1초 안팎)
cargo run --release -p cli -- --board "Kh 9d 4c 2s" --pot 20 --stack 87 --max-raises 1 --oop "22-JJ,A2s-AJs,K2s+,Q4s+,J6s+,T6s+,95s+,85s+,74s+,63s+,53s+,43s,A2o-AJo,K8o+,Q9o+,J9o+,T8o+,98o" --ip "22+,A2s+,K2s+,Q5s+,J7s+,T7s+,96s+,86s+,75s+,64s+,54s,A2o+,K9o+,Q9o+,J9o+,T9o"

# 옵션 전체
cargo run --release -p cli -- --help

# Kuhn 포커 (학습용)
python learning/kuhn_cfr.py
cargo run --release -p kuhn
```

플랍은 메모리를 많이 씁니다 (턴/리버 사이즈 1개씩이어도 약 6GB, 3분). 예: `--board "Kh 9d 4c" --pot 6 --stack 97 --bet 33,75 --turn-bet 75 --river-bet 75 --max-raises 1`

엔진 설계·구현 현황·성능 기록: [docs/DESIGN.md](docs/DESIGN.md)

---

## 1. 전체 구조 (결정 사항)

```
[TypeScript / React]  ← UI, CRUD(로컬 SQLite)
        │  invoke (비동기 요청/응답)
        │  emit/listen (Rust → TS 진행률 이벤트)
        ▼
[Rust]  솔버 엔진 (CFR, 핸드 평가기, 게임 트리)
```

- 앱 프레임워크: **Tauri** (Rust 백엔드 + TS 프론트 한 앱)
- 데스크탑 앱이라 별도 백엔드 서버 불필요 → CRUD는 SQLite (`tauri-plugin-sql`)
- 엔진 결과(수 GB)는 Rust 메모리에 두고, 화면에서 보는 노드만 TS로 조회
- 범위: **헤즈업 포스트플랍**부터. 프리플랍·멀티웨이는 나중에

## 2. 왜 Rust인가
- 연산량: 노드 수십만~수백만 × 콤보 ~1000 × 액션 2~4 × 반복 수천 회
- 메모리 수 GB → 네이티브 언어 필수 (Rust ≈ C++ 성능, 메모리 안전, `rayon` 병렬화)
- 엔진 핵심은 객체지향보다 **데이터 지향**: 콤보를 객체로 쪼개지 말고 노드마다 연속 배열(`Vec<f32>`)로

## 3. 동기 / 비동기 / 이벤트 / 병렬
- **동기**: 호출한 스레드가 끝날 때까지 블로킹
- **비동기**: 호출 즉시 Promise 반환, 작업은 다른 스레드에서. 이점은 *응답성*이지 속도가 아님
  - CFR은 비동기로 해도 워커 스레드가 3분 점유함 (점유 주체만 바뀜)
- **병렬**: 여러 코어로 동시 계산 (`rayon`) → 이게 속도를 올림
- **이벤트**: pub/sub 통지 패턴. 비동기 전용 아님 (Node `EventEmitter.emit`은 동기). Tauri `emit`은 IPC라 비동기 전달
- 비동기 비용: 상태 관리(계산중/완료/실패/취소), 경쟁 상태, 에러 전달, 오버헤드
- 솔버 원칙: **경계(TS↔Rust)는 비동기, 엔진 내부 CFR 루프는 동기 + 병렬**

## 4. 솔버 원리 (포커 용어로)
- 질문: "이 보드에서 내 레인지 모든 핸드를 각각 몇 % 벳/체크해야 상대가 공략 못 하나?"
- 계산: 모든 내 핸드 × 모든 선택 × 모든 상대 핸드 × 모든 남은 턴/리버 카드 → 각 선택의 EV
- 손익 판단은 **칩만**: 모든 라인은 폴드 or 쇼다운으로 끝남 → 칩 결과를 확률로 평균 = EV
- 다 계산 가능한 이유: 범위를 좁힘
  - 벳 사이즈 몇 개만 (33%, 75%, 올인)
  - 한 스팟만 (예: BTN vs BB SRP, 플랍 AsKd7c)
  - 레인지는 사용자 입력
  - 2인 기준
- 신뢰도: **익스플로이터빌리티** = 상대가 내 전략을 완벽히 공략할 때 뺏어갈 수 있는 양. 팟의 0.3~0.5% 이하면 충분. 헤즈업에서 CFR 수렴은 수학적으로 증명됨. 단, 입력 조건(레인지·벳 사이즈) 안에서만 정답

## 5. CFR (Counterfactual Regret Minimization)
1. 현재 전략으로 각 액션의 EV 계산
2. **Regret(아쉬움 점수)** = 그 액션 EV − 현재 전략 EV → 누적
3. **Regret Matching**: 누적 regret이 플러스인 액션에 비례해 다음 전략 배분
4. 양쪽 플레이어가 서로 고치기를 수천 번 반복
5. 최종 답은 **마지막 전략이 아니라 평균 전략** → GTO로 수렴

예) KK 리버, 체크 50% / 벳 50% 시작. 체크 EV +2, 벳 EV +6, 전략 EV +4
→ regret 체크 −2, 벳 +2 → 다음 전략 벳 100% → 상대가 대응하며 다시 조정

## 6. 학습용 코드: Kuhn Poker
[learning/kuhn_cfr.py](learning/kuhn_cfr.py) — 카드 3장(J,Q,K) 포커 CFR

```
python learning/kuhn_cfr.py
```

200,000회 결과 (이론값과 일치):

| 항목 | 결과 | 이론 |
|---|---|---|
| P1 Q 먼저 벳 | 0.0% | 0% |
| P1 K 벳 ÷ J 블러프 | 3.05 | 3.00 |
| P2 J 블러프 (P1 체크 후) | 33.7% | 33.3% |
| P2 Q 콜 (P1 벳에) | 34.0% | 33.3% |
| P1 게임 가치 | −0.061 | −0.056 (초반 반복·무작위 딜 때문에 약간 차이) |

코드에서 볼 곳: `strategy()` (Regret Matching), `cfr()` 주석 1~3번 (EV·regret 계산), `average_strategy()` (평균 전략)

## 7. 개발 원칙
- 바이브코딩: UI·CRUD는 적극 활용. **CFR 엔진은 한 줄씩 설명할 수 있을 때까지 이해**
  - 엔진 버그는 에러 없이 "그럴듯한 틀린 전략"을 냄 → 정답 아는 게임(Kuhn)과 오픈소스 솔버 결과로 검증 테스트 필수
- 참고 오픈소스: `b-inary/postflop-solver`(Rust), `desktop-postflop`(Tauri), `TexasSolver`(C++)
  - ⚠️ AGPL 라이선스 → 상용이면 구조만 참고, 코드 복사 금지

## 8. 다음 할 일
엔진 설계와 단계별 마일스톤: [docs/DESIGN.md](docs/DESIGN.md)

- [ ] `ITER`를 100, 1000으로 줄여 돌려보기 → 반복 수에 따른 수렴 체감
- [x] Rust 설치 (`rustup`) 후 Kuhn CFR을 Rust로 옮기기
- [x] NLHE 리버 솔버 (핸드 평가기, 레인지 파서, 트리 빌더, DCFR, 익스플로이터빌리티)
- [x] 턴/플랍 확장 (찬스 노드) + 찬스 노드 병렬화
- [ ] 외부 솔버와 결과 비교
- [ ] 최적화 (수트 동형성, 메모리 압축)
- [ ] Tauri로 UI 연결

### 코드 읽는 순서 (추천)
1. [engine/src/solver.rs](engine/src/solver.rs) 맨 위 주석 → `cfr()` 함수. Kuhn의 `cfr()`과 같은 구조인데 숫자 하나 대신 **콤보 벡터**를 다룸
2. `showdown_value()` — 쇼다운을 O(n)으로 계산하는 부분
3. 테스트 `river_toy_game_matches_theory` — 정답을 어떻게 검증하는지
4. [engine/src/tree.rs](engine/src/tree.rs) — 벳/레이즈/올인 트리가 만들어지는 규칙
