//! Synapse プラグイン C ABI — Single Source of Truth (Rust).
//!
//! このファイルが ABI の正本。C ヘッダ `synapse_abi.h` は cbindgen で生成する
//! (`cargo build` の build.rs か、`cbindgen --config cbindgen.toml -o synapse_abi.h .`)。
//!
//! 生成される C を「行儀のよい C」に保つための方針:
//!   - 型は `#[repr(C)]`。ハンドルは Nomicon 推奨のゼロサイズ struct + PhantomData
//!     （空 enum は uninhabited で参照を作ると即 UB のため不採用）にして、C 側は
//!     不完全型 `typedef struct Foo Foo;`（`Foo *` として使用）。
//!   - コールバックは `Option<unsafe extern "C" fn ...>`（= NULL 可能な C 関数ポインタ）。
//!   - 定数は `pub const` とし、cbindgen 設定で `#define` に落とす（enum の基底型は
//!     処理系定義なので ABI では #define が安全、という方針を Rust 側でも踏襲）。
//!   - `usize` は size_t、`c_int` は int、`*const c_char` は const char * に写す。
//!   - doc コメント `///` がそのまま C コメントになる。
//!
//! 設計意図・アーキテクチャの柱・ロードマップ・用語は、生成ヘッダ冒頭に
//! cbindgen の `header` として差し込む（cbindgen.toml 参照）。本体の Rust は
//! 宣言に徹し、散文は toml 側に置くことで「正本は Rust・説明はヘッダに出る」を両立する。
//!
//! 境界規約（両側が守る不変条件）:
//!   - **unwind は越えない**: コールバック境界を越える巻き戻し（Rust panic / C++ 例外）は
//!     未定義ではなく **abort** になる（関数ポインタは `extern "C"`）。実装側は境界内で必ず
//!     捕捉し、`SynStatus`（`SYN_ERR_*`）へ変換して返すこと。`extern "C-unwind"` は採らない
//!     ——「ABI は C・エラーは戻り値」という規律を優先する（ADR 参照）。
//!   - **ポインタは明記なき限り非 NULL**: 関数ポインタ引数のうち、doc で「NULL 可」と書いた
//!     ものだけが NULL を許す（例: サイズ問い合わせの `out=NULL`/`cap=0`、任意コールバック）。
//!     それ以外に NULL を渡すのは契約違反。信頼境界（プラグイン→ホスト）の実装は防御的に
//!     NULL を検査して `SYN_ERR_BAD_ARG` を返してよい。
//!   - **関数ポインタは `unsafe`**: 全コールバックは `Option<unsafe extern "C" fn ...>`。
//!     呼び出しは健全性前提（有効な ctx/ポインタ・寿命）に依存するため、呼ぶ側に `unsafe` を
//!     強制する。NULL 可能性（`Option`）と未検証ポインタ（`unsafe`）が型に表れる。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![warn(missing_docs)]

use core::ffi::{c_char, c_int, c_void};
use core::marker::{PhantomData, PhantomPinned};

// 文字列 ABI 定数(スイートID等)の正本。C 側へは build.rs が #define を生成する。
include!("abi_strings.rs");

/* ======================================================================== */
/*  基本スカラ型                                                            */
/* ======================================================================== */

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

/// URI をセッション内で写像した安定整数。型 ID もこの空間。0 は無効。
pub type SynUrid = u32;
/// 型 ID（URID と同一空間）。
pub type SynTypeId = SynUrid;

/// 無効 URID。空値 sentinel（`type_id == 0`）でもある。
pub const SYN_URID_INVALID: SynUrid = 0;
/// 予約: 汎用ノードの出力/入力で「任意型」を表す（方式a）。
/// 接続判定で ANY は全許容。データは実体 type_id を運ぶ。
pub const SYN_TYPE_ANY: SynTypeId = 1;

/* ======================================================================== */
/*  不透明ハンドル（すべてホスト所有）                                      */
/* ======================================================================== */

// 不透明ハンドルは「中身を見せない不完全型へのポインタ」として C へ渡す。
// 空 enum は uninhabited なので、それへの参照（`&SynNode`）を作ると即 UB になる
// （C 側から渡るポインタはハンドルとして有効だが Rust 的には居住者ゼロの型のため）。
// Nomicon 推奨のゼロサイズ struct + PhantomData 方式にすると、不完全型としての性質に加え
// `!Send`/`!Sync`/`!Unpin`（生ポインタと PhantomPinned に由来）も表現でき、cbindgen も
// 不完全型 `typedef struct Foo Foo;` に落とす。
macro_rules! opaque_handle {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[repr(C)]
        pub struct $name {
            _data: [u8; 0],
            _marker: PhantomData<(*mut u8, PhantomPinned)>,
        }
    };
}

opaque_handle!(
    /// ノードインスタンス側ハンドル（不透明）。
    SynNode
);
opaque_handle!(
    /// 宣言ビルダ（不透明）。
    SynDeclBuilder
);
opaque_handle!(
    /// 1 評価分の評価コンテキスト（不透明）。
    SynEvalCtx
);

/* ======================================================================== */
/*  データ単位（ワイヤフォーマット）                                        */
/* ======================================================================== */

/// エッジを流れるデータ単位。
///
/// `size <= sizeof(void*)` のとき payload は `ptr` フィールドに直接格納する
/// (small-value optimization)。読み書きは型 pun ではなく memcpy 経由で行うこと。
/// 不変条件: PLAIN 型の payload は位置独立な素のバイト列で、生ポインタを含まない。
/// 空（未接続かつデフォルト無し）の表現は `type_id == 0`。`ptr == NULL` は使わない
/// （SVO では inline の零値と区別できないため）。
#[repr(C)]
pub struct SynValue {
    /// 実体型の URID。0 は空。
    pub type_id: SynTypeId,
    /// size>ptr幅: 領域ポインタ / size<=ptr幅: 値そのもの(SVO) / opaque型: 不透明ハンドル。
    pub ptr: *mut c_void,
    /// 意味的なバイト数。
    pub size: usize,
}

/* ======================================================================== */
/*  型 vtable と型レジストリ（lookup 方式 + エスケープハッチ）              */
/* ======================================================================== */

/// memcpy/シリアライズ可能な素のバイト列。
pub const SYN_TYPE_PLAIN_BYTES: u32 = 1 << 0;
/// ptr は不透明ハンドル。get_api で操作する。
pub const SYN_TYPE_OPAQUE: u32 = 1 << 1;
/// clone は参照カウント増加（浅いコピー）。
pub const SYN_TYPE_SHARED: u32 = 1 << 2;

/// 型ごとの操作テーブル。メモリ確保/解放はホスト。型は構築/破棄/複製のみ知る。
#[repr(C)]
pub struct SynTypeVTable {
    /// SYN_TYPE_* フラグ。
    pub flags: u32,
    /// 固定サイズ。可変なら 0。
    pub size: usize,
    /// アラインメント要件（2の冪。register_type が検証する）。可変サイズ型では
    /// ホスト確保バッファ先頭のアラインメントとして解釈する（ADR-029）。
    pub align: usize,

    /// 既定値を dst に構築する。
    pub init: Option<unsafe extern "C" fn(dst: *mut c_void, t: SynTypeId) -> SynStatus>,
    /// 複製。PLAIN/可変型はディープコピー。SHARED/OPAQUE は refcount++ でよい
    /// （passthrough で大きな/GPU リソースを複製しないために重要）。
    pub clone: Option<unsafe extern "C" fn(dst: *mut c_void, src: *const c_void, t: SynTypeId) -> SynStatus>,
    /// 破棄のみ（free はホスト）。SHARED は refcount--、0 で実リソース解放。
    pub drop: Option<unsafe extern "C" fn(obj: *mut c_void, t: SynTypeId)>,

    /// 永続化（PLAIN のみ）。out=NULL/cap=0 で必要サイズを written に返す。OPAQUE は NULL 可。
    pub serialize:
        Option<unsafe extern "C" fn(obj: *const c_void, t: SynTypeId, out: *mut c_void, cap: usize, written: *mut usize) -> SynStatus>,
    /// 復元（PLAIN のみ）。OPAQUE は NULL 可。
    pub deserialize:
        Option<unsafe extern "C" fn(dst: *mut c_void, t: SynTypeId, input: *const c_void, len: usize) -> SynStatus>,

    /// 型+API 二層: opaque 型が自前の API テーブルを公開する（例 "synapse:gpu:texture"）。
    /// PLAIN 型は NULL。
    pub get_api: Option<unsafe extern "C" fn(t: SynTypeId, api_id: *const c_char) -> *const c_void>,
}

/// 型の登録・解決。
#[repr(C)]
pub struct SynTypeRegistrySuite {
    /// 型を URI と vtable で登録する。vt->align が 2 の冪でなければ SYN_ERR_BAD_ARG（ADR-029）。
    pub register_type: Option<unsafe extern "C" fn(uri: *const c_char, vt: *const SynTypeVTable) -> SynStatus>,
    /// 型 ID から vtable を解決する（結果はセッション中キャッシュ可）。
    pub lookup: Option<unsafe extern "C" fn(t: SynTypeId) -> *const SynTypeVTable>,
    /// URI から型 ID を得る。
    pub type_of: Option<unsafe extern "C" fn(uri: *const c_char) -> SynTypeId>,
}

/// URI と URID の相互変換。
#[repr(C)]
pub struct SynUridSuite {
    /// URI を URID に写像（intern、セッション不変）。
    pub map: Option<unsafe extern "C" fn(uri: *const c_char) -> SynUrid>,
    /// URID から URI を借用（セッション中のみ有効）。
    pub unmap: Option<unsafe extern "C" fn(id: SynUrid) -> *const c_char>,
}

/* ======================================================================== */
/*  宣言ビルダ（Blender 流: 状態からフル再宣言、ホストが key で接続を整合）  */
/* ======================================================================== */

/// fan-in: N 本のリンクを受け、順序付き配列で配送する入力ポート。
pub const SYN_PORT_MULTI: u32 = 1 << 0;

/// declare 内で呼ぶソケット宣言関数群。プラグインは確保せずこれらを呼ぶだけ。
/// key は配列インデックスではなく論理的同一性から導くこと（接続が壊れる）。
#[repr(C)]
pub struct SynDeclSuite {
    /// 出力ポート: 型はちょうど 1 つ。汎用ノードは SYN_TYPE_ANY を渡してよい（方式a）。
    pub output: Option<unsafe extern "C" fn(b: *mut SynDeclBuilder, key: *const c_char, label: *const c_char, ty: SynTypeId) -> SynStatus>,
    /// 入力ポート: types のいずれかを受理（多態）。SYN_TYPE_ANY で全許容。
    /// flags は SYN_PORT_*。
    pub input: Option<
        unsafe extern "C" fn(
            b: *mut SynDeclBuilder,
            key: *const c_char,
            label: *const c_char,
            types: *const SynTypeId,
            n_types: usize,
            flags: u32,
        ) -> SynStatus,
    >,
    /// 方式b（型伝播）: 接続中の入力の実体型を返す。declare 内で出力型の導出に使う。
    /// 未接続/不定なら SYN_TYPE_ANY。v1 は使わず ANY で書いてよい。
    pub connected_type: Option<unsafe extern "C" fn(b: *mut SynDeclBuilder, input_key: *const c_char, link_index: u32) -> SynTypeId>,
    /// 入力ソケットの初期デフォルト値（未接続時に get_input が返す値=パラメータ）。
    /// value は値渡し（SynValue 自体はコピー。大型データは value.ptr 経由の借用で、
    /// 呼び出し中のみ有効）。ホストが型 vtable で複製して保持する。
    /// 再 declare では key 一致ソケットの既存値を保持し、初期値は新規 key にのみ適用。
    pub input_default: Option<unsafe extern "C" fn(b: *mut SynDeclBuilder, key: *const c_char, value: SynValue) -> SynStatus>,
}

/* ======================================================================== */
/*  評価コンテキスト                                                        */
/* ======================================================================== */

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
/// 参照は SynValue 内の `ptr` を通してのみ行い、大型データ(>ptr幅)の `ptr` が指す領域は
/// ホスト所有・その呼び出し中のみ借用可能。SVO(≤ptr幅)は値が `ptr` フィールドに入った
/// まま丸ごとコピーされるので、プラグインローカルがホストから見えない問題は起きない。
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

    /// process 中: 大型(>ptr幅)出力用にホスト所有バッファを確保して先頭ポインタを返す。
    /// プラグインはここへ書き、その ptr を SynValue.ptr に入れて set_output に値渡しする。
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

/* ======================================================================== */
/*  ノード記述子                                                            */
/* ======================================================================== */

/// インスタンス再入タイリング（宣言のみ・実装は後回し可）。
pub const SYN_CAP_REENTRANT_TILING: u32 = 1 << 0;
/// フレーム並列レンダ（宣言のみ）。
pub const SYN_CAP_PARALLEL_FRAMES: u32 = 1 << 1;
/// UI と process の同一インスタンス並行を許可。
pub const SYN_CAP_THREAD_SAFE_UI: u32 = 1 << 2;

/// 1 ノード種別の記述子。ロード時に host->register_node で登録する。
/// 必須: create/destroy/declare/negotiate/process。
#[repr(C)]
pub struct SynNodeDesc {
    /// SYN_CAP_*。
    pub caps: u32,
    /// ノード URI（"com.vendor.blur.gaussian"）。モジュール寿命まで存続(static 推奨)。
    pub node_uri: *const c_char,
    /// 表示名。
    pub display_name: *const c_char,

    /// 既定状態でインスタンスを生成。node は instance に保持し host->mark_dirty 等に使う。
    pub create: Option<unsafe extern "C" fn(node: *mut SynNode, out_instance: *mut *mut c_void) -> SynStatus>,
    /// インスタンス破棄。
    pub destroy: Option<unsafe extern "C" fn(instance: *mut c_void)>,

    /// 宣言フェーズ（データ無し）。状態からフル再宣言する冪等関数。
    pub declare: Option<unsafe extern "C" fn(instance: *mut c_void, b: *mut SynDeclBuilder) -> SynStatus>,

    /// poll 列挙（データ無し・単発）。必要入力を request で積み SYN_OK を返す。
    /// 静的ノードは全入力を列挙するだけ。値依存の枝刈りは将来の二層 API で対応。
    pub negotiate: Option<unsafe extern "C" fn(instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus>,

    /// 処理（1 回）。要求が全充足された後に呼ばれる。
    pub process: Option<unsafe extern "C" fn(instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus>,

    /// ソケットに出ない内部パラメータの永続化（不透明 blob）。out=NULL/cap=0 で
    /// 必要サイズを written に返し、確保後に再呼び出しで書き込む。blob は自己記述
    /// （自前の version を内包）。ソケットデフォルトは host 専有でプラグインは値を
    /// 保持しない（get_input の借用のみ）ため、blob には構造的に内部パラメータだけが
    /// 入る。NULL 可。
    pub save_state: Option<unsafe extern "C" fn(instance: *mut c_void, out: *mut c_void, cap: usize, written: *mut usize) -> SynStatus>,
    /// 内部パラメータの復元。NULL 可。
    pub load_state: Option<unsafe extern "C" fn(instance: *mut c_void, input: *const c_void, len: usize) -> SynStatus>,

    /// 任意拡張（UI 等）。未対応は NULL を返す。
    pub get_extension: Option<unsafe extern "C" fn(instance: *mut c_void, ext_id: *const c_char) -> *const c_void>,
}

/// UI コンポーネント拡張。
#[repr(C)]
pub struct SynUiExt {
    /// UI を構築する（host_ui_handle はホスト側 UI コンテキスト）。
    pub build: Option<unsafe extern "C" fn(instance: *mut c_void, host_ui_handle: *mut c_void) -> SynStatus>,
    /// パラメータ変更の通知（param_key は変更されたソケット key）。
    pub on_change: Option<unsafe extern "C" fn(instance: *mut c_void, param_key: *const c_char) -> SynStatus>,
}

/* ======================================================================== */
/*  ホスト & モジュールエントリポイント                                     */
/* ======================================================================== */

/// ホストが提供する操作群。on_load で受け取りモジュール側に保持する（1 モジュール=1 ホスト）。
#[repr(C)]
pub struct SynHostStruct {
    /// ホスト側の不透明ポインタ。
    pub host_ctx: *mut c_void,
    /// スイートを id で取得（未提供なら NULL）。
    pub fetch_suite: Option<unsafe extern "C" fn(h: *mut SynHostStruct, suite_id: *const c_char) -> *const c_void>,
    /// ノード記述子を登録（on_load 中に呼ぶ）。
    pub register_node: Option<unsafe extern "C" fn(h: *mut SynHostStruct, desc: *const SynNodeDesc) -> SynStatus>,
    /// 状態変更通知。当該ノード+下流サブツリーのキャッシュを無効化する。
    pub mark_dirty: Option<unsafe extern "C" fn(h: *mut SynHostStruct, node: *mut SynNode)>,
    /// ログ出力（level は SYN_LOG_*）。
    pub log: Option<unsafe extern "C" fn(h: *mut SynHostStruct, level: c_int, msg: *const c_char)>,
}

/// ホストハンドル（SynHostStruct へのポインタ）。
pub type SynHost = *mut SynHostStruct;

/// モジュール記述子。各 .so/.dll は `synapse_module` を 1 つだけエクスポートする。
///
/// ロードは 2 フェーズ（ADR-027）: ホストは**全モジュール**の `on_register_types` を先に呼び、
/// その後**全モジュール**の `on_register_nodes` を呼ぶ。これにより「ノード登録時には参照しうる
/// 型がすべて出揃っている」を保証する。型登録フェーズ内で他モジュールの型に依存してはならない
/// （型-型依存の禁止、ADR-028 ★）。
#[repr(C)]
pub struct SynModule {
    /// ビルド時の SYN_ABI_VERSION。
    pub abi_version: u32,
    /// 名前空間（"com.vendor.blur"）。
    pub module_uri: *const c_char,
    /// semver 文字列。
    pub module_version: *const c_char,
    /// フェーズ1: 型をここで登録する（スイート fetch もここで行う）。型が無ければ NULL 可。
    /// 他モジュールの型を lookup してはならない（ADR-028。ホストは違反を拒否してよい）。
    pub on_register_types: Option<unsafe extern "C" fn(h: SynHost) -> SynStatus>,
    /// フェーズ2: ノードをここで登録する。全モジュールの型登録後に呼ばれる。
    /// ノードが無ければ NULL 可。
    pub on_register_nodes: Option<unsafe extern "C" fn(h: SynHost) -> SynStatus>,
    /// アンロード前に 1 回。
    pub on_unload: Option<unsafe extern "C" fn(h: SynHost)>,
}

/// ABI バージョン（当面はこの 1 個で足りる）。
/// v2: 2フェーズロード（ADR-027）＋ alloc の type_id 引数（ADR-029）。
pub const SYN_ABI_VERSION: u32 = 2;

/// モジュールエントリのシグネチャ: `const SynModule *synapse_module(void);`
pub type SynModuleEntryFn = Option<unsafe extern "C" fn() -> *const SynModule>;