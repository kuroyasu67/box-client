//! BOXのREST APIを呼び出すメソッド
//!
//! リソースごとにサブモジュールを分け、各サブモジュールで`impl BoxClient`を定義する。

pub mod files;
pub mod folders;
pub mod trash;
pub mod types;
pub mod uploads;
pub mod users;
pub mod web_links;
