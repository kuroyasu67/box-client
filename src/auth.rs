use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use pkcs8::{DecodePrivateKey as _, EncodePrivateKey as _, LineEnding, PrivateKeyInfoOwned};
use reqwest::{StatusCode, header::DATE};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};
use tracing::Instrument as _;
use uuid::Uuid;

use crate::{
    assertion_claims::AssertionClaims,
    config::BoxConfig,
    errors::{ApiError, BoxClientError, BoxClientResult, CryptoError},
    retry::{
        AttemptError, Idempotency, RetryPolicy, classify_api_error, classify_transport_error,
        read_error_response, read_json, run_with_retry,
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

/// バックグラウンドでの取得に失敗した後、次にバックグラウンドで取得し直すまでの間隔の上限
///
/// 失敗した直後から、APIが呼び出されるたびに取得し直すと、トークンエンドポイントへのリクエストが絶えず発生する。
/// アプリケーションの認可が取り消された場合など、再試行しても成功しない失敗では、期限が切れるまでリクエストが続く。
/// したがって、トークン管理者は、失敗した後は間隔をあけてからバックグラウンドで取得し直す。
/// アクセストークンが期限切れの場合は、間隔に関係なく、呼び出し元を待たせて取得する。
///
/// 実際の間隔は[`refresh_interval_after_failure`]で決める。
const MAX_REFRESH_INTERVAL_AFTER_FAILURE: Duration = Duration::from_secs(10);

/// 取得し直す期間の中で、バックグラウンドで取得を試みる機会の最小数
///
/// 間隔を`refresh_before_expiry`のこの数分の1以下にし、`refresh_before_expiry`を短く設定した場合でも、
/// 期限が切れるまでに複数回バックグラウンドで取得し直す機会を残す。
const MIN_BACKGROUND_REFRESH_CHANCES: u32 = 4;

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
        )
        .map_err(|e| BoxClientError::DecryptPrivateKey(CryptoError::new(e)))?;
        let private_key_pem = private_key_info
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| BoxClientError::DecryptPrivateKey(CryptoError::new(e)))?;
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .map_err(|e| BoxClientError::CreateSigningKey(CryptoError::new(e)))?;

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
    /// # Arguments
    ///
    /// - `clock_offset_secs`: BOXの時刻からシステム時刻を引いた秒数。有効期限をBOXの時刻に合わせるために使用する。
    ///
    /// # Errors
    ///
    /// システム時刻を取得できない場合、または署名できない場合はエラーを返す。
    fn sign(&self, clock_offset_secs: i64) -> BoxClientResult<String> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs()
            .saturating_add_signed(clock_offset_secs);
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

        jsonwebtoken::encode(&self.header, &claims, &self.encoding_key)
            .map_err(|e| BoxClientError::SignAssertion(CryptoError::new(e)))
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
/// 複数のタスクから同時に呼び出されても、トークン管理者はトークンエンドポイントへのリクエストを1つにまとめる。
///
/// トークン管理者は、キャッシュしたアクセストークンの状態に応じて、次のように振る舞う。
///
/// - 有効期限までの残り時間が`refresh_before_expiry`より長い場合、トークン管理者はキャッシュしたアクセストークンを返す。
/// - 有効期限までの残り時間が`refresh_before_expiry`以下で、アクセストークンがまだ期限切れでない場合、
///   トークン管理者はバックグラウンドのタスクでアクセストークンを取得し直し、呼び出し元にはキャッシュしたアクセストークンを返す。
///   キャッシュしたアクセストークンは期限が切れるまで使用できるため、トークン管理者は呼び出し元を取得の完了まで待たせない。
///   バックグラウンドでの取得に失敗した場合、トークン管理者は[`refresh_interval_after_failure`]が返す間隔が経過するまで、
///   バックグラウンドで取得し直さない。
/// - アクセストークンが期限切れの場合、またはキャッシュがない場合、トークン管理者はアクセストークンを取得し終えるまで呼び出し元を待たせる。
///
/// バックグラウンドのタスクはトークン管理者の`Arc`を保持するため、呼び出し元は[`TokenManager::access_token`]を`Arc<Self>`から呼び出す。
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
    /// 401を受け取ったときに、別のタスクが取得し直した新しいトークンを誤って破棄しないように、
    /// `Arc`のポインターでトークンを識別する。
    /// トークンエンドポイントと通信する間は、このロックを保持しない。
    cache: RwLock<Option<Arc<AccessToken>>>,

    /// アクセストークンを取得するタスクを1つに限るためのロック
    ///
    /// ロックは、バックグラウンドでの取得に最後に失敗した時刻を保持する。
    /// バックグラウンドで取得に成功した場合は`None`に戻す。
    /// バックグラウンドのタスクにロックを渡せるように、`Arc`で保持する。
    refresh_lock: Arc<Mutex<Option<Instant>>>,

    /// バックグラウンドでの取得に失敗した後、次にバックグラウンドで取得し直すまでの間隔
    ///
    /// `refresh_before_expiry`から決める。テストで変更できるように、フィールドで保持する。
    refresh_interval_after_failure: Duration,

    /// BOXの時刻からシステム時刻を引いた秒数
    ///
    /// BOXがJWTアサーションの有効期限を理由に拒否したときに、レスポンスの`Date`ヘッダーから求める。
    /// 一度求めた値は、以後に作成するJWTアサーションの有効期限にも使用する。
    clock_offset_secs: AtomicI64,
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
            refresh_lock: Arc::new(Mutex::new(None)),
            refresh_interval_after_failure: refresh_interval_after_failure(refresh_before_expiry),
            clock_offset_secs: AtomicI64::new(0),
        })
    }

    /// バックグラウンドでの取得に失敗した後、次にバックグラウンドで取得し直すまでの間隔を変更する。
    #[cfg(test)]
    fn with_refresh_interval_after_failure(mut self, interval: Duration) -> Self {
        self.refresh_interval_after_failure = interval;
        self
    }

    /// 有効なアクセストークンを返す。
    ///
    /// キャッシュしたアクセストークンの有効期限が近い場合、このメソッドはバックグラウンドのタスクにアクセストークンを取得し直させ、
    /// キャッシュしたアクセストークンを返す。
    /// キャッシュしたアクセストークンが期限切れの場合、またはキャッシュがない場合、このメソッドはアクセストークンを取得してから返す。
    ///
    /// 呼び出し元が取得を待つ途中でfutureを破棄しても、ロックが解放されてキャッシュが更新されないだけで、不整合は生じない。
    /// ただし、バックグラウンドのタスクは、呼び出し元がfutureを破棄しても中断せず、取得の成功または失敗まで実行する。
    ///
    /// # Errors
    ///
    /// キャッシュしたアクセストークンが期限切れの場合、またはキャッシュがない場合に、
    /// このメソッドはアクセストークンを取得できなければエラーを返す。
    pub(crate) async fn access_token(self: &Arc<Self>) -> BoxClientResult<Arc<AccessToken>> {
        let cached = self.cache.read().await.clone();
        if let Some(token) = cached {
            let now = Instant::now();
            if !needs_refresh(token.expires_at, now, self.refresh_before_expiry) {
                return Ok(token);
            }
            if !is_expired(token.expires_at, now) {
                self.spawn_refresh();
                return Ok(token);
            }
        }

        // キャッシュしたアクセストークンが期限切れの場合、またはキャッシュがない場合は、ロックを得た1つのタスクだけが取得し、
        // 他のタスクはロックを待つことで、そのタスクの取得を待つ。
        let _guard = self.refresh_lock.lock().await;

        // ロックを待つ間に別のタスクまたはバックグラウンドのタスクが取得し直している場合、このタスクはそのアクセストークンを返す。
        if let Some(token) = self.unexpired_cached_token().await {
            return Ok(token);
        }

        let token = Arc::new(self.fetch().await?);
        *self.cache.write().await = Some(Arc::clone(&token));

        Ok(token)
    }

    /// アクセストークンを取得し直すバックグラウンドのタスクを起動する。
    ///
    /// 別のタスクがすでにアクセストークンを取得している場合、このメソッドは何もしない。
    /// バックグラウンドでの取得に失敗してから`refresh_interval_after_failure`が経過していない場合も、何もしない。
    fn spawn_refresh(self: &Arc<Self>) {
        let Ok(guard) = Arc::clone(&self.refresh_lock).try_lock_owned() else {
            return;
        };
        if guard.is_some_and(|failed_at| failed_at.elapsed() < self.refresh_interval_after_failure)
        {
            return;
        }
        let manager = Arc::clone(self);
        // 取得のきっかけとなったリクエストとログを関連付けるため、バックグラウンドのタスクは呼び出し元のスパンを引き継ぐ。
        tokio::spawn(async move { manager.refresh(guard).await }.in_current_span());
    }

    /// アクセストークンを取得し直してキャッシュする。
    ///
    /// `guard`はアクセストークンを取得するタスクを1つに限るためのロックであり、このメソッドは取得を終えるとロックを解放する。
    /// 取得に失敗した場合、このメソッドはログを出力し、失敗した時刻を`guard`に記録するだけにする。
    /// キャッシュしたアクセストークンは期限が切れるまで使用できるためである。
    /// `refresh_interval_after_failure`が経過した後に呼び出し元が[`TokenManager::access_token`]を呼び出すと、
    /// トークン管理者は再びバックグラウンドでアクセストークンを取得し直す。
    async fn refresh(&self, mut guard: OwnedMutexGuard<Option<Instant>>) {
        // ロックを得るまでの間に別のタスクが取得し直している場合、このタスクはアクセストークンを取得しない。
        if self.fresh_cached_token().await.is_some() {
            return;
        }

        match self.fetch().await {
            Ok(token) => {
                *self.cache.write().await = Some(Arc::new(token));
                *guard = None;
            }
            Err(error) => {
                *guard = Some(Instant::now());
                tracing::warn!(
                    error = &error as &(dyn std::error::Error + 'static),
                    retry_interval_ms = self.refresh_interval_after_failure.as_millis(),
                    "failed to refresh access token in background; using the cached token until it expires"
                );
            }
        }
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

    /// キャッシュしたアクセストークンが取得し直す必要のない場合に、そのアクセストークンを返す。
    async fn fresh_cached_token(&self) -> Option<Arc<AccessToken>> {
        self.cache
            .read()
            .await
            .as_ref()
            .filter(|token| {
                !needs_refresh(token.expires_at, Instant::now(), self.refresh_before_expiry)
            })
            .map(Arc::clone)
    }

    /// キャッシュしたアクセストークンが期限切れでない場合に、そのアクセストークンを返す。
    async fn unexpired_cached_token(&self) -> Option<Arc<AccessToken>> {
        self.cache
            .read()
            .await
            .as_ref()
            .filter(|token| !is_expired(token.expires_at, Instant::now()))
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
    ///
    /// システム時刻がBOXの時刻とずれていると、BOXはJWTアサーションの有効期限を理由にリクエストを拒否する。
    /// この場合は、レスポンスの`Date`ヘッダーが示すBOXの時刻に有効期限を合わせて、JWTアサーションを作り直して1回だけ再送する。
    /// BOXはリクエストを処理せずに拒否しているため、この再送は再試行回数に数えない。
    async fn request_access_token(&self) -> Result<AccessToken, AttemptError> {
        // アクセストークンを二重に発行しても害はなく、JWTアサーションも試行ごとに作り直すため、冪等なリクエストとして再試行する。
        let idempotency = Idempotency::Idempotent;
        let mut clock_adjusted = false;
        let response = loop {
            let assertion = self
                .assertion_signer
                .sign(self.clock_offset_secs.load(Ordering::Relaxed))
                .map_err(AttemptError::Fatal)?;
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
            if response.status().is_success() {
                break response;
            }

            let box_time = response
                .headers()
                .get(DATE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| httpdate::parse_http_date(value).ok());
            let (api_error, retry_after) = read_error_response(response).await;
            if !clock_adjusted
                && is_assertion_expiration_rejected(&api_error)
                && let Some(box_time) = box_time
            {
                let offset = clock_offset_secs(box_time, SystemTime::now());
                self.clock_offset_secs.store(offset, Ordering::Relaxed);
                clock_adjusted = true;
                tracing::warn!(
                    clock_offset_secs = offset,
                    "jwt assertion was rejected because of its expiration; adjusting to the box server time and resending"
                );
                continue;
            }
            return Err(classify_api_error(api_error, retry_after, idempotency));
        };

        let body: TokenResponse = read_json(response, idempotency).await?;
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

/// BOXがJWTアサーションの有効期限（`exp`クレーム）を理由にトークンリクエストを拒否したかどうかを返す。
///
/// BOXは、エラーコード`invalid_grant`の400を返し、エラーの説明で`exp`クレームに言及する。
/// 公式ドキュメントはエラーの説明の文言を記載していないため、BOXの旧版のPython SDKと同じく、
/// エラーの説明に`exp`が含まれるかどうかで判定する。
fn is_assertion_expiration_rejected(api_error: &ApiError) -> bool {
    api_error.status == StatusCode::BAD_REQUEST
        && api_error.code.as_deref() == Some("invalid_grant")
        && api_error
            .message
            .as_deref()
            .is_some_and(|message| message.contains("exp"))
}

/// BOXの時刻からシステム時刻を引いた秒数を返す。
///
/// `Date`ヘッダーの精度は1秒であるため、1秒未満は切り捨てる。
fn clock_offset_secs(box_time: SystemTime, local_time: SystemTime) -> i64 {
    match box_time.duration_since(local_time) {
        Ok(ahead) => i64::try_from(ahead.as_secs()).unwrap_or(i64::MAX),
        Err(behind) => i64::try_from(behind.duration().as_secs()).map_or(i64::MIN, |secs| -secs),
    }
}

/// アクセストークンを取得し直す必要があるかどうかを返す。
///
/// 有効期限までの残り時間が`refresh_before_expiry`以下の場合、トークン管理者は、期限が切れる前に新しいアクセストークンを用意するため、
/// アクセストークンを取得し直す。
fn needs_refresh(expires_at: Instant, now: Instant, refresh_before_expiry: Duration) -> bool {
    expires_at.saturating_duration_since(now) <= refresh_before_expiry
}

/// バックグラウンドでの取得に失敗した後、次にバックグラウンドで取得し直すまでの間隔を返す。
///
/// 間隔は、[`MAX_REFRESH_INTERVAL_AFTER_FAILURE`]と、`refresh_before_expiry`の[`MIN_BACKGROUND_REFRESH_CHANCES`]分の1の
/// 短いほうとする。既定の`refresh_before_expiry`（60秒）では10秒になる。
/// `refresh_before_expiry`を短く設定した場合も、期限が切れるまでに複数回バックグラウンドで取得し直す機会を残す。
fn refresh_interval_after_failure(refresh_before_expiry: Duration) -> Duration {
    MAX_REFRESH_INTERVAL_AFTER_FAILURE.min(refresh_before_expiry / MIN_BACKGROUND_REFRESH_CHANCES)
}

/// アクセストークンが期限切れかどうかを返す。
///
/// 有効期限の時刻ちょうどのアクセストークンはBOXが拒否する可能性があるため、この関数は期限切れとして扱う。
fn is_expired(expires_at: Instant, now: Instant) -> bool {
    expires_at <= now
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

    /// BOXが求めるヘッダーとクレームを持つJWTアサーションに署名することを確認
    #[test]
    fn test_sign() -> anyhow::Result<()> {
        let assertion_signer = AssertionSigner::new(&test_config(), AUDIENCE)?;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        let assertion = assertion_signer.sign(0)?;

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

    /// BOXとの時刻差だけずらした有効期限を設定することを確認
    #[rstest::rstest]
    #[case::box_ahead(120)]
    #[case::box_behind(-120)]
    fn test_sign_applies_clock_offset(#[case] clock_offset_secs: i64) -> anyhow::Result<()> {
        let assertion_signer = AssertionSigner::new(&test_config(), AUDIENCE)?;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        let assertion = assertion_signer.sign(clock_offset_secs)?;

        let claims: serde_json::Value =
            jsonwebtoken::dangerous::insecure_decode(&assertion)?.claims;
        let exp = claims["exp"].as_u64().unwrap_or_default();
        let expected = now.saturating_add_signed(clock_offset_secs) + ASSERTION_LIFETIME_SECS;
        // 署名の前後で秒が変わる場合があるため、1秒の幅を許容する。
        assert!(
            (expected..=expected + 1).contains(&exp),
            "exp: {exp}, expected: {expected}"
        );

        Ok(())
    }

    /// 署名するたびに異なるjtiを設定することを確認
    #[test]
    fn test_sign_unique_jti() -> anyhow::Result<()> {
        let assertion_signer = AssertionSigner::new(&test_config(), AUDIENCE)?;

        let jtis = (0..3)
            .map(|_| {
                let assertion = assertion_signer.sign(0)?;
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
mod test_is_expired {
    use rstest::rstest;

    use super::*;

    /// 有効期限の時刻ちょうど以降を期限切れと判定することを確認
    #[rstest]
    #[case::before_expiration(Duration::from_secs(1), Duration::ZERO, false)]
    #[case::at_expiration(Duration::ZERO, Duration::ZERO, true)]
    #[case::after_expiration(Duration::ZERO, Duration::from_secs(1), true)]
    fn test_is_expired(
        #[case] until_expiration: Duration,
        #[case] since_now: Duration,
        #[case] expected: bool,
    ) {
        let base = Instant::now();

        assert_eq!(
            is_expired(base + until_expiration, base + since_now),
            expected
        );
    }
}

#[cfg(test)]
mod test_assertion_expiration {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    /// エラーコードが`invalid_grant`の400で、エラーの説明に`exp`が含まれる場合だけ、有効期限による拒否と判定することを確認
    #[rstest]
    #[case::exp(400, json!({"error": "invalid_grant", "error_description": "Please check the 'exp' claim."}), true)]
    #[case::other_description(400, json!({"error": "invalid_grant", "error_description": "Invalid client credentials"}), false)]
    #[case::other_code(400, json!({"error": "invalid_request", "error_description": "Please check the 'exp' claim."}), false)]
    #[case::other_status(401, json!({"error": "invalid_grant", "error_description": "Please check the 'exp' claim."}), false)]
    #[case::no_description(400, json!({"error": "invalid_grant"}), false)]
    fn test_is_assertion_expiration_rejected(
        #[case] status: u16,
        #[case] body: serde_json::Value,
        #[case] expected: bool,
    ) -> anyhow::Result<()> {
        let api_error = ApiError::new(StatusCode::from_u16(status)?, body.to_string());

        assert_eq!(is_assertion_expiration_rejected(&api_error), expected);

        Ok(())
    }

    /// BOXの時刻からシステム時刻を引いた秒数を、1秒未満を切り捨てて返すことを確認
    #[rstest]
    #[case::same(Duration::from_secs(1000), Duration::from_secs(1000), 0)]
    #[case::box_ahead(Duration::from_secs(1120), Duration::from_secs(1000), 120)]
    #[case::box_behind(Duration::from_secs(880), Duration::from_secs(1000), -120)]
    #[case::sub_second(Duration::from_secs(1000), Duration::from_millis(1_000_500), 0)]
    fn test_clock_offset_secs(
        #[case] box_since_epoch: Duration,
        #[case] local_since_epoch: Duration,
        #[case] expected: i64,
    ) {
        let box_time = SystemTime::UNIX_EPOCH + box_since_epoch;
        let local_time = SystemTime::UNIX_EPOCH + local_since_epoch;

        assert_eq!(clock_offset_secs(box_time, local_time), expected);
    }
}

#[cfg(test)]
mod test_refresh_interval_after_failure {
    use rstest::rstest;

    use super::*;

    /// 上限の10秒と、`refresh_before_expiry`の4分の1の短いほうを間隔とすることを確認
    #[rstest]
    // 既定の`refresh_before_expiry`では、4分の1の15秒が上限を超えるため、上限の10秒になる
    #[case::default(Duration::from_secs(60), Duration::from_secs(10))]
    // 4分の1が上限とちょうど一致する
    #[case::equals_max(Duration::from_secs(40), Duration::from_secs(10))]
    // 4分の1が上限より短い
    #[case::short(Duration::from_secs(20), Duration::from_secs(5))]
    #[case::zero(Duration::ZERO, Duration::ZERO)]
    fn test_refresh_interval_after_failure(
        #[case] refresh_before_expiry: Duration,
        #[case] expected: Duration,
    ) {
        assert_eq!(
            refresh_interval_after_failure(refresh_before_expiry),
            expected
        );
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
    ) -> anyhow::Result<Arc<TokenManager>> {
        let http_client = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Arc::new(TokenManager::new(
            http_client,
            &test_config(),
            format!("{}{TOKEN_PATH}", server.uri()),
            retry,
            refresh_before_expiry,
        )?))
    }

    /// 指定したアクセストークンと有効期間を返すトークンエンドポイントの成功レスポンスを返す。
    fn token_response_with(access_token: &str, expires_in: u64) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": access_token,
            "expires_in": expires_in,
            "token_type": "bearer",
        }))
    }

    /// キャッシュしたアクセストークンが`access_token`になるまで待ち、そのアクセストークンを返す。
    ///
    /// バックグラウンドのタスクがアクセストークンを取得し直すのを待つために使用する。
    async fn wait_for_cached_token(
        manager: &TokenManager,
        access_token: &str,
    ) -> anyhow::Result<Arc<AccessToken>> {
        let wait = async {
            loop {
                if let Some(token) = manager.cache.read().await.clone()
                    && token.secret() == access_token
                {
                    return token;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(2), wait)
            .await
            .map_err(|_| anyhow::anyhow!("access token was not refreshed: {access_token}"))
    }

    /// トークンエンドポイントが受信したリクエストの数が`expected`になるまで待つ。
    ///
    /// バックグラウンドのタスクが取得を終えるのを待つために使用する。
    async fn wait_for_request_count(server: &MockServer, expected: usize) -> anyhow::Result<()> {
        let wait = async {
            while request_count(server).await < expected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(2), wait)
            .await
            .map_err(|_| anyhow::anyhow!("token endpoint did not receive {expected} requests"))
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

    /// 有効期限までの残り時間が`refresh_before_expiry`以下の場合は、取得を待たずにキャッシュしたアクセストークンを返し、
    /// バックグラウンドで取得し直すことを確認
    #[tokio::test]
    async fn test_access_token_refreshes_in_background() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_first_n(&server, token_response_with("old-token", 3600), 1).await;
        // 呼び出し元が取得を待っていないことを確かめるため、取得し直すときのレスポンスを遅らせる。
        mount_always(
            &server,
            token_response_with("new-token", 3600).set_delay(Duration::from_millis(500)),
        )
        .await;
        // `refresh_before_expiry`がトークンの有効期間3600秒と等しいため、取得した直後から取得し直す期間に入る。
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            Duration::from_secs(3600),
        )?;
        let old = manager.access_token().await?;

        let cached =
            tokio::time::timeout(Duration::from_millis(200), manager.access_token()).await??;

        assert!(Arc::ptr_eq(&old, &cached));
        let new = wait_for_cached_token(&manager, "new-token").await?;
        assert!(!Arc::ptr_eq(&old, &new));
        assert_eq!(request_count(&server).await, 2);

        Ok(())
    }

    /// 有効期限が近いときに同時に呼び出されても、バックグラウンドで取得し直すタスクを1つに限ることを確認
    #[tokio::test]
    async fn test_access_token_refreshes_in_background_once() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_first_n(&server, token_response_with("old-token", 3600), 1).await;
        // すべての呼び出しがバックグラウンドの取得中に行われるように、レスポンスを遅らせる。
        mount_always(
            &server,
            token_response_with("new-token", 3600).set_delay(Duration::from_millis(500)),
        )
        .await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            Duration::from_secs(3600),
        )?;
        let old = manager.access_token().await?;

        let (a, b, c) = tokio::join!(
            manager.access_token(),
            manager.access_token(),
            manager.access_token()
        );

        for token in [a?, b?, c?] {
            assert!(Arc::ptr_eq(&old, &token));
        }
        wait_for_cached_token(&manager, "new-token").await?;
        assert_eq!(request_count(&server).await, 2);

        Ok(())
    }

    /// バックグラウンドでの取得に失敗しても、期限が切れていないアクセストークンを使い続けることを確認
    #[tokio::test]
    async fn test_access_token_keeps_cached_token_when_background_refresh_fails()
    -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_first_n(&server, token_response_with("old-token", 3600), 1).await;
        mount_always(&server, ResponseTemplate::new(500)).await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            Duration::from_secs(3600),
        )?;
        let old = manager.access_token().await?;

        let cached = manager.access_token().await?;
        wait_for_request_count(&server, 2).await?;
        let after_failure = manager.access_token().await?;

        assert!(Arc::ptr_eq(&old, &cached));
        assert!(Arc::ptr_eq(&old, &after_failure));

        Ok(())
    }

    /// バックグラウンドでの取得に失敗した後は、間隔が経過するまでバックグラウンドで取得し直さないことを確認
    #[tokio::test]
    async fn test_access_token_waits_interval_after_background_refresh_failure()
    -> anyhow::Result<()> {
        const INTERVAL: Duration = Duration::from_millis(300);
        let server = MockServer::start().await;
        mount_first_n(&server, token_response_with("old-token", 3600), 1).await;
        mount_always(&server, ResponseTemplate::new(500)).await;
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        // `refresh_before_expiry`がトークンの有効期間3600秒と等しいため、取得した直後から取得し直す期間に入る。
        let manager = Arc::new(
            TokenManager::new(
                http_client,
                &test_config(),
                format!("{}{TOKEN_PATH}", server.uri()),
                short_retry(0)?,
                Duration::from_secs(3600),
            )?
            .with_refresh_interval_after_failure(INTERVAL),
        );
        manager.access_token().await?;

        // 1回目のバックグラウンドでの取得が失敗し、失敗した時刻を記録するまで待つ。
        manager.access_token().await?;
        wait_for_request_count(&server, 2).await?;
        let _ = manager.refresh_lock.lock().await;

        // 間隔が経過するまでは、何度呼び出してもバックグラウンドで取得し直さない。
        for _ in 0..5 {
            manager.access_token().await?;
        }
        tokio::time::sleep(INTERVAL / 3).await;
        assert_eq!(request_count(&server).await, 2);

        // 間隔が経過した後は、再びバックグラウンドで取得し直す。
        tokio::time::sleep(INTERVAL).await;
        manager.access_token().await?;
        wait_for_request_count(&server, 3).await?;

        Ok(())
    }

    /// キャッシュしたアクセストークンが期限切れの場合は、取得を待ってから新しいアクセストークンを返すことを確認
    #[tokio::test]
    async fn test_access_token_fetches_when_expired() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        // 有効期間0秒のアクセストークンは、取得した時点で期限切れになる。
        mount_first_n(&server, token_response_with("expired-token", 0), 1).await;
        mount_always(&server, token_response_with("new-token", 3600)).await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            REFRESH_BEFORE_EXPIRY,
        )?;
        manager.access_token().await?;

        let token = manager.access_token().await?;

        assert_eq!(token.secret(), "new-token");
        assert_eq!(request_count(&server).await, 2);

        Ok(())
    }

    /// キャッシュしたアクセストークンが期限切れで、取得にも失敗した場合は、エラーを返すことを確認
    #[tokio::test]
    async fn test_access_token_fails_when_expired_and_fetch_fails() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_first_n(&server, token_response_with("expired-token", 0), 1).await;
        mount_always(&server, ResponseTemplate::new(500)).await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            REFRESH_BEFORE_EXPIRY,
        )?;
        manager.access_token().await?;

        let result = manager.access_token().await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 500),
            "result: {:?}",
            result.err()
        );

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

    /// トークンリクエストの本文からJWTアサーションの有効期限を取り出す。
    fn assertion_exp(request: &wiremock::Request) -> anyhow::Result<u64> {
        let body = String::from_utf8_lossy(&request.body);
        let assertion = body
            .split('&')
            .find_map(|pair| pair.strip_prefix("assertion="))
            .ok_or_else(|| anyhow::anyhow!("assertion was not found: {body}"))?;
        let claims: serde_json::Value = jsonwebtoken::dangerous::insecure_decode(assertion)?.claims;
        claims["exp"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("exp was not found: {claims}"))
    }

    /// JWTアサーションの有効期限を理由に拒否するトークンエンドポイントのレスポンスを返す。
    fn expiration_rejected_response(box_time: SystemTime) -> ResponseTemplate {
        ResponseTemplate::new(400)
            .insert_header("date", httpdate::fmt_http_date(box_time))
            .set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "Please check the 'exp' claim.",
            }))
    }

    /// JWTアサーションの有効期限を理由に拒否された場合は、BOXの時刻に合わせて1回だけ再送し、
    /// 以後のJWTアサーションにもBOXとの時刻差を使うことを確認
    #[tokio::test]
    async fn test_access_token_adjusts_clock_when_expiration_rejected() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        // BOXの時刻がシステム時刻より2分進んでいる。
        let box_time = SystemTime::now() + Duration::from_secs(120);
        mount_first_n(&server, expiration_rejected_response(box_time), 1).await;
        mount_always(&server, token_response()).await;
        // 再試行回数を0にし、再送が再試行回数に含まれないことも確かめる。
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        manager.access_token().await?;

        let requests = server.received_requests().await.unwrap_or_default();
        assert_eq!(requests.len(), 2);
        let box_secs = box_time.duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
        let expected = box_secs + ASSERTION_LIFETIME_SECS;
        let exp = assertion_exp(&requests[1])?;
        // `Date`ヘッダーの精度と、署名の前後で秒が変わる場合を考慮し、前後1秒の幅を許容する。
        assert!(
            (expected - 1..=expected + 1).contains(&exp),
            "exp: {exp}, expected: {expected}"
        );
        // `Date`ヘッダーは1秒未満を切り捨て、時刻差も1秒未満を切り捨てるため、2分より最大2秒短くなりうる。
        let offset = manager.clock_offset_secs.load(Ordering::Relaxed);
        assert!((118..=120).contains(&offset), "offset: {offset}");

        Ok(())
    }

    /// 時刻を合わせて再送しても有効期限を理由に拒否された場合は、それ以上再送せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_access_token_adjusts_clock_only_once() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(
            &server,
            expiration_rejected_response(SystemTime::now() + Duration::from_secs(120)),
        )
        .await;
        let manager = create_manager(
            &server,
            Duration::from_secs(2),
            short_retry(0)?,
            REFRESH_BEFORE_EXPIRY,
        )?;

        let result = manager.access_token().await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == StatusCode::BAD_REQUEST),
            "result: {:?}",
            result.err()
        );
        assert_eq!(request_count(&server).await, 2);

        Ok(())
    }

    /// 有効期限以外の理由でJWTアサーションを拒否された場合は、再送せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_access_token_does_not_resend_other_invalid_grant() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_always(
            &server,
            ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "Invalid client credentials",
            })),
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
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == StatusCode::BAD_REQUEST),
            "result: {:?}",
            result.err()
        );
        assert_eq!(request_count(&server).await, 1);
        assert_eq!(manager.clock_offset_secs.load(Ordering::Relaxed), 0);

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
            matches!(&result, Err(BoxClientError::DeserializeResponse(_))),
            "result: {:?}",
            result.err()
        );
        assert_eq!(request_count(&server).await, 1);

        Ok(())
    }
}
