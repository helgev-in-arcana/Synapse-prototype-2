/* ============================================================================
 *  自動生成ファイル — 手で編集しないこと。正本: abi/synapse-abi-core/src/
 * ----------------------------------------------------------------------------
 *  基底 ABI 層 — 版一致必須の凍結面。
 *  ここの型レイアウトを変えることは破壊的変更であり SYN_ABI_VERSION の更新を要する
 *  （ホストは != で拒否する。ADR-020）。この層は suite/ext を参照しない。
 *
 *  通常は synapse_abi.h（アンブレラ）を include すればよい。
 * ========================================================================== */


#ifndef SYNAPSE_ABI_CORE_H
#define SYNAPSE_ABI_CORE_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <assert.h>

/* ---- 不透明ハンドル（不完全型・正本: src/handle.rs の opaque_handle!） ---- */
typedef struct SynNode SynNode;
typedef struct SynDeclBuilder SynDeclBuilder;
typedef struct SynEvalCtx SynEvalCtx;

/* ---- 文字列 ABI 定数 (正本: このクレートの Rust ソース) ---- */
#define SYN_MODULE_ENTRY_SYMBOL "synapse_module"


/*
 ABI バージョン（当面はこの 1 個で足りる）。

 基底 ABI（本モジュールと `SynNodeDesc` / `SynValue`）は版内で固定で、ホストは
 `!=` で拒否する（ADR-020）。スイートは id 引きで版に載らない。
 v2: 2フェーズロード（ADR-027）＋ alloc の type_id 引数（ADR-029）。
 v3: SVO インライン幅を 16byte へ拡張、payload を union 化（ADR-006/Open-20）。
 */
#define SYN_ABI_VERSION 3

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
 SVO インライン幅（バイト）。`size <= SYN_VALUE_INLINE` の値は `SynValue` の payload に
 直接格納する。ポインタ幅ではなく定数 16（color RGBA f32・vec2 f64・time rational 等の
 最頻パラメータ型が収まる幅。32-bit/64-bit で挙動が揃う）。
 */
#define SYN_VALUE_INLINE 16

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

 ロードは 2 フェーズ（ADR-027）: ホストは**全モジュール**の `on_register_types` を先に呼び、
 その後**全モジュール**の `on_register_nodes` を呼ぶ。これにより「ノード登録時には参照しうる
 型がすべて出揃っている」を保証する。型登録フェーズ内で他モジュールの型に依存してはならない
 （型-型依存の禁止、ADR-028 ★）。
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
   フェーズ1: 型をここで登録する（スイート fetch もここで行う）。型が無ければ NULL 可。
   他モジュールの型を lookup してはならない（ADR-028。ホストは違反を拒否してよい）。
   */
  SynStatus (*on_register_types)(SynHost h);
  /*
   フェーズ2: ノードをここで登録する。全モジュールの型登録後に呼ばれる。
   ノードが無ければ NULL 可。
   */
  SynStatus (*on_register_nodes)(SynHost h);
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
 `SynValue` の payload 領域（インライン格納と領域ポインタの重ね合わせ）。

 どちらのフィールドが有効かは `SynValue::size` で決まる（`size <= SYN_VALUE_INLINE` なら
 `data`、超えるなら `ptr`）。`data` の読み書きは型 pun ではなく memcpy 経由で行うこと。
 */
typedef union SynValuePayload {
  /*
   size > SYN_VALUE_INLINE: ホスト所有領域へのポインタ / OPAQUE 型: 不透明ハンドル。
   */
  void *ptr;
  /*
   size <= SYN_VALUE_INLINE: 値そのもの（SVO インライン。memcpy で出し入れする）。
   */
  uint8_t data[16];
} SynValuePayload;

/*
 エッジを流れるデータ単位。

 `size <= SYN_VALUE_INLINE`（16byte）のとき payload は `data` に直接格納する
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
   size>16: 領域ポインタ（`ptr`） / size<=16: 値そのもの（`data`, SVO） /
   opaque型: 不透明ハンドル（`ptr`）。
   */
  union SynValuePayload payload;
  /*
   意味的なバイト数。
   */
  size_t size;
} SynValue;

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

#endif  /* SYNAPSE_ABI_CORE_H */
