//! ファイルのダウンロードに関するAPI
//!
//! BOXは、ファイルの内容を要求されると、`dl.boxcloud.com`のダウンロード用のURLへリダイレクトする302を返す。
//! HTTPクライアントはリダイレクトを自動で追跡し、ダウンロード用のURLからファイルの内容を受信する。
//! HTTPクライアントは、ホストが異なるURLへリダイレクトするときに`Authorization`ヘッダーを送信しないため、
//! アクセストークンを`dl.boxcloud.com`へ送信しない。

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use reqwest::{Response, StatusCode};
use tokio::io::AsyncWriteExt as _;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, FileId, FileVersionId,
    request::ApiRequest,
    retry::{AttemptError, Idempotency, classify_body_error, parse_retry_after},
};

/// ファイルのダウンロードのオプション
///
/// # Examples
///
/// ```
/// use box_client::{DownloadFileOptions, FileVersionId};
///
/// let version = FileVersionId::new("12345");
/// let options = DownloadFileOptions::default().with_version(&version);
/// assert_eq!(options.version, Some(&version));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DownloadFileOptions<'a> {
    /// ダウンロードするバージョンのID
    ///
    /// `None`の場合は、ファイルの現在のバージョンをダウンロードする。
    pub version: Option<&'a FileVersionId>,
}

impl<'a> DownloadFileOptions<'a> {
    /// ダウンロードするバージョンのIDを設定する。
    ///
    /// 過去のバージョンのIDは、[`BoxClient::list_file_versions`]で取得できる。
    #[must_use]
    pub fn with_version(mut self, version: &'a FileVersionId) -> Self {
        self.version = Some(version);
        self
    }
}

/// ファイルのダウンロードの1回のリクエストの結果
#[derive(Debug)]
enum DownloadOutcome {
    /// ファイルの内容を一時ファイルに書き込んだ。
    Downloaded {
        /// 書き込んだバイト数
        size: u64,
    },

    /// BOXがダウンロードの準備を終えていないため、リクエストを再送する必要がある。
    NotReady {
        /// `Retry-After`ヘッダーが示す待機時間
        retry_after: Option<std::time::Duration>,
    },
}

/// ダウンロードしている内容を書き込む一時ファイル
///
/// ダウンロードに失敗した場合、またはダウンロードしているfutureが破棄された場合に、
/// 書きかけの一時ファイルを残さないように、[`TempFile::persist`]を呼び出さずに破棄すると一時ファイルを削除する。
#[derive(Debug)]
struct TempFile {
    /// 一時ファイルのパス
    path: PathBuf,

    /// 一時ファイルを保存先に移動したかどうか
    persisted: bool,
}

impl TempFile {
    /// 保存先と同じディレクトリに、保存先のファイル名とUUIDから一時ファイルのパスを決める。
    ///
    /// 一時ファイルは、保存先と同じファイルシステムに作成して、保存先への移動で内容を複製しないようにする。
    /// 一時ファイルは、ダウンロードの開始時に作成する。
    ///
    /// # Errors
    ///
    /// 保存先のパスからファイル名を取得できない場合は、[`BoxClientError::InvalidArgument`]を返す。
    fn new(path: &Path) -> BoxClientResult<Self> {
        let file_name = path.file_name().ok_or(BoxClientError::InvalidArgument(
            "the file name cannot be taken from the path",
        ))?;
        let mut temp_name = OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".{}.download", uuid::Uuid::new_v4().simple()));

        Ok(Self {
            path: path.with_file_name(temp_name),
            persisted: false,
        })
    }

    /// 一時ファイルを保存先に移動する。
    ///
    /// 保存先にファイルが存在する場合は、そのファイルを置き換える。
    async fn persist(mut self, path: &Path) -> std::io::Result<()> {
        tokio::fs::rename(&self.path, path).await?;
        self.persisted = true;
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.persisted {
            // 一時ファイルを作成する前に失敗した場合は、一時ファイルが存在しないため、削除の失敗を無視する。
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// ダウンロードの成功レスポンスのボディを、一時ファイルに書き込む。
///
/// BOXは、アップロードの直後などでダウンロードの準備ができていない場合に、202と`Retry-After`ヘッダーを返す。
/// 再試行のたびに一時ファイルを作成し直すため、受信の途中で失敗して再試行した場合も、内容を先頭から書き込む。
///
/// # Arguments
///
/// - `response`: 成功レスポンス
/// - `idempotency`: リクエストの冪等性
/// - `temp_path`: 一時ファイルのパス
/// - `path`: エラーに含める保存先のパス
async fn write_response(
    mut response: Response,
    idempotency: Idempotency,
    temp_path: &Path,
    path: &Path,
) -> Result<DownloadOutcome, AttemptError> {
    if response.status() == StatusCode::ACCEPTED {
        return Ok(DownloadOutcome::NotReady {
            retry_after: parse_retry_after(response.headers()),
        });
    }

    let write_error = |source: std::io::Error| {
        AttemptError::Fatal(BoxClientError::WriteFile {
            path: path.to_owned(),
            source,
        })
    };
    let mut file = tokio::fs::File::create(temp_path)
        .await
        .map_err(write_error)?;
    let mut size = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| classify_body_error(e, idempotency))?
    {
        file.write_all(&chunk).await.map_err(write_error)?;
        size += chunk.len() as u64;
    }
    // 保存先に移動した後に内容が欠けないように、書き込みを完了させてからディスクに反映する。
    file.flush().await.map_err(write_error)?;
    file.sync_all().await.map_err(write_error)?;

    Ok(DownloadOutcome::Downloaded { size })
}

impl BoxClient {
    /// ファイルの内容をダウンロードし、ローカルのファイルに保存する。
    ///
    /// 内容をメモリに読み込まずに、受信した順に保存先と同じディレクトリの一時ファイルへ書き込む。
    /// すべての内容を受信した後に、一時ファイルを保存先に移動する。
    /// したがって、ダウンロードに失敗した場合は、保存先に書きかけのファイルを残さず、保存先に存在したファイルも変更しない。
    /// 保存先にファイルが存在する場合は、ダウンロードしたファイルで置き換える。
    ///
    /// BOXがダウンロードの準備ができていないことを示す202を返した場合は、`Retry-After`ヘッダーが示す時間だけ
    /// 待機してから、リクエストを再送する。再送する回数の上限は、[`BoxClient::max_download_polls`]が返す値とする。
    /// `Retry-After`ヘッダーがない場合、または待機時間の上限を超える場合は、
    /// 再試行設定に従って待機時間を決める。
    ///
    /// [`BoxClientTimeout::request_timeout`](crate::BoxClientTimeout::request_timeout)は、内容の受信の完了までに適用される。
    /// したがって、大きいファイルをダウンロードする場合は、通信速度に応じて、ファイル全体を受信できる長さにする。
    ///
    /// # Arguments
    ///
    /// - `file_id`: ダウンロードするファイルのID
    /// - `path`: ダウンロードしたファイルを保存するパス。親ディレクトリは存在している必要がある。
    /// - `options`: ダウンロードするバージョンのオプション
    ///
    /// # Returns
    ///
    /// 保存したファイルのバイト数を返す。
    ///
    /// # Errors
    ///
    /// `path`からファイル名を取得できない場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// 一時ファイルの作成、書き込み、または保存先への移動に失敗した場合は、[`BoxClientError::WriteFile`]を返す。
    /// ファイルまたはバージョンが存在しない場合、またはファイルにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 202を受け取ってリクエストを再送した回数が、[`BoxClientBuilder::max_download_polls`](crate::BoxClientBuilder::max_download_polls)で指定した上限に達した場合は、
    /// [`BoxClientError::UnexpectedResponse`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn download_file(
        &self,
        file_id: &FileId,
        path: impl AsRef<Path>,
        options: &DownloadFileOptions<'_>,
    ) -> BoxClientResult<u64> {
        let path = path.as_ref();
        let temp_file = TempFile::new(path)?;

        let mut polls = 0;
        let size = loop {
            let mut request = ApiRequest::get(["files", file_id.as_ref(), "content"]);
            if let Some(version) = options.version {
                request = request.query("version", version.as_ref());
            }
            let outcome = self
                .send(request, |response, idempotency| {
                    write_response(response, idempotency, &temp_file.path, path)
                })
                .await?;
            let retry_after = match outcome {
                DownloadOutcome::Downloaded { size } => break size,
                DownloadOutcome::NotReady { retry_after } => retry_after,
            };

            if self.max_download_polls() <= polls {
                return Err(BoxClientError::UnexpectedResponse(
                    "file was not ready to download before reaching max download polls",
                ));
            }
            polls += 1;

            // `ThreadRng`は`Send`を実装していないため、待機時間の計算と待機を別の文に分け、`ThreadRng`を待機の前に破棄する。
            let policy = self.retry_policy();
            let delay = policy
                .retry_delay(polls, retry_after, &mut rand::rng())
                .unwrap_or_else(|_| policy.max_delay());
            tracing::debug!(
                file_id = file_id.as_ref(),
                polls,
                delay_ms = delay.as_millis(),
                "file is not ready to download; resending request"
            );
            tokio::time::sleep(delay).await;
        };

        temp_file
            .persist(path)
            .await
            .map_err(|source| BoxClientError::WriteFile {
                path: path.to_owned(),
                source,
            })?;
        tracing::debug!(file_id = file_id.as_ref(), size, "downloaded file");

        Ok(size)
    }
}

#[cfg(test)]
mod test_download_file {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use reqwest::Url;
    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::{io::AsyncReadExt as _, net::TcpListener};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    use super::*;
    use crate::test_support::{
        TOKEN_PATH, client_builder, create_client, mount_token, short_retry, test_config,
    };

    /// ダウンロードするファイルのID
    const FILE_ID: &str = "11";

    /// ダウンロードのAPIのパス
    const CONTENT_PATH: &str = "/2.0/files/11/content";

    /// ダウンロードするファイルの内容
    const CONTENT: &[u8] = b"hello, box";

    /// 保存先のディレクトリと、そのディレクトリ内の保存先のパスを返す。
    fn destination() -> anyhow::Result<(TempDir, PathBuf)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("downloaded.txt");
        Ok((dir, path))
    }

    /// ディレクトリ内のファイル名をソートして返す。
    fn file_names(dir: &TempDir) -> anyhow::Result<Vec<String>> {
        let mut names = std::fs::read_dir(dir.path())?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect::<anyhow::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }

    /// ファイルの内容を保存先に保存し、保存したバイト数を返すことを確認
    ///
    /// バージョンを指定した場合は、`version`クエリパラメーターで送信することも確認する。
    #[rstest]
    #[case::current(None)]
    #[case::version(Some("22"))]
    #[tokio::test]
    async fn test_download_file(#[case] version: Option<&str>) -> anyhow::Result<()> {
        let (dir, file_path) = destination()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let mut mock = Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .and(header("authorization", "Bearer access-token"));
        if let Some(version) = version {
            mock = mock.and(query_param("version", version));
        }
        mock.respond_with(ResponseTemplate::new(200).set_body_bytes(CONTENT))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let version = version.map(FileVersionId::new);
        let options = DownloadFileOptions {
            version: version.as_ref(),
        };

        let size = client
            .download_file(&FileId::new(FILE_ID), &file_path, &options)
            .await?;

        assert_eq!(size, CONTENT.len() as u64);
        assert_eq!(std::fs::read(&file_path)?, CONTENT);
        // 一時ファイルを残さない。
        assert_eq!(file_names(&dir)?, ["downloaded.txt"]);

        Ok(())
    }

    /// 保存先にファイルが存在する場合は、ダウンロードしたファイルで置き換えることを確認
    #[tokio::test]
    async fn test_download_file_replaces_existing_file() -> anyhow::Result<()> {
        let (_dir, file_path) = destination()?;
        std::fs::write(&file_path, b"old content that is longer than the new one")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(CONTENT))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        client
            .download_file(
                &FileId::new(FILE_ID),
                &file_path,
                &DownloadFileOptions::default(),
            )
            .await?;

        assert_eq!(std::fs::read(&file_path)?, CONTENT);

        Ok(())
    }

    /// 別のホストのダウンロード用のURLへのリダイレクトを追跡し、そのホストにアクセストークンを送信しないことを確認
    #[tokio::test]
    async fn test_download_file_follows_redirect_without_token() -> anyhow::Result<()> {
        let (_dir, file_path) = destination()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        // ポートが異なるため、HTTPクライアントは別のホストとして扱う。
        let download_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/d/1/download"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(CONTENT))
            .expect(1)
            .mount(&download_server)
            .await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "location",
                format!("{}/d/1/download", download_server.uri()),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let size = client
            .download_file(
                &FileId::new(FILE_ID),
                &file_path,
                &DownloadFileOptions::default(),
            )
            .await?;

        assert_eq!(size, CONTENT.len() as u64);
        assert_eq!(std::fs::read(&file_path)?, CONTENT);
        let requests = download_server
            .received_requests()
            .await
            .unwrap_or_default();
        let [request] = requests.as_slice() else {
            anyhow::bail!("expected one request, got {}", requests.len());
        };
        assert!(
            !request.headers.contains_key("authorization"),
            "headers: {:?}",
            request.headers
        );

        Ok(())
    }

    /// BOXが202を返した場合は、待機してからリクエストを再送することを確認
    #[tokio::test]
    async fn test_download_file_waits_until_ready() -> anyhow::Result<()> {
        let (_dir, file_path) = destination()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(202).insert_header("retry-after", "0"))
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(CONTENT))
            .expect(1)
            .mount(&server)
            .await;
        // 202の後の再送が再試行回数に含まれないことも確かめるため、再試行回数を0にする。
        let client = create_client(&server, short_retry(0)?).await?;

        client
            .download_file(
                &FileId::new(FILE_ID),
                &file_path,
                &DownloadFileOptions::default(),
            )
            .await?;

        assert_eq!(std::fs::read(&file_path)?, CONTENT);

        Ok(())
    }

    /// BOXが202を返し続けた場合は、指定した再送回数の上限に達した後にエラーを返すことを確認
    #[rstest]
    #[case::no_poll(0)]
    #[case::two_polls(2)]
    #[tokio::test]
    async fn test_download_file_gives_up_when_never_ready(
        #[case] max_download_polls: u32,
    ) -> anyhow::Result<()> {
        let (dir, file_path) = destination()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(202).insert_header("retry-after", "0"))
            // 最初のリクエストと、上限までの再送を受け付ける。
            .expect(u64::from(max_download_polls) + 1)
            .mount(&server)
            .await;
        let client = client_builder(&server, short_retry(0)?)?
            .max_download_polls(max_download_polls)?
            .build()
            .await?;

        let result = client
            .download_file(
                &FileId::new(FILE_ID),
                &file_path,
                &DownloadFileOptions::default(),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );
        assert_eq!(file_names(&dir)?, Vec::<String>::new());

        Ok(())
    }

    /// ダウンロードに失敗した場合は、保存先に存在したファイルを変更せず、一時ファイルも残さないことを確認
    #[tokio::test]
    async fn test_download_file_not_found_keeps_existing_file() -> anyhow::Result<()> {
        let (dir, file_path) = destination()?;
        std::fs::write(&file_path, b"existing")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .download_file(
                &FileId::new(FILE_ID),
                &file_path,
                &DownloadFileOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );
        assert_eq!(std::fs::read(&file_path)?, b"existing");
        assert_eq!(file_names(&dir)?, ["downloaded.txt"]);

        Ok(())
    }

    /// 1回目の接続ではボディの途中で接続を切り、2回目以降の接続では内容をすべて返すHTTPサーバーを起動する。
    ///
    /// 戻り値は、サーバーを指すREST APIのベースURLと、サーバーが受け付けた接続の数である。
    async fn start_truncating_once_server() -> anyhow::Result<(Url, Arc<AtomicUsize>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let api_base_url = Url::parse(&format!("http://{}/2.0", listener.local_addr()?))?;
        let connections = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&connections);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let connection = counter.fetch_add(1, Ordering::SeqCst);
                // リクエストヘッダーを読み終えてから、レスポンスを返す。
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let header = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n",
                    CONTENT.len()
                );
                let body = if connection == 0 {
                    &CONTENT[..CONTENT.len() / 2]
                } else {
                    CONTENT
                };
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body).await;
            }
        });

        Ok((api_base_url, connections))
    }

    /// ボディの受信の途中で接続が切れた場合は、内容を先頭から受信し直して保存することを確認
    #[tokio::test]
    async fn test_download_file_retries_from_start() -> anyhow::Result<()> {
        let (dir, file_path) = destination()?;
        let token_server = MockServer::start().await;
        mount_token(&token_server).await;
        let (api_base_url, connections) = start_truncating_once_server().await?;
        let client = BoxClient::builder(test_config())
            .retry(short_retry(1)?)
            .endpoints(api_base_url, format!("{}{TOKEN_PATH}", token_server.uri()))
            .build()
            .await?;

        let size = client
            .download_file(
                &FileId::new(FILE_ID),
                &file_path,
                &DownloadFileOptions::default(),
            )
            .await?;

        assert_eq!(size, CONTENT.len() as u64);
        // 1回目に受信した内容の後ろに追記せず、2回目に受信した内容だけを保存する。
        assert_eq!(std::fs::read(&file_path)?, CONTENT);
        assert_eq!(file_names(&dir)?, ["downloaded.txt"]);
        assert_eq!(connections.load(Ordering::SeqCst), 2);

        Ok(())
    }

    /// 保存先のパスからファイル名を取得できない場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_download_file_rejects_path_without_file_name() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .download_file(
                &FileId::new(FILE_ID),
                Path::new(".."),
                &DownloadFileOptions::default(),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        // BOXクライアントの作成時に送信したトークンリクエストだけを受信している。
        assert_eq!(
            server.received_requests().await.unwrap_or_default().len(),
            1
        );

        Ok(())
    }

    /// 保存先の親ディレクトリが存在しない場合は、書き込みエラーを返すことを確認
    #[tokio::test]
    async fn test_download_file_missing_directory() -> anyhow::Result<()> {
        let (dir, _) = destination()?;
        let file_path = dir.path().join("missing").join("downloaded.txt");
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(CONTENT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(CONTENT))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .download_file(
                &FileId::new(FILE_ID),
                &file_path,
                &DownloadFileOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::WriteFile { path, .. }) if *path == file_path),
            "result: {result:?}"
        );

        Ok(())
    }
}
