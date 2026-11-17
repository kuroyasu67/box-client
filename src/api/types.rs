use serde::Deserialize;

macro_rules! box_item_struct {
    ($name: ident ) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

box_item_struct!(FolderId);
box_item_struct!(FileId);
box_item_struct!(WebLinkId);

impl FolderId {
    pub fn root() -> Self {
        FolderId::new("0")
    }

    /// ルートフォルダのIDであるかを返す。
    pub fn is_root(&self) -> bool {
        self.0 == "0"
    }
}

/// フォルダ内の項目
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BoxItem {
    /// ファイル
    File(FileItem),

    /// フォルダ
    Folder(FolderItem),

    /// ウェブリンク
    WebLink(WebLinkItem),

    /// このクライアントが知らない種類の項目
    ///
    /// BOXが項目の種類を追加しても、ページ全体の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// フォルダ内のファイル
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileItem {
    /// ファイルID
    pub id: FileId,

    /// ファイル名
    pub name: String,

    /// ファイルのバージョンを識別するETag
    pub etag: Option<String>,
}

/// フォルダ内のフォルダ
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FolderItem {
    /// フォルダID
    pub id: FolderId,

    /// フォルダ名
    pub name: String,

    /// フォルダのバージョンを識別するETag
    pub etag: Option<String>,
}

/// フォルダ内のウェブリンク
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WebLinkItem {
    /// ウェブリンクID
    pub id: WebLinkId,

    /// ウェブリンク名
    pub name: String,

    /// ウェブリンクのバージョンを識別するETag
    pub etag: Option<String>,
}

/// ルートフォルダから項目の親フォルダまでのパス
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PathCollection {
    /// パスに含まれるフォルダの数
    pub total_count: u64,

    /// ルートフォルダから順に並べた、パスに含まれるフォルダ
    pub entries: Vec<FolderItem>,
}

/// 項目の状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    /// 有効
    Active,

    /// ごみ箱に移動済み
    Trashed,

    /// 削除済み
    Deleted,

    /// このクライアントが知らない状態
    ///
    /// BOXが状態を追加しても、項目の情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod test_folder_id_is_root {
    use rstest::rstest;

    use super::*;

    /// IDが0の場合だけルートフォルダと判定することを確認
    #[rstest]
    #[case::root("0", true)]
    #[case::other("1", false)]
    #[case::leading_zero("00", false)]
    #[case::empty("", false)]
    fn test_is_root(#[case] id: &str, #[case] expected: bool) {
        assert_eq!(FolderId::new(id).is_root(), expected);
    }
}

#[cfg(test)]
mod test_item_status {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    /// 項目の状態をデシリアライズし、知らない状態を受け入れることを確認
    #[rstest]
    #[case::active("active", ItemStatus::Active)]
    #[case::trashed("trashed", ItemStatus::Trashed)]
    #[case::deleted("deleted", ItemStatus::Deleted)]
    #[case::unknown("archived", ItemStatus::Unknown)]
    fn test_deserialize(#[case] value: &str, #[case] expected: ItemStatus) -> anyhow::Result<()> {
        let status: ItemStatus = serde_json::from_value(json!(value))?;

        assert_eq!(status, expected);

        Ok(())
    }
}
