# M0 flake card

## 목적

untracked working tree를 포함하는 `path:.`에서 output shape와 current-system check를 검증하고, 별도 x86_64-linux builder로 production package lane을 증명한다.

## Local-system 명령

```bash
nix develop path:. --command just task-1 flake-local
```

이 card는 `aarch64-darwin`, `x86_64-linux` output을 모두 show하고 현재 system의 flake checks를 실행한다. lock을 갱신하지 않는다.

## Production Linux 명령

```bash
nix develop path:. --command just task-1 flake-linux
```

macOS에서 실행하려면 설정된 x86_64-linux remote/VM builder가 필요하다. 이 저장소는 builder를 생성하거나 설정하지 않는다.

## 부작용

Nix store에 derivation과 build result가 추가될 수 있다. `--no-link`이므로 repository result symlink는 만들지 않는다. lock, Git, service, production host는 변경하지 않는다.

## 성공 기준

- output에 두 system의 `api`, `worker`, `checks`, `devShells.default`가 보인다.
- local flake check가 exit `0`이다.
- Linux builder가 있는 환경에서 x86_64-linux `api`, `worker` build가 exit `0`이다.

## 복구

Linux builder가 없으면 그 오류를 PF2 blocker로 기록하고 production matrix 성공을 주장하지 않는다. flake evaluation 또는 package 오류면 full trace의 첫 substantive error를 보존해 수정한 뒤 다시 실행한다.
