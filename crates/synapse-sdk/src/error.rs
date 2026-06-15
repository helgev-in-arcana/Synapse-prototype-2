//! SDK のエラー型と結果型。
//!
//! # 意図
//! ノード作者の `process` / `load_state` が返しうる失敗を [`Error`] に集約し、ABI 境界では
//! `SynStatus`（整数コード）へ変換する。作者は `?` で素直に失敗を伝播でき、SDK 側が
//! トランポリンでステータスへ落とす。

use synapse_abi::{SynStatus, SYN_ERR_BAD_ARG, SYN_ERR_TYPE_MISMATCH};

/// ノード処理が返しうるエラー。`SynStatus` へ変換されて ABI 境界を越える。
#[derive(Debug)]
pub enum Error {
    /// 入力値の型が期待と異なる。
    TypeMismatch,
    /// 内部状態が不正（load_state の入力長不足など）。
    BadState,
    /// 任意のステータスコードを直接返す。
    Status(SynStatus),
}

impl Error {
    /// ABI 境界へ返す `SynStatus` へ変換する。
    pub(crate) fn to_status(&self) -> SynStatus {
        match self {
            Error::TypeMismatch => SYN_ERR_TYPE_MISMATCH,
            Error::BadState => SYN_ERR_BAD_ARG,
            Error::Status(s) => *s,
        }
    }
}

/// SDK 共通の結果型。
pub type Result<T> = core::result::Result<T, Error>;
