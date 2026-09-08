# 모바일 OAuth callback bridge

로그인 제공자에 등록하는 redirect URI는 앱 scheme이 아니라 API의 HTTPS 경로다.

- Kakao: `https://jamye-api.ridewithmin.com/api/v1/auth/oauth/kakao/callback`
- Google: `https://jamye-api.ridewithmin.com/api/v1/auth/oauth/google/callback`

브리지는 제공자의 GET callback을 받아 해당 제공자에 고정된 앱 URI로만 `302` 응답을 반환한다.

- Kakao: `jamye://oauth/kakao`
- Google: `jamye://oauth/google`

성공하면 form 인코딩한 `code`와 `state`만 전달한다. 제공자가 오류를 반환하면 같은 `state`와 함께 `error=access_denied` 또는 `error=oauth_failed`를 전달한다. 제공자의 metadata, 설명, 오류 URL은 전달하지 않는다. 이 callback은 상태를 보관하지 않으며, 제공자를 호출하거나 OAuth 시도를 소비하거나 토큰을 발급하거나 세션을 생성하지 않는다. 앱은 받은 코드를 원래의 HTTPS redirect URI, PKCE verifier와 함께 기존 exchange endpoint에 POST해야 한다.

브리지는 알 수 없는 제공자, 잘못된 percent 인코딩, 중복 parameter, 지원하지 않는 필드, 누락되거나 서로 충돌하는 성공 또는 오류 필드, 유효하지 않은 state, 크기 제한을 넘는 입력을 거부한다. Redirect와 오류 응답 모두에 `Cache-Control: no-store`, `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`를 설정한다. Callback query 값은 로그에 남기면 안 된다.

제공자 콘솔 등록과 실제 계정의 동의는 별도로 진행해야 하는 운영 절차다. 콘솔에는 위의 HTTPS URI 두 개를 정확히 등록해야 한다. `jamye://`를 제공자 callback으로 등록하거나 앱에 client secret을 넣으면 안 된다.
