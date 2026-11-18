//! Webリンクに関するAPI

use serde::Deserialize;
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientResult, FolderItem, Idempotency, ItemStatus, PathCollection, UserMini,
    WebLinkId, request::ApiRequest,
};

/// Webリンク情報
///
/// フォルダ情報およびファイル情報と同様に、BOXがnullを返したり省略したりしても取得を失敗させないため、
/// IDおよび名前以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct WebLink {
    /// WebリンクID
    pub id: WebLinkId,

    /// Webリンクのバージョンを識別する`ETag`
    pub etag: Option<String>,

    /// Webリンクの変更を識別するシーケンスID
    pub sequence_id: Option<String>,

    /// Webリンク名
    pub name: String,

    /// Webリンクが指すURL
    pub url: Option<String>,

    /// Webリンクの説明
    pub description: Option<String>,

    /// ルートフォルダからこのWebリンクの親フォルダまでのパス
    pub path_collection: Option<PathCollection>,

    /// BOX上でWebリンクを作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// BOX上でWebリンクを最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// Webリンクをゴミ箱に移動した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,

    /// ゴミ箱に移動したWebリンクを完全に削除する予定の日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub purged_at: Option<OffsetDateTime>,

    /// Webリンクを作成したユーザー
    pub created_by: Option<UserMini>,

    /// Webリンクを最後に更新したユーザー
    pub modified_by: Option<UserMini>,

    /// Webリンクを所有するユーザー
    pub owned_by: Option<UserMini>,

    /// 親フォルダ
    pub parent: Option<FolderItem>,

    /// Webリンクの状態
    pub item_status: Option<ItemStatus>,
}

impl BoxClient {
    /// Webリンク情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 情報を取得するWebリンクのID
    ///
    /// # Errors
    ///
    /// Webリンクが存在しない場合、またはWebリンクにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn web_link_information(&self, web_link_id: &WebLinkId) -> BoxClientResult<WebLink> {
        self.send_json(ApiRequest::get(["web_links", web_link_id.as_ref()]))
            .await
    }

    /// Webリンクを削除する。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがWebリンクの削除を完了している可能性がある。
    /// BOXが削除した後にリクエストを再送すると、削除済みのWebリンクに対する削除となるため、BOXはエラーを返しうる。
    /// その結果、削除に成功したにもかかわらず、削除に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// BOXがリクエストを処理していないことが明らかな429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 削除するWebリンクのID
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがWebリンクを削除した可能性があるため、
    /// [`BoxClient::web_link_information`]でWebリンクの状態を確認する。
    pub async fn delete_web_link(&self, web_link_id: &WebLinkId) -> BoxClientResult<()> {
        let request = ApiRequest::delete(["web_links", web_link_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent);

        self.send_empty(request).await?;

        tracing::debug!(web_link_id = web_link_id.as_ref(), "deleted web link");

        Ok(())
    }
}

#[cfg(test)]
mod test_web_link {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;
    use crate::FolderId;

    /// 日時をRFC 3339形式で解析し、URL、パス、親フォルダおよびユーザーをデシリアライズすることを確認
    #[test]
    fn test_deserialize() -> anyhow::Result<()> {
        let user = json!({"type": "user", "id": "33224", "name": "Test User", "login": "user@example.com"});
        let web_link: WebLink = serde_json::from_value(json!({
            "type": "web_link",
            "id": "12345",
            "etag": "1",
            "sequence_id": "3",
            "name": "Example",
            "url": "https://www.example.com/?a=1&b=2",
            "description": "Example web site",
            "path_collection": {
                "total_count": 2,
                "entries": [
                    {"type": "folder", "id": "0", "sequence_id": null, "etag": null, "name": "All Files"},
                    {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Links"},
                ],
            },
            "created_at": "2012-12-12T10:53:43-08:00",
            "modified_at": "2012-12-12T11:15:04+09:00",
            "trashed_at": null,
            "purged_at": null,
            "created_by": user,
            "modified_by": user,
            "owned_by": user,
            "shared_link": null,
            "parent": {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Links"},
            "item_status": "active",
        }))?;

        let user = UserMini {
            id: crate::UserId::new("33224"),
            user_type: "user".to_owned(),
            name: Some("Test User".to_owned()),
            login: Some("user@example.com".to_owned()),
        };
        let links = FolderItem {
            id: FolderId::new("111"),
            name: "Links".to_owned(),
            etag: Some("0".to_owned()),
        };
        assert_eq!(
            web_link,
            WebLink {
                id: WebLinkId::new("12345"),
                etag: Some("1".to_owned()),
                sequence_id: Some("3".to_owned()),
                name: "Example".to_owned(),
                url: Some("https://www.example.com/?a=1&b=2".to_owned()),
                description: Some("Example web site".to_owned()),
                path_collection: Some(PathCollection {
                    total_count: 2,
                    entries: vec![
                        FolderItem {
                            id: FolderId::root(),
                            name: "All Files".to_owned(),
                            etag: None,
                        },
                        links.clone(),
                    ],
                }),
                created_at: Some(datetime!(2012-12-12 10:53:43 -08:00)),
                modified_at: Some(datetime!(2012-12-12 11:15:04 +09:00)),
                trashed_at: None,
                purged_at: None,
                created_by: Some(user.clone()),
                modified_by: Some(user.clone()),
                owned_by: Some(user),
                parent: Some(links),
                item_status: Some(ItemStatus::Active),
            }
        );

        Ok(())
    }

    /// IDおよび名前以外の属性が欠落しているWebリンクをデシリアライズすることを確認
    #[test]
    fn test_deserialize_minimal() -> anyhow::Result<()> {
        let web_link: WebLink = serde_json::from_value(json!({
            "type": "web_link",
            "id": "12345",
            "name": "Example",
        }))?;

        assert_eq!(web_link.id, WebLinkId::new("12345"));
        assert_eq!(web_link.name, "Example");
        assert_eq!(web_link.url, None);
        assert_eq!(web_link.created_at, None);
        assert_eq!(web_link.parent, None);

        Ok(())
    }

    /// RFC 3339形式でない日時の場合は、デシリアライズに失敗することを確認
    #[test]
    fn test_deserialize_invalid_datetime() {
        let result = serde_json::from_value::<WebLink>(json!({
            "id": "12345",
            "name": "Example",
            "created_at": "2012/12/12 10:53:43",
        }));

        assert!(result.is_err(), "result: {result:?}");
    }
}

#[cfg(test)]
mod test_web_link_information {
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

    /// `GET /web_links/{web_link_id}`を呼び出し、Webリンク情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_web_link_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/web_links/12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "web_link",
                "id": "12345",
                "etag": "1",
                "name": "Example",
                "url": "https://www.example.com/",
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let web_link = client
            .web_link_information(&WebLinkId::new("12345"))
            .await?;

        assert_eq!(web_link.id, WebLinkId::new("12345"));
        assert_eq!(web_link.name, "Example");
        assert_eq!(web_link.url.as_deref(), Some("https://www.example.com/"));
        assert_eq!(web_link.parent.map(|p| p.id), Some(FolderId::root()));

        Ok(())
    }

    /// Webリンクが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_web_link_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/web_links/999"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client.web_link_information(&WebLinkId::new("999")).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_delete_web_link {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError,
        test_support::{create_client, mount_token, short_retry},
    };

    /// WebリンクのAPIのパス
    const WEB_LINK_PATH: &str = "/2.0/web_links/123";

    /// WebリンクのAPIへのDELETEリクエストの数を返す。
    async fn delete_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| r.method.as_str() == "DELETE" && r.url.path() == WEB_LINK_PATH)
            .count()
    }

    /// `DELETE /web_links/{web_link_id}`を呼び出し、204を受け取ると成功することを確認
    #[tokio::test]
    async fn test_delete_web_link() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(WEB_LINK_PATH))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        client.delete_web_link(&WebLinkId::new("123")).await?;

        assert_eq!(delete_count(&server).await, 1);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、削除済みのWebリンクに対してエラーを受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_delete_web_link_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(WEB_LINK_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client.delete_web_link(&WebLinkId::new("123")).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(delete_count(&server).await, 1);

        Ok(())
    }

    /// 429または他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[rstest]
    #[case::too_many_requests(ResponseTemplate::new(429))]
    #[case::operation_blocked_temporary(ResponseTemplate::new(409).set_body_json(json!({
        "type": "error",
        "status": 409,
        "code": "operation_blocked_temporary",
        "message": "The operation is blocked by another on-going operation",
    })))]
    #[tokio::test]
    async fn test_delete_web_link_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(WEB_LINK_PATH))
            .respond_with(first_response)
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(WEB_LINK_PATH))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;

        client.delete_web_link(&WebLinkId::new("123")).await?;

        assert_eq!(delete_count(&server).await, 2);

        Ok(())
    }
}
