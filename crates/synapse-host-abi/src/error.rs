//! ホスト側境界層のエラー型と結果型。
//!
//! # 意図
//! ABI 境界で起こりうる失敗（ロード失敗・シンボル欠落・ABI 不一致・プラグインの
//! エラーステータス・必須コールバック NULL・呼び出し順序違反）を、ひとつの [`Error`] 列挙へ
//! 集約する。`SynStatus`（C-ABI の整数コード）はそのまま外へ漏らさず [`Error::Status`] に包む。
//!
//! # 使い方
//! 公開 API はすべて [`Result`]（= `std::result::Result<T, Error>`）を返す。プラグインの
//! `SynStatus` は [`check`] で `Result<()>` に変換する。

use synapse_abi::{SynStatus, SYN_OK};

/// ホスト側境界層のエラー。
#[derive(Debug)]
pub enum Error {
    /// ライブラリ（DLL / .so / .dylib）のロード失敗。
    Load(String),
    /// `synapse_module` エントリシンボルが見つからない、または NULL を返した。
    MissingEntry,
    /// ABI バージョン不一致（プラグインとホストの想定が食い違う）。
    AbiVersion {
        /// プラグインが申告した ABI バージョン。
        found: u32,
        /// ホストが期待する ABI バージョン。
        expected: u32,
    },
    /// プラグインがエラーステータス（`SYN_ERR_*`）を返した。
    Status(SynStatus),
    /// 必須コールバック関数ポインタが NULL だった（フィールド名を保持）。
    NullCallback(&'static str),
    /// `declare` を呼ぶ前に `negotiate` / `process` を呼んだ（順序違反）。
    NotDeclared,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Load(s) => write!(f, "module load failed: {s}"),
            Error::MissingEntry => write!(f, "`synapse_module` symbol not found"),
            Error::AbiVersion { found, expected } => {
                write!(f, "ABI version mismatch: plugin={found} host={expected}")
            }
            Error::Status(s) => write!(f, "plugin returned error status {s}"),
            Error::NullCallback(n) => write!(f, "required callback is NULL: {n}"),
            Error::NotDeclared => write!(f, "declare() must be called before negotiate/process"),
        }
    }
}
impl std::error::Error for Error {}

/// 本クレート共通の結果型。
pub type Result<T> = std::result::Result<T, Error>;

/// プラグインの `SynStatus` を `Result<()>` に変換する。`SYN_OK` 以外は [`Error::Status`]。
pub(crate) fn check(st: SynStatus) -> Result<()> {
    if st == SYN_OK {
        Ok(())
    } else {
        Err(Error::Status(st))
    }
}
