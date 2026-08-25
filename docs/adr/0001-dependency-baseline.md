# ADR 0001: M0/C0 dependency baseline

- 상태: Accepted; M0 and task-3c lock/dependency/GREEN evidence complete
- 날짜: 2026-08-24
- 범위: M0 platform과 C0 contract generation에 실제 사용하는 dependency만

## Context

초기 dependency를 미래 전체 backend 목록으로 채우면 compile graph, license surface, upgrade 비용이 불필요하게 커진다. 반대로 Rust/Nix/runtime client가 제각기 floating하면 macOS development와 Linux production이 같은 source를 다르게 해석한다.

이번 baseline은 M0 config/logging/request ID/shutdown/health와 C0 deterministic OpenAPI/JSON Schema 생성을 구현하는 데 필요한 graph만 고정한다. auth provider, Expo push delivery, AWS S3 SDK는 각 owner task 전까지 추가하지 않는다.

## Decision

### Rust와 Nix

- stable Rust `1.98.0`을 `rust-toolchain.toml`의 exact channel로 고정한다.
- profile, components, targets도 같은 파일에만 둔다.
- flake는 Fenix `fromToolchainFile`로 이 파일을 한 번 소비한다.
- Fenix에 전달하는 SRI hash `sha256-P30Tm3O7vQAE725YtDCDHGjNrSsfZO4us11UwJGZSJo=`는 official `channel-rust-1.98.0.toml` manifest의 integrity 값이다. 두 번째 version 선언이 아니다.
- 같은 resulting toolchain derivation을 Crane `api`/`worker` package와 devShell에 사용한다.
- flake input의 exact revisions와 Nix-provided tool versions는 사용자가 생성할 `flake.lock`이 고정한다.
- `flake.lock`이 고정한 Crane의 `cargoClippy`와 `cargoTest`는 `cargoExtraArgs` 기본값으로 이미 `--locked`를 전달한다. 따라서 `cargoClippyExtraArgs`/`cargoTestExtraArgs`에는 target·feature·lint selector만 두며 `--locked`를 중복하지 않는다.

Fenix는 upstream이 `aarch64-darwin`과 `x86_64-linux`를 supported platform으로 명시하고 binary cache도 두 platform을 제공하므로 선택했다. oxalica rust-overlay의 `fromRustupToolchainFile`은 hash 없이 더 단순하지만 공식 README가 CI 보장을 주로 x86_64-linux에 한정해 dual-system baseline의 근거가 더 약했다.

Official evidence:

- Rust 1.98.0 release: <https://blog.rust-lang.org/releases/>
- Rust manifest: <https://static.rust-lang.org/dist/channel-rust-1.98.0.toml>
- Fenix API and supported platforms: <https://github.com/nix-community/fenix>
- Crane toolchain override: <https://github.com/ipetkov/crane>

### Cargo graph

`Cargo.toml`의 direct dependency와 feature가 baseline이다. 정확한 resolved transitive graph는 사용자가 만든 `Cargo.lock`이 결정한다. 2026-08-24 official crate release/docs를 확인한 M0/C0 direct requirements는 다음과 같다.

| 책임 | Direct requirement |
|---|---|
| HTTP runtime/middleware/shutdown | `axum 0.8.9`, `tokio 1.53.1`, `tower 0.5.3`, `tower-http 0.7.0` |
| JSON/config shape | `serde 1.0.229`, `serde_json 1.0.151`, `url 2.5.8` |
| structured JSON logging | `tracing 0.1.44`, `tracing-subscriber 0.3.23` |
| ID/time primitive | `uuid 1.25.0`, `time 0.3.55` |
| PostgreSQL readiness/migration baseline | `sqlx 0.9.0` with PostgreSQL, migrate, UUID/time/JSON types, Tokio, Rustls features only |
| Redis degraded readiness | `redis 1.6.0` with Tokio compatibility only |
| MinIO health-only HTTP | `reqwest 0.13.4` with Rustls and no S3/AWS feature |
| deterministic contract generation | `utoipa 5.5.0`, `schemars 1.2.2` |

Registry/source evidence is linked from the crate pages under <https://crates.io/> and each upstream documentation/repository linked there. Feature selection is intentionally narrower than crate defaults where the M0 behavior permits it.

`[features]`는 M0에서 정확히 `default=[]`, `dev-fixtures=[]`로 시작했다. M0
default composition에는 fixture route/key/issuer가 없고 task-3c만 이 feature의
동작과 필요한 optional dependency를 추가할 수 있다.

### task-3c dev-only JWT extension

2026-08-25 RED 확인 뒤 사용자가 option A를 승인했다. task-3c는
`dev-fixtures=["dep:jsonwebtoken"]`과 optional `jsonwebtoken 11.0.0` 하나를 추가한다.
crate default feature의 PEM/ASN.1 surface는 끄고 `aws_lc_rs` crypto backend만
선택한다. upstream manifest의 MSRV는 Rust 1.88이며 현재 exact Rust 1.98.0 범위에
포함된다. default feature graph에는 이 dependency가 들어오지 않는다.

처음 선택한 `rust_crypto` feature는 task-3c가 사용하는 HS256만 세분화해 제공하지
않고 RSA/ECDSA/EdDSA 구현 전체를 함께 활성화한다. 그 결과 사용하지 않는
`rsa 0.9.10`이 `RUSTSEC-2023-0071`에 걸려 dependency gate가 실패했다. advisory
예외를 만들거나 취약 crate를 유지하지 않고, jsonwebtoken이 공식 제공하는
`aws_lc_rs` backend로 교체한다. 이 backend는 같은 HS256 API를 제공하고 기존
repository의 Rustls graph에도 이미 존재하므로 별도 crypto 구현이나 두 번째 JWT
library를 추가하지 않는다.

Official evidence:

- jsonwebtoken 11.0.0 manifest/features/MSRV: <https://docs.rs/crate/jsonwebtoken/11.0.0/source/Cargo.toml.orig>
- backend selection and no-PEM example: <https://docs.rs/crate/jsonwebtoken/11.0.0>
- typed encode/decode API: <https://docs.rs/jsonwebtoken/11.0.0/jsonwebtoken/>
- issuer/audience/required-claim/zero-leeway validation: <https://docs.rs/jsonwebtoken/11.0.0/jsonwebtoken/struct.Validation.html>
- rejected RSA dependency advisory: <https://rustsec.org/advisories/RUSTSEC-2023-0071.html>

첫 lock/no-drift card는 `rust_crypto` graph를 byte-stable하게 고정했지만 이어진
dependency card가 RSA advisory로 실패했으므로 완료 evidence가 아니다. 사용자가
`aws_lc_rs` 교체 뒤 lock/no-drift card를 다시 실행해 `Cargo.lock` SHA-256
`1dce2310998050f3f00e8dd418f169d36ddbc4e91538e2815beedaff4386d87e`와 기존
`flake.lock` SHA-256
`31403f6a698d7386579ca297f53952fd8cb47616affa8ff49c9fc71517f05bd9`가 즉시
재해석 뒤에도 동일함을 확인했다. 새 graph에서 jsonwebtoken은 `aws-lc-rs`만
참조하며 `rsa`, `ed25519-dalek`, `p256`, `p384` package는 없다.
advisory/ban/license/source와 GREEN card가 모두 통과하기 전에는 이 extension을
완료로 표시하지 않는다. 이후 Cargo owner는 필요한 crate만 추가하고 같은
no-drift card를 다시 실행한다.

이어진 사용자 dependency card는 중복 버전 경고를 정보성으로 보고하면서
`advisories ok, bans ok, licenses ok, sources ok`와 exit `0`을 반환했다. 따라서
`RUSTSEC-2023-0071` 회귀는 해소됐고 task-3c 완료에 남은 dependency 관련 blocker는
없다. 최종 사용자 GREEN card도 기본 graph 1/1, guarded unit 1/1, 실제 PostgreSQL
통합 5/5, architecture 4/4와 exit `0`으로 통과했다.

### Security와 workflow tools

devShell은 Nix로 다음 tool을 제공한다.

- `cargo-deny`: advisory, ban, source, explicit SPDX license policy
- `gitleaks`: Git history가 아닌 working-directory scan. 기본 rule을 그대로 사용하고 재생성 가능한 `target/` compiler/linker output만 제외하며 `.env.local`과 ignored/untracked source/state는 계속 포함한다.
- `sqlx-cli`: migration commands
- `just`: stable task-module command catalog
- `podman` + `podman-compose`: rootless local harness
- `minio-client`: M0 local admin/app identity 분리

`PODMAN_COMPOSE_PROVIDER`는 `${pkgs.podman-compose}/bin/podman-compose` derivation path로 설정한다. ambient provider fallback은 허용하지 않는다. 실제 tool version은 flake lock gate에서 출력하고 `flake.lock`으로 고정한다.

`deny.toml`은 conservative explicit SPDX allowlist, unlicensed/copyleft/unknown deny, narrowly crate-and-version-scoped exception, unused exception failure를 사용한다. blanket exception은 금지한다.

### Local service images

Compose image는 mutable `latest`가 아닌 exact release tag를 사용한다.

| Service | M0 local-only tag | 근거 |
|---|---|---|
| PostgreSQL | `17.11-bookworm` | Docker Official Image의 current 17 patch tag |
| Redis | `8.10.1-alpine3.23` | Docker Official Image의 exact 8.10 patch/base tag |
| MinIO | `RELEASE.2025-09-07T16-13-09Z` | official registry의 마지막 published community container tag |

Sources:

- PostgreSQL Official Image: <https://hub.docker.com/_/postgres>
- Redis Official Image: <https://hub.docker.com/_/redis>
- MinIO image tags: <https://hub.docker.com/r/minio/minio/tags>
- MinIO releases: <https://github.com/minio/minio/releases>

MinIO upstream repository는 2026-04-25 archived됐고 2025-10 security source release 이후 새 official prebuilt container를 제공하지 않았다. 따라서 이 tag는 loopback-bound disposable local compatibility test에만 사용한다. production image 또는 lifecycle 결정을 뜻하지 않으며 task-8 D11과 homelab security review가 별도로 materialize한다.

## Lock and verification gate

에이전트는 lock/build를 실행하지 않는다. 사용자가 [lock card](../commands/task-1/locks.md)로 다음을 증명한다.

1. `Cargo.lock`, `flake.lock` 생성
2. 두 파일 SHA-256 기록
3. locked Cargo resolution 성공
4. `path:.` Nix metadata/show evaluation 성공
5. 반복 실행 후 두 checksum 무변경

package build, current-system flake check, Linux builder matrix는 별도 card다. lockfile 생성은 staging/commit/push 승인을 포함하지 않는다.

## Consequences

- 장점: Rust value가 한 곳에 있고 development/package drift를 막는다.
- 장점: M0/C0에 필요 없는 auth/push/S3 graph와 license surface를 미룬다.
- 비용: dependency owner마다 official compatibility 확인과 user lock gate가 반복된다.
- 위험: nixos-unstable과 community tool input은 symbolic ref로 시작한다. `flake.lock`이 없으면 reproducible하다고 주장할 수 없다.
- 위험: MinIO local image는 production 적합성 근거가 아니다. loopback/local-only 경계를 깨뜨리면 이 ADR의 허용 범위를 벗어난다.
