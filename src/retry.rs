use std::{
    future::Future,
    time::{Duration, Instant},
};

use rand::{Rng, RngExt};
use reqwest::{
    Method, Response, StatusCode,
    header::{HeaderMap, RETRY_AFTER},
};
use serde::de::DeserializeOwned;

use crate::errors::{ApiError, BoxClientError, BoxClientResult};

/// 再試行設定
///
/// 再試行するまでの待機時間を指数バックオフで決定する。
/// ただし、レスポンスに`Retry-After`ヘッダーがある場合は、指数バックオフを使用せず、
/// `Retry-After`ヘッダーが示す時間だけ待機する。
///
/// 不正な設定値を持つインスタンスを作成させないため、フィールドを非公開にし、[`RetryPolicy::new`]で検証する。
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// 最大再試行回数
    ///
    /// 最初のリクエストは回数に含めない。`0`の場合は再試行しない。
    max_retries: u32,

    /// 1回目の再試行前に待機する時間
    ///
    /// n回目の再試行前の待機時間は`initial_delay * 2^(n - 1)`とする。
    initial_delay: Duration,

    /// 待機時間の上限
    ///
    /// `Retry-After`ヘッダーが示す時間がこの値よりも大きい場合は、再試行を中止する。
    /// また、指数バックオフで算出した待機時間がこの値よりも大きい場合は、待機時間をこの値に変更する。
    max_delay: Duration,

    /// 待機時間に加算するジッターの上限
    ///
    /// 待機時間に`0`以上この値以下の乱数の時間を加算する。
    /// ジッターを加算した後は`max_delay`で切り詰めないため、待機時間の最大値は`max_delay + max_jitter`となる。
    max_jitter: Duration,
}

impl Default for RetryPolicy {
    /// 最大5回、0.5秒から30秒まで倍増させながら、最大1秒のジッターを加えて再試行する設定を返す。
    fn default() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            max_jitter: Duration::from_secs(1),
        }
    }
}

impl RetryPolicy {
    /// コンストラクタ
    ///
    /// # Arguments
    ///
    /// - `max_retries`: 最大再試行回数
    /// - `initial_delay`: 1回目の再試行前に待機する時間。`0`より大きく`max_delay`以下とする。
    /// - `max_delay`: 待機時間の上限
    /// - `max_jitter`: 待機時間に加算するジッターの上限。`max_delay`以下とする。
    ///
    /// # Errors
    ///
    /// 設定値が上記の条件を満たさない場合は、[`BoxClientError::InvalidRetryPolicy`]を返す。
    pub fn new(
        max_retries: u32,
        initial_delay: Duration,
        max_delay: Duration,
        max_jitter: Duration,
    ) -> BoxClientResult<Self> {
        // initial_delayが0の場合は、指数バックオフの待機時間が常に0となり、待機せずに再試行し続ける。
        if initial_delay.is_zero() {
            return Err(BoxClientError::InvalidRetryPolicy(
                "initial_delay must be greater than zero",
            ));
        }
        if max_delay < initial_delay {
            return Err(BoxClientError::InvalidRetryPolicy(
                "initial_delay must be less than or equal to max_delay",
            ));
        }
        if max_delay < max_jitter {
            return Err(BoxClientError::InvalidRetryPolicy(
                "max_jitter must be less than or equal to max_delay",
            ));
        }

        Ok(Self {
            max_retries,
            initial_delay,
            max_delay,
            max_jitter,
        })
    }

    /// 最大再試行回数を返す。
    #[must_use]
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// 1回目の再試行前に待機する時間を返す。
    #[must_use]
    pub fn initial_delay(&self) -> Duration {
        self.initial_delay
    }

    /// 待機時間の上限を返す。
    #[must_use]
    pub fn max_delay(&self) -> Duration {
        self.max_delay
    }

    /// 待機時間に加算するジッターの上限を返す。
    #[must_use]
    pub fn max_jitter(&self) -> Duration {
        self.max_jitter
    }

    /// 指数バックオフで算出し、上限で切り詰めた待機時間を返す。
    ///
    /// # Arguments
    ///
    /// - `number_of_retries`: 今回の再試行を含む再試行回数。`0`は最初のリクエストを表す。
    pub(crate) fn delay_duration(&self, number_of_retries: u32) -> Duration {
        // 再試行でない場合は待機しない。
        if number_of_retries < 1 {
            return Duration::ZERO;
        }
        // 倍率または待機時間がオーバーフローする場合は、必ず上限を超えているため上限を返す。
        2u32.checked_pow(number_of_retries - 1)
            .and_then(|factor| self.initial_delay.checked_mul(factor))
            .map_or(self.max_delay, |duration| duration.min(self.max_delay))
    }

    /// ジッターを加算した待機時間を返す。
    ///
    /// 指数バックオフで算出して上限で切り詰めた待機時間に、`0`以上`max_jitter`以下の乱数の時間を加算する。
    ///
    /// # Arguments
    ///
    /// - `number_of_retries`: 今回の再試行を含む再試行回数。`0`は最初のリクエストを表す。
    /// - `rng`: 乱数生成器。通常は`rand::rng()`を渡し、テストではシードを固定した乱数生成器を渡す。
    ///
    /// 引数の型が`rand`クレートのトレイトに依存するため、`rand`のメジャーバージョンを上げると、このメソッドのシグネチャが変わる。
    /// したがって、このクレートの破壊的変更とならないように、このメソッドをクレートの外に公開しない。
    pub(crate) fn jittered_delay_duration<R: Rng + ?Sized>(
        &self,
        number_of_retries: u32,
        rng: &mut R,
    ) -> Duration {
        if number_of_retries < 1 {
            return Duration::ZERO;
        }
        // 上限に達した後も再試行する時刻を分散させるため、ジッターを加算した後は上限で切り詰めない。
        self.delay_duration(number_of_retries)
            .saturating_add(self.jitter(rng))
    }

    /// 再試行する前に待機する時間を決定する。
    ///
    /// `Retry-After`ヘッダーが示す待機時間がある場合は、その時間にジッターを加算する。
    /// サーバーが指定した時刻より早く再試行しないように、ジッターは加算のみとする。
    /// `Retry-After`ヘッダーが示す待機時間がない場合は、指数バックオフにジッターを加算した時間とする。
    ///
    /// # Arguments
    ///
    /// - `number_of_retries`: 今回の再試行を含む再試行回数。`0`は最初のリクエストを表す。
    /// - `retry_after`: `Retry-After`ヘッダーが示す待機時間
    /// - `rng`: 乱数生成器
    ///
    /// # Errors
    ///
    /// `Retry-After`ヘッダーが示す待機時間が上限を超える場合は、その待機時間を返す。
    /// 呼び出し元は、再試行の原因となったエラーを含めて[`BoxClientError::RetryAfterTooLong`]を作成する。
    pub(crate) fn retry_delay<R: Rng + ?Sized>(
        &self,
        number_of_retries: u32,
        retry_after: Option<Duration>,
        rng: &mut R,
    ) -> Result<Duration, Duration> {
        match retry_after {
            Some(retry_after) if self.max_delay < retry_after => Err(retry_after),
            Some(retry_after) => Ok(retry_after.saturating_add(self.jitter(rng))),
            None => Ok(self.jittered_delay_duration(number_of_retries, rng)),
        }
    }

    /// `0`以上`max_jitter`以下の乱数の時間を返す。
    ///
    /// # Arguments
    ///
    /// - `rng`: 乱数生成器
    #[expect(
        clippy::cast_possible_truncation,
        reason = "`nanos`は`max_jitter`のナノ秒以下であるため、秒はu64に、秒未満のナノ秒はu32に収まる"
    )]
    fn jitter<R: Rng + ?Sized>(&self, rng: &mut R) -> Duration {
        const NANOS_PER_SEC: u128 = 1_000_000_000;

        // `max_jitter`のナノ秒はu64に収まらない場合があるため、u128で乱数を生成する。
        let nanos = rng.random_range(0..=self.max_jitter.as_nanos());
        // `nanos`は`max_jitter`のナノ秒以下であるため、秒はu64に、秒未満のナノ秒はu32に収まる。
        Duration::new(
            (nanos / NANOS_PER_SEC) as u64,
            (nanos % NANOS_PER_SEC) as u32,
        )
    }
}

/// リクエストの冪等性
///
/// 冪等でないリクエストは、サーバーが処理した可能性がある失敗の後に再送すると、
/// リソースを二重に作成するおそれがある。そのため、冪等性に応じて再試行する条件を変える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idempotency {
    /// 同じリクエストを何度送信しても結果が変わらない。
    ///
    /// 429、エラーコードが`operation_blocked_temporary`の409、5xx、タイムアウトおよび通信エラーの後に再試行する。
    Idempotent,

    /// 同じリクエストを送信するたびに結果が変わりうる。
    ///
    /// サーバーがリクエストを処理していないことが確実な、429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    NonIdempotent,
}

impl Idempotency {
    /// HTTPメソッドの意味から冪等性を決定する。
    ///
    /// GET、HEAD、PUT、DELETE、OPTIONSおよびTRACEは冪等、それ以外は冪等でないとする。
    #[must_use]
    pub fn from_method(method: &Method) -> Self {
        if method.is_idempotent() {
            Self::Idempotent
        } else {
            Self::NonIdempotent
        }
    }
}

/// 1回の試行で発生したエラー
pub(crate) enum AttemptError {
    /// 再試行すれば成功する可能性があるエラー
    Retryable {
        /// エラー
        error: BoxClientError,

        /// `Retry-After`ヘッダーが示す待機時間
        retry_after: Option<Duration>,
    },

    /// 再試行しても結果が変わらないエラー
    Fatal(BoxClientError),
}

/// 再試行設定に従って、`attempt`が成功するか、再試行できないエラーを返すまで繰り返し呼び出す。
///
/// 処理の途中でfutureを破棄しても、待機中または試行中の処理が中断されるだけで、状態は残らない。
///
/// # Arguments
///
/// - `policy`: 再試行設定
/// - `operation`: ログに出力する処理の名前
/// - `attempt`: 1回分の処理を実行するfutureを返すクロージャー
///
/// # Errors
///
/// 再試行できないエラーが発生した場合、再試行回数の上限に達した場合、
/// または`Retry-After`ヘッダーが示す待機時間が上限を超えた場合はエラーを返す。
pub(crate) async fn run_with_retry<T, F, Fut>(
    policy: &RetryPolicy,
    operation: &'static str,
    mut attempt: F,
) -> BoxClientResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AttemptError>>,
{
    let started_at = Instant::now();
    let mut number_of_retries = 0;

    // `BoxClientError`のメッセージは原因を含まないため、エラーを`%`で`Display`として記録すると原因がログから失われる。
    // したがって、エラーを`dyn Error`として記録し、tracing-subscriberに原因の連鎖も出力させる。
    loop {
        let attempts = number_of_retries + 1;
        let (error, retry_after) = match attempt().await {
            Ok(value) => {
                tracing::debug!(
                    operation,
                    attempts,
                    elapsed_ms = started_at.elapsed().as_millis(),
                    "succeeded"
                );
                return Ok(value);
            }
            Err(AttemptError::Fatal(error)) => {
                // 再試行できないエラーは、そのまま呼び出し元に返す。404のように呼び出し元が想定しているエラーもあり、
                // 異常とみなすかどうかは呼び出し元が判断するため、DEBUGで出力する。
                tracing::debug!(
                    operation,
                    attempts,
                    elapsed_ms = started_at.elapsed().as_millis(),
                    error = &error as &(dyn std::error::Error + 'static),
                    "failed with non-retryable error"
                );
                return Err(error);
            }
            Err(AttemptError::Retryable { error, retry_after }) => (error, retry_after),
        };

        // 再試行を尽くしたエラーも呼び出し元に返すため、エラーの重さは呼び出し元が判断する。
        // ただし、呼び出し元が受け取るエラーにない試行回数と経過時間を残すため、WARNで出力する。
        if policy.max_retries() <= number_of_retries {
            tracing::warn!(
                operation,
                attempts,
                elapsed_ms = started_at.elapsed().as_millis(),
                error = &error as &(dyn std::error::Error + 'static),
                "failed after reaching max retries"
            );
            return Err(error);
        }
        number_of_retries += 1;

        // `ThreadRng`は`Send`を実装していないため、`await`をまたいで保持するとfutureが`Send`でなくなる。
        // したがって、待機時間の計算と待機を別の文に分け、`ThreadRng`を待機の前に破棄する。
        let delay = match policy.retry_delay(number_of_retries, retry_after, &mut rand::rng()) {
            Ok(delay) => delay,
            Err(retry_after) => {
                tracing::warn!(
                    operation,
                    attempts,
                    elapsed_ms = started_at.elapsed().as_millis(),
                    retry_after_ms = retry_after.as_millis(),
                    max_delay_ms = policy.max_delay().as_millis(),
                    error = &error as &(dyn std::error::Error + 'static),
                    "gave up retrying"
                );
                return Err(BoxClientError::RetryAfterTooLong {
                    retry_after,
                    max_delay: policy.max_delay(),
                    source: Box::new(error),
                });
            }
        };
        tracing::warn!(
            operation,
            attempts,
            delay_ms = delay.as_millis(),
            delay_source = if retry_after.is_some() {
                "retry_after"
            } else {
                "backoff"
            },
            error = &error as &(dyn std::error::Error + 'static),
            "retrying"
        );
        tokio::time::sleep(delay).await;
    }
}

/// 移動、コピー、削除または復元の操作が項目をロックしているため、BOXが操作を拒否したことを示すエラーコード
const OPERATION_BLOCKED_TEMPORARY: &str = "operation_blocked_temporary";

/// 再試行すれば成功する可能性があるエラーレスポンスかどうかを返す。
///
/// 429はサーバーがリクエストを処理せずに拒否したため、冪等性によらず再試行する。
/// エラーコードが`operation_blocked_temporary`の409も、他の操作によるロックが解除されるまで
/// BOXがリクエストを処理せずに拒否したため、冪等性によらず再試行する。
/// 5xxはサーバーが処理した可能性があるため、冪等なリクエストのみ再試行する。
/// そのほかの4xxは、リクエストの内容に問題があり、再送しても結果が変わらないため再試行しない。
/// 名前の重複などによる409も、再送しても結果が変わらないため再試行しない。
pub(crate) fn is_retryable_error_response(
    status: StatusCode,
    code: Option<&str>,
    idempotency: Idempotency,
) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::CONFLICT && code == Some(OPERATION_BLOCKED_TEMPORARY))
        || (idempotency == Idempotency::Idempotent && status.is_server_error())
}

/// HTTP通信エラーを、再試行できるエラーと再試行できないエラーに分類する。
///
/// 接続エラーはリクエストを送信する前の失敗であるため、冪等性によらず再試行する。
/// タイムアウト、送信中または受信中の通信エラーは、サーバーが処理した可能性があるため、
/// 冪等なリクエストのみ再試行する。
///
/// 成功レスポンスのボディの受信中に発生したエラーは、この関数ではなく[`read_json`]で分類する。
pub(crate) fn classify_transport_error(
    error: reqwest::Error,
    idempotency: Idempotency,
) -> AttemptError {
    let retryable = error.is_connect()
        || (idempotency == Idempotency::Idempotent
            && (error.is_timeout() || error.is_request() || error.is_body()));
    if retryable {
        AttemptError::Retryable {
            error: error.into(),
            retry_after: None,
        }
    } else {
        AttemptError::Fatal(error.into())
    }
}

/// 成功レスポンスのボディを受信し、JSONとしてデシリアライズする。
///
/// reqwestは、ボディの受信中に発生した通信エラーを`is_body()`ではなく`is_decode()`が`true`を返すエラーとして返す。
/// そのため、この関数はボディの受信とデシリアライズを分け、受信の失敗とJSONの解釈の失敗を区別する。
///
/// - ボディの受信中の通信エラーは、サーバーが処理した可能性があるため、冪等なリクエストのみ再試行する。
/// - JSONを解釈できない場合は、再送しても同じレスポンスを受け取る可能性が高いため、再試行せずに
///   [`BoxClientError::DeserializeResponse`]を返す。
pub(crate) async fn read_json<T: DeserializeOwned>(
    response: Response,
    idempotency: Idempotency,
) -> Result<T, AttemptError> {
    let body = response
        .bytes()
        .await
        .map_err(|error| classify_body_error(error, idempotency))?;

    serde_json::from_slice(&body)
        .map_err(|e| AttemptError::Fatal(BoxClientError::DeserializeResponse(e)))
}

/// 成功レスポンスのボディの受信中に発生した通信エラーを、再試行できるエラーと再試行できないエラーに分類する。
///
/// ボディの受信中の通信エラーは、サーバーが処理した可能性があるため、冪等なリクエストのみ再試行する。
pub(crate) fn classify_body_error(error: reqwest::Error, idempotency: Idempotency) -> AttemptError {
    if idempotency == Idempotency::Idempotent {
        AttemptError::Retryable {
            error: error.into(),
            retry_after: None,
        }
    } else {
        AttemptError::Fatal(error.into())
    }
}

/// 成功以外のレスポンスを、再試行できるエラーと再試行できないエラーに分類する。
pub(crate) async fn classify_error_response(
    response: Response,
    idempotency: Idempotency,
) -> AttemptError {
    let (api_error, retry_after) = read_error_response(response).await;
    classify_api_error(api_error, retry_after, idempotency)
}

/// 成功以外のレスポンスのボディを読み込み、BOXのエラーレスポンスと`Retry-After`ヘッダーが示す待機時間を返す。
pub(crate) async fn read_error_response(response: Response) -> (ApiError, Option<Duration>) {
    let status = response.status();
    let retry_after = parse_retry_after(response.headers());
    // ボディは原因を調べるための補助情報であるため、読み込めない場合は読み込めなかった理由をボディの代わりに記録する。
    let body = response
        .text()
        .await
        .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

    (ApiError::new(status, body), retry_after)
}

/// BOXのエラーレスポンスを、再試行できるエラーと再試行できないエラーに分類する。
pub(crate) fn classify_api_error(
    api_error: ApiError,
    retry_after: Option<Duration>,
    idempotency: Idempotency,
) -> AttemptError {
    let retryable =
        is_retryable_error_response(api_error.status, api_error.code.as_deref(), idempotency);
    let error = BoxClientError::Api(Box::new(api_error));
    if retryable {
        AttemptError::Retryable { error, retry_after }
    } else {
        AttemptError::Fatal(error)
    }
}

/// BOXが処理を終えるまで再送するリクエストの、1回の送信の結果
///
/// BOXは、アップロードセッションのコミットやファイルのダウンロードで、処理を終えていない場合に
/// 202と`Retry-After`ヘッダーを返す。
#[derive(Debug)]
pub(crate) enum PollOutcome<T> {
    /// BOXが処理を終えた。
    Ready(T),

    /// BOXが処理を終えていないため、リクエストを再送する必要がある。
    NotReady {
        /// `Retry-After`ヘッダーが示す待機時間
        retry_after: Option<Duration>,
    },
}

/// 成功レスポンスが202の場合は[`PollOutcome::NotReady`]を返し、それ以外の場合は`read`で読み込んだ結果を返す。
pub(crate) async fn read_poll_response<T, F, Fut>(
    response: Response,
    idempotency: Idempotency,
    read: F,
) -> Result<PollOutcome<T>, AttemptError>
where
    F: FnOnce(Response, Idempotency) -> Fut,
    Fut: Future<Output = Result<T, AttemptError>>,
{
    if response.status() == StatusCode::ACCEPTED {
        return Ok(PollOutcome::NotReady {
            retry_after: parse_retry_after(response.headers()),
        });
    }
    read(response, idempotency).await.map(PollOutcome::Ready)
}

/// `Retry-After`ヘッダーが示す待機時間を返す。
///
/// BOXは`Retry-After`ヘッダーに秒数を設定するため、秒数として解釈できない場合は`None`を返し、
/// 指数バックオフで待機させる。
pub(crate) fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod test_retry_policy {
    use std::collections::HashSet;

    use rand::{SeedableRng, rngs::StdRng};
    use rstest::rstest;

    use super::*;

    /// テストの結果を再現できるように、乱数生成器のシードを固定する。
    const SEED: u64 = 42;

    /// 乱数を使うため、結果が範囲内に収まることを複数回の試行で確かめる。
    const TRIALS: usize = 1_000;

    /// 設定値が条件を満たす場合は、再試行設定を作成できることを確認
    #[rstest]
    // `initial_delay`の下限
    #[case::min_initial_delay(Duration::from_nanos(1), Duration::from_secs(1))]
    // `initial_delay`が`max_delay`と一致
    #[case::initial_delay_equals_max_delay(Duration::from_secs(3), Duration::from_secs(1))]
    // ジッターを加算しない
    #[case::zero_jitter(Duration::from_millis(500), Duration::ZERO)]
    // `max_jitter`が`max_delay`と一致
    #[case::max_jitter_equals_max_delay(Duration::from_millis(500), Duration::from_secs(3))]
    fn test_new_valid(
        #[case] initial_delay: Duration,
        #[case] max_jitter: Duration,
    ) -> anyhow::Result<()> {
        let max_delay = Duration::from_secs(3);

        let policy = RetryPolicy::new(5, initial_delay, max_delay, max_jitter)?;

        assert_eq!(policy.max_retries(), 5);
        assert_eq!(policy.initial_delay(), initial_delay);
        assert_eq!(policy.max_delay(), max_delay);
        assert_eq!(policy.max_jitter(), max_jitter);

        Ok(())
    }

    /// 設定値が条件を満たさない場合は、再試行設定の不正エラーを返すことを確認
    #[rstest]
    // `initial_delay`が0
    #[case::zero_initial_delay(Duration::ZERO, Duration::from_secs(1))]
    // `initial_delay`が`max_delay`よりも1ナノ秒大きい
    #[case::initial_delay_exceeds_max_delay(
        Duration::from_secs(3) + Duration::from_nanos(1),
        Duration::from_secs(1)
    )]
    // `max_jitter`が`max_delay`よりも1ナノ秒大きい
    #[case::max_jitter_exceeds_max_delay(
        Duration::from_millis(500),
        Duration::from_secs(3) + Duration::from_nanos(1)
    )]
    fn test_new_invalid(#[case] initial_delay: Duration, #[case] max_jitter: Duration) {
        let result = RetryPolicy::new(5, initial_delay, Duration::from_secs(3), max_jitter);

        assert!(
            matches!(result, Err(BoxClientError::InvalidRetryPolicy(_))),
            "result: {result:?}"
        );
    }

    /// 既定の再試行設定が、コンストラクタの検証を通る値であることを確認
    #[test]
    fn test_default_is_valid() -> anyhow::Result<()> {
        let default = RetryPolicy::default();

        RetryPolicy::new(
            default.max_retries(),
            default.initial_delay(),
            default.max_delay(),
            default.max_jitter(),
        )?;

        Ok(())
    }

    /// 再試行回数に応じて、指数バックオフで算出した待機時間を上限で切り詰めて返すことを確認
    #[rstest]
    // 再試行でない場合は待機しない
    #[case::not_retry(Duration::from_millis(500), 0, Duration::ZERO)]
    // 1秒未満の待機時間を切り捨てない
    #[case::first_retry(Duration::from_millis(500), 1, Duration::from_millis(500))]
    // 上限の手前
    #[case::below_max_delay(Duration::from_millis(500), 3, Duration::from_secs(2))]
    // 算出した3秒が上限と一致する
    #[case::equal_max_delay(Duration::from_millis(750), 3, Duration::from_secs(3))]
    // 算出した4秒を上限の3秒に切り詰める
    #[case::above_max_delay(Duration::from_millis(500), 4, Duration::from_secs(3))]
    // 倍率2^31はオーバーフローしない
    #[case::max_factor(Duration::from_millis(500), 32, Duration::from_secs(3))]
    // 倍率2^32はu32でオーバーフローする
    #[case::factor_overflow(Duration::from_millis(500), 33, Duration::from_secs(3))]
    // 再試行回数がu32の最大値でもパニックしない
    #[case::max_number_of_retries(Duration::from_millis(500), u32::MAX, Duration::from_secs(3))]
    fn test_delay_duration(
        #[case] initial_delay: Duration,
        #[case] number_of_retries: u32,
        #[case] expected: Duration,
    ) -> anyhow::Result<()> {
        let policy = RetryPolicy::new(5, initial_delay, Duration::from_secs(3), Duration::ZERO)?;

        assert_eq!(policy.delay_duration(number_of_retries), expected);

        Ok(())
    }

    /// 倍率はオーバーフローしないが、待機時間がDurationでオーバーフローする場合は、上限を返すことを確認
    #[test]
    fn test_delay_duration_overflow() -> anyhow::Result<()> {
        let policy = RetryPolicy::new(5, Duration::MAX, Duration::MAX, Duration::ZERO)?;

        assert_eq!(policy.delay_duration(2), Duration::MAX);

        Ok(())
    }

    /// ジッターを加算する前の待機時間が500ミリ秒、1秒、2秒と倍増し、2秒で頭打ちになる再試行設定を返す。
    fn policy_with_jitter(max_jitter: Duration) -> anyhow::Result<RetryPolicy> {
        Ok(RetryPolicy::new(
            5,
            Duration::from_millis(500),
            Duration::from_secs(2),
            max_jitter,
        )?)
    }

    /// 上限で切り詰めた待機時間にジッターを加算し、ジッターが0と`max_jitter`の両端の値を取りうることを確認
    #[test]
    fn test_jittered_delay_duration_reaches_both_ends() -> anyhow::Result<()> {
        // 取りうるジッターを0ナノ秒と1ナノ秒の2通りに限定し、結果が両端の値だけになることを確かめる。
        // 上限で切り詰められる再試行回数を指定し、ジッターを加算した後に上限で切り詰めないことも確かめる。
        let max_jitter = Duration::from_nanos(1);
        let policy = policy_with_jitter(max_jitter)?;
        let mut rng = StdRng::seed_from_u64(SEED);
        let max_delay = policy.max_delay();

        let durations: HashSet<Duration> = (0..TRIALS)
            .map(|_| policy.jittered_delay_duration(4, &mut rng))
            .collect();

        assert_eq!(
            durations,
            HashSet::from([max_delay, max_delay + max_jitter])
        );

        Ok(())
    }

    /// 再試行でない場合は、ジッターを加算せずに待機しないことを確認
    #[test]
    fn test_jittered_delay_duration_not_retry() -> anyhow::Result<()> {
        let policy = policy_with_jitter(Duration::from_secs(1))?;
        let mut rng = StdRng::seed_from_u64(SEED);

        for _ in 0..TRIALS {
            assert_eq!(policy.jittered_delay_duration(0, &mut rng), Duration::ZERO);
        }

        Ok(())
    }

    /// ジッターの上限が0の場合は、ジッターを加算しないことを確認
    #[test]
    fn test_jittered_delay_duration_without_jitter() -> anyhow::Result<()> {
        let policy = policy_with_jitter(Duration::ZERO)?;
        let mut rng = StdRng::seed_from_u64(SEED);

        assert_eq!(
            policy.jittered_delay_duration(1, &mut rng),
            Duration::from_millis(500)
        );

        Ok(())
    }

    /// ジッターの上限のナノ秒がu64に収まらない場合でも、ジッターが上限以下になることを確認
    #[test]
    fn test_jitter_exceeds_u64_nanos() -> anyhow::Result<()> {
        // u64のナノ秒は約584年分であるため、それを超える上限を指定する。
        let max_jitter = Duration::from_secs(u64::MAX);
        let policy = RetryPolicy::new(5, Duration::from_secs(1), Duration::MAX, max_jitter)?;
        let mut rng = StdRng::seed_from_u64(SEED);

        for _ in 0..TRIALS {
            let jitter = policy.jitter(&mut rng);
            assert!(jitter <= max_jitter, "jitter: {jitter:?}");
        }

        Ok(())
    }

    /// 待機時間とジッターの和がDurationの最大値を超えても、パニックせずに最大値を返すことを確認
    #[test]
    fn test_jittered_delay_duration_saturates() -> anyhow::Result<()> {
        let policy = RetryPolicy::new(5, Duration::MAX, Duration::MAX, Duration::MAX)?;
        let mut rng = StdRng::seed_from_u64(SEED);

        for _ in 0..TRIALS {
            let duration = policy.jittered_delay_duration(1, &mut rng);
            assert_eq!(duration, Duration::MAX);
        }

        Ok(())
    }

    /// Retry-Afterヘッダーが示す待機時間の有無と上限に応じて、再試行前の待機時間を決定することを確認
    #[rstest]
    // Retry-Afterヘッダーがない場合は指数バックオフで待機する
    #[case::backoff(None, Ok(Duration::from_millis(500)))]
    #[case::zero_retry_after(Some(Duration::ZERO), Ok(Duration::ZERO))]
    // Retry-Afterヘッダーが示す待機時間が上限と一致する
    #[case::retry_after_equals_max_delay(Some(Duration::from_secs(3)), Ok(Duration::from_secs(3)))]
    // Retry-Afterヘッダーが示す待機時間が上限を1ナノ秒超える場合は、その待機時間をエラーとして返す
    #[case::retry_after_exceeds_max_delay(
        Some(Duration::from_secs(3) + Duration::from_nanos(1)),
        Err(Duration::from_secs(3) + Duration::from_nanos(1))
    )]
    fn test_retry_delay(
        #[case] retry_after: Option<Duration>,
        #[case] expected: Result<Duration, Duration>,
    ) -> anyhow::Result<()> {
        let policy = RetryPolicy::new(
            5,
            Duration::from_millis(500),
            Duration::from_secs(3),
            Duration::ZERO,
        )?;
        let mut rng = StdRng::seed_from_u64(SEED);

        let result = policy.retry_delay(1, retry_after, &mut rng);

        assert_eq!(result, expected);

        Ok(())
    }

    /// Retry-Afterヘッダーが示す待機時間より短くならないように、ジッターを加算のみすることを確認
    #[test]
    fn test_retry_delay_adds_jitter_to_retry_after() -> anyhow::Result<()> {
        let max_jitter = Duration::from_nanos(1);
        let retry_after = Duration::from_secs(1);
        let policy = policy_with_jitter(max_jitter)?;
        let mut rng = StdRng::seed_from_u64(SEED);

        let delays = (0..TRIALS)
            .map(|_| policy.retry_delay(1, Some(retry_after), &mut rng))
            .collect::<Result<HashSet<Duration>, Duration>>()
            .map_err(|d| anyhow::anyhow!("retry_after was rejected: {d:?}"))?;

        assert_eq!(
            delays,
            HashSet::from([retry_after, retry_after + max_jitter])
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_idempotency {
    use rstest::rstest;

    use super::*;

    /// HTTPメソッドの意味から冪等性を決定することを確認
    #[rstest]
    #[case::get(Method::GET, Idempotency::Idempotent)]
    #[case::head(Method::HEAD, Idempotency::Idempotent)]
    #[case::put(Method::PUT, Idempotency::Idempotent)]
    #[case::delete(Method::DELETE, Idempotency::Idempotent)]
    #[case::options(Method::OPTIONS, Idempotency::Idempotent)]
    #[case::post(Method::POST, Idempotency::NonIdempotent)]
    #[case::patch(Method::PATCH, Idempotency::NonIdempotent)]
    fn test_from_method(#[case] method: Method, #[case] expected: Idempotency) {
        assert_eq!(Idempotency::from_method(&method), expected);
    }
}

#[cfg(test)]
mod test_is_retryable_error_response {
    use rstest::rstest;

    use super::*;

    use Idempotency::{Idempotent, NonIdempotent};

    /// 429は冪等性によらず、5xxは冪等なリクエストのみ再試行することを確認
    #[rstest]
    #[case::bad_request(400, Idempotent, false)]
    #[case::unauthorized(401, Idempotent, false)]
    #[case::before_too_many_requests(428, Idempotent, false)]
    #[case::too_many_requests(429, Idempotent, true)]
    #[case::too_many_requests_non_idempotent(429, NonIdempotent, true)]
    #[case::after_too_many_requests(430, Idempotent, false)]
    #[case::max_client_error(499, Idempotent, false)]
    #[case::min_server_error(500, Idempotent, true)]
    #[case::min_server_error_non_idempotent(500, NonIdempotent, false)]
    #[case::max_server_error(599, Idempotent, true)]
    #[case::max_server_error_non_idempotent(599, NonIdempotent, false)]
    #[case::after_server_error(600, Idempotent, false)]
    fn test_is_retryable_error_response(
        #[case] status: u16,
        #[case] idempotency: Idempotency,
        #[case] expected: bool,
    ) -> anyhow::Result<()> {
        assert_eq!(
            is_retryable_error_response(StatusCode::from_u16(status)?, None, idempotency),
            expected
        );

        Ok(())
    }

    /// エラーコードが`operation_blocked_temporary`の409だけを、冪等性によらず再試行することを確認
    #[rstest]
    #[case::blocked(409, Some("operation_blocked_temporary"), Idempotent, true)]
    #[case::blocked_non_idempotent(409, Some("operation_blocked_temporary"), NonIdempotent, true)]
    #[case::conflict_without_code(409, None, NonIdempotent, false)]
    #[case::name_in_use(409, Some("item_name_in_use"), NonIdempotent, false)]
    #[case::code_differs_in_case(409, Some("OPERATION_BLOCKED_TEMPORARY"), NonIdempotent, false)]
    #[case::blocked_code_before_conflict(
        408,
        Some("operation_blocked_temporary"),
        NonIdempotent,
        false
    )]
    #[case::blocked_code_after_conflict(
        410,
        Some("operation_blocked_temporary"),
        NonIdempotent,
        false
    )]
    fn test_is_retryable_error_response_conflict(
        #[case] status: u16,
        #[case] code: Option<&str>,
        #[case] idempotency: Idempotency,
        #[case] expected: bool,
    ) -> anyhow::Result<()> {
        assert_eq!(
            is_retryable_error_response(StatusCode::from_u16(status)?, code, idempotency),
            expected
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_parse_retry_after {
    use reqwest::header::HeaderValue;
    use rstest::rstest;

    use super::*;

    /// Retry-Afterヘッダーを秒数として解釈できる場合のみ待機時間を返すことを確認
    #[rstest]
    #[case::zero("0", Some(Duration::ZERO))]
    #[case::one("1", Some(Duration::from_secs(1)))]
    #[case::with_spaces(" 30 ", Some(Duration::from_secs(30)))]
    #[case::negative("-1", None)]
    #[case::fraction("1.5", None)]
    #[case::http_date("Wed, 21 Oct 2015 07:28:00 GMT", None)]
    #[case::empty("", None)]
    fn test_parse_retry_after(#[case] value: &'static str, #[case] expected: Option<Duration>) {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static(value));

        assert_eq!(parse_retry_after(&headers), expected);
    }

    /// Retry-Afterヘッダーがない場合は、待機時間を返さないことを確認
    #[test]
    fn test_parse_retry_after_without_header() {
        assert_eq!(parse_retry_after(&HeaderMap::new()), None);
    }
}

#[cfg(test)]
mod test_run_with_retry {
    use std::sync::atomic::{AtomicU32, Ordering};

    use rstest::rstest;

    use super::*;

    /// テストが長時間待機しないように、短い待機時間の再試行設定を返す。
    fn short_policy(max_retries: u32) -> anyhow::Result<RetryPolicy> {
        Ok(RetryPolicy::new(
            max_retries,
            Duration::from_millis(1),
            Duration::from_millis(5),
            Duration::from_millis(1),
        )?)
    }

    /// 試行が失敗したことを表すエラーを返す。
    fn dummy_error() -> BoxClientError {
        BoxClientError::InvalidRetryPolicy("dummy")
    }

    /// 再試行できるエラーの後に再試行し、成功した値を返すことを確認
    #[tokio::test]
    async fn test_retries_until_success() -> anyhow::Result<()> {
        let attempts = AtomicU32::new(0);

        let value = run_with_retry(&short_policy(3)?, "test", || async {
            if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(AttemptError::Retryable {
                    error: dummy_error(),
                    retry_after: None,
                })
            } else {
                Ok("done")
            }
        })
        .await?;

        assert_eq!(value, "done");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        Ok(())
    }

    /// 再試行できないエラーの場合は、再試行せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_does_not_retry_fatal_error() -> anyhow::Result<()> {
        let attempts = AtomicU32::new(0);

        let result: BoxClientResult<()> = run_with_retry(&short_policy(3)?, "test", || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(AttemptError::Fatal(dummy_error()))
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        Ok(())
    }

    /// 再試行回数の上限に達した場合は、最初の試行と上限回数の再試行の後にエラーを返すことを確認
    #[rstest]
    #[case::no_retry(0, 1)]
    #[case::one_retry(1, 2)]
    #[case::two_retries(2, 3)]
    #[tokio::test]
    async fn test_gives_up_after_max_retries(
        #[case] max_retries: u32,
        #[case] expected_attempts: u32,
    ) -> anyhow::Result<()> {
        let attempts = AtomicU32::new(0);

        let result: BoxClientResult<()> =
            run_with_retry(&short_policy(max_retries)?, "test", || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(AttemptError::Retryable {
                    error: dummy_error(),
                    retry_after: None,
                })
            })
            .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), expected_attempts);

        Ok(())
    }

    /// Retry-Afterヘッダーが示す待機時間が上限を超える場合は、再試行せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_gives_up_when_retry_after_too_long() -> anyhow::Result<()> {
        let attempts = AtomicU32::new(0);

        let result: BoxClientResult<()> = run_with_retry(&short_policy(3)?, "test", || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(AttemptError::Retryable {
                error: dummy_error(),
                retry_after: Some(Duration::from_secs(1)),
            })
        })
        .await;

        // 呼び出し元が原因を調べられるように、Retry-Afterヘッダーを返したリクエストのエラーを保持する。
        assert!(
            matches!(
                &result,
                Err(BoxClientError::RetryAfterTooLong { retry_after, source, .. })
                    if *retry_after == Duration::from_secs(1)
                        && matches!(**source, BoxClientError::InvalidRetryPolicy("dummy"))
            ),
            "result: {result:?}"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        Ok(())
    }
}
