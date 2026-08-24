# jamye-plz → jamye-server M1 이관 범위 잠금

> 상태: **COMPLETE — M1_SCOPE_LOCK passed**
> legacy Git HEAD: 19140f16b50f032d9e3d2390731604b5d39e825e
> PF1 regular-file rows: 89
> required status-entry fingerprint: 1360c3354d0bd25b8681691300e11bdabc3c5410b0f22afb00f522792e92a06f over 4 sorted NUL-terminated git status --porcelain=v1 -z entries
> auxiliary expanded-untracked fingerprint: 02986643241567c52f05bfef41778c19da2b711c97b487797abc5be74715bf08 over 5 sorted NUL-terminated git status --porcelain=v1 -z --untracked-files=all entries
> source policy: 이 문서는 roadmap이나 과거 review를 증거로 사용하지 않는다. 원본 prompt의 지정 span과 legacy source/test/migration을 직접 읽은 M1 기준선이다.

이 문서는 Rust 코드를 Python으로 줄 단위 번역하기 위한 목록이 아니다. 검증된 제품 의미를 보존하고, 승인된 변경과 non-goal을 분리하며, 모든 legacy behavior와 target contract에 owner와 검증 지점을 부여한다. /Users/poby/Developer/jamye-plz에는 어떤 변경도 하지 않았다.

## 1. 기준선과 재현 규칙

- server prompt는 docs/greenfield/jamye-server-initial-prompt.md 8–304행만, app prompt는 docs/greenfield/jamye-app-initial-prompt.md 1–319행만 요구사항 근거로 사용한다. 각 full-file SHA는 drift 보조 증거다.
- 고정 7개 문서와 backend/app/**/*.py, backend/alembic/versions/*.py, backend/tests/**/*.py의 regular file을 bytewise path 정렬한다. __pycache__, .pytest_cache, .pyc, .pyo는 제외한다.
- 각 파일 SHA-256과 legacy HEAD를 비교한다. status fingerprint는 NUL entry를 bytewise 정렬하고 각 entry 뒤 NUL을 유지한 blob의 SHA-256이다.
- 현재 legacy dirty state는 사용자 소유다. 문서 생성 중 숨기거나 수정하거나 rebaseline하지 않았다.

### 1.1 동결 당시 status entry

~~~text
 M docs/README.md
?? .serena/memories/architecture/greenfield-native-recommendation-20260822.md
?? .serena/memories/trust-registry-cache.md
?? docs/greenfield/jamye-app-initial-prompt.md
?? docs/greenfield/jamye-server-initial-prompt.md
~~~

### 1.2 PF1 file manifest

~~~tsv
sha256	kind	canonical_absolute_path
c234f745f790b7752c48506bb8964df95178448a46ae329d0d8a9f0732fc842b	migration	/Users/poby/Developer/jamye-plz/backend/alembic/versions/0a5d7bbeb961_initial_schema.py
249a22d9d13eb806a495de58e3ba8c29a056fea75c7d888caf2f730a7c8c222b	migration	/Users/poby/Developer/jamye-plz/backend/alembic/versions/a1b2c3d4e5f6_add_chatroom_reads_and_notification_dedup.py
1b26cd1b2a8cfbc69bd171c58042d05aaa1054bd069910b7c149bbd53bb954ec	migration	/Users/poby/Developer/jamye-plz/backend/alembic/versions/c3d4e5f6a7b8_add_group_deleted_at.py
f5be0de8b0be05c645b290ed1d69005b6e5616d47a7290065fab1277d9f33077	migration	/Users/poby/Developer/jamye-plz/backend/alembic/versions/d4e5f6a7b8c9_add_message_media.py
b7e92f741c72f1f72fbc0cb045e6213adcc939f6797c70f063012a07808ddfb4	migration	/Users/poby/Developer/jamye-plz/backend/alembic/versions/e5f6a7b8c9d0_add_message_media_position.py
52fa3bd3d1223761448470d2bfea4d1e984e08b5a4afb3c074d9e59ee4aa88f8	migration	/Users/poby/Developer/jamye-plz/backend/alembic/versions/f6a7b8c9d0e1_add_transcript_and_filename.py
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/__init__.py
9d1ab628083d4b2ef8946a04dac9ad8d707ac66618f958e2b93f83dcc160348a	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/config.py
4ef14583481aed55edf23ad2c54b17abd7227aaa8bf00d11b1b90f7618ad4ba3	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/deps.py
cf5656966373a70a857f101069998806c725d305e3876db34e36d64e12c79565	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/exceptions.py
ee408603ca3d1ba09eb2a5aad5e9514e456d6d425a5ca487cafc7b2035e4e3f9	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/push_endpoint.py
d6235bf4d01ac62cde63caebaae3035d3d80364a912656a7cbeef2b9e001cfca	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/queue.py
5a00a34eb38f3183699614d82604ae31b6137cdaa35109d27fb292e2e40db7ad	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/security.py
26b52c96a51c8c3b9fa0fb6d22e0b09d980232722311b4dfabf705f0810589d5	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/storage.py
3e6c93d855e0271628dadb18f7978bee17306359ec3e0143fc97b2852d4bb2af	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/timeutil.py
3f1f65876a4b4e732f8eb94d54c9b0c268f20aaa3f0229f497ff488894c9eaff	app_source	/Users/poby/Developer/jamye-plz/backend/app/core/ws_hub.py
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855	app_source	/Users/poby/Developer/jamye-plz/backend/app/db/__init__.py
a426b45c692685caa136546aec9e896e29c72e475eea69ac32c54bc790eeb2ba	app_source	/Users/poby/Developer/jamye-plz/backend/app/db/session.py
c597b3c186f9d0a6ebc3b3b6a9a3214fe83f01776c9674c9bb35daaa6234a520	app_source	/Users/poby/Developer/jamye-plz/backend/app/main.py
8abb9b42b7ce45eb95ca39081c98ba733b90fc7fb6907c29086bacc9ce169ab1	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/__init__.py
275832b2e31ba1b6be19e9daacfa02eefb40e9e1a4bdbda5cadba2793266f50a	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/base.py
a2d3d29516cc0457a57682d2730af37611424029177a4422ae65e4fce671f265	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/chatroom.py
4db0abe98d58cea631550f967fcd02836430b31c97fd3947e59461e0ede1a17b	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/chatroom_read.py
fe7be14c0c6105444f9cf7ee58bdcad760db71213453e92278aec68fa564bc61	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/group.py
db871ca7265b7ac5d84461ff928e166b56dd28e85677e41441842907af409c26	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/invite.py
67b5d5313fcae636236af550a2cdc10294ea71f5aff68633b830a5c801a0ac59	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/membership.py
054c890dd3a970494a1e7457f19870c55c1516fcc39cf2144e1e7a7f5947aabb	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/message.py
0321b514483b6c8fafbc55d286e1b59be0dad68916adea998898a48479a2fa21	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/message_media.py
d5f6ec8f1c76a81283f124d383ee9aad7e4c57aad0e333bd9774a578b868be06	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/notification.py
f9734d9aeb918517ec879dd1c13db173882e2ed45fa96bed6da4a0c56cf474ec	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/push_subscription.py
3195a91953f8ae43dc2cf3b513044db7de37aad5f15798c1cacc1ebbade691f5	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/topic.py
fd0eadfc23baa5a99afd36d77dcbbc98bb15a870dfb804b97e5caca02a75ad92	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/topic_media.py
26b70a171e182f3149a52a0b4196cb08ea10b0fc6c6c175059e9f7e0c28e4c86	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/topic_tag.py
311f73e0ad4843e7db4360860bd5addd6aa18d759bb0ac1d7d87e5a249600a1a	app_source	/Users/poby/Developer/jamye-plz/backend/app/models/user.py
39610aa5cbead49b13329bcf9786d3531d19d18dcbf5ff9b7101cac1be5f593b	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/__init__.py
b6a3625d802d46ceccf47301a3605af72d2323beff40a27fbec65387edc19dd6	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/chatroom_read_repository.py
658936b9fab3d7eff5d0488b916c1375479f1a0c3a823479b6c265b3f0b592e9	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/group_repository.py
fa57226bdc1dfa79eb57145c14ad675c041b10a3c6763e28edf18a76f8ddeb82	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/invite_repository.py
390e5014bd3c0bcc42335fbfd51e94f94cb9f362d39f7dd41bee5725b8724fbb	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/message_media_repository.py
cbd0e5cb4a4a3d7f9008b0ccd06cb7519f68b4f8fc4cec409d2956305e8af9a0	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/message_repository.py
8cd66237a91fd4d2af61989e5c97a4e5070dac7f2b3eb9f6e3256de2f928c7bd	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/notification_repository.py
e7a1210ac69d65bd2b4ed4b83c645a2f2d3310d42c6a634a2fae7af8836c7996	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/topic_repository.py
c99f886ade230303ae986199da914d16f5bb7cd7715302fc0a4c6f57c9bc7381	app_source	/Users/poby/Developer/jamye-plz/backend/app/repositories/user_repository.py
9ddbef52fba394823575f4df8e36e266f25ab7ebbc57bb31d7835a28af6b28d3	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/__init__.py
31558e2cfeb09d18a8920a494c26cb84125a2f2682fdba276ea725d4d087fdb9	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/auth.py
38b72e159cbe7bbc93efc5208b42178d68817fd89024d24621a8f0cd34b2d4ae	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/chat_media.py
7233a7a1e48e5a1166cdc327d8ef28c3007754e296c95826a172a979439317c2	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/chatrooms.py
0ce71184e111cd6c171c0cd0f8e7880339c66875199c96cead5ac859b8edb8b9	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/groups.py
0005234c38f346c29327f8bf48a1b3372f6e815a00d40cc9ab99a26d43bbae52	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/invites.py
20e547ea7e4424b8548052ca78bcd91bc9ce16c85e5dd47554c7b8f1722251fc	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/me.py
eec7ef85e6c123029206e8037ce89b991cb2317c3d6abe8dfc4fa174cb16475a	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/media.py
d25dc5da5d99b6467b23d2700ed78de0c56f9982d9d528efc2db404c73e461cd	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/notifications.py
da28feb4fdff0d70fbd57f08d18a3cc5a8d6ad50bc4863da01854e29d0b86d31	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/push.py
f25bc96649949fa5f9a148965d4fb28b9b48add823494e509dabc43771801a03	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/tags.py
67d48172ec74c7eb0e0aafb31ac38045cab313005d1a1c5782985cb8e8b8fc05	app_source	/Users/poby/Developer/jamye-plz/backend/app/routers/topics.py
f51382c44ce0dc7034f6ae49ac937073e40b3b9171523229e7afb777eef2e0e5	app_source	/Users/poby/Developer/jamye-plz/backend/app/schemas/__init__.py
1c6029d90ae657c67a5a7b38646b0a52d2235ddadf491ec305600b05e3f3cb98	app_source	/Users/poby/Developer/jamye-plz/backend/app/schemas/chat.py
86bae84331304c655a09f0c30595d70b4e24acaa44a11f09e3b4042a2647bba6	app_source	/Users/poby/Developer/jamye-plz/backend/app/schemas/group.py
c4c15c92fd9dfd32501220326dbb66ebe3c934b01e8a8b1d8a859f4a869607d2	app_source	/Users/poby/Developer/jamye-plz/backend/app/schemas/invite.py
5d81fc636fd069f42f0326202ff78635768c1896c183e4de5b0dc5de491766cd	app_source	/Users/poby/Developer/jamye-plz/backend/app/schemas/topic.py
1f61aa7c1221a9d18ffb48c1dbe85d0e022724f5e12235c360e6f8004c5b8776	app_source	/Users/poby/Developer/jamye-plz/backend/app/schemas/user.py
99fb311ad7596355e3a359ec183a18cc5df95d4c52b5d814c186c168a78d2861	app_source	/Users/poby/Developer/jamye-plz/backend/app/services/__init__.py
2de6a43f4d42dbd2300786a003dd98c69805478ba30676b214e46b1961805159	app_source	/Users/poby/Developer/jamye-plz/backend/app/services/auth_service.py
a42612d9e9feaec246ee1f156a129706d67f0a5624ba6532a0618994d77fc2f8	app_source	/Users/poby/Developer/jamye-plz/backend/app/services/chat_service.py
549c78573098e8eb7c231f982c1b8de36cd04ac9cc0f582079c065605f5a25b2	app_source	/Users/poby/Developer/jamye-plz/backend/app/services/group_service.py
7407154f97781ae3c665c99bb5bfe0890031ae8a604633166799c345a9bd0f3e	app_source	/Users/poby/Developer/jamye-plz/backend/app/services/invite_service.py
9966d84524e1596eb10dc88ca0e7fd97d25635e0ba0f766e467bec4937d1900c	app_source	/Users/poby/Developer/jamye-plz/backend/app/services/notification_service.py
a451ecdab185489e22a82dcab3247cdb40d656b4e8a1f92c06d0a25c6dc60c34	app_source	/Users/poby/Developer/jamye-plz/backend/app/services/push_dispatch.py
d5d28433c413e2874fe5280e34f60271ac9649f2cf2f6932564499e7147342ef	app_source	/Users/poby/Developer/jamye-plz/backend/app/services/topic_service.py
7292e6c3ad4ceee2e1c03e9a0bb38c6a1bd6d11d95d400345ec67d5d34712cc4	app_source	/Users/poby/Developer/jamye-plz/backend/app/workers/__init__.py
f50f27d90071655c34aaa397caa1ccea71b288adf0374731c2fb6709cf98b74c	app_source	/Users/poby/Developer/jamye-plz/backend/app/workers/transcribe.py
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/__init__.py
2d1548fba9fc80a50134f6a4aa77df6192181d518429623afe0bd00cba5ea695	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_chat_media.py
fd79aa409ecb93b5a524eba645d6760810fe100d91e4d054f63989c6fc5c0d34	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_chat_service.py
f9d836c53ba9cd01d448b82fa93b33ae0403a365b493af56e9b731fae1285f1a	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_group_management.py
de8d51b0a5af5ffcfaad4bc91b3991e58fca7acc1a3f2a3c2ae685aa26a76a53	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_push.py
72bfdc40eef3cb9fcf68450580b07037c410a3783f1fb41b63a804843ecf9218	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_storage.py
c418019e999c8df7e01fb17d07fb587c76b6198730d6bc525316e724e02c57dd	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_timeutil.py
6e4c7755e0672e0d84c2398db7e1188f470a21f8dbaa39610259e391d210ac06	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_topic_rename.py
d5ba4996bab13f34b535b0617c32511acc758bc0f64d5e96b8b0e62c562818d9	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_voice_messages.py
0f2dfdd3f6312c1191094cd034a7d0870d8d6e08e34ea4b11fbc70a11be1ce27	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_websocket_heartbeat.py
d3b351f6c007ee539b3490ee1d8155d328e661cfe462065720bb1caeb55af39f	legacy_test	/Users/poby/Developer/jamye-plz/backend/tests/test_ws_hub.py
0a9ab700b43f280c70805a6fc0543796be7bc029cbabbc0208179e53e18805e8	authoritative_doc	/Users/poby/Developer/jamye-plz/docs/architecture/api-contract.md
4d2011ad6aeef9c09fe78dc42758f3107d486f37b48b03522a38886565ba02fb	authoritative_doc	/Users/poby/Developer/jamye-plz/docs/architecture/data-model.md
b983059fc78c1a86c2269feab7b51f6e1fed8cf7425c8b2d9665112282fd491e	authoritative_doc	/Users/poby/Developer/jamye-plz/docs/deployment/nixos-alfheim.md
a58c2017e43f3e0fdb3bb8bff1eb5c564dd3f547e5817544d5d50da596dc46eb	prompt(requirements=lines 1-319; full SHA auxiliary)	/Users/poby/Developer/jamye-plz/docs/greenfield/jamye-app-initial-prompt.md
5c8ddb504c628a12a01c31019e11d709e1bff828706d5af1a63e43a9d6d92219	prompt(requirements=lines 8-304; full SHA auxiliary)	/Users/poby/Developer/jamye-plz/docs/greenfield/jamye-server-initial-prompt.md
ab0b4bc6ad1146cdefc4f85d02253cb4582cd13825f003478d4f2d2d5930ba46	authoritative_doc	/Users/poby/Developer/jamye-plz/docs/product/features.md
87f4d5407f531fae8d751d3e19f2340c356b0bb60dbe5f5943b83cec201ea852	authoritative_doc	/Users/poby/Developer/jamye-plz/docs/product/vision-and-scope.md
~~~

## 2. 완전성 계수

| 항목 | 기준 | 이번 문서 |
| --- | --- | --- |
| legacy app operations | 40 = 38 APIRouter + app health + WebSocket | 40 |
| legacy HTTP operations | 39 = 38 APIRouter + app health | 39 |
| legacy WebSocket operations | 1 | 1 |
| legacy ORM tables | 13 | 13 |
| legacy tests | 189 across 10 files | 189 across 10 files |
| legacy Alembic revisions | 6 ordered revisions | 6 |
| selected target REST operations | 42 | 42 |
| selected realtime variants | 2 | 2 |

FastAPI가 자동 생성하는 OpenAPI/docs/redoc route는 migration surface가 아니다.

## 3. Legacy operation → target behavior matrix

| behavior_id | legacy_evidence_file_line_test | observed_behavior | disposition_preserve_change_non_goal | owning_task | contract_operation_event_frame_or_internal_id | target_test_or_fixture | discrepancy_id | product_visible | user_approval_evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| OP-MEDIA-01 | backend/app/routers/media.py:22 | topic media PUT presign | change | task-8 | MD1 | tests/media | L09 | yes | approved_locked_from_initial_prompt |
| OP-MEDIA-02 | backend/app/routers/media.py:45 | topic media confirm and persist | change | task-8 | MD2 | tests/media | L09,L13 | yes | approved_locked_from_initial_prompt |
| OP-MEDIA-03 | backend/app/routers/media.py:69 | member lists topic media | preserve | task-8 | MD3 | tests/media | - | yes | not_required_preserve |
| OP-GROUP-01 | backend/app/routers/groups.py:20 | create closed group | preserve | task-6 | G1 | tests/groups | - | yes | not_required_preserve |
| OP-GROUP-02 | backend/app/routers/groups.py:27 | list current-user groups | preserve | task-6 | G2 | tests/groups | - | yes | not_required_preserve |
| OP-GROUP-03 | backend/app/routers/groups.py:34 | get member-visible group | preserve | task-6 | G3 | tests/groups | - | yes | not_required_preserve |
| OP-GROUP-04 | backend/app/routers/groups.py:42 | list group members | preserve | task-6 | G4 | tests/groups | - | yes | not_required_preserve |
| OP-GROUP-05 | backend/app/routers/groups.py:49 | owner renames group | preserve | task-6 | G5 | tests/groups | - | yes | not_required_preserve |
| OP-GROUP-06 | backend/app/routers/groups.py:56 | owner soft-deletes group | preserve | task-6+task-6c | G6 | tests/groups;tests/realtime_membership | - | yes | not_required_preserve |
| OP-GROUP-07 | backend/app/routers/groups.py:62 | owner removes member or member leaves | preserve | task-6+task-6c | G7 | tests/groups;tests/realtime_membership | - | yes | not_required_preserve |
| OP-GROUP-08 | backend/app/routers/groups.py:73 | owner changes role/transfers ownership | preserve | task-6 | G8 | tests/groups | - | yes | not_required_preserve |
| OP-ME-01 | backend/app/routers/me.py:12 | read current profile | preserve | task-5 | U1 | tests/profile | - | yes | not_required_preserve |
| OP-ME-02 | backend/app/routers/me.py:17 | update current profile | preserve | task-5 | U2 | tests/profile | - | yes | not_required_preserve |
| OP-CHAT-01 | backend/app/routers/chatrooms.py:23 | list group chatrooms | preserve | task-6b | C1 | tests/chatrooms | - | yes | not_required_preserve |
| OP-CHAT-02 | backend/app/routers/chatrooms.py:31 | read message history | preserve | task-6b+task-8 | C2 | tests/chatrooms;tests/media | - | yes | not_required_preserve |
| OP-CHAT-03 | backend/app/routers/chatrooms.py:48 | mark chat read | change | task-6b+task-9 | C3 | tests/chatrooms;tests/notifications | L06 | yes | approved_locked_from_initial_prompt |
| OP-NOTIFY-01 | backend/app/routers/notifications.py:31 | list notification history and unread count | preserve | task-9 | N1 | tests/notifications | L11 | yes | not_required_preserve; exact representation deferred D9 |
| OP-NOTIFY-02 | backend/app/routers/notifications.py:45 | mark owned notification read | preserve | task-9 | N2 | tests/notifications | - | yes | not_required_preserve |
| OP-CHAT-MEDIA-01 | backend/app/routers/chat_media.py:23 | chat media presign | change | task-8 | MD1 | tests/media | L09 | yes | approved_locked_from_initial_prompt |
| OP-CHAT-MEDIA-02 | backend/app/routers/chat_media.py:49 | refresh authorized media URL | preserve | task-8 | MD4 | tests/media | - | yes | not_required_preserve |
| OP-CHAT-MEDIA-03 | backend/app/routers/chat_media.py:70 | authorized media download redirect | preserve | task-8 | MD5 | tests/media | - | yes | not_required_preserve |
| OP-AUTH-01 | backend/app/routers/auth.py:85 | browser Kakao authorization start | change | task-5 | A1 | tests/auth | L02 | yes | approved mobile-token boundary; exact D12 branch pending |
| OP-AUTH-02 | backend/app/routers/auth.py:99 | browser Kakao callback/session | change | task-5 | A2 | tests/auth | L02 | yes | approved mobile-token boundary; exact D12 branch pending |
| OP-AUTH-03 | backend/app/routers/auth.py:114 | browser Google authorization start | change | task-5 | A1 | tests/auth | L02 | yes | approved mobile-token boundary; exact D12 branch pending |
| OP-AUTH-04 | backend/app/routers/auth.py:128 | browser Google callback/session | change | task-5 | A2 | tests/auth | L02 | yes | approved mobile-token boundary; exact D12 branch pending |
| OP-AUTH-05 | backend/app/routers/auth.py:143 | logout browser session | change | task-5 | A4 | tests/auth | L02 | yes | approved mobile-token boundary; D13 authority pending |
| OP-PUSH-01 | backend/app/routers/push.py:97 | serve VAPID public key | non_goal | task-9 | L07 | tests/notifications/expo_only_surface | L07 | yes | approved_locked_from_later_user_directive_D2_A |
| OP-PUSH-02 | backend/app/routers/push.py:112 | register Web Push subscription | change | task-9 | P2 | tests/notifications | L07 | yes | approved_locked_from_later_user_directive_D2_A |
| OP-PUSH-03 | backend/app/routers/push.py:124 | bodyless delete-all Web Push subscriptions | change | task-9 | P4 | tests/notifications | L08 | yes | approved_by_user_2026-08-25_option_A |
| OP-TAG-01 | backend/app/routers/tags.py:13 | author or owner replaces topic tags | preserve | task-7 | T6 | tests/topics | - | yes | not_required_preserve |
| OP-TAG-02 | backend/app/routers/tags.py:29 | member lists topic tags | preserve | task-7 | T7 | tests/topics | - | yes | not_required_preserve |
| OP-INVITE-01 | backend/app/routers/invites.py:18 | owner creates bounded invite | preserve | task-6 | I1 | tests/groups | - | yes | not_required_preserve |
| OP-INVITE-02 | backend/app/routers/invites.py:40 | authenticated user redeems invite | preserve | task-6 | I2 | tests/groups | - | yes | not_required_preserve |
| OP-TOPIC-01 | backend/app/routers/topics.py:29 | member creates seed topic | preserve | task-7+task-9 | T1 | tests/topics;tests/notifications | - | yes | not_required_preserve |
| OP-TOPIC-02 | backend/app/routers/topics.py:118 | member lists topic dates | preserve | task-7 | T2 | tests/topics | - | yes | not_required_preserve |
| OP-TOPIC-03 | backend/app/routers/topics.py:130 | member lists topics | preserve | task-7 | T3 | tests/topics | - | yes | not_required_preserve |
| OP-TOPIC-04 | backend/app/routers/topics.py:151 | member reads topic | preserve | task-7 | T4 | tests/topics | - | yes | not_required_preserve |
| OP-TOPIC-05 | backend/app/routers/topics.py:165 | author patches topic | preserve | task-7 | T5 | tests/topics | L13 | yes | not_required for legacy body; photo enrichment change separately approved |
| OP-WS-01 | backend/app/main.py:172 | cookie-authenticated WS join/send/leave/ping | change | task-4a+task-4b+task-6c | C4,S1,R1,WS | tests/messaging;tests/realtime;tests/realtime_membership | L03,L05 | yes | approved_locked_from_initial_prompt |
| OP-HEALTH-01 | backend/app/main.py:362 | single /api/health process probe | change | task-1 | H1,H2 | tests/platform | L01 | yes | approved_locked_from_initial_prompt |

## 4. Discrepancy register

| discrepancy_id | candidate | disposition | resolution/target behavior | owner | contract/internal ID | product_visible | approval evidence | target milestone |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| L01 | public wire normalization | change | /api/v1, snake_case, exact error envelope, opaque cursor pages, split liveness/readiness | task-1, task-3b, feature owners, task-12 | H1,H2,all REST | yes | approved_locked_from_initial_prompt | M0/C0/C2 |
| L02 | browser-cookie auth to mobile tokens | change | short Bearer + rotated hashed refresh; OAuth mobile flow selected later by D12 | task-5 | A1-A4,U1,U2 | yes | approved mobile boundary; D12 pending at M4 | M4 |
| L03 | WS message command to REST idempotent command | change | C4 is canonical write; WS accelerates committed events only | task-4a,task-8 | C4,message.created | yes | approved_locked_from_initial_prompt and later outbox directive | M3a/M7 |
| L04 | volatile realtime to durable outbox+delta | change | messages+conversation_events+outbox atomically; Redis loss recovered by paginated S1 | task-3a,task-4a,task-4b,task-12 | C4,S1,message.created | yes | approved_locked_from_initial_prompt and later outbox directive | M2-M3/C2 |
| L05 | cookie WS auth/join topology | change | one-time Redis ticket; subscribed causal ack; denied join terminal 4001; DB delta is correctness path | task-3b,task-4b,task-6c | R1,WS,4001,4401 | yes | approved_locked_from_initial_prompt; D13 expiry pending | C0/M3b/M5c |
| L06 | client timestamp read marker | change | server conversation cursor, monotonic same/older idempotent no-op | task-6b,task-9 | C3,chatroom_reads | yes | approved_locked_from_initial_prompt | M5b/M8 |
| L07 | Web Push/VAPID to Expo-only native push | non_goal | no VAPID endpoint, subscription key, SSRF adapter, Web Push credential or fixture; preserve durable push hint semantics via Expo | task-9,task-12,task-13 | P2-P4,N1,N2 | yes | approved_locked_from_later_user_directive_D2_A | M8/C2/M11b |
| L08 | delete-all subscription to installation-specific delete | change | selected A: P4 deletes exactly one globally identified installation owned by current user; stale owner is non-revealing missing | task-9 | P4 | yes | approved_by_user_2026-08-25_option_A | M1 locked; implement at M8 |
| L09 | prefix-trusted media to DB-backed upload intent | change | server-minted key; owner/target/MIME/size/expiry + HEAD verification; one-time bind; private short URLs | task-8 | MD1-MD5,C4 | yes | approved_locked_from_initial_prompt | M7 |
| L10 | API bucket creation/fallback boundary | change | D11 selects homelab provisioning+API HEAD-only or exact 404 create behavior; production remains fail-closed | task-8,task-13 | internal bucket lifecycle | no | pending D11; non-product operational choice does not block M1 | M7/M11b |
| L11 | notification string representation | preserve | legacy notification capability is retained; task-9 materializes user-selected D9 structured args vs server Korean strings | task-9 | N1,N2,push payload | yes | preserve at M1; D9 required before any representation change | M8 |
| L12 | account deletion release obligation | change | net-new DELETE /api/v1/me; exact sole-owner and data disposition selected by D5/D10 | task-11,task-12,task-13 | U3 | yes | approved endpoint obligation from initial prompt; D5/D10 pending | M10/C2 |
| L13 | topic photo enrichment mismatch | change | successful scope=topic photo finalize promotes seed to enriched, matching body enrichment | task-7,task-8 | T5,MD2,MD3 | yes | approved_locked_from_initial_prompt | M6/M7 |
| L14 | STT exclusion while preserving voice media | non_goal | no transcript/job/event/worker/provider/package; exactly one finalized audio remains ordinary message media through send/history/delta/playback | task-2,task-8,task-12 | C2,C4,MD1,MD2,MD4,MD5,message.created | yes | approved_locked_from_user_D3_C | M1/M7/C2 |

### L08 사용자 결정 결과

- **A — installation-specific delete (선택됨):** DELETE /api/v1/push/installations/{installation_id}는 현재 인증 사용자가 소유한 설치 하나만 제거한다. 계정 전환 뒤의 stale owner는 존재 여부를 알 수 없다.
- **B — legacy delete-all 보존:** 현재 사용자의 모든 설치를 한 command로 제거하는 별도 collection-level 계약을 C2에 추가하고, 특정 설치 logout과 전체 logout을 구분한다.

사용자가 2026-08-25에 A를 직접 선택했다. 이 승인으로 L08은 잠겼으며, B는 C2 surface에 포함하지 않는다. D11은 운영 수명주기 결정이라 M1 제품 scope-lock을 막지 않지만 task-8 전에는 선택해야 한다. D9/D12/D13/D5/D10은 각 계획상 earliest materializer의 gate를 유지한다.

## 5. Target contract reverse coverage

| target_id | method | path | source behavior or net-new authority | owner | coverage |
| --- | --- | --- | --- | --- | --- |
| H1 | GET | /health/live | OP-HEALTH-01 | task-1/M0 | mapped |
| H2 | GET | /health/ready | OP-HEALTH-01 | task-1/M0 | mapped |
| A1 | POST | /api/v1/auth/oauth/{provider}/authorize | OP-AUTH-01,OP-AUTH-03 | task-5/M4 | mapped |
| A2 | POST | /api/v1/auth/oauth/{provider}/exchange | OP-AUTH-02,OP-AUTH-04 | task-5/M4 | mapped |
| A3 | POST | /api/v1/auth/refresh | initial server prompt 8-304 + app prompt 1-319 | task-5/M4 | mapped |
| A4 | POST | /api/v1/auth/logout | OP-AUTH-05 | task-5/M4 | mapped |
| U1 | GET | /api/v1/me | OP-ME-01 | task-5/M4 | mapped |
| U2 | PATCH | /api/v1/me | OP-ME-02 | task-5/M4 | mapped |
| U3 | DELETE | /api/v1/me | initial server prompt account-delete obligation | task-11/M10 | mapped |
| G1 | POST | /api/v1/groups | OP-GROUP-01 | task-6/M5 | mapped |
| G2 | GET | /api/v1/groups | OP-GROUP-02 | task-6/M5 | mapped |
| G3 | GET | /api/v1/groups/{group_id} | OP-GROUP-03 | task-6/M5 | mapped |
| G4 | GET | /api/v1/groups/{group_id}/members | OP-GROUP-04 | task-6/M5 | mapped |
| G5 | PATCH | /api/v1/groups/{group_id} | OP-GROUP-05 | task-6/M5 | mapped |
| G6 | DELETE | /api/v1/groups/{group_id} | OP-GROUP-06 | task-6/M5+task-6c/M5c | mapped |
| G7 | DELETE | /api/v1/groups/{group_id}/members/{user_id} | OP-GROUP-07 | task-6/M5+task-6c/M5c | mapped |
| G8 | PATCH | /api/v1/groups/{group_id}/members/{user_id} | OP-GROUP-08 | task-6/M5 | mapped |
| I1 | POST | /api/v1/groups/{group_id}/invites | OP-INVITE-01 | task-6/M5 | mapped |
| I2 | POST | /api/v1/invites/{code}/join | OP-INVITE-02 | task-6/M5 | mapped |
| T1 | POST | /api/v1/groups/{group_id}/topics | OP-TOPIC-01 | task-7/M6 core+task-9/M8 full atomic | mapped |
| T2 | GET | /api/v1/groups/{group_id}/topics/dates | OP-TOPIC-02 | task-7/M6 | mapped |
| T3 | GET | /api/v1/groups/{group_id}/topics | OP-TOPIC-03 | task-7/M6 | mapped |
| T4 | GET | /api/v1/groups/{group_id}/topics/{topic_id} | OP-TOPIC-04 | task-7/M6 | mapped |
| T5 | PATCH | /api/v1/groups/{group_id}/topics/{topic_id} | OP-TOPIC-05 | task-7/M6 | mapped |
| T6 | PUT | /api/v1/groups/{group_id}/topics/{topic_id}/tags | OP-TAG-01 | task-7/M6 | mapped |
| T7 | GET | /api/v1/groups/{group_id}/topics/{topic_id}/tags | OP-TAG-02 | task-7/M6 | mapped |
| MD1 | POST | /api/v1/media/uploads | OP-MEDIA-01,OP-CHAT-MEDIA-01 | task-8/M7 | mapped |
| MD2 | POST | /api/v1/media/uploads/{upload_id}/finalize | OP-MEDIA-02 + initial prompt chat-finalize | task-8/M7 | mapped |
| MD3 | GET | /api/v1/topics/{topic_id}/media | OP-MEDIA-03 | task-8/M7 | mapped |
| C1 | GET | /api/v1/groups/{group_id}/chatrooms | OP-CHAT-01 | task-6b/M5b | mapped |
| C2 | GET | /api/v1/chatrooms/{chatroom_id}/messages | OP-CHAT-02 | task-6b/M5b text+media:[]; task-8/M7 adds ordered media | mapped |
| C3 | POST | /api/v1/chatrooms/{chatroom_id}/read | OP-CHAT-03 | task-6b/M5b core+task-9/M8 bounded clear | mapped |
| C4 | POST | /api/v1/chatrooms/{chatroom_id}/messages | OP-WS-01 + test_chat_media.py content tests | task-4a/M3a text boundary+task-8/M7 media | mapped |
| MD4 | GET | /api/v1/media/{media_id}/url | OP-CHAT-MEDIA-02 | task-8/M7 | mapped |
| MD5 | GET | /api/v1/media/{media_id}/download | OP-CHAT-MEDIA-03 | task-8/M7 | mapped |
| S1 | GET | /api/v1/conversations/{conversation_id}/events | initial server/app prompt delta-sync requirement + OP-WS-01 recovery gap | task-4a/M3a | mapped |
| R1 | POST | /api/v1/realtime/tickets | initial server/app prompt mobile realtime auth + OP-WS-01 | task-4b/M3b | mapped |
| P2 | POST | /api/v1/push/installations | OP-PUSH-02 | task-9/M8 common Expo scope | mapped |
| P3 | PUT | /api/v1/push/installations/{installation_id} | initial app prompt installation update | task-9/M8 common Expo scope | mapped |
| P4 | DELETE | /api/v1/push/installations/{installation_id} | OP-PUSH-03 | task-9/M8 | mapped |
| N1 | GET | /api/v1/notifications | OP-NOTIFY-01 | task-9/M8 | mapped |
| N2 | POST | /api/v1/notifications/{notification_id}/read | OP-NOTIFY-02 | task-9/M8 | mapped |

### 5.1 Realtime event/frame coverage

| target | source authority | owner | target fixture |
| --- | --- | --- | --- |
| message.created | OP-WS-01 chat message fan-out + initial prompt committed-event envelope | task-3b/task-4a/task-4b/task-8/task-12 | tests/contract;tests/messaging;tests/realtime |
| topic.created | OP-TOPIC-01 + initial prompt topic-chat meaning | task-7/task-9/task-12 | tests/topics;tests/notifications;tests/contract |
| subscribe/subscribed | legacy join/joined tests in test_websocket_heartbeat.py + initial prompt authorization | task-4b | tests/realtime |
| unsubscribe/unsubscribed | legacy leave/no-op behavior + explicit target protocol | task-4b | tests/realtime |
| ping/pong | test_websocket_heartbeat.py::test_ping_returns_direct_pong_without_mutating_socket_state | task-4b | tests/realtime |
| 4001 membership_required\|membership_revoked\|group_deleted | legacy rejected join and ws_hub eviction tests | task-4b/task-6c | tests/realtime;tests/realtime_membership |
| 4400 protocol_error | net-new explicit protocol guard from initial prompt contract ownership | task-4b | tests/realtime |
| 4401 realtime_auth_failed\|realtime_auth_expired | mobile ticket requirement + selected future D13 evidence | task-3b/task-4b | tests/contract;tests/realtime |
| 1011 internal_error | legacy reconnect semantics + delta-first correctness requirement | task-4b | tests/realtime |

## 6. Legacy table ownership

| legacy_table | evidence | observed meaning | target owner | disposition |
| --- | --- | --- | --- | --- |
| users | backend/app/models/user.py:22 | users + profile; auth/deletion state added | task-3a,task-5,task-11 | preserve+additive |
| groups | backend/app/models/group.py:21 | closed group, owner, soft delete | task-3a,task-6,task-11 | preserve |
| memberships | backend/app/models/membership.py:18 | group role membership | task-3a,task-6,task-6c,task-11 | preserve |
| invites | backend/app/models/invite.py:18 | bounded invite code/use state | task-6 | preserve |
| topics | backend/app/models/topic.py:21 | seed/enriched topic | task-7 | preserve+idempotency |
| topic_media | backend/app/models/topic_media.py:17 | topic photo metadata | task-7,task-8 | preserve+one-time upload binding |
| topic_tags | backend/app/models/topic_tag.py:16 | AI/user tags | task-7 | preserve |
| chatrooms | backend/app/models/chatroom.py:19 | one main room + topic rooms | task-3a,task-6,task-7 | preserve+DB uniqueness |
| messages | backend/app/models/message.py:19 | text/system message; client id | task-3a,task-4a,task-8 | preserve+DB idempotency |
| message_media | backend/app/models/message_media.py:23 | ordered media/audio attachment | task-8 | preserve; transcript fields excluded L14 |
| chatroom_reads | backend/app/models/chatroom_read.py:13 | legacy timestamp read state | task-6b | change to cursor L06 |
| notifications | backend/app/models/notification.py:17 | authoritative in-app history | task-9 | preserve+cursor/dedup |
| push_subscriptions | backend/app/models/push_subscription.py:17 | browser endpoint/key state | task-9 | non_goal L07; replaced by push_installations/intents |

Target-only authoritative state—auth_identities, refresh_sessions, conversation_events, outbox_events, push_installations, push_delivery_intents, media_uploads, object_deletion_intents—comes from the approved initial/mobile/outbox requirements and is owned by the corresponding machine-plan tasks. Redis is never authoritative for committed messages.

## 7. Alembic chain and SQLx migration mapping

| legacy revision | down_revision | legacy change | target mapping |
| --- | --- | --- | --- |
| 0a5d7bbeb961_initial_schema.py | root | users/groups/memberships/invites/topics/topic_media/topic_tags/chatrooms/messages/push_subscriptions | 0001 core plus 0003 invites/0005 topics/0006 media/0007 push; ordered semantic split |
| a1b2c3d4e5f6_add_chatroom_reads_and_notification_dedup.py | 0a5d7bbeb961 | chatroom_reads + notification dedup | 0004 chatroom_reads + 0007 notifications |
| c3d4e5f6a7b8_add_group_deleted_at.py | a1b2c3d4e5f6 | groups.deleted_at | 0001 core |
| d4e5f6a7b8c9_add_message_media.py | c3d4e5f6a7b8 | message_media | 0006 media |
| e5f6a7b8c9d0_add_message_media_position.py | d4e5f6a7b8c9 | message_media.position | 0006 media |
| f6a7b8c9d0e1_add_transcript_and_filename.py | e5f6a7b8c9d0 | filename + transcript fields | filename -> 0006 media; transcript portion non_goal L14 |

Target migration execution is forward-only and ordered 0001 through 0008. This section records semantic inputs only; it does not authorize importing production rows or running either migration chain.

## 8. Legacy test evidence index (189/189)

각 qualified test는 위 operation/discrepancy behavior_id 하나에 연결된다. 같은 test가 여러 불변식을 확인해도 primary behavior 하나만 적고, target fixture가 그 결합 조건을 재검증한다.

~~~tsv
legacy_test_file	line	qualified_test	behavior_id	target_test_or_fixture
test_chat_media.py	116	test_object_key_from_another_chatroom_is_rejected	L09	tests/media
test_chat_media.py	121	test_object_key_for_topic_namespace_is_rejected	L09	tests/media
test_chat_media.py	127	test_object_key_with_extra_path_segment_is_rejected	L09	tests/media
test_chat_media.py	133	test_object_key_with_empty_suffix_is_rejected	L09	tests/media
test_chat_media.py	138	test_object_key_prefix_confusion_is_rejected	L09	tests/media
test_chat_media.py	144	test_valid_object_key_passes	L09	tests/media
test_chat_media.py	148	test_send_message_rejects_foreign_object_key	C4	tests/messaging;tests/media
test_chat_media.py	162	test_message_with_neither_body_nor_media_is_rejected	C4	tests/messaging;tests/media
test_chat_media.py	168	test_media_only_message_is_allowed	C4	tests/messaging;tests/media
test_chat_media.py	182	test_text_only_message_still_works	C4	tests/messaging;tests/media
test_chat_media.py	194	test_more_than_max_attachments_is_rejected	C4	tests/messaging;tests/media
test_chat_media.py	204	test_max_attachments_exactly_is_allowed	C4	tests/messaging;tests/media
test_chat_media.py	216	test_duplicate_object_key_is_rejected	C4	tests/messaging;tests/media
test_chat_media.py	233	test_disallowed_mime_is_rejected	MD1	tests/media
test_chat_media.py	239	test_image_over_cap_is_rejected	MD1	tests/media
test_chat_media.py	248	test_video_over_cap_is_rejected	MD1	tests/media
test_chat_media.py	257	test_video_between_image_and_video_cap_is_allowed	MD1	tests/media
test_chat_media.py	267	test_video_cap_stays_under_cloudflare_limit	MD1	tests/media
test_chat_media.py	274	test_duration_is_dropped_for_images	MD1	tests/media
test_chat_media.py	285	test_duration_is_kept_for_video	MD1	tests/media
test_chat_media.py	299	test_presign_rejects_disallowed_mime	MD1	tests/media
test_chat_media.py	304	test_presign_rejects_oversized_image	MD1	tests/media
test_chat_media.py	309	test_presign_accepts_video_up_to_video_cap	MD1	tests/media
test_chat_media.py	314	test_presign_rejects_zero_bytes	MD1	tests/media
test_chat_media.py	350	test_download_rejects_media_from_another_chatroom	MD5	tests/media
test_chat_media.py	357	test_download_rejects_unknown_media_id	MD5	tests/media
test_chat_media.py	363	test_download_returns_a_url_for_own_chatroom	MD5	tests/media
test_chat_media.py	369	test_refresh_url_rejects_media_from_another_chatroom	MD4	tests/media
test_chat_media.py	375	test_refresh_url_returns_media_with_a_url	MD4	tests/media
test_chat_media.py	388	test_attachment_positions_follow_pick_order	C2	tests/chatrooms;tests/media
test_chat_media.py	423	test_download_filename_uses_extension_from_mime	MD5	tests/media
test_chat_media.py	430	test_download_filename_has_no_path_or_quote_characters	MD5	tests/media
test_chat_media.py	435	test_media_out_attaches_a_url_per_row	C2	tests/chatrooms;tests/media
test_chat_service.py	91	test_require_member_access_on_deleted_group_is_not_found	G3	tests/groups
test_chat_service.py	97	test_require_member_access_on_missing_group_is_not_found	G3	tests/groups
test_chat_service.py	103	test_require_member_access_missing_chatroom_is_not_found	C2	tests/chatrooms;tests/media
test_chat_service.py	112	test_require_member_access_non_member_is_forbidden	C2	tests/chatrooms;tests/media
test_chat_service.py	118	test_require_member_access_member_on_live_group_succeeds	G3	tests/groups
test_group_management.py	148	test_rename_by_non_owner_is_forbidden	G5	tests/groups
test_group_management.py	154	test_delete_by_non_owner_is_forbidden	G6	tests/groups;tests/realtime_membership
test_group_management.py	160	test_remove_member_by_non_owner_is_forbidden	G7	tests/groups;tests/realtime_membership
test_group_management.py	168	test_transfer_ownership_by_non_owner_is_forbidden	G8	tests/groups
test_group_management.py	177	test_owner_leave_is_conflict	G8	tests/groups
test_group_management.py	184	test_member_leave_succeeds	G7	tests/groups;tests/realtime_membership
test_group_management.py	195	test_transfer_ownership_swaps_owner_id_and_roles	G8	tests/groups
test_group_management.py	208	test_transfer_ownership_target_already_owner_is_conflict	G8	tests/groups
test_group_management.py	217	test_remove_non_member_is_not_found	G7	tests/groups;tests/realtime_membership
test_group_management.py	226	test_owner_self_remove_is_conflict	G8	tests/groups
test_group_management.py	233	test_owner_removes_member_succeeds	G4	tests/groups
test_group_management.py	241	test_rename_by_owner_succeeds	G5	tests/groups
test_group_management.py	249	test_delete_by_owner_succeeds	G6	tests/groups;tests/realtime_membership
test_group_management.py	260	test_soft_deleted_group_is_not_found	G6	tests/groups;tests/realtime_membership
test_group_management.py	266	test_missing_group_is_not_found	G6	tests/groups;tests/realtime_membership
test_group_management.py	272	test_mutation_on_soft_deleted_group_is_not_found	G6	tests/groups;tests/realtime_membership
test_group_management.py	281	test_set_member_role_owner_transfers_ownership	G8	tests/groups
test_group_management.py	290	test_set_member_role_member_on_owner_target_is_conflict	G8	tests/groups
test_group_management.py	296	test_set_member_role_member_on_member_is_noop	G8	tests/groups
test_group_management.py	311	test_require_membership_on_deleted_group_is_not_found	G6	tests/groups;tests/realtime_membership
test_group_management.py	317	test_require_membership_on_missing_group_is_not_found	G6	tests/groups;tests/realtime_membership
test_group_management.py	323	test_require_owner_on_deleted_group_is_not_found	G6	tests/groups;tests/realtime_membership
test_group_management.py	331	test_require_membership_still_forbidden_for_non_member_on_live_group	G4	tests/groups
test_group_management.py	342	test_leave_group_evicts_member_from_all_group_chatrooms	L05	tests/realtime;tests/realtime_membership
test_group_management.py	359	test_soft_delete_evicts_all_sockets_from_group_chatrooms	L05	tests/realtime;tests/realtime_membership
test_group_management.py	377	test_remove_member_evicts_target_from_all_group_chatrooms	L05	tests/realtime;tests/realtime_membership
test_group_management.py	394	test_eviction_failure_does_not_break_leave_group	L05	tests/realtime;tests/realtime_membership
test_push.py	128	TestSendPushEnabled.test_sends_to_all_subscriptions_with_correct_args	N1	tests/notifications
test_push.py	172	TestSendPushEnabled.test_no_subscriptions_is_a_no_op	N1	tests/notifications
test_push.py	190	TestSendPushPrunesExpired.test_410_prunes_only_that_subscription_others_still_sent	N1	tests/notifications
test_push.py	225	TestSendPushPrunesExpired.test_404_also_prunes	N1	tests/notifications
test_push.py	246	TestSendPushPrunesExpired.test_other_status_logs_and_does_not_prune	N1	tests/notifications
test_push.py	267	TestSendPushPrunesExpired.test_unexpected_exception_logs_and_continues	N1	tests/notifications
test_push.py	297	TestSendPushPrunesExpired.test_unsafe_stored_endpoint_is_pruned_not_sent	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	337	TestSendPushDisabled.test_no_webpush_calls_when_vapid_disabled	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	372	TestPayloadContract.test_payload_serializes_exactly_title_body_url	N1	tests/notifications
test_push.py	413	TestDispatchPush.test_swallows_send_push_exceptions_and_processes_all_users	N1	tests/notifications
test_push.py	433	TestDispatchPush.test_swallows_session_factory_errors	N1	tests/notifications
test_push.py	441	TestDispatchPush.test_schedule_push_dispatch_runs_task_and_cleans_up	N1	tests/notifications
test_push.py	457	TestDispatchPush.test_schedule_push_dispatch_is_a_no_op_for_empty_user_ids	N1	tests/notifications
test_push.py	467	TestDispatchPush.test_schedule_sheds_event_when_in_flight_cap_reached	N1	tests/notifications
test_push.py	481	TestDispatchPush.test_dispatch_bounds_concurrent_network_sends	N1	tests/notifications
test_push.py	515	TestEndpointSsrfValidation.test_accepts_public_https_push_endpoint	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	525	TestEndpointSsrfValidation.test_rejects_non_https	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	535	TestEndpointSsrfValidation.test_rejects_localhost	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	543	TestEndpointSsrfValidation.test_rejects_loopback_host_aliases	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	565	TestEndpointSsrfValidation.test_rejects_unicode_dot_lookalike_loopback	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	578	TestEndpointSsrfValidation.test_rejects_multicast_literals	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	590	TestEndpointSsrfValidation.test_rejects_private_ip_literal	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	600	TestEndpointSsrfValidation.test_rejects_loopback_ip_literal	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	610	TestEndpointSsrfValidation.test_rejects_numeric_alias_hosts	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	622	TestEndpointSsrfValidation.test_accepts_public_ip_literal	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	630	TestEndpointSsrfValidation.test_rejects_cgnat_shared_range	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	641	TestSubscriptionKeyValidation.test_accepts_well_formed_keys	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	649	TestSubscriptionKeyValidation.test_rejects_65_byte_blob_that_is_not_a_curve_point	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	660	TestSubscriptionKeyValidation.test_rejects_non_base64url_or_wrong_size_p256dh	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	676	TestSubscriptionKeyValidation.test_rejects_non_base64url_or_wrong_size_auth	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	692	TestVapidPublicKeyEndpoint.test_returns_key_when_fully_enabled	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	703	TestVapidPublicKeyEndpoint.test_returns_empty_when_half_configured	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	720	TestNoRedirectSession.test_post_forces_allow_redirects_false	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	735	TestNoRedirectSession.test_send_push_passes_no_redirect_session	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	769	TestSubscriptionCap.test_upsert_prunes_to_per_user_limit	P2	tests/notifications
test_push.py	797	TestExpectedUserIdBinding.test_mismatched_expected_user_id_raises_forbidden_and_touches_no_row	P2	tests/notifications
test_push.py	823	TestExpectedUserIdBinding.test_matching_expected_user_id_registers_normally	P2	tests/notifications
test_push.py	842	TestExpectedUserIdBinding.test_omitted_expected_user_id_registers_normally	P2	tests/notifications
test_push.py	862	TestExpectedUserIdBinding.test_push_subscribe_body_expected_user_id_defaults_to_none	P2	tests/notifications
test_push.py	870	TestExpectedUserIdBinding.test_push_subscribe_body_accepts_expected_user_id	P2	tests/notifications
test_push.py	886	TestSendFanOutCap.test_send_push_only_targets_capped_recent_subscriptions	N1	tests/notifications
test_push.py	929	TestResolvedAddressGuard.test_rejects_hostname_resolving_to_loopback	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	943	TestResolvedAddressGuard.test_rejects_hostname_resolving_to_private_range	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	957	TestResolvedAddressGuard.test_rejects_unresolvable_hostname	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	972	TestResolvedAddressGuard.test_accepts_hostname_resolving_to_public_address	L07	tests/notifications/expo_only_surface;tests/contract
test_push.py	986	TestResolvedAddressGuard.test_send_path_does_not_resolve	L07	tests/notifications/expo_only_surface;tests/contract
test_storage.py	83	TestPresignRealPath.test_presign_put_calls_generate_presigned_url_with_expected_params	MD1	tests/media
test_storage.py	107	TestPresignRealPath.test_presign_get_calls_generate_presigned_url_with_expected_params	MD4	tests/media
test_storage.py	130	TestPresignFallbackPath.test_presign_put_returns_deterministic_url_without_touching_boto3	L09	tests/media
test_storage.py	145	TestPresignFallbackPath.test_presign_get_returns_deterministic_url_without_touching_boto3	L09	tests/media
test_storage.py	164	TestEnsureBucket.test_creates_bucket_only_when_head_bucket_404s	L10	tests/media/bucket_lifecycle_selected_D11
test_storage.py	177	TestEnsureBucket.test_does_not_create_bucket_when_it_already_exists	L10	tests/media/bucket_lifecycle_selected_D11
test_storage.py	189	TestEnsureBucket.test_reraises_non_404_client_errors	L10	tests/media/bucket_lifecycle_selected_D11
test_storage.py	211	TestMediaPresignRequestValidation.test_accepts_allowed_mime_within_cap	MD1	tests/media
test_storage.py	216	TestMediaPresignRequestValidation.test_accepts_exact_cap_boundary	MD1	tests/media
test_storage.py	220	TestMediaPresignRequestValidation.test_rejects_disallowed_mime	MD1	tests/media
test_storage.py	224	TestMediaPresignRequestValidation.test_rejects_oversize	MD1	tests/media
test_storage.py	233	TestMediaConfirmRequestValidation.test_accepts_allowed_mime_within_cap	MD2	tests/media
test_storage.py	240	TestMediaConfirmRequestValidation.test_accepts_missing_byte_size	MD2	tests/media
test_storage.py	245	TestMediaConfirmRequestValidation.test_rejects_disallowed_mime	MD2	tests/media
test_storage.py	249	TestMediaConfirmRequestValidation.test_rejects_oversize	MD2	tests/media
test_storage.py	262	TestValidateObjectKeyForTopic.test_accepts_object_key_minted_for_this_topic	L09	tests/media
test_storage.py	268	TestValidateObjectKeyForTopic.test_rejects_object_key_belonging_to_a_different_topic	L09	tests/media
test_storage.py	274	TestValidateObjectKeyForTopic.test_rejects_path_traversal_style_key	L09	tests/media
test_storage.py	280	TestValidateObjectKeyForTopic.test_rejects_non_uuid_suffix	L09	tests/media
test_storage.py	285	TestValidateObjectKeyForTopic.test_rejects_empty_suffix	L09	tests/media
test_storage.py	295	TestProdStorageKeysFailClosed.test_raises_when_production_and_minio_keys_absent	L09	tests/media
test_storage.py	308	TestProdStorageKeysFailClosed.test_allows_production_when_minio_keys_present	L09	tests/media
test_storage.py	323	TestProdStorageKeysFailClosed.test_raises_when_production_endpoint_is_localhost	L09	tests/media
test_storage.py	337	TestProdStorageKeysFailClosed.test_raises_when_production_endpoint_is_plain_http	L09	tests/media
test_storage.py	351	TestProdStorageKeysFailClosed.test_dev_env_does_not_require_minio_keys	L09	tests/media
test_timeutil.py	9	TestSeoulDayWindow.test_midday_window_utc_boundaries	T2	tests/topics
test_timeutil.py	17	TestSeoulDayWindow.test_window_is_exactly_24h	T2	tests/topics
test_timeutil.py	21	TestSeoulDayWindow.test_month_end_rollover_does_not_crash	T2	tests/topics
test_timeutil.py	27	TestSeoulDayWindow.test_year_end_rollover	T2	tests/topics
test_timeutil.py	35	TestTodayStr.test_format_is_yyyy_mm_dd	T2	tests/topics
test_timeutil.py	47	TestTopicIsUnread.test_no_read_record_is_unread	C3	tests/chatrooms;tests/notifications
test_timeutil.py	51	TestTopicIsUnread.test_read_after_creation_no_messages_is_read	C3	tests/chatrooms;tests/notifications
test_timeutil.py	56	TestTopicIsUnread.test_new_message_after_read_is_unread	C3	tests/chatrooms;tests/notifications
test_timeutil.py	62	TestTopicIsUnread.test_message_before_read_is_read	C3	tests/chatrooms;tests/notifications
test_timeutil.py	68	TestTopicIsUnread.test_naive_datetimes_are_coerced_to_utc	C3	tests/chatrooms;tests/notifications
test_topic_rename.py	144	test_empty_title_is_rejected	T5	tests/topics
test_topic_rename.py	149	test_whitespace_only_title_is_rejected	T5	tests/topics
test_topic_rename.py	154	test_title_exceeding_256_chars_is_rejected	T5	tests/topics
test_topic_rename.py	159	test_title_exactly_256_chars_is_allowed	T5	tests/topics
test_topic_rename.py	164	test_title_is_stripped_server_side	T5	tests/topics
test_topic_rename.py	169	test_null_title_is_allowed	T5	tests/topics
test_topic_rename.py	176	test_author_can_update_title	T5	tests/topics
test_topic_rename.py	184	test_title_only_patch_does_not_set_enriched_status	L13	tests/topics;tests/media
test_topic_rename.py	194	test_body_update_still_works	L13	tests/topics;tests/media
test_topic_rename.py	203	test_title_and_body_together_sets_enriched_via_status_param	L13	tests/topics;tests/media
test_topic_rename.py	216	test_non_author_member_is_forbidden	T5	tests/topics
test_topic_rename.py	227	test_non_member_is_forbidden_by_require_membership	T5	tests/topics
test_voice_messages.py	48	test_audio_mimes_are_allowed	C4	tests/messaging;tests/media
test_voice_messages.py	56	test_audio_over_cap_is_rejected	C4	tests/messaging;tests/media
test_voice_messages.py	65	test_audio_within_cap_is_allowed	C4	tests/messaging;tests/media
test_voice_messages.py	74	test_audio_mixed_with_image_is_rejected	C4	tests/messaging;tests/media
test_voice_messages.py	88	test_two_audios_are_rejected	C4	tests/messaging;tests/media
test_voice_messages.py	105	test_single_audio_message_is_allowed_and_keeps_duration	C4	tests/messaging;tests/media
test_voice_messages.py	117	test_audio_without_byte_size_is_rejected	C4	tests/messaging;tests/media
test_voice_messages.py	124	test_image_without_byte_size_is_still_allowed	C4	tests/messaging;tests/media
test_voice_messages.py	133	test_without_redis_audio_sends_untranscribed	L14	tests/media/no_transcript_surface;tests/contract/no_stt_surface
test_voice_messages.py	159	test_with_redis_audio_is_marked_pending_and_enqueued	L14	tests/media/no_transcript_surface;tests/contract/no_stt_surface
test_voice_messages.py	185	test_images_are_never_marked_pending	L14	tests/media/no_transcript_surface;tests/contract/no_stt_surface
test_voice_messages.py	211	test_failed_enqueue_reverts_pending	L14	tests/media/no_transcript_surface;tests/contract/no_stt_surface
test_voice_messages.py	239	test_parse_transcript_event_builds_ws_frame	L14	tests/media/no_transcript_surface;tests/contract/no_stt_surface
test_voice_messages.py	254	test_parse_transcript_event_drops_malformed	L14	tests/media/no_transcript_surface;tests/contract/no_stt_surface
test_voice_messages.py	260	test_parse_transcript_event_allows_null_transcript	L14	tests/media/no_transcript_surface;tests/contract/no_stt_surface
test_voice_messages.py	274	test_filename_is_persisted_and_served	MD5	tests/media
test_voice_messages.py	287	test_download_prefers_stored_filename	MD5	tests/media
test_voice_messages.py	312	test_download_falls_back_to_synthesised_name	MD5	tests/media
test_voice_messages.py	358	test_probe_reads_real_container_duration	C4	tests/messaging;tests/media
test_voice_messages.py	364	test_probe_rejects_non_audio_bytes	C4	tests/messaging;tests/media
test_voice_messages.py	371	test_overlong_audio_exceeds_cap	C4	tests/messaging;tests/media
test_websocket_heartbeat.py	51	test_ping_returns_direct_pong_without_mutating_socket_state	L05	tests/realtime;tests/realtime_membership
test_websocket_heartbeat.py	89	test_auth_failure_is_sent_as_terminal_close_after_websocket_accept	L05	tests/realtime;tests/realtime_membership
test_websocket_heartbeat.py	111	test_join_acknowledges_only_after_socket_subscription	L05	tests/realtime;tests/realtime_membership
test_websocket_heartbeat.py	153	test_rejected_join_closes_with_terminal_eviction_code	L05	tests/realtime;tests/realtime_membership
test_ws_hub.py	42	test_evict_user_closes_and_removes_only_that_users_sockets	L05	tests/realtime;tests/realtime_membership
test_ws_hub.py	58	test_broadcast_skips_evicted_user	L05	tests/realtime;tests/realtime_membership
test_ws_hub.py	71	test_evict_user_is_noop_for_unrelated_user	L05	tests/realtime;tests/realtime_membership
test_ws_hub.py	81	test_leave_forgets_user_mapping_once_no_rooms_remain	L05	tests/realtime;tests/realtime_membership
test_ws_hub.py	90	test_evict_user_sweeps_every_room_it_is_told_about	L05	tests/realtime;tests/realtime_membership
test_ws_hub.py	102	test_evict_user_swallows_close_errors	L05	tests/realtime;tests/realtime_membership
~~~

### 8.1 파일별 재검산

| legacy test file | count |
| --- | --- |
| test_chat_media.py | 33 |
| test_chat_service.py | 5 |
| test_group_management.py | 27 |
| test_push.py | 46 |
| test_storage.py | 25 |
| test_timeutil.py | 10 |
| test_topic_rename.py | 12 |
| test_voice_messages.py | 21 |
| test_websocket_heartbeat.py | 4 |
| test_ws_hub.py | 6 |
| TOTAL | 189 |

## 9. Future production migration boundary

이 문서는 다음을 **향후 계획 입력**으로만 기록한다.

- 13개 legacy table row와 관계
- 새 auth/session, conversation event/outbox, media upload binding, Expo installation/push occurrence, account-deletion cleanup state
- legacy read timestamp/unread/dedup/topology에서 파생할 cursor·read marker·notification 상태
- private MinIO object bytes, server object key, DB metadata의 대응 관계
- L14에 따라 import 대상이 아닌 transcript/STT 상태

production import/drop/rederive 선택, dual-run authority, row/object checksum, rollback/cutover runbook, 실제 migration/deploy는 새 사용자 승인 계획이 필요하다. 이 문서는 그 결정을 내리거나 운영 시스템에 연결하지 않는다.

## 10. 완료 판정

- [x] PF1 89-file path/kind/SHA manifest와 legacy HEAD/status fingerprint 기록
- [x] 40 operation, 13 table, 189 test/10 file, 6 migration 계수 일치
- [x] 42 target REST operation과 2 realtime variant reverse coverage
- [x] L13 photo enrichment와 L14 STT exclusion 승인 증거 기록
- [x] future production migration은 input class만 기록하고 실행 결정 제외
- [x] **L08 P4 installation-specific delete 사용자 승인(A, 2026-08-25)**

L08 승인 뒤 PF1을 같은 알고리즘으로 재검증해 path/kind/SHA와 legacy HEAD/status fingerprint가 모두 동결 기준선과 일치함을 확인했다. task-2는 completed다. 이후 기준선 변화가 있으면 자동 갱신하지 않고 downstream parity 작업을 중단해 사용자에게 알린다.
