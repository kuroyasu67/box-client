//! ファイルに関するAPI

use serde::Deserialize;
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientResult, FileId, FolderItem, ItemStatus, PathCollection, UserMini,
    request::ApiRequest,
};

/// ファイル情報
///
/// フォルダ情報と同様に、BOXがnullを返したり省略したりしても取得を失敗させないため、
/// IDおよび名前以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct File {
    /// ファイルID
    pub id: FileId,

    /// ファイルのバージョンを識別するETag
    pub etag: Option<String>,

    /// ファイルの変更を識別するシーケンスID
    pub sequence_id: Option<String>,

    /// ファイル名
    pub name: String,

    /// ファイルの内容のSHA-1ハッシュ値
    pub sha1: Option<String>,

    /// ファイルの現在のバージョン
    pub file_version: Option<FileVersion>,

    /// ファイルの説明
    pub description: Option<String>,

    /// ファイルのバイト数
    pub size: Option<u64>,

    /// ルートフォルダからこのファイルの親フォルダまでのパス
    pub path_collection: Option<PathCollection>,

    /// BOX上でファイルを作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// BOX上でファイルを最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// ファイルをごみ箱に移動した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,

    /// ごみ箱に移動したファイルを完全に削除する予定の日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub purged_at: Option<OffsetDateTime>,

    /// ファイルの内容を作成した日時
    ///
    /// アップロードしたクライアントが指定した日時であり、BOX上での作成日時と異なる場合がある。
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub content_created_at: Option<OffsetDateTime>,

    /// ファイルの内容を最後に更新した日時
    ///
    /// アップロードしたクライアントが指定した日時であり、BOX上での更新日時と異なる場合がある。
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub content_modified_at: Option<OffsetDateTime>,

    /// ファイルを作成したユーザー
    pub created_by: Option<UserMini>,

    /// ファイルを最後に更新したユーザー
    pub modified_by: Option<UserMini>,

    /// ファイルを所有するユーザー
    pub owned_by: Option<UserMini>,

    /// 親フォルダ
    pub parent: Option<FolderItem>,

    /// ファイルの状態
    pub item_status: Option<ItemStatus>,
}

/// ファイルのバージョン
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileVersion {
    /// ファイルバージョンID
    pub id: String,

    /// このバージョンのファイルの内容のSHA-1ハッシュ値
    pub sha1: Option<String>,
}

impl BoxClient {
    /// ファイル情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 情報を取得するファイルのID
    ///
    /// # Errors
    ///
    /// ファイルが存在しない場合、またはファイルにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn file_information(&self, file_id: &FileId) -> BoxClientResult<File> {
        self.send_json(ApiRequest::get(["files", file_id.as_ref()]))
            .await
    }
}

#[cfg(test)]
mod test_file {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;
    use crate::FolderId;

    /// 日時をRFC 3339形式で解析し、バージョン、パス、親フォルダおよびユーザーをデシリアライズすることを確認
    #[test]
    fn test_deserialize() -> anyhow::Result<()> {
        let user = json!({"type": "user", "id": "33224", "name": "Test User", "login": "user@example.com"});
        let file: File = serde_json::from_value(json!({
            "type": "file",
            "id": "12345",
            "etag": "1",
            "sequence_id": "3",
            "name": "Contract.pdf",
            "sha1": "85136c79cbf9fe36bb9d05d0639c70c265c18d37",
            "file_version": {
                "type": "file_version",
                "id": "67890",
                "sha1": "85136c79cbf9fe36bb9d05d0639c70c265c18d37",
            },
            "description": "Contract for the project",
            "size": 629_644,
            "path_collection": {
                "total_count": 2,
                "entries": [
                    {"type": "folder", "id": "0", "sequence_id": null, "etag": null, "name": "All Files"},
                    {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Legal"},
                ],
            },
            "created_at": "2012-12-12T10:53:43-08:00",
            "modified_at": "2012-12-12T11:15:04+09:00",
            "trashed_at": null,
            "purged_at": null,
            "content_created_at": "2012-12-12T10:53:43Z",
            "content_modified_at": "2012-12-12T11:15:04Z",
            "created_by": user,
            "modified_by": user,
            "owned_by": user,
            "shared_link": null,
            "parent": {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Legal"},
            "item_status": "active",
        }))?;

        let user = UserMini {
            id: "33224".to_owned(),
            user_type: "user".to_owned(),
            name: "Test User".to_owned(),
            login: "user@example.com".to_owned(),
        };
        let legal = FolderItem {
            id: FolderId::new("111"),
            name: "Legal".to_owned(),
            etag: Some("0".to_owned()),
        };
        assert_eq!(
            file,
            File {
                id: FileId::new("12345"),
                etag: Some("1".to_owned()),
                sequence_id: Some("3".to_owned()),
                name: "Contract.pdf".to_owned(),
                sha1: Some("85136c79cbf9fe36bb9d05d0639c70c265c18d37".to_owned()),
                file_version: Some(FileVersion {
                    id: "67890".to_owned(),
                    sha1: Some("85136c79cbf9fe36bb9d05d0639c70c265c18d37".to_owned()),
                }),
                description: Some("Contract for the project".to_owned()),
                size: Some(629_644),
                path_collection: Some(PathCollection {
                    total_count: 2,
                    entries: vec![
                        FolderItem {
                            id: FolderId::root(),
                            name: "All Files".to_owned(),
                            etag: None,
                        },
                        legal.clone(),
                    ],
                }),
                created_at: Some(datetime!(2012-12-12 10:53:43 -08:00)),
                modified_at: Some(datetime!(2012-12-12 11:15:04 +09:00)),
                trashed_at: None,
                purged_at: None,
                content_created_at: Some(datetime!(2012-12-12 10:53:43 UTC)),
                content_modified_at: Some(datetime!(2012-12-12 11:15:04 UTC)),
                created_by: Some(user.clone()),
                modified_by: Some(user.clone()),
                owned_by: Some(user),
                parent: Some(legal),
                item_status: Some(ItemStatus::Active),
            }
        );

        Ok(())
    }

    /// IDおよび名前以外の属性が欠落しているファイルをデシリアライズすることを確認
    #[test]
    fn test_deserialize_minimal() -> anyhow::Result<()> {
        let file: File = serde_json::from_value(json!({
            "type": "file",
            "id": "12345",
            "name": "Contract.pdf",
        }))?;

        assert_eq!(file.id, FileId::new("12345"));
        assert_eq!(file.name, "Contract.pdf");
        assert_eq!(file.file_version, None);
        assert_eq!(file.created_at, None);
        assert_eq!(file.parent, None);

        Ok(())
    }

    /// RFC 3339形式でない日時の場合は、デシリアライズに失敗することを確認
    #[test]
    fn test_deserialize_invalid_datetime() {
        let result = serde_json::from_value::<File>(json!({
            "id": "12345",
            "name": "Contract.pdf",
            "modified_at": "2012/12/12 11:15:04",
        }));

        assert!(result.is_err(), "result: {result:?}");
    }
}

#[cfg(test)]
mod test_file_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError, FolderId,
        test_support::{create_client, mount_token, short_retry},
    };

    /// GET /files/{file_id}を呼び出し、ファイル情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_file_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/files/12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "file",
                "id": "12345",
                "etag": "1",
                "name": "Contract.pdf",
                "size": 629_644,
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client.file_information(&FileId::new("12345")).await?;

        assert_eq!(file.id, FileId::new("12345"));
        assert_eq!(file.name, "Contract.pdf");
        assert_eq!(file.size, Some(629_644));
        assert_eq!(file.parent.map(|p| p.id), Some(FolderId::root()));

        Ok(())
    }

    /// ファイルが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_file_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/files/999"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client.file_information(&FileId::new("999")).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}
