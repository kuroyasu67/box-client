//! ゴミ箱に関するAPI

use serde::Serialize;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, BoxItem, Folder, FolderId,
    api::folders::{ItemsPage, MAX_ITEMS_LIMIT, SortDirection, collect_all_pages, validate_limit},
    request::ApiRequest,
};

/// ゴミ箱内の項目をリストするときのオプション
///
/// BOXはマーカー方式のページ送りと`sort`クエリパラメーターの併用を受け付けないため、
/// 並び替えに使用する属性を指定できない。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListTrashedItemsOptions {
    /// 1ページで取得する項目数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,

    /// 並び替えの方向
    pub direction: Option<SortDirection>,
}

/// ゴミ箱内のフォルダを復元するときのオプション
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RestoreFolderOptions<'a> {
    /// 復元後のフォルダ名
    ///
    /// 復元先に同じ名前の項目がある場合に、別の名前で復元するために指定する。
    /// `None`の場合は、ゴミ箱に移動する前の名前で復元する。
    pub name: Option<&'a str>,

    /// 元の親フォルダが存在しない場合の復元先の親フォルダのID
    ///
    /// BOXは、元の親フォルダが存在しない場合にだけこのIDを使用する。
    /// 元の親フォルダが存在する場合は、このIDを指定しても元の親フォルダに復元する。
    pub parent_id: Option<&'a FolderId>,
}

/// フォルダを復元するリクエストのボディ
#[derive(Debug, Serialize)]
struct RestoreFolderBody<'a> {
    /// 復元後のフォルダ名
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,

    /// 元の親フォルダが存在しない場合の復元先の親フォルダ
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<ParentReference<'a>>,
}

/// リクエストボディで親フォルダを指定する参照
#[derive(Debug, Serialize)]
struct ParentReference<'a> {
    /// 親フォルダのID
    id: &'a str,
}

impl<'a> From<&RestoreFolderOptions<'a>> for RestoreFolderBody<'a> {
    fn from(options: &RestoreFolderOptions<'a>) -> Self {
        Self {
            name: options.name,
            parent: options
                .parent_id
                .map(|id| ParentReference { id: id.as_ref() }),
        }
    }
}

impl BoxClient {
    /// ゴミ箱内のフォルダの情報を取得する。
    ///
    /// 取得できるのは、ゴミ箱に直接移動したフォルダだけである。
    /// フォルダをゴミ箱に移動すると、そのフォルダの子フォルダ、孫フォルダなど、すべての子孫フォルダもゴミ箱に入る。
    /// ただし、子孫フォルダはゴミ箱に直接移動したフォルダではないため、子孫フォルダを指定するとBOXは404を返す。
    /// 子孫フォルダの状態を確認する場合は、その子孫フォルダの祖先のうち、ゴミ箱に直接移動したフォルダの情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 情報を取得するゴミ箱内のフォルダのID
    ///
    /// # Errors
    ///
    /// ルートフォルダはゴミ箱に移動できないため、ルートフォルダを指定した場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 404: フォルダがゴミ箱に直接移動されていない、フォルダが存在しない、
    ///   またはフォルダにアクセスする権限がない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn trashed_folder_information(
        &self,
        folder_id: &FolderId,
    ) -> BoxClientResult<Folder> {
        if folder_id.is_root() {
            return Err(BoxClientError::InvalidArgument(
                "the root folder cannot be in the trash",
            ));
        }

        self.send_json(ApiRequest::get(["folders", folder_id.as_ref(), "trash"]))
            .await
    }

    /// ゴミ箱内のフォルダを復元する。
    ///
    /// 復元できるのは、ゴミ箱に直接移動したフォルダだけである。
    /// BOXはフォルダを元の親フォルダに復元する。元の親フォルダが存在しない場合に限り、
    /// `options.parent_id`で指定したフォルダに復元する。
    ///
    /// 復元中は、復元するフォルダ、復元するフォルダの子孫にあたるすべての項目および復元先のフォルダをBOXがロックする。
    /// ロックしている間は、それらのフォルダを対象とする移動、コピー、削除および復元を実行できない。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがフォルダの復元を完了している可能性がある。
    /// 復元に成功したフォルダはゴミ箱からなくなるため、BOXが復元した後にリクエストを再送すると、BOXは404を返す。
    /// その結果、復元に成功したにもかかわらず、復元に失敗したと誤って判断するおそれがある。
    /// したがって、このリクエストを冪等でないリクエストとして送信し、BOXがリクエストを処理していないことが
    /// 明らかな429および接続エラーの後にのみ再試行する。
    /// POSTリクエストは既定で冪等でないリクエストとして扱われるため、冪等性を上書きしていない。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 復元するゴミ箱内のフォルダのID
    /// - `options`: 復元後のフォルダ名および復元先の親フォルダのオプション
    ///
    /// # Errors
    ///
    /// ルートフォルダはゴミ箱に移動できないため、ルートフォルダを指定した場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: ゴミ箱からフォルダを復元する権限がない、または復元先のフォルダにアクセスする権限がない。
    /// - 404: フォルダがゴミ箱にない。
    /// - 409: 次のいずれかの理由で復元できない。
    ///   - 復元先に同じ名前のフォルダがある。`options.name`に別の名前を指定して再度呼び出す。
    ///   - 移動、コピー、削除または復元の操作が、復元するフォルダまたは復元先のフォルダをロックしている。
    ///     エラーコードは`operation_blocked_temporary`である。時間をおいて再度呼び出す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがフォルダを復元した可能性があるため、
    /// [`BoxClient::folder_information`]でフォルダの状態を確認する。
    pub async fn restore_folder(
        &self,
        folder_id: &FolderId,
        options: &RestoreFolderOptions<'_>,
    ) -> BoxClientResult<Folder> {
        if folder_id.is_root() {
            return Err(BoxClientError::InvalidArgument(
                "the root folder cannot be in the trash",
            ));
        }

        let request = ApiRequest::post(["folders", folder_id.as_ref()])
            .json(&RestoreFolderBody::from(options))?;
        let folder: Folder = self.send_json(request).await?;

        tracing::info!(
            folder_id = folder_id.as_ref(),
            parent_id = folder.parent.as_ref().map(|p| p.id.as_ref()),
            "restored folder"
        );

        Ok(folder)
    }

    /// ゴミ箱内の項目を1ページ分リストする。
    ///
    /// BOXは、ユーザーがゴミ箱に直接移動したファイル、フォルダおよびウェブリンクを返す。
    /// ゴミ箱に直接移動したフォルダの子孫にあたる項目は、ゴミ箱に入っていても返さない。
    ///
    /// 1回の呼び出しで返す項目は最大で`limit`件であるため、ゴミ箱に後続の項目が残っている
    /// 場合がある。後続の項目がある場合は、戻り値の[`ItemsPage::next_marker`]が`Some`になる。
    /// そのマーカーを`marker`に指定して再度呼び出すと、次のページを取得できる。
    /// すべての項目が必要な場合は、[`BoxClient::list_all_trashed_items`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `marker`: 前のページの[`ItemsPage::next_marker`]。最初のページを取得する場合は`None`
    /// - `options`: 項目数の上限および並び替えの方向のオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`crate::BoxClientError::InvalidArgument`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_trashed_items(
        &self,
        marker: Option<&str>,
        options: &ListTrashedItemsOptions,
    ) -> BoxClientResult<ItemsPage> {
        let mut request = ApiRequest::get(["folders", "trash", "items"]).query("usemarker", "true");
        if let Some(limit) = options.limit {
            request = request.query("limit", validate_limit(limit)?.to_string());
        }
        if let Some(marker) = marker {
            request = request.query("marker", marker);
        }
        if let Some(direction) = options.direction {
            request = request.query("direction", direction.as_str());
        }

        self.send_json(request).await
    }

    /// ゴミ箱内のすべての項目を、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。すべての項目をメモリに保持するため、
    /// 項目数が非常に多いゴミ箱では[`BoxClient::list_trashed_items`]でページごとに処理する。
    ///
    /// # Arguments
    ///
    /// - `options`: 項目数の上限および並び替えの方向のオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`crate::BoxClientError::InvalidArgument`]を返す。
    /// BOXが同じマーカーを返し続ける場合は、無限ループを避けるため[`crate::BoxClientError::UnexpectedResponse`]を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みの項目を破棄してエラーを返す。
    pub async fn list_all_trashed_items(
        &self,
        options: &ListTrashedItemsOptions,
    ) -> BoxClientResult<Vec<BoxItem>> {
        let options = ListTrashedItemsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
            ..*options
        };
        let (items, pages) =
            collect_all_pages(async |marker| self.list_trashed_items(marker, &options).await)
                .await?;

        tracing::info!(pages, items = items.len(), "listed all trashed items");

        Ok(items)
    }
}

#[cfg(test)]
mod test_trashed_folder_information {
    use serde_json::json;
    use time::macros::datetime;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        ItemStatus,
        test_support::{create_client, mount_token, request_count, short_retry},
    };

    /// GET /folders/{folder_id}/trashを呼び出し、ゴミ箱に移動した日時と状態を含むフォルダ情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_trashed_folder_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/folders/12345/trash"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "folder",
                "id": "12345",
                "etag": "1",
                "sequence_id": "3",
                "name": "Contracts",
                "trashed_at": "2012-12-12T10:53:43-08:00",
                "purged_at": "2013-01-11T10:53:43-08:00",
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
                "item_status": "trashed",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let folder = client
            .trashed_folder_information(&FolderId::new("12345"))
            .await?;

        assert_eq!(folder.id, FolderId::new("12345"));
        assert_eq!(folder.name, "Contracts");
        assert_eq!(
            folder.trashed_at,
            Some(datetime!(2012-12-12 10:53:43 -08:00))
        );
        assert_eq!(
            folder.purged_at,
            Some(datetime!(2013-01-11 10:53:43 -08:00))
        );
        assert_eq!(folder.item_status, Some(ItemStatus::Trashed));

        Ok(())
    }

    /// ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_trashed_folder_information_rejects_root() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client.trashed_folder_information(&FolderId::root()).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// フォルダがゴミ箱に直接移動されていない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_trashed_folder_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/folders/999/trash"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .trashed_folder_information(&FolderId::new("999"))
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_restore_folder_body {
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::*;

    /// 指定したオプションだけをリクエストボディにシリアライズすることを確認
    #[rstest]
    #[case::default(None, None, json!({}))]
    #[case::name(Some("restored"), None, json!({"name": "restored"}))]
    #[case::parent(None, Some("111"), json!({"parent": {"id": "111"}}))]
    #[case::name_and_parent(
        Some("restored"),
        Some("111"),
        json!({"name": "restored", "parent": {"id": "111"}})
    )]
    fn test_serialize(
        #[case] name: Option<&str>,
        #[case] parent_id: Option<&str>,
        #[case] expected: Value,
    ) -> anyhow::Result<()> {
        let parent_id = parent_id.map(FolderId::new);
        let options = RestoreFolderOptions {
            name,
            parent_id: parent_id.as_ref(),
        };

        let body = serde_json::to_value(RestoreFolderBody::from(&options))?;

        assert_eq!(body, expected);

        Ok(())
    }
}

#[cfg(test)]
mod test_restore_folder {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::{
        ItemStatus,
        test_support::{create_client, mount_token, request_count, short_retry},
    };

    /// フォルダのAPIのパス
    const FOLDER_PATH: &str = "/2.0/folders/123";

    /// 復元したフォルダのレスポンスを返す。
    fn restored_response() -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(json!({
            "type": "folder",
            "id": "123",
            "etag": "2",
            "name": "restored",
            "trashed_at": null,
            "purged_at": null,
            "parent": {"type": "folder", "id": "111", "etag": "0", "name": "Legal"},
            "item_status": "active",
        }))
    }

    /// フォルダのAPIへのPOSTリクエストの数を返す。
    async fn post_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == FOLDER_PATH)
            .count()
    }

    /// POST /folders/{folder_id}にオプションをボディとして送信し、復元したフォルダ情報を返すことを確認
    #[tokio::test]
    async fn test_restore_folder() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .and(body_json(
                json!({"name": "restored", "parent": {"id": "111"}}),
            ))
            .respond_with(restored_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let parent_id = FolderId::new("111");
        let options = RestoreFolderOptions {
            name: Some("restored"),
            parent_id: Some(&parent_id),
        };

        let folder = client
            .restore_folder(&FolderId::new("123"), &options)
            .await?;

        assert_eq!(folder.id, FolderId::new("123"));
        assert_eq!(folder.name, "restored");
        assert_eq!(folder.parent.map(|p| p.id), Some(parent_id));
        assert_eq!(folder.trashed_at, None);
        assert_eq!(folder.item_status, Some(ItemStatus::Active));

        Ok(())
    }

    /// ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_restore_folder_rejects_root() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .restore_folder(&FolderId::root(), &RestoreFolderOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、復元済みのフォルダに対して404を受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::conflict_or_locked(409)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_restore_folder_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .restore_folder(&FolderId::new("123"), &RestoreFolderOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(post_count(&server).await, 1);

        Ok(())
    }

    /// 429の場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[tokio::test]
    async fn test_restore_folder_retries_too_many_requests() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .and(body_json(json!({})))
            .respond_with(restored_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;

        client
            .restore_folder(&FolderId::new("123"), &RestoreFolderOptions::default())
            .await?;

        assert_eq!(post_count(&server).await, 2);

        Ok(())
    }
}

#[cfg(test)]
mod test_list_trashed_items {
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

    /// ゴミ箱の項目のAPIのパス
    const TRASH_ITEMS_PATH: &str = "/2.0/folders/trash/items";

    /// ゴミ箱の項目のAPIへのリクエストのクエリ文字列を返す。
    async fn trash_queries(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.url.path() == TRASH_ITEMS_PATH)
            .map(|r| r.url.query().unwrap_or_default().to_owned())
            .collect()
    }

    /// GET /folders/trash/itemsを呼び出し、マーカーとオプションをクエリパラメーターに変換することを確認
    #[rstest]
    #[case::default(None, ListTrashedItemsOptions::default(), "usemarker=true")]
    #[case::min_limit(
        None,
        ListTrashedItemsOptions { limit: Some(1), direction: None },
        "usemarker=true&limit=1"
    )]
    #[case::all(
        Some("m1"),
        ListTrashedItemsOptions {
            limit: Some(1000),
            direction: Some(SortDirection::Asc),
        },
        "usemarker=true&limit=1000&marker=m1&direction=ASC"
    )]
    #[tokio::test]
    async fn test_list_trashed_items(
        #[case] marker: Option<&str>,
        #[case] options: ListTrashedItemsOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "folder", "id": "2", "name": "docs", "etag": "1"}],
                "limit": 100,
                "next_marker": "m2",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let page = client.list_trashed_items(marker, &options).await?;

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.next_marker.as_deref(), Some("m2"));
        assert_eq!(trash_queries(&server).await, vec![expected_query]);

        Ok(())
    }

    /// 項目数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::zero(0)]
    #[case::over_max(1001)]
    #[tokio::test]
    async fn test_list_trashed_items_invalid_limit(#[case] limit: u32) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListTrashedItemsOptions {
            limit: Some(limit),
            ..ListTrashedItemsOptions::default()
        };

        let result = client.list_trashed_items(None, &options).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert!(trash_queries(&server).await.is_empty());

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_trashed_items {
    use rstest::rstest;
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;
    use crate::{
        BoxClientError, FolderId, FolderItem,
        test_support::{create_client, mount_token, short_retry},
    };

    /// ゴミ箱の項目のAPIのパス
    const TRASH_ITEMS_PATH: &str = "/2.0/folders/trash/items";

    /// 指定したIDのフォルダだけを含むページのレスポンスを返す。
    fn page_response(id: &str, next_marker: Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "entries": [{"type": "folder", "id": id, "name": id, "etag": "0"}],
            "limit": 1000,
            "next_marker": next_marker,
        }))
    }

    /// 指定したIDのフォルダの項目を返す。
    fn folder(id: &str) -> BoxItem {
        BoxItem::Folder(FolderItem {
            id: FolderId::new(id),
            name: id.to_owned(),
            etag: Some("0".to_owned()),
        })
    }

    /// next_markerがnullまたは空文字列になるまでページを送り、すべての項目を返すことを確認
    #[rstest]
    #[case::null(Value::Null)]
    #[case::empty(json!(""))]
    #[tokio::test]
    async fn test_list_all_trashed_items(#[case] last_marker: Value) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        // 項目数の上限を省略した場合は、上限の1000件ずつ取得する
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("second", last_marker))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let items = client
            .list_all_trashed_items(&ListTrashedItemsOptions::default())
            .await?;

        assert_eq!(items, vec![folder("first"), folder("second")]);

        Ok(())
    }

    /// 項目数の上限と並び替えの方向を指定した場合は、すべてのページの取得に使用することを確認
    #[tokio::test]
    async fn test_list_all_trashed_items_with_options() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("limit", "1"))
            .and(query_param("direction", "DESC"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("limit", "1"))
            .and(query_param("direction", "DESC"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("second", Value::Null))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListTrashedItemsOptions {
            limit: Some(1),
            direction: Some(SortDirection::Desc),
        };

        let items = client.list_all_trashed_items(&options).await?;

        assert_eq!(items, vec![folder("first"), folder("second")]);

        Ok(())
    }

    /// BOXが同じマーカーを返し続ける場合は、無限ループせずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_trashed_items_marker_not_advanced() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .respond_with(page_response("same", json!("m1")))
            .expect(2)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .list_all_trashed_items(&ListTrashedItemsOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );

        Ok(())
    }

    /// いずれかのページの取得に失敗した場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_list_all_trashed_items_page_error() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", json!("m1")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("marker", "m1"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .list_all_trashed_items(&ListTrashedItemsOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 403),
            "result: {result:?}"
        );

        Ok(())
    }
}
