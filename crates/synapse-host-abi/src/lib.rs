//! ホスト側 C-ABI 境界層。
//!
//! 責務は **C-ABI を安全な Rust に写すこと**だけ。扱う粒度は「個別モジュール」と
//! 「個別ノードインスタンス」まで。ノード同士の関係（リンク・評価順序・値の配線・
//! キャッシュ）は一切知らない——それは本体ホスト（FFI/unsafe 無しの上位クレート）の責務。
//!
//! 設計上の制約（ADR 参照）:
//!   - urid intern / 型レジストリ / ノード登録はプロセスグローバル（ADR-023, 1プロセス1
//!     セッション）。`Session` がその唯一の窓口。
//!   - `SynValue` は値渡し（ADR-022）。SVO の capture/present は [`OwnedValue`] が内包。
//!   - 同一インスタンスへの declare/negotiate/process は重ならない（ADR-019）。各メソッドが
//!     `&mut self` を取るため Rust の借用規則でコンパイル時に保証される。
//!
//! 制御の反転: 評価ループ（上流の充足）は本体ホストが回す。本層は negotiate で必要入力の
//! 一覧（[`Request`]）を返し、process には本体が用意した入力（[`InputBindings`]）を受け取る。

#![warn(missing_docs)]

use core::ffi::{c_char, c_int, c_void};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::ptr::null_mut;
use std::sync::{Mutex, MutexGuard, OnceLock};

use libloading::{Library, Symbol};
use synapse_abi::*;

/* ======================================================================= */
/*  FFI 越えパニック遮断 & ポインタ運搬                                     */
/* ======================================================================= */

// ホスト側コールバックは `extern "C"`。境界を越える巻き戻しは abort になる（ABI 契約）。
// プラグインの不正引数等で発火した panic をプロセス全体の abort にせず、安全なデフォルト値へ
// 落として返す（堅牢性方針: 不正は SYN_ERR_* / 空値で表す）。

/// `SynStatus` を返すコールバックのガード。panic は `SYN_ERR_UNKNOWN`。
fn guard_status(f: impl FnOnce() -> SynStatus) -> SynStatus {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(SYN_ERR_UNKNOWN)
}
/// 任意の値を返すコールバックのガード。panic 時は `default`。
fn guard_or<R>(default: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}
/// 返り値なしコールバックのガード。panic は握り潰す。
fn guard_unit(f: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

/// `Send`/`Sync` を付けた生ポインタ運搬用 newtype（`usize` で運ぶより provenance と意図が明確）。
///
/// # Safety
/// 指す先はホスト所有またはモジュールイメージ内の静的データ。1プロセス=1セッション(ADR-023)で、
/// 登録/解決/除去は [`SESSION`] の `Mutex` でシリアライズされるためデータレースしない。生存は
/// 登録元モジュールに従う（アンロード時に [`LoadedModule::drop`] が当該エントリを除去する）。
struct SendPtr<T>(*const T);
// `derive(Clone, Copy)` は `T: Copy` 境界を付けてしまう（T=vtable/desc は非 Copy）ため手書きする。
// ポインタ自体は常に Copy なので SendPtr<T> は T によらず Copy。
impl<T> Clone for SendPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for SendPtr<T> {}
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

/* ======================================================================= */
/*  エラー                                                                 */
/* ======================================================================= */

/// ホスト側境界層のエラー。
#[derive(Debug)]
pub enum Error {
    /// ライブラリのロード失敗。
    Load(String),
    /// `synapse_module` シンボルが見つからない。
    MissingEntry,
    /// ABI バージョン不一致。
    AbiVersion {
        /// プラグインの ABI バージョン。
        found: u32,
        /// ホストが期待する ABI バージョン。
        expected: u32,
    },
    /// プラグインがエラーステータスを返した。
    Status(SynStatus),
    /// 必須コールバックが NULL。
    NullCallback(&'static str),
    /// declare 前に negotiate/process を呼んだ。
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

fn check(st: SynStatus) -> Result<()> {
    if st == SYN_OK {
        Ok(())
    } else {
        Err(Error::Status(st))
    }
}

fn cstr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/* ======================================================================= */
/*  値（SynValue の値渡し ⇄ ホスト所有バイト列）                            */
/* ======================================================================= */

const PTR_SIZE: usize = core::mem::size_of::<*mut c_void>();

/// ホスト所有の値。SVO（≤8byte）も大型も、先頭から `size` バイトに実体を持つ。
pub struct OwnedValue {
    type_id: SynTypeId,
    size: usize,
    bytes: Vec<u8>, // 長さは size.max(8)
}

impl OwnedValue {
    /// 空値（`type_id == 0`）。未接続かつ既定値なしのソケットを表す。
    pub fn empty() -> Self {
        Self {
            type_id: SYN_URID_INVALID,
            size: 0,
            bytes: vec![0u8; PTR_SIZE],
        }
    }

    /// 生バイト列から PLAIN 値を作る。
    pub fn from_plain_bytes(type_id: SynTypeId, src: &[u8]) -> Self {
        let mut bytes = vec![0u8; src.len().max(PTR_SIZE)];
        bytes[..src.len()].copy_from_slice(src);
        Self {
            type_id,
            size: src.len(),
            bytes,
        }
    }

    /// 空値（`type_id == 0`）か。
    pub fn is_empty(&self) -> bool {
        self.type_id == SYN_URID_INVALID
    }
    /// 値の型 ID。
    pub fn type_id(&self) -> SynTypeId {
        self.type_id
    }
    /// 実体バイト列（長さ `size`）。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.size]
    }

    /// プラグインが値渡しした `SynValue` をホスト所有へコピーする。
    unsafe fn from_value(v: &SynValue) -> Self {
        let sz = v.size;
        let mut bytes = vec![0u8; sz.max(PTR_SIZE)];
        if sz <= PTR_SIZE {
            // SVO: 値は ptr フィールドそのもの。
            core::ptr::copy_nonoverlapping(
                (&v.ptr as *const *mut c_void) as *const u8,
                bytes.as_mut_ptr(),
                sz,
            );
        } else {
            core::ptr::copy_nonoverlapping(v.ptr as *const u8, bytes.as_mut_ptr(), sz);
        }
        Self {
            type_id: v.type_id,
            size: sz,
            bytes,
        }
    }

    /// プラグインへ値渡しする `SynValue` を組み立てる。≤8byte は SVO（ptr フィールドへインライン）。
    /// 大型は self.bytes を指す（self が生存する間のみ有効）。
    unsafe fn to_value(&self) -> SynValue {
        if self.size <= PTR_SIZE {
            let mut bits: usize = 0;
            core::ptr::copy_nonoverlapping(
                self.bytes.as_ptr(),
                (&mut bits as *mut usize) as *mut u8,
                self.size,
            );
            SynValue {
                type_id: self.type_id,
                ptr: bits as *mut c_void,
                size: self.size,
            }
        } else {
            SynValue {
                type_id: self.type_id,
                ptr: self.bytes.as_ptr() as *mut c_void,
                size: self.size,
            }
        }
    }
}

/* ======================================================================= */
/*  宣言結果                                                               */
/* ======================================================================= */

/// 入力ポートの宣言。
#[derive(Debug)]
pub struct InputDecl {
    /// 論理同一性を担う安定 key（再 declare を跨いで安定）。
    pub key: String,
    /// 受理する型集合（多態）。
    pub types: Vec<SynTypeId>,
    /// `SYN_PORT_*` フラグ（multi-input 等）。
    pub flags: u32,
    /// 既定値を持つか。
    pub has_default: bool,
}
/// 出力ポートの宣言。
#[derive(Debug)]
pub struct OutputDecl {
    /// 安定 key。
    pub key: String,
    /// 出力型（ちょうど 1 つ）。
    pub ty: SynTypeId,
}

/// declare の結果。本体ホストはこの key/型/flags を見て接続を解決する。
#[derive(Debug)]
pub struct NodeDecl {
    /// 入力ポート（宣言順）。
    pub inputs: Vec<InputDecl>,
    /// 出力ポート（宣言順）。
    pub outputs: Vec<OutputDecl>,
}

impl NodeDecl {
    /// key から入力インデックスを引く。
    pub fn input_index(&self, key: &str) -> Option<u32> {
        self.inputs.iter().position(|p| p.key == key).map(|i| i as u32)
    }
    /// key から出力インデックスを引く。
    pub fn output_index(&self, key: &str) -> Option<u32> {
        self.outputs.iter().position(|p| p.key == key).map(|i| i as u32)
    }
    /// 指定入力ポートが multi-input（fan-in）か。
    pub fn is_multi(&self, input_index: u32) -> bool {
        self.inputs
            .get(input_index as usize)
            .is_some_and(|p| p.flags & SYN_PORT_MULTI != 0)
    }
}

/// declare 中にビルダへ積まれる内部状態（既定値の OwnedValue も保持）。
struct DeclScope {
    inputs: Vec<InputDecl>,
    outputs: Vec<OutputDecl>,
    defaults: Vec<(usize, OwnedValue)>, // (input_index, default)
}

/* ======================================================================= */
/*  評価コンテキスト（negotiate / process のバックエンド）                 */
/* ======================================================================= */

/// negotiate が返す入力要求。frame は v1（単一フレーム）では無視する。
#[derive(Debug, Clone, Copy)]
pub struct Request {
    /// 要求する入力ポート（宣言順）。
    pub input_index: u32,
    /// multi-input 上のリンク番号（単一入力なら 0）。
    pub link_index: u32,
}

/// process に与える入力束。本体ホストが上流の評価結果をリンク順に詰める。
/// link_count は詰めたリンク数から導出される（multi-input 未接続は 0）。
pub struct InputBindings {
    links: Vec<Vec<OwnedValue>>,
    /// declare で既定値を持つ入力ポートの既定値（接続なしポートで配送）。
    defaults: Vec<Option<OwnedValue>>,
}

impl InputBindings {
    /// 入力ポート数を指定して空の束を作る。
    pub fn new(n_inputs: usize) -> Self {
        Self {
            links: (0..n_inputs).map(|_| Vec::new()).collect(),
            defaults: (0..n_inputs).map(|_| None).collect(),
        }
    }
    /// 入力ポートにリンク値を 1 本追加する（追加順がリンク順）。
    pub fn push_link(&mut self, input_index: usize, value: OwnedValue) {
        self.links[input_index].push(value);
    }
    fn link_count(&self, i: usize) -> u32 {
        self.links.get(i).map_or(0, |v| v.len() as u32)
    }
    fn get(&self, i: usize, l: usize) -> Option<&OwnedValue> {
        self.links.get(i).and_then(|v| v.get(l))
    }
    fn default(&self, i: usize) -> Option<&OwnedValue> {
        self.defaults.get(i).and_then(|o| o.as_ref())
    }
}

const MODE_NEGOTIATE: u8 = 0;
const MODE_PROCESS: u8 = 1;

struct EvalScope {
    #[allow(dead_code)]
    mode: u8,
    link_counts: Vec<u32>,
    requests: Vec<Request>,
    inputs: *const InputBindings, // process 中のみ非 NULL
    outputs: Vec<Option<OwnedValue>>,
    scratch: Vec<Box<[u8]>>,
}

/* ======================================================================= */
/*  decl スイート                                                          */
/* ======================================================================= */

extern "C" fn decl_output(
    b: *mut SynDeclBuilder,
    key: *const c_char,
    _label: *const c_char,
    ty: SynTypeId,
) -> SynStatus {
    guard_status(|| {
        if b.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(b as *mut DeclScope) };
        s.outputs.push(OutputDecl {
            key: cstr_to_string(key),
            ty,
        });
        SYN_OK
    })
}

extern "C" fn decl_input(
    b: *mut SynDeclBuilder,
    key: *const c_char,
    _label: *const c_char,
    types: *const SynTypeId,
    n_types: usize,
    flags: u32,
) -> SynStatus {
    guard_status(|| {
        if b.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(b as *mut DeclScope) };
        let ts = if types.is_null() || n_types == 0 {
            Vec::new()
        } else {
            unsafe { core::slice::from_raw_parts(types, n_types) }.to_vec()
        };
        s.inputs.push(InputDecl {
            key: cstr_to_string(key),
            types: ts,
            flags,
            has_default: false,
        });
        SYN_OK
    })
}

extern "C" fn decl_connected_type(
    _b: *mut SynDeclBuilder,
    _input_key: *const c_char,
    _link_index: u32,
) -> SynTypeId {
    SYN_TYPE_ANY // 方式a: v1 は常に ANY
}

extern "C" fn decl_input_default(
    b: *mut SynDeclBuilder,
    key: *const c_char,
    value: SynValue,
) -> SynStatus {
    guard_status(|| {
        if b.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(b as *mut DeclScope) };
        let k = cstr_to_string(key);
        if let Some(idx) = s.inputs.iter().position(|p| p.key == k) {
            s.inputs[idx].has_default = true;
            s.defaults.push((idx, unsafe { OwnedValue::from_value(&value) }));
            SYN_OK
        } else {
            SYN_ERR_BAD_ARG
        }
    })
}

static DECL_SUITE: SynDeclSuite = SynDeclSuite {
    output: Some(decl_output),
    input: Some(decl_input),
    connected_type: Some(decl_connected_type),
    input_default: Some(decl_input_default),
};

/* ======================================================================= */
/*  eval スイート                                                          */
/* ======================================================================= */

extern "C" fn ev_request(ctx: *mut SynEvalCtx, req: *const SynRequest) -> SynStatus {
    guard_status(|| {
        if ctx.is_null() || req.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(ctx as *mut EvalScope) };
        let r = unsafe { &*req };
        s.requests.push(Request {
            input_index: r.input_index,
            link_index: r.link_index,
        });
        SYN_OK
    })
}

extern "C" fn ev_link_count(ctx: *mut SynEvalCtx, input_index: u32) -> u32 {
    guard_or(0, || {
        if ctx.is_null() {
            return 0;
        }
        let s = unsafe { &*(ctx as *mut EvalScope) };
        s.link_counts.get(input_index as usize).copied().unwrap_or(0)
    })
}

extern "C" fn ev_get_input(ctx: *mut SynEvalCtx, input_index: u32, link_index: u32) -> SynValue {
    // 空値 sentinel（ADR-018: type_id==0）。SynValue は非 Copy なので都度組み立てる。
    fn empty() -> SynValue {
        SynValue {
            type_id: SYN_URID_INVALID,
            ptr: null_mut(),
            size: 0,
        }
    }
    guard_or(empty(), || {
        if ctx.is_null() {
            return empty();
        }
        let s = unsafe { &*(ctx as *mut EvalScope) };
        let ii = input_index as usize;
        if !s.inputs.is_null() {
            let inp = unsafe { &*s.inputs };
            if let Some(v) = inp.get(ii, link_index as usize) {
                return unsafe { v.to_value() };
            }
            // 接続なし: 既定値があれば配送、なければ空。
            if let Some(d) = inp.default(ii) {
                return unsafe { d.to_value() };
            }
        }
        empty()
    })
}

extern "C" fn ev_alloc(ctx: *mut SynEvalCtx, size: usize) -> *mut c_void {
    guard_or(null_mut(), || {
        if ctx.is_null() {
            return null_mut();
        }
        let s = unsafe { &mut *(ctx as *mut EvalScope) };
        let mut b = vec![0u8; size].into_boxed_slice();
        let p = b.as_mut_ptr();
        s.scratch.push(b);
        p as *mut c_void
    })
}

extern "C" fn ev_set_output(ctx: *mut SynEvalCtx, output_index: u32, value: SynValue) -> SynStatus {
    guard_status(|| {
        if ctx.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(ctx as *mut EvalScope) };
        let oi = output_index as usize;
        if oi < s.outputs.len() {
            s.outputs[oi] = Some(unsafe { OwnedValue::from_value(&value) });
            SYN_OK
        } else {
            SYN_ERR_BAD_ARG
        }
    })
}

extern "C" fn ev_passthrough(
    ctx: *mut SynEvalCtx,
    output_index: u32,
    input_value: SynValue,
) -> SynStatus {
    ev_set_output(ctx, output_index, input_value)
}

static EVAL_SUITE: SynEvalSuite = SynEvalSuite {
    request: Some(ev_request),
    link_count: Some(ev_link_count),
    get_input: Some(ev_get_input),
    alloc: Some(ev_alloc),
    set_output: Some(ev_set_output),
    passthrough: Some(ev_passthrough),
};

/* ======================================================================= */
/*  セッション（プロセスグローバル: ADR-023）                              */
/* ======================================================================= */

/// モジュール識別子。ロードごとに採番し、登録（vtable/node desc）の出所を追跡する。
/// アンロード時に同 ID の登録をグローバルから除去（purge）して dangling を防ぐ。
type ModuleId = u64;

struct SessionInner {
    uri_to_id: HashMap<String, u32>,
    id_to_uri: HashMap<u32, CString>,
    next_id: u32,
    next_module_id: ModuleId,
    /// 現在 on_load 実行中のモジュール ID（reg_type/register_node が出所を付与するのに使う）。
    /// ロードは [`LoadedModule::load`] が LOAD_LOCK で直列化するので競合しない。
    current_loading: Option<ModuleId>,
    /// 型 ID → (登録元モジュール, vtable ポインタ)。ポインタはモジュールイメージ内 static。
    vtables: HashMap<u32, (ModuleId, SendPtr<SynTypeVTable>)>,
    /// (登録元モジュール, node desc ポインタ)。desc はモジュールイメージ内 static。
    nodes: Vec<(ModuleId, SendPtr<SynNodeDesc>)>,
}

static SESSION: OnceLock<Mutex<SessionInner>> = OnceLock::new();

fn session_inner() -> &'static Mutex<SessionInner> {
    SESSION.get_or_init(|| {
        Mutex::new(SessionInner {
            uri_to_id: HashMap::new(),
            id_to_uri: HashMap::new(),
            next_id: 2, // 0=invalid, 1=ANY
            next_module_id: 1, // 0 は「ロード文脈外の登録」用に予約（purge 対象外）
            current_loading: None,
            vtables: HashMap::new(),
            nodes: Vec::new(),
        })
    })
}

/// セッションロックを取得する。poison は回収して続行する（intern マップ中心の状態で、
/// 一部のコールバックが panic しても半端な状態が壊滅的にならないため、可用性を優先）。
fn lock_session() -> MutexGuard<'static, SessionInner> {
    session_inner().lock().unwrap_or_else(|e| e.into_inner())
}

/// プロセスグローバル状態（URID intern / 型 vtable / ノード登録）への安全な窓口。
/// 1プロセス=1ホスト=1セッション（ADR-023）なので状態はグローバルが正。
pub struct Session;

impl Session {
    /// URI をセッション安定な URID に intern する。
    pub fn urid(uri: &CStr) -> SynUrid {
        urid_map(uri.as_ptr())
    }
    /// 型 ID から登録済み vtable を引く（未登録/アンロード済みなら `None`）。
    ///
    /// 返るポインタは**登録元モジュールの生存中のみ有効**（モジュールイメージ内 static を指す）。
    /// 当該モジュールがアンロードされると、対応エントリは [`LoadedModule::drop`] が除去するため
    /// 以後 `None` を返す（dangling ポインタは返さない）。
    pub fn type_vtable(id: SynTypeId) -> Option<*const SynTypeVTable> {
        lock_session().vtables.get(&id).map(|&(_, p)| p.0)
    }
}

extern "C" fn urid_map(uri: *const c_char) -> SynUrid {
    guard_or(SYN_URID_INVALID, || {
        if uri.is_null() {
            return SYN_URID_INVALID;
        }
        let s = cstr_to_string(uri);
        let mut st = lock_session();
        if let Some(&id) = st.uri_to_id.get(&s) {
            return id;
        }
        let id = st.next_id;
        st.next_id += 1;
        st.uri_to_id.insert(s.clone(), id);
        // s は cstr 由来で内部 NUL を含まないため CString 化は失敗しない。
        if let Ok(c) = CString::new(s) {
            st.id_to_uri.insert(id, c);
        }
        id
    })
}

extern "C" fn urid_unmap(id: SynUrid) -> *const c_char {
    guard_or(core::ptr::null(), || {
        lock_session()
            .id_to_uri
            .get(&id)
            .map_or(core::ptr::null(), |c| c.as_ptr())
    })
}

static URID_SUITE: SynUridSuite = SynUridSuite {
    map: Some(urid_map),
    unmap: Some(urid_unmap),
};

extern "C" fn reg_type(uri: *const c_char, vt: *const SynTypeVTable) -> SynStatus {
    guard_status(|| {
        if uri.is_null() || vt.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let id = urid_map(uri); // 先に完全 return するので Mutex 二重ロックにならない
        let mut st = lock_session();
        let mid = st.current_loading.unwrap_or(0);
        st.vtables.insert(id, (mid, SendPtr(vt)));
        SYN_OK
    })
}
extern "C" fn reg_lookup(t: SynTypeId) -> *const SynTypeVTable {
    guard_or(core::ptr::null(), || {
        lock_session()
            .vtables
            .get(&t)
            .map_or(core::ptr::null(), |&(_, p)| p.0)
    })
}
extern "C" fn reg_type_of(uri: *const c_char) -> SynTypeId {
    urid_map(uri)
}

static TYPE_SUITE: SynTypeRegistrySuite = SynTypeRegistrySuite {
    register_type: Some(reg_type),
    lookup: Some(reg_lookup),
    type_of: Some(reg_type_of),
};

/* ======================================================================= */
/*  ホストコールバック                                                     */
/* ======================================================================= */

extern "C" fn h_fetch_suite(_h: *mut SynHostStruct, id: *const c_char) -> *const c_void {
    guard_or(core::ptr::null(), || {
        if id.is_null() {
            return core::ptr::null();
        }
        let s = cstr_to_string(id);
        if s == SYN_DECL_SUITE {
            &DECL_SUITE as *const _ as *const c_void
        } else if s == SYN_EVAL_SUITE {
            &EVAL_SUITE as *const _ as *const c_void
        } else if s == SYN_URID_SUITE {
            &URID_SUITE as *const _ as *const c_void
        } else if s == SYN_TYPE_REGISTRY_SUITE {
            &TYPE_SUITE as *const _ as *const c_void
        } else {
            core::ptr::null()
        }
    })
}

extern "C" fn h_register_node(_h: *mut SynHostStruct, desc: *const SynNodeDesc) -> SynStatus {
    guard_status(|| {
        if desc.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let mut st = lock_session();
        let mid = st.current_loading.unwrap_or(0);
        st.nodes.push((mid, SendPtr(desc)));
        SYN_OK
    })
}

extern "C" fn h_mark_dirty(_h: *mut SynHostStruct, _node: *mut SynNode) {
    // 本層はキャッシュを持たない。本体ホストが dirty 伝播を実装する受け口のみ。
}

extern "C" fn h_log(_h: *mut SynHostStruct, level: c_int, msg: *const c_char) {
    guard_unit(|| {
        eprintln!("[plugin log L{level}] {}", cstr_to_string(msg));
    });
}

/* ======================================================================= */
/*  モジュール                                                             */
/* ======================================================================= */

/// ロード元モジュールの全登録（node desc / vtable）をグローバルから除去する。
/// dlclose で desc/vtable のアドレス（モジュールイメージ内 static）が無効化されるため、
/// アンロード時や on_load 失敗時にこれを呼んで stale ポインタを残さない。
fn purge_module(id: ModuleId) {
    let mut st = lock_session();
    st.nodes.retain(|&(mid, _)| mid != id);
    st.vtables.retain(|_, &mut (mid, _)| mid != id);
}

/// ロード済みプラグインモジュール。Drop で `on_unload` を呼ぶ。
/// ノードインスタンスはこのモジュールより長生きできない（ライフタイムで強制）。
pub struct LoadedModule {
    _lib: Library,
    _host: Box<SynHostStruct>,
    module: *const SynModule,
    /// このモジュールの ID（登録の出所追跡・アンロード時 purge に使う）。
    id: ModuleId,
    /// このモジュールが on_load で登録した node desc（モジュールイメージ内 static を指す）。
    descs: Vec<*const SynNodeDesc>,
}

/// ノード種別ハンドル（モジュールに紐づく）。
#[derive(Clone, Copy)]
pub struct NodeType<'m> {
    desc: *const SynNodeDesc,
    _m: PhantomData<&'m LoadedModule>,
}

impl<'m> NodeType<'m> {
    /// ノード URI。
    pub fn uri(&self) -> String {
        cstr_to_string(unsafe { &*self.desc }.node_uri)
    }
    /// 表示名。
    pub fn display_name(&self) -> String {
        cstr_to_string(unsafe { &*self.desc }.display_name)
    }
}

impl LoadedModule {
    /// DLL をロードし、`synapse_module` を取得、ABI を検査し、`on_load` まで実行する。
    ///
    /// # 信頼境界
    /// `Library::new` の時点で DLL のグローバルコンストラクタ等、任意コードが走り得る。これは
    /// dlopen の本質で防げない——プラグインは信頼できる供給元のみロードする前提（コード署名等は
    /// 上位の責務）。
    ///
    /// ノード登録はプロセスグローバル（ADR-023）に積まれる。このモジュールに [`ModuleId`] を
    /// 採番し `current_loading` にセットすることで、on_load 中の register_node/reg_type 登録に
    /// 出所を付与し、「このモジュール分」を ID で正確に切り出す（before/after 差分は不要）。ロード
    /// 全体を LOAD_LOCK で直列化する（実運用では起動時に逐次ロードするので制約にならない）。
    pub fn load(path: &Path) -> Result<Self> {
        static LOAD_LOCK: Mutex<()> = Mutex::new(());
        let _load_guard = LOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let lib = unsafe { Library::new(path) }.map_err(|e| Error::Load(e.to_string()))?;
        let entry: Symbol<unsafe extern "C" fn() -> *const SynModule> =
            unsafe { lib.get(b"synapse_module\0") }.map_err(|_| Error::MissingEntry)?;
        let module = unsafe { entry() };
        if module.is_null() {
            return Err(Error::MissingEntry);
        }
        let m = unsafe { &*module };
        if m.abi_version != SYN_ABI_VERSION {
            return Err(Error::AbiVersion {
                found: m.abi_version,
                expected: SYN_ABI_VERSION,
            });
        }

        let mut host = Box::new(SynHostStruct {
            host_ctx: null_mut(),
            fetch_suite: Some(h_fetch_suite),
            register_node: Some(h_register_node),
            mark_dirty: Some(h_mark_dirty),
            log: Some(h_log),
        });

        // ID を採番し、on_load 中の登録に出所を付与する。
        let id = {
            let mut st = lock_session();
            let id = st.next_module_id;
            st.next_module_id += 1;
            st.current_loading = Some(id);
            id
        };
        let on_load = m.on_load.ok_or(Error::NullCallback("on_load"))?;
        let status = unsafe { on_load(host.as_mut() as *mut SynHostStruct) };
        // 成否によらず current_loading を必ずクリアし、この ID の登録ノードを回収する。
        let descs: Vec<*const SynNodeDesc> = {
            let mut st = lock_session();
            st.current_loading = None;
            st.nodes
                .iter()
                .filter(|&&(mid, _)| mid == id)
                .map(|&(_, p)| p.0)
                .collect()
        };
        // on_load 失敗時は部分登録の残骸を purge してから返す（dlclose は lib drop で起こる）。
        if let Err(e) = check(status) {
            purge_module(id);
            return Err(e);
        }

        Ok(LoadedModule {
            _lib: lib,
            _host: host,
            module,
            id,
            descs,
        })
    }

    /// モジュール URI（名前空間）。
    pub fn module_uri(&self) -> String {
        cstr_to_string(unsafe { &*self.module }.module_uri)
    }

    /// このモジュールが登録した全ノード種別。
    pub fn node_types(&self) -> Vec<NodeType<'_>> {
        self.descs
            .iter()
            .map(|&desc| NodeType {
                desc,
                _m: PhantomData,
            })
            .collect()
    }

    /// URI でノード種別を引く。
    pub fn find_node(&self, uri: &str) -> Option<NodeType<'_>> {
        self.node_types().into_iter().find(|t| t.uri() == uri)
    }

    /// ノードインスタンスを生成する（`create`）。
    pub fn instantiate<'m>(&'m self, ty: NodeType<'m>) -> Result<NodeInstance<'m>> {
        let desc = unsafe { &*ty.desc };
        let create = desc.create.ok_or(Error::NullCallback("create"))?;
        let mut instance: *mut c_void = null_mut();
        check(unsafe { create(null_mut(), &mut instance) })?;
        Ok(NodeInstance {
            desc: ty.desc,
            instance,
            decl: None,
            declared_defaults: Vec::new(),
            _m: PhantomData,
        })
    }
}

impl Drop for LoadedModule {
    fn drop(&mut self) {
        let m = unsafe { &*self.module };
        if let Some(on_unload) = m.on_unload {
            unsafe { on_unload(self._host.as_mut() as *mut SynHostStruct) };
        }
        // dlclose（_lib の drop）より前に、このモジュール由来のグローバル登録を除去する。
        // これで以後の Session::type_vtable / 別モジュールロードが stale ポインタを掴まない。
        purge_module(self.id);
    }
}

/* ======================================================================= */
/*  ノードインスタンス（RAII）                                             */
/* ======================================================================= */

/// 1 ノードインスタンス。Drop で `destroy`。
/// declare/negotiate/process が `&mut self` を取るため、同一インスタンスへの
/// 呼び出しの非重複（ADR-019）が借用規則で保証される。
pub struct NodeInstance<'m> {
    desc: *const SynNodeDesc,
    instance: *mut c_void,
    decl: Option<NodeDecl>,
    /// declare で得た入力ポートごとの既定値（make_input_bindings で配る）。
    declared_defaults: Vec<Option<OwnedValue>>,
    _m: PhantomData<&'m LoadedModule>,
}

impl NodeInstance<'_> {
    fn desc(&self) -> &SynNodeDesc {
        unsafe { &*self.desc }
    }

    /// `declare` を実行し、宣言結果（key/型/flags/既定値の有無）を返す。結果は内部にも保持。
    /// 既定値の実体は process 時に既定値配送するため [`InputBindings`] に積む必要がある——
    /// [`NodeInstance::make_input_bindings`] が宣言済み既定値を埋めた束を返す。
    pub fn declare(&mut self) -> Result<&NodeDecl> {
        let declare = self.desc().declare.ok_or(Error::NullCallback("declare"))?;
        let mut scope = DeclScope {
            inputs: Vec::new(),
            outputs: Vec::new(),
            defaults: Vec::new(),
        };
        check(unsafe { declare(self.instance, (&mut scope as *mut DeclScope).cast()) })?;
        // 既定値を input ごとに格納（process の InputBindings 構築に使う）。
        let mut defaults: Vec<Option<OwnedValue>> = (0..scope.inputs.len()).map(|_| None).collect();
        for (idx, val) in scope.defaults {
            defaults[idx] = Some(val);
        }
        self.decl = Some(NodeDecl {
            inputs: scope.inputs,
            outputs: scope.outputs,
        });
        self.declared_defaults = defaults;
        Ok(self.decl.as_ref().unwrap())
    }

    /// 直近の declare 結果。
    pub fn decl(&self) -> Option<&NodeDecl> {
        self.decl.as_ref()
    }

    /// 宣言済み既定値を埋めた入力束を作る。本体ホストはここへ接続リンクを push して process に渡す。
    pub fn make_input_bindings(&self) -> Result<InputBindings> {
        let decl = self.decl.as_ref().ok_or(Error::NotDeclared)?;
        let mut b = InputBindings::new(decl.inputs.len());
        for (i, d) in self.declared_defaults.iter().enumerate() {
            if let Some(v) = d {
                b.defaults[i] = Some(OwnedValue {
                    type_id: v.type_id,
                    size: v.size,
                    bytes: v.bytes.clone(),
                });
            }
        }
        Ok(b)
    }

    /// `negotiate` を実行し、必要入力の一覧を返す。link_counts は接続トポロジ（本体ホストが知る）。
    pub fn negotiate(&mut self, link_counts: &[u32]) -> Result<Vec<Request>> {
        if self.decl.is_none() {
            return Err(Error::NotDeclared);
        }
        let negotiate = self
            .desc()
            .negotiate
            .ok_or(Error::NullCallback("negotiate"))?;
        let mut scope = EvalScope {
            mode: MODE_NEGOTIATE,
            link_counts: link_counts.to_vec(),
            requests: Vec::new(),
            inputs: core::ptr::null(),
            outputs: Vec::new(),
            scratch: Vec::new(),
        };
        check(unsafe { negotiate(self.instance, (&mut scope as *mut EvalScope).cast()) })?;
        Ok(scope.requests)
    }

    /// `process` を実行する。入力束 `inputs` の各ポートのリンク数が link_count として渡る。
    /// 出力スロット（宣言した出力数）を返す。
    pub fn process(&mut self, inputs: &InputBindings) -> Result<Vec<Option<OwnedValue>>> {
        let decl = self.decl.as_ref().ok_or(Error::NotDeclared)?;
        let n_out = decl.outputs.len();
        let link_counts: Vec<u32> = (0..decl.inputs.len())
            .map(|i| inputs.link_count(i))
            .collect();
        let process = unsafe { &*self.desc }
            .process
            .ok_or(Error::NullCallback("process"))?;
        let mut scope = EvalScope {
            mode: MODE_PROCESS,
            link_counts,
            requests: Vec::new(),
            inputs: inputs as *const InputBindings,
            outputs: (0..n_out).map(|_| None).collect(),
            scratch: Vec::new(),
        };
        check(unsafe { process(self.instance, (&mut scope as *mut EvalScope).cast()) })?;
        Ok(scope.outputs)
    }

    /// 内部状態を保存（2 段サイズ問い合わせを内包）。状態が無ければ `None`。
    pub fn save_state(&mut self) -> Result<Option<Vec<u8>>> {
        let save = match self.desc().save_state {
            Some(f) => f,
            None => return Ok(None),
        };
        let mut written: usize = 0;
        check(unsafe { save(self.instance, null_mut(), 0, &mut written) })?;
        if written == 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; written];
        check(unsafe {
            save(
                self.instance,
                buf.as_mut_ptr() as *mut c_void,
                written,
                &mut written,
            )
        })?;
        buf.truncate(written);
        Ok(Some(buf))
    }

    /// 内部状態を復元。
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<()> {
        let load = match self.desc().load_state {
            Some(f) => f,
            None => return Ok(()),
        };
        check(unsafe { load(self.instance, bytes.as_ptr() as *const c_void, bytes.len()) })
    }
}

impl Drop for NodeInstance<'_> {
    fn drop(&mut self) {
        if let Some(destroy) = self.desc().destroy {
            unsafe { destroy(self.instance) };
        }
    }
}
