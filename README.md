# Box Client

## 再試行の仕組み

`BoxClient`は、BOXのREST APIおよびトークンエンドポイントへのリクエストが失敗すると、失敗の種類とリクエストの冪等性から再試行するかどうかを判定する。
再試行する場合は、待機した後に同じリクエストを送信し直す。

### 処理の流れ

1. リクエストを送信する。
2. 成功レスポンスを受け取った場合は、結果を返して終了する。
3. 失敗した場合は、失敗を「再試行できるエラー」と「再試行できないエラー」に分類する。
4. 再試行できないエラーの場合は、エラーを返して終了する。
5. 再試行回数が上限に達している場合は、最後のエラーを返して終了する。
6. 待機時間を決定して待機し、手順1に戻る。

### 再試行するかどうかの判定

冪等でないリクエストを、BOXが処理したかもしれない失敗の後に再送すると、リソースを二重に作成するおそれがある。
したがって、失敗の種類だけでなく、リクエストの冪等性に応じて再試行するかどうかを変える。

| 失敗の種類 | 冪等なリクエスト | 冪等でないリクエスト | 理由 |
| --- | --- | --- | --- |
| 429 Too Many Requests | 再試行する | 再試行する | BOXはリクエストを処理せずに拒否している。 |
| エラーコードが`operation_blocked_temporary`の409 Conflict | 再試行する | 再試行する | 移動、コピー、削除または復元の操作が項目をロックしているため、BOXはリクエストを処理せずに拒否している。ロックが解除されれば成功する可能性がある。 |
| 5xx | 再試行する | 再試行しない | BOXがリクエストを処理した可能性がある。 |
| 上記以外の4xx（401を除く） | 再試行しない | 再試行しない | リクエストの内容に問題があり、再送しても結果は変わらない。名前の重複による409なども含む。 |
| 接続エラー | 再試行する | 再試行する | リクエストを送信する前に失敗している。 |
| タイムアウト | 再試行する | 再試行しない | BOXがリクエストを処理した可能性がある。 |
| リクエストの送信中またはレスポンスボディの受信中の通信エラー | 再試行する | 再試行しない | BOXがリクエストを処理した可能性がある。 |
| レスポンスのJSONを解釈できない | 再試行しない | 再試行しない | 再送しても同じレスポンスを受け取る可能性が高い。 |

再試行しない場合や、再試行回数が上限に達した場合、BOXのエラーレスポンスは`BoxClientError::Api`として返る。
`BoxClientError::Api`は、ステータスとレスポンスボディを保持する。

### 401 Unauthorizedの扱い

BOXがアクセストークンを拒否して401を返した場合は、キャッシュしているアクセストークンを破棄する。
その後、アクセストークンを取得し直して、リクエストを1回だけ再送する。
401はBOXがリクエストを処理していないことを示すため、冪等でないリクエストも再送する。

この再送は、再試行回数に数えない。
再送しても401を受け取った場合は、再試行せずに`BoxClientError::Api`を返す。

### 冪等性の決定

リクエストの冪等性は、HTTPメソッドから次のように決定する。

- 冪等: GET、HEAD、PUT、DELETE、OPTIONSおよびTRACE
- 冪等でない: POST、PATCHなど、上記以外のメソッド

ただし、HTTPメソッドの意味とAPIの実際の振る舞いが一致しない場合がある。
したがって、`ApiRequest::idempotency()`で冪等性を上書きできる。
このクレートでは、次のリクエストで冪等性を上書きしている。

| リクエスト | HTTPメソッド | 冪等性 | 理由 |
| --- | --- | --- | --- |
| アクセストークンの取得 | POST | 冪等 | アクセストークンを二重に発行しても害はない。また、JWTアサーションは試行ごとに作り直す。 |
| フォルダの削除（`BoxClient::delete_folder`） | DELETE | 冪等でない | BOXは削除に600秒以上かかると503を返すが、削除処理はその後も継続する。再送すると409や404を受け取り、削除の結果を誤って判断するおそれがある。 |
| ファイルの削除（`BoxClient::delete_file`） | DELETE | 冪等でない | BOXは削除済みのファイルに対して404を返す。BOXが削除した後に再送すると404を受け取り、削除の結果を誤って判断するおそれがある。 |

フォルダの削除で503を受け取った場合は、`BoxClient::folder_information`でフォルダの状態を確認する。
ファイルの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::file_information`でファイルの状態を確認する。

### 待機時間の決定

再試行する前の待機時間は、次のように決定する。

- レスポンスに`Retry-After`ヘッダーがある場合
  - `Retry-After`ヘッダーが示す秒数にジッターを加算した時間だけ待機する。
    BOXが指定した時刻より早く再試行しないように、ジッターは加算だけを行う。
  - `Retry-After`ヘッダーが示す秒数が待機時間の上限より大きい場合は、再試行を中止して`BoxClientError::RetryAfterTooLong`を返す。
  - `Retry-After`ヘッダーを秒数として解釈できない場合は、ヘッダーがない場合と同じく指数バックオフで待機する。
- レスポンスに`Retry-After`ヘッダーがない場合
  - 指数バックオフで算出した時間にジッターを加算した時間だけ待機する。
  - n回目の再試行前の待機時間は、`initial_delay * 2^(n - 1)`を`max_delay`で切り詰めた時間とする。
  - 多数のクライアントが同時に再試行しないように、`0`以上`max_jitter`以下の乱数の時間を加算する。
    ジッターを加算した後は`max_delay`で切り詰めないため、待機時間の最大値は`max_delay + max_jitter`となる。

### 再試行設定

再試行設定は`RetryPolicy`で指定する。
`RetryPolicy`は、REST APIとトークンエンドポイントの両方のリクエストに適用する。

| 設定項目 | 意味 | 既定値 |
| --- | --- | --- |
| `max_retries` | 最大再試行回数。最初のリクエストは回数に含めない。`0`の場合は再試行しない。 | 5回 |
| `initial_delay` | 1回目の再試行前に待機する時間 | 0.5秒 |
| `max_delay` | 指数バックオフで算出する待機時間の上限、および`Retry-After`ヘッダーが示す待機時間の許容値 | 30秒 |
| `max_jitter` | 待機時間に加算するジッターの上限 | 1秒 |

既定値を使用した場合、ジッターを加算する前の待機時間は、0.5秒、1秒、2秒、4秒、8秒と倍増する。

`RetryPolicy::new()`は、次の条件を満たさない設定値に対して`BoxClientError::InvalidRetryPolicy`を返す。

- `initial_delay`が`0`より大きい。
  `0`の場合は、待機せずに再試行し続けるためである。
- `initial_delay`が`max_delay`以下である。
- `max_jitter`が`max_delay`以下である。

再試行設定とタイムアウトは、`BoxClientBuilder`で指定する。

```rust
use std::time::Duration;

use box_client::{BoxClient, BoxClientTimeout, RetryPolicy};

let client = BoxClient::builder(config)
    .retry(RetryPolicy::new(
        3,
        Duration::from_secs(1),
        Duration::from_secs(10),
        Duration::from_millis(500),
    )?)
    .timeout(BoxClientTimeout {
        request_timeout: Duration::from_secs(30),
        connect_timeout: Duration::from_secs(10),
    })
    .build()
    .await?;
```

### ログ

再試行に関するイベントは、`tracing`で構造化ログとして出力する。
401を受け取ったときのイベントを除き、各イベントは処理名`operation`および試行回数`attempts`を含む。
`operation`は、REST APIへのリクエストでは`box_api_request`、アクセストークンの取得では`fetch_access_token`である。
`retrying`以外のイベントは、最初のリクエストからの経過時間`elapsed_ms`も含む。

| レベル | メッセージ | 出力する場面 |
| --- | --- | --- |
| INFO | `succeeded` | リクエストが成功した。 |
| WARN | `retrying` | 再試行する。待機時間`delay_ms`と、待機時間を決めた方法`delay_source`を含む。`delay_source`は`retry_after`または`backoff`である。 |
| WARN | `access token was rejected; fetching a new one and resending` | 401を受け取り、アクセストークンを取得し直して再送する。 |
| ERROR | `failed with non-retryable error` | 再試行できないエラーが発生した。 |
| ERROR | `failed after reaching max retries` | 再試行回数が上限に達した。 |
| ERROR | `gave up retrying` | `Retry-After`ヘッダーが示す待機時間が上限を超えたため、再試行を中止した。 |

### キャンセル

リクエストを送信するfutureは、待機中または送信中に破棄しても状態を残さない。
したがって、`tokio::time::timeout`などで処理全体の時間を制限できる。
ただし、送信中に破棄した場合は、BOXがリクエストを処理したかどうかを判断できない。

## 開発環境

## prek

Gitフックを導入する。

```shell
cargo install cargo-deny
cargo install prek
prek install
```
