# M0 environment card

## 목적

flake가 제공하는 도구를 확인하고, 서비스 시작 없이 Compose provider가 Nix store에 고정됐는지 검증한 뒤 disposable `.env.local`을 생성한다.

## 명령

먼저 devShell에 들어간다.

```bash
nix develop path:.
```

shell 안에서 provider를 확인한다. 이 명령은 container나 VM을 시작하지 않는다.

```bash
just task task-1 provider-check
```

이후 lock card와 security card의 gitleaks self-test를 먼저 완료한다. `gitleaks dir .`은 `.gitignore`와 무관하게 local file까지 읽으므로 credential-bearing file을 먼저 만들지 않는다.

두 gate가 통과한 뒤 처음 한 번만 local 값을 생성한다.

```bash
just task task-1 local-env-create
```

## 부작용

- `nix develop path:.`는 flake input을 fetch하고 local Nix store를 채울 수 있다. lock이 없으면 Nix가 `flake.lock`을 만들 수 있으므로 lock card와 같은 승인 구간에서 실행한다.
- local-env card는 mode `0600`의 untracked `.env.local` 하나를 만든다. 실제 값은 실행 시 `/dev/urandom`에서 생성한다.
- VM, container, volume, bucket, MinIO policy는 만들지 않는다.

## 성공 기준

- `rustc`, `cargo`, `podman`, `podman-compose` 경로가 `/nix/store/` 아래다.
- active `rustc --version`이 `rust-toolchain.toml`에서 읽은 exact channel과 일치한다.
- `PODMAN_COMPOSE_PROVIDER`가 같은 Nix `podman-compose` executable을 가리킨다.
- `.env.local`이 만들어지고 MinIO admin/app 값은 서로 다르다.

## 복구

provider가 ambient 경로라면 shell을 나갔다가 `nix develop path:.`로 다시 들어간다. `.env.local`이 이미 있으면 script는 덮어쓰지 않는다. 값 회전이 필요하면 local infra를 먼저 reset하고 사용자가 `.env.local`을 별도로 삭제한 뒤 card를 다시 실행한다.
