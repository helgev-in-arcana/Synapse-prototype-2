//! 作者が実装する [`Node`] トレイト。
//!
//! # 意図
//! プラグイン作者の唯一の実装責務。ポート宣言（[`Node::declare`]）と処理本体
//! （[`Node::process`]）を書けば、SDK が C-ABI 境界・グローバル・negotiate の縮退形を補う。
//! `negotiate` / `save_state` / `load_state` は既定実装を持ち、必要なときだけ override する。
//! 作者のコードに unsafe / グローバル / negotiate は現れない。

use core::ffi::CStr;

use crate::context::{Declarer, NegotiateCtx, ProcessCtx};
use crate::error::Result;

/// 処理ノード。作者はこれを実装する。
pub trait Node: Default + 'static {
    /// ノード URI（例 `c"com.vendor.blur.gaussian"`）。
    const URI: &'static CStr;
    /// エディタ等に表示する名前。
    const DISPLAY_NAME: &'static CStr;

    /// ポートを宣言する（状態からフル再宣言・冪等）。
    fn declare(&mut self, d: &mut Declarer);

    /// 処理本体。`ctx.get` / `ctx.set` で入出力する。
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<()>;

    /// 必要入力の列挙。既定は全入力（静的ノード）。値依存の枝刈りが要る時だけ override する。
    fn negotiate(&mut self, ctx: &mut NegotiateCtx) {
        ctx.request_all();
    }

    /// ソケットに出ない内部パラメータの保存。既定は無し。
    fn save_state(&self) -> Option<Vec<u8>> {
        None
    }

    /// 内部パラメータの復元。
    fn load_state(&mut self, _bytes: &[u8]) -> Result<()> {
        Ok(())
    }
}
