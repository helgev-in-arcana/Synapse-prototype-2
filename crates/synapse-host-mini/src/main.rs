//! 最小ホスト（raw / SDK ラッパー無し）。
//!
//! プラグイン DLL をロードし、固定グラフ
//!     const(3.0) ──▶ add.a
//!                    add.b = 既定値 4.0（未接続）
//! を組んで `add.out` を評価し、7.0 を得るまでを一周する。
//!
//! 検証対象: register_type / register_node / create / declare(出力・入力・既定値) /
//!           negotiate(request) / 非再帰 pull 評価 / get_input(値返し: 上流値・既定値) /
//!           set_output(値渡し) / SVO 入出力 / save_state・load_state 往復。
//!
//! 実行: ワークスペース全体をビルドしてから走らせる（プラグインは dlopen でロードされ、
//!   `cargo run -p synapse-host-mini` 単独では cdylib が再ビルドされないため）。
//!     cargo build && cargo run -p synapse-host-mini
//!
//! ⚠ このクレートは**編集凍結**。ラッパー（synapse-host-abi / synapse-sdk）開発前の最小動作
//! 確認として書かれ、現在は相互運用テスト（SDK プラグイン×最小ホスト、最小プラグイン×ラッパー
//! ホスト 等）の対向実装として使う。`evaluate` は `*mut Graph` を介した関数再帰で、各 FFI 呼び
//! 出しから戻ってから次を呼ぶ前提（参照を呼び出しまたぎで保持しない）でのみ健全＝**意図的に
//! fragile**。ノードを跨ぐ借用を 1 つ足すと Tree/Stacked Borrows を破る。「綺麗にしよう」として
//! 触らないこと。堅牢な評価器は本体ホスト（FFI/unsafe 無しの上位クレート）側の責務。
//! dlopen 経由のため Miri では検証不可（スイート単体の Miri 検証は host-abi 側で行う）。

use core::ffi::{c_char, c_int, c_void};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::ptr::null_mut;
use std::sync::{Mutex, OnceLock};

use libloading::{Library, Symbol};
use synapse_abi::*;

/* ======================================================================= */
/*  ホストグローバル状態                                                   */
/*  urid/type-registry スイートは ctx 引数を持たないため、グローバル必須。  */
/* ======================================================================= */

struct HostState {
    uri_to_id: HashMap<String, u32>,
    id_to_uri: HashMap<u32, CString>,
    next_id: u32,
    vtables: HashMap<u32, usize>, // *const SynTypeVTable
    nodes: Vec<usize>,            // *const SynNodeDesc（モジュール寿命）
}

static HOST: OnceLock<Mutex<HostState>> = OnceLock::new();

fn hs() -> &'static Mutex<HostState> {
    HOST.get_or_init(|| {
        Mutex::new(HostState {
            uri_to_id: HashMap::new(),
            id_to_uri: HashMap::new(),
            next_id: 2, // 0=invalid, 1=ANY
            vtables: HashMap::new(),
            nodes: Vec::new(),
        })
    })
}

fn cstr(p: *const c_char) -> String {
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/* ======================================================================= */
/*  値の保持・受け渡し                                                     */
/* ======================================================================= */

/// ホストが所有する値。バイト列は size>=実バイト数。SVO（≤ptr幅）値も buf に詰める。
struct OwnedVal {
    type_id: u32,
    size: usize,
    buf: Vec<u8>,
}

/// プラグインが渡した SynValue をホスト所有にコピーする。
unsafe fn capture(v: &SynValue) -> OwnedVal {
    let sz = v.size;
    let mut buf = vec![0u8; sz.max(8)];
    if sz <= core::mem::size_of::<*mut c_void>() {
        core::ptr::copy_nonoverlapping(
            (&v.ptr as *const *mut c_void) as *const u8,
            buf.as_mut_ptr(),
            sz,
        );
    } else {
        core::ptr::copy_nonoverlapping(v.ptr as *const u8, buf.as_mut_ptr(), sz);
    }
    OwnedVal {
        type_id: v.type_id,
        size: sz,
        buf,
    }
}

/// ホスト所有値を SynValue として値で組み立てる。≤ptr幅 は SVO（ptr フィールドにインライン）。
/// 大型は v.buf を指す（ホスト所有・呼び出し中有効）。返り値は値で渡す。
unsafe fn present(v: &OwnedVal) -> SynValue {
    if v.size <= core::mem::size_of::<*mut c_void>() {
        let mut bits: usize = 0;
        core::ptr::copy_nonoverlapping(v.buf.as_ptr(), (&mut bits as *mut usize) as *mut u8, v.size);
        SynValue {
            type_id: v.type_id,
            ptr: bits as *mut c_void,
            size: v.size,
        }
    } else {
        SynValue {
            type_id: v.type_id,
            ptr: v.buf.as_ptr() as *mut c_void,
            size: v.size,
        }
    }
}

fn empty_value() -> SynValue {
    SynValue {
        type_id: SYN_URID_INVALID,
        ptr: null_mut(),
        size: 0,
    }
}

fn read_owned_f32(o: &OwnedVal) -> f32 {
    f32::from_ne_bytes([o.buf[0], o.buf[1], o.buf[2], o.buf[3]])
}

/* ======================================================================= */
/*  宣言ビルダ（declare のバックエンド）                                   */
/* ======================================================================= */

struct InDecl {
    key: String,
    #[allow(dead_code)]
    types: Vec<u32>,
    #[allow(dead_code)]
    flags: u32,
    default: Option<OwnedVal>,
}
struct OutDecl {
    #[allow(dead_code)]
    key: String,
    #[allow(dead_code)]
    ty: u32,
}

struct HostDeclBuilder {
    inputs: Vec<InDecl>,
    outputs: Vec<OutDecl>,
}

extern "C" fn decl_output(
    b: *mut SynDeclBuilder,
    key: *const c_char,
    _label: *const c_char,
    ty: SynTypeId,
) -> SynStatus {
    let bb = unsafe { &mut *(b as *mut HostDeclBuilder) };
    bb.outputs.push(OutDecl {
        key: cstr(key),
        ty,
    });
    SYN_OK
}

extern "C" fn decl_input(
    b: *mut SynDeclBuilder,
    key: *const c_char,
    _label: *const c_char,
    types: *const SynTypeId,
    n_types: usize,
    flags: u32,
) -> SynStatus {
    let bb = unsafe { &mut *(b as *mut HostDeclBuilder) };
    let ts = unsafe { core::slice::from_raw_parts(types, n_types) }.to_vec();
    bb.inputs.push(InDecl {
        key: cstr(key),
        types: ts,
        flags,
        default: None,
    });
    SYN_OK
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
    let bb = unsafe { &mut *(b as *mut HostDeclBuilder) };
    let k = cstr(key);
    if let Some(p) = bb.inputs.iter_mut().find(|i| i.key == k) {
        p.default = Some(unsafe { capture(&value) });
        SYN_OK
    } else {
        SYN_ERR_BAD_ARG
    }
}

static DECL_SUITE: SynDeclSuite = SynDeclSuite {
    output: Some(decl_output),
    input: Some(decl_input),
    connected_type: Some(decl_connected_type),
    input_default: Some(decl_input_default),
};

/* ======================================================================= */
/*  評価コンテキスト（negotiate / process のバックエンド）                 */
/* ======================================================================= */

/// alloc() が返す型アラインメント準拠バッファ（process 中のみ有効）。
struct AlignedBuf {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}

impl AlignedBuf {
    fn new(size: usize, align: usize) -> Option<Self> {
        let layout = std::alloc::Layout::from_size_align(size, align).ok()?;
        if layout.size() == 0 {
            return None;
        }
        // 安全性: layout.size() > 0 を確認済み。
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            return None;
        }
        Some(Self { ptr, layout })
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // 安全性: ptr は同じ layout で alloc_zeroed した非 NULL ポインタ。
        unsafe { std::alloc::dealloc(self.ptr, self.layout) };
    }
}

struct HostEvalCtx {
    graph: *mut Graph,
    node: usize,
    requests: Vec<(u32, u32)>, // (input_index, link_index)
    scratch: Vec<AlignedBuf>,  // alloc() が返す大型出力用バッファ（process 中のみ有効）
}

extern "C" fn ev_request(ctx: *mut SynEvalCtx, req: *const SynRequest) -> SynStatus {
    let c = unsafe { &mut *(ctx as *mut HostEvalCtx) };
    let r = unsafe { &*req };
    c.requests.push((r.input_index, r.link_index));
    SYN_OK
}

extern "C" fn ev_link_count(ctx: *mut SynEvalCtx, input_index: u32) -> u32 {
    let c = unsafe { &*(ctx as *mut HostEvalCtx) };
    let g = unsafe { &*c.graph };
    g.0[c.node].links[input_index as usize].len() as u32
}

extern "C" fn ev_get_input(
    ctx: *mut SynEvalCtx,
    input_index: u32,
    link_index: u32,
) -> SynValue {
    let c = unsafe { &*(ctx as *mut HostEvalCtx) };
    let g = unsafe { &*c.graph };
    let node = &g.0[c.node];
    let ii = input_index as usize;

    // 接続があれば上流の出力値、無ければ既定値、どちらも無ければ空。
    let owned: *const OwnedVal = if let Some(&(sn, so)) = node.links[ii].get(link_index as usize) {
        g.0[sn].out_cache[so].as_ref().unwrap() as *const OwnedVal
    } else if let Some(d) = node.in_decls[ii].default.as_ref() {
        d as *const OwnedVal
    } else {
        core::ptr::null()
    };

    if owned.is_null() {
        empty_value()
    } else {
        unsafe { present(&*owned) }
    }
}

/// 大型出力用バッファをホストが確保（process 中のみ有効）。
/// アラインメントは型の静的属性から引く（ADR-029）。未登録型は確保しない。
extern "C" fn ev_alloc(ctx: *mut SynEvalCtx, size: usize, t: SynTypeId) -> *mut c_void {
    let vt = reg_lookup(t);
    if vt.is_null() {
        return null_mut();
    }
    let align = unsafe { (*vt).align };
    let c = unsafe { &mut *(ctx as *mut HostEvalCtx) };
    match AlignedBuf::new(size, align) {
        Some(b) => {
            let p = b.ptr as *mut c_void;
            c.scratch.push(b);
            p
        }
        None => null_mut(),
    }
}

/// プラグインが値渡しした出力値をホスト所有にコピーして out_cache に格納。
extern "C" fn ev_set_output(
    ctx: *mut SynEvalCtx,
    output_index: u32,
    value: SynValue,
) -> SynStatus {
    let c = unsafe { &mut *(ctx as *mut HostEvalCtx) };
    let g = unsafe { &mut *c.graph };
    g.0[c.node].out_cache[output_index as usize] = Some(unsafe { capture(&value) });
    SYN_OK
}

extern "C" fn ev_passthrough(
    ctx: *mut SynEvalCtx,
    output_index: u32,
    input_value: SynValue,
) -> SynStatus {
    let c = unsafe { &mut *(ctx as *mut HostEvalCtx) };
    let g = unsafe { &mut *c.graph };
    g.0[c.node].out_cache[output_index as usize] = Some(unsafe { capture(&input_value) });
    SYN_OK
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
/*  URID / 型レジストリスイート                                            */
/* ======================================================================= */

extern "C" fn urid_map(uri: *const c_char) -> SynUrid {
    let s = cstr(uri);
    let mut st = hs().lock().unwrap();
    if let Some(&id) = st.uri_to_id.get(&s) {
        return id;
    }
    let id = st.next_id;
    st.next_id += 1;
    st.uri_to_id.insert(s.clone(), id);
    st.id_to_uri.insert(id, CString::new(s).unwrap());
    id
}

extern "C" fn urid_unmap(id: SynUrid) -> *const c_char {
    let st = hs().lock().unwrap();
    match st.id_to_uri.get(&id) {
        Some(c) => c.as_ptr(),
        None => core::ptr::null(),
    }
}

static URID_SUITE: SynUridSuite = SynUridSuite {
    map: Some(urid_map),
    unmap: Some(urid_unmap),
};

extern "C" fn reg_type(uri: *const c_char, vt: *const SynTypeVTable) -> SynStatus {
    // align は 2 の冪であること（ADR-029。alloc がこの属性を信頼して確保する）。
    let align = unsafe { (*vt).align };
    if align == 0 || !align.is_power_of_two() {
        return SYN_ERR_BAD_ARG;
    }
    let id = urid_map(uri); // 先に完全に return するので Mutex 二重ロックにならない
    hs().lock().unwrap().vtables.insert(id, vt as usize);
    SYN_OK
}
extern "C" fn reg_lookup(t: SynTypeId) -> *const SynTypeVTable {
    let st = hs().lock().unwrap();
    match st.vtables.get(&t) {
        Some(&p) => p as *const SynTypeVTable,
        None => core::ptr::null(),
    }
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
/*  ホスト操作（SynHostStruct のコールバック）                             */
/* ======================================================================= */

extern "C" fn h_fetch_suite(_h: *mut SynHostStruct, id: *const c_char) -> *const c_void {
    let s = cstr(id);
    let p: *const c_void = if s == SYN_DECL_SUITE {
        &DECL_SUITE as *const _ as *const c_void
    } else if s == SYN_EVAL_SUITE {
        &EVAL_SUITE as *const _ as *const c_void
    } else if s == SYN_URID_SUITE {
        &URID_SUITE as *const _ as *const c_void
    } else if s == SYN_TYPE_REGISTRY_SUITE {
        &TYPE_SUITE as *const _ as *const c_void
    } else {
        core::ptr::null()
    };
    p
}

extern "C" fn h_register_node(_h: *mut SynHostStruct, desc: *const SynNodeDesc) -> SynStatus {
    hs().lock().unwrap().nodes.push(desc as usize);
    SYN_OK
}

extern "C" fn h_mark_dirty(_h: *mut SynHostStruct, _node: *mut SynNode) {}

extern "C" fn h_log(_h: *mut SynHostStruct, level: c_int, msg: *const c_char) {
    println!("  [plugin/log L{}] {}", level, cstr(msg));
}

/* ======================================================================= */
/*  グラフ                                                                 */
/* ======================================================================= */

struct GNode {
    desc: usize, // *const SynNodeDesc
    instance: *mut c_void,
    in_decls: Vec<InDecl>,
    #[allow(dead_code)]
    out_decls: Vec<OutDecl>,
    links: Vec<Vec<(usize, usize)>>, // 入力ポートごと: (src_node, src_output)
    out_cache: Vec<Option<OwnedVal>>,
    evaluated: bool,
}

struct Graph(Vec<GNode>);

fn make_gnode(desc: usize) -> GNode {
    let d = unsafe { &*(desc as *const SynNodeDesc) };
    let mut inst: *mut c_void = null_mut();
    let st = unsafe { (d.create.unwrap())(null_mut(), &mut inst) };
    assert_eq!(st, SYN_OK, "create 失敗");
    GNode {
        desc,
        instance: inst,
        in_decls: Vec::new(),
        out_decls: Vec::new(),
        links: Vec::new(),
        out_cache: Vec::new(),
        evaluated: false,
    }
}

fn declare_node(g: &mut Graph, idx: usize) {
    let d = unsafe { &*(g.0[idx].desc as *const SynNodeDesc) };
    let mut b = HostDeclBuilder {
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    let st =
        unsafe { (d.declare.unwrap())(g.0[idx].instance, (&mut b as *mut HostDeclBuilder).cast()) };
    assert_eq!(st, SYN_OK, "declare 失敗");
    let ni = b.inputs.len();
    let no = b.outputs.len();
    g.0[idx].in_decls = b.inputs;
    g.0[idx].out_decls = b.outputs;
    g.0[idx].links = (0..ni).map(|_| Vec::new()).collect();
    g.0[idx].out_cache = (0..no).map(|_| None).collect();
}

/// const ノードの内部パラメータを load_state で設定する。
fn load_const(g: &Graph, idx: usize, v: f32) {
    let d = unsafe { &*(g.0[idx].desc as *const SynNodeDesc) };
    let bytes = v.to_ne_bytes();
    let st =
        unsafe { (d.load_state.unwrap())(g.0[idx].instance, bytes.as_ptr() as *const c_void, 4) };
    assert_eq!(st, SYN_OK, "load_state 失敗");
}

/// 非再帰 pull 評価。FFI は再帰しない（negotiate(X) は戻ってから上流を評価する）。
/// Rust 側の関数再帰で DAG を辿るが、各プラグイン呼び出しは戻ってから次を呼ぶため
/// FFI スタックに同一インスタンスが重ならない（ADR-001/019 の不変条件を満たす）。
unsafe fn evaluate(g: *mut Graph, idx: usize) {
    if (&(*g).0)[idx].evaluated {
        return;
    }
    let desc = &*((&(*g).0)[idx].desc as *const SynNodeDesc);
    let inst = (&(*g).0)[idx].instance;

    // 1) negotiate: 要求列挙（データ無し・単発）
    let mut nctx = HostEvalCtx {
        graph: g,
        node: idx,
        requests: Vec::new(),
        scratch: Vec::new(),
    };
    (desc.negotiate.unwrap())(inst, (&mut nctx as *mut HostEvalCtx).cast());

    // 2) 要求された上流を充足（明示スタック相当の関数再帰）
    for k in 0..nctx.requests.len() {
        let (ii, li) = nctx.requests[k];
        let link = (&(*g).0)[idx].links[ii as usize][li as usize];
        evaluate(g, link.0);
    }

    // 3) process: 全充足後に 1 回
    let mut pctx = HostEvalCtx {
        graph: g,
        node: idx,
        requests: Vec::new(),
        scratch: Vec::new(),
    };
    (desc.process.unwrap())(inst, (&mut pctx as *mut HostEvalCtx).cast());

    (&mut (*g).0)[idx].evaluated = true;
}

/* ======================================================================= */
/*  main                                                                   */
/* ======================================================================= */

fn main() {
    // --- プラグイン DLL のパスを current_exe から導出 ---
    // 既定は raw 版 test_scalar_plugin。引数でプラグイン名を差し替え可能
    //（例: `... -- test_scalar_sdk` で SDK 版を raw ホストで駆動＝逆方向の漏れ確認）。
    let plugin_name = std::env::args().nth(1).unwrap_or_else(|| "test_scalar_plugin".to_string());
    let exe = std::env::current_exe().expect("current_exe");
    let dir = exe.parent().unwrap();
    let fname = format!(
        "{}{plugin_name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let path = dir.join(&fname);
    println!("loading plugin: {}", path.display());

    let lib = unsafe { Library::new(&path) }.expect("プラグインのロード失敗");
    let entry: Symbol<unsafe extern "C" fn() -> *const SynModule> =
        unsafe { lib.get(b"synapse_module\0") }.expect("synapse_module シンボルが無い");

    let module = unsafe { &*entry() };
    assert_eq!(
        module.abi_version, SYN_ABI_VERSION,
        "ABI バージョン不一致: plugin={} host={}",
        module.abi_version, SYN_ABI_VERSION
    );
    println!(
        "module: {} v{} (abi {})",
        cstr(module.module_uri),
        cstr(module.module_version),
        module.abi_version
    );

    // --- 2フェーズロード（ADR-027）: 型登録 → ノード登録 ---
    // モジュールは1つだが、フェーズ順（全型登録の後にノード登録）は本番ローダと同じ。
    let mut host = SynHostStruct {
        host_ctx: null_mut(),
        fetch_suite: Some(h_fetch_suite),
        register_node: Some(h_register_node),
        mark_dirty: Some(h_mark_dirty),
        log: Some(h_log),
    };
    if let Some(f) = module.on_register_types {
        let st = unsafe { f(&mut host as *mut SynHostStruct) };
        assert_eq!(st, SYN_OK, "on_register_types 失敗");
    }
    if let Some(f) = module.on_register_nodes {
        let st = unsafe { f(&mut host as *mut SynHostStruct) };
        assert_eq!(st, SYN_OK, "on_register_nodes 失敗");
    }

    // --- 登録済みノード記述子を引く ---
    let nodes = hs().lock().unwrap().nodes.clone();
    let mut const_desc = 0usize;
    let mut add_desc = 0usize;
    let mut subfold_desc = 0usize;
    for &p in &nodes {
        let d = unsafe { &*(p as *const SynNodeDesc) };
        match cstr(d.node_uri).as_str() {
            "synapse.test.const" => const_desc = p,
            "synapse.test.add" => add_desc = p,
            "synapse.test.subfold" => subfold_desc = p,
            other => println!("(未使用ノード登録: {})", other),
        }
    }
    assert!(
        const_desc != 0 && add_desc != 0 && subfold_desc != 0,
        "必要ノードが登録されていない"
    );
    println!("registered nodes: const + add + subfold");

    // --- グラフ構築: [0]=const, [1]=add ---
    let mut graph = Graph(Vec::new());
    graph.0.push(make_gnode(const_desc));
    graph.0.push(make_gnode(add_desc));
    declare_node(&mut graph, 0);
    declare_node(&mut graph, 1);

    // add.a <- const.out
    graph.0[1].links[0].push((0, 0));
    // add.b は未接続 → 既定値 4.0

    // --- const の内部パラメータを 3.0 に load_state ---
    load_const(&graph, 0, 3.0);

    // --- 評価 ---
    let gp = &mut graph as *mut Graph;
    unsafe { evaluate(gp, 1) };

    let result = read_owned_f32(graph.0[1].out_cache[0].as_ref().expect("add 出力なし"));
    println!("evaluate(add.out) = {} (期待 7.0 = const 3.0 + 既定 4.0)", result);
    assert_eq!(result, 7.0, "加算結果が誤り");

    // --- save_state 往復（const）---
    {
        let d = unsafe { &*(graph.0[0].desc as *const SynNodeDesc) };
        let mut written: usize = 0;
        unsafe { (d.save_state.unwrap())(graph.0[0].instance, null_mut(), 0, &mut written) };
        assert_eq!(written, 4, "save_state サイズ問い合わせが誤り");
        let mut buf = vec![0u8; written];
        unsafe {
            (d.save_state.unwrap())(
                graph.0[0].instance,
                buf.as_mut_ptr() as *mut c_void,
                written,
                &mut written,
            )
        };
        let saved = f32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
        println!("save_state(const) = {} (期待 3.0)", saved);
        assert_eq!(saved, 3.0, "save_state 内容が誤り");
    }

    // --- 後始末（グラフ1）---
    for i in 0..graph.0.len() {
        let d = unsafe { &*(graph.0[i].desc as *const SynNodeDesc) };
        unsafe { (d.destroy.unwrap())(graph.0[i].instance) };
    }

    // ===================================================================== //
    //  fan-in (multi-input) テスト                                          //
    //    const(10) ─┐                                                       //
    //    const(1)  ─┼─▶ subfold.in (1ポート3リンク)  out = 10-1-2 = 7       //
    //    const(2)  ─┘                                                       //
    //  link_count>1 / 同一ポートへの繰り返し get_input / 順序保持 を検証。   //
    //  subfold は順序依存（畳み込み減算）なので、リンク順が崩れると 7 にならない。
    // ===================================================================== //
    let mut g2 = Graph(Vec::new());
    g2.0.push(make_gnode(const_desc)); // [0] = 10.0
    g2.0.push(make_gnode(const_desc)); // [1] = 1.0
    g2.0.push(make_gnode(const_desc)); // [2] = 2.0
    g2.0.push(make_gnode(subfold_desc)); // [3]
    for i in 0..g2.0.len() {
        declare_node(&mut g2, i);
    }
    load_const(&g2, 0, 10.0);
    load_const(&g2, 1, 1.0);
    load_const(&g2, 2, 2.0);

    // subfold.in（port 0）へ 3 リンクをこの順で接続。
    g2.0[3].links[0].push((0, 0));
    g2.0[3].links[0].push((1, 0));
    g2.0[3].links[0].push((2, 0));

    let gp2 = &mut g2 as *mut Graph;
    unsafe { evaluate(gp2, 3) };

    let fanin = read_owned_f32(g2.0[3].out_cache[0].as_ref().expect("subfold 出力なし"));
    println!(
        "evaluate(subfold.out) = {} (期待 7.0 = 10 - 1 - 2, リンク順保持)",
        fanin
    );
    assert_eq!(fanin, 7.0, "fan-in 結果が誤り（順序崩れ or link_count 誤り）");

    for i in 0..g2.0.len() {
        let d = unsafe { &*(g2.0[i].desc as *const SynNodeDesc) };
        unsafe { (d.destroy.unwrap())(g2.0[i].instance) };
    }

    unsafe { (module.on_unload.unwrap())(&mut host as *mut SynHostStruct) };

    println!("\nOK: 全アサーション通過。declare→negotiate→process の一周 + fan-in を確認。");
}
