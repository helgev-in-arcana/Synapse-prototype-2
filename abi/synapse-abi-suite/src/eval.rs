//! 評価スイート（`SYN_EVAL_SUITE`）。

use core::ffi::c_void;

use synapse_abi_core::{SynEvalCtx, SynStatus, SynTypeId, SynValue};

/// 時刻（フレーム）を有理数で表す。単一フレームなら未使用可。
#[repr(C)]
pub struct SynRational {
    /// 分子。
    pub num: i64,
    /// 分母。
    pub den: i64,
}

/// 入力要求記述子。領域(ROI)は v1 では扱わない。
#[repr(C)]
pub struct SynRequest {
    /// 宣言した入力ポート（declare の呼び出し順 0 始まり）。
    pub input_index: u32,
    /// multi-input 上のどのリンクか（単一なら 0）。
    pub link_index: u32,
    /// 必要な時刻。
    pub frame: SynRational,
}

/// negotiate/process から使う評価操作群。
///
/// データ受け渡しの規約: `SynValue` は常に**値渡し**で境界を越える（構造体自体はコピー）。
/// 参照は payload の `ptr` を通してのみ行い、大型データ(>SYN_VALUE_INLINE)の `ptr` が指す
/// 領域はホスト所有・その呼び出し中のみ借用可能。SVO(≤SYN_VALUE_INLINE)は値が payload に
/// 入ったまま丸ごとコピーされるので、プラグインローカルがホストから見えない問題は起きない。
#[repr(C)]
pub struct SynEvalSuite {
    /// negotiate 中: 必要入力を積む。
    pub request: Option<unsafe extern "C" fn(ctx: *mut SynEvalCtx, req: *const SynRequest) -> SynStatus>,
    /// multi-input ポートに接続されたリンク数。
    pub link_count: Option<unsafe extern "C" fn(ctx: *mut SynEvalCtx, input_index: u32) -> u32>,

    /// process 中: 入力値を**値で受け取る**（この呼び出しの間のみ有効＝大型データの ptr が
    /// 指すホスト所有領域はこの呼び出し中だけ借用可能）。
    /// 未接続でも、デフォルトを持つソケットはホスト用意のデフォルト値を返す。
    /// デフォルトの無い未接続ソケットは type_id==0（空）→plugin が処理する。
    pub get_input: Option<unsafe extern "C" fn(ctx: *mut SynEvalCtx, input_index: u32, link_index: u32) -> SynValue>,

    /// process 中: 大型(>SYN_VALUE_INLINE)出力用にホスト所有バッファを確保して先頭ポインタを
    /// 返す。プラグインはここへ書き、その ptr を payload に入れて set_output に値渡しする。
    /// 確保はホスト（ADR-012）。`t` は値の実体型（ANY 宣言の汎用ノードは解決済み実体型を
    /// 渡す）。ホストは登録済み vtable の align 属性を満たすバッファを返す（ADR-029）。
    /// SVO 型は確保不要（set_output だけで完結）。失敗・未登録型は NULL。
    pub alloc: Option<unsafe extern "C" fn(ctx: *mut SynEvalCtx, size: usize, t: SynTypeId) -> *mut c_void>,

    /// process 中: 生産した出力値を**値渡し**でホストへ引き渡す。SVO はこれだけで完結。
    /// value.type_id は宣言した出力型に一致（ANY 宣言の汎用ノードは解決済み実体型を入れる）。
    pub set_output: Option<unsafe extern "C" fn(ctx: *mut SynEvalCtx, output_index: u32, value: SynValue) -> SynStatus>,

    /// 未知型パススルー保証: 中身を見ず入力値を出力へ転送する（値渡し。ホストが clone）。
    pub passthrough: Option<unsafe extern "C" fn(ctx: *mut SynEvalCtx, output_index: u32, input_value: SynValue) -> SynStatus>,
}

/// スイート ID: 評価。
pub const SYN_EVAL_SUITE: &str = "synapse:eval";
