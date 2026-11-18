//! BOXのREST APIを呼び出すメソッド
//!
//! リソースごとにサブモジュールを分け、各サブモジュールで`impl BoxClient`を定義する。

pub mod downloads;
pub mod files;
pub mod folders;
pub mod pagination;
pub mod trash;
pub mod types;
pub mod updates;
pub mod uploads;
pub mod users;
pub mod versions;
pub mod web_links;
pub mod zip_downloads;
