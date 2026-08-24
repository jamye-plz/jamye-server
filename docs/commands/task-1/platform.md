# M0 platform card

## 목적

M0/C0 코드의 format, warning-as-error Clippy, default/all-feature test, 독립 architecture boundary test를 실행한다.

## 선행 조건

- lock card가 통과했다.
- 이 card 자체는 PostgreSQL/Redis/MinIO를 시작하지 않는다. M0 platform tests는 test double 또는 자신이 소유한 process fixture를 사용해야 한다.

## 명령

```bash
nix develop path:. --command just task-1 platform-check
```

## 부작용

Cargo가 dependency와 test artifact를 Nix/Cargo cache에 compile한다. source, lock, service state, Git state는 변경하지 않아야 한다.

## 성공 기준

- format check exit `0`
- `cargo clippy --all-targets --all-features`가 warning 없이 exit `0`
- default와 `--all-features` test가 각각 exit `0`
- `tests/architecture` target이 dependency direction 위반 없이 exit `0`

## 복구

첫 실패에서 멈춘다. 해당 command와 diagnostic을 그대로 전달하고 code 또는 test를 고친 뒤 card 전체를 다시 실행한다. failure를 warning으로 낮추거나 test를 skip하지 않는다.
