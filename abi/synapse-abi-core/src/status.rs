//! ステータスコードとログレベル（基底層・スカラのみ）。

use core::ffi::c_int;

/// ステータスコード（SYN_OK / SYN_ERR_*）。
pub type SynStatus = i32;

/// 成功。
pub const SYN_OK: SynStatus = 0;
/// 原因不明の失敗（FFI 越えパニックの遮断時にも使う）。
pub const SYN_ERR_UNKNOWN: SynStatus = -1;
/// 要求された操作が未対応。
pub const SYN_ERR_UNSUPPORTED: SynStatus = -2;
/// 引数が不正。
pub const SYN_ERR_BAD_ARG: SynStatus = -3;
/// メモリ確保に失敗。
pub const SYN_ERR_NO_MEMORY: SynStatus = -4;
/// 型が一致しない。
pub const SYN_ERR_TYPE_MISMATCH: SynStatus = -5;

/// ログレベル（host->log の level 引数）: エラー。
pub const SYN_LOG_ERROR: c_int = 0;
/// ログレベル: 警告。
pub const SYN_LOG_WARN: c_int = 1;
/// ログレベル: 情報。
pub const SYN_LOG_INFO: c_int = 2;
/// ログレベル: デバッグ。
pub const SYN_LOG_DEBUG: c_int = 3;
