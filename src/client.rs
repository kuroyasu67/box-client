use std::{future::Future, sync::Arc, time::Duration};

use reqwest::{Response, StatusCode, Url, header::CONTENT_TYPE};
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

use crate::{
    auth::{BOX_TOKEN_URL, TokenManager},
    config::BoxConfig,
    errors::{BoxClientError, BoxClientResult},
    request::{ApiHost, ApiRequest},
    retry::{
        AttemptError, Idempotency, RetryPolicy, classify_error_response, classify_transport_error,
        read_json, run_with_retry,
    },
};

/// BOXのREST APIのベースURL
const BOX_API_BASE_URL: &str = "https://api.box.com/2.0";

/// BOXのアップロードのAPIのベースURL
///
/// BOXは、ファイルのアップロードを、REST APIとは別のホストで受け付ける。
const BOX_UPLOAD_BASE_URL: &str = "https://upload.box.com/api/2.0";

/// アクセストークンを有効期限のどれだけ前に取得し直すかの既定値
///
/// <https://developer.box.com/guides/api-calls/permissions-and-errors/expiration>
///
/// 上記ドキュメントによると、アクセストークンは発行から60分後に期限切れになる。
/// 有効期限までの残り時間がこの値以下になると、トークン管理者はバックグラウンドで新しいアクセストークンを取得する。
/// トークン管理者は、取得を終えるまで、期限が切れていないキャッシュのアクセストークンを使い続ける。
/// 期限が切れるまでにトークン管理者が取得を終えられないと、APIを呼び出すタスクは取得を待つことになる。
/// したがって、この値は、再試行を含めてアクセストークンの取得にかかる時間より長くする必要がある。
///
/// 通常、トークン管理者はアクセストークンを1秒未満で取得できる。トークンエンドポイントが一時的に失敗し、
/// トークン管理者が再試行設定に従って数回再試行しても期限内に取得を終えられるように、この既定値を60秒とする。
/// その結果、トークン管理者は発行から約59分後にアクセストークンを取得し直す。
const DEFAULT_REFRESH_BEFORE_EXPIRY: Duration = Duration::from_secs(60);

/// アクセストークンを有効期限のどれだけ前に取得し直すかの上限
///
/// アクセストークンの有効期間である60分以上を指定すると、トークン管理者は、取得した直後からアクセストークンを取得し直す必要があると判定する。
/// その結果、トークン管理者は、APIが呼び出されるたびにバックグラウンドで新しいアクセストークンを取得し、
/// トークンエンドポイントへのリクエストが絶えず発生する。
/// したがって、この上限を有効期間の半分とし、トークン管理者に、取得したアクセストークンを少なくとも有効期間の半分は再利用させる。
pub const MAX_REFRESH_BEFORE_EXPIRY: Duration = Duration::from_mins(30);

/// BOXクライアント
///
/// BOXのAPIを呼び出すそれぞれのメソッドは、再試行設定に従って待機と再試行を行い、アクセストークンの
/// 有効期限が近づくと自動でアクセストークンを取得し直す。
///
/// APIメソッドを追加するときは、[`ApiRequest`]を組み立てて[`BoxClient::send_json`]または
/// [`BoxClient::send_empty`]に渡す。
pub struct BoxClient {
    /// HTTPクライアント
    http_client: reqwest::Client,

    /// REST APIのベースURL
    api_base_url: Url,

    /// アップロードのAPIのベースURL
    upload_base_url: Url,

    /// 再試行設定
    retry: RetryPolicy,

    /// トークン管理者
    ///
    /// アクセストークンをバックグラウンドで取得し直すタスクと共有するため、`Arc`で保持する。
    token_manager: Arc<TokenManager>,
}

impl std::fmt::Debug for BoxClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxClient")
            .field("http_client", &self.http_client)
            .field("api_base_url", &self.api_base_url.as_str())
            .field("upload_base_url", &self.upload_base_url.as_str())
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl BoxClient {
    /// BOXクライアントのビルダーを返す。
    ///
    /// # Arguments
    ///
    /// - `config`: BOX設定
    #[must_use]
    pub fn builder(config: BoxConfig) -> BoxClientBuilder {
        BoxClientBuilder {
            config,
            timeout: BoxClientTimeout::default(),
            retry: RetryPolicy::default(),
            refresh_before_expiry: DEFAULT_REFRESH_BEFORE_EXPIRY,
            api_base_url: None,
            upload_base_url: None,
            token_url: None,
        }
    }

    /// 再試行設定を返す。
    ///
    /// 1回のAPI呼び出しの外側で待機するAPIメソッドが、再試行設定に従って待機時間を決めるために使用する。
    pub(crate) fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    /// リクエストを送信し、成功レスポンスのJSONボディをデシリアライズして返す。
    ///
    /// ボディの受信中に失敗した場合も、冪等なリクエストであれば再試行する。
    ///
    /// # Errors
    ///
    /// 成功レスポンスのボディをJSONとして解釈できない場合は、再試行せずに
    /// [`BoxClientError::DeserializeResponse`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn send_json<T: DeserializeOwned>(
        &self,
        request: ApiRequest<'_>,
    ) -> BoxClientResult<T> {
        self.send(request, read_json::<T>).await
    }

    /// リクエストを送信し、成功レスポンスのボディを読まずに破棄する。
    ///
    /// `204 No Content`を返すDELETEなどに使用する。
    ///
    /// # Errors
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn send_empty(&self, request: ApiRequest<'_>) -> BoxClientResult<()> {
        self.send(request, |_, _| std::future::ready(Ok(()))).await
    }

    /// 再試行設定に従ってリクエストを送信し、成功レスポンスを`handle`で処理した結果を返す。
    ///
    /// 202 Acceptedのように、成功レスポンスのステータスによって処理を変えるAPIは、`handle`でステータスを調べる。
    pub(crate) async fn send<T, F, Fut>(
        &self,
        request: ApiRequest<'_>,
        handle: F,
    ) -> BoxClientResult<T>
    where
        F: Fn(Response, Idempotency) -> Fut,
        Fut: Future<Output = Result<T, AttemptError>>,
    {
        let base_url = match request.host {
            ApiHost::Api => &self.api_base_url,
            ApiHost::Upload => &self.upload_base_url,
        };
        let url = request.url(base_url)?;
        // 成功のイベントはDEBUGで出力するが、スパンはINFOのままにする。
        // スパンをDEBUGにすると、INFOで記録する利用者のWARNおよびERRORのイベントから、メソッドとパスが失われるためである。
        let span = tracing::info_span!(
            "box_api_request",
            method = %request.method,
            path = url.path(),
        );
        run_with_retry(&self.retry, "box_api_request", || {
            self.attempt(&request, &url, &handle)
        })
        .instrument(span)
        .await
    }

    /// リクエストを1回送信し、成功レスポンスを`handle`で処理する。
    ///
    /// BOXがアクセストークンを拒否して401を返した場合は、アクセストークンを取得し直して1回だけ再送する。
    /// 401はBOXがリクエストを処理していないことを示すため、冪等でないリクエストでも再送できる。
    async fn attempt<T, F, Fut>(
        &self,
        request: &ApiRequest<'_>,
        url: &Url,
        handle: &F,
    ) -> Result<T, AttemptError>
    where
        F: Fn(Response, Idempotency) -> Fut,
        Fut: Future<Output = Result<T, AttemptError>>,
    {
        let idempotency = request.idempotency;
        let mut token_refreshed = false;
        loop {
            let token = self
                .token_manager
                .access_token()
                .await
                .map_err(AttemptError::Fatal)?;
            let mut builder = self
                .http_client
                .request(request.method.clone(), url.clone())
                .bearer_auth(token.secret());
            for (name, value) in &request.headers {
                builder = builder.header(name, value);
            }
            if let Some(body) = &request.body {
                builder = builder
                    .header(CONTENT_TYPE, body.content_type.clone())
                    .body(body.bytes.clone());
            }
            // スパンもメソッドとパスを持つが、スパンを出力しない設定のサブスクライバーでも呼び出したAPIを識別できるように、
            // イベントにもメソッドとパスを記録する。クエリ文字列にはページ送りのマーカーなどが含まれ、長くなるため記録しない。
            // 再試行および401の後の再送でも、送信のたびに出力する。
            tracing::debug!(
                method = %request.method,
                path = url.path(),
                "sending request to BOX API"
            );
            let response = builder
                .send()
                .await
                .map_err(|e| classify_transport_error(e, idempotency))?;

            let status = response.status();
            if status == StatusCode::UNAUTHORIZED && !token_refreshed {
                tracing::warn!("access token was rejected; fetching a new one and resending");
                self.token_manager.invalidate(&token).await;
                token_refreshed = true;
                continue;
            }
            if !status.is_success() {
                return Err(classify_error_response(response, idempotency).await);
            }

            return handle(response, idempotency).await;
        }
    }
}

/// BOXクライアントのビルダー
#[derive(Debug)]
pub struct BoxClientBuilder {
    /// BOX設定
    config: BoxConfig,

    /// タイムアウト設定
    timeout: BoxClientTimeout,

    /// 再試行設定
    retry: RetryPolicy,

    /// アクセストークンを有効期限のどれだけ前に取得し直すか
    refresh_before_expiry: Duration,

    /// REST APIのベースURL。`None`の場合はBOXのURLを使用する。
    api_base_url: Option<Url>,

    /// アップロードのAPIのベースURL。`None`の場合はBOXのURLを使用する。
    upload_base_url: Option<Url>,

    /// トークンエンドポイントのURL。`None`の場合はBOXのURLを使用する。
    token_url: Option<String>,
}

impl BoxClientBuilder {
    /// タイムアウト設定を指定する。
    #[must_use]
    pub fn timeout(mut self, timeout: BoxClientTimeout) -> Self {
        self.timeout = timeout;
        self
    }

    /// 再試行設定を指定する。
    ///
    /// アクセストークンの取得とREST APIの呼び出しの両方に適用する。
    #[must_use]
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// アクセストークンを有効期限のどれだけ前に取得し直すかを指定する。既定値は60秒とする。
    ///
    /// アクセストークンの有効期間は60分である。有効期限までの残り時間がこの値以下になると、
    /// BOXクライアントはバックグラウンドで新しいアクセストークンを取得する。BOXクライアントは、取得を終えるまで、
    /// 期限が切れていないアクセストークンを使い続けるため、APIの呼び出しを待たせない。
    /// 利用者は、期限が切れるまでにBOXクライアントが取得を終えられるように、この値を、
    /// 再試行を含めてアクセストークンの取得にかかる時間より長くする。
    ///
    /// [`MAX_REFRESH_BEFORE_EXPIRY`]以下を指定する。上限を超える値は[`BoxClientBuilder::build`]でエラーになる。
    #[must_use]
    pub fn refresh_before_expiry(mut self, refresh_before_expiry: Duration) -> Self {
        self.refresh_before_expiry = refresh_before_expiry;
        self
    }

    /// テストでモックサーバーに接続できるように、REST APIとトークンエンドポイントのURLを指定する。
    #[cfg(test)]
    pub(crate) fn endpoints(mut self, api_base_url: Url, token_url: String) -> Self {
        self.api_base_url = Some(api_base_url);
        self.token_url = Some(token_url);
        self
    }

    /// テストでモックサーバーに接続できるように、アップロードのAPIのベースURLを指定する。
    #[cfg(test)]
    pub(crate) fn upload_base_url(mut self, upload_base_url: Url) -> Self {
        self.upload_base_url = Some(upload_base_url);
        self
    }

    /// BOXクライアントを作成する。
    ///
    /// 設定の誤りに起動時に気づけるように、最初のアクセストークンをここで取得する。
    ///
    /// # Errors
    ///
    /// `refresh_before_expiry`が[`MAX_REFRESH_BEFORE_EXPIRY`]を超える場合は、トークンエンドポイントへリクエストせずに
    /// [`BoxClientError::InvalidRefreshBeforeExpiry`]を返す。
    /// HTTPクライアントを作成できない場合、秘密鍵を復号できない場合、
    /// またはアクセストークンを取得できない場合はエラーを返す。
    pub async fn build(self) -> BoxClientResult<BoxClient> {
        if MAX_REFRESH_BEFORE_EXPIRY < self.refresh_before_expiry {
            return Err(BoxClientError::InvalidRefreshBeforeExpiry {
                refresh_before_expiry: self.refresh_before_expiry,
                max: MAX_REFRESH_BEFORE_EXPIRY,
            });
        }

        let api_base_url = match self.api_base_url {
            Some(url) => url,
            None => Url::parse(BOX_API_BASE_URL)
                .map_err(|e| BoxClientError::InvalidBaseUrl(format!("{BOX_API_BASE_URL}: {e}")))?,
        };
        let upload_base_url = match self.upload_base_url {
            Some(url) => url,
            None => Url::parse(BOX_UPLOAD_BASE_URL).map_err(|e| {
                BoxClientError::InvalidBaseUrl(format!("{BOX_UPLOAD_BASE_URL}: {e}"))
            })?,
        };
        let token_url = self.token_url.unwrap_or_else(|| BOX_TOKEN_URL.to_owned());
        let http_client = reqwest::Client::builder()
            .timeout(self.timeout.request_timeout)
            .connect_timeout(self.timeout.connect_timeout)
            .build()?;
        let token_manager = Arc::new(TokenManager::new(
            http_client.clone(),
            &self.config,
            token_url,
            self.retry.clone(),
            self.refresh_before_expiry,
        )?);
        token_manager.access_token().await?;

        Ok(BoxClient {
            http_client,
            api_base_url,
            upload_base_url,
            retry: self.retry,
            token_manager,
        })
    }
}

/// BOXクライアントのタイムアウト設定
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// ```
/// use std::time::Duration;
///
/// use box_client::BoxClientTimeout;
///
/// let timeout = BoxClientTimeout::default()
///     .with_request_timeout(Duration::from_secs(60))
///     .with_connect_timeout(Duration::from_secs(5));
/// assert_eq!(timeout.request_timeout, Duration::from_secs(60));
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BoxClientTimeout {
    /// 接続の開始からレスポンスボディの受信完了までのタイムアウト
    pub request_timeout: Duration,

    /// 接続が確立するまでのタイムアウト
    pub connect_timeout: Duration,
}

impl Default for BoxClientTimeout {
    /// レスポンスボディの受信完了までを30秒、接続の確立までを10秒でタイムアウトさせる設定を返す。
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

impl BoxClientTimeout {
    /// 接続の開始からレスポンスボディの受信完了までのタイムアウトを設定する。
    #[must_use]
    pub fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// 接続が確立するまでのタイムアウトを設定する。
    #[must_use]
    pub fn with_connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }
}

#[cfg(test)]
mod test_box_client {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rstest::rstest;
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    use super::*;
    use crate::test_support::{
        TOKEN_PATH, create_client, create_client_with_timeout, mount_token, short_retry,
        test_config,
    };

    /// APIのパス
    const API_PATH: &str = "/2.0/items/1";

    /// `API_PATH`へのリクエストを数える。
    async fn api_request_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| r.url.path() == API_PATH)
            .count()
    }

    /// トークンエンドポイントへのリクエストを数える。
    async fn token_request_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| r.url.path() == TOKEN_PATH)
            .count()
    }

    /// 最初の`n`回だけAPIが`response`を返すモックを登録する。
    async fn mount_api_first_n(server: &MockServer, response: ResponseTemplate, n: u64) {
        Mock::given(path(API_PATH))
            .respond_with(response)
            .up_to_n_times(n)
            .mount(server)
            .await;
    }

    /// APIが常に`response`を返すモックを登録する。
    async fn mount_api_always(server: &MockServer, response: ResponseTemplate) {
        Mock::given(path(API_PATH))
            .respond_with(response)
            .mount(server)
            .await;
    }

    /// 成功レスポンスを返す。
    fn ok_response() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({"id": "1"}))
    }

    /// テストで送信するリクエストを返す。
    fn request(method: &str) -> anyhow::Result<ApiRequest<'static>> {
        Ok(ApiRequest::new(method.parse()?, ["items", "1"]))
    }

    /// BOXクライアントを作成するときに、アクセストークンを取得することを確認
    #[tokio::test]
    async fn test_build_fetches_access_token() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;

        create_client(&server, short_retry(0)?).await?;

        assert_eq!(token_request_count(&server).await, 1);

        Ok(())
    }

    /// `refresh_before_expiry`が上限以下の場合のみ、BOXクライアントを作成することを確認
    #[rstest]
    #[case::zero(Duration::ZERO, true)]
    #[case::default(DEFAULT_REFRESH_BEFORE_EXPIRY, true)]
    #[case::below_max(MAX_REFRESH_BEFORE_EXPIRY.saturating_sub(Duration::from_nanos(1)), true)]
    #[case::equal_to_max(MAX_REFRESH_BEFORE_EXPIRY, true)]
    #[case::above_max(MAX_REFRESH_BEFORE_EXPIRY + Duration::from_nanos(1), false)]
    #[case::token_lifetime(Duration::from_secs(3600), false)]
    #[tokio::test]
    async fn test_build_validates_refresh_before_expiry(
        #[case] refresh_before_expiry: Duration,
        #[case] valid: bool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;

        let result = BoxClient::builder(test_config())
            .retry(short_retry(0)?)
            .refresh_before_expiry(refresh_before_expiry)
            .endpoints(
                Url::parse(&format!("{}/2.0", server.uri()))?,
                format!("{}{TOKEN_PATH}", server.uri()),
            )
            .build()
            .await;

        if valid {
            assert!(result.is_ok(), "result: {:?}", result.err());
            assert_eq!(token_request_count(&server).await, 1);
        } else {
            assert!(
                matches!(
                    &result,
                    Err(BoxClientError::InvalidRefreshBeforeExpiry { refresh_before_expiry: r, max })
                        if *r == refresh_before_expiry && *max == MAX_REFRESH_BEFORE_EXPIRY
                ),
                "result: {:?}",
                result.err()
            );
            // 設定の誤りを検出した時点で、トークンエンドポイントへリクエストしない。
            assert_eq!(token_request_count(&server).await, 0);
        }

        Ok(())
    }

    /// アクセストークンを取得できない場合は、BOXクライアントの作成に失敗することを確認
    #[tokio::test]
    async fn test_build_fails_without_access_token() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        Mock::given(path(TOKEN_PATH))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let result = create_client(&server, short_retry(0)?).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 400),
            "result: {:?}",
            result.err()
        );

        Ok(())
    }

    /// アクセストークンをBearer認証で送信し、レスポンスのJSONをデシリアライズすることを確認
    #[tokio::test]
    async fn test_send_json() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(API_PATH))
            .and(header("authorization", "Bearer access-token"))
            .respond_with(ok_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let value: serde_json::Value = client.send_json(request("GET")?).await?;

        assert_eq!(value, json!({"id": "1"}));

        Ok(())
    }

    /// リクエストに追加したヘッダーを送信することを確認
    #[tokio::test]
    async fn test_send_with_header() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(API_PATH))
            .and(header("if-match", "1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        client
            .send_empty(request("DELETE")?.header(reqwest::header::IF_MATCH, "1")?)
            .await?;

        Ok(())
    }

    /// リクエストボディをJSONとして送信することを確認
    #[tokio::test]
    async fn test_send_json_with_body() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(API_PATH))
            .and(header("content-type", "application/json"))
            .and(body_json(json!({"name": "new"})))
            .respond_with(ok_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let value: serde_json::Value = client
            .send_json(request("POST")?.json(&json!({"name": "new"}))?)
            .await?;

        assert_eq!(value, json!({"id": "1"}));

        Ok(())
    }

    /// ボディのない成功レスポンスを受け取れることを確認
    #[tokio::test]
    async fn test_send_empty() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_api_always(&server, ResponseTemplate::new(204)).await;
        let client = create_client(&server, short_retry(0)?).await?;

        client.send_empty(request("DELETE")?).await?;

        assert_eq!(api_request_count(&server).await, 1);

        Ok(())
    }

    /// ステータスとHTTPメソッドの冪等性に応じて、再試行するかどうかを決めることを確認
    #[rstest]
    #[case::get_429("GET", 429, 2)]
    #[case::post_429("POST", 429, 2)]
    #[case::get_500("GET", 500, 2)]
    #[case::put_503("PUT", 503, 2)]
    #[case::delete_500("DELETE", 500, 2)]
    #[case::post_500("POST", 500, 1)]
    #[case::get_404("GET", 404, 1)]
    #[tokio::test]
    async fn test_send_retries_by_status_and_idempotency(
        #[case] method: &str,
        #[case] status: u16,
        #[case] expected_requests: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_api_first_n(&server, ResponseTemplate::new(status), 1).await;
        mount_api_always(&server, ok_response()).await;
        let client = create_client(&server, short_retry(1)?).await?;

        let result: BoxClientResult<serde_json::Value> = client.send_json(request(method)?).await;

        assert_eq!(result.is_ok(), expected_requests == 2, "result: {result:?}");
        assert_eq!(api_request_count(&server).await, expected_requests);

        Ok(())
    }

    /// 冪等でないリクエストでも、冪等性を上書きすれば5xxの後に再試行することを確認
    #[tokio::test]
    async fn test_send_retries_overridden_idempotency() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_api_first_n(&server, ResponseTemplate::new(500), 1).await;
        mount_api_always(&server, ok_response()).await;
        let client = create_client(&server, short_retry(1)?).await?;

        let _: serde_json::Value = client
            .send_json(request("POST")?.idempotency(Idempotency::Idempotent))
            .await?;

        assert_eq!(api_request_count(&server).await, 2);

        Ok(())
    }

    /// タイムアウトの後は、冪等なリクエストのみ再試行することを確認
    #[rstest]
    #[case::get("GET", 2)]
    #[case::post("POST", 1)]
    #[tokio::test]
    async fn test_send_retries_timeout_by_idempotency(
        #[case] method: &str,
        #[case] expected_requests: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_api_always(&server, ok_response().set_delay(Duration::from_secs(1))).await;
        let client =
            create_client_with_timeout(&server, short_retry(1)?, Duration::from_millis(200))
                .await?;

        let result: BoxClientResult<serde_json::Value> = client.send_json(request(method)?).await;

        assert!(
            matches!(&result, Err(BoxClientError::Http(e)) if e.is_timeout()),
            "result: {result:?}"
        );
        assert_eq!(api_request_count(&server).await, expected_requests);

        Ok(())
    }

    /// 401を受け取った場合は、アクセストークンを取得し直して1回だけ再送することを確認
    #[rstest]
    #[case::get("GET")]
    #[case::post("POST")]
    #[tokio::test]
    async fn test_send_refreshes_token_once_on_unauthorized(
        #[case] method: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_api_first_n(&server, ResponseTemplate::new(401), 1).await;
        mount_api_always(&server, ok_response()).await;
        // 再試行回数を0にし、再送が再試行回数に含まれないことも確かめる。
        let client = create_client(&server, short_retry(0)?).await?;

        let _: serde_json::Value = client.send_json(request(method)?).await?;

        assert_eq!(api_request_count(&server).await, 2);
        // 構築時に1回、401の後に1回取得する。
        assert_eq!(token_request_count(&server).await, 2);

        Ok(())
    }

    /// アクセストークンを取得し直しても401を受け取った場合は、エラーを返すことを確認
    #[tokio::test]
    async fn test_send_fails_on_repeated_unauthorized() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_api_always(&server, ResponseTemplate::new(401)).await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result: BoxClientResult<serde_json::Value> = client.send_json(request("GET")?).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 401),
            "result: {result:?}"
        );
        assert_eq!(api_request_count(&server).await, 2);

        Ok(())
    }

    /// BOXのエラーレスポンスから、エラーコード、メッセージおよびリクエストIDを取り出すことを確認
    #[tokio::test]
    async fn test_send_returns_api_error() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_api_always(
            &server,
            ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
                "request_id": "request-id",
            })),
        )
        .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result: BoxClientResult<serde_json::Value> = client.send_json(request("GET")?).await;

        match result {
            Err(BoxClientError::Api(e)) => {
                assert_eq!(e.status, 404);
                assert_eq!(e.code.as_deref(), Some("not_found"));
                assert_eq!(e.message.as_deref(), Some("Not Found"));
                assert_eq!(e.request_id.as_deref(), Some("request-id"));
            }
            other => anyhow::bail!("unexpected result: {other:?}"),
        }

        Ok(())
    }

    /// 成功レスポンスのJSONを解釈できない場合は、再試行せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_send_json_does_not_retry_decode_error() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_api_always(
            &server,
            ResponseTemplate::new(200).set_body_string("not json"),
        )
        .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result: BoxClientResult<serde_json::Value> = client.send_json(request("GET")?).await;

        assert!(
            matches!(&result, Err(BoxClientError::DeserializeResponse(_))),
            "result: {result:?}"
        );
        assert_eq!(api_request_count(&server).await, 1);

        Ok(())
    }

    /// 成功レスポンスのヘッダーを返した後、ボディの途中で接続を切るHTTPサーバーを起動する。
    ///
    /// 戻り値は、サーバーを指すREST APIのベースURLと、サーバーが受け付けた接続の数である。
    async fn start_truncating_server() -> anyhow::Result<(Url, Arc<AtomicUsize>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let api_base_url = Url::parse(&format!("http://{}/2.0", listener.local_addr()?))?;
        let connections = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&connections);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                // リクエストヘッダーを読み終えてから、宣言した長さより短いボディを返して接続を切る。
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                          content-length: 100\r\n\r\n{\"id\":",
                    )
                    .await;
            }
        });

        Ok((api_base_url, connections))
    }

    /// 成功レスポンスのボディの受信中に接続が切れた場合は、冪等なリクエストのみ再試行することを確認
    #[rstest]
    #[case::idempotent("GET", 3)]
    #[case::non_idempotent("POST", 1)]
    #[tokio::test]
    async fn test_send_json_retries_body_error_only_if_idempotent(
        #[case] method: &str,
        #[case] expected_connections: usize,
    ) -> anyhow::Result<()> {
        let token_server = MockServer::start().await;
        mount_token(&token_server).await;
        let (api_base_url, connections) = start_truncating_server().await?;
        let client = BoxClient::builder(test_config())
            .retry(short_retry(2)?)
            .endpoints(api_base_url, format!("{}{TOKEN_PATH}", token_server.uri()))
            .build()
            .await?;

        let result: BoxClientResult<serde_json::Value> = client.send_json(request(method)?).await;

        // ボディの受信の失敗は、JSONの解釈の失敗ではなく通信エラーとして返す。
        assert!(
            matches!(&result, Err(BoxClientError::Http(_))),
            "result: {result:?}"
        );
        assert_eq!(connections.load(Ordering::SeqCst), expected_connections);

        Ok(())
    }

    /// 接続できない場合は、冪等でないリクエストでも最大再試行回数まで再試行した後に接続エラーを返すことを確認
    #[rstest]
    #[case::no_retry(0)]
    #[case::one_retry(1)]
    #[case::two_retries(2)]
    #[tokio::test]
    async fn test_send_retries_connection_error(#[case] max_retries: u32) -> anyhow::Result<()> {
        // 接続を拒否されたリクエストはモックサーバーに届かず、リクエストの数を数えられない。
        // したがって、ジッターをなくして毎回の待機時間を`DELAY`に固定し、経過時間から再試行回数を確かめる。
        // ローカルホストへの接続はすぐに拒否されるため、`DELAY`を十分に長くして、接続にかかる時間と区別する。
        const DELAY: Duration = Duration::from_millis(500);
        let server = MockServer::start().await;
        mount_token(&server).await;
        // wiremockはMockServerを破棄しても待ち受けを続けるため、OSが割り当てたポートを閉じて、
        // 接続を拒否されるURLを用意する。
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let api_base_url = Url::parse(&format!("http://{}/2.0", listener.local_addr()?))?;
        drop(listener);
        let client = BoxClient::builder(test_config())
            .retry(RetryPolicy::new(max_retries, DELAY, DELAY, Duration::ZERO)?)
            .endpoints(api_base_url, format!("{}{TOKEN_PATH}", server.uri()))
            .build()
            .await?;

        let started_at = std::time::Instant::now();
        let result: BoxClientResult<serde_json::Value> = client.send_json(request("POST")?).await;
        let elapsed = started_at.elapsed();

        assert!(
            matches!(&result, Err(BoxClientError::Http(e)) if e.is_connect()),
            "result: {result:?}"
        );
        // 再試行した回数だけ`DELAY`を待機し、それ以上は待機していない。
        assert!(
            DELAY * max_retries <= elapsed && elapsed < DELAY * (max_retries + 1),
            "elapsed: {elapsed:?}"
        );

        Ok(())
    }

    /// パス要素が不正な場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_send_rejects_invalid_path_segment() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result: BoxClientResult<serde_json::Value> =
            client.send_json(ApiRequest::get(["items", ".."])).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidPathSegment(_))),
            "result: {result:?}"
        );
        // BOXクライアントの作成時に送信したトークンリクエストだけを受信している。
        assert_eq!(
            server.received_requests().await.unwrap_or_default().len(),
            1
        );

        Ok(())
    }

    /// Debug出力にクライアントシークレット、秘密鍵、パスフレーズおよびアクセストークンを含めないことを確認
    #[tokio::test]
    async fn test_debug_does_not_expose_secrets() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let builder = format!("{:?}", BoxClient::builder(test_config()));
        let client = format!("{:?}", create_client(&server, short_retry(0)?).await?);

        for secret in [
            "client-secret",
            "test-passphrase",
            "PRIVATE KEY",
            "access-token",
        ] {
            assert!(!builder.contains(secret), "builder: {builder}");
            assert!(!client.contains(secret), "client: {client}");
        }
        // シークレット以外の設定は出力し、デバッグに使えることも確かめる。
        assert!(builder.contains("client-id"), "builder: {builder}");
        assert!(
            client.contains(&format!("\"{}/2.0\"", server.uri())),
            "client: {client}"
        );

        Ok(())
    }

    /// `ThreadRng`を使用しても、リクエストを送信するfutureとBOXクライアントを作成するfutureが`Send`を実装し、`tokio::spawn`に渡せることを確認
    #[tokio::test]
    async fn test_send_future_is_send() -> anyhow::Result<()> {
        fn assert_send<T: Send>(_: &T) {}
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;

        let future = client.send_json::<serde_json::Value>(request("GET")?);

        assert_send(&future);
        assert_send(&BoxClient::builder(test_config()).build());

        Ok(())
    }
}
