# M0 security card

## 목적

locked Cargo graph의 advisory/license/source 정책과 gitleaks working-directory scan 자체를 검증한다.

## 명령

선행 조건: lock card가 통과했고 `.env.local`은 아직 존재하지 않아야 한다.

```bash
nix develop path:. --command just task task-1 dependency-check
nix develop path:. --command just task task-1 secret-scan
```

## 부작용

dependency check는 advisory database/cache를 읽거나 갱신할 수 있다. secret scan은 repository 안에 guarded untracked sentinel을 잠시 만들고 trap으로 제거한다. 실제 credential을 생성·읽기·기록하지 않고 Git history도 scan하지 않는다. 기본 gitleaks rule 전체를 유지하되 `.gitleaks.toml`은 재생성 가능한 compiler/linker output인 `target/`만 경로 제외한다. 세 개의 known generic-key false positive는 rule ID + exact path + exact line shape가 모두 일치할 때만 허용한다. `.env.local`, ignored/untracked 파일, 그 밖의 source, 문서와 agent state는 제외하지 않는다.

## 성공 기준

- `cargo deny check`가 explicit license allowlist, advisory, ban, source 정책을 통과한다.
- 첫 configured `gitleaks dir . --no-banner --redact`가 `target/` 밖의 전체 working directory에서 clean이다.
- runtime에 non-matching fragment를 합쳐 만든 synthetic sentinel을 같은 full-directory scan이 탐지한다.
- sentinel 제거 뒤 같은 scan이 다시 clean이다.

## 복구

`.env.local` guard가 실패하면 file을 scan에서 숨기지 않는다. 필요하다면 local infra를 안전하게 내리고 사용자가 credential rotation 의도로 파일을 별도 제거한 뒤 self-test를 실행한다. 첫 scan이 실패하면 실제 finding을 수정하고 card를 처음부터 다시 실행한다. sentinel 탐지가 실패하면 scanner/config regression으로 처리한다. script가 중단되면 trap이 sentinel을 지우며, 경로가 남아 있다면 내용 확인 없이 `.gitleaks-task-1-sentinel.txt` 하나만 제거한 뒤 다시 scan한다.
