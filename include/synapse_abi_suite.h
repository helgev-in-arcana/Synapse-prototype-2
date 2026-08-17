/* ============================================================================
 *  自動生成ファイル — 手で編集しないこと。正本: abi/synapse-abi-suite/src/
 * ----------------------------------------------------------------------------
 *  スイート層 — host->fetch_suite(id) で取得する。
 *  未知の id には NULL が返るため、スイートの「追加」は基底 ABI に触れない非破壊変更。
 *  ただし個々のスイート構造体に版数は無く、フィールド追加は基底同様に破壊的（Open-13）。
 *
 *  通常は synapse_abi.h（アンブレラ）を include すればよい。
 * ========================================================================== */


#ifndef SYNAPSE_ABI_SUITE_H
#define SYNAPSE_ABI_SUITE_H


#include "synapse_abi_core.h"

/* ---- 文字列 ABI 定数 (正本: このクレートの Rust ソース) ---- */
#define SYN_DECL_SUITE          "synapse:decl"
#define SYN_EVAL_SUITE          "synapse:eval"
#define SYN_TYPE_REGISTRY_SUITE "synapse:type-registry"
#define SYN_URID_SUITE          "synapse:urid"


/*
 fan-in: N 本のリンクを受け、順序付き配列で配送する入力ポート。
 */
#define SYN_PORT_MULTI (1 << 0)

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
  SynStatus (*input_default)(SynDeclBuilder *b, const char *key, SynValue value);
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
 参照は payload の `ptr` を通してのみ行い、大型データ(>SYN_VALUE_INLINE)の `ptr` が指す
 領域はホスト所有・その呼び出し中のみ借用可能。SVO(≤SYN_VALUE_INLINE)は値が payload に
 入ったまま丸ごとコピーされるので、プラグインローカルがホストから見えない問題は起きない。
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
  SynValue (*get_input)(SynEvalCtx *ctx, uint32_t input_index, uint32_t link_index);
  /*
   process 中: 大型(>SYN_VALUE_INLINE)出力用にホスト所有バッファを確保して先頭ポインタを
   返す。プラグインはここへ書き、その ptr を payload に入れて set_output に値渡しする。
   確保はホスト（ADR-012）。`t` は値の実体型（ANY 宣言の汎用ノードは解決済み実体型を
   渡す）。ホストは登録済み vtable の align 属性を満たすバッファを返す（ADR-029）。
   SVO 型は確保不要（set_output だけで完結）。失敗・未登録型は NULL。
   */
  void *(*alloc)(SynEvalCtx *ctx, size_t size, SynTypeId t);
  /*
   process 中: 生産した出力値を**値渡し**でホストへ引き渡す。SVO はこれだけで完結。
   value.type_id は宣言した出力型に一致（ANY 宣言の汎用ノードは解決済み実体型を入れる）。
   */
  SynStatus (*set_output)(SynEvalCtx *ctx, uint32_t output_index, SynValue value);
  /*
   未知型パススルー保証: 中身を見ず入力値を出力へ転送する（値渡し。ホストが clone）。
   */
  SynStatus (*passthrough)(SynEvalCtx *ctx, uint32_t output_index, SynValue input_value);
} SynEvalSuite;

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
   アラインメント要件（2の冪。register_type が検証する）。可変サイズ型では
   ホスト確保バッファ先頭のアラインメントとして解釈する（ADR-029）。
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
   型を URI と vtable で登録する。vt->align が 2 の冪でなければ SYN_ERR_BAD_ARG（ADR-029）。
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

#endif  /* SYNAPSE_ABI_SUITE_H */
