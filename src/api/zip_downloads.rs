//! ZIPダウンロードに関するAPI
//!
//! BOXは、複数のファイルおよびフォルダを1つのZIPファイルにまとめてダウンロードする機能を、次の3つのAPIで提供する。
//!
//! 1. ZIPダウンロードを作成する（`POST /zip_downloads`）。BOXは、ZIPファイルのダウンロード用のURLと、
//!    ダウンロードの状態を確認するURLを返す。
//! 2. ダウンロード用のURLからZIPファイルをダウンロードする（`GET /zip_downloads/{zip_download_id}/content`）。
//!    BOXは、このURLを`dl.boxcloud.com`などのダウンロード用のホストで提供し、認証を求めない。
//! 3. ダウンロードの状態を取得する（`GET /zip_downloads/{zip_download_id}/status`）。
//!
//! ダウンロード用のURLは、ダウンロードを始めるまで数秒しか有効でなく、一度しか使用できない。
//! したがって、ZIPダウンロードを作成したら、すぐにダウンロードする。
//! [`BoxClient::download_zip`]は、これらの3つのAPIを順に呼び出す。

use std::path::Path;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, FileId, FolderId, Idempotency,
    api::downloads::{TempFile, write_response},
    client::has_secure_scheme,
    request::ApiRequest,
};

/// 1つのZIPダウンロードに指定できる項目の最大数
///
/// BOXは、1つのZIPファイルに含めるファイルを10,000個以下に制限する。
/// フォルダを指定した場合は、フォルダ内のファイルも数えるため、項目数がこの値以下でも、BOXが400を返す場合がある。
pub const MAX_ZIP_DOWNLOAD_ITEMS: usize = 10_000;

/// ZIPファイルに含めるファイルまたはフォルダ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ZipDownloadItem<'a> {
    /// ファイル
    File(&'a FileId),

    /// フォルダ
    ///
    /// フォルダ内のすべての項目をZIPファイルに含める。ルートフォルダは指定できない。
    Folder(&'a FolderId),
}

/// ZIPダウンロードの作成のオプション
///
/// # Examples
///
/// ```
/// use box_client::CreateZipDownloadOptions;
///
/// let options = CreateZipDownloadOptions::default().with_download_file_name("January Financials");
/// assert_eq!(options.download_file_name, Some("January Financials"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreateZipDownloadOptions<'a> {
    /// ZIPファイルの名前
    ///
    /// BOXは、指定した名前の末尾に`.zip`を付ける。`None`の場合は、BOXが名前を決める。
    pub download_file_name: Option<&'a str>,
}

impl<'a> CreateZipDownloadOptions<'a> {
    /// ZIPファイルの名前を設定する。
    #[must_use]
    pub fn with_download_file_name(mut self, download_file_name: &'a str) -> Self {
        self.download_file_name = Some(download_file_name);
        self
    }
}

/// ZIPダウンロード
///
/// BOXクライアントは、`download_url`をデバッグ出力に含めない。
/// `download_url`は認証なしでZIPファイルをダウンロードできるURLであるため、ログに残さないようにする。
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct ZipDownload {
    /// ZIPファイルのダウンロード用のURL
    ///
    /// ダウンロードを始めるまでは数秒（`expires_at`まで）しか有効でない。一度しか使用できない。
    pub download_url: String,

    /// ダウンロードの状態を確認するURL
    ///
    /// ダウンロードを始めてから12時間有効である。ダウンロードを始める前は、BOXは404を返す。
    pub status_url: String,

    /// `download_url`および`status_url`の有効期限
    ///
    /// ダウンロードを始めると、`download_url`はダウンロードの間、`status_url`は12時間、有効期限が延びる。
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,

    /// 名前が重複したため、ZIPファイル内で名前を変更した項目
    ///
    /// 内側の`Vec`が1つの重複を表し、名前が重複したすべての項目を含む。BOXは、重複したすべての項目の名前を変更する。
    #[serde(default)]
    pub name_conflicts: Vec<Vec<ZipDownloadNameConflict>>,
}

impl std::fmt::Debug for ZipDownload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipDownload")
            .field("download_url", &"[REDACTED]")
            .field("status_url", &self.status_url)
            .field("expires_at", &self.expires_at)
            .field("name_conflicts", &self.name_conflicts)
            .finish()
    }
}

impl ZipDownload {
    /// `status_url`から、ZIPダウンロードのIDを取り出す。
    ///
    /// BOXのガイドは、`status_url`の`zip_downloads`と`status`の間のパス要素をIDとして案内している。
    ///
    /// # Errors
    ///
    /// `status_url`を解釈できない場合、またはIDを含まない場合は、[`BoxClientError::UnexpectedResponse`]を返す。
    fn id(&self) -> BoxClientResult<String> {
        let invalid = || BoxClientError::UnexpectedResponse("status_url has no zip download id");
        let url = Url::parse(&self.status_url).map_err(|_| invalid())?;
        let mut segments = url.path_segments().ok_or_else(invalid)?;
        segments
            .by_ref()
            .find(|segment| *segment == "zip_downloads")
            .ok_or_else(invalid)?;
        match (segments.next(), segments.next()) {
            (Some(id), Some("status")) if !id.is_empty() => Ok(id.to_owned()),
            _ => Err(invalid()),
        }
    }
}

/// ZIPファイル内で名前を変更した項目
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct ZipDownloadNameConflict {
    /// 項目のID
    pub id: String,

    /// 項目の種類
    #[serde(rename = "type")]
    pub item_type: ZipDownloadItemType,

    /// BOX上の項目の名前
    pub original_name: String,

    /// ZIPファイル内の項目の名前
    pub download_name: String,
}

/// ZIPファイル内の項目の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ZipDownloadItemType {
    /// ファイル
    File,

    /// フォルダ
    Folder,

    /// BOXクライアントが知らない種類
    #[serde(other)]
    Unknown,
}

/// ZIPダウンロードの状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct ZipDownloadStatus {
    /// ZIPファイルに含めるファイルの数
    pub total_file_count: Option<u64>,

    /// ダウンロードしたファイルの数
    pub downloaded_file_count: Option<u64>,

    /// ダウンロードできずにスキップしたファイルの数
    ///
    /// ZIPダウンロードの作成後に権限が変わった場合などに、BOXはファイルをスキップする。
    pub skipped_file_count: Option<u64>,

    /// ダウンロードできずにスキップしたフォルダの数
    pub skipped_folder_count: Option<u64>,

    /// ダウンロードの状態
    pub state: Option<ZipDownloadState>,
}

/// ZIPダウンロードの進み具合
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ZipDownloadState {
    /// ダウンロード中
    InProgress,

    /// 失敗
    Failed,

    /// 成功
    Succeeded,

    /// BOXクライアントが知らない状態
    #[serde(other)]
    Unknown,
}

/// [`BoxClient::download_zip`]の結果
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ZipDownloadSummary {
    /// 保存したZIPファイルのバイト数
    pub size: u64,

    /// 名前が重複したため、ZIPファイル内で名前を変更した項目
    pub name_conflicts: Vec<Vec<ZipDownloadNameConflict>>,

    /// ダウンロードを終えた後の状態
    ///
    /// スキップしたファイルおよびフォルダの数を確認できる。
    pub status: ZipDownloadStatus,
}

/// ZIPダウンロードの作成リクエストのボディ
#[derive(Debug, Serialize)]
struct CreateZipDownloadBody<'a> {
    /// ZIPファイルに含める項目
    items: Vec<ZipDownloadItemBody<'a>>,

    /// ZIPファイルの名前
    #[serde(skip_serializing_if = "Option::is_none")]
    download_file_name: Option<&'a str>,
}

/// ZIPファイルに含める項目のリクエストボディ
#[derive(Debug, Serialize)]
struct ZipDownloadItemBody<'a> {
    /// 項目の種類
    #[serde(rename = "type")]
    item_type: &'static str,

    /// 項目のID
    id: &'a str,
}

impl<'a> CreateZipDownloadBody<'a> {
    /// 項目とオプションからボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// 項目が空の場合、項目が[`MAX_ZIP_DOWNLOAD_ITEMS`]個より多い場合、またはルートフォルダを含む場合は、
    /// [`BoxClientError::InvalidArgument`]を返す。
    fn new(
        items: &[ZipDownloadItem<'a>],
        options: &CreateZipDownloadOptions<'a>,
    ) -> BoxClientResult<Self> {
        if items.is_empty() {
            return Err(BoxClientError::InvalidArgument(
                "items for a zip download must not be empty",
            ));
        }
        if MAX_ZIP_DOWNLOAD_ITEMS < items.len() {
            return Err(BoxClientError::InvalidArgument(
                "items for a zip download must not exceed 10000",
            ));
        }
        let items = items
            .iter()
            .map(|item| match *item {
                ZipDownloadItem::File(file_id) => Ok(ZipDownloadItemBody {
                    item_type: "file",
                    id: file_id.as_ref(),
                }),
                ZipDownloadItem::Folder(folder_id) if folder_id.is_root() => Err(
                    BoxClientError::InvalidArgument("the root folder cannot be zip downloaded"),
                ),
                ZipDownloadItem::Folder(folder_id) => Ok(ZipDownloadItemBody {
                    item_type: "folder",
                    id: folder_id.as_ref(),
                }),
            })
            .collect::<BoxClientResult<Vec<_>>>()?;

        Ok(Self {
            items,
            download_file_name: options.download_file_name,
        })
    }
}

impl BoxClient {
    /// ZIPダウンロードを作成する。
    ///
    /// BOXは、ZIPファイルをこの時点では作成せず、ダウンロード用のURLと状態を確認するURLを返す。
    /// ダウンロード用のURLは数秒しか有効でないため、作成したらすぐに[`BoxClient::download_zip_content`]でダウンロードする。
    /// 作成からダウンロードまでをまとめて行う場合は、[`BoxClient::download_zip`]を使用する。
    ///
    /// HTTPメソッドはPOSTだが、冪等なリクエストとして送信する。
    /// ZIPダウンロードを重複して作成しても、BOXに項目は作成されず、使用しなかったZIPダウンロードは期限が切れて無効になるためである。
    ///
    /// # Arguments
    ///
    /// - `items`: ZIPファイルに含めるファイルおよびフォルダ。1個以上[`MAX_ZIP_DOWNLOAD_ITEMS`]個以下で指定する。
    /// - `options`: ZIPファイルの名前のオプション
    ///
    /// # Errors
    ///
    /// `items`が空の場合、`items`が[`MAX_ZIP_DOWNLOAD_ITEMS`]個より多い場合、または`items`にルートフォルダを含む場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 400: 項目が不正である。または、ZIPファイルに含めるファイルが10,000個を超える
    ///   （`zip_download_file_count_exceeded_limit`）、もしくは合計のサイズが上限を超える
    ///   （`zip_download_pre_compressed_bytes_exceeded_limit`）。後者の場合は、項目を分けて作成する。
    /// - 403: 項目にアクセスする権限がない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn create_zip_download(
        &self,
        items: &[ZipDownloadItem<'_>],
        options: &CreateZipDownloadOptions<'_>,
    ) -> BoxClientResult<ZipDownload> {
        let body = CreateZipDownloadBody::new(items, options)?;
        let request = ApiRequest::post(["zip_downloads"])
            .idempotency(Idempotency::Idempotent)
            .json(&body)?;

        let zip_download: ZipDownload = self.send_json(request).await?;

        tracing::debug!(
            items = items.len(),
            name_conflicts = zip_download.name_conflicts.len(),
            "created zip download"
        );

        Ok(zip_download)
    }

    /// ZIPダウンロードのZIPファイルをダウンロードし、ローカルのファイルに保存する。
    ///
    /// `zip_download.download_url`へ、アクセストークンを送信せずにリクエストする。
    /// BOXはこのURLで認証を求めないため、アクセストークンをダウンロード用のホストへ送信しない。
    ///
    /// [`BoxClient::download_file`]と同じく、内容をメモリに読み込まずに保存先と同じディレクトリの一時ファイルへ書き込み、
    /// すべての内容を受信した後に保存先に移動する。ダウンロードに失敗した場合は、保存先に存在したファイルを変更しない。
    /// タイムアウトも、[`BoxClient::download_file`]と同じく
    /// [`TimeoutConfig::download_read_timeout`](crate::TimeoutConfig::download_read_timeout)を適用する。
    ///
    /// ダウンロード用のURLは一度しか使用できず、BOXは2回目以降のリクエストに404を返す。
    /// したがって、HTTPメソッドがGETであっても冪等でないリクエストとして送信し、BOXがダウンロードを始めていないことが明らかな
    /// 429（同時にダウンロードできる数の上限）および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `zip_download`: [`BoxClient::create_zip_download`]で作成したZIPダウンロード
    /// - `path`: ZIPファイルを保存するパス。親ディレクトリは存在している必要がある。
    ///
    /// # Returns
    ///
    /// 保存したZIPファイルのバイト数を返す。
    ///
    /// # Errors
    ///
    /// `path`からファイル名を取得できない場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// `download_url`を解釈できない場合、または`download_url`のスキームが`https`でない場合は、
    /// リクエストを送信せずに[`BoxClientError::UnexpectedResponse`]を返す。
    /// 一時ファイルの作成、書き込み、または保存先への移動に失敗した場合は、[`BoxClientError::WriteFile`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 404: ダウンロード用のURLの有効期限が切れた、またはすでに使用した。ZIPダウンロードを作成し直す。
    /// - 429: ユーザーが同時にダウンロードしているZIPファイルが5個、またはエンタープライズで10個に達している。
    ///   クライアントは再試行設定に従って再試行し、再試行回数の上限に達した場合にこのエラーを返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn download_zip_content(
        &self,
        zip_download: &ZipDownload,
        path: impl AsRef<Path>,
    ) -> BoxClientResult<u64> {
        let path = path.as_ref();
        let temp_file = TempFile::new(path)?;
        let url = Url::parse(&zip_download.download_url)
            .map_err(|_| BoxClientError::UnexpectedResponse("download_url cannot be parsed"))?;
        if !has_secure_scheme(&url) {
            return Err(BoxClientError::UnexpectedResponse(
                "download_url must use https",
            ));
        }
        let request = ApiRequest::unauthenticated_get(url)
            .download_client()
            .idempotency(Idempotency::NonIdempotent);

        let size = self
            .send(request, |response, idempotency| {
                write_response(response, idempotency, &temp_file.path, path)
            })
            .await?;

        temp_file
            .persist(path)
            .await
            .map_err(|source| BoxClientError::WriteFile {
                path: path.to_owned(),
                source,
            })?;
        tracing::debug!(size, "downloaded zip file");

        Ok(size)
    }

    /// ZIPダウンロードの状態を取得する。
    ///
    /// BOXは、ダウンロードを始めてから12時間、状態を返す。ダウンロードを始める前は404を返す。
    /// ダウンロードの途中で進み具合を確認する場合や、ダウンロードを終えた後にスキップした項目の数を確認する場合に使用する。
    ///
    /// # Arguments
    ///
    /// - `zip_download`: [`BoxClient::create_zip_download`]で作成したZIPダウンロード
    ///
    /// # Errors
    ///
    /// `zip_download.status_url`からZIPダウンロードのIDを取り出せない場合は、
    /// リクエストを送信せずに[`BoxClientError::UnexpectedResponse`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: 項目にアクセスする権限がない。
    /// - 404: IDが不正である、ダウンロードをまだ始めていない、またはダウンロードを始めてから12時間が経過した。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn zip_download_status(
        &self,
        zip_download: &ZipDownload,
    ) -> BoxClientResult<ZipDownloadStatus> {
        let id = zip_download.id()?;
        self.send_json(ApiRequest::get(["zip_downloads", &id, "status"]))
            .await
    }

    /// ファイルおよびフォルダを1つのZIPファイルにまとめてダウンロードし、ローカルのファイルに保存する。
    ///
    /// [`BoxClient::create_zip_download`]でZIPダウンロードを作成し、[`BoxClient::download_zip_content`]で
    /// ZIPファイルを保存してから、[`BoxClient::zip_download_status`]でダウンロードを終えた後の状態を取得する。
    /// BOXは、ZIPダウンロードの作成後に権限が変わった項目などをスキップしてもダウンロードを続ける。
    /// したがって、戻り値の[`ZipDownloadSummary::status`]で、スキップしたファイルおよびフォルダの数を確認する。
    ///
    /// # Arguments
    ///
    /// - `items`: ZIPファイルに含めるファイルおよびフォルダ。1個以上[`MAX_ZIP_DOWNLOAD_ITEMS`]個以下で指定する。
    /// - `path`: ZIPファイルを保存するパス。親ディレクトリは存在している必要がある。
    /// - `options`: ZIPファイルの名前のオプション
    ///
    /// # Errors
    ///
    /// [`BoxClient::create_zip_download`]、[`BoxClient::download_zip_content`]および
    /// [`BoxClient::zip_download_status`]のエラーを返す。
    /// 状態の取得でエラーを返した場合も、ZIPファイルは保存されている。
    pub async fn download_zip(
        &self,
        items: &[ZipDownloadItem<'_>],
        path: impl AsRef<Path>,
        options: &CreateZipDownloadOptions<'_>,
    ) -> BoxClientResult<ZipDownloadSummary> {
        let zip_download = self.create_zip_download(items, options).await?;
        let size = self.download_zip_content(&zip_download, path).await?;
        let status = self.zip_download_status(&zip_download).await?;

        Ok(ZipDownloadSummary {
            size,
            name_conflicts: zip_download.name_conflicts,
            status,
        })
    }
}

#[cfg(test)]
mod test_zip_download_id {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    /// `status_url`から作成したZIPダウンロード
    fn zip_download(status_url: &str) -> anyhow::Result<ZipDownload> {
        Ok(serde_json::from_value(json!({
            "download_url": "https://dl.boxcloud.com/2.0/zip_downloads/abc/content",
            "status_url": status_url,
        }))?)
    }

    /// `status_url`の`zip_downloads`と`status`の間のパス要素をIDとして取り出すことを確認
    #[rstest]
    #[case::box_url("https://api.box.com/2.0/zip_downloads/abc/status", Some("abc"))]
    #[case::other_base("http://127.0.0.1:8080/2.0/zip_downloads/xyz/status", Some("xyz"))]
    #[case::no_status("https://api.box.com/2.0/zip_downloads/abc", None)]
    #[case::no_zip_downloads("https://api.box.com/2.0/files/abc/status", None)]
    #[case::unparsable("not a url", None)]
    fn test_id(#[case] status_url: &str, #[case] expected: Option<&str>) -> anyhow::Result<()> {
        let result = zip_download(status_url)?.id();

        assert_eq!(result.ok().as_deref(), expected);

        Ok(())
    }

    /// デバッグ出力に`download_url`を含めないことを確認
    #[test]
    fn test_debug_redacts_download_url() -> anyhow::Result<()> {
        let debug = format!(
            "{:?}",
            zip_download("https://api.box.com/2.0/zip_downloads/abc/status")?
        );

        assert!(!debug.contains("dl.boxcloud.com"), "debug: {debug}");

        Ok(())
    }
}

#[cfg(test)]
mod test_create_zip_download {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        count_requests, create_client, is_invalid_argument, mount_first_n, mount_token,
        request_count, short_retry,
    };

    /// ZIPダウンロードのAPIのパス
    const ZIP_DOWNLOADS_PATH: &str = "/2.0/zip_downloads";

    /// ZIPダウンロードの作成のレスポンス
    fn created_response() -> ResponseTemplate {
        ResponseTemplate::new(202).set_body_json(json!({
            "download_url": "https://dl.boxcloud.com/2.0/zip_downloads/abc/content",
            "status_url": "https://api.box.com/2.0/zip_downloads/abc/status",
            "expires_at": "2026-11-18T12:00:00Z",
            "name_conflicts": [[
                {"id": "1", "type": "file", "original_name": "Report.pdf", "download_name": "3aa6a7.pdf"},
                {"id": "2", "type": "file", "original_name": "Report.pdf", "download_name": "5d53f2.pdf"},
            ]],
        }))
    }

    /// 項目とZIPファイルの名前を送信し、ZIPダウンロードをデシリアライズすることを確認
    #[tokio::test]
    async fn test_create() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(ZIP_DOWNLOADS_PATH))
            .and(body_json(json!({
                "items": [{"type": "file", "id": "1"}, {"type": "folder", "id": "2"}],
                "download_file_name": "January Financials",
            })))
            .respond_with(created_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let file_id = FileId::new("1");
        let folder_id = FolderId::new("2");

        let zip_download = client
            .create_zip_download(
                &[
                    ZipDownloadItem::File(&file_id),
                    ZipDownloadItem::Folder(&folder_id),
                ],
                &CreateZipDownloadOptions::default().with_download_file_name("January Financials"),
            )
            .await?;

        assert_eq!(
            zip_download.status_url,
            "https://api.box.com/2.0/zip_downloads/abc/status"
        );
        assert_eq!(zip_download.name_conflicts.len(), 1);
        assert_eq!(
            zip_download.name_conflicts[0][1].download_name,
            "5d53f2.pdf"
        );
        assert_eq!(
            zip_download.name_conflicts[0][0].item_type,
            ZipDownloadItemType::File
        );

        Ok(())
    }

    /// 冪等なリクエストとして、5xxの後に再試行することを確認
    #[tokio::test]
    async fn test_retries_after_server_error() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_first_n(
            &server,
            path(ZIP_DOWNLOADS_PATH),
            ResponseTemplate::new(500),
            1,
        )
        .await;
        Mock::given(path(ZIP_DOWNLOADS_PATH))
            .respond_with(created_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;
        let file_id = FileId::new("1");

        client
            .create_zip_download(
                &[ZipDownloadItem::File(&file_id)],
                &CreateZipDownloadOptions::default(),
            )
            .await?;

        assert_eq!(count_requests(&server, "POST", ZIP_DOWNLOADS_PATH).await, 2);

        Ok(())
    }

    /// BOXが拒否する項目は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_rejects_invalid_items() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;
        let root = FolderId::root();
        let file_id = FileId::new("1");
        let too_many = vec![ZipDownloadItem::File(&file_id); MAX_ZIP_DOWNLOAD_ITEMS + 1];
        let options = CreateZipDownloadOptions::default();

        for items in [
            &[][..],
            &[ZipDownloadItem::Folder(&root)][..],
            too_many.as_slice(),
        ] {
            let result = client.create_zip_download(items, &options).await;

            assert!(
                result.as_ref().is_err_and(is_invalid_argument),
                "items: {}, result: {result:?}",
                items.len()
            );
        }
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }
}

#[cfg(test)]
mod test_download_zip {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{
        count_requests, create_client, mount_token, not_found_response, requests, short_retry,
    };

    /// ZIPファイルのダウンロード用のパス
    const CONTENT_PATH: &str = "/dl/2.0/zip_downloads/abc/content";

    /// ZIPダウンロードの状態のパス
    const STATUS_PATH: &str = "/2.0/zip_downloads/abc/status";

    /// ZIPファイルの内容
    const ZIP_CONTENT: &[u8] = b"PK\x03\x04zip content";

    /// モックサーバーを指すZIPダウンロード
    fn zip_download(server: &MockServer) -> anyhow::Result<ZipDownload> {
        Ok(serde_json::from_value(json!({
            "download_url": format!("{}{CONTENT_PATH}", server.uri()),
            "status_url": format!("{}{STATUS_PATH}", server.uri()),
        }))?)
    }

    /// アクセストークンを送信せずにZIPファイルをダウンロードし、保存することを確認
    #[tokio::test]
    async fn test_download_content_without_token() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(ZIP_CONTENT))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let dir = tempfile::tempdir()?;
        let zip_path = dir.path().join("archive.zip");

        let size = client
            .download_zip_content(&zip_download(&server)?, &zip_path)
            .await?;

        assert_eq!(size, ZIP_CONTENT.len() as u64);
        assert_eq!(tokio::fs::read(&zip_path).await?, ZIP_CONTENT);
        let received = requests(&server, "GET", CONTENT_PATH).await;
        assert!(
            received
                .iter()
                .all(|request| !request.headers.contains_key("authorization")),
            "the access token must not be sent to download_url"
        );

        Ok(())
    }

    /// 一度しか使用できないダウンロード用のURLへ、5xxの後に再送しないことを確認
    #[tokio::test]
    async fn test_content_server_error_is_not_retried() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;
        let dir = tempfile::tempdir()?;

        let result = client
            .download_zip_content(&zip_download(&server)?, dir.path().join("archive.zip"))
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(error)) if error.status == 500),
            "result: {result:?}"
        );
        assert_eq!(count_requests(&server, "GET", CONTENT_PATH).await, 1);

        Ok(())
    }

    /// 使用済みのダウンロード用のURLで404を受け取った場合は、エラーを返し、保存先にファイルを作成しないことを確認
    #[tokio::test]
    async fn test_content_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(path(CONTENT_PATH))
            .respond_with(not_found_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let dir = tempfile::tempdir()?;
        let zip_path = dir.path().join("archive.zip");

        let result = client
            .download_zip_content(&zip_download(&server)?, &zip_path)
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(error)) if error.status == 404),
            "result: {result:?}"
        );
        assert!(!zip_path.exists());
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);

        Ok(())
    }

    /// 暗号化しない通信のダウンロード用のURLは、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_rejects_insecure_download_url() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let dir = tempfile::tempdir()?;
        let zip_download: ZipDownload = serde_json::from_value(json!({
            "download_url": "http://dl.example.com/2.0/zip_downloads/abc/content",
            "status_url": "https://api.box.com/2.0/zip_downloads/abc/status",
        }))?;

        let result = client
            .download_zip_content(&zip_download, dir.path().join("archive.zip"))
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );

        Ok(())
    }

    /// 作成、ダウンロードおよび状態の取得を順に行い、結果を返すことを確認
    #[tokio::test]
    async fn test_download_zip() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path("/2.0/zip_downloads"))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({
                "download_url": format!("{}{CONTENT_PATH}", server.uri()),
                "status_url": format!("{}{STATUS_PATH}", server.uri()),
                "name_conflicts": [],
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(ZIP_CONTENT))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total_file_count": 3,
                "downloaded_file_count": 2,
                "skipped_file_count": 1,
                "skipped_folder_count": 0,
                "state": "succeeded",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let dir = tempfile::tempdir()?;
        let zip_path = dir.path().join("archive.zip");
        let folder_id = FolderId::new("2");

        let summary = client
            .download_zip(
                &[ZipDownloadItem::Folder(&folder_id)],
                &zip_path,
                &CreateZipDownloadOptions::default(),
            )
            .await?;

        assert_eq!(summary.size, ZIP_CONTENT.len() as u64);
        assert_eq!(
            summary.name_conflicts.len(),
            0,
            "name_conflicts: {:?}",
            summary.name_conflicts
        );
        assert_eq!(summary.status.skipped_file_count, Some(1));
        assert_eq!(summary.status.state, Some(ZipDownloadState::Succeeded));
        assert_eq!(tokio::fs::read(&zip_path).await?, ZIP_CONTENT);

        Ok(())
    }
}
