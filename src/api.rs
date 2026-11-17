//! BoxのREST APIを呼び出すメソッド
//!
//! リソースごとにサブモジュールを分け、各サブモジュールで`impl BoxClient`を定義する。

pub mod folders;
pub mod types;
pub mod users;
