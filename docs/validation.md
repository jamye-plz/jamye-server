# 검증 절차

루트 `Justfile`이 유일한 공개 명령 목록이다. Task 번호, RED/GREEN card,
evidence generation, 수동 manifest digest는 과거 구현 과정의 기록이며 현재 검증
절차에는 포함하지 않는다.

## 완료 기준

| 명령 | 목적 | 통과가 의미하는 것 |
| --- | --- | --- |
| `just check` | format, strict Clippy, default/all-feature test, contract drift | 커밋 대상 범위의 백엔드 구현 완료 |
| `just test-recovery` | guarded PostgreSQL/Redis stop-start scenario | release용 복구 동작 확인 |
| `just flake-check` | output tree 평가와 현재 host flake check | 현재 host 검증 통과. NixOS 배포 module 완료를 뜻하지 않음 |
| `just coverage` | all-feature coverage report 생성 | 참고 지표. 특정 퍼센트가 release 권한을 갖지 않음 |

일상 개발의 기본 gate는 `just check`다. 개발 fixture가 production 기본 graph에서
빠지는 것도 서버 contract이므로 default와 all-feature 두 mode를 모두 실행한다.
architecture test는 전체 test inventory에 이미 포함되므로 별도 aggregate 명령으로
중복 실행하지 않는다.

`just test-recovery`는 repository가 소유하는 local test container만 변경한다.
named-volume data를 보존한 채 PostgreSQL을 한 번, Redis를 세 번 정지하고 재시작한다.
느리고 stateful한 검증이므로 `just check`와 분리한다.

## 보조 검증

- `just format`은 Rust source를 변경하고 `just format-check`는 검사만 한다.
- `just clippy`는 모든 target/feature를 warning deny로 검사한다.
- `just test`는 `.env.local`을 읽어 default와 all-feature suite를 실행한다.
- `just contract-generate`는 committed contract를 갱신한다.
- `just contract-check`는 contract 생성의 결정성과 byte 일치를 검사한다.
- `just dependency-check`, `just secret-check`, `just lock-check`는 모든 local test에
  중복 결합하지 않고 필요할 때 명시적으로 실행하는 supply-chain 검사다.
  `secret-check`는 과거 commit 전체가 아니라 tracked file과 ignore되지 않은 새 file의
  현재 내용을 검사한다.

검토된 구현 이력은 Git commit이 보존하고 dependency resolution은 `Cargo.lock`과
`flake.lock`이 보존한다. validator 자체를 다시 검증하는 별도 hash chain은 두지 않는다.
flake 명령은 Git-tracked working tree를 source로 사용하므로 새 flake 입력 파일은 먼저
index에 추가해야 하며 `target/` 같은 ignored local output은 Nix store에 복사하지 않는다.

다음 Nix 배포 단계에서는 실제 NixOS module을 추가·평가하고 production architecture별
package를 실현한 뒤 service health smoke를 수행해야 한다. `just check` 통과만으로 배포
준비 완료를 선언하지 않는다.
