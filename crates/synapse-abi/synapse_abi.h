/* ============================================================================
 *  自動生成ファイル — 手で編集しないこと。
 *  正本(SSoT)は Rust の synapse-abi crate (src/lib.rs, src/abi_strings.rs)。
 *  再生成: `cargo build`、または
 *    cbindgen --config cbindgen.toml -o synapse_abi.h .
 *    （CLI 直叩きは文字列 ABI 定数を出さない。完全な生成は cargo build 経由）。
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
 *    URID=URI のセッション安定整数 / SVO=8byte以下を ptr に直接格納 /
 *    型パッケージ=データ型登録 / 処理プラグイン=ノードロジック登録 /
 *    multi-input=1ポートで N リンク受理(fan-in) / canonical型=ホストが理解する標準型(Tier A) /
 *    方式a/b=汎用ノード出力型の ANY 解決 / 接続入力型の伝播 /
 *    dirty伝播=状態変更で当該+下流のキャッシュ無効化 / passthrough=未知型の素通し転送。
 *
 *  詳細な決定根拠・却下案・未決事項は ADR (docs/synapse_adr.md) を参照。
 * ========================================================================== */


#ifndef SYNAPSE_ABI_H
#define SYNAPSE_ABI_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <assert.h>

/* ---- 不透明ハンドル（不完全型・正本: src/lib.rs の opaque_handle!） ---- */
typedef struct SynNode SynNode;
typedef struct SynDeclBuilder SynDeclBuilder;
typedef struct SynEvalCtx SynEvalCtx;

/* ---- 文字列 ABI 定数 (正本: src/abi_strings.rs) ---- */
#define SYN_TYPE_REGISTRY_SUITE "synapse:type-registry"
#define SYN_URID_SUITE          "synapse:urid"
#define SYN_DECL_SUITE          "synapse:decl"
#define SYN_EVAL_SUITE          "synapse:eval"
#define SYN_EXT_UI              "synapse:ext:ui"
#define SYN_MODULE_ENTRY_SYMBOL "synapse_module"


/*
 ログレベル（host->log の level 引数）: エラー。
 */
#define SYN_LOG_ERROR 0

/*
 ログレベル: 警告。
 */
#define SYN_LOG_WARN 1

/*
 ログレベル: 情報。
 */
#define SYN_LOG_INFO 2

/*
 ログレベル: デバッグ。
 */
#define SYN_LOG_DEBUG 3

/*
 memcpy/シリアライズ可能な素のバイト列。
 */
#define SYN_TYPE_PLAIN_BYTES (1 << 0)

/*
 ptr は不透明ハンドル。get_api で操作する。
 */
#define SYN_TYPE_OPAQUE (1 << 1)

/*
 clone は参照カウント増加（浅いコピー）。
 */
#define SYN_TYPE_SHARED (1 << 2)

/*
 fan-in: N 本のリンクを受け、順序付き配列で配送する入力ポート。
 */
#define SYN_PORT_MULTI (1 << 0)

/*
 インスタンス再入タイリング（宣言のみ・実装は後回し可）。
 */
#define SYN_CAP_REENTRANT_TILING (1 << 0)

/*
 フレーム並列レンダ（宣言のみ）。
 */
#define SYN_CAP_PARALLEL_FRAMES (1 << 1)

/*
 UI と process の同一インスタンス並行を許可。
 */
#define SYN_CAP_THREAD_SAFE_UI (1 << 2)

/*
 ABI バージョン（当面はこの 1 個で足りる）。
 */
#define SYN_ABI_VERSION 1

/*
 ステータスコード（SYN_OK / SYN_ERR_*）。
 */
typedef int32_t SynStatus;

/*
 URI をセッション内で写像した安定整数。型 ID もこの空間。0 は無効。
 */
typedef uint32_t SynUrid;

/*
 型 ID（URID と同一空間）。
 */
typedef SynUrid SynTypeId;

/*
 1 ノード種別の記述子。ロード時に host->register_node で登録する。
 必須: create/destroy/declare/negotiate/process。
 */
typedef struct SynNodeDesc {
  /*
   SYN_CAP_*。
   */
  uint32_t caps;
  /*
   ノード URI（"com.vendor.blur.gaussian"）。モジュール寿命まで存続(static 推奨)。
   */
  const char *node_uri;
  /*
   表示名。
   */
  const char *display_name;
  /*
   既定状態でインスタンスを生成。node は instance に保持し host->mark_dirty 等に使う。
   */
  SynStatus (*create)(SynNode *node, void **out_instance);
  /*
   インスタンス破棄。
   */
  void (*destroy)(void *instance);
  /*
   宣言フェーズ（データ無し）。状態からフル再宣言する冪等関数。
   */
  SynStatus (*declare)(void *instance, SynDeclBuilder *b);
  /*
   poll 列挙（データ無し・単発）。必要入力を request で積み SYN_OK を返す。
   静的ノードは全入力を列挙するだけ。値依存の枝刈りは将来の二層 API で対応。
   */
  SynStatus (*negotiate)(void *instance, SynEvalCtx *ctx);
  /*
   処理（1 回）。要求が全充足された後に呼ばれる。
   */
  SynStatus (*process)(void *instance, SynEvalCtx *ctx);
  /*
   ソケットに出ない内部パラメータの永続化（不透明 blob）。out=NULL/cap=0 で
   必要サイズを written に返し、確保後に再呼び出しで書き込む。blob は自己記述
   （自前の version を内包）。ソケットデフォルトは host 専有でプラグインは値を
   保持しない（get_input の借用のみ）ため、blob には構造的に内部パラメータだけが
   入る。NULL 可。
   */
  SynStatus (*save_state)(void *instance, void *out, size_t cap, size_t *written);
  /*
   内部パラメータの復元。NULL 可。
   */
  SynStatus (*load_state)(void *instance, const void *input, size_t len);
  /*
   任意拡張（UI 等）。未対応は NULL を返す。
   */
  const void *(*get_extension)(void *instance, const char *ext_id);
} SynNodeDesc;

/*
 ホストが提供する操作群。on_load で受け取りモジュール側に保持する（1 モジュール=1 ホスト）。
 */
typedef struct SynHostStruct {
  /*
   ホスト側の不透明ポインタ。
   */
  void *host_ctx;
  /*
   スイートを id で取得（未提供なら NULL）。
   */
  const void *(*fetch_suite)(struct SynHostStruct *h, const char *suite_id);
  /*
   ノード記述子を登録（on_load 中に呼ぶ）。
   */
  SynStatus (*register_node)(struct SynHostStruct *h, const struct SynNodeDesc *desc);
  /*
   状態変更通知。当該ノード+下流サブツリーのキャッシュを無効化する。
   */
  void (*mark_dirty)(struct SynHostStruct *h, SynNode *node);
  /*
   ログ出力（level は SYN_LOG_*）。
   */
  void (*log)(struct SynHostStruct *h, int level, const char *msg);
} SynHostStruct;

/*
 ホストハンドル（SynHostStruct へのポインタ）。
 */
typedef struct SynHostStruct *SynHost;

/*
 モジュール記述子。各 .so/.dll は `synapse_module` を 1 つだけエクスポートする。
 */
typedef struct SynModule {
  /*
   ビルド時の SYN_ABI_VERSION。
   */
  uint32_t abi_version;
  /*
   名前空間（"com.vendor.blur"）。
   */
  const char *module_uri;
  /*
   semver 文字列。
   */
  const char *module_version;
  /*
   型/ノードをここで登録する。
   */
  SynStatus (*on_load)(SynHost h);
  /*
   アンロード前に 1 回。
   */
  void (*on_unload)(SynHost h);
} SynModule;

/*
 モジュールエントリのシグネチャ: `const SynModule *synapse_module(void);`
 */
typedef const struct SynModule *(*SynModuleEntryFn)(void);

/*
 エッジを流れるデータ単位。

 `size <= sizeof(void*)` のとき payload は `ptr` フィールドに直接格納する
 (small-value optimization)。読み書きは型 pun ではなく memcpy 経由で行うこと。
 不変条件: PLAIN 型の payload は位置独立な素のバイト列で、生ポインタを含まない。
 空（未接続かつデフォルト無し）の表現は `type_id == 0`。`ptr == NULL` は使わない
 （SVO では inline の零値と区別できないため）。
 */
typedef struct SynValue {
  /*
   実体型の URID。0 は空。
   */
  SynTypeId type_id;
  /*
   size>ptr幅: 領域ポインタ / size<=ptr幅: 値そのもの(SVO) / opaque型: 不透明ハンドル。
   */
  void *ptr;
  /*
   意味的なバイト数。
   */
  size_t size;
} SynValue;

/*
 型ごとの操作テーブル。メモリ確保/解放はホスト。型は構築/破棄/複製のみ知る。
 */
typedef struct SynTypeVTable {
  /*
   SYN_TYPE_* フラグ。
   */
  uint32_t flags;
  /*
   固定サイズ。可変なら 0。
   */
  size_t size;
  /*
   アラインメント要件。
   */
  size_t align;
  /*
   既定値を dst に構築する。
   */
  SynStatus (*init)(void *dst, SynTypeId t);
  /*
   複製。PLAIN/可変型はディープコピー。SHARED/OPAQUE は refcount++ でよい
   （passthrough で大きな/GPU リソースを複製しないために重要）。
   */
  SynStatus (*clone)(void *dst, const void *src, SynTypeId t);
  /*
   破棄のみ（free はホスト）。SHARED は refcount--、0 で実リソース解放。
   */
  void (*drop)(void *obj, SynTypeId t);
  /*
   永続化（PLAIN のみ）。out=NULL/cap=0 で必要サイズを written に返す。OPAQUE は NULL 可。
   */
  SynStatus (*serialize)(const void *obj, SynTypeId t, void *out, size_t cap, size_t *written);
  /*
   復元（PLAIN のみ）。OPAQUE は NULL 可。
   */
  SynStatus (*deserialize)(void *dst, SynTypeId t, const void *input, size_t len);
  /*
   型+API 二層: opaque 型が自前の API テーブルを公開する（例 "synapse:gpu:texture"）。
   PLAIN 型は NULL。
   */
  const void *(*get_api)(SynTypeId t, const char *api_id);
} SynTypeVTable;

/*
 型の登録・解決。
 */
typedef struct SynTypeRegistrySuite {
  /*
   型を URI と vtable で登録する。
   */
  SynStatus (*register_type)(const char *uri, const struct SynTypeVTable *vt);
  /*
   型 ID から vtable を解決する（結果はセッション中キャッシュ可）。
   */
  const struct SynTypeVTable *(*lookup)(SynTypeId t);
  /*
   URI から型 ID を得る。
   */
  SynTypeId (*type_of)(const char *uri);
} SynTypeRegistrySuite;

/*
 URI と URID の相互変換。
 */
typedef struct SynUridSuite {
  /*
   URI を URID に写像（intern、セッション不変）。
   */
  SynUrid (*map)(const char *uri);
  /*
   URID から URI を借用（セッション中のみ有効）。
   */
  const char *(*unmap)(SynUrid id);
} SynUridSuite;

/*
 declare 内で呼ぶソケット宣言関数群。プラグインは確保せずこれらを呼ぶだけ。
 key は配列インデックスではなく論理的同一性から導くこと（接続が壊れる）。
 */
typedef struct SynDeclSuite {
  /*
   出力ポート: 型はちょうど 1 つ。汎用ノードは SYN_TYPE_ANY を渡してよい（方式a）。
   */
  SynStatus (*output)(SynDeclBuilder *b, const char *key, const char *label, SynTypeId ty);
  /*
   入力ポート: types のいずれかを受理（多態）。SYN_TYPE_ANY で全許容。
   flags は SYN_PORT_*。
   */
  SynStatus (*input)(SynDeclBuilder *b,
                     const char *key,
                     const char *label,
                     const SynTypeId *types,
                     size_t n_types,
                     uint32_t flags);
  /*
   方式b（型伝播）: 接続中の入力の実体型を返す。declare 内で出力型の導出に使う。
   未接続/不定なら SYN_TYPE_ANY。v1 は使わず ANY で書いてよい。
   */
  SynTypeId (*connected_type)(SynDeclBuilder *b, const char *input_key, uint32_t link_index);
  /*
   入力ソケットの初期デフォルト値（未接続時に get_input が返す値=パラメータ）。
   value は値渡し（SynValue 自体はコピー。大型データは value.ptr 経由の借用で、
   呼び出し中のみ有効）。ホストが型 vtable で複製して保持する。
   再 declare では key 一致ソケットの既存値を保持し、初期値は新規 key にのみ適用。
   */
  SynStatus (*input_default)(SynDeclBuilder *b, const char *key, struct SynValue value);
} SynDeclSuite;

/*
 時刻（フレーム）を有理数で表す。単一フレームなら未使用可。
 */
typedef struct SynRational {
  /*
   分子。
   */
  int64_t num;
  /*
   分母。
   */
  int64_t den;
} SynRational;

/*
 入力要求記述子。領域(ROI)は v1 では扱わない。
 */
typedef struct SynRequest {
  /*
   宣言した入力ポート（declare の呼び出し順 0 始まり）。
   */
  uint32_t input_index;
  /*
   multi-input 上のどのリンクか（単一なら 0）。
   */
  uint32_t link_index;
  /*
   必要な時刻。
   */
  struct SynRational frame;
} SynRequest;

/*
 negotiate/process から使う評価操作群。

 データ受け渡しの規約: `SynValue` は常に**値渡し**で境界を越える（構造体自体はコピー）。
 参照は SynValue 内の `ptr` を通してのみ行い、大型データ(>ptr幅)の `ptr` が指す領域は
 ホスト所有・その呼び出し中のみ借用可能。SVO(≤ptr幅)は値が `ptr` フィールドに入った
 まま丸ごとコピーされるので、プラグインローカルがホストから見えない問題は起きない。
 */
typedef struct SynEvalSuite {
  /*
   negotiate 中: 必要入力を積む。
   */
  SynStatus (*request)(SynEvalCtx *ctx, const struct SynRequest *req);
  /*
   multi-input ポートに接続されたリンク数。
   */
  uint32_t (*link_count)(SynEvalCtx *ctx, uint32_t input_index);
  /*
   process 中: 入力値を**値で受け取る**（この呼び出しの間のみ有効＝大型データの ptr が
   指すホスト所有領域はこの呼び出し中だけ借用可能）。
   未接続でも、デフォルトを持つソケットはホスト用意のデフォルト値を返す。
   デフォルトの無い未接続ソケットは type_id==0（空）→plugin が処理する。
   */
  struct SynValue (*get_input)(SynEvalCtx *ctx, uint32_t input_index, uint32_t link_index);
  /*
   process 中: 大型(>ptr幅)出力用にホスト所有バッファを確保して先頭ポインタを返す。
   プラグインはここへ書き、その ptr を SynValue.ptr に入れて set_output に値渡しする。
   確保はホスト（ADR-012）。SVO 型は確保不要（set_output だけで完結）。失敗時 NULL。
   */
  void *(*alloc)(SynEvalCtx *ctx, size_t size);
  /*
   process 中: 生産した出力値を**値渡し**でホストへ引き渡す。SVO はこれだけで完結。
   value.type_id は宣言した出力型に一致（ANY 宣言の汎用ノードは解決済み実体型を入れる）。
   */
  SynStatus (*set_output)(SynEvalCtx *ctx, uint32_t output_index, struct SynValue value);
  /*
   未知型パススルー保証: 中身を見ず入力値を出力へ転送する（値渡し。ホストが clone）。
   */
  SynStatus (*passthrough)(SynEvalCtx *ctx, uint32_t output_index, struct SynValue input_value);
} SynEvalSuite;

/*
 UI コンポーネント拡張。
 */
typedef struct SynUiExt {
  /*
   UI を構築する（host_ui_handle はホスト側 UI コンテキスト）。
   */
  SynStatus (*build)(void *instance, void *host_ui_handle);
  /*
   パラメータ変更の通知（param_key は変更されたソケット key）。
   */
  SynStatus (*on_change)(void *instance, const char *param_key);
} SynUiExt;

/*
 成功。
 */
#define SYN_OK 0

/*
 原因不明の失敗（FFI 越えパニックの遮断時にも使う）。
 */
#define SYN_ERR_UNKNOWN -1

/*
 要求された操作が未対応。
 */
#define SYN_ERR_UNSUPPORTED -2

/*
 引数が不正。
 */
#define SYN_ERR_BAD_ARG -3

/*
 メモリ確保に失敗。
 */
#define SYN_ERR_NO_MEMORY -4

/*
 型が一致しない。
 */
#define SYN_ERR_TYPE_MISMATCH -5

/*
 無効 URID。空値 sentinel（`type_id == 0`）でもある。
 */
#define SYN_URID_INVALID 0

/*
 予約: 汎用ノードの出力/入力で「任意型」を表す（方式a）。
 接続判定で ANY は全許容。データは実体 type_id を運ぶ。
 */
#define SYN_TYPE_ANY 1

#endif  /* SYNAPSE_ABI_H */

/* 末尾(インクルードガード外)に置かれるため、補助定義には独自ガードを付ける。 */
#ifndef SYNAPSE_ABI_HELPERS_INCLUDED
#define SYNAPSE_ABI_HELPERS_INCLUDED

/* ---- インライン補助関数 (cbindgen は本体を生成しないので C 側で付与) ------- */
static inline bool syn_is_inline(const SynValue *v) { return v->size <= sizeof(void *); }
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
