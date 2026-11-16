use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use pkcs8::{DecodePrivateKey as _, EncodePrivateKey as _, LineEnding, PrivateKeyInfoOwned};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    assertion_claims::AssertionClaims,
    config::BoxConfig,
    errors::{BoxClientError, BoxClientResult},
    retry::{
        AttemptError, Idempotency, RetryPolicy, classify_error_response, classify_transport_error,
        run_with_retry,
    },
};

/// BOXのトークンエンドポイントURL
pub(crate) const BOX_TOKEN_URL: &str = "https://api.box.com/oauth2/token";

/// JWTベアラーによる許可タイプ
const JWT_BEARER_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

/// エンタープライズのサービスアカウントとして認証するためのサブタイプ
const BOX_SUB_TYPE_ENTERPRISE: &str = "enterprise";

/// JWTアサーションの有効期間（秒）
///
/// BOXはJWTアサーションの有効期限を発行から60秒以内とするよう求めるため、時刻のずれを考慮して60秒より短くする。
/// JWTアサーションはアクセストークンとの交換にだけ使うため、この値はアクセストークンの有効期間に影響しない。
const ASSERTION_LIFETIME_SECS: u64 = 45;

/// トークンエンドポイントが発行したアクセストークン
///
/// アクセストークンはJWTではなく、中身を解釈できない文字列である。
pub(crate) struct AccessToken {
    /// アクセストークン
    token: SecretString,

    /// 有効期限
    expires_at: Instant,
}

impl AccessToken {
    /// アクセストークンの文字列を返す。
    pub(crate) fn secret(&self) -> &str {
        self.token.expose_secret()
    }
}

/// アクセストークンと交換するために、BOXのトークンエンドポイントに送信するJWTアサーションを作成する署名者
///
/// 秘密鍵の復号は計算コストが高いため、作成時に1回だけ復号し、試行ごとに署名だけを行う。
struct AssertionSigner {
    /// クライアントID
    client_id: String,

    /// エンタープライズID
    enterprise_id: String,

    /// JWTアサーションの受信者
    audience: String,

    /// JWTアサーションのヘッダー
    header: Header,

    /// 署名鍵
    encoding_key: EncodingKey,
}

impl AssertionSigner {
    /// コンストラクタ
    ///
    /// # Arguments
    ///
    /// - `config`: BOX設定
    /// - `audience`: JWTアサーションの受信者。トークンエンドポイントのURLを指定する。
    ///
    /// # Errors
    ///
    /// 秘密鍵を復号できない場合、または署名鍵を作成できない場合はエラーを返す。
    fn new(config: &BoxConfig, audience: &str) -> BoxClientResult<Self> {
        let app_auth = &config.box_app_settings.app_auth;
        let private_key_info = PrivateKeyInfoOwned::from_pkcs8_encrypted_pem(
            app_auth.private_key.expose_secret(),
            app_auth.passphrase.expose_secret(),
        )?;
        let private_key_pem = private_key_info.to_pkcs8_pem(LineEnding::LF)?;
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .map_err(BoxClientError::CreateSigningKey)?;

        let mut header = Header::new(Algorithm::RS512);
        header.kid = Some(app_auth.public_key_id.clone());

        Ok(Self {
            client_id: config.box_app_settings.client_id.clone(),
            enterprise_id: config.enterprise_id.clone(),
            audience: audience.to_owned(),
            header,
            encoding_key,
        })
    }

    /// 署名したJWTアサーションを作成する。
    ///
    /// BOXは使用済みの`jti`を持つJWTアサーションを拒否するため、呼び出すたびに新しい`jti`と有効期限を設定する。
    ///
    /// # Errors
    ///
    /// システム時刻を取得できない場合、または署名できない場合はエラーを返す。
    fn sign(&self) -> BoxClientResult<String> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();
        let mut jti_buffer = Uuid::encode_buffer();
        let jti = Uuid::new_v4().hyphenated().encode_lower(&mut jti_buffer);
        let claims = AssertionClaims {
            iss: &self.client_id,
            sub: &self.enterprise_id,
            box_sub_type: BOX_SUB_TYPE_ENTERPRISE,
            aud: &self.audience,
            jti,
            exp: now + ASSERTION_LIFETIME_SECS,
        };

        Ok(jsonwebtoken::encode(
            &self.header,
            &claims,
            &self.encoding_key,
        )?)
    }
}

/// トークンリクエスト
#[derive(Serialize)]
struct TokenRequest<'a> {
    /// 許可タイプ
    grant_type: &'static str,

    /// 署名済みのJWTアサーション
    assertion: &'a str,

    /// クライアントID
    client_id: &'a str,

    /// クライアントシークレット
    client_secret: &'a str,
}

/// トークンレスポンス
#[derive(Deserialize)]
struct TokenResponse {
    /// アクセストークン
    access_token: SecretString,

    /// アクセストークンの有効期間（秒）
    expires_in: u64,
}

/// アクセストークンをキャッシュし、有効期限が近づいたら取得し直すトークン管理者
///
/// 複数のタスクから同時に呼び出されても、トークンエンドポイントへのリクエストを1つにまとめる。
pub(crate) struct TokenManager {
    /// HTTPクライアント
    http_client: reqwest::Client,

    /// トークンエンドポイントのURL
    token_url: String,

    /// クライアントシークレット
    client_secret: SecretString,

    /// JWTアサーションの署名者
    assertion_signer: AssertionSigner,

    /// 再試行設定
    retry: RetryPolicy,

    /// 有効期限のどれだけ前にアクセストークンを取得し直すか
    refresh_before_expiry: Duration,

    /// キャッシュしたアクセストークン
    ///
    /// 401を受けたときに、別のタスクが取得し直した新しいトークンを誤って破棄しないように、
    /// `Arc`のポインターでトークンを識別する。
    cache: RwLock<Option<Arc<AccessToken>>>,
}

impl TokenManager {
    /// コンストラクタ
    ///
    /// アクセストークンはまだ取得しない。
    ///
    /// # Arguments
    ///
    /// - `http_client`: HTTPクライアント
    /// - `config`: BOX設定
    /// - `token_url`: トークンエンドポイントのURL
    /// - `retry`: 再試行設定
    /// - `refresh_before_expiry`: 有効期限のどれだけ前にアクセストークンを取得し直すか
    ///
    /// # Errors
    ///
    /// 秘密鍵を復号できない場合はエラーを返す。
    pub(crate) fn new(
        http_client: reqwest::Client,
        config: &BoxConfig,
        token_url: String,
        retry: RetryPolicy,
        refresh_before_expiry: Duration,
    ) -> BoxClientResult<Self> {
        let assertion_signer = AssertionSigner::new(config, &token_url)?;

        Ok(Self {
            http_client,
            token_url,
            client_secret: config.box_app_settings.client_secret.clone(),
            assertion_signer,
            retry,
            refresh_before_expiry,
            cache: RwLock::new(None),
        })
    }

    /// 有効なアクセストークンを返す。
    ///
    /// キャッシュしたアクセストークンの有効期限が近い場合、またはキャッシュがない場合は取得し直す。
    /// 処理の途中でfutureを破棄しても、キャッシュは更新されないだけで、不整合は生じない。
    ///
    /// # Errors
    ///
    /// アクセストークンを取得できない場合はエラーを返す。
    pub(crate) async fn access_token(&self) -> BoxClientResult<Arc<AccessToken>> {
        if let Some(token) = self.fresh_cached_token(&*self.cache.read().await) {
            return Ok(token);
        }

        let mut cache = self.cache.write().await;

        // 書き込みロックを待つ間に、別のタスクが取得し直している場合は、別のタスクが取得したアクセストークンを返す。
        if let Some(token) = self.fresh_cached_token(&cache) {
            return Ok(token);
        }

        // 有効なアクセストークンがキャッシュにない場合は、アクセストークンを取得してキャッシュする。
        let token = Arc::new(self.fetch().await?);
        *cache = Some(Arc::clone(&token));

        Ok(token)
    }

    /// BOXに拒否されたアクセストークンをキャッシュから破棄する。
    ///
    /// キャッシュしているアクセストークンが`rejected`と異なる場合は、別のタスクがすでに取得し直しているため破棄しない。
    pub(crate) async fn invalidate(&self, rejected: &Arc<AccessToken>) {
        let mut cache = self.cache.write().await;
        if cache
            .as_ref()
            .is_some_and(|cached| Arc::ptr_eq(cached, rejected))
        {
            *cache = None;
        }
    }

    /// キャッシュしたアクセストークンが更新不要な場合に、そのアクセストークンを返す。
    fn fresh_cached_token(&self, cache: &Option<Arc<AccessToken>>) -> Option<Arc<AccessToken>> {
        cache
            .as_ref()
            .filter(|token| {
                !needs_refresh(token.expires_at, Instant::now(), self.refresh_before_expiry)
            })
            .map(Arc::clone)
    }

    /// 再試行設定に従って、アクセストークンを取得する。
    async fn fetch(&self) -> BoxClientResult<AccessToken> {
        run_with_retry(&self.retry, "fetch_access_token", || {
            self.request_access_token()
        })
        .await
    }

    /// トークンエンドポイントにリクエストを1回送信して、アクセストークンを取得する。
    async fn request_access_token(&self) -> Result<AccessToken, AttemptError> {
        // アクセストークンを二重に発行しても害はなく、JWTアサーションも試行ごとに作り直すため、冪等なリクエストとして再試行する。
        let idempotency = Idempotency::Idempotent;
        let assertion = self.assertion_signer.sign().map_err(AttemptError::Fatal)?;
        let request = TokenRequest {
            grant_type: JWT_BEARER_GRANT_TYPE,
            assertion: &assertion,
            client_id: &self.assertion_signer.client_id,
            client_secret: self.client_secret.expose_secret(),
        };
        let response = self
            .http_client
            .post(&self.token_url)
            .form(&request)
            .send()
            .await
            .map_err(|e| classify_transport_error(e, idempotency))?;
        if !response.status().is_success() {
            return Err(classify_error_response(response, idempotency).await);
        }

        let body: TokenResponse = response
            .json()
            .await
            .map_err(|e| classify_transport_error(e, idempotency))?;
        let now = Instant::now();
        // 有効期限が表現できないほど大きい場合は、期限切れとして扱い、アクセストークンを取得し直させる。
        let expires_at = now
            .checked_add(Duration::from_secs(body.expires_in))
            .unwrap_or(now);

        Ok(AccessToken {
            token: body.access_token,
            expires_at,
        })
    }
}

/// アクセストークンを取得し直す必要があるかどうかを返す。
///
/// 有効期限までの残り時間が`refresh_before_expiry`以下の場合は、通信中に期限が切れないように取得し直す。
fn needs_refresh(expires_at: Instant, now: Instant, refresh_before_expiry: Duration) -> bool {
    expires_at.saturating_duration_since(now) <= refresh_before_expiry
}

#[cfg(test)]
mod test_assertion_signer {
    use std::collections::HashSet;

    use jsonwebtoken::{DecodingKey, Validation};

    use super::*;
    use crate::{errors::BoxClientError, test_support::test_config};

    /// テスト用の公開鍵
    const TEST_PUBLIC_KEY: &str = include_str!("../resources/tests/public_key.pem");

    /// JWTアサーションの受信者
    const AUDIENCE: &str = "https://example.com/oauth2/token";

    /// Boxが求めるヘッダーとクレームを持つJWTアサーションに署名することを確認
    #[test]
    fn test_sign() -> anyhow::Result<()> {
        let assertion_signer = AssertionSigner::new(&test_config(), AUDIENCE)?;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        let assertion = assertion_signer.sign()?;

        let mut validation = Validation::new(Algorithm::RS512);
        validation.set_audience(&[AUDIENCE]);
        validation.set_issuer(&["client-id"]);
        let decoded = jsonwebtoken::decode::<serde_json::Value>(
            &assertion,
            &DecodingKey::from_rsa_pem(TEST_PUBLIC_KEY.as_bytes())?,
            &validation,
        )?;
        assert_eq!(decoded.header.kid.as_deref(), Some("public-key-id"));
        assert_eq!(decoded.claims["sub"], "12345");
        assert_eq!(decoded.claims["box_sub_type"], "enterprise");
        let exp = decoded.claims["exp"].as_u64().unwrap_or_default();
        // 署名の前後で秒が変わる場合があるため、1秒の幅を許容する。
        assert!(
            (now + ASSERTION_LIFETIME_SECS..=now + ASSERTION_LIFETIME_SECS + 1).contains(&exp),
            "exp: {exp}, now: {now}"
        );

        Ok(())
    }

    /// 署名するたびに異なるjtiを設定することを確認
    #[test]
    fn test_sign_unique_jti() -> anyhow::Result<()> {
        let assertion_signer = AssertionSigner::new(&test_config(), AUDIENCE)?;

        let jtis = (0..3)
            .map(|_| {
                let assertion = assertion_signer.sign()?;
                let claims: serde_json::Value =
                    jsonwebtoken::dangerous::insecure_decode(&assertion)?.claims;
                Ok(claims["jti"].as_str().unwrap_or_default().to_owned())
            })
            .collect::<anyhow::Result<HashSet<String>>>()?;

        assert_eq!(jtis.len(), 3);

        Ok(())
    }

    /// パスフレーズが誤っている場合は、秘密鍵の復号エラーを返すことを確認
    #[test]
    fn test_new_with_wrong_passphrase() {
        let mut config = test_config();
        config.box_app_settings.app_auth.passphrase = SecretString::from("wrong-passphrase");

        let result = AssertionSigner::new(&config, AUDIENCE);

        assert!(matches!(result, Err(BoxClientError::DecryptPrivateKey(_))));
    }
}

#[cfg(test)]
mod test_needs_refresh {
    use rstest::rstest;

    use super::*;

    /// 有効期限までの残り時間が`refresh_before_expiry`以下の場合のみ、取得し直すことを確認
    #[rstest]
    #[case::longer_than_refresh_before_expiry(Duration::from_secs(61), false)]
    #[case::equal_to_refresh_before_expiry(Duration::from_secs(60), true)]
    #[case::shorter_than_refresh_before_expiry(Duration::from_secs(59), true)]
    #[case::just_expired(Duration::ZERO, true)]
    fn test_needs_refresh(#[case] remaining: Duration, #[case] expected: bool) {
        let now = Instant::now();

        assert_eq!(
            needs_refresh(now + remaining, now, Duration::from_secs(60)),
            expected
        );
    }

    /// 有効期限を過ぎている場合は、パニックせずに取得し直すと判定することを確認
    #[test]
    fn test_needs_refresh_after_expiration() {
        let expires_at = Instant::now();
        let now = expires_at + Duration::from_secs(1);

        assert!(needs_refresh(expires_at, now, Duration::from_secs(60)));
    }
}

#[cfg(test)]
mod test_token_manager {
    use rstest::rstest;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    use super::*;
    use crate::{
        errors::BoxClientError,
        test_support::{TOKEN_PATH, request_count, short_retry, test_config, token_response},
    };

    /// テストで使用する`refresh_before_expiry`
    const REFRESH_BEFORE_EXPIRY: Duration = Duration::from_secs(60);

    /// モックサーバーのトークンエンドポイントに接続するトークン管理者を作成する。
    fn create_manager(
        server: &MockServer,
        timeout: Duration,
        retry: RetryPolicy,
        refresh_before_expiry: Duration,
    ) -> anyhow::Result<TokenManager> {
        let http_client = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(TokenManager::new(
            http_client,
            &test_config(),
            format!("{}{TOKEN_PATH}", server.uri()),
            retry,
            refresh_before_expiry,
        )?)
    }

    /// 最初の`n`回だけ`response`を返すモックを登録する。
    async fn mount_first_n(server: &MockServer, response: ResponseTemplate, n: u64) {
        Mock::given(method("POST"))
            .respond_with(response)
            .up_to_n_times(n)
            .mount(server)
            .await;
    }

    /// 常に`response`を返すモックを登録する。
    async fn mount_always(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .respond_with(response)
            .mount(server)
            .await;
    }

    /// JWTベアラーでトークンをリクエストし、取得したアクセストークンをキャッシュすることを確認
    #[tokio::test]
    async fn test_access_token_fetches_and_caches() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(&server, token_response()).await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let first = manager.access_token().await?;
        let second = manager.access_token().await?;

        assert_eq!(first.secret(), "access-token");
        assert!(first.expires_at > Instant::now() + Duration::from_secs(3500));
        assert!(Arc::ptr_eq(&first, &second));
        let requests = server.received_requests().await.unwrap_or_default();
        assert_eq!(requests.len(), 1);
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"));
        assert!(body.contains("assertion="));
        assert!(body.contains("client_id=client-id"));
        assert!(body.contains("client_secret=client-secret"));

        Ok(())
    }

    /// 有効期限までの残り時間が`refresh_before_expiry`以下の場合は、アクセストークンを取得し直すことを確認
    #[tokio::test]
    async fn test_access_token_refreshes_near_expiration() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(&server, token_response()).await;
        // `refresh_before_expiry`がトークンの有効期間3600秒以上であるため、毎回取得し直す。
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            Duration::from_secs(3600),
        )?;

        manager.access_token().await?;
        manager.access_token().await?;

        assert_eq!(request_count(&server).await, 2);

        Ok(())
    }

    /// 同時に呼び出されても、トークンエンドポイントへのリクエストを1つにまとめることを確認
    #[tokio::test]
    async fn test_access_token_single_flight() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        // 最初の取得が完了する前に3つの呼び出しが始まるように、レスポンスを遅らせる。
        mount_always(
            &server,
            token_response().set_delay(Duration::from_millis(100)),
        )
        .await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let (a, b, c) = tokio::join!(
            manager.access_token(),
            manager.access_token(),
            manager.access_token()
        );

        let (a, b, c) = (a?, b?, c?);
        assert!(Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(&b, &c));
        assert_eq!(request_count(&server).await, 1);

        Ok(())
    }

    /// 拒否されたアクセストークンがキャッシュと一致する場合のみ破棄し、次の呼び出しで取得し直すことを確認
    #[rstest]
    #[case::current_token(true, 2)]
    #[case::stale_token(false, 1)]
    #[tokio::test]
    async fn test_invalidate(
        #[case] reject_current: bool,
        #[case] expected_requests: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(&server, token_response()).await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            REFRESH_BEFORE_EXPIRY,
        )?;
        let current = manager.access_token().await?;
        let rejected = if reject_current {
            current
        } else {
            Arc::new(AccessToken {
                token: SecretString::from("stale-token"),
                expires_at: Instant::now(),
            })
        };

        manager.invalidate(&rejected).await;
        manager.access_token().await?;

        assert_eq!(request_count(&server).await, expected_requests);

        Ok(())
    }

    /// 429または5xxの後は再試行し、アクセストークンを取得することを確認
    #[rstest]
    #[case::too_many_requests(429)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_access_token_retries_retryable_status(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_first_n(&server, ResponseTemplate::new(status), 1).await;
        mount_always(&server, token_response()).await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(1)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let token = manager.access_token().await?;

        assert_eq!(token.secret(), "access-token");
        assert_eq!(request_count(&server).await, 2);

        Ok(())
    }

    /// Retry-Afterヘッダーがある場合は、指数バックオフではなくRetry-Afterヘッダーが示す時間だけ待機することを確認
    #[tokio::test]
    async fn test_access_token_waits_retry_after() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_first_n(
            &server,
            ResponseTemplate::new(429).insert_header("Retry-After", "0"),
            1,
        )
        .await;
        mount_always(&server, token_response()).await;
        // 指数バックオフで待機すると10秒かかる設定にする。
        let retry = RetryPolicy::new(
            1,
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::ZERO,
        )?;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            retry,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let token = tokio::time::timeout(Duration::from_secs(5), manager.access_token()).await??;

        assert_eq!(token.secret(), "access-token");

        Ok(())
    }

    /// 再試行回数の上限に達した場合は、最後のエラーを返すことを確認
    #[rstest]
    #[case::no_retry(0, 1)]
    #[case::retry_twice(2, 3)]
    #[tokio::test]
    async fn test_access_token_fails_after_max_retries(
        #[case] max_retries: u32,
        #[case] expected_requests: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(&server, ResponseTemplate::new(500)).await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(max_retries)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let result = manager.access_token().await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 500),
            "result: {:?}",
            result.err()
        );
        assert_eq!(request_count(&server).await, expected_requests);

        Ok(())
    }

    /// 429以外の4xxの場合は、再試行せずにエラーコードを含むエラーを返すことを確認
    #[rstest]
    #[case::bad_request(400)]
    #[case::unauthorized(401)]
    #[tokio::test]
    async fn test_access_token_does_not_retry_client_error(
        #[case] status: u16,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(
            &server,
            ResponseTemplate::new(status).set_body_string(r#"{"error":"invalid_grant"}"#),
        )
        .await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(2)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let result = manager.access_token().await;

        match result {
            Err(BoxClientError::Api(e)) => {
                assert_eq!(e.status, status);
                assert_eq!(e.code.as_deref(), Some("invalid_grant"));
            }
            other => anyhow::bail!("unexpected result: {:?}", other.err()),
        }
        assert_eq!(request_count(&server).await, 1);

        Ok(())
    }

    /// タイムアウトした場合は、再試行した後にタイムアウトエラーを返すことを確認
    #[tokio::test]
    async fn test_access_token_retries_timeout() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(&server, token_response().set_delay(Duration::from_secs(1))).await;
        let manager = create_manager(
            &server,
            Duration::from_millis(100),
            short_retry(1)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let result = manager.access_token().await;

        assert!(
            matches!(&result, Err(BoxClientError::Http(e)) if e.is_timeout()),
            "result: {:?}",
            result.err()
        );
        assert_eq!(request_count(&server).await, 2);

        Ok(())
    }

    /// トークンレスポンスを解釈できない場合は、再試行せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_access_token_does_not_retry_invalid_response() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(
            &server,
            ResponseTemplate::new(200).set_body_string("not json"),
        )
        .await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(2)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let result = manager.access_token().await;

        assert!(
            matches!(&result, Err(BoxClientError::Http(e)) if e.is_decode()),
            "result: {:?}",
            result.err()
        );
        assert_eq!(request_count(&server).await, 1);

        Ok(())
    }
}
