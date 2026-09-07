# ADR 0007: 루트 Justfile 검증 체계

- 상태: Accepted
- 날짜: 2026-09-07
- 대체 대상: ADR 0002의 task module과 command card 구성

## Context

feature 구현 과정에서는 task별 Just module, command card, generation manifest,
digest receipt, immutable dispatcher 사본을 사용했다. 이 장치는 세밀한 TDD 이력을
남겼지만 점차 제품보다 validator를 검증하는 비용이 커졌다. 일상 명령은 repository
루트에서 찾기 어려웠고 무해한 수정도 전체 evidence generation을 반복해서 무효화했다.

## Decision

루트 `Justfile`을 유일한 공개 workflow surface로 정한다. format, lint, test,
contract drift, coverage, dependency 검사, local infrastructure, recovery test,
flake check를 기능 이름 그대로 노출한다.

Shell script는 guarded local-volume 삭제, credential 생성, contract 임시 디렉터리
정리, MinIO policy 설정, service stop/start recovery처럼 실제 안전 또는 orchestration
경계가 있을 때만 유지한다. 이 script는 역사적 Task 번호가 아니라 `scripts/dev/`,
`scripts/recovery/`, 최상위 `scripts/`에 둔다.

구현 기록은 Git history가, dependency resolution 기록은 Cargo/Nix lockfile이 맡는다.
coverage는 참고 지표로 사용한다. `just check`는 구현 완료 gate이며 release 준비는
명시적인 recovery/package/module 검증이 담당한다.

## Consequences

- `just` 한 곳에서 지원하는 모든 공개 명령을 확인할 수 있다.
- feature test가 문서나 task runner 파일의 존재에 의존하지 않는다.
- 과거 RED/GREEN card와 manifest generation은 Git에서 복구할 수 있지만 active tree에는
  남기지 않는다.
- stateful recovery는 명시적으로 유지하되 일반 gate 안에 숨기지 않는다.
- NixOS 배포 준비는 concrete module, target build, evaluation, runtime smoke를 갖춘 별도
  단계로 유지한다.
