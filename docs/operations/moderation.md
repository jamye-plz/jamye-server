# 운영 런북: UGC 모더레이션 (SSH)

task-20c. 신고 처리, 메시지 삭제, 계정 정지, 웹 삭제 요청 처리, 필터 목록 관리를 프로덕션 호스트에서
SSH로 수행하는 절차다. 관리 화면이나 HTTP 관리 API는 없다. 모든 작업은 `admin` 바이너리
(`jamye-server-admin` 명령)로 하며 PostgreSQL에만 연결하고 listener를 열지 않는다.

## 1. 호스트에서 실행하기

### 1.1 기본: `jamye-server-admin`

NixOS module(`services.jamye-server`)이 `jamye-server-admin` 명령을 호스트에 설치한다. root 권한이
필요하므로 `sudo`로 실행한다.

```bash
sudo jamye-server-admin --help
sudo jamye-server-admin reports list
```

이 명령은 내부에서 `systemd-run`으로 다음을 맞춘다. API 프로세스와 같은 환경이다.

- 서비스 계정 `jamye-server`(그룹 `jamye-server`)로 실행한다. PostgreSQL은
  `postgresql://jamye-server@localhost/jamye-server?host=/run/postgresql`(unix socket, peer 인증)이라
  OS 계정이 서비스 계정이어야 접속된다.
- `services.jamye-server.environmentFile`(배포가 소유한 EnvironmentFile, 보통 sops 템플릿 경로)을
  systemd가 root 권한으로 읽어 주입한다. 파일을 직접 열거나 `sudo -u jamye-server`로 읽지 않는다.
  module이 정하는 `JAMYE_ENVIRONMENT=production`, `DATABASE_URL` 등은 API 시작 스크립트와 같은
  값으로 export된다.
- 명령줄은 시작 전에 호스트 journal에 한 줄로 남는다(`authpriv`, 태그 `jamye-server-admin`,
  `operator=<sudo한 사용자> arguments: ...`).

EnvironmentFile 경로 확인(값은 출력되지 않는다):

```bash
systemctl show jamye-server-api -p EnvironmentFiles
```

`jamye-server-admin: command not found`이면 호스트가 아직 이 버전의 module로 배포되지 않은 것이다.
§8의 homelab 배선을 먼저 끝낸다.

### 1.2 래퍼가 없을 때

호스트에 아직 이 버전의 module이 배포되지 않았다면 래퍼가 없다. `sudo -u jamye-server ...`로 바이너리를
직접 실행하는 것은 동작하지 않는다(root 전용 EnvironmentFile을 읽지 못해 `DATABASE_URL`이 비어 실패).
module 배포(§8)를 먼저 끝낸다. 래퍼가 하는 일은 `cat "$(command -v jamye-server-admin)"`로 읽을 수 있다.

### 1.3 출력, 종료 코드, 감사 로그

| 종료 코드 | 의미 |
|---|---|
| 0 | 성공(이미 처리된 상태를 다시 요청한 경우 포함) |
| 1 | 운영 실패: 없는 id, 이미 다른 상태로 처리된 신고, 계정 소유 그룹, DB 오류, 설정 오류 |
| 2 | 사용법 오류: 알 수 없는 명령, 잘못된 id/옵션, 범위를 벗어난 값 |

- 사람이 읽는 결과는 stdout, 오류와 구조화 로그는 stderr에 나온다. `--json`은 stdout에 JSON만 쓴다.
- **감사 기록은 구조화 로그 한 줄이다**(감사 table은 없다). 변경 명령마다 JSON 한 줄이 stderr에
  나오고 같은 줄이 syslog(journal, 태그 `jamye-server-admin`)로도 전달된다.
  `event_kind`: `account_suspended`, `account_unsuspended`, `admin_report_handled`,
  `admin_message_deleted`, `admin_reports_purged`, `admin_account_deletion_started`.
  로그에는 id와 시각만 있고 이름, 메시지 본문, 정지 사유 문장은 없다. syslog 전달은 best effort라
  소켓이 막혀 있어도 명령은 성공하므로, 중요한 처리는 stderr 출력도 운영 기록에 붙여 둔다.

```bash
journalctl -t jamye-server-admin --since "-7 days"
journalctl -t jamye-server-admin -o cat | jq -R 'fromjson? | select(.event_kind == "admin_message_deleted")'
```

- **셸 history 주의.** `--reason` 문장과 id는 셸 history와 위 journal의 `arguments:` 줄에 남는다.
  사유에는 개인정보(이름, 연락처, 메시지 인용)를 쓰지 않고 짧은 사실만 쓴다. 예: `repeated harassment, see report <report_id>`.
  본문이 필요하면 `reports show`로 다시 읽으면 된다.
- 신고 본문 snapshot은 사용자가 쓴 글이다. 출력에서 제어 문자와 방향 전환 문자는 `\u{001b}`처럼
  이스케이프되고 줄바꿈은 `\n`으로 보인다.

## 2. 명령

id는 모두 UUID다. 아래 `<...>`는 자리표시자다. `sudo jamye-server-admin`을 줄여 `admin`으로 쓴다.

### 2.1 신고

```bash
admin reports list                              # open 신고, 오래된 순, 기본 50건
admin reports list --status all --limit 200
admin reports list --status dismissed --json
admin reports show <report_id>                  # snapshot 전체
admin reports show <report_id> --json
admin reports dismiss <report_id>               # 조치 없이 종료
admin reports resolve <report_id>               # 조치 후 종료(status actioned)
```

- `--status`는 `open`(기본), `actioned`, `dismissed`, `all`이다. `--limit`은 1부터 200까지다.
- 목록은 신고 id, 상태, 사유, 접수 시각(RFC 3339, UTC), 대상(메시지 id 또는 사용자 id)과 그룹 id,
  신고자 id, 메시지 신고면 snapshot 본문(200자까지)을 보인다. 이메일, 토큰, 닉네임은 나오지 않는다.
- `dismiss`와 `resolve`는 open인 신고만 바꾼다. 같은 처리를 반복하면 `no change`로 종료 코드 0,
  다른 최종 상태(dismissed를 resolve, actioned를 dismiss)로 바꾸려 하면 종료 코드 1이다.
- 사용자 신고(`target user`)는 자동으로 닫히지 않는다. 정지 등으로 조치한 뒤 `resolve`로 닫는다.

### 2.2 메시지 삭제

```bash
admin messages delete <message_id>
```

운영자 삭제는 작성자 삭제와 같은 의미다: 소프트 삭제와 본문 비우기, 이전 `message.created` 이벤트와
outbox의 본문 제거, `message.deleted` 이벤트(클라이언트 sync와 실시간이 보통 삭제로 본다), 첨부 파일
정리 대기열 등록이 한 transaction으로 일어난다. 그 메시지를 대상으로 하는 **open 신고는 모두
actioned**로 바뀐다(이미 dismissed인 신고는 그대로). 같은 명령을 반복해도 안전하다(이미 삭제됨,
종료 코드 0).

- `message.deleted` 이벤트의 `reason`은 `moderator_deleted`, `deleted_by`는 계정이 없으므로 nil UUID
  (`00000000-0000-0000-0000-000000000000`)다.
- 없는 id, 그룹/주제/대화방이 삭제된 메시지, 시스템 메시지는 종료 코드 1이며 아무것도 바꾸지 않는다.

### 2.3 계정 정지

```bash
admin users suspend <user_id> --reason "<short factual reason>"
admin users unsuspend <user_id>
```

- 정지는 모든 refresh 세션을 폐지하고 이후 인증 요청을 `403 account_suspended`로 막는다.
  `--reason`은 1-500자, 제어 문자 없음이다. 반복 실행해도 첫 정지 시각이 유지된다.
- 해제는 정지만 지운다. 폐지된 세션은 되살아나지 않으므로 사용자는 다시 로그인해야 한다.
- 없는 id나 이미 삭제된 계정은 종료 코드 1이다.
- **이미 열린 WebSocket은 정지해도 끊기지 않는다.** access token 만료(기본 900초)까지 이어질 수 있고
  재연결과 새 ticket 발급은 403이 된다.
- **정지된 사용자는 앱에서 계정을 삭제할 수 없다**(인증 요청이 403). 삭제를 원하면 §5의 웹 삭제 요청과
  `accounts delete`로 처리한다.

### 2.4 계정 삭제 시작

```bash
admin accounts delete <user_id>
```

앱의 계정 삭제와 같은 30일 복구 기간 삭제를 시작한다(`JAMYE_ACCOUNT_PURGE_GRACE_DAYS`, 기본 30일).
멤버십 제거, 세션 폐지, 푸시 설치 비활성화, 로그인 연결 소프트 삭제가 한 transaction이고, 복구 기간이
끝나면 worker가 purge한다. 본인 확인은 명령 전에 운영자가 끝내야 한다(§5). 소유한 그룹이 있는 계정은
종료 코드 1이며 소유권을 먼저 넘겨야 한다(CLI는 소유권을 옮기지 못한다). 이미 삭제가 시작된 계정도
종료 코드 1이다.

**Apple 제한.** Sign in with Apple 계정은 앱에서 지울 때 사용자의 재인증으로 Apple 권한을 철회한다.
운영자는 그 재인증을 받을 수 없으므로 이 명령은 **Apple 권한(token)을 철회하지 못한다.** 해당 계정이면
명령이 `warning: ... NOT revoked`를 출력한다. 사용자에게 iOS 설정 > Apple ID > 로그인 및 보안 >
Apple로 로그인에서 앱을 직접 삭제하도록 안내하고 처리 답변에 적는다.

### 2.5 처리 완료 신고 보존 기간

```bash
admin reports purge                          # 처리 후 365일이 지난 신고 삭제(기본)
admin reports purge --older-than-days 365
```

개인정보 처리 방침의 "처리 완료 신고는 1년 뒤 삭제" 약속을 지키는 명령이다. handled_at이 기준보다
오래된 `actioned`/`dismissed` 신고와 그 신고의 운영자 알림 푸시 행을 한 transaction으로 지우고 건수를
출력한다. open 신고는 오래되어도 지우지 않는다. 일수는 1부터 3650까지(0은 거부)이며 반복 실행해도
안전하다.

1년 절차: **매월 첫 영업일에 한 번** 실행한다(자동 스케줄러는 없다). 실행 날짜와 출력 건수를 운영
기록에 남긴다. 간격이 길어지면 보존 기간이 약속보다 길어지므로 빠뜨리지 않는다.

## 3. 24시간 처리 절차

신고는 접수 후 24시간 안에 확인하고 조치한다(스토어 UGC 요구). 운영자 계정(`JAMYE_OPERATOR_ACCOUNT_IDS`)
기기로는 "새 신고" 알림이 가지만 알림은 보조 수단이다. 알림이 없어도 하루 두 번(예: 오전, 저녁) 큐를
확인한다.

1. `admin reports list`로 open 신고를 오래된 순으로 본다. 접수 시각이 24시간에 가까운 것부터 처리한다.
2. `admin reports show <report_id>`로 snapshot(사유, 본문, 첨부 종류)을 읽는다. 신고 사유와 내용이 맞는지
   판단한다. 대상 사용자 신고(snapshot 없음)는 같은 사용자를 가리키는 다른 신고도 함께 본다.
3. 조치를 정한다.
   - 위반 메시지: `admin messages delete <message_id>`. 연결된 open 신고가 자동으로 actioned가 된다.
   - 반복 또는 심한 위반: `admin users suspend <user_id> --reason "..."` 후 `admin reports resolve <report_id>`.
   - 위반 아님: `admin reports dismiss <report_id>`.
4. 조치 후 `admin reports list`로 큐에서 빠졌는지 확인한다. 같은 사용자에 대한 신고가 쌓여 있으면
   중복을 같은 방식으로 닫는다.
5. 불법 콘텐츠(`illegal`)나 아동 관련 의심은 삭제와 정지로 끝내지 말고 보존과 신고 의무를 법률
   자문 기준으로 따로 판단한다. snapshot은 삭제 후에도 보존기간 동안 남는다.
6. 판단과 조치는 journal(`journalctl -t jamye-server-admin`)에 남는다. 사유 설명이 더 필요하면 별도
   운영 기록에 신고 id로 적는다.

## 4. 필터 목록

채팅 본문은 저장 전에 목록의 용어를 `***`로 마스킹한다(거부하지 않는다). 이미 저장된 메시지는 다시
마스킹되지 않는다.

- **기본 목록 위치**: 소스 `src/domain/moderation/default_terms.txt`(빌드에 포함, 변경은 재배포).
  한 줄에 용어 하나, `#` 주석과 빈 줄 무시, 대소문자 무시 부분 문자열 일치.
- **재배포 없이 바꾸기**: EnvironmentFile에 `JAMYE_CONTENT_FILTER_TERMS_FILE=<절대 경로>`를 두고 그
  경로에 같은 형식의 목록 파일을 둔다. 지정하면 **기본 목록을 완전히 대체한다**(합쳐지지 않는다).
  파일은 서비스 계정(`jamye-server`)이 읽을 수 있어야 한다.
- 검사 규칙: 용어 2자 이상 64자 이하, 최대 500개, 제어 문자 없음, 파일 256 KiB 이하, 용어 1개 이상.
  어긋나면 시작 오류가 키 이름(값은 아님)과 함께 나온다. 1글자 용어는 그 글자가 모든 메시지에서
  가려지므로 거부된다.
- **적용은 재시작으로 한다.** 파일은 프로세스 시작 시에만 읽는다.

```bash
# 1) 파일을 배치한 뒤 같은 환경으로 검증한다(admin도 같은 설정을 읽어 오류가 있으면 종료 코드 1).
sudo jamye-server-admin reports list --limit 1
# 2) 검증이 통과하면 API를 재시작한다. 필터를 쓰는 것은 API뿐이다.
sudo systemctl restart jamye-server-api
```

잘못된 파일로 재시작하면 API가 시작하지 못하므로 반드시 1)을 먼저 한다. 기본 목록 내용은 사용자의
정책 승인 task에서 검토한다(목록과 알려진 오탐은 `docs/adr/0013-ugc-moderation.md`).

## 5. 웹 삭제 요청 처리

앱을 쓸 수 없는 사용자는 웹 삭제 안내 페이지(`/account-deletion`)의 안내대로 메일로 요청한다. 요청에는
로그인 방식, 앱 닉네임, 가입한 그룹 이름 하나 이상이 있다. 접수 후 **7일 이내**에 상태를 답변한다.

1. **본인 확인.** 요청의 닉네임과 그룹 이름이 계정 기록과 맞는지 확인한다. 아래 조회로 후보를 찾는다.
   이메일이나 token은 조회하지 않는다. 필요하면 요청자에게 추가로 묻는다. 맞지 않으면 삭제하지 않는다.

닉네임은 요청자가 보낸 신뢰할 수 없는 문자열이다. SQL에 직접 붙여 넣지 말고 아래처럼 psql 변수로 넘긴다.
`:'nick'`은 psql이 문자열 리터럴로 인용하므로 따옴표나 역슬래시가 들어 있어도 SQL이 바뀌지 않는다.

```bash
read -r -p '요청에 적힌 닉네임: ' NICK
sudo -u jamye-server psql -d jamye-server -v nick="$NICK" <<'SQL'
SELECT u.id AS user_id, u.nickname, identity.provider, g.name AS group_name
FROM users u
JOIN auth_identities identity ON identity.user_id = u.id AND identity.deleted_at IS NULL
JOIN memberships m ON m.user_id = u.id AND m.deleted_at IS NULL
JOIN groups g ON g.id = m.group_id AND g.deleted_at IS NULL
WHERE u.deleted_at IS NULL AND u.nickname = :'nick'
ORDER BY u.id, g.name;
SQL
```

   `nickname`은 중복될 수 있다. 닉네임, 로그인 방식(`provider`), 그룹 이름이 **모두** 요청과 하나의
   `user_id`에서 맞을 때만 진행한다. 후보가 둘 이상이거나 불확실하면 진행하지 않는다.
2. **소유 그룹 확인.** 그 계정이 그룹 소유자이면 먼저 소유권을 넘기도록 안내한다(삭제 시 종료 코드 1).
3. `admin accounts delete <user_id>`를 실행한다. 출력의 `deletion started`를 확인한다.
4. `provider`가 `apple`이면 Apple 권한이 철회되지 않았음을 답변에 적고 §2.4처럼 직접 삭제를 안내한다.
5. 요청자에게 30일 복구 기간(그 안에 로그인하면 복구)과 purge 시점을 답변한다. 처리 사실은 journal의
   `admin_account_deletion_started`로 남는다.
6. 처리 기록에는 처리 날짜, 처리한 `user_id`, 답변 날짜만 남기고 닉네임, 그룹 이름 같은 확인 자료는 기록에
   필요한 만큼만 보관한다.

## 6. 문제 해결

| 증상 | 원인과 조치 |
|---|---|
| `invalid configuration for DATABASE_URL: is required` | 환경 없이 실행됨. `sudo jamye-server-admin`으로 실행했는지, `systemctl show jamye-server-api -p EnvironmentFiles`가 비어 있지 않은지 확인 |
| `invalid configuration for JAMYE_CONTENT_FILTER_TERMS_FILE: ...` | 필터 파일 문제(§4). API도 같은 이유로 시작하지 못한다 |
| `the database operation failed; see the structured log lines` | stderr의 `operation` 필드 또는 `journalctl -u postgresql`을 본다. 변경은 transaction이라 부분 적용되지 않는다 |
| `permission denied` (PostgreSQL) | 서비스 계정이 아닌 계정으로 실행됨. 래퍼를 쓴다 |
| 종료 코드 2 | 사용법. `sudo jamye-server-admin --help` |

## 7. 인수인계 메모 (클라이언트와 계약)

- 운영자 메시지 삭제는 기존 `message.deleted` 이벤트(계약 v1)를 그대로 쓴다. `reason` 값
  `moderator_deleted`(기존 `author_deleted` 외)와 nil UUID `deleted_by`가 새로 나올 수 있다. 계약의
  `reason`은 자유 문자열이라 스키마는 바뀌지 않았지만 앱은 두 값 모두 일반 삭제로 처리해야 한다.

## 8. homelab 배선 (이 저장소 밖, 별도 작업)

jamye-server의 module은 `jamye-server-admin`을 `environment.systemPackages`에 넣는다. homelab에서 필요한 것:

1. homelab의 `jamye-server` flake input을 이 변경이 들어간 커밋으로 갱신하고(`nix flake update jamye-server`)
   호스트를 재배포한다. 별도 module 옵션 추가는 없다.
2. 운영자가 SSH로 접속해 `sudo`를 쓸 수 있어야 한다(`wheel` 또는 해당 명령 한정 sudoers 규칙).
   규칙을 좁히려면 `jamye-server-admin`(래퍼)만 허용한다.
3. EnvironmentFile에 `JAMYE_OPERATOR_ACCOUNT_IDS`(신고 알림 수신 운영자 계정 id, 쉼표 구분 10개 이하)를
   넣는다. 선택으로 `JAMYE_CONTENT_FILTER_TERMS_FILE`과 그 파일 배치(서비스 계정이 읽기 가능)를 둔다.
4. 배포 후 호스트에서 `sudo jamye-server-admin reports list`가 `no reports` 또는 목록을 출력하는지 확인한다.
   이 확인과 같은 내용이 module의 NixOS 스모크 테스트(Linux builder)에도 들어 있다.
5. `reports purge`를 월 1회 실행하는 일정(알림 또는 systemd timer)은 homelab 쪽 운영 정책이다. timer로
   만들 때는 `jamye-server-admin reports purge`를 root 서비스로 실행하면 된다.
