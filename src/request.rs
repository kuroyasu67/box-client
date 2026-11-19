use std::borrow::Cow;

use bytes::Bytes;
use reqwest::{
    Method, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderName, HeaderValue, IF_MATCH},
};
use serde::Serialize;

use crate::{
    errors::{BoxClientError, BoxClientResult},
    retry::Idempotency,
};

/// BOXのREST APIへのリクエスト
///
/// 再試行のたびに同じリクエストを組み立てられるように、送信する内容を保持する。
///
/// ```no_run
/// # async fn example(client: &box_client::BoxClient) -> box_client::BoxClientResult<()> {
/// use box_client::ApiRequest;
///
/// let folder: serde_json::Value = client
///     .send_json(ApiRequest::get(["folders", "0"]).query("fields", "id,name"))
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ApiRequest<'a> {
    /// HTTPメソッド
    pub(crate) method: Method,

    /// ベースURLに続けるパス要素
    path: Vec<&'a str>,

    /// クエリパラメーター
    query: Vec<(&'a str, Cow<'a, str>)>,

    /// リクエストヘッダー
    pub(crate) headers: Vec<(HeaderName, HeaderValue)>,

    /// リクエストボディ
    pub(crate) body: Option<RequestBody>,

    /// 冪等性
    pub(crate) idempotency: Idempotency,

    /// リクエストを送信するホスト
    pub(crate) host: ApiHost,

    /// リクエストを送信するHTTPクライアントの種類
    pub(crate) client: HttpClientKind,

    /// ベースURLとパス要素の代わりに使用する、BOXが返した絶対URL
    ///
    /// 指定した場合は、パス要素とクエリパラメーターを使用せず、アクセストークンも送信しない。
    absolute_url: Option<Url>,
}

/// リクエストボディ
///
/// 再試行のたびにシリアライズし直したり、ボディを複製したりしないように、ボディを参照カウントで共有する。
#[derive(Debug, Clone)]
pub(crate) struct RequestBody {
    /// `Content-Type`ヘッダーの値
    pub(crate) content_type: HeaderValue,

    /// ボディのバイト列
    pub(crate) bytes: Bytes,
}

/// リクエストを送信するHTTPクライアントの種類
///
/// HTTPクライアントによって、適用するタイムアウトが異なる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpClientKind {
    /// リクエスト全体に[`TimeoutConfig::request_timeout`](crate::TimeoutConfig::request_timeout)を適用するHTTPクライアント
    Default,

    /// 無通信の時間に[`TimeoutConfig::download_read_timeout`](crate::TimeoutConfig::download_read_timeout)を適用する、
    /// ファイルのダウンロード用のHTTPクライアント
    Download,
}

/// リクエストを送信するホスト
///
/// BOXは、ファイルのアップロードを、REST APIとは別のホストで受け付ける。
/// 各ホストのベースURLは、[`BoxClientBuilder`](crate::BoxClientBuilder)で変更できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiHost {
    /// REST APIのホスト（既定値は`api.box.com`）
    Api,

    /// アップロードのホスト（既定値は`upload.box.com`）
    Upload,
}

impl<'a> ApiRequest<'a> {
    /// HTTPメソッドとパス要素からリクエストを作成する。
    ///
    /// このメソッドは、冪等性をHTTPメソッドから決定する。
    /// BOXクライアントは、パス要素をURLエンコードしてから結合するため、IDに`/`などが含まれていても、意図したエンドポイントを呼び出す。
    ///
    /// # Arguments
    ///
    /// - `method`: HTTPメソッド
    /// - `path`: `["folders", folder_id]`のような、ベースURLに続けるパス要素
    pub fn new(method: Method, path: impl IntoIterator<Item = &'a str>) -> Self {
        let idempotency = Idempotency::from_method(&method);
        Self {
            method,
            path: path.into_iter().collect(),
            query: Vec::new(),
            headers: Vec::new(),
            body: None,
            idempotency,
            host: ApiHost::Api,
            client: HttpClientKind::Default,
            absolute_url: None,
        }
    }

    /// BOXが返した絶対URLへ、アクセストークンを送信せずにGETリクエストを送信するリクエストを作成する。
    ///
    /// ZIPダウンロードの`download_url`のように、BOXが認証を求めないURLに使用する。
    /// アクセストークンを送信しないため、BOX以外のホストのURLでもアクセストークンは漏れない。
    pub(crate) fn unauthenticated_get(url: Url) -> Self {
        Self {
            absolute_url: Some(url),
            ..Self::new(Method::GET, [])
        }
    }

    /// アクセストークンを`Authorization`ヘッダーで送信するかどうかを返す。
    pub(crate) fn is_authenticated(&self) -> bool {
        self.absolute_url.is_none()
    }

    /// GETリクエストを作成する。
    pub fn get(path: impl IntoIterator<Item = &'a str>) -> Self {
        Self::new(Method::GET, path)
    }

    /// POSTリクエストを作成する。
    pub fn post(path: impl IntoIterator<Item = &'a str>) -> Self {
        Self::new(Method::POST, path)
    }

    /// PUTリクエストを作成する。
    pub fn put(path: impl IntoIterator<Item = &'a str>) -> Self {
        Self::new(Method::PUT, path)
    }

    /// DELETEリクエストを作成する。
    pub fn delete(path: impl IntoIterator<Item = &'a str>) -> Self {
        Self::new(Method::DELETE, path)
    }

    /// クエリパラメーターを追加する。
    #[must_use]
    pub fn query(mut self, key: &'a str, value: impl Into<Cow<'a, str>>) -> Self {
        self.query.push((key, value.into()));
        self
    }

    /// リクエストヘッダーを追加する。
    ///
    /// # Errors
    ///
    /// `Authorization`ヘッダーまたは`Content-Type`ヘッダーを指定した場合は、[`BoxClientError::InvalidArgument`]を返す。
    /// BOXクライアントは、`Authorization`ヘッダーをアクセストークンから、`Content-Type`ヘッダーをリクエストボディから設定する。
    /// したがって、このメソッドは、ヘッダーを重複して送信しないように、これらのヘッダーを拒否する。
    /// ヘッダーの値に制御文字など使用できない文字が含まれる場合は、
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    pub fn header(mut self, name: HeaderName, value: &str) -> BoxClientResult<Self> {
        if name == AUTHORIZATION {
            return Err(BoxClientError::InvalidArgument(
                "authorization header is set by the client",
            ));
        }
        if name == CONTENT_TYPE {
            return Err(BoxClientError::InvalidArgument(
                "content-type header is set from the request body",
            ));
        }
        let value = HeaderValue::from_str(value).map_err(BoxClientError::InvalidHeaderValue)?;
        self.headers.push((name, value));
        Ok(self)
    }

    /// `ETag`を指定した場合は、`If-Match`ヘッダーを追加する。
    ///
    /// BOXは、項目が`ETag`の取得後に変更されている場合に、リクエストを処理せずに412を返す。
    ///
    /// # Errors
    ///
    /// `etag`に制御文字など使用できない文字が含まれる場合は、[`BoxClientError::InvalidHeaderValue`]を返す。
    pub(crate) fn if_match(self, etag: Option<&str>) -> BoxClientResult<Self> {
        match etag {
            Some(etag) => self.header(IF_MATCH, etag),
            None => Ok(self),
        }
    }

    /// リクエストボディをJSONにシリアライズして設定する。
    ///
    /// # Errors
    ///
    /// JSONにシリアライズできない場合は、[`BoxClientError::SerializeBody`]を返す。
    pub fn json<B: Serialize + ?Sized>(self, body: &B) -> BoxClientResult<Self> {
        let body = serde_json::to_vec(body).map_err(BoxClientError::SerializeBody)?;
        Ok(self.body(
            HeaderValue::from_static("application/json"),
            Bytes::from(body),
        ))
    }

    /// `Content-Type`ヘッダーの値とバイト列で、リクエストボディを設定する。
    #[must_use]
    pub(crate) fn body(mut self, content_type: HeaderValue, bytes: Bytes) -> Self {
        self.body = Some(RequestBody {
            content_type,
            bytes,
        });
        self
    }

    /// リクエストを、REST APIのホストではなく、アップロードのホストへ送信する。
    #[must_use]
    pub(crate) fn upload_host(mut self) -> Self {
        self.host = ApiHost::Upload;
        self
    }

    /// リクエストを、ファイルのダウンロード用のHTTPクライアントで送信する。
    #[must_use]
    pub(crate) fn download_client(mut self) -> Self {
        self.client = HttpClientKind::Download;
        self
    }

    /// HTTPメソッドから決定した冪等性を上書きする。
    ///
    /// BOXが重複を検出して409を返すPOSTなど、再送しても安全だとわかっているAPIで
    /// [`Idempotency::Idempotent`]を指定し、5xxおよびタイムアウトの後にも再試行させる。
    #[must_use]
    pub fn idempotency(mut self, idempotency: Idempotency) -> Self {
        self.idempotency = idempotency;
        self
    }

    /// ベースURLにパス要素とクエリパラメーターを結合したURLを返す。
    ///
    /// 絶対URLを指定したリクエストでは、その絶対URLを返す。
    ///
    /// # Errors
    ///
    /// パス要素が空文字列、`.`または`..`の場合は、[`BoxClientError::InvalidPathSegment`]を返す。
    /// ベースURLにパスを結合できない場合は、[`BoxClientError::InvalidBaseUrl`]を返す。
    pub(crate) fn url(&self, base_url: &Url) -> BoxClientResult<Url> {
        if let Some(url) = &self.absolute_url {
            return Ok(url.clone());
        }
        if let Some(segment) = self
            .path
            .iter()
            .find(|segment| matches!(**segment, "" | "." | ".."))
        {
            return Err(BoxClientError::InvalidPathSegment((*segment).to_owned()));
        }

        let mut url = base_url.clone();
        url.path_segments_mut()
            .map_err(|()| BoxClientError::InvalidBaseUrl(base_url.to_string()))?
            .pop_if_empty()
            .extend(&self.path);
        if !self.query.is_empty() {
            url.query_pairs_mut().extend_pairs(&self.query);
        }

        Ok(url)
    }
}

#[cfg(test)]
mod test_api_request {
    use rstest::rstest;

    use super::*;

    /// テスト用のベースURL
    const BASE_URL: &str = "https://api.box.com/2.0";

    /// パス要素をURLエンコードしてベースURLに結合することを確認
    #[rstest]
    #[case::single(vec!["users"], "https://api.box.com/2.0/users")]
    #[case::with_id(vec!["folders", "123"], "https://api.box.com/2.0/folders/123")]
    // パス要素に含まれる/および?をエンコードし、パス要素の一部として扱う。
    #[case::slash_in_id(vec!["folders", "1/items"], "https://api.box.com/2.0/folders/1%2Fitems")]
    #[case::query_in_id(vec!["folders", "1?a=b"], "https://api.box.com/2.0/folders/1%3Fa=b")]
    #[case::percent_in_id(vec!["folders", "%2E%2E"], "https://api.box.com/2.0/folders/%252E%252E")]
    fn test_url(#[case] path: Vec<&str>, #[case] expected: &str) -> anyhow::Result<()> {
        let request = ApiRequest::get(path);

        let url = request.url(&Url::parse(BASE_URL)?)?;

        assert_eq!(url.as_str(), expected);

        Ok(())
    }

    /// ベースURLの末尾に/がある場合でも、/を重ねずに結合することを確認
    #[test]
    fn test_url_with_trailing_slash_base() -> anyhow::Result<()> {
        let request = ApiRequest::get(["users", "me"]);

        let url = request.url(&Url::parse("https://api.box.com/2.0/")?)?;

        assert_eq!(url.as_str(), "https://api.box.com/2.0/users/me");

        Ok(())
    }

    /// クエリパラメーターをURLエンコードして付与することを確認
    #[test]
    fn test_url_with_query() -> anyhow::Result<()> {
        let request = ApiRequest::get(["folders", "0", "items"])
            .query("fields", "id,name")
            .query("limit", 100.to_string())
            .query("q", "a&b");

        let url = request.url(&Url::parse(BASE_URL)?)?;

        assert_eq!(
            url.as_str(),
            "https://api.box.com/2.0/folders/0/items?fields=id%2Cname&limit=100&q=a%26b"
        );

        Ok(())
    }

    /// パス要素が空文字列、.または..の場合は、パス要素の不正エラーを返すことを確認
    #[rstest]
    #[case::empty("")]
    #[case::current(".")]
    #[case::parent("..")]
    fn test_url_invalid_segment(#[case] segment: &str) -> anyhow::Result<()> {
        let request = ApiRequest::get(["folders", segment]);

        let result = request.url(&Url::parse(BASE_URL)?);

        assert!(
            matches!(&result, Err(BoxClientError::InvalidPathSegment(s)) if s == segment),
            "result: {result:?}"
        );

        Ok(())
    }

    /// パスを結合できないベースURLの場合は、ベースURLの不正エラーを返すことを確認
    #[test]
    fn test_url_cannot_be_a_base() -> anyhow::Result<()> {
        let request = ApiRequest::get(["users"]);

        let result = request.url(&Url::parse("mailto:user@example.com")?);

        assert!(
            matches!(result, Err(BoxClientError::InvalidBaseUrl(_))),
            "result: {result:?}"
        );

        Ok(())
    }

    /// 冪等性をHTTPメソッドから決定し、明示的に上書きできることを確認
    #[rstest]
    #[case::get(ApiRequest::get(["users"]), Idempotency::Idempotent)]
    #[case::put(ApiRequest::put(["users"]), Idempotency::Idempotent)]
    #[case::delete(ApiRequest::delete(["users"]), Idempotency::Idempotent)]
    #[case::post(ApiRequest::post(["users"]), Idempotency::NonIdempotent)]
    #[case::post_overridden(
        ApiRequest::post(["users"]).idempotency(Idempotency::Idempotent),
        Idempotency::Idempotent
    )]
    fn test_idempotency(#[case] request: ApiRequest<'_>, #[case] expected: Idempotency) {
        assert_eq!(request.idempotency, expected);
    }

    /// リクエストヘッダーを追加した順に保持することを確認
    #[test]
    fn test_header() -> anyhow::Result<()> {
        let request = ApiRequest::delete(["folders", "123"])
            .header(reqwest::header::IF_MATCH, "1")?
            .header(HeaderName::from_static("boxapi"), "shared_link=x")?;

        assert_eq!(
            request.headers,
            vec![
                (reqwest::header::IF_MATCH, HeaderValue::from_static("1")),
                (
                    HeaderName::from_static("boxapi"),
                    HeaderValue::from_static("shared_link=x")
                ),
            ]
        );

        Ok(())
    }

    /// クライアントが設定するAuthorizationヘッダーまたはContent-Typeヘッダーを指定した場合は、引数の不正エラーを返すことを確認
    #[rstest]
    #[case::authorization(AUTHORIZATION, "Bearer other")]
    #[case::content_type(CONTENT_TYPE, "text/plain")]
    fn test_header_rejects_client_managed_headers(#[case] name: HeaderName, #[case] value: &str) {
        let result = ApiRequest::get(["users"]).header(name, value);

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
    }

    /// ヘッダーの値に使用できない文字が含まれる場合は、ヘッダーの値の不正エラーを返すことを確認
    #[rstest]
    #[case::newline("1\r\nX-Injected: 1")]
    #[case::null("1\0")]
    fn test_header_invalid_value(#[case] value: &str) {
        let result = ApiRequest::get(["users"]).header(reqwest::header::IF_MATCH, value);

        assert!(
            matches!(result, Err(BoxClientError::InvalidHeaderValue(_))),
            "result: {result:?}"
        );
    }

    /// `ETag`を指定した場合だけ、`If-Match`ヘッダーを追加することを確認
    #[rstest]
    #[case::some(Some("1"), vec![(IF_MATCH, HeaderValue::from_static("1"))])]
    #[case::none(None, vec![])]
    fn test_if_match(
        #[case] etag: Option<&str>,
        #[case] expected: Vec<(HeaderName, HeaderValue)>,
    ) -> anyhow::Result<()> {
        let request = ApiRequest::put(["files", "1"]).if_match(etag)?;

        assert_eq!(request.headers, expected);

        Ok(())
    }

    /// リクエストボディをJSONにシリアライズして保持することを確認
    #[test]
    fn test_json() -> anyhow::Result<()> {
        let request = ApiRequest::post(["folders"]).json(&serde_json::json!({"name": "new"}))?;

        let body = request.body.as_ref();
        assert_eq!(
            body.map(|b| b.content_type.as_bytes()),
            Some(b"application/json".as_slice())
        );
        assert_eq!(
            body.map(|b| b.bytes.as_ref()),
            Some(br#"{"name":"new"}"#.as_slice())
        );

        Ok(())
    }
}
