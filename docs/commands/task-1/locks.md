# M0 lock card

## 목적

사용자가 현재 M0 dependency graph의 `Cargo.lock`과 flake input의 `flake.lock`을 생성하고, exact checksum과 반복 평가의 no-drift를 증명한다.

## 명령

repository root에서 실행한다.

```bash
nix develop path:. --command just task-1 locks-create
nix develop path:. --command just task-1 locks-verify
```

## 부작용

- registry와 flake input을 network에서 읽고 Nix/Cargo cache를 채울 수 있다.
- `Cargo.lock`, `flake.lock`을 생성하거나 dependency 변경에 맞게 갱신한다.
- staging, commit, tag, push는 수행하지 않는다.

## 성공 기준

- 두 lockfile이 존재한다.
- create가 두 SHA-256을 출력한다.
- verify가 locked Cargo metadata와 `path:.` flake metadata/show를 평가한 뒤 같은 SHA-256을 출력한다.

## 복구

official-current dependency가 resolve되지 않으면 오류를 보존하고 해당 dependency owner에게 돌려보낸다. public contract, security boundary, supported system이 바뀌지 않는 호환 patch는 해당 task가 조정하고 다시 실행한다. lock을 수동 편집하지 않는다.
