//! フォルダに関するAPI

use serde::Deserialize;

use crate::{BoxClient, BoxClientError, BoxClientResult, BoxItem, FolderId, request::ApiRequest};

/// 1回のリクエストで取得できる項目数の上限
pub const MAX_ITEMS_LIMIT: u32 = 1000;

/// フォルダ内の項目の1ページ
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
    fn as_str(self) -> &'static str {
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
    /// ルートフォルダでは、マーカー方式のページ送りと並び替えを併用できない。
    pub sort: Option<ItemSort>,

    /// 並び替えの方向
    pub direction: Option<SortDirection>,
}

/// 1ページで取得する項目数の上限が、BOXが受け付ける範囲内であることを検証する。
fn validate_limit(limit: u32) -> BoxClientResult<u32> {
    if (1..=MAX_ITEMS_LIMIT).contains(&limit) {
        Ok(limit)
    } else {
        Err(BoxClientError::InvalidArgument(
            "limit must be between 1 and 1000",
        ))
    }
}

impl BoxClient {
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
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_items_in_folder(
        &self,
        folder_id: &FolderId,
        marker: Option<&str>,
        options: &ListItemsOptions,
    ) -> BoxClientResult<ItemsPage> {
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
    /// BOXが同じマーカーを返し続ける場合は、無限ループを避けるため[`BoxClientError::UnexpectedResponse`]を返す。
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
        let mut items = Vec::new();
        let mut marker: Option<String> = None;
        let mut pages: u32 = 0;

        loop {
            let page = self
                .list_items_in_folder(folder_id, marker.as_deref(), &options)
                .await?;
            pages += 1;
            items.extend(page.entries);

            // BOXは最終ページで`next_marker`をnullにするが、空文字列を返す場合もあるため、どちらも終端として扱う。
            match page.next_marker {
                Some(next) if !next.is_empty() => {
                    if marker.as_deref() == Some(next.as_str()) {
                        return Err(BoxClientError::UnexpectedResponse(
                            "next_marker did not advance",
                        ));
                    }
                    marker = Some(next);
                }
                _ => break,
            }
        }

        tracing::info!(
            folder_id = folder_id.as_ref(),
            pages,
            items = items.len(),
            "listed all items in folder"
        );

        Ok(items)
    }
}

/*
#[cfg(test)]
mod test_folder_id {
    use super::*;

    /// ルートフォルダのIDが0であることを確認
    #[test]
    fn test_root() {
        assert_eq!(FolderId::root().as_ref(), "0");
    }
}
*/

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
        api::models::{FileId, FolderId, WebLinkId},
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

    /// next_markerがnullの場合と省略された場合に、後続がないと判定することを確認
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
        assert!(items_queries(&server).await.is_empty());

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
    fn page_response(name: &str, next_marker: Value) -> ResponseTemplate {
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

    /// next_markerがnullまたは空文字列になるまでページを送り、すべての項目を返すことを確認
    #[rstest]
    #[case::null(Value::Null)]
    #[case::empty(json!(""))]
    #[tokio::test]
    async fn test_list_all_items_in_folder(#[case] last_marker: Value) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        // 項目数の上限を省略した場合は、上限の1000件ずつ取得する
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("second", last_marker))
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

    /// BOXが同じマーカーを返し続ける場合は、無限ループせずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_items_in_folder_marker_not_advanced() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .respond_with(page_response("same", json!("m1")))
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
