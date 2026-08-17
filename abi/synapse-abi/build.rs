// アンブレラヘッダ `include/synapse_abi.h` を書き出す。
//
// 型宣言は各層クレートの build.rs が生成する（このクレート自身は宣言を持たない）。
// ここが受け持つのは「3 本を取り込む口」と、設計概要・C 側の補助定義という散文部分だけ。

/// 取り込む層ヘッダ。依存順（core が先）に並べる。
const LAYERS: &[&str] = &[
    "synapse_abi_core.h",
    "synapse_abi_suite.h",
    "synapse_abi_ext.h",
];

const HEADER: &str = r#"/* ============================================================================
 *  自動生成ファイル — 手で編集しないこと。
 *  正本(SSoT)は Rust。1 層 = 1 クレート = 1 ヘッダで対応する:
 *    abi/synapse-abi-core/   -> synapse_abi_core.h   基底 ABI。版一致必須の凍結面。
 *    abi/synapse-abi-suite/  -> synapse_abi_suite.h  スイート。fetch_suite(id) 引き。
 *    abi/synapse-abi-ext/    -> synapse_abi_ext.h    拡張。get_extension(ext_id) 引き。
 *    abi/synapse-abi/        -> synapse_abi.h        このファイル(アンブレラ)。
 *  依存は suite/ext -> core の一方向（Cargo が強制する）。
 *  再生成は `cargo build`（各層の build.rs が cbindgen を回す）。
 * ----------------------------------------------------------------------------
 *  Synapse プラグイン C ABI  (draft v0.5)
 *
 *  ■ これは何か
 *    ノードベース映像合成/編集フレームワーク "Synapse" のプラグイン C ABI。
 *    UX は Blender の shader/geometry nodes の感触を目標にしつつ基盤は独自 ABI。
 *
 *  ■ 最上位の方針
 *    - 最小カーネル: 複雑さはホスト/SDK に集中。小さな契約面を保ち、プラグイン
 *      開発を AI/コミュニティに委譲できるようにする。
 *    - OFX 由来のパターン(単一エントリ/不透明ハンドル/純粋 C ABI/スイート版数)は
 *      採るが、画像エフェクト中心モデルや外部ガバナンスは継がない。
 *    - データ宣言と振る舞いを分離。ホストはデータ意味論に不可知。
 *
 *  ■ アーキテクチャの柱 (括弧内は理由)
 *    - Pull型・スタック駆動・非再帰評価器 (FFI 再帰を避ける: 再入/ホットリロード/
 *      キャンセル/並列スケジューリングが壊れるため)。末尾の評価ループ参照。
 *    - 2フェーズ: declare(データ無し交渉) + process(要求バッファのみ受領)。
 *    - 2種のプラグイン: 型パッケージ(register_type) と 処理プラグイン(register_node)。
 *    - ワイヤは {URID, ptr, size}。URID はセッション安定で IPC/永続化に安全。
 *      vtable はプロセスローカルで揮発するので lookup で引く。
 *    - 出力型は1つ固定(キャッシュキーが (node,params,frame,region) に縮む)。入力は多態。
 *    - 直列が保守的既定、並列は caps による宣言制 opt-in。
 *    - このヘッダは最下層の契約。開発体験(静的/動的ノードの2トレイト等)は上の SDK 層。
 *
 *  ■ ロードマップ (このヘッダにまだ無い/部分的なもの)
 *    - 型伝播 方式b(connected_type) への移行、ホスト側 convert ノード自動挿入。
 *    - 二層 SDK API(簡易 eager + 再開可能 lazy)。これが入ると negotiate は反復poll化し得る。
 *    - Salsa 風インクリメンタル計算層、ROI/領域交渉、並列軸2/4。
 *    - canonical 型集合の確定、field 意味論(意図的に延期)、粗粒度ホットリロード。
 *
 *  ■ 用語
 *    URID=URI のセッション安定整数 / SVO=SYN_VALUE_INLINE(16byte)以下を payload に直接格納 /
 *    型パッケージ=データ型登録 / 処理プラグイン=ノードロジック登録 /
 *    multi-input=1ポートで N リンク受理(fan-in) / canonical型=ホストが理解する標準型(Tier A) /
 *    方式a/b=汎用ノード出力型の ANY 解決 / 接続入力型の伝播 /
 *    dirty伝播=状態変更で当該+下流のキャッシュ無効化 / passthrough=未知型の素通し転送。
 *
 *  詳細な決定根拠・却下案・未決事項は ADR (docs/synapse_adr.md) を参照。
 * ========================================================================== */
"#;

const TRAILER: &str = r#"
/* 末尾(インクルードガード外)に置かれるため、補助定義には独自ガードを付ける。 */
#ifndef SYNAPSE_ABI_HELPERS_INCLUDED
#define SYNAPSE_ABI_HELPERS_INCLUDED

/* ---- インライン補助関数 (cbindgen は本体を生成しないので C 側で付与) ------- */
static inline bool syn_is_inline(const SynValue *v) { return v->size <= SYN_VALUE_INLINE; }
static inline bool syn_is_empty (const SynValue *v) { return v->type_id == SYN_URID_INVALID; }

static_assert(sizeof(SynUrid) == 4, "type_id 空間を安定させるため SynUrid は 32-bit");

#endif /* SYNAPSE_ABI_HELPERS_INCLUDED */

/* ============================================================================
 *  ホスト側の評価ループ(非再帰・スタック駆動・pull) — v1 の2パス形
 * ----------------------------------------------------------------------------
 *   create(inst)                 // 1回(キャッシュ)
 *   declare(inst, builder)       // トポロジ/状態変化時に冪等再宣言
 *   for each (output, frame) demanded:
 *       negotiate(inst, ctx)     // データ無し。必要入力を request で1回列挙
 *       上流を作業スタックに push して全要求を充足(FFIは再帰しない)
 *       process(inst, ctx)       // get_input で読み(値渡し), set_output/passthrough で書く
 * ========================================================================== */
"#;

fn main() {
    let includes: String = LAYERS
        .iter()
        .map(|h| format!("#include \"{h}\"\n"))
        .collect();

    let body = format!(
        "{HEADER}\
         #ifndef SYNAPSE_ABI_H\n\
         #define SYNAPSE_ABI_H\n\
         \n\
         /* 層ヘッダ（正本と 1:1）。個別に include してもよいが、通常はこの 1 本でよい。 */\n\
         {includes}\n\
         #endif /* SYNAPSE_ABI_H */\n\
         {TRAILER}"
    );

    let out = synapse_abi_buildgen::include_dir().join("synapse_abi.h");
    std::fs::write(&out, body).expect("write umbrella header");

    println!("cargo:rerun-if-changed=build.rs");
}
