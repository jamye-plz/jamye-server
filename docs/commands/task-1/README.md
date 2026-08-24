# task-1 command cards

M0의 consequential 명령은 사용자가 직접 실행한다. 에이전트는 아래 card의 script를 작성하고 결과를 해석하지만 Nix lock/develop, Cargo resolution/build/test, Podman lifecycle, secret scan을 대신 실행하지 않는다.

모든 card는 repository root에서 `path:.` flake를 사용한다.

| Card | 목적 |
|---|---|
| [environment](environment.md) | devShell, provider, disposable local env 준비 |
| [locks](locks.md) | Cargo/Nix lock 생성과 no-drift 검증 |
| [platform](platform.md) | format, warning-as-error lint, default/all-feature test, architecture test |
| [local-infrastructure](local-infrastructure.md) | rootless Podman의 명시적 lifecycle과 guarded reset |
| [runtime-health](runtime-health.md) | API/worker 실행과 live/ready 확인 |
| [security](security.md) | license/advisory와 working-directory secret scan self-test |
| [flake](flake.md) | local-system flake check와 Linux production builder gate |

명령 결과를 전달할 때 command, exit code, 마지막 실패 구간을 함께 보존한다. Linux builder나 외부 service가 없으면 성공으로 간주하거나 조용히 skip하지 않고 blocker로 기록한다.

M0의 canonical 실행 순서는 다음과 같다.

1. environment card의 devShell/provider/toolchain 확인
2. lock create/verify
3. dependency check와 gitleaks clean/detect/clean self-test
4. environment card의 `.env.local` 생성
5. platform check
6. rootless local infrastructure up/status
7. API/worker/health 확인
8. local flake check
9. x86_64-linux package build 또는 explicit Linux-builder blocker

`gitleaks dir .`은 `.gitignore`를 자동 적용하지 않고 directory 전체를 읽는다. 따라서 synthetic scanner self-test는 의도적으로 credential-bearing `.env.local`을 만들기 전에 수행한다. script는 파일을 숨기거나 임시 이동하지 않고 `.env.local`이 있으면 안전하게 실패한다.
