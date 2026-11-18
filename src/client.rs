use std::{future::Future, net::IpAddr, sync::Arc, time::Duration};

use reqwest::{
    Response, StatusCode, Url,
    header::{CONTENT_TYPE, HeaderValue},
};
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

use crate::{
    auth::{BOX_TOKEN_URL, TokenManager},
    config::BoxConfig,
    errors::{BoxClientError, BoxClientResult},
    request::{ApiHost, ApiRequest, HttpClientKind},
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

/// `User-Agent`ヘッダーで送信する、このクレートの名前とバージョン
///
/// 利用者がアプリケーションの名前を指定した場合は、その後ろに続けて送信する。
const CRATE_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

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

/// 分割アップロードで、並列にアップロードする部分の数の既定値
///
/// <https://developer.box.com/guides/uploads/chunked/upload-part/>
///
/// 上記ドキュメントは、3から5の部分を並列にアップロードすることを推奨しているため、その中間の4とする。
pub const DEFAULT_UPLOAD_CONCURRENCY: usize = 4;

/// 分割アップロードで、並列にアップロードする部分の数の最小値
///
/// `0`ではアップロードが進まないため、部分を1つずつ順にアップロードする`1`を最小値とする。
pub const MIN_UPLOAD_CONCURRENCY: usize = 1;

/// 分割アップロードで、並列にアップロードする部分の数の最大値
///
/// BOXが推奨する並列数の上限である5を最大値とする。
/// 並列にアップロードする部分はそれぞれメモリに読み込むため、上限を設けてメモリの使用量も抑える。
pub const MAX_UPLOAD_CONCURRENCY: usize = 5;

/// 分割アップロードに切り替えるファイルサイズ（バイト）の既定値
///
/// この値を超えるファイルは、分割アップロードのAPIでアップロードする。
/// BOXは、50MBを超えるファイルに分割アップロードを推奨しているため、指定できる最大値と同じ50MiBとする。
pub const DEFAULT_CHUNKED_UPLOAD_THRESHOLD: u64 = MAX_CHUNKED_UPLOAD_THRESHOLD;

/// 分割アップロードに切り替えるファイルサイズ（バイト）に指定できる最小値
///
/// 分割アップロードのAPIは20MB未満のファイルを拒否する。
/// 分割アップロードするファイルが必ず20MB以上になるように、20MBより大きい20MiBを最小値とする。
pub const MIN_CHUNKED_UPLOAD_THRESHOLD: u64 = 20 * 1024 * 1024;

/// 分割アップロードに切り替えるファイルサイズ（バイト）に指定できる最大値
///
/// この値以下のファイルは、ファイル全体をメモリに読み込んで、1回のリクエストでアップロードする。
/// BOXは、50MBを超えるファイルに分割アップロードを推奨しているため、50MiBを最大値とする。
pub const MAX_CHUNKED_UPLOAD_THRESHOLD: u64 = 50 * 1024 * 1024;

/// 分割アップロードのコミットで、BOXが処理中を示す202を返した場合に、コミットを再送する回数の上限の既定値
///
/// BOXは、部分の処理を終えるまで202を返し続ける。大きいファイルでは処理に時間がかかるため、
/// 再試行設定の既定値で待機した場合に数分間はコミットを再送し続けられるように、30回とする。
pub const DEFAULT_MAX_COMMIT_POLLS: u32 = 30;

/// 分割アップロードのコミットで、コミットを再送する回数の上限に指定できる最大値
///
/// 1回の再送の前に、最大で`RetryPolicy::max_delay`とジッターの合計だけ待機する。
/// 再送回数が大きすぎると、BOXが処理を終えない場合に、呼び出し元が非常に長い時間待たされるため、最大値を設ける。
pub const MAX_COMMIT_POLLS_LIMIT: u32 = 100;

/// ファイルのダウンロードで、BOXがダウンロードの準備ができていないことを示す202を返した場合に、
/// リクエストを再送する回数の上限の既定値
///
/// BOXのドキュメントによると、202はアップロードの直後にダウンロードした場合などに返され、
/// 通常は短時間で準備が終わるため、10回とする。
pub const DEFAULT_MAX_DOWNLOAD_POLLS: u32 = 10;

/// ファイルのダウンロードで、リクエストを再送する回数の上限に指定できる最大値
///
/// [`MAX_COMMIT_POLLS_LIMIT`]と同じ理由で、最大値を設ける。
pub const MAX_DOWNLOAD_POLLS_LIMIT: u32 = 100;

/// BOXクライアント
///
/// BOXのAPIを呼び出すそれぞれのメソッドは、再試行設定に従って待機と再試行を行い、アクセストークンの
/// 有効期限が近づくと自動でアクセストークンを取得し直す。
///
/// APIメソッドを追加するときは、[`ApiRequest`]を組み立てて[`BoxClient::send_json`]または
/// [`BoxClient::send_empty`]に渡す。
pub struct BoxClient {
    /// HTTPクライアント
    ///
    /// リクエスト全体に[`BoxClientTimeout::request_timeout`]を適用する。
    http_client: reqwest::Client,

    /// ファイルのダウンロードに使用するHTTPクライアント
    ///
    /// ダウンロードはファイルサイズによって時間が大きく変わるため、リクエスト全体のタイムアウトを適用せずに、
    /// 無通信の時間に[`BoxClientTimeout::download_read_timeout`]を適用する。
    download_client: reqwest::Client,

    /// REST APIのベースURL
    api_base_url: Url,

    /// アップロードのAPIのベースURL
    upload_base_url: Url,

    /// 再試行設定
    retry: RetryPolicy,

    /// 分割アップロードで、並列にアップロードする部分の数
    upload_concurrency: usize,

    /// 分割アップロードに切り替えるファイルサイズ（バイト）
    chunked_upload_threshold: u64,

    /// 分割アップロードのコミットで、BOXが202を返した場合にコミットを再送する回数の上限
    max_commit_polls: u32,

    /// ファイルのダウンロードで、BOXが202を返した場合にリクエストを再送する回数の上限
    max_download_polls: u32,

    /// トークン管理者
    ///
    /// アクセストークンをバックグラウンドで取得し直すタスクと共有するため、`Arc`で保持する。
    token_manager: Arc<TokenManager>,
}

impl std::fmt::Debug for BoxClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxClient")
            .field("http_client", &self.http_client)
            .field("download_client", &self.download_client)
            .field("api_base_url", &self.api_base_url.as_str())
            .field("upload_base_url", &self.upload_base_url.as_str())
            .field("retry", &self.retry)
            .field("upload_concurrency", &self.upload_concurrency)
            .field("chunked_upload_threshold", &self.chunked_upload_threshold)
            .field("max_commit_polls", &self.max_commit_polls)
            .field("max_download_polls", &self.max_download_polls)
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
            upload_concurrency: DEFAULT_UPLOAD_CONCURRENCY,
            chunked_upload_threshold: DEFAULT_CHUNKED_UPLOAD_THRESHOLD,
            max_commit_polls: DEFAULT_MAX_COMMIT_POLLS,
            max_download_polls: DEFAULT_MAX_DOWNLOAD_POLLS,
            api_base_url: None,
            upload_base_url: None,
            token_url: None,
            user_agent: HeaderValue::from_static(CRATE_USER_AGENT),
        }
    }

    /// 再試行設定を返す。
    ///
    /// 1回のAPI呼び出しの外側で待機するAPIメソッドが、再試行設定に従って待機時間を決めるために使用する。
    pub(crate) fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    /// 分割アップロードで、並列にアップロードする部分の数を返す。
    #[must_use]
    pub fn upload_concurrency(&self) -> usize {
        self.upload_concurrency
    }

    /// 分割アップロードに切り替えるファイルサイズ（バイト）を返す。
    #[must_use]
    pub fn chunked_upload_threshold(&self) -> u64 {
        self.chunked_upload_threshold
    }

    /// 分割アップロードのコミットで、BOXが202を返した場合にコミットを再送する回数の上限を返す。
    #[must_use]
    pub fn max_commit_polls(&self) -> u32 {
        self.max_commit_polls
    }

    /// ファイルのダウンロードで、BOXが202を返した場合にリクエストを再送する回数の上限を返す。
    #[must_use]
    pub fn max_download_polls(&self) -> u32 {
        self.max_download_polls
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
            let http_client = match request.client {
                HttpClientKind::Default => &self.http_client,
                HttpClientKind::Download => &self.download_client,
            };
            let mut builder = http_client
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

    /// 分割アップロードで、並列にアップロードする部分の数
    upload_concurrency: usize,

    /// 分割アップロードに切り替えるファイルサイズ（バイト）
    chunked_upload_threshold: u64,

    /// 分割アップロードのコミットで、BOXが202を返した場合にコミットを再送する回数の上限
    max_commit_polls: u32,

    /// ファイルのダウンロードで、BOXが202を返した場合にリクエストを再送する回数の上限
    max_download_polls: u32,

    /// REST APIのベースURL。`None`の場合はBOXのURLを使用する。
    api_base_url: Option<Url>,

    /// アップロードのAPIのベースURL。`None`の場合はBOXのURLを使用する。
    upload_base_url: Option<Url>,

    /// トークンエンドポイントのURL。`None`の場合はBOXのURLを使用する。
    token_url: Option<String>,

    /// `User-Agent`ヘッダーの値
    user_agent: HeaderValue,
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

    /// 分割アップロードで、並列にアップロードする部分の数を指定する。既定値は[`DEFAULT_UPLOAD_CONCURRENCY`]とする。
    ///
    /// 並列にアップロードする部分は、それぞれメモリに読み込む。部分のサイズはBOXが決め、
    /// 例えば8MiBの場合は、この値が4のときに最大32MiBをメモリに保持する。
    /// `1`を指定すると、部分を1つずつ順にアップロードする。
    ///
    /// # Errors
    ///
    /// `upload_concurrency`が[`MIN_UPLOAD_CONCURRENCY`]以上[`MAX_UPLOAD_CONCURRENCY`]以下でない場合は、
    /// [`BoxClientError::InvalidUploadConcurrency`]を返す。
    pub fn upload_concurrency(mut self, upload_concurrency: usize) -> BoxClientResult<Self> {
        if !(MIN_UPLOAD_CONCURRENCY..=MAX_UPLOAD_CONCURRENCY).contains(&upload_concurrency) {
            return Err(BoxClientError::InvalidUploadConcurrency {
                upload_concurrency,
                min: MIN_UPLOAD_CONCURRENCY,
                max: MAX_UPLOAD_CONCURRENCY,
            });
        }
        self.upload_concurrency = upload_concurrency;
        Ok(self)
    }

    /// 分割アップロードに切り替えるファイルサイズ（バイト）を指定する。
    /// 既定値は[`DEFAULT_CHUNKED_UPLOAD_THRESHOLD`]とする。
    ///
    /// この値以下のファイルは、ファイル全体をメモリに読み込んで、1回のリクエストでアップロードする。
    /// この値を超えるファイルは、分割アップロードのAPIでアップロードする。
    /// 値を小さくすると、1回のリクエストでアップロードするときに使用するメモリと、1回のリクエストで送信するデータを減らせる。
    ///
    /// # Errors
    ///
    /// `chunked_upload_threshold`が[`MIN_CHUNKED_UPLOAD_THRESHOLD`]以上[`MAX_CHUNKED_UPLOAD_THRESHOLD`]以下でない場合は、
    /// [`BoxClientError::InvalidChunkedUploadThreshold`]を返す。
    pub fn chunked_upload_threshold(
        mut self,
        chunked_upload_threshold: u64,
    ) -> BoxClientResult<Self> {
        if !(MIN_CHUNKED_UPLOAD_THRESHOLD..=MAX_CHUNKED_UPLOAD_THRESHOLD)
            .contains(&chunked_upload_threshold)
        {
            return Err(BoxClientError::InvalidChunkedUploadThreshold {
                chunked_upload_threshold,
                min: MIN_CHUNKED_UPLOAD_THRESHOLD,
                max: MAX_CHUNKED_UPLOAD_THRESHOLD,
            });
        }
        self.chunked_upload_threshold = chunked_upload_threshold;
        Ok(self)
    }

    /// 分割アップロードのコミットで、BOXが処理中を示す202を返した場合に、コミットを再送する回数の上限を指定する。
    /// 既定値は[`DEFAULT_MAX_COMMIT_POLLS`]とする。
    ///
    /// `0`を指定すると、BOXが202を返した時点で再送せずにエラーを返す。
    /// 202の後の再送は、再試行設定の最大再試行回数に含めない。
    ///
    /// # Errors
    ///
    /// `max_commit_polls`が[`MAX_COMMIT_POLLS_LIMIT`]を超える場合は、[`BoxClientError::InvalidMaxPolls`]を返す。
    pub fn max_commit_polls(mut self, max_commit_polls: u32) -> BoxClientResult<Self> {
        self.max_commit_polls =
            validate_max_polls("max_commit_polls", max_commit_polls, MAX_COMMIT_POLLS_LIMIT)?;
        Ok(self)
    }

    /// ファイルのダウンロードで、BOXがダウンロードの準備ができていないことを示す202を返した場合に、
    /// リクエストを再送する回数の上限を指定する。既定値は[`DEFAULT_MAX_DOWNLOAD_POLLS`]とする。
    ///
    /// `0`を指定すると、BOXが202を返した時点で再送せずにエラーを返す。
    /// 202の後の再送は、再試行設定の最大再試行回数に含めない。
    ///
    /// # Errors
    ///
    /// `max_download_polls`が[`MAX_DOWNLOAD_POLLS_LIMIT`]を超える場合は、[`BoxClientError::InvalidMaxPolls`]を返す。
    pub fn max_download_polls(mut self, max_download_polls: u32) -> BoxClientResult<Self> {
        self.max_download_polls = validate_max_polls(
            "max_download_polls",
            max_download_polls,
            MAX_DOWNLOAD_POLLS_LIMIT,
        )?;
        Ok(self)
    }

    /// `User-Agent`ヘッダーで送信するアプリケーションの名前を指定する。
    ///
    /// `User-Agent`ヘッダーは、指定した値の後ろに空白とこのクレートの名前とバージョンを続けた
    /// `<user_agent> box-client/<version>`とする。指定しない場合は、`box-client/<version>`とする。
    /// BOXのログで呼び出し元のアプリケーションを識別しやすくなるように、`my-app/1.0`のような形式で指定する。
    ///
    /// アクセストークンの取得とファイルのダウンロードを含む、すべてのリクエストで送信する。
    ///
    /// # Errors
    ///
    /// `user_agent`が空文字列または空白だけの場合は、[`BoxClientError::InvalidArgument`]を返す。
    /// `user_agent`に制御文字など、ヘッダーの値に使用できない文字が含まれる場合は、
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    pub fn user_agent(mut self, user_agent: &str) -> BoxClientResult<Self> {
        let user_agent = user_agent.trim();
        if user_agent.is_empty() {
            return Err(BoxClientError::InvalidArgument(
                "user agent must not be empty",
            ));
        }
        self.user_agent = HeaderValue::from_str(&format!("{user_agent} {CRATE_USER_AGENT}"))
            .map_err(BoxClientError::InvalidHeaderValue)?;
        Ok(self)
    }

    /// REST APIのベースURLを指定する。既定値は`https://api.box.com/2.0`とする。
    ///
    /// BOX以外のサーバーにアクセストークンを送信しないように注意する。
    /// 利用者のテストで、BOXの代わりにモックサーバーへ接続する場合などに使用する。
    /// ベースURLの後ろに、`folders/0`のようなAPIのパスを続けてリクエストを送信する。
    ///
    /// # Errors
    ///
    /// URLが[`BoxClientBuilder::token_url`]の条件を満たさない場合は、[`BoxClientError::InvalidEndpointUrl`]を返す。
    pub fn api_base_url(mut self, api_base_url: &str) -> BoxClientResult<Self> {
        self.api_base_url = Some(parse_endpoint_url("api_base_url", api_base_url)?);
        Ok(self)
    }

    /// アップロードのAPIのベースURLを指定する。既定値は`https://upload.box.com/api/2.0`とする。
    ///
    /// BOXは、ファイルのアップロードを、REST APIとは別のホストで受け付ける。
    /// 用途と注意点は、[`BoxClientBuilder::api_base_url`]と同じである。
    ///
    /// # Errors
    ///
    /// URLが[`BoxClientBuilder::token_url`]の条件を満たさない場合は、[`BoxClientError::InvalidEndpointUrl`]を返す。
    pub fn upload_base_url(mut self, upload_base_url: &str) -> BoxClientResult<Self> {
        self.upload_base_url = Some(parse_endpoint_url("upload_base_url", upload_base_url)?);
        Ok(self)
    }

    /// トークンエンドポイントのURLを指定する。既定値は`https://api.box.com/oauth2/token`とする。
    ///
    /// JWTアサーションの`aud`クレームは、このURLによらず、BOXのトークンエンドポイントのURLとする。
    /// BOXは、`aud`クレームがBOXのトークンエンドポイントのURLと一致することを求めるためである。
    /// したがって、BOXに転送するプロキシをトークンエンドポイントに指定しても、BOXはJWTアサーションを受け付ける。
    /// 用途と注意点は、[`BoxClientBuilder::api_base_url`]と同じである。
    ///
    /// アクセストークンやクライアントシークレットを、BOX以外のサーバーや暗号化しない通信で送信しないように、
    /// このメソッド、[`BoxClientBuilder::api_base_url`]および[`BoxClientBuilder::upload_base_url`]は、
    /// 次の条件を満たすURLだけを受け付ける。
    ///
    /// - スキームが`https`である。ただし、ホストが`localhost`またはループバックアドレスの場合は、`http`も受け付ける。
    /// - ユーザー名とパスワード、クエリおよびフラグメントを含まない。
    ///
    /// # Errors
    ///
    /// URLを解釈できない場合、または上記の条件を満たさない場合は、[`BoxClientError::InvalidEndpointUrl`]を返す。
    pub fn token_url(mut self, token_url: &str) -> BoxClientResult<Self> {
        self.token_url = Some(parse_endpoint_url("token_url", token_url)?.into());
        Ok(self)
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
            .user_agent(self.user_agent.clone())
            .timeout(self.timeout.request_timeout)
            .connect_timeout(self.timeout.connect_timeout)
            .build()?;
        let download_client = reqwest::Client::builder()
            .user_agent(self.user_agent)
            .read_timeout(self.timeout.download_read_timeout)
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
            download_client,
            api_base_url,
            upload_base_url,
            retry: self.retry,
            upload_concurrency: self.upload_concurrency,
            chunked_upload_threshold: self.chunked_upload_threshold,
            max_commit_polls: self.max_commit_polls,
            max_download_polls: self.max_download_polls,
            token_manager,
        })
    }
}

/// BOXクライアントがリクエストを送信するURLを解釈し、[`BoxClientBuilder::token_url`]に記載した条件を検証する。
///
/// # Arguments
///
/// - `name`: エラーに含める設定の名前
/// - `value`: URL
///
/// # Errors
///
/// URLを解釈できない場合、または条件を満たさない場合は、[`BoxClientError::InvalidEndpointUrl`]を返す。
fn parse_endpoint_url(name: &'static str, value: &str) -> BoxClientResult<Url> {
    let invalid = |reason: &'static str| BoxClientError::InvalidEndpointUrl {
        name,
        url: value.to_owned(),
        reason,
    };
    let url = Url::parse(value).map_err(|_| invalid("the url cannot be parsed"))?;
    if url.cannot_be_a_base() {
        return Err(invalid("the url cannot be a base url"));
    }
    // IPv6アドレスのホストは`[::1]`のように角括弧で囲まれているため、角括弧を除いてから解釈する。
    let is_loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    match url.scheme() {
        "https" => {}
        "http" if is_loopback => {}
        _ => {
            return Err(invalid(
                "the scheme must be https, or http for localhost and loopback addresses",
            ));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("the url must not contain a username or password"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("the url must not contain a query or fragment"));
    }
    Ok(url)
}

/// 202を受け取った場合に再送する回数の上限が、指定できる最大値以下であることを検証する。
///
/// # Errors
///
/// `max_polls`が`limit`を超える場合は、[`BoxClientError::InvalidMaxPolls`]を返す。
fn validate_max_polls(name: &'static str, max_polls: u32, limit: u32) -> BoxClientResult<u32> {
    if limit < max_polls {
        return Err(BoxClientError::InvalidMaxPolls {
            name,
            max_polls,
            limit,
        });
    }
    Ok(max_polls)
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
    ///
    /// ファイルのダウンロード以外のリクエストに適用する。
    /// ファイルのアップロードでは、1回のリクエストで送信するデータを送信し終えられる長さにする。
    pub request_timeout: Duration,

    /// 接続が確立するまでのタイムアウト
    ///
    /// ファイルのダウンロードを含む、すべてのリクエストに適用する。
    pub connect_timeout: Duration,

    /// ファイルのダウンロードで、データを受信しない状態が続いた場合のタイムアウト
    ///
    /// リクエストを送信してからレスポンスヘッダーを受信するまでの時間と、レスポンスボディのデータを受信する間隔に適用し、
    /// データを受信するたびに計測をやり直す。
    /// ダウンロードの時間はファイルサイズによって大きく変わるため、ダウンロードには
    /// [`BoxClientTimeout::request_timeout`]を適用せず、このタイムアウトで通信が止まったことを検出する。
    /// ダウンロード全体の時間を制限する場合は、呼び出し元が`tokio::time::timeout`などで制限する。
    pub download_read_timeout: Duration,
}

impl Default for BoxClientTimeout {
    /// レスポンスボディの受信完了までを30秒、接続の確立までを10秒、
    /// ダウンロードでデータを受信しない状態が30秒続いた場合にタイムアウトさせる設定を返す。
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            download_read_timeout: Duration::from_secs(30),
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

    /// ファイルのダウンロードで、データを受信しない状態が続いた場合のタイムアウトを設定する。
    #[must_use]
    pub fn with_download_read_timeout(mut self, download_read_timeout: Duration) -> Self {
        self.download_read_timeout = download_read_timeout;
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
        TOKEN_PATH, client_builder, create_client, create_client_with_timeout, mount_token,
        short_retry, test_config,
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

    /// 分割アップロードの並列数が最小値以上最大値以下の場合のみ、ビルダーに設定することを確認
    #[rstest]
    #[case::zero(0, false)]
    #[case::min(MIN_UPLOAD_CONCURRENCY, true)]
    #[case::default(DEFAULT_UPLOAD_CONCURRENCY, true)]
    #[case::max(MAX_UPLOAD_CONCURRENCY, true)]
    #[case::above_max(MAX_UPLOAD_CONCURRENCY + 1, false)]
    fn test_builder_validates_upload_concurrency(
        #[case] upload_concurrency: usize,
        #[case] valid: bool,
    ) {
        let result = BoxClient::builder(test_config()).upload_concurrency(upload_concurrency);

        if valid {
            assert!(
                matches!(&result, Ok(builder) if builder.upload_concurrency == upload_concurrency),
                "result: {result:?}"
            );
        } else {
            assert!(
                matches!(
                    &result,
                    Err(BoxClientError::InvalidUploadConcurrency { upload_concurrency: u, min, max })
                        if *u == upload_concurrency
                            && *min == MIN_UPLOAD_CONCURRENCY
                            && *max == MAX_UPLOAD_CONCURRENCY
                ),
                "result: {result:?}"
            );
        }
    }

    /// 分割アップロードに切り替えるファイルサイズが最小値以上最大値以下の場合のみ、ビルダーに設定することを確認
    #[rstest]
    #[case::below_min(MIN_CHUNKED_UPLOAD_THRESHOLD - 1, false)]
    #[case::min(MIN_CHUNKED_UPLOAD_THRESHOLD, true)]
    #[case::max(MAX_CHUNKED_UPLOAD_THRESHOLD, true)]
    #[case::above_max(MAX_CHUNKED_UPLOAD_THRESHOLD + 1, false)]
    fn test_builder_validates_chunked_upload_threshold(
        #[case] threshold: u64,
        #[case] valid: bool,
    ) {
        let result = BoxClient::builder(test_config()).chunked_upload_threshold(threshold);

        if valid {
            assert!(
                matches!(&result, Ok(builder) if builder.chunked_upload_threshold == threshold),
                "result: {result:?}"
            );
        } else {
            assert!(
                matches!(
                    &result,
                    Err(BoxClientError::InvalidChunkedUploadThreshold {
                        chunked_upload_threshold: t,
                        min,
                        max,
                    }) if *t == threshold
                        && *min == MIN_CHUNKED_UPLOAD_THRESHOLD
                        && *max == MAX_CHUNKED_UPLOAD_THRESHOLD
                ),
                "result: {result:?}"
            );
        }
    }

    /// 分割アップロードに切り替えるファイルサイズを指定しない場合は既定値を、指定した場合はその値をBOXクライアントが返すことを確認
    #[rstest]
    #[case::default(None, DEFAULT_CHUNKED_UPLOAD_THRESHOLD)]
    #[case::specified(Some(MIN_CHUNKED_UPLOAD_THRESHOLD), MIN_CHUNKED_UPLOAD_THRESHOLD)]
    #[tokio::test]
    async fn test_build_sets_chunked_upload_threshold(
        #[case] specified: Option<u64>,
        #[case] expected: u64,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let mut builder = client_builder(&server, short_retry(0)?)?;
        if let Some(threshold) = specified {
            builder = builder.chunked_upload_threshold(threshold)?;
        }

        let client = builder.build().await?;

        assert_eq!(client.chunked_upload_threshold(), expected);

        Ok(())
    }

    /// 202を受け取った場合に再送する回数の上限が、指定できる最大値以下の場合のみ、ビルダーに設定することを確認
    #[rstest]
    #[case::zero(0, true)]
    #[case::default_commit(DEFAULT_MAX_COMMIT_POLLS, true)]
    #[case::default_download(DEFAULT_MAX_DOWNLOAD_POLLS, true)]
    #[case::limit(MAX_COMMIT_POLLS_LIMIT, true)]
    #[case::above_limit(MAX_COMMIT_POLLS_LIMIT + 1, false)]
    fn test_builder_validates_max_polls(#[case] max_polls: u32, #[case] valid: bool) {
        // 2つの上限は同じ最大値を持つため、同じ値で両方を検証する。
        assert_eq!(MAX_COMMIT_POLLS_LIMIT, MAX_DOWNLOAD_POLLS_LIMIT);
        let commit = BoxClient::builder(test_config()).max_commit_polls(max_polls);
        let download = BoxClient::builder(test_config()).max_download_polls(max_polls);

        if valid {
            assert!(
                matches!(&commit, Ok(builder) if builder.max_commit_polls == max_polls),
                "commit: {commit:?}"
            );
            assert!(
                matches!(&download, Ok(builder) if builder.max_download_polls == max_polls),
                "download: {download:?}"
            );
        } else {
            for (result, expected_name) in [
                (commit.map(|_| ()), "max_commit_polls"),
                (download.map(|_| ()), "max_download_polls"),
            ] {
                assert!(
                    matches!(
                        &result,
                        Err(BoxClientError::InvalidMaxPolls { name, max_polls: m, limit })
                            if *name == expected_name
                                && *m == max_polls
                                && *limit == MAX_COMMIT_POLLS_LIMIT
                    ),
                    "result: {result:?}"
                );
            }
        }
    }

    /// 202を受け取った場合に再送する回数の上限を指定しない場合は既定値を、指定した場合はその値をBOXクライアントが返すことを確認
    #[rstest]
    #[case::default(None, DEFAULT_MAX_COMMIT_POLLS, DEFAULT_MAX_DOWNLOAD_POLLS)]
    #[case::specified(Some((3, 4)), 3, 4)]
    #[tokio::test]
    async fn test_build_sets_max_polls(
        #[case] specified: Option<(u32, u32)>,
        #[case] expected_commit: u32,
        #[case] expected_download: u32,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let mut builder = client_builder(&server, short_retry(0)?)?;
        if let Some((commit, download)) = specified {
            builder = builder
                .max_commit_polls(commit)?
                .max_download_polls(download)?;
        }

        let client = builder.build().await?;

        assert_eq!(client.max_commit_polls(), expected_commit);
        assert_eq!(client.max_download_polls(), expected_download);

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
            .api_base_url(&format!("{}/2.0", server.uri()))?
            .token_url(&format!("{}{TOKEN_PATH}", server.uri()))?
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
            .api_base_url(api_base_url.as_str())?
            .token_url(&format!("{}{TOKEN_PATH}", token_server.uri()))?
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
            .api_base_url(api_base_url.as_str())?
            .token_url(&format!("{}{TOKEN_PATH}", server.uri()))?
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

    /// アクセストークンの取得、REST APIの呼び出しおよびファイルのダウンロードで、`User-Agent`ヘッダーを送信することを確認
    ///
    /// アプリケーションの名前を指定した場合は、その後ろにこのクレートの名前とバージョンを続けることも確認する。
    #[rstest]
    #[case::default(None, CRATE_USER_AGENT.to_owned())]
    #[case::specified(Some(" my-app/1.0 "), format!("my-app/1.0 {CRATE_USER_AGENT}"))]
    #[tokio::test]
    async fn test_send_user_agent(
        #[case] user_agent: Option<&str>,
        #[case] expected: String,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        Mock::given(path(TOKEN_PATH))
            .and(header("user-agent", expected.as_str()))
            .respond_with(crate::test_support::token_response())
            .mount(&server)
            .await;
        Mock::given(path(API_PATH))
            .and(header("user-agent", expected.as_str()))
            .respond_with(ok_response())
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/2.0/files/11/content"))
            .and(header("user-agent", expected.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"content".as_slice()))
            .expect(1)
            .mount(&server)
            .await;
        let mut builder = client_builder(&server, short_retry(0)?)?;
        if let Some(user_agent) = user_agent {
            builder = builder.user_agent(user_agent)?;
        }
        // `User-Agent`ヘッダーが一致しない場合は、トークンエンドポイントが404を返すため、構築に失敗する。
        let client = builder.build().await?;
        let dir = tempfile::tempdir()?;

        let _: serde_json::Value = client.send_json(request("GET")?).await?;
        client
            .download_file(
                &crate::FileId::new("11"),
                dir.path().join("content.txt"),
                &crate::DownloadFileOptions::default(),
            )
            .await?;

        Ok(())
    }

    /// 空のアプリケーションの名前、またはヘッダーの値に使用できない文字を含むアプリケーションの名前を拒否することを確認
    #[rstest]
    #[case::empty("", "InvalidArgument")]
    #[case::whitespace("  ", "InvalidArgument")]
    #[case::control_character("my-app\n1.0", "InvalidHeaderValue")]
    fn test_builder_rejects_invalid_user_agent(#[case] user_agent: &str, #[case] expected: &str) {
        let result = BoxClient::builder(test_config()).user_agent(user_agent);

        let actual = match &result {
            Err(BoxClientError::InvalidArgument(_)) => "InvalidArgument",
            Err(BoxClientError::InvalidHeaderValue(_)) => "InvalidHeaderValue",
            _ => "other",
        };
        assert_eq!(actual, expected, "result: {result:?}");
    }

    /// httpsのURLと、ローカルホストまたはループバックアドレスのhttpのURLだけを受け付けることを確認
    #[rstest]
    #[case::https("https://api.box.com/2.0", true)]
    #[case::https_with_port("https://box.example.com:8443/api", true)]
    #[case::http_localhost("http://localhost:8080/2.0", true)]
    #[case::http_localhost_uppercase("http://LOCALHOST/2.0", true)]
    #[case::http_ipv4_loopback("http://127.0.0.1:8080/2.0", true)]
    #[case::http_ipv4_loopback_range("http://127.1.2.3/2.0", true)]
    #[case::http_ipv6_loopback("http://[::1]:8080/2.0", true)]
    #[case::http_remote("http://api.box.com/2.0", false)]
    #[case::http_localhost_subdomain("http://localhost.example.com/2.0", false)]
    #[case::other_scheme("ftp://api.box.com/2.0", false)]
    #[case::not_base("mailto:user@example.com", false)]
    #[case::credentials("https://user:password@api.box.com/2.0", false)]
    #[case::username("https://user@api.box.com/2.0", false)]
    #[case::query("https://api.box.com/2.0?key=value", false)]
    #[case::fragment("https://api.box.com/2.0#top", false)]
    #[case::unparsable("not a url", false)]
    #[case::relative("/2.0", false)]
    fn test_builder_validates_endpoint_urls(#[case] url: &str, #[case] valid: bool) {
        type Setter = fn(BoxClientBuilder, &str) -> BoxClientResult<BoxClientBuilder>;
        let setters: [(&str, Setter); 3] = [
            ("api_base_url", BoxClientBuilder::api_base_url),
            ("upload_base_url", BoxClientBuilder::upload_base_url),
            ("token_url", BoxClientBuilder::token_url),
        ];

        for (expected_name, setter) in setters {
            let result = setter(BoxClient::builder(test_config()), url);

            if valid {
                assert!(result.is_ok(), "{expected_name}: {:?}", result.err());
            } else {
                assert!(
                    matches!(
                        &result,
                        Err(BoxClientError::InvalidEndpointUrl { name, url: u, .. })
                            if *name == expected_name && u == url
                    ),
                    "{expected_name}: {:?}",
                    result.err()
                );
            }
        }
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
