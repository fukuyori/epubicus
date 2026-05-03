# 共通処理メモ

この文書は、epubicus の各コマンドに共通する処理を整理したものです。機能追加や不具合修正のときに、「どこを直せば他の経路にも効くか」を追いやすくするためのメモです。

## 全体の考え方

epubicus はコマンドが多いですが、実際には次の共通処理を組み合わせています。

1. 入力 EPUB のロック
2. EPUB から block を抽出
3. inline マーカーを埋め込んだ source text を作る
4. cache key を計算してキャッシュ確認
5. provider 呼び出し
6. 応答の validation
7. 必要なら retry prompt で再試行
8. 成功したら cache に保存
9. 失敗したら recovery / batch state に記録
10. 最後に cache から EPUB を組み立てる

そのため、複数の機能にまたがる問題は、末端のコマンドごとではなく、この共通処理のどこにあるかで考えると追いやすくなります。

## 1. 入力ロック

同じ入力 EPUB に対する同時実行は、入力ロックで防ぎます。

- 実装:
  - [src/input_lock.rs](D:/home/source/rust/epubicus/src/input_lock.rs)
  - [src/lock.rs](D:/home/source/rust/epubicus/src/lock.rs)
- 主に使うコマンド:
  - `translate`
  - `inspect`
  - `toc`
  - `glossary`
  - `batch prepare/submit/fetch/import/health/verify/reroute-local/translate-local/retry-requests`
  - `unlock`

ここを直すと、通常翻訳と batch 系の両方に効きます。

## 2. block 抽出と inline マーカー化

XHTML から翻訳対象 block を抜き出し、タグを `⟦...⟧` に置き換えた source text を作る処理です。

- 実装:
  - [src/xhtml.rs](D:/home/source/rust/epubicus/src/xhtml.rs)
  - [src/main.rs](D:/home/source/rust/epubicus/src/main.rs)
  - [src/recovery/scan.rs](D:/home/source/rust/epubicus/src/recovery/scan.rs)

共通点:

- 通常翻訳でも batch prepare でも recovery scan でも、最終的には同じ source text の考え方を使います。
- placeholder 崩れやタグ復元失敗を調べるときは、この層を見るのが先です。

## 3. provider 呼び出し前の prompt 構築

通常 prompt と retry prompt は共通です。

- 実装:
  - [src/prompt.rs](D:/home/source/rust/epubicus/src/prompt.rs)

重要な点:

- glossary の埋め込み
- source block の埋め込み
- validation failure reason ごとの retry 指示

`untranslated_segment` や `missing_placeholder` の retry 挙動を揃えたいときは、まずここを確認します。

## 4. provider 呼び出しと validation

翻訳の本体は `Translator` に集約されています。

- 実装:
  - [src/translator.rs](D:/home/source/rust/epubicus/src/translator.rs)

共通化されている処理:

- `translate_uncached_source()`
  - 未キャッシュ block の共通翻訳入口
- `translate_with_validation()`
  - provider 応答の validation と retry
- `request_json_with_retry()`
  - HTTP retry
- `validate_translation_response()`
  - placeholder、未翻訳英語、truncated、explanation 混入などの検証
- `validation_failure_reason()`
  - failure を理由コードに分類
- `is_provider_auth_error()`
  - 401/403 などの認証系判定

この層は次の機能で共有されます。

- 通常の `translate`
- `recover`
- `batch translate-local`
- batch import 後の validation
- recovery scan の suspicious 判定

つまり、translation quality の問題や validation 基準の変更は、ここを直せば複数経路に反映されます。

## 5. cache

翻訳結果の正本は cache です。

- 実装:
  - [src/cache.rs](D:/home/source/rust/epubicus/src/cache.rs)

共通点:

- 通常翻訳も batch も、成功結果は最終的に同じ cache に入ります。
- EPUB 再生成は cache を見て行います。
- `partial-from-cache`、`batch import`、`batch translate-local`、`recover` はすべて cache を介してつながります。

そのため、batch の成果物より cache の状態のほうが運用上は重要です。

## 6. recovery record と suggested action

失敗時の人間向け記録は recovery record の考え方に寄せています。

- 実装:
  - [src/recovery/report.rs](D:/home/source/rust/epubicus/src/recovery/report.rs)
  - [src/recovery/log.rs](D:/home/source/rust/epubicus/src/recovery/log.rs)
  - [src/recovery/command.rs](D:/home/source/rust/epubicus/src/recovery/command.rs)
  - [src/batch/reroute.rs](D:/home/source/rust/epubicus/src/batch/reroute.rs)

共通化したい見方:

- `reason`
- `validation_reason`
- `suggested_action`

通常の recovery log と `batch translate-local` の `last_error` は、できるだけ同じ読み方で判断できるように揃えるのが方針です。

### 記録先の役割分担

- `work_items.jsonl`
  - batch の現在状態
  - `state`
  - `last_error`
- `rejected.jsonl`
  - remote Batch import 時の validation reject
- `errors.jsonl`
  - remote request failure
- `recovery.jsonl`
  - 最終 EPUB 側の復旧対象
- `failed.jsonl`
  - `recover` で最後まで埋めきれなかった対象

設計上は、「その場の state 管理」は batch 配下、「最終出力の復旧対象」は recovery 配下、という分担です。

## 7. batch state machine

batch 系は request/result のリモート処理を扱いますが、状態管理の考え方は共通です。

- 実装:
  - [src/batch/model.rs](D:/home/source/rust/epubicus/src/batch/model.rs)
  - [src/batch/local.rs](D:/home/source/rust/epubicus/src/batch/local.rs)
  - [src/batch/remote.rs](D:/home/source/rust/epubicus/src/batch/remote.rs)
  - [src/batch/report.rs](D:/home/source/rust/epubicus/src/batch/report.rs)
  - [src/batch/reroute.rs](D:/home/source/rust/epubicus/src/batch/reroute.rs)

代表的な state:

- `prepared`
- `submitted`
- `imported`
- `rejected`
- `failed`
- `local_pending`
- `local_imported`
- `local_exhausted`
- `skipped`

個別コマンドを触るときも、この state のどこを前提にしているかを見ると整理しやすいです。

## 8. 進捗表示と ETA

通常翻訳と local batch では表示内容が違いますが、進捗の考え方は共有しています。

- 実装:
  - [src/main.rs](D:/home/source/rust/epubicus/src/main.rs)
  - [src/batch/reroute.rs](D:/home/source/rust/epubicus/src/batch/reroute.rs)
  - [docs/runtime-progress.ja.md](D:/home/source/rust/epubicus/docs/runtime-progress.ja.md)

見ているもの:

- 完了 block 数
- cache 済み block 数
- 未キャッシュ文字数
- provider 実行時間
- stalled 状態

ETA や進捗のズレを直すときは、表示だけでなく「分母に何を入れているか」「何を completion とみなすか」を共通の論点として確認します。

## 9. 変更時の見方

### A. 通常翻訳と batch local の両方に効かせたい

優先して見る場所:

- [src/translator.rs](D:/home/source/rust/epubicus/src/translator.rs)
- [src/prompt.rs](D:/home/source/rust/epubicus/src/prompt.rs)
- [src/xhtml.rs](D:/home/source/rust/epubicus/src/xhtml.rs)

### B. batch の運用だけを変えたい

優先して見る場所:

- [src/batch/reroute.rs](D:/home/source/rust/epubicus/src/batch/reroute.rs)
- [src/batch/report.rs](D:/home/source/rust/epubicus/src/batch/report.rs)
- [src/batch/local.rs](D:/home/source/rust/epubicus/src/batch/local.rs)

### C. 人間向けの失敗記録や次アクションを揃えたい

優先して見る場所:

- [src/recovery/report.rs](D:/home/source/rust/epubicus/src/recovery/report.rs)
- [src/recovery/command.rs](D:/home/source/rust/epubicus/src/recovery/command.rs)
- [src/batch/reroute.rs](D:/home/source/rust/epubicus/src/batch/reroute.rs)

### D. 中断再開や stale lock を直したい

優先して見る場所:

- [src/input_lock.rs](D:/home/source/rust/epubicus/src/input_lock.rs)
- [src/lock.rs](D:/home/source/rust/epubicus/src/lock.rs)

## 10. 関連文書

- [docs/operation-guide.ja.md](D:/home/source/rust/epubicus/docs/operation-guide.ja.md)
- [docs/runtime-progress.ja.md](D:/home/source/rust/epubicus/docs/runtime-progress.ja.md)
- [docs/batch-recovery.ja.md](D:/home/source/rust/epubicus/docs/batch-recovery.ja.md)
- [docs/batch-translate-local.ja.md](D:/home/source/rust/epubicus/docs/batch-translate-local.ja.md)
- [docs/batch-api-design.md](D:/home/source/rust/epubicus/docs/batch-api-design.md)
