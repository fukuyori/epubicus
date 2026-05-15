# スクリプト整理 完了サマリ

`scripts/` 整理作業の完了サマリと、残タスク。詳細なスクリプト仕様は [scripts-reference.ja.md](scripts-reference.ja.md) 参照。

## 整理後の構成（14 スクリプト）

| カテゴリ | スクリプト | 役割 |
|---|---|---|
| 前処理 | `inspect-epub.ps1` | EPUB の spine/toc 表示（API なし） |
| 前処理 | `create-glossary.ps1` | 用語集候補生成（API なし） |
| 翻訳本体 | `translate-claude.{ps1,sh}` | Claude 翻訳 |
| 翻訳本体 | `translate-deepseek.{ps1,sh}` | DeepSeek 翻訳 |
| 翻訳本体 | `translate-ollama.{ps1,sh}` | Ollama（ローカル）翻訳 |
| 翻訳本体 | `translate-openai.{ps1,sh}` | OpenAI 翻訳 |
| 翻訳本体 | `translate-openai-batch.{ps1,sh}` | OpenAI Batch API 翻訳 |
| 見積もり | `usage.ps1` | API 使用量見積もり（汎用、`-Provider` 必須、API なし） |
| 再構築 | `rebuild-from-cache.ps1` | キャッシュから EPUB 再構築（汎用、provider 自動検出、API なし） |
| リカバリ | `recover-from-cache.ps1` | recovery.jsonl ベース recover（汎用、`-Manual` で手動 JSON 対応） |
| Batch リカバリ | `batch-recover-local.ps1` | Batch API 専用ワークフロー（独立） |
| スキャン | `scan-and-recover.ps1` | 出力 EPUB スキャン + recover（汎用、`-Provider` 必須） |
| ユーティリティ | `clear-all-caches.ps1` | 全キャッシュ削除 |
| ビルド | `build-release.{ps1,sh}` | リリースビルド（議論対象外） |

## 完了した整理項目

### 命名統一

- `*-env.template.ps1/.sh` → `translate-<provider>.ps1/.sh`
- `local-ollama-*` → `ollama-*`（`local-` 接頭辞削除）
- `rebuild.ps1` → `rebuild-from-cache.ps1`（`recover-from-cache.ps1` と並列性）

### 削除されたスクリプト

provider 専用の薄ラッパー 9 本を削除し、汎用本体への直接呼び出しに移行:

| 削除 | 代替コマンド |
|---|---|
| `convert-deepseek.ps1` | `translate-deepseek.ps1 .\book.epub` |
| `page-deepseek.ps1` | `translate-deepseek.ps1 .\book.epub -From X -To Y` |
| `usage-deepseek.ps1` | `usage.ps1 .\book.epub X Y -Provider deepseek` |
| `recover-deepseek.ps1` | `recover-from-cache.ps1 .\book.epub -Provider deepseek` |
| `recover-openai.ps1` | `recover-from-cache.ps1 .\book.epub -Provider openai` |
| `manual-recover-deepseek.ps1` | `recover-from-cache.ps1 .\book.epub -Provider deepseek -Manual <json>` |
| `scan-deepseek.ps1` | `scan-and-recover.ps1 .\book.epub -Provider deepseek -ScanOnly` |
| `scan-recover-deepseek.ps1` | `scan-and-recover.ps1 .\book.epub -Provider deepseek` |
| `rebuild-deepseek.ps1` | `rebuild-from-cache.ps1 .\book.epub` |

### translate-* 5 ファイルのパラメータ統一

全 5 つの translate スクリプトに共通パラメータを揃えた:

- `$InputPath` / `$From=0` / `$To=0` / `$Model` / `$Concurrency` / `$Style="essay"` / `$ExtraArgs` / `$UsageOnly` / `$PartialFromCache` / `$KindleFixedLayout` / `$NoKindleFixedLayout` / `$DevBuild` / `$NoRun` / `$PassthroughArgs`

ヘルパー関数も統一:

- `New-EpubicusTranslateArgs` / `Show-EpubicusTranslateCommands` / `Invoke-EpubicusTranslate`

ollama の特殊性（`$From=3, $To=3` 既定値、`$Mode="full|page|cache"` パラメータ、複数の `Invoke-EpubicusLocal*` ヘルパー）を除去。

### キャッシュ統一（案 A）

provider 別キャッシュディレクトリを廃止し、共通 `<ProjectRoot>/.cache/` に統一:

- `.deepseek-cache/` / `.claude-cache/` / `.openai-cache/` / `.local-ollama-cache/` / `.batch-openai-cache/` （旧）
- → `.cache/`（新、全 provider 共通）

`cache_key` には provider が SHA256 ハッシュとして含まれるため、複数 provider が同じディレクトリに書いても衝突しない。

### scan-and-recover.ps1 の `-Provider` 必須化

旧: `[string]$Provider = "ollama"` （ollama がデフォルト）
新: `[Parameter(Mandatory=$true)] [string]$Provider`（必須）

`usage.ps1` と一貫性のあるパターン。

### rebuild-from-cache.ps1 の provider 自動検出

`<.cache>/<input_hash>/translations.jsonl` を読んで、エントリの `provider`/`model` の最頻値を採用。`-Provider` 未指定で動作可能。

### recover-from-cache.ps1 の `-Provider` 既定値見直し

`Resolve-CacheRoot` を「ディレクトリ未作成でもエラーにしない」よう修正（初回実行時に `.cache/` がまだ存在しなくても epubicus が作成する）。

## 残タスク

### キャッシュルート未対応の 3 スクリプト

| スクリプト | 現在の値 | 対応 |
|---|---|---|
| `usage.ps1` | provider 別 switch（`.local-ollama-cache` 他 4 種） | `.cache` 単一に統一 |
| `batch-recover-local.ps1` | `.batch-openai-cache` | `.cache` に変更 |
| `clear-all-caches.ps1` | provider 別 5 dir 一覧 | `.cache` に集約 |

### ドキュメント更新

- [x] [scripts-reference.ja.md](scripts-reference.ja.md) — 完了
- [x] [script-cleanup-plan.ja.md](script-cleanup-plan.ja.md)（このファイル） — 完了
- [ ] README.md / README.ja.md
- [ ] docs/operation-guide.ja.md
- [ ] docs/translation-workflow.ja.md
- [ ] docs/detailed-examples.ja.md
- [ ] docs/cache-recovery-resume-design.ja.md
- [ ] docs/batch-*.md（cache root 参照のみ）

### 移行作業（既存ユーザ向け）

- 既存の `.deepseek-cache/<input_hash>/` 等の中身を `.cache/<input_hash>/` へ統合（SHA256 サブディレクトリは衝突しない、単純な `mv` で OK）

## 設計上の方針メモ

### env テンプレート → translate スクリプトへの移行理由

旧 `*-env.template.ps1` は「コピーしてカスタマイズする雛形」のニュアンスを持っていたが、実体は完全な翻訳実行スクリプト。命名が誤解を招くため `translate-<provider>.ps1` に改名。

### 共通キャッシュにする理由

- `cache_key` 内に provider が含まれるため、provider が異なれば同じ source でも別エントリとして共存できる
- リカバリ時の cross-provider 操作（DeepSeek 失敗を OpenAI で追い翻訳）も同じ cache root で完結
- ディレクトリが 1 つになるため `.gitignore` も簡潔
- provider 別の独立性は cache_key レベルで保証されている

### `-Provider` 必須化の方針（usage / scan-and-recover）

`-Provider` のデフォルトを設けない。理由:

- 必ず明示することで意図しない provider への呼び出しを防ぐ
- 多 provider 環境ではデフォルトが無いほうが安全
- `usage.ps1` / `scan-and-recover.ps1` の挙動を統一

### `rebuild-from-cache.ps1` の自動検出

cache_key 計算には provider/model が必要だが、cache メタデータから推定可能なので UX 上は省略可能とした。明示指定で上書きできる。
