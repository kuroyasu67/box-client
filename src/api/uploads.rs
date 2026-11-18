//! ファイルのアップロードに関するAPI
//!
//! BOXは、ファイルのアップロードを、REST APIとは別のホスト（`upload.box.com`）で受け付ける。
//! 小さいファイルは1回のリクエストでアップロードし、大きいファイルは分割アップロードのAPIで部分ごとにアップロードする。

use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use futures_util::stream::{FuturesUnordered, StreamExt as _};
use reqwest::{
    Method, Response,
    header::{CONTENT_RANGE, HeaderName, HeaderValue},
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};
use tokio::io::AsyncReadExt as _;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, File, FileId, FolderId,
    api::types::{ParentFolder, RenameBody},
    request::ApiRequest,
    retry::{AttemptError, Idempotency, read_json, read_poll_response},
};

/// 内容のハッシュを送信するヘッダー
///
/// BOXは、ヘッダー名に反して、MD5ではなくSHA-1のハッシュを16進数で受け取る。
const CONTENT_MD5: HeaderName = HeaderName::from_static("content-md5");

/// 内容のダイジェストを`sha=<Base64で符号化したSHA-1>`の形式で送信するヘッダー
const DIGEST: HeaderName = HeaderName::from_static("digest");

/// ファイルのアップロードのオプション
///
/// # Examples
///
/// ```
/// use box_client::UploadFileOptions;
///
/// let options = UploadFileOptions::default().with_name("report.pdf");
/// assert_eq!(options.name, Some("report.pdf"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UploadFileOptions<'a> {
    /// BOXに保存するファイル名
    ///
    /// `None`の場合は、アップロードするファイルのパスのファイル名を使用する。
    pub name: Option<&'a str>,
}

impl<'a> UploadFileOptions<'a> {
    /// BOXに保存するファイル名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }
}

/// ファイルの内容の更新のオプション
///
/// # Examples
///
/// ```
/// use box_client::UpdateFileContentOptions;
///
/// let options = UpdateFileContentOptions::default()
///     .with_name("report-v2.pdf")
///     .with_etag("1");
/// assert_eq!(options.name, Some("report-v2.pdf"));
/// assert_eq!(options.etag, Some("1"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UpdateFileContentOptions<'a> {
    /// 新しいファイル名
    ///
    /// 指定した場合、BOXは内容の更新とともにファイル名を変更する。`None`の場合はファイル名を変更しない。
    pub name: Option<&'a str>,

    /// 最後に取得したファイルの`ETag`
    ///
    /// 指定した場合は、`If-Match`ヘッダーで送信する。ファイルが`ETag`の取得後に変更されていると、
    /// BOXはファイルの内容を更新せずに412を返す。
    pub etag: Option<&'a str>,
}

impl<'a> UpdateFileContentOptions<'a> {
    /// 新しいファイル名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// 最後に取得したファイルの`ETag`を設定する。
    #[must_use]
    pub fn with_etag(mut self, etag: &'a str) -> Self {
        self.etag = Some(etag);
        self
    }
}

/// アップロードの対象
///
/// 新しいファイルのアップロードと、既存のファイルの内容の更新では、呼び出すAPIと送信する属性だけが異なる。
#[derive(Debug, Clone, Copy)]
enum UploadTarget<'a> {
    /// 新しいファイル
    NewFile {
        /// アップロード先の親フォルダのID
        parent_id: &'a FolderId,

        /// ファイル名
        name: &'a str,
    },

    /// 既存のファイルの新しいバージョン
    NewVersion {
        /// 内容を更新するファイルのID
        file_id: &'a FileId,

        /// 新しいファイル名。`None`の場合はファイル名を変更しない。
        name: Option<&'a str>,

        /// 最後に取得したファイルの`ETag`
        etag: Option<&'a str>,
    },
}

impl UploadTarget<'_> {
    /// 1回のリクエストでアップロードするAPIのパス要素を返す。
    fn direct_upload_path(&self) -> Vec<&str> {
        match self {
            Self::NewFile { .. } => vec!["files", "content"],
            Self::NewVersion { file_id, .. } => vec!["files", file_id.as_ref(), "content"],
        }
    }

    /// 1回のリクエストでアップロードする場合の、`attributes`パートのJSONを返す。
    ///
    /// # Errors
    ///
    /// JSONにシリアライズできない場合は、[`BoxClientError::SerializeBody`]を返す。
    fn direct_upload_attributes(&self) -> BoxClientResult<Vec<u8>> {
        match *self {
            Self::NewFile { parent_id, name } => serde_json::to_vec(&UploadAttributes {
                name,
                parent: ParentFolder::new(parent_id),
            }),
            Self::NewVersion { name, .. } => serde_json::to_vec(&UploadVersionAttributes { name }),
        }
        .map_err(BoxClientError::SerializeBody)
    }

    /// アップロードセッションの作成リクエストを返す。
    ///
    /// # Errors
    ///
    /// ボディをJSONにシリアライズできない場合は、[`BoxClientError::SerializeBody`]を返す。
    fn create_upload_session_request(&self, file_size: u64) -> BoxClientResult<ApiRequest<'_>> {
        match *self {
            Self::NewFile { parent_id, name } => ApiRequest::post(["files", "upload_sessions"])
                .upload_host()
                .json(&CreateUploadSessionBody {
                    folder_id: parent_id,
                    file_size,
                    file_name: name,
                }),
            Self::NewVersion { file_id, name, .. } => {
                ApiRequest::post(["files", file_id.as_ref(), "upload_sessions"])
                    .upload_host()
                    .json(&CreateVersionUploadSessionBody {
                        file_size,
                        file_name: name,
                    })
            }
        }
    }

    /// `If-Match`ヘッダーで送信する`ETag`を返す。
    fn etag(&self) -> Option<&str> {
        match *self {
            Self::NewFile { .. } => None,
            Self::NewVersion { etag, .. } => etag,
        }
    }
}

/// ファイル名を検証する。
///
/// BOXは、ファイル名をファイル名の変更と同じ規則で検証するため、名前変更リクエストのボディの検証を使用する。
///
/// # Errors
///
/// ファイル名をBOXが拒否する場合は、[`BoxClientError::InvalidRequestBody`]を返す。
fn validate_file_name(name: &str) -> BoxClientResult<()> {
    RenameBody::new(name).map(|_| ())
}

/// 1回のリクエストでアップロードする場合の、ファイルの属性
#[derive(Debug, Serialize)]
struct UploadAttributes<'a> {
    /// ファイル名
    name: &'a str,

    /// アップロード先の親フォルダ
    parent: ParentFolder<'a>,
}

/// 1回のリクエストで新しいバージョンをアップロードする場合の、ファイルの属性
#[derive(Debug, Serialize)]
struct UploadVersionAttributes<'a> {
    /// 新しいファイル名
    ///
    /// `None`の場合は送信せず、BOXはファイル名を変更しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// プリフライトチェックのリクエストのボディ
#[derive(Debug, Serialize)]
struct PreflightUploadBody<'a> {
    /// ファイル名
    name: &'a str,

    /// ファイルのバイト数
    size: u64,

    /// アップロード先の親フォルダ
    parent: ParentFolder<'a>,
}

/// 既存のファイルの新しいバージョンのプリフライトチェックのリクエストのボディ
#[derive(Debug, Serialize)]
struct PreflightVersionBody<'a> {
    /// 新しいファイル名
    ///
    /// `None`の場合は送信せず、BOXはファイル名を変更しない場合として確認する。
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,

    /// ファイルのバイト数
    size: u64,
}

/// プリフライトチェックに合格した場合に、BOXが返すアップロード用のURLとアクセストークン
///
/// BOXのAPI仕様（OpenAPI）は、どちらの属性も必須としていないため、`Option`で受け取る。
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct UploadUrl {
    /// ファイルをアップロードできるアップロードセッションのURL
    pub upload_url: Option<String>,

    /// ファイルのアップロードに使用できるアクセストークン
    ///
    /// アクセストークンをログやデバッグ出力に残さないように、[`SecretString`]で保持する。
    pub upload_token: Option<SecretString>,
}

/// プリフライトチェックの成功レスポンスのボディを読み込む。
///
/// BOXのガイドは、成功レスポンスのボディを記載していない。
/// ボディが空の場合も確認には合格しているため、すべての属性を`None`とした[`UploadUrl`]を返す。
async fn read_upload_url(
    response: Response,
    idempotency: Idempotency,
) -> Result<UploadUrl, AttemptError> {
    let body = response.bytes().await.map_err(|error| {
        if idempotency == Idempotency::Idempotent {
            AttemptError::Retryable {
                error: error.into(),
                retry_after: None,
            }
        } else {
            AttemptError::Fatal(error.into())
        }
    })?;
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(UploadUrl::default());
    }
    serde_json::from_slice(&body)
        .map_err(|e| AttemptError::Fatal(BoxClientError::DeserializeResponse(e)))
}

/// アップロードしたファイルのリスト
#[derive(Debug, Deserialize)]
struct FileCollection {
    /// アップロードしたファイル
    entries: Vec<File>,
}

impl FileCollection {
    /// 最初のファイルを返す。
    ///
    /// # Errors
    ///
    /// ファイルが含まれていない場合は、[`BoxClientError::UnexpectedResponse`]を返す。
    fn into_first(self) -> BoxClientResult<File> {
        self.entries
            .into_iter()
            .next()
            .ok_or(BoxClientError::UnexpectedResponse(
                "upload response contains no file",
            ))
    }
}

/// アップロードセッションの作成リクエストのボディ
#[derive(Debug, Serialize)]
struct CreateUploadSessionBody<'a> {
    /// アップロード先の親フォルダのID
    folder_id: &'a FolderId,

    /// ファイルのバイト数
    file_size: u64,

    /// ファイル名
    file_name: &'a str,
}

/// 既存のファイルの新しいバージョンをアップロードする、アップロードセッションの作成リクエストのボディ
#[derive(Debug, Serialize)]
struct CreateVersionUploadSessionBody<'a> {
    /// ファイルのバイト数
    file_size: u64,

    /// 新しいファイル名
    ///
    /// `None`の場合は送信せず、BOXはファイル名を変更しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<&'a str>,
}

/// アップロードセッション
#[derive(Debug, Deserialize)]
struct UploadSession {
    /// アップロードセッションのID
    id: String,

    /// 最後の部分を除く、すべての部分のバイト数
    part_size: u64,

    /// 部分の数
    total_parts: u64,
}

/// アップロードした部分
///
/// コミットでは、この内容をそのままBOXに送り返す。
/// BOXは部分のハッシュ（`sha1`または`sha512`）を省略できるため、送り返さない。
/// ファイル全体の内容は、コミットの`Digest`ヘッダーで検証される。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
struct UploadedPart {
    /// 部分のID
    part_id: String,

    /// ファイルの先頭からの部分の位置（バイト）
    offset: u64,

    /// 部分のバイト数
    size: u64,
}

/// 部分のアップロードのレスポンス
///
/// BOXのAPIリファレンスは部分を`part`属性で包んで返すと記載し、ガイドは部分を直接返すと記載している。
/// どちらの形式でも受け付ける。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum UploadPartResponse {
    /// `part`属性で包んだ部分
    Wrapped {
        /// アップロードした部分
        part: UploadedPart,
    },

    /// 部分
    Bare(UploadedPart),
}

impl UploadPartResponse {
    /// アップロードした部分を返す。
    fn into_part(self) -> UploadedPart {
        match self {
            Self::Wrapped { part } | Self::Bare(part) => part,
        }
    }
}

/// アップロードセッションのコミットのリクエストのボディ
#[derive(Debug, Serialize)]
struct CommitUploadSessionBody<'a> {
    /// アップロードしたすべての部分
    parts: &'a [UploadedPart],
}

/// アップロードセッションのコミットの成功レスポンスのボディ
///
/// BOXのAPIリファレンスはファイルのリストを返すと記載し、ガイドはファイルを直接返すと記載している。
/// どちらの形式でも受け付ける。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CommitResponse {
    /// ファイルのリスト
    Files(FileCollection),

    /// ファイル
    File(Box<File>),
}

impl CommitResponse {
    /// コミットしたファイルを返す。
    ///
    /// # Errors
    ///
    /// ファイルのリストにファイルが含まれていない場合は、[`BoxClientError::UnexpectedResponse`]を返す。
    fn into_file(self) -> BoxClientResult<File> {
        match self {
            Self::Files(files) => files.into_first(),
            Self::File(file) => Ok(*file),
        }
    }
}

/// コミットのエラーが、BOXがコミットを処理している可能性を残すかどうかを返す。
///
/// 5xx、タイムアウト、通信エラー、およびBOXが処理を終えないまま再送の上限に達した場合は、
/// BOXがコミットを処理している可能性がある。
fn commit_may_be_processing(error: &BoxClientError) -> bool {
    match error {
        BoxClientError::Api(api_error) => api_error.status.is_server_error(),
        BoxClientError::Http(_)
        | BoxClientError::RetryAfterTooLong { .. }
        | BoxClientError::UnexpectedResponse(_) => true,
        _ => false,
    }
}

/// ファイルの読み込みエラーを、パスを含むエラーに変換する関数を返す。
fn read_file_error(path: &Path) -> impl FnOnce(std::io::Error) -> BoxClientError + '_ {
    move |source| BoxClientError::ReadFile {
        path: path.to_owned(),
        source,
    }
}

/// 新しいファイルとしてアップロードする場合の、BOXに保存するファイル名を決めて検証する。
///
/// `options.name`を指定した場合はその名前を、指定しない場合はパスのファイル名を使用する。
///
/// # Errors
///
/// `options.name`を指定せず、パスからUTF-8のファイル名を取得できない場合は、[`BoxClientError::InvalidArgument`]を返す。
/// ファイル名をBOXが拒否する場合は、[`BoxClientError::InvalidRequestBody`]を返す。
fn upload_file_name<'a>(
    path: &'a Path,
    options: &UploadFileOptions<'a>,
) -> BoxClientResult<&'a str> {
    let name = match options.name {
        Some(name) => name,
        None => path.file_name().and_then(|name| name.to_str()).ok_or(
            BoxClientError::InvalidArgument("the file name cannot be taken from the path"),
        )?,
    };
    validate_file_name(name)?;
    Ok(name)
}

/// `Digest`ヘッダーの値を返す。
fn sha1_digest_header(digest: &[u8]) -> String {
    format!("sha={}", BASE64.encode(digest))
}

impl BoxClient {
    /// ファイルをアップロードする。
    ///
    /// ファイルサイズが[`BoxClient::chunked_upload_threshold`]以下の場合は、ファイル全体をメモリに読み込み、
    /// 1回のリクエストでアップロードする（`POST /files/content`）。
    /// [`BoxClient::chunked_upload_threshold`]を超える場合は、分割アップロードのAPIを使用する。
    /// 分割アップロードでは、アップロードセッションを作成し、BOXが指定したサイズの部分をアップロードしてから、
    /// アップロードセッションをコミットする。部分は、[`BoxClient::upload_concurrency`]個まで並列にアップロードする。
    /// メモリには、並列にアップロードしている部分だけを読み込む。
    ///
    /// 1回のリクエストでのアップロードとコミットは、HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 部分のアップロードは、HTTPメソッドがPUTであり、同じ範囲を同じ内容で送り直すため、冪等なリクエストとして送信する。
    ///
    /// 部分のアップロードまたはコミットに失敗した場合は、アップロードセッションを中止してからエラーを返す。
    ///
    /// 1回のリクエストで送信するデータが大きいため、
    /// [`BoxClientTimeout::request_timeout`](crate::BoxClientTimeout::request_timeout)は、
    /// 通信速度に応じて、[`BoxClient::chunked_upload_threshold`]または部分のサイズを送信できる長さにする。
    ///
    /// # Arguments
    ///
    /// - `path`: アップロードするファイルのパス
    /// - `parent_id`: アップロード先の親フォルダのID
    /// - `options`: BOXに保存するファイル名のオプション
    ///
    /// # Errors
    ///
    /// `options.name`を指定せず、パスからUTF-8のファイル名を取得できない場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ファイル名をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// ファイルを読み込めない場合は、[`BoxClientError::ReadFile`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 400: ファイル名が不正である。
    /// - 403: ストレージの上限を超える。
    /// - 404: 親フォルダが存在しない、またはアクセスする権限がない。
    /// - 409: 同じ名前の項目が親フォルダに存在する、またはストレージの空き容量がない。
    ///
    /// 分割アップロードのコミットで、BOXが部分の処理を終えないまま、コミットの再送の上限に達した場合は、
    /// [`BoxClientError::UnexpectedResponse`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイルを作成した可能性がある。
    /// したがって、[`BoxClient::list_items_in_folder`]で親フォルダを確認する。
    pub async fn upload_file(
        &self,
        path: impl AsRef<Path>,
        parent_id: &FolderId,
        options: &UploadFileOptions<'_>,
    ) -> BoxClientResult<File> {
        self.upload_file_with_threshold(
            path.as_ref(),
            parent_id,
            options,
            self.chunked_upload_threshold(),
        )
        .await
    }

    /// 分割アップロードに切り替えるファイルサイズを指定して、ファイルをアップロードする。
    ///
    /// テストで大きなファイルを作成せずに分割アップロードを確認できるように、分割アップロードに切り替えるファイルサイズを引数で受け取る。
    async fn upload_file_with_threshold(
        &self,
        path: &Path,
        parent_id: &FolderId,
        options: &UploadFileOptions<'_>,
        threshold: u64,
    ) -> BoxClientResult<File> {
        let name = upload_file_name(path, options)?;

        self.upload(path, UploadTarget::NewFile { parent_id, name }, threshold)
            .await
    }

    /// ファイルをアップロードする前に、BOXがファイルを受け付けるかを確認する（プリフライトチェック）。
    ///
    /// ファイルの内容を送信せずに、ファイル名、ファイルサイズおよびアップロード先の親フォルダだけをBOXに送信し、
    /// 実際のアップロードと同じ検査を受ける。BOXは、ファイル名の重複、ファイルサイズの上限、
    /// およびフォルダとアカウントのストレージの容量を検査する。
    /// 大きいファイルの内容を送信してから拒否されることを避けるために使用する。
    ///
    /// 引数は[`BoxClient::upload_file`]と同じであり、ファイル名とファイルサイズも`upload_file`と同じ方法で決める。
    /// ファイルの内容は読み込まず、ファイルのメタデータだけを読み込む。
    ///
    /// 確認に合格した場合は、BOXが返すアップロード用のURLとアクセストークンを[`UploadUrl`]として返す。
    /// [`BoxClient::upload_file`]はこれらを使用しないため、`upload_file`でアップロードする場合は戻り値を使用しなくてよい。
    /// なお、確認に合格しても、その後のアップロードが成功するとは限らない。確認とアップロードの間に、
    /// 同じ名前のファイルが作成される場合などがあるためである。
    ///
    /// HTTPメソッドがOPTIONSであり、BOXは状態を変更しないため、冪等なリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `path`: アップロードするファイルのパス
    /// - `parent_id`: アップロード先の親フォルダのID
    /// - `options`: BOXに保存するファイル名のオプション
    ///
    /// # Errors
    ///
    /// `options.name`を指定せず、パスからUTF-8のファイル名を取得できない場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ファイル名をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// ファイルのメタデータを読み込めない場合は、[`BoxClientError::ReadFile`]を返す。
    /// BOXがファイルを受け付けない場合は、[`BoxClientError::Api`]を返す。
    /// 例えば、同じ名前のファイルが親フォルダに存在する場合、BOXは409を返す。
    /// このとき、[`ApiError::body`](crate::ApiError::body)の`context_info.conflicts`に、名前が重複するファイルが含まれる。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn preflight_upload_file(
        &self,
        path: impl AsRef<Path>,
        parent_id: &FolderId,
        options: &UploadFileOptions<'_>,
    ) -> BoxClientResult<UploadUrl> {
        let path = path.as_ref();
        let name = upload_file_name(path, options)?;
        let size = tokio::fs::metadata(path)
            .await
            .map_err(read_file_error(path))?
            .len();

        let request =
            ApiRequest::new(Method::OPTIONS, ["files", "content"]).json(&PreflightUploadBody {
                name,
                size,
                parent: ParentFolder::new(parent_id),
            })?;
        let upload_url = self.send(request, read_upload_url).await?;

        tracing::debug!(
            path = %path.display(),
            size,
            parent_id = parent_id.as_ref(),
            "preflight check passed"
        );

        Ok(upload_url)
    }

    /// ファイルの内容を更新する前に、BOXが新しいバージョンを受け付けるかを確認する（プリフライトチェック）。
    ///
    /// ファイルの内容を送信せずに、ファイルサイズと、指定した場合は新しいファイル名だけをBOXに送信し、
    /// 実際の更新と同じ検査を受ける。BOXは、ファイル名の重複、ファイルサイズの上限、
    /// およびフォルダとアカウントのストレージの容量を検査する。
    ///
    /// 引数は[`BoxClient::update_file_content`]と同じであり、ファイルサイズは`update_file_content`と同じく
    /// ファイルのメタデータから決める。ファイルの内容は読み込まない。
    /// BOXのドキュメントは、このAPIが`If-Match`ヘッダーを受け付けることを記載していないため、`options.etag`は送信しない。
    ///
    /// 確認に合格した場合は、BOXが返すアップロード用のURLとアクセストークンを[`UploadUrl`]として返す。
    /// [`BoxClient::update_file_content`]はこれらを使用しない。
    /// なお、確認に合格しても、その後の更新が成功するとは限らない。
    ///
    /// HTTPメソッドがOPTIONSであり、BOXは状態を変更しないため、冪等なリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 内容を更新するファイルのID
    /// - `path`: 新しい内容のファイルのパス
    /// - `options`: 新しいファイル名のオプション。`options.etag`は使用しない。
    ///
    /// # Errors
    ///
    /// `options.name`をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// ファイルのメタデータを読み込めない場合は、[`BoxClientError::ReadFile`]を返す。
    /// BOXが新しいバージョンを受け付けない場合は、[`BoxClientError::Api`]を返す。
    /// 例えば、新しいファイル名と同じ名前の項目が親フォルダに存在する場合、BOXは409を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn preflight_update_file_content(
        &self,
        file_id: &FileId,
        path: impl AsRef<Path>,
        options: &UpdateFileContentOptions<'_>,
    ) -> BoxClientResult<UploadUrl> {
        let path = path.as_ref();
        if let Some(name) = options.name {
            validate_file_name(name)?;
        }
        let size = tokio::fs::metadata(path)
            .await
            .map_err(read_file_error(path))?
            .len();

        let request = ApiRequest::new(Method::OPTIONS, ["files", file_id.as_ref(), "content"])
            .json(&PreflightVersionBody {
                name: options.name,
                size,
            })?;
        let upload_url = self.send(request, read_upload_url).await?;

        tracing::debug!(
            path = %path.display(),
            size,
            file_id = file_id.as_ref(),
            "preflight check for new file version passed"
        );

        Ok(upload_url)
    }

    /// ファイルの内容を更新する。
    ///
    /// BOXは、ファイルの新しいバージョンとして内容を保存し、以前の内容をバージョン履歴に残す。
    ///
    /// ファイルサイズが[`BoxClient::chunked_upload_threshold`]以下の場合は、ファイル全体をメモリに読み込み、
    /// 1回のリクエストでアップロードする（`POST /files/{file_id}/content`）。
    /// [`BoxClient::chunked_upload_threshold`]を超える場合は、分割アップロードのAPIを使用する
    /// （`POST /files/{file_id}/upload_sessions`で作成したアップロードセッションに部分をアップロードしてコミットする）。
    /// 分割アップロードの進め方、冪等性および失敗した場合の扱いは、[`BoxClient::upload_file`]と同じである。
    ///
    /// `options.etag`を指定した場合は、1回のリクエストでのアップロード、または分割アップロードのコミットで、
    /// `If-Match`ヘッダーとして送信する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 内容を更新するファイルのID
    /// - `path`: 新しい内容のファイルのパス
    /// - `options`: 新しいファイル名および`ETag`のオプション
    ///
    /// # Errors
    ///
    /// `options.name`をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、[`BoxClientError::InvalidHeaderValue`]を返す。
    /// ファイルを読み込めない場合は、[`BoxClientError::ReadFile`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 404: ファイルが存在しない、またはアクセスする権限がない。
    /// - 409: `options.name`と同じ名前の項目が親フォルダに存在する、またはストレージの空き容量がない。
    /// - 412: `options.etag`がファイルの現在の`ETag`と一致しない。
    ///
    /// 分割アップロードのコミットで、BOXが部分の処理を終えないまま、コミットの再送の上限に達した場合は、
    /// [`BoxClientError::UnexpectedResponse`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイルの内容を更新した可能性がある。
    /// したがって、[`BoxClient::file_information`]でファイルのバージョンを確認する。
    pub async fn update_file_content(
        &self,
        file_id: &FileId,
        path: impl AsRef<Path>,
        options: &UpdateFileContentOptions<'_>,
    ) -> BoxClientResult<File> {
        self.update_file_content_with_threshold(
            file_id,
            path.as_ref(),
            options,
            self.chunked_upload_threshold(),
        )
        .await
    }

    /// 分割アップロードに切り替えるファイルサイズを指定して、ファイルの内容を更新する。
    ///
    /// テストで大きなファイルを作成せずに分割アップロードを確認できるように、分割アップロードに切り替えるファイルサイズを引数で受け取る。
    async fn update_file_content_with_threshold(
        &self,
        file_id: &FileId,
        path: &Path,
        options: &UpdateFileContentOptions<'_>,
        threshold: u64,
    ) -> BoxClientResult<File> {
        if let Some(name) = options.name {
            validate_file_name(name)?;
        }
        // 分割アップロードでは、`ETag`をコミットで初めて送信する。
        // すべての部分をアップロードした後に`ETag`の不正に気づかないように、アップロードを始める前に検証する。
        if let Some(etag) = options.etag {
            HeaderValue::from_str(etag).map_err(BoxClientError::InvalidHeaderValue)?;
        }

        let target = UploadTarget::NewVersion {
            file_id,
            name: options.name,
            etag: options.etag,
        };
        self.upload(path, target, threshold).await
    }

    /// ファイルサイズに応じて、1回のリクエストまたは分割アップロードのAPIでアップロードする。
    async fn upload(
        &self,
        path: &Path,
        target: UploadTarget<'_>,
        threshold: u64,
    ) -> BoxClientResult<File> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(read_file_error(path))?;
        let size = file.metadata().await.map_err(read_file_error(path))?.len();

        let uploaded = if size <= threshold {
            self.upload_file_directly(path, &mut file, size, &target)
                .await?
        } else {
            self.upload_file_in_chunks(path, &mut file, size, &target)
                .await?
        };

        tracing::debug!(
            path = %path.display(),
            size,
            ?target,
            file_id = uploaded.id.as_ref(),
            chunked = threshold < size,
            "uploaded file"
        );

        Ok(uploaded)
    }

    /// ファイルを1回のリクエストでアップロードする。
    ///
    /// BOXは、`attributes`パートが`file`パートより前にない場合に400（`metadata_after_file_contents`）を返す。
    /// したがって、`attributes`パート、`file`パートの順に送信する。
    /// 再試行のたびにボディを作り直さないように、multipartのボディをバイト列として組み立てる。
    async fn upload_file_directly(
        &self,
        path: &Path,
        file: &mut tokio::fs::File,
        size: u64,
        target: &UploadTarget<'_>,
    ) -> BoxClientResult<File> {
        /// ボディのうち、ファイルの内容と`attributes`パートの内容以外の、multipartの区切りとヘッダーのバイト数の見積もり
        const MULTIPART_OVERHEAD: usize = 512;

        let attributes = target.direct_upload_attributes()?;
        let boundary = format!("box-client-{}", uuid::Uuid::new_v4().simple());

        // `read_to_end`がバッファを倍々に拡張して、ファイルサイズの約2倍のメモリを確保しないように、必要な容量を先に確保する。
        // ファイルサイズは分割アップロードに切り替えるファイルサイズ以下であるため、`usize`に収まる。
        let capacity = usize::try_from(size)
            .unwrap_or(0)
            .saturating_add(attributes.len() + MULTIPART_OVERHEAD);
        let mut body = Vec::with_capacity(capacity);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\n\
                 Content-Disposition: form-data; name=\"attributes\"\r\n\
                 Content-Type: application/json\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&attributes);
        // BOXはファイル名を`attributes`から取得するため、`filename`には固定の値を指定し、ファイル名のエスケープを避ける。
        body.extend_from_slice(
            format!(
                "\r\n--{boundary}\r\n\
                 Content-Disposition: form-data; name=\"file\"; filename=\"file\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        // ファイルの内容をボディに直接読み込み、ファイルの内容とボディを別々にメモリに保持しない。
        let content_start = body.len();
        file.read_to_end(&mut body)
            .await
            .map_err(read_file_error(path))?;
        let content_sha1 = format!("{:x}", Sha1::digest(&body[content_start..]));
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let content_type =
            HeaderValue::from_str(&format!("multipart/form-data; boundary={boundary}"))
                .map_err(BoxClientError::InvalidHeaderValue)?;
        let mut request = ApiRequest::post(target.direct_upload_path())
            .upload_host()
            .header(CONTENT_MD5, &content_sha1)?
            .body(content_type, Bytes::from(body));
        request = request.if_match(target.etag())?;

        let files: FileCollection = self.send_json(request).await?;
        files.into_first()
    }

    /// ファイルを分割アップロードのAPIでアップロードする。
    ///
    /// 部分のアップロードに失敗した場合は、アップロードセッションを中止してからエラーを返す。
    /// コミットに失敗した場合は、BOXがコミットを拒否したことが明らかな場合だけ、アップロードセッションを中止する。
    /// 5xx、タイムアウトおよび通信エラーの場合は、BOXがコミットを処理している可能性があるため、中止しない。
    /// 中止すると、BOXが処理しているコミットを取り消すおそれがあるためである。
    async fn upload_file_in_chunks(
        &self,
        path: &Path,
        file: &mut tokio::fs::File,
        size: u64,
        target: &UploadTarget<'_>,
    ) -> BoxClientResult<File> {
        let request = target.create_upload_session_request(size)?;
        let session: UploadSession = self.send_json(request).await?;
        tracing::debug!(
            upload_session_id = session.id,
            part_size = session.part_size,
            total_parts = session.total_parts,
            "created upload session"
        );

        let (digest, parts) = match self.upload_parts(path, file, size, &session).await {
            Ok(uploaded) => uploaded,
            Err(error) => {
                self.abort_upload_session(&session.id).await;
                return Err(error);
            }
        };

        let result = self
            .commit_upload_session(&session, &digest, &parts, target.etag())
            .await;
        if let Err(error) = &result {
            if commit_may_be_processing(error) {
                tracing::debug!(
                    upload_session_id = session.id,
                    "commit may be being processed; not aborting upload session"
                );
            } else {
                self.abort_upload_session(&session.id).await;
            }
        }
        result
    }

    /// ファイルを部分ごとにアップロードし、ファイル全体の`Digest`ヘッダーの値と、位置の順に並べた部分を返す。
    ///
    /// ファイルを先頭から順に読み込みながら、[`BoxClient::upload_concurrency`]個までの部分を並列にアップロードする。
    /// いずれかの部分のアップロードに失敗した場合は、アップロード中の部分を取り消してエラーを返す。
    ///
    /// # Errors
    ///
    /// ファイルを読み込めない場合、またはアップロード中にファイルサイズが変わった場合は、[`BoxClientError::ReadFile`]を返す。
    async fn upload_parts(
        &self,
        path: &Path,
        file: &mut tokio::fs::File,
        size: u64,
        session: &UploadSession,
    ) -> BoxClientResult<(String, Vec<UploadedPart>)> {
        // 部分のサイズが0の場合は、ファイルを読み進められず無限ループになるため、BOXの応答を不正として扱う。
        if session.part_size == 0 {
            return Err(BoxClientError::UnexpectedResponse(
                "upload session part_size is 0",
            ));
        }

        let concurrency = self.upload_concurrency();
        // ファイルを先頭から順に読み込むため、部分を読み込むたびにファイル全体のハッシュも計算する。
        let mut file_hasher = Sha1::new();
        let mut parts = Vec::new();
        let mut uploading = FuturesUnordered::new();
        let mut offset = 0;
        loop {
            // 空きがある限り、次の部分を読み込んでアップロードを始める。
            // 読み込みを待つ間はアップロード中の部分をポーリングしないが、ローカルのファイルの読み込みは
            // 部分の送信より十分に短いため、送信の遅れは無視できる。
            while uploading.len() < concurrency && offset < size {
                let part_size = session.part_size.min(size - offset);
                let mut part = vec![
                    0;
                    usize::try_from(part_size).map_err(|_| {
                        BoxClientError::UnexpectedResponse("upload session part_size is too large")
                    })?
                ];
                file.read_exact(&mut part)
                    .await
                    .map_err(read_file_error(path))?;
                file_hasher.update(&part);
                uploading.push(self.upload_part(session, offset, size, Bytes::from(part)));
                offset += part_size;
            }

            // アップロード中の部分がなければ、すべての部分のアップロードを終えている。
            let Some(uploaded) = uploading.next().await else {
                break;
            };
            let part = uploaded?;
            tracing::debug!(
                upload_session_id = session.id,
                offset = part.offset,
                size = part.size,
                uploaded_parts = parts.len() + 1,
                total_parts = session.total_parts,
                "uploaded part"
            );
            parts.push(part);
        }

        // アップロード中にファイルが大きくなった場合は、BOXがファイルの先頭部分だけを保存しないように、エラーを返す。
        // ファイルが小さくなった場合は、部分の読み込みで`read_exact`がエラーを返す。
        let mut probe = [0u8; 1];
        if file.read(&mut probe).await.map_err(read_file_error(path))? != 0 {
            return Err(read_file_error(path)(std::io::Error::other(
                "the file grew during the upload",
            )));
        }

        // 部分は完了した順に並ぶため、コミットの前にファイルの先頭からの位置の順に並べ替える。
        parts.sort_by_key(|part| part.offset);
        let digest = sha1_digest_header(&file_hasher.finalize());
        Ok((digest, parts))
    }

    /// 部分をアップロードする。
    ///
    /// # Arguments
    ///
    /// - `session`: アップロードセッション
    /// - `offset`: ファイルの先頭からの部分の位置（バイト）
    /// - `file_size`: ファイルのバイト数
    /// - `part`: 部分の内容
    async fn upload_part(
        &self,
        session: &UploadSession,
        offset: u64,
        file_size: u64,
        part: Bytes,
    ) -> BoxClientResult<UploadedPart> {
        let digest = sha1_digest_header(&Sha1::digest(&part));
        let last = offset + part.len() as u64 - 1;
        let content_range = format!("bytes {offset}-{last}/{file_size}");
        let request = ApiRequest::put(["files", "upload_sessions", &session.id])
            .upload_host()
            .header(DIGEST, &digest)?
            .header(CONTENT_RANGE, &content_range)?
            .body(HeaderValue::from_static("application/octet-stream"), part);
        let uploaded: UploadPartResponse = self.send_json(request).await?;
        Ok(uploaded.into_part())
    }

    /// アップロードセッションをコミットする。
    ///
    /// BOXが部分の処理を終えていないことを示す202を返した場合、このメソッドは、
    /// [`BoxClient::poll_until_ready`]に従って待機してから、コミットを再送する。
    /// 再送する回数の上限は、[`BoxClient::max_commit_polls`]が返す値とする。
    ///
    /// `etag`を指定した場合は、`If-Match`ヘッダーで送信する。
    async fn commit_upload_session(
        &self,
        session: &UploadSession,
        digest: &str,
        parts: &[UploadedPart],
        etag: Option<&str>,
    ) -> BoxClientResult<File> {
        let body = CommitUploadSessionBody { parts };
        let response: CommitResponse = self
            .poll_until_ready(
                self.max_commit_polls(),
                "upload session was not committed before reaching max commit polls",
                |polls, delay| {
                    tracing::debug!(
                        upload_session_id = session.id,
                        polls,
                        delay_ms = delay.as_millis(),
                        "upload session is being processed; resending commit"
                    );
                },
                || async {
                    let request =
                        ApiRequest::post(["files", "upload_sessions", &session.id, "commit"])
                            .upload_host()
                            .header(DIGEST, digest)?
                            .json(&body)?
                            .if_match(etag)?;
                    self.send(request, |response, idempotency| {
                        read_poll_response(response, idempotency, read_json)
                    })
                    .await
                },
            )
            .await?;
        response.into_file()
    }

    /// アップロードセッションを中止する。
    ///
    /// アップロードの失敗を呼び出し元に返すことを優先するため、中止に失敗した場合はWARNで出力するだけにする。
    /// BOXは、中止しなかったアップロードセッションを7日後に破棄する。
    async fn abort_upload_session(&self, session_id: &str) {
        let request = ApiRequest::delete(["files", "upload_sessions", session_id]).upload_host();
        match self.send_empty(request).await {
            Ok(()) => tracing::debug!(upload_session_id = session_id, "aborted upload session"),
            Err(error) => tracing::warn!(
                upload_session_id = session_id,
                error = &error as &(dyn std::error::Error + 'static),
                "failed to abort upload session"
            ),
        }
    }
}

#[cfg(test)]
mod chunked_upload_test_constants {
    //! 分割アップロードのテストで共有する定数

    /// 分割アップロードに切り替えるファイルサイズ
    ///
    /// 大きなファイルを作成せずに分割アップロードを確認できるように、小さい値にする。
    pub(super) const THRESHOLD: u64 = 20;

    /// 分割アップロードでアップロードするファイルの内容
    ///
    /// 部分のサイズを10バイトとし、最後の部分が部分のサイズより小さくなる長さにする。
    pub(super) const CHUNKED_CONTENT: &[u8] = b"abcdefghijklmnopqrstuvwxy";

    /// 分割アップロードの部分のサイズ
    pub(super) const PART_SIZE: usize = 10;

    /// アップロードセッションのID
    pub(super) const SESSION_ID: &str = "S1";
}

#[cfg(test)]
mod test_upload_file {
    use std::{path::PathBuf, time::Duration};

    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    use super::{
        chunked_upload_test_constants::{CHUNKED_CONTENT, PART_SIZE, SESSION_ID, THRESHOLD},
        *,
    };
    use crate::{
        MIN_CHUNKED_UPLOAD_THRESHOLD,
        test_support::{
            client_builder, create_client, create_client_with_upload_concurrency, mount_token,
            request_count, requests, short_retry, upload_path,
        },
    };

    /// 一時ディレクトリに、指定した名前と内容のファイルを作成する。
    fn create_file(name: &str, content: &[u8]) -> anyhow::Result<(TempDir, PathBuf)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(name);
        std::fs::write(&path, content)?;
        Ok((dir, path))
    }

    /// ファイル情報のJSONを返す。
    fn file_json(name: &str) -> serde_json::Value {
        json!({"type": "file", "id": "11", "name": name})
    }

    /// 1回のリクエストで、`attributes`パート、`file`パートの順に送信し、内容のSHA-1をヘッダーで送信することを確認
    ///
    /// ファイル名を指定しない場合は、パスのファイル名を使用することも確認する。
    #[rstest]
    #[case::name_from_path(None, "report.txt")]
    #[case::name_from_options(Some("renamed.txt"), "renamed.txt")]
    #[tokio::test]
    async fn test_upload_file_directly(
        #[case] option_name: Option<&'static str>,
        #[case] expected_name: &str,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("report.txt", b"hello")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(upload_path("/files/content")))
            // "hello"のSHA-1
            .and(header(
                "content-md5",
                "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d",
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "total_count": 1,
                "entries": [file_json(expected_name)],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = UploadFileOptions { name: option_name };

        let file = client
            .upload_file_with_threshold(&file_path, &FolderId::root(), &options, THRESHOLD)
            .await?;

        assert_eq!(file.name, expected_name);
        let requests = requests(&server, "POST", &upload_path("/files/content")).await;
        let [request] = requests.as_slice() else {
            anyhow::bail!("expected one request, got {}", requests.len());
        };
        let content_type = request
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("multipart/form-data; boundary="),
            "content-type: {content_type}"
        );
        let body = String::from_utf8_lossy(&request.body);
        let attributes = format!(r#"{{"name":"{expected_name}","parent":{{"id":"0"}}}}"#);
        let attributes_at = body.find(&attributes);
        let file_at = body.find("name=\"file\"");
        let content_at = body.find("\r\n\r\nhello\r\n");
        assert!(
            matches!((attributes_at, file_at, content_at), (Some(a), Some(f), Some(c)) if a < f && f < c),
            "body: {body}"
        );

        Ok(())
    }

    /// BOXが拒否するファイル名、またはパスから取得できないファイル名の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::invalid_name(Some("a/b"), "InvalidRequestBody")]
    #[case::too_long_name(Some(&*"あ".repeat(256)), "InvalidRequestBody")]
    #[case::name_from_dot_dot_path(None, "InvalidArgument")]
    #[tokio::test]
    async fn test_upload_file_rejects_name_without_request(
        #[case] option_name: Option<&str>,
        #[case] expected: &str,
    ) -> anyhow::Result<()> {
        let (dir, _) = create_file("report.txt", b"hello")?;
        // `..`で終わるパスからは、ファイル名を取得できない。
        let file_path = dir.path().join("..");
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;
        let options = UploadFileOptions { name: option_name };

        let result = client
            .upload_file_with_threshold(&file_path, &FolderId::root(), &options, THRESHOLD)
            .await;

        let matched = match expected {
            "InvalidArgument" => matches!(result, Err(BoxClientError::InvalidArgument(_))),
            _ => matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
        };
        assert!(matched, "result: {result:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// ファイルが存在しない場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_upload_file_not_found() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .upload_file(
                dir.path().join("missing.txt"),
                &FolderId::root(),
                &UploadFileOptions::default(),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::ReadFile { .. })),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// アップロードセッションの作成と、すべての部分のアップロードのモックを登録する。
    ///
    /// BOXのAPIリファレンスとガイドで部分のレスポンスの形式が異なるため、最後の部分だけ`part`属性で包まずに返す。
    async fn mount_session_and_parts(server: &MockServer) {
        mount_session_and_delayed_parts(server, [Duration::ZERO; 3]).await;
    }

    /// アップロードセッションの作成と、指定した時間だけ遅れて応答する部分のアップロードのモックを登録する。
    async fn mount_session_and_delayed_parts(server: &MockServer, part_delays: [Duration; 3]) {
        Mock::given(method("POST"))
            .and(path(upload_path("/files/upload_sessions")))
            .and(body_json(json!({
                "folder_id": "0",
                "file_size": CHUNKED_CONTENT.len(),
                "file_name": "big.bin",
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "upload_session",
                "id": SESSION_ID,
                "part_size": PART_SIZE,
                "total_parts": 3,
                "num_parts_processed": 0,
            })))
            .expect(1)
            .mount(server)
            .await;

        let chunks: Vec<&[u8]> = CHUNKED_CONTENT.chunks(PART_SIZE).collect();
        let last = chunks.len() - 1;
        for ((i, chunk), delay) in chunks.into_iter().enumerate().zip(part_delays) {
            let offset = i * PART_SIZE;
            let part = json!({
                "part_id": format!("P{i}"),
                "offset": offset,
                "size": chunk.len(),
                "sha1": format!("{:x}", Sha1::digest(chunk)),
            });
            let body = if i == last {
                part
            } else {
                json!({"part": part})
            };
            Mock::given(method("PUT"))
                .and(path(upload_path(&format!(
                    "/files/upload_sessions/{SESSION_ID}"
                ))))
                .and(header(
                    "content-range",
                    format!(
                        "bytes {offset}-{}/{}",
                        offset + chunk.len() - 1,
                        CHUNKED_CONTENT.len()
                    )
                    .as_str(),
                ))
                .and(header(
                    "digest",
                    sha1_digest_header(&Sha1::digest(chunk)).as_str(),
                ))
                .and(header("content-type", "application/octet-stream"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(body)
                        .set_delay(delay),
                )
                .expect(1)
                .mount(server)
                .await;
        }
    }

    /// コミットで送信するボディを返す。
    fn commit_body() -> serde_json::Value {
        json!({"parts": [
            {"part_id": "P0", "offset": 0, "size": 10},
            {"part_id": "P1", "offset": 10, "size": 10},
            {"part_id": "P2", "offset": 20, "size": 5},
        ]})
    }

    /// 分割アップロードに切り替えるファイルサイズを超えるファイルを、部分ごとにアップロードしてからコミットすることを確認
    #[tokio::test]
    async fn test_upload_file_in_chunks() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_session_and_parts(&server).await;
        Mock::given(method("POST"))
            .and(path(upload_path(&format!(
                "/files/upload_sessions/{SESSION_ID}/commit"
            ))))
            .and(header(
                "digest",
                sha1_digest_header(&Sha1::digest(CHUNKED_CONTENT)).as_str(),
            ))
            .and(body_json(commit_body()))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "total_count": 1,
                "entries": [file_json("big.bin")],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await?;

        assert_eq!(file.name, "big.bin");
        let aborts = requests(
            &server,
            "DELETE",
            &upload_path(&format!("/files/upload_sessions/{SESSION_ID}")),
        )
        .await;
        assert!(aborts.is_empty());

        Ok(())
    }

    /// コミットでBOXが202を返した場合は、待機してからコミットを再送することを確認
    ///
    /// BOXのAPIリファレンスとガイドでコミットのレスポンスの形式が異なるため、ファイルを直接返す形式も確認する。
    #[tokio::test]
    async fn test_upload_file_in_chunks_waits_for_processing() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_session_and_parts(&server).await;
        let commit_path = upload_path(&format!("/files/upload_sessions/{SESSION_ID}/commit"));
        Mock::given(method("POST"))
            .and(path(commit_path.as_str()))
            .respond_with(ResponseTemplate::new(202).insert_header("retry-after", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(commit_path.as_str()))
            .respond_with(ResponseTemplate::new(201).set_body_json(file_json("big.bin")))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await?;

        assert_eq!(file.name, "big.bin");
        assert_eq!(requests(&server, "POST", &commit_path).await.len(), 2);

        Ok(())
    }

    /// コミットでBOXが202を返し続けた場合は、指定した再送回数の上限に達した後にエラーを返すことを確認
    #[rstest]
    #[case::no_poll(0)]
    #[case::two_polls(2)]
    #[tokio::test]
    async fn test_upload_file_in_chunks_gives_up_when_never_committed(
        #[case] max_commit_polls: u32,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_session_and_parts(&server).await;
        Mock::given(method("DELETE"))
            .and(path(upload_path(&format!(
                "/files/upload_sessions/{SESSION_ID}"
            ))))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let commit_path = upload_path(&format!("/files/upload_sessions/{SESSION_ID}/commit"));
        Mock::given(method("POST"))
            .and(path(commit_path.as_str()))
            .respond_with(ResponseTemplate::new(202).insert_header("retry-after", "0"))
            .mount(&server)
            .await;
        let client = client_builder(&server, short_retry(0)?)?
            .max_commit_polls(max_commit_polls)?
            .build()
            .await?;

        let result = client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );
        // 最初のコミットと、上限までの再送を送信する。
        assert_eq!(
            requests(&server, "POST", &commit_path).await.len(),
            max_commit_polls as usize + 1
        );

        Ok(())
    }

    /// 部分の応答の遅延
    const PART_DELAY: Duration = Duration::from_millis(300);

    /// 並列数に応じて部分を並列にアップロードし、完了の順によらず位置の順に部分をコミットすることを確認
    ///
    /// 各部分の応答を同じ時間だけ遅らせ、アップロードにかかった時間から並列にアップロードしたかを判定する。
    /// 並列数が部分の数以上の場合は、最初の部分の応答を最も遅らせて、完了の順を位置の順と逆にする。
    /// 並列数が1の場合は、すべての部分を順にアップロードするため、遅延の合計以上の時間がかかる。
    #[rstest]
    #[case::sequential(1, [PART_DELAY; 3], PART_DELAY * 3, None)]
    #[case::parallel(
        3,
        [PART_DELAY * 2, PART_DELAY, Duration::ZERO],
        PART_DELAY * 2,
        Some(PART_DELAY * 3)
    )]
    #[tokio::test]
    async fn test_upload_file_in_chunks_concurrently(
        #[case] concurrency: usize,
        #[case] part_delays: [Duration; 3],
        #[case] min_elapsed: Duration,
        #[case] max_elapsed: Option<Duration>,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_session_and_delayed_parts(&server, part_delays).await;
        Mock::given(method("POST"))
            .and(path(upload_path(&format!(
                "/files/upload_sessions/{SESSION_ID}/commit"
            ))))
            .and(body_json(commit_body()))
            .respond_with(ResponseTemplate::new(201).set_body_json(file_json("big.bin")))
            .expect(1)
            .mount(&server)
            .await;
        let client =
            create_client_with_upload_concurrency(&server, short_retry(0)?, concurrency).await?;

        let started_at = std::time::Instant::now();
        client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await?;
        let elapsed = started_at.elapsed();

        assert!(min_elapsed <= elapsed, "elapsed: {elapsed:?}");
        if let Some(max_elapsed) = max_elapsed {
            assert!(elapsed < max_elapsed, "elapsed: {elapsed:?}");
        }

        Ok(())
    }

    /// 部分のアップロードに失敗した場合は、アップロードセッションを中止してからエラーを返すことを確認
    #[tokio::test]
    async fn test_upload_file_in_chunks_aborts_session_on_failure() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let session_path = upload_path(&format!("/files/upload_sessions/{SESSION_ID}"));
        Mock::given(method("POST"))
            .and(path(upload_path("/files/upload_sessions")))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "upload_session",
                "id": SESSION_ID,
                "part_size": PART_SIZE,
                "total_parts": 3,
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(session_path.as_str()))
            .respond_with(ResponseTemplate::new(416))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(session_path.as_str()))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 416),
            "result: {result:?}"
        );
        assert_eq!(requests(&server, "DELETE", &session_path).await.len(), 1);

        Ok(())
    }

    /// コミットに失敗した場合は、BOXがコミットを拒否したことが明らかな場合だけアップロードセッションを中止することを確認
    ///
    /// 5xxの場合は、BOXがコミットを処理している可能性があるため、中止しない。
    #[rstest]
    #[case::precondition_failed(412, 1)]
    #[case::internal_server_error(500, 0)]
    #[tokio::test]
    async fn test_upload_file_in_chunks_aborts_session_only_if_commit_rejected(
        #[case] status: u16,
        #[case] expected_aborts: usize,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_session_and_parts(&server).await;
        let session_path = upload_path(&format!("/files/upload_sessions/{SESSION_ID}"));
        Mock::given(method("POST"))
            .and(path(format!("{session_path}/commit")))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(session_path.as_str()))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(
            requests(&server, "DELETE", &session_path).await.len(),
            expected_aborts
        );

        Ok(())
    }

    /// アップロード中にファイルが大きくなった場合は、コミットせずにエラーを返すことを確認
    ///
    /// アップロードを始めたときのファイルサイズとして、実際より小さいサイズを渡し、ファイルが大きくなった状態を再現する。
    #[tokio::test]
    async fn test_upload_parts_rejects_grown_file() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(upload_path(&format!(
                "/files/upload_sessions/{SESSION_ID}"
            ))))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "part": {"part_id": "P", "offset": 0, "size": PART_SIZE, "sha1": "0"},
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let session = UploadSession {
            id: SESSION_ID.to_owned(),
            part_size: PART_SIZE as u64,
            total_parts: 2,
        };
        let mut file = tokio::fs::File::open(&file_path).await?;

        let result = client
            .upload_parts(&file_path, &mut file, 2 * PART_SIZE as u64, &session)
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::ReadFile { path, .. }) if *path == file_path),
            "result: {result:?}"
        );

        Ok(())
    }

    /// BOXクライアントに設定した、分割アップロードに切り替えるファイルサイズに従って、アップロードの方法を選ぶことを確認
    ///
    /// アップロードの方法だけを確かめるため、BOXはどちらのアップロードにもエラーを返す。
    #[rstest]
    // 既定値（50MiB）以下のファイルは、1回のリクエストでアップロードする。
    #[case::default_threshold(None, "/files/content")]
    // 最小値を指定した場合は、最小値を超えるファイルを分割アップロードする。
    #[case::min_threshold(Some(MIN_CHUNKED_UPLOAD_THRESHOLD), "/files/upload_sessions")]
    #[tokio::test]
    async fn test_upload_file_uses_chunked_upload_threshold(
        #[case] threshold: Option<u64>,
        #[case] expected_path: &str,
    ) -> anyhow::Result<()> {
        // 内容を書き込まずにサイズだけを設定し、大きいファイルをすばやく作成する。
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("big.bin");
        std::fs::File::create(&file_path)?.set_len(MIN_CHUNKED_UPLOAD_THRESHOLD + 1)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        for request_path in ["/files/content", "/files/upload_sessions"] {
            Mock::given(method("POST"))
                .and(path(upload_path(request_path)))
                .respond_with(ResponseTemplate::new(400))
                .mount(&server)
                .await;
        }
        let mut builder = client_builder(&server, short_retry(0)?)?;
        if let Some(threshold) = threshold {
            builder = builder.chunked_upload_threshold(threshold)?;
        }
        let client = builder.build().await?;

        let result = client
            .upload_file(&file_path, &FolderId::root(), &UploadFileOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 400),
            "result: {result:?}"
        );
        for request_path in ["/files/content", "/files/upload_sessions"] {
            let expected = usize::from(request_path == expected_path);
            assert_eq!(
                requests(&server, "POST", &upload_path(request_path))
                    .await
                    .len(),
                expected,
                "path: {request_path}"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod test_update_file_content {
    use std::path::PathBuf;

    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    use super::{
        chunked_upload_test_constants::{CHUNKED_CONTENT, PART_SIZE, SESSION_ID, THRESHOLD},
        *,
    };
    use crate::test_support::{
        create_client, if_match, mount_token, request_count, requests, short_retry, upload_path,
    };

    /// 一時ディレクトリに、指定した内容のファイルを作成する。
    fn create_file(content: &[u8]) -> anyhow::Result<(TempDir, PathBuf)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("report.txt");
        std::fs::write(&path, content)?;
        Ok((dir, path))
    }

    /// ファイル情報のJSONを返す。
    fn file_json(name: &str) -> serde_json::Value {
        json!({"type": "file", "id": "123", "etag": "2", "name": name})
    }

    /// 1回のリクエストで、`POST /files/{file_id}/content`に新しい内容を送信することを確認
    ///
    /// 新しいファイル名を指定した場合だけ`attributes`パートに名前を含め、`ETag`を指定した場合だけ
    /// `If-Match`ヘッダーを送信することも確認する。
    #[rstest]
    #[case::default(UpdateFileContentOptions::default(), "{}", None)]
    #[case::name_and_etag(
        UpdateFileContentOptions::default().with_name("report-v2.txt").with_etag("1"),
        r#"{"name":"report-v2.txt"}"#,
        Some("1")
    )]
    #[tokio::test]
    async fn test_update_file_content_directly(
        #[case] options: UpdateFileContentOptions<'static>,
        #[case] expected_attributes: &str,
        #[case] expected_if_match: Option<&str>,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file(b"hello")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let content_path = upload_path("/files/123/content");
        Mock::given(method("POST"))
            .and(path(content_path.as_str()))
            // "hello"のSHA-1
            .and(header(
                "content-md5",
                "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total_count": 1,
                "entries": [file_json("report.txt")],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .update_file_content_with_threshold(
                &FileId::new("123"),
                &file_path,
                &options,
                THRESHOLD,
            )
            .await?;

        assert_eq!(file.id, FileId::new("123"));
        let requests = requests(&server, "POST", &content_path).await;
        let [request] = requests.as_slice() else {
            anyhow::bail!("expected one request, got {}", requests.len());
        };
        assert_eq!(if_match(request).as_deref(), expected_if_match);
        let body = String::from_utf8_lossy(&request.body);
        let attributes = format!("\r\n\r\n{expected_attributes}\r\n");
        let attributes_at = body.find(&attributes);
        let content_at = body.find("\r\n\r\nhello\r\n");
        assert!(
            matches!((attributes_at, content_at), (Some(a), Some(c)) if a < c),
            "body: {body}"
        );

        Ok(())
    }

    /// BOXが拒否する新しいファイル名、またはヘッダーに使用できない文字を含む`ETag`を指定した場合は、
    /// リクエストを送信せずにエラーを返すことを確認
    ///
    /// 分割アップロードでは`ETag`をコミットで初めて送信するため、分割アップロードに切り替えるファイルサイズを超えるファイルでも、部分をアップロードする前に検証する。
    #[rstest]
    #[case::invalid_name(UpdateFileContentOptions::default().with_name("a/b"), "InvalidRequestBody")]
    #[case::invalid_etag(UpdateFileContentOptions::default().with_etag("1\n"), "InvalidHeaderValue")]
    #[tokio::test]
    async fn test_update_file_content_rejects_without_request(
        #[case] options: UpdateFileContentOptions<'static>,
        #[case] expected: &str,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file(CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .update_file_content_with_threshold(
                &FileId::new("123"),
                &file_path,
                &options,
                THRESHOLD,
            )
            .await;

        let actual = match &result {
            Err(BoxClientError::InvalidRequestBody(_)) => "InvalidRequestBody",
            Err(BoxClientError::InvalidHeaderValue(_)) => "InvalidHeaderValue",
            _ => "other",
        };
        assert_eq!(actual, expected, "result: {result:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// ファイルサイズが分割アップロードに切り替えるファイルサイズを超える場合は、`POST /files/{file_id}/upload_sessions`でアップロードセッションを作成し、
    /// 部分をアップロードしてから、`If-Match`ヘッダーを付けてコミットすることを確認
    #[tokio::test]
    async fn test_update_file_content_in_chunks() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file(CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(upload_path("/files/123/upload_sessions")))
            .and(body_json(json!({
                "file_size": CHUNKED_CONTENT.len(),
                "file_name": "report-v2.txt",
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "upload_session",
                "id": SESSION_ID,
                "part_size": PART_SIZE,
                "total_parts": 3,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(upload_path(&format!(
                "/files/upload_sessions/{SESSION_ID}"
            ))))
            .respond_with(|request: &Request| {
                // `Content-Range`ヘッダーから部分の位置とサイズを取り出し、アップロードした部分として返す。
                let range = request
                    .headers
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                let (start, end) = range
                    .trim_start_matches("bytes ")
                    .split_once('/')
                    .and_then(|(r, _)| r.split_once('-'))
                    .and_then(|(s, e)| Some((s.parse::<u64>().ok()?, e.parse::<u64>().ok()?)))
                    .unwrap_or_default();
                ResponseTemplate::new(200).set_body_json(json!({"part": {
                    "part_id": format!("P{start}"),
                    "offset": start,
                    "size": end - start + 1,
                }}))
            })
            .expect(3)
            .mount(&server)
            .await;
        let commit_path = upload_path(&format!("/files/upload_sessions/{SESSION_ID}/commit"));
        Mock::given(method("POST"))
            .and(path(commit_path.as_str()))
            .and(header("if-match", "1"))
            .and(header(
                "digest",
                sha1_digest_header(&Sha1::digest(CHUNKED_CONTENT)).as_str(),
            ))
            .and(body_json(json!({"parts": [
                {"part_id": "P0", "offset": 0, "size": 10},
                {"part_id": "P10", "offset": 10, "size": 10},
                {"part_id": "P20", "offset": 20, "size": 5},
            ]})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "total_count": 1,
                "entries": [file_json("report-v2.txt")],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = UpdateFileContentOptions::default()
            .with_name("report-v2.txt")
            .with_etag("1");

        let file = client
            .update_file_content_with_threshold(
                &FileId::new("123"),
                &file_path,
                &options,
                THRESHOLD,
            )
            .await?;

        assert_eq!(file.name, "report-v2.txt");
        // アップロードセッションの作成では、`If-Match`ヘッダーを送信しない。
        let sessions = requests(&server, "POST", &upload_path("/files/123/upload_sessions")).await;
        assert!(sessions.iter().all(|r| if_match(r).is_none()));

        Ok(())
    }
}

#[cfg(test)]
mod test_preflight_upload_file {
    use std::path::PathBuf;

    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use secrecy::ExposeSecret;

    use super::*;
    use crate::test_support::{create_client, mount_token, request_count, short_retry};

    /// プリフライトチェックのAPIのパス
    ///
    /// アップロードと異なり、REST APIのホストで受け付ける。
    const PREFLIGHT_PATH: &str = "/2.0/files/content";

    /// 一時ディレクトリに、5バイトのファイルを作成する。
    fn create_file() -> anyhow::Result<(TempDir, PathBuf)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("report.txt");
        std::fs::write(&path, b"hello")?;
        Ok((dir, path))
    }

    /// REST APIのホストに、ファイル名、ファイルサイズおよび親フォルダをOPTIONSで送信することを確認
    ///
    /// ファイル名を指定しない場合は、パスのファイル名を送信することも確認する。
    #[rstest]
    #[case::name_from_path(None, "report.txt")]
    #[case::name_from_options(Some("renamed.txt"), "renamed.txt")]
    #[tokio::test]
    async fn test_preflight_upload_file(
        #[case] option_name: Option<&'static str>,
        #[case] expected_name: &str,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("OPTIONS"))
            .and(path(PREFLIGHT_PATH))
            .and(body_json(json!({
                "name": expected_name,
                "size": 5,
                "parent": {"id": "456"},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "upload_url": "https://upload.box.com/api/2.0/files/content?upload_session_id=1234",
                "upload_token": "token",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = UploadFileOptions { name: option_name };

        let upload_url = client
            .preflight_upload_file(&file_path, &FolderId::new("456"), &options)
            .await?;

        assert_eq!(
            upload_url.upload_url.as_deref(),
            Some("https://upload.box.com/api/2.0/files/content?upload_session_id=1234")
        );
        assert_eq!(
            upload_url
                .upload_token
                .as_ref()
                .map(ExposeSecret::expose_secret),
            Some("token")
        );

        Ok(())
    }

    /// 成功レスポンスのボディが空の場合、または属性が省略された場合は、その属性を`None`として返すことを確認
    #[rstest]
    #[case::empty("", None)]
    #[case::without_token(
        r#"{"upload_url":"https://upload.box.com/x"}"#,
        Some("https://upload.box.com/x")
    )]
    #[tokio::test]
    async fn test_preflight_upload_file_optional_fields(
        #[case] body: &'static str,
        #[case] expected_url: Option<&str>,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("OPTIONS"))
            .and(path(PREFLIGHT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let upload_url = client
            .preflight_upload_file(
                &file_path,
                &FolderId::new("456"),
                &UploadFileOptions::default(),
            )
            .await?;

        assert_eq!(upload_url.upload_url.as_deref(), expected_url);
        assert!(upload_url.upload_token.is_none());

        Ok(())
    }

    /// アクセストークンを`Debug`で出力しないことを確認
    #[test]
    fn test_upload_url_debug_does_not_expose_token() -> anyhow::Result<()> {
        let upload_url: UploadUrl = serde_json::from_str(
            r#"{"upload_url":"https://upload.box.com/x","upload_token":"secret-token"}"#,
        )?;

        let debug = format!("{upload_url:?}");

        assert!(!debug.contains("secret-token"), "debug: {debug}");

        Ok(())
    }

    /// BOXがファイルを受け付けない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_preflight_upload_file_conflict() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("OPTIONS"))
            .and(path(PREFLIGHT_PATH))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "type": "error",
                "status": 409,
                "code": "item_name_in_use",
                "message": "Item with the same name already exists",
                "context_info": {"conflicts": {"type": "file", "id": "11", "name": "report.txt"}},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .preflight_upload_file(
                &file_path,
                &FolderId::new("456"),
                &UploadFileOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e))
                if e.status == 409 && e.body.contains("conflicts")),
            "result: {result:?}"
        );

        Ok(())
    }

    /// ファイル名が不正な場合、またはファイルが存在しない場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::invalid_name(Some("a/b"), "report.txt", "InvalidRequestBody")]
    #[case::missing_file(None, "missing.txt", "ReadFile")]
    #[tokio::test]
    async fn test_preflight_upload_file_rejects_without_request(
        #[case] option_name: Option<&'static str>,
        #[case] file_name: &str,
        #[case] expected: &str,
    ) -> anyhow::Result<()> {
        let (dir, _) = create_file()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;
        let options = UploadFileOptions { name: option_name };

        let result = client
            .preflight_upload_file(dir.path().join(file_name), &FolderId::root(), &options)
            .await;

        let matched = match expected {
            "ReadFile" => matches!(result, Err(BoxClientError::ReadFile { .. })),
            _ => matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
        };
        assert!(matched, "result: {result:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }
}

#[cfg(test)]
mod test_preflight_update_file_content {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, request_count, short_retry};

    /// 新しいバージョンのプリフライトチェックのAPIのパス
    ///
    /// アップロードと異なり、REST APIのホストで受け付ける。
    const PREFLIGHT_PATH: &str = "/2.0/files/123/content";

    /// ファイルサイズと、指定した場合だけ新しいファイル名を、REST APIのホストにOPTIONSで送信することを確認
    ///
    /// `ETag`を指定しても、`If-Match`ヘッダーを送信しないことも確認する。
    #[rstest]
    #[case::without_name(UpdateFileContentOptions::default(), json!({"size": 5}))]
    #[case::with_name_and_etag(
        UpdateFileContentOptions::default().with_name("report-v2.txt").with_etag("1"),
        json!({"name": "report-v2.txt", "size": 5})
    )]
    #[tokio::test]
    async fn test_preflight_update_file_content(
        #[case] options: UpdateFileContentOptions<'static>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("report.txt");
        std::fs::write(&file_path, b"hello")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("OPTIONS"))
            .and(path(PREFLIGHT_PATH))
            .and(body_json(expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "upload_url": "https://upload.box.com/api/2.0/files/123/content?upload_session_id=1234",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let upload_url = client
            .preflight_update_file_content(&FileId::new("123"), &file_path, &options)
            .await?;

        assert_eq!(
            upload_url.upload_url.as_deref(),
            Some("https://upload.box.com/api/2.0/files/123/content?upload_session_id=1234")
        );
        let requests: Vec<Request> = server.received_requests().await.unwrap_or_default();
        assert!(
            requests
                .iter()
                .filter(|r| r.url.path() == PREFLIGHT_PATH)
                .all(|r| !r.headers.contains_key("if-match"))
        );

        Ok(())
    }

    /// 新しいファイル名が不正な場合、またはファイルが存在しない場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::invalid_name(Some("a/b"), "report.txt", "InvalidRequestBody")]
    #[case::missing_file(None, "missing.txt", "ReadFile")]
    #[tokio::test]
    async fn test_preflight_update_file_content_rejects_without_request(
        #[case] option_name: Option<&'static str>,
        #[case] file_name: &str,
        #[case] expected: &str,
    ) -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("report.txt"), b"hello")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;
        let options = UpdateFileContentOptions {
            name: option_name,
            etag: None,
        };

        let result = client
            .preflight_update_file_content(
                &FileId::new("123"),
                dir.path().join(file_name),
                &options,
            )
            .await;

        let matched = match expected {
            "ReadFile" => matches!(result, Err(BoxClientError::ReadFile { .. })),
            _ => matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
        };
        assert!(matched, "result: {result:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }
}
