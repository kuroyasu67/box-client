//! フォルダに関するAPI

use std::collections::HashSet;

use serde::Deserialize;
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, BoxItem, FolderId, FolderItem, Idempotency,
    ItemStatus, UserMini, api::types::PathCollection, request::ApiRequest,
};
use reqwest::header::IF_MATCH;

/// 1回のリクエストで取得できる項目数の上限
pub const MAX_ITEMS_LIMIT: u32 = 1000;

/// フォルダ情報
///
/// ルートフォルダでは、`ETag`、作成日時、親フォルダなどの多くの属性をBOXがnullで返すため、
/// IDおよび名前以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Folder {
    /// フォルダID
    pub id: FolderId,

    /// フォルダのバージョンを識別する`ETag`
    pub etag: Option<String>,

    /// フォルダの変更を識別するシーケンスID
    pub sequence_id: Option<String>,

    /// フォルダ名
    pub name: String,

    /// フォルダの説明
    pub description: Option<String>,

    /// フォルダ内のすべてのファイルの合計バイト数
    pub size: Option<u64>,

    /// ルートフォルダからこのフォルダの親フォルダまでのパス
    pub path_collection: Option<PathCollection>,

    /// BOX上でフォルダを作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// BOX上でフォルダを最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// フォルダをゴミ箱に移動した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,

    /// ゴミ箱に移動したフォルダを完全に削除する予定の日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub purged_at: Option<OffsetDateTime>,

    /// フォルダの内容を作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub content_created_at: Option<OffsetDateTime>,

    /// フォルダの内容を最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub content_modified_at: Option<OffsetDateTime>,

    /// フォルダを作成したユーザー
    pub created_by: Option<UserMini>,

    /// フォルダを最後に更新したユーザー
    pub modified_by: Option<UserMini>,

    /// フォルダを所有するユーザー
    pub owned_by: Option<UserMini>,

    /// 親フォルダ
    ///
    /// ルートフォルダの場合は`None`になる。
    pub parent: Option<FolderItem>,

    /// フォルダの状態
    pub item_status: Option<ItemStatus>,
}

/// フォルダ内の項目の1ページ
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct ItemsPage {
    /// このページの項目
    pub entries: Vec<BoxItem>,

    /// このページの取得に使用した項目数の上限
    pub limit: u32,

    /// 次のページを取得するためのマーカー
    ///
    /// 後続の項目がない場合は`None`になる。
    pub next_marker: Option<String>,
}

/// 項目の並び替えに使用する属性
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemSort {
    /// ID
    Id,

    /// 名前
    Name,

    /// 作成日時
    Date,

    /// サイズ
    Size,
}

impl ItemSort {
    /// クエリパラメーターに指定する文字列を返す。
    fn as_str(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Name => "name",
            Self::Date => "date",
            Self::Size => "size",
        }
    }
}

/// 並び替えの方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    /// 昇順
    Asc,

    /// 降順
    Desc,
}

impl SortDirection {
    /// クエリパラメーターに指定する文字列を返す。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// フォルダ内の項目をリストするときのオプション
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListItemsOptions {
    /// 1ページで取得する項目数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,

    /// 並び替えに使用する属性
    ///
    /// BOXはルートフォルダでマーカー方式のページ送りと並び替えの併用を受け付けない。
    /// このクライアントは常にマーカー方式でページを送るため、ルートフォルダでは指定できない。
    pub sort: Option<ItemSort>,

    /// 並び替えの方向
    pub direction: Option<SortDirection>,
}

/// フォルダを削除するときのオプション
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteFolderOptions<'a> {
    /// 空でないフォルダを、フォルダ内のすべての項目とともに削除することを指示するフラグ
    ///
    /// `false`の場合に空でないフォルダを削除しようとすると、BOXは400（`folder_not_empty`）を返す。
    pub recursive: bool,

    /// 最後に取得したフォルダの`ETag`
    ///
    /// 指定した場合は、`If-Match`ヘッダーで送信する。フォルダが`ETag`の取得後に変更されていると、
    /// BOXはフォルダを削除せずに412を返す。
    pub etag: Option<&'a str>,
}

/// 1ページで取得する項目数の上限が、BOXが受け付ける範囲内であることを検証する。
pub(crate) fn validate_limit(limit: u32) -> BoxClientResult<u32> {
    if (1..=MAX_ITEMS_LIMIT).contains(&limit) {
        Ok(limit)
    } else {
        Err(BoxClientError::InvalidArgument(
            "limit must be between 1 and 1000",
        ))
    }
}

/// マーカー方式のページ送りで、すべてのページの項目を取得する。
///
/// `fetch_page`には前のページの`next_marker`を渡す。最初のページでは`None`を渡す。
/// 戻り値は、すべての項目と取得したページ数である。
///
/// # Errors
///
/// BOXが以前に返したマーカーを再び返した場合、この関数は無限ループを避けるため[`BoxClientError::UnexpectedResponse`]を返す。
/// `fetch_page`がエラーを返した場合、この関数は取得済みの項目を破棄してそのエラーを返す。
pub(crate) async fn collect_all_pages<F>(mut fetch_page: F) -> BoxClientResult<(Vec<BoxItem>, u32)>
where
    F: AsyncFnMut(Option<&str>) -> BoxClientResult<ItemsPage>,
{
    let mut items = Vec::new();
    let mut marker: Option<String> = None;
    // `m1 → m2 → m1`のように、BOXが以前のマーカーを再び返す循環も検出するため、使用したマーカーを記録する。
    // 現在のマーカーは`marker`が保持するため、このセットには現在のマーカーより前に使用したマーカーだけを記録する。
    let mut used_markers: HashSet<String> = HashSet::new();
    let mut pages: u32 = 0;

    loop {
        let page = fetch_page(marker.as_deref()).await?;
        pages += 1;
        items.extend(page.entries);

        // BOXは最終ページで`next_marker`をnullにするが、空文字列を返す場合もあるため、どちらも終端として扱う。
        match page.next_marker {
            Some(next) if !next.is_empty() => {
                if marker.as_deref() == Some(next.as_str()) || used_markers.contains(&next) {
                    return Err(BoxClientError::UnexpectedResponse(
                        "next_marker was already returned",
                    ));
                }
                if let Some(previous) = marker.replace(next) {
                    used_markers.insert(previous);
                }
            }
            _ => break,
        }
    }

    Ok((items, pages))
}

impl BoxClient {
    /// フォルダ情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 情報を取得するフォルダのID。ルートフォルダの場合は[`FolderId::root`]を指定する。
    ///
    /// # Errors
    ///
    /// フォルダが存在しない場合、またはフォルダにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn folder_information(&self, folder_id: &FolderId) -> BoxClientResult<Folder> {
        self.send_json(ApiRequest::get(["folders", folder_id.as_ref()]))
            .await
    }

    /// フォルダを削除する。
    ///
    /// BOXは、ゴミ箱が有効な場合はフォルダをゴミ箱に移動し、無効な場合はフォルダを完全に削除する。
    /// BOXでは、ゴミ箱は既定で有効である。
    ///
    /// 項目の多いフォルダの削除には時間がかかる。BOXは削除に600秒以上かかると503を返すが、削除処理はその後も継続する。
    /// ただし、[`BoxClientTimeout::request_timeout`](crate::BoxClientTimeout::request_timeout)の既定値は30秒であるため、
    /// 通常はBOXが503を返す前にクライアントがタイムアウトする。タイムアウトした場合も、BOXは削除処理を継続している可能性がある。
    /// 503を受け取るまで待つ場合は、[`BoxClientBuilder::timeout`](crate::BoxClientBuilder::timeout)で
    /// `request_timeout`を600秒より長くする。
    ///
    /// 503を受け取った後、またはタイムアウトの後に再送すると、削除済みのフォルダに対して404を受け取る可能性がある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// 429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 削除するフォルダのID
    /// - `options`: 再帰的に削除するかどうか、および`ETag`のオプション
    ///
    /// # Errors
    ///
    /// ルートフォルダを指定した場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 400: `options.recursive`が`false`で、フォルダが空でない。
    /// - 403: フォルダを削除する権限がない、またはフォルダがロックされている。
    /// - 404: フォルダが存在しない、またはフォルダにアクセスする権限がない。
    /// - 409: 移動、コピーなど、他の操作がフォルダをロックしている。エラーコードが`operation_blocked_temporary`の
    ///   場合、クライアントは再試行設定に従って再試行し、再試行回数の上限に達してもロックが解除されない場合に
    ///   このエラーを返す。
    /// - 412: `options.etag`がフォルダの現在の`ETag`と一致しない。
    /// - 500: BOXの内部エラー。BOXがフォルダを削除した可能性がある。
    /// - 503: 削除に600秒以上かかっている。削除処理は継続している。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// タイムアウトした場合は、`is_timeout()`が`true`を返す[`BoxClientError::Http`]を返す。
    ///
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがフォルダを削除したか、削除処理を継続している可能性がある。
    /// したがって、[`BoxClient::folder_information`]でフォルダの状態を確認する。
    /// ゴミ箱が有効な場合は、[`BoxClient::trashed_folder_information`]でゴミ箱に移動したことも確認できる。
    pub async fn delete_folder(
        &self,
        folder_id: &FolderId,
        options: &DeleteFolderOptions<'_>,
    ) -> BoxClientResult<()> {
        if folder_id.is_root() {
            return Err(BoxClientError::InvalidArgument(
                "the root folder cannot be deleted",
            ));
        }

        let mut request = ApiRequest::delete(["folders", folder_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent);
        if options.recursive {
            request = request.query("recursive", "true");
        }
        if let Some(etag) = options.etag {
            request = request.header(IF_MATCH, etag)?;
        }

        self.send_empty(request).await?;

        tracing::info!(
            folder_id = folder_id.as_ref(),
            recursive = options.recursive,
            "deleted folder"
        );

        Ok(())
    }

    /// フォルダ内の項目を1ページ分リストする。
    ///
    /// 1回の呼び出しで返す項目は最大で`limit`件であるため、フォルダに後続の項目が残っている
    /// 場合がある。後続の項目がある場合は、戻り値の[`ItemsPage::next_marker`]が`Some`になる。
    /// そのマーカーを`marker`に指定して再度呼び出すと、次のページを取得できる。
    /// すべての項目が必要な場合は、[`BoxClient::list_all_items_in_folder`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 項目をリストするフォルダのID
    /// - `marker`: 前のページの[`ItemsPage::next_marker`]。最初のページを取得する場合は`None`
    /// - `options`: 項目数の上限、並び替えなどのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ルートフォルダで`options.sort`を指定した場合も、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_items_in_folder(
        &self,
        folder_id: &FolderId,
        marker: Option<&str>,
        options: &ListItemsOptions,
    ) -> BoxClientResult<ItemsPage> {
        if folder_id.is_root() && options.sort.is_some() {
            return Err(BoxClientError::InvalidArgument(
                "sort is not supported for marker-based pagination in the root folder",
            ));
        }

        let mut request =
            ApiRequest::get(["folders", folder_id.as_ref(), "items"]).query("usemarker", "true");
        if let Some(limit) = options.limit {
            request = request.query("limit", validate_limit(limit)?.to_string());
        }
        if let Some(marker) = marker {
            request = request.query("marker", marker);
        }
        if let Some(sort) = options.sort {
            request = request.query("sort", sort.as_str());
        }
        if let Some(direction) = options.direction {
            request = request.query("direction", direction.as_str());
        }

        self.send_json(request).await
    }

    /// フォルダ内のすべての項目を、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。すべての項目をメモリに保持するため、
    /// 項目数が非常に多いフォルダでは[`BoxClient::list_items_in_folder`]でページごとに処理する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 項目をリストするフォルダのID
    /// - `options`: 項目数の上限、並び替えなどのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ルートフォルダで`options.sort`を指定した場合も、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXが以前に返したマーカーを再び返した場合、このメソッドは無限ループを避けるため[`BoxClientError::UnexpectedResponse`]を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みの項目を破棄してエラーを返す。
    pub async fn list_all_items_in_folder(
        &self,
        folder_id: &FolderId,
        options: &ListItemsOptions,
    ) -> BoxClientResult<Vec<BoxItem>> {
        let options = ListItemsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
            ..*options
        };
        let (items, pages) = collect_all_pages(async |marker| {
            self.list_items_in_folder(folder_id, marker, &options).await
        })
        .await?;

        tracing::info!(
            folder_id = folder_id.as_ref(),
            pages,
            items = items.len(),
            "listed all items in folder"
        );

        Ok(items)
    }
}

#[cfg(test)]
mod test_folder_id {
    use super::*;

    /// ルートフォルダのIDが0であることを確認
    #[test]
    fn test_root() {
        assert_eq!(FolderId::root().as_ref(), "0");
    }
}

#[cfg(test)]
mod test_folder {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;

    /// 日時をRFC 3339形式で解析し、パス、親フォルダおよびユーザーをデシリアライズすることを確認
    #[test]
    fn test_deserialize() -> anyhow::Result<()> {
        let user = json!({"type": "user", "id": "33224", "name": "Test User", "login": "user@example.com"});
        let folder: Folder = serde_json::from_value(json!({
            "type": "folder",
            "id": "12345",
            "etag": "1",
            "sequence_id": "3",
            "name": "Contracts",
            "description": "Legal contracts",
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
            "parent": {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Legal"},
            "item_status": "active",
            "item_collection": {"total_count": 0, "entries": []},
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
            folder,
            Folder {
                id: FolderId::new("12345"),
                etag: Some("1".to_owned()),
                sequence_id: Some("3".to_owned()),
                name: "Contracts".to_owned(),
                description: Some("Legal contracts".to_owned()),
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

    /// 日時、親フォルダなどがnullまたは欠落しているルートフォルダをデシリアライズすることを確認
    #[test]
    fn test_deserialize_root() -> anyhow::Result<()> {
        let folder: Folder = serde_json::from_value(json!({
            "type": "folder",
            "id": "0",
            "etag": null,
            "sequence_id": null,
            "name": "All Files",
            "created_at": null,
            "modified_at": null,
            "path_collection": {"total_count": 0, "entries": []},
            "parent": null,
            "item_status": "active",
        }))?;

        assert_eq!(folder.id, FolderId::root());
        assert_eq!(folder.etag, None);
        assert_eq!(folder.created_at, None);
        // レスポンスに存在しない日時の属性も`None`として受け入れる。
        assert_eq!(folder.trashed_at, None);
        assert_eq!(folder.parent, None);
        assert_eq!(folder.path_collection.map(|p| p.entries.len()), Some(0));

        Ok(())
    }

    /// RFC 3339形式でない日時の場合は、デシリアライズに失敗することを確認
    #[test]
    fn test_deserialize_invalid_datetime() {
        let result = serde_json::from_value::<Folder>(json!({
            "id": "12345",
            "name": "Contracts",
            "created_at": "2012/12/12 10:53:43",
        }));

        assert!(result.is_err(), "result: {result:?}");
    }
}

#[cfg(test)]
mod test_folder_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// `GET /folders/{folder_id}`を呼び出し、フォルダ情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_folder_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/folders/12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "folder",
                "id": "12345",
                "etag": "1",
                "name": "Contracts",
                "created_at": "2012-12-12T10:53:43-08:00",
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let folder = client.folder_information(&FolderId::new("12345")).await?;

        assert_eq!(folder.id, FolderId::new("12345"));
        assert_eq!(folder.name, "Contracts");
        assert_eq!(folder.parent.map(|p| p.id), Some(FolderId::root()));

        Ok(())
    }

    /// フォルダが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_folder_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/folders/999"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client.folder_information(&FolderId::new("999")).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_delete_folder {
    use rstest::rstest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, request_count, short_retry};

    /// フォルダのAPIのパス
    const FOLDER_PATH: &str = "/2.0/folders/123";

    /// フォルダのAPIへのリクエストのクエリ文字列とIf-Matchヘッダーを返す。
    async fn delete_requests(server: &MockServer) -> Vec<(String, Option<String>)> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "DELETE" && r.url.path() == FOLDER_PATH)
            .map(|r| {
                let query = r.url.query().unwrap_or_default().to_owned();
                let if_match = r
                    .headers
                    .get("if-match")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                (query, if_match)
            })
            .collect()
    }

    /// オプションに応じて、recursiveクエリパラメーターとIf-Matchヘッダーを送信することを確認
    #[rstest]
    #[case::default(DeleteFolderOptions::default(), "", None)]
    #[case::recursive(
        DeleteFolderOptions { recursive: true, etag: None },
        "recursive=true",
        None
    )]
    #[case::etag(
        DeleteFolderOptions { recursive: false, etag: Some("1") },
        "",
        Some("1")
    )]
    #[case::recursive_and_etag(
        DeleteFolderOptions { recursive: true, etag: Some("1") },
        "recursive=true",
        Some("1")
    )]
    #[tokio::test]
    async fn test_delete_folder(
        #[case] options: DeleteFolderOptions<'static>,
        #[case] expected_query: &str,
        #[case] expected_if_match: Option<&str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        client
            .delete_folder(&FolderId::new("123"), &options)
            .await?;

        assert_eq!(
            delete_requests(&server).await,
            vec![(
                expected_query.to_owned(),
                expected_if_match.map(str::to_owned)
            )]
        );

        Ok(())
    }

    /// ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_delete_folder_rejects_root() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .delete_folder(&FolderId::root(), &DeleteFolderOptions::default())
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
    /// 503は削除処理が継続していることを示すため、再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::not_empty(400)]
    #[case::not_found(404)]
    #[case::locked(409)]
    #[case::etag_mismatch(412)]
    #[case::internal_server_error(500)]
    #[case::still_deleting(503)]
    #[tokio::test]
    async fn test_delete_folder_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .delete_folder(&FolderId::new("123"), &DeleteFolderOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(delete_requests(&server).await.len(), 1);

        Ok(())
    }

    /// 429の場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[tokio::test]
    async fn test_delete_folder_retries_too_many_requests() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;

        client
            .delete_folder(&FolderId::new("123"), &DeleteFolderOptions::default())
            .await?;

        assert_eq!(delete_requests(&server).await.len(), 2);

        Ok(())
    }
}

#[cfg(test)]
mod test_validate_limit {
    use rstest::rstest;

    use super::*;

    /// 1以上1000以下の項目数の上限だけを受け付けることを確認
    #[rstest]
    #[case::zero(0, false)]
    #[case::min(1, true)]
    #[case::max(1000, true)]
    #[case::over_max(1001, false)]
    fn test_validate_limit(#[case] limit: u32, #[case] valid: bool) {
        let result = validate_limit(limit);

        if valid {
            assert!(matches!(result, Ok(l) if l == limit), "result: {result:?}");
        } else {
            assert!(
                matches!(result, Err(BoxClientError::InvalidArgument(_))),
                "result: {result:?}"
            );
        }
    }
}

#[cfg(test)]
mod test_items_page {
    use serde_json::json;

    use crate::{
        FileItem, FolderItem, WebLinkItem,
        api::types::{FileId, FolderId, WebLinkId},
    };

    use super::*;

    /// type属性に従って項目をデシリアライズし、知らない種類の項目を受け入れることを確認
    #[test]
    fn test_deserialize() -> anyhow::Result<()> {
        let page: ItemsPage = serde_json::from_value(json!({
            "entries": [
                {"type": "file", "id": "1", "name": "a.txt", "etag": "0"},
                {"type": "folder", "id": "2", "name": "docs", "etag": null},
                {"type": "web_link", "id": "3", "name": "link", "etag": "1"},
                {"type": "hubs", "id": "4", "name": "hub"},
            ],
            "limit": 100,
            "next_marker": "m1",
        }))?;

        assert_eq!(
            page,
            ItemsPage {
                entries: vec![
                    BoxItem::File(FileItem {
                        id: FileId::new("1"),
                        name: "a.txt".to_owned(),
                        etag: Some("0".to_owned()),
                    }),
                    BoxItem::Folder(FolderItem {
                        id: FolderId::new("2"),
                        name: "docs".to_owned(),
                        etag: None,
                    }),
                    BoxItem::WebLink(WebLinkItem {
                        id: WebLinkId::new("3"),
                        name: "link".to_owned(),
                        etag: Some("1".to_owned()),
                    }),
                    BoxItem::Unknown,
                ],
                limit: 100,
                next_marker: Some("m1".to_owned()),
            }
        );

        Ok(())
    }

    /// `next_marker`がnullの場合と省略された場合に、後続がないと判定することを確認
    #[test]
    fn test_deserialize_without_next_marker() -> anyhow::Result<()> {
        let null_marker: ItemsPage =
            serde_json::from_value(json!({"entries": [], "limit": 100, "next_marker": null}))?;
        let missing_marker: ItemsPage =
            serde_json::from_value(json!({"entries": [], "limit": 100}))?;

        assert_eq!(null_marker.next_marker, None);
        assert_eq!(missing_marker.next_marker, None);

        Ok(())
    }
}

#[cfg(test)]
mod test_list_items_in_folder {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// 項目のAPIのパス
    const ITEMS_PATH: &str = "/2.0/folders/123/items";

    /// 項目のAPIへのリクエストのクエリ文字列を返す。
    async fn items_queries(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.url.path() == ITEMS_PATH)
            .map(|r| r.url.query().unwrap_or_default().to_owned())
            .collect()
    }

    /// マーカーとオプションをクエリパラメーターに変換し、ページを返すことを確認
    #[rstest]
    #[case::default(None, ListItemsOptions::default(), "usemarker=true")]
    #[case::all(
        Some("m1"),
        ListItemsOptions {
            limit: Some(1000),
            sort: Some(ItemSort::Name),
            direction: Some(SortDirection::Desc),
        },
        "usemarker=true&limit=1000&marker=m1&sort=name&direction=DESC"
    )]
    #[tokio::test]
    async fn test_list_items_in_folder(
        #[case] marker: Option<&str>,
        #[case] options: ListItemsOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "file", "id": "1", "name": "a.txt", "etag": "0"}],
                "limit": 100,
                "next_marker": "m2",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let page = client
            .list_items_in_folder(&FolderId::new("123"), marker, &options)
            .await?;

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.next_marker.as_deref(), Some("m2"));
        assert_eq!(items_queries(&server).await, vec![expected_query]);

        Ok(())
    }

    /// 項目数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::zero(0)]
    #[case::over_max(1001)]
    #[tokio::test]
    async fn test_list_items_in_folder_invalid_limit(#[case] limit: u32) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListItemsOptions {
            limit: Some(limit),
            ..ListItemsOptions::default()
        };

        let result = client
            .list_items_in_folder(&FolderId::new("123"), None, &options)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(items_queries(&server).await, Vec::<String>::new());

        Ok(())
    }

    /// ルートフォルダの項目のAPIのパス
    const ROOT_ITEMS_PATH: &str = "/2.0/folders/0/items";

    /// ルートフォルダの項目のAPIへのリクエストのクエリ文字列を返す。
    async fn root_items_queries(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.url.path() == ROOT_ITEMS_PATH)
            .map(|r| r.url.query().unwrap_or_default().to_owned())
            .collect()
    }

    /// ルートフォルダで並び替えに使用する属性を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_items_in_folder_root_with_sort() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListItemsOptions {
            sort: Some(ItemSort::Name),
            ..ListItemsOptions::default()
        };

        let result = client
            .list_items_in_folder(&FolderId::root(), None, &options)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(root_items_queries(&server).await, Vec::<String>::new());

        Ok(())
    }

    /// ルートフォルダでも、並び替えに使用する属性を指定しなければ、並び替えの方向とともにリストできることを確認
    #[tokio::test]
    async fn test_list_items_in_folder_root_without_sort() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(ROOT_ITEMS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [],
                "limit": 100,
                "next_marker": null,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListItemsOptions {
            direction: Some(SortDirection::Desc),
            ..ListItemsOptions::default()
        };

        let page = client
            .list_items_in_folder(&FolderId::root(), None, &options)
            .await?;

        assert_eq!(page.entries, Vec::<BoxItem>::new());
        assert_eq!(
            root_items_queries(&server).await,
            vec!["usemarker=true&direction=DESC"]
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_items_in_folder {
    use rstest::rstest;
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// 項目のAPIのパス
    const ITEMS_PATH: &str = "/2.0/folders/123/items";

    /// 指定した名前のファイルだけを含むページのレスポンスを返す。
    fn page_response(name: &str, next_marker: &Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "entries": [{"type": "file", "id": name, "name": name, "etag": "0"}],
            "limit": 1000,
            "next_marker": next_marker,
        }))
    }

    /// 項目の名前を返す。
    fn item_name(item: &BoxItem) -> Option<&str> {
        match item {
            BoxItem::File(file) => Some(&file.name),
            BoxItem::Folder(folder) => Some(&folder.name),
            BoxItem::WebLink(link) => Some(&link.name),
            BoxItem::Unknown => None,
        }
    }

    /// `next_marker`がnullまたは空文字列になるまでページを送り、すべての項目を返すことを確認
    #[rstest]
    #[case::null(Value::Null)]
    #[case::empty(json!(""))]
    #[tokio::test]
    async fn test_list_all_items_in_folder(#[case] last_marker: Value) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        // 項目数の上限を省略した場合は、上限の1000件ずつ取得する。
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", &json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("second", &last_marker))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let items = client
            .list_all_items_in_folder(&FolderId::new("123"), &ListItemsOptions::default())
            .await?;

        let names: Vec<_> = items.iter().filter_map(item_name).collect();
        assert_eq!(names, vec!["first", "second"]);

        Ok(())
    }

    /// BOXが同じマーカーを続けて返す場合は、無限ループせずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_items_in_folder_marker_not_advanced() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .respond_with(page_response("same", &json!("m1")))
            .expect(2)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .list_all_items_in_folder(&FolderId::new("123"), &ListItemsOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_collect_all_pages {
    use rstest::rstest;

    use super::*;

    /// `next_markers`の順に`next_marker`を返すページを取得し、`fetch_page`に渡されたマーカーを記録する。
    ///
    /// 用意したページを使い切った後に呼び出された場合は、無限ループでテストが終わらなくなるのを防ぐため、エラーを返す。
    async fn collect(
        next_markers: &[Option<&str>],
    ) -> (BoxClientResult<(Vec<BoxItem>, u32)>, Vec<Option<String>>) {
        let mut received_markers = Vec::new();
        let mut remaining = next_markers.iter();
        let result = collect_all_pages(async |marker: Option<&str>| {
            received_markers.push(marker.map(str::to_owned));
            let Some(next_marker) = remaining.next() else {
                return Err(BoxClientError::InvalidArgument("no more pages"));
            };
            Ok(ItemsPage {
                entries: vec![BoxItem::Unknown],
                limit: 1,
                next_marker: next_marker.map(str::to_owned),
            })
        })
        .await;

        (result, received_markers)
    }

    /// 終端までマーカーが重複しない場合は、すべてのページの項目とページ数を返し、前のページのマーカーを渡すことを確認
    #[rstest]
    #[case::single_page(vec![None], vec![None])]
    #[case::empty_marker_terminates(vec![Some("")], vec![None])]
    #[case::three_pages(
        vec![Some("m1"), Some("m2"), None],
        vec![None, Some("m1"), Some("m2")]
    )]
    #[tokio::test]
    async fn test_collect_all_pages(
        #[case] next_markers: Vec<Option<&str>>,
        #[case] expected_markers: Vec<Option<&str>>,
    ) -> anyhow::Result<()> {
        let (result, received_markers) = collect(&next_markers).await;

        let (items, pages) = result?;
        assert_eq!(items.len(), next_markers.len());
        assert_eq!(pages as usize, next_markers.len());
        assert_eq!(
            received_markers,
            expected_markers
                .into_iter()
                .map(|m| m.map(str::to_owned))
                .collect::<Vec<_>>()
        );

        Ok(())
    }

    /// BOXが以前に返したマーカーを再び返した場合は、そのマーカーでページを取得せずにエラーを返すことを確認
    #[rstest]
    // 直前のマーカーと同じマーカーを返す
    #[case::same_as_previous(vec![Some("m1"), Some("m1")], 2)]
    // 2つ前のマーカーに戻る
    #[case::two_page_cycle(vec![Some("m1"), Some("m2"), Some("m1")], 3)]
    // 最初のマーカーに戻る
    #[case::back_to_first(vec![Some("m1"), Some("m2"), Some("m3"), Some("m1")], 4)]
    #[tokio::test]
    async fn test_collect_all_pages_detects_repeated_marker(
        #[case] next_markers: Vec<Option<&str>>,
        #[case] expected_fetches: usize,
    ) {
        let (result, received_markers) = collect(&next_markers).await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );
        assert_eq!(received_markers.len(), expected_fetches);
    }
}
