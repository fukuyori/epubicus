# scripts/ リファレンス

`scripts/` 以下の各スクリプトの目的・引数・使用例を記載するユーザ向けリファレンス。
整理作業（[script-cleanup-plan.ja.md](script-cleanup-plan.ja.md)）と並行して内容を埋めていく。

## 構成

ワークフロー順で記載:

1. [前処理](#前処理) — `inspect-epub` / `create-glossary`
2. [見積もり](#見積もり) — `usage`（汎用、API 呼び出しなし）
3. [翻訳本体](#翻訳本体) — provider ごとの 5 種（`translate-*`）
4. [リカバリ](#リカバリ) — 通常 / Batch 専用に分類
5. [スキャン](#スキャン) — `scan-and-recover`（汎用、`-Provider` 必須）
6. [再構築](#再構築) — `rebuild-from-cache`（汎用、provider 自動検出）
7. [キャッシュクリア](#キャッシュクリア) — `clear-all-caches`

すべての翻訳・見積もり・recover・scan・rebuild は **共通キャッシュ `<ProjectRoot>/.cache/`** を使う（cache_key に provider が含まれるため衝突しない）。
API 呼び出しの有無は各節で明記する。
すべてのスクリプトに `.ps1` (PowerShell) 版と `.sh` (POSIX shell) 版がある。

## ヘルプ表示

すべてのスクリプトに `-h` / `--help`（PowerShell では `-Help` / `-h`）フラグがあります。実行するとファイル先頭のコメントブロック（用法・引数説明）を表示して終了します。

```powershell
.\scripts\translate-deepseek.ps1 -Help
.\scripts\usage.ps1 -h
```

```sh
scripts/translate-deepseek.sh --help
scripts/usage.sh -h
```

## 未翻訳ブロックの扱い

`translate` / `rebuild-from-cache` / `recover-from-cache` 実行後に未翻訳ブロックが残った場合:

- **EPUB は出力される**（該当ブロックは原文がそのまま入る）
- メッセージは `warning:` 接頭辞で表示
- **終了コード 0（正常終了）** — 自動化スクリプトの動作を止めない
- `recovery.jsonl` に未翻訳ブロックが記録され、後で `recover-from-cache` で再試行可能

致命的エラー（EPUB を出力できない、ロック競合など）のみ `error:` + 終了コード 1 となります。

## オプションの短縮エイリアス

よく使うパラメータには PowerShell の `[Alias()]` および shell の short option が設定されている。両者で同じ短縮表記が使える。

| 長い名前 | 短縮 | 適用範囲 |
|---|---|---|
| `-Provider` / `--provider` | `-p` | usage / recover-from-cache / scan-and-recover / rebuild-from-cache |
| `-Model` / `--model` | `-m` | translate-* / usage / recover-from-cache / scan-and-recover / rebuild-from-cache |
| `-From` / `--from` | `-f` | translate-* |
| `-To` / `--to` | `-t` | translate-* |
| `-Concurrency` / `--concurrency` | `-c` | translate-* / recover-from-cache |
| `-Glossary` / `--glossary` | `-g` | translate-openai-batch / usage / recover-from-cache / scan-and-recover / rebuild-from-cache |
| `-Limit` / `--limit` | `-l` | recover-from-cache / scan-and-recover |
| `-Manual` / `--manual` | `-mn` | recover-from-cache |
| `-Style` / `--style` | `-s` | translate-* |
| `-CacheRoot` / `--cache-root` | `-cr` | usage / recover-from-cache / scan-and-recover / rebuild-from-cache |

使用例:

```powershell
# PowerShell
.\scripts\usage.ps1 .\book.epub 9 9 -p deepseek -m deepseek-v4-pro
.\scripts\translate-deepseek.ps1 .\book.epub -f 9 -t 9 -m deepseek-v4-pro
.\scripts\recover-from-cache.ps1 .\book.epub -p deepseek -m deepseek-v4-pro -l 50
```

```sh
# Shell
scripts/usage.sh ./book.epub 9 9 -p deepseek -m deepseek-v4-pro
scripts/translate-deepseek.sh ./book.epub -f 9 -t 9 -m deepseek-v4-pro
scripts/recover-from-cache.sh ./book.epub -p deepseek -m deepseek-v4-pro -l 50
```

---

## 前処理

### inspect-epub.ps1 / .sh

#### 概要

EPUB の本文ファイル順序（spine）と目次（toc）を表示する。
翻訳・見積もりコマンドの `FROM` / `TO` に渡す 1-based の spine 番号を選ぶために最初に実行する。
provider 非依存・API 呼び出しなし。

内部では `epubicus inspect` と `epubicus toc` の 2 サブコマンドを順次実行する。

#### 使用方法

```powershell
.\scripts\inspect-epub.ps1 <入力EPUB>
```

| 引数 | 必須 | 説明 |
|---|---|---|
| `[Position 0] $InputPath` | ◎ | 対象 EPUB のパス |
| `-NoRun` | | コマンドを表示するだけで実行しない |

#### 出力例

```text
InputEpub = D:\home\source\rust\epubicus\test\retro.epub

[inspect]
OPF: ...\OEBPS\content.opf

  No  Linear  Exists         Bytes   Blocks  Media Type              Href
------------------------------------------------------------------------------------------------
   1  yes     yes             5811       49  application/xhtml+xml   toc-page.xhtml
   2  yes     yes              322        0  application/xhtml+xml   chapter0.xhtml
   3  yes     yes            34179      120  application/xhtml+xml   chapter1.xhtml
   4  yes     yes           143071      489  application/xhtml+xml   chapter2.xhtml
  ...

[toc]
TOC: EPUB3 nav (nav.xhtml)

- title -> chapter0.xhtml
- Steve Jobs and NeXT Part 1 -> chapter1.xhtml#steve-jobs-and-next-part-1
- Steve Jobs and NeXT Part 2: The Long Road to Mac OS X -> chapter2.xhtml#...
- The Rise and Fall of the IBM PC -> chapter3.xhtml#...
  - **Shots Fired: The PS/2 and Micro-Channel** -> chapter3.xhtml#...
  ...
```

#### 出力の読み方

- **`[inspect]`**: spine 順に並んだ本文ファイルの一覧
  - `No` 列が翻訳コマンドに渡す 1-based の spine 番号
  - `Blocks` 列が翻訳対象ブロック数（多いほど翻訳量が多い）
  - `Bytes` 列が当該 XHTML のサイズ
- **`[toc]`**: EPUB の目次（章タイトル → XHTML パス）
- **使い分け**: `[toc]` で目次を見て章タイトルから「翻訳開始したい本文 XHTML」を特定し、`[inspect]` の同じ `Href` 行の `No` を後段の `FROM` / `TO` に渡す
- **試し翻訳の選び方**: `Blocks` が中程度（数十〜100 程度）で、章タイトルが本文っぽい行を 1 つ選ぶ。例えば上記の場合 `chapter1`（No 3、120 ブロック）で `3 3` を試す

---

### create-glossary.ps1 / .sh

#### 概要

EPUB から用語集候補を抽出し、入力 EPUB の隣に 2 ファイルを生成する:

- `book.json` — 用語集本体（`src` / `dst` ペア。翻訳時に `--glossary` で参照される）
- `book.md` — LLM への用語集レビュー依頼マークダウン（人名・地名・固有名詞の訳語を整える指示書）

provider 非依存・翻訳エンジン未使用（用語抽出は内部処理）。`book.json` が存在すれば後続の翻訳で自動的に `--glossary` に渡される（隣接ファイル自動検出）。

#### 使用方法

```powershell
.\scripts\create-glossary.ps1 <入力EPUB>
```

| 引数 | 既定値 | 説明 |
|---|---|---|
| `[Position 0] $InputPath` | — | 対象 EPUB のパス（必須） |
| `-MinOccurrences <n>` | `3` | 用語候補に含める最低出現回数 |
| `-MaxEntries <n>` | `200` | 用語候補の最大件数 |
| `-Force` | | 既存の `.json` / `.md` を上書き |
| `-EpubicusExe <path>` | | epubicus バイナリを指定（既定は `cargo run --release` 経由） |
| `-DevBuild` | | debug プロファイルで実行 |
| `-NoRun` | | コマンドを表示するだけで実行しない |

#### 出力例

コンソール出力:

```text
InputEpub = D:\home\source\rust\epubicus\test\retro.epub
JSON      = D:\home\source\rust\epubicus\test\retro.json
Markdown  = D:\home\source\rust\epubicus\test\retro.md
```

`retro.json`（用語集本体、抜粋）:

```json
{
  "source_lang": "en",
  "target_lang": "ja",
  "entries": [
    { "src": "Windows", "dst": "Windows" },
    { "src": "Microsoft", "dst": "マイクロソフト" },
    { "src": "Apple", "dst": "アップル" },
    { "src": "Macintosh", "dst": "Macintosh" },
    { "src": "MacOS", "dst": "Mac OS" }
  ]
}
```

`retro.md`（LLM レビュー依頼マークダウン、抜粋）:

```markdown
# EPUB 翻訳用語集レビュー依頼

以下は、原文 EPUB から自動抽出した用語集候補です。
この文章全体を作業指示として読み、最後の JSON を修正してください。

## 修正方針

- 重要な人名、地名、組織名、製品名、プロジェクト名、作品名、専門用語を残してください。
- 誤検出、章見出し、一般語、文頭に多いだけの単語は削除してください。
- 同じ対象を指す表記ゆれや重複は、最も標準的な `src` に統合してください。
- `dst` には、文脈上自然な日本語訳または一般的なカタカナ表記を入れてください。
- 出力は有効な JSON のみ。Markdown のコードフェンスや説明文は付けないでください。
```

#### 出力の使い方

1. 自動生成された `book.json` をそのまま使うこともできるが、固有名詞の訳語が空や雑なケースが多い
2. `book.md` の内容を LLM（Claude / GPT 等）に貼り付けて、添付の JSON を修正してもらう
3. LLM が返した整理済み JSON で `book.json` を上書きする
4. これ以降の翻訳コマンドが新しい用語集を自動的に使う

---

## 見積もり

### usage.ps1 / .sh

#### 概要

指定範囲の本文ファイルを翻訳した場合に **API リクエスト数と入出力 token を見積もる**スクリプト。
`epubicus translate --usage-only` を直接呼び出すため、translate スクリプトに依存しない。
**API キー不要・provider への呼び出しなし**で、コスト試算と進行可否判断に使う。

#### 使用方法

```powershell
.\scripts\usage.ps1 <入力EPUB> <開始番号> <終了番号> -Provider <provider>
```

| 引数 | 必須 | 既定値 | 説明 |
|---|---|---|---|
| `[Position 0] $InputPath` | ◎ | — | 対象 EPUB のパス |
| `[Position 1] $From` | ◎ | — | 1-based spine 番号（開始） |
| `[Position 2] $To` | ◎ | — | 1-based spine 番号（終了） |
| `-Provider` | ◎ | — | `ollama` / `openai` / `claude` / `deepseek` |
| `-Model` | | provider 別（下記） | model 名を上書き |
| `-CacheRoot` | | `.cache` | cache root を上書き |
| `-Glossary` | | 自動検出 | `<入力EPUB>` と同名 `.json` があれば自動使用 |
| `-DevBuild` | | | debug プロファイルで実行 |
| `-NoRun` | | | コマンドを表示するだけで実行しない |

##### provider 別 Model デフォルト

| Provider | Model |
|---|---|
| `ollama` | `qwen3:14b` |
| `openai` | `gpt-5-mini` |
| `claude` | `claude-sonnet-4-5` |
| `deepseek` | `deepseek-v4-flash` |

##### 典型的な呼び出し例

```powershell
.\scripts\usage.ps1 .\book.epub 9 9 -Provider deepseek
.\scripts\usage.ps1 .\book.epub 5 10 -Provider claude
.\scripts\usage.ps1 .\book.epub 9 9 -Provider deepseek -Model deepseek-v4-pro
.\scripts\usage.ps1 .\book.epub 9 9 -Provider ollama
```

#### 出力例

実行コマンド:
```powershell
.\scripts\usage.ps1 .\test\retro.epub 9 9 -Provider deepseek
```

出力:
```text
InputEpub  = D:\home\source\rust\epubicus\test\retro.epub
Provider   = deepseek
Model      = deepseek-v4-flash
CacheRoot  = D:\home\source\rust\epubicus\.cache
Range      = 9..9
Glossary   = D:\home\source\rust\epubicus\test\retro.json

Usage estimate only. No translation provider was called.
Provider: deepseek
Model: deepseek-v4-flash
Pages: 1/30 selected
Blocks: 140 total, 140 cached, 0 uncached
Source chars: 29066 total, 0 uncached
Estimated API requests: 0
Estimated tokens: input 0, output 0, total 0
Note: token counts are approximate before the API returns actual usage.
Usage estimate complete: elapsed 00:00:02
```

#### 出力の読み方

- **`Pages: 1/30 selected`** — 全 30 spine のうち 1 個（範囲指定で絞った結果）
- **`Blocks: 140 total, 140 cached, 0 uncached`** — 翻訳対象ブロック総数 / キャッシュ済み / 未キャッシュ
  - **キャッシュ済み**: 過去に翻訳した結果が既に手元にある（API 不要）
  - **未キャッシュ**: API に送る必要がある
  - 上の例は「過去に翻訳済みで全件キャッシュにある」状態
- **`Source chars: 29066 total, 0 uncached`** — 原文の文字数（未キャッシュ分が API 送信対象）
- **`Estimated API requests: 0`** — 実際に送る予定のリクエスト数
- **`Estimated tokens: input 0, output 0, total 0`** — 推定 token 消費

#### 初回見積もりとの違い

未キャッシュ状態（初回や別 provider への切り替え時）の出力例:

```text
Blocks: 8 total, 0 cached, 8 uncached
Source chars: 3594 total, 3594 uncached
Estimated API requests: 8
Estimated tokens: input 2035, output 902, total 2937
```

キャッシュが効いていない場合は `uncached` 値が `total` と一致し、`Estimated tokens` で実コストの目安が分かる。token 数は概算（実際の API usage とは多少ずれる）。

---

## 翻訳本体

`translate-<provider>.ps1` / `.sh` は provider ごとの翻訳実行スクリプト。
全体翻訳・範囲翻訳・見積もり・キャッシュ再構築すべてをこの 1 本で扱える。

すべて統一されたパラメータ体系（`-From / -To / -Model / -Concurrency / -Style / -UsageOnly / -PartialFromCache / -KindleFixedLayout / -NoKindleFixedLayout / -DevBuild / -NoRun / -ExtraArgs / -PassthroughArgs`）と統一された関数（`Invoke-EpubicusTranslate` / `New-EpubicusTranslateArgs` / `Show-EpubicusTranslateCommands`）を持つ。

### 共通の引数パターン（DeepSeek を例に）

```powershell
# 全体翻訳
.\scripts\translate-deepseek.ps1 .\book.epub

# 指定範囲のみ翻訳（試し翻訳・部分翻訳）
.\scripts\translate-deepseek.ps1 .\book.epub -From 9 -To 9

# 使用量見積もりのみ（API 未呼出）— `usage.ps1` でも代替可
.\scripts\translate-deepseek.ps1 .\book.epub -From 9 -To 9 -UsageOnly

# キャッシュから EPUB 再構築のみ（API 未呼出）— `rebuild-from-cache.ps1` でも代替可
.\scripts\translate-deepseek.ps1 .\book.epub -PartialFromCache

# Kindle 固定レイアウト指定
.\scripts\translate-deepseek.ps1 .\book.epub -KindleFixedLayout
.\scripts\translate-deepseek.ps1 .\book.epub -NoKindleFixedLayout

# epubicus translate へのオプションを追加渡し
.\scripts\translate-deepseek.ps1 .\book.epub --style novel
```

### translate-claude.ps1 / .sh

- **Provider**: Claude (Anthropic)
- **API キー**: `$env:ANTHROPIC_API_KEY`（未設定時はマスク入力プロンプト）
- **既定 Model**: `claude-sonnet-4-5`
- **既定 Concurrency**: 1
- **CacheRoot**: `<ProjectRoot>/.cache/`（共通）
- **使用例**:
  ```powershell
  $env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput
  .\scripts\translate-claude.ps1 .\book.epub
  ```

### translate-deepseek.ps1 / .sh

- **Provider**: DeepSeek
- **API キー**: `$env:DEEPSEEK_API_KEY`
- **既定 Model**: `deepseek-v4-flash`
- **既定 Concurrency**: 2
- **CacheRoot**: `<ProjectRoot>/.cache/`
- **使用例**:
  ```powershell
  $env:DEEPSEEK_API_KEY = Read-Host "DeepSeek API key" -MaskInput
  .\scripts\translate-deepseek.ps1 .\book.epub
  .\scripts\translate-deepseek.ps1 .\book.epub -From 9 -To 9
  .\scripts\translate-deepseek.ps1 .\book.epub -PartialFromCache
  ```

### translate-ollama.ps1 / .sh

- **Provider**: Ollama（ローカル）
- **API キー**: 不要
- **既定 Model**: `qwen3:14b`
- **既定 Concurrency**: 3
- **追加環境変数**: `EPUBICUS_OLLAMA_HOST=http://localhost:11434`、`EPUBICUS_NUM_CTX=8192`
- **CacheRoot**: `<ProjectRoot>/.cache/`
- **使用例**:
  ```powershell
  .\scripts\translate-ollama.ps1 .\book.epub
  ```

### translate-openai.ps1 / .sh

- **Provider**: OpenAI
- **API キー**: `$env:OPENAI_API_KEY`
- **既定 Model**: `gpt-5-mini`
- **既定 Concurrency**: 4
- **CacheRoot**: `<ProjectRoot>/.cache/`
- **使用例**:
  ```powershell
  $env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
  .\scripts\translate-openai.ps1 .\book.epub
  ```

### translate-openai-batch.ps1 / .sh

- **Provider**: OpenAI Batch API（別経路、`epubicus batch run` を呼ぶ）
- **API キー**: `$env:OPENAI_API_KEY`
- **既定 Model**: `gpt-5-mini`
- **CacheRoot**: `<ProjectRoot>/.cache/`
- **特徴**: バッチ送信、ローカル fallback、待機・取得・取り込みを管理
- **追加引数**: `-LocalModel`、`-LocalLimit`、`-PollSecs`、`-NoLocalFallback`、`-NoWait`
- **使用例**:
  ```powershell
  $env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
  .\scripts\translate-openai-batch.ps1 .\book.epub
  ```

---

## リカバリ

未翻訳ブロックの再翻訳・救済処理。入力ソースと処理経路で 2 系統に分かれる。

| 系統 | スクリプト | 入力ソース | API 呼出 |
|---|---|---|---|
| 通常リカバリ | `recover-from-cache.ps1` | `recovery.jsonl`（前回 translate の失敗ログ）または手動 JSON | 通常あり、`-Manual` 指定時はなし |
| Batch 専用 | `batch-recover-local.ps1` | Batch ワークスペース | あり |

### recover-from-cache.ps1 / .sh（通常リカバリ）

#### 概要

translate コマンドが残した `recovery.jsonl` を読み、未翻訳 / 検証失敗ブロックを provider に再送して翻訳キャッシュへ書き戻す。EPUB の再構築まで自動で行う（`-NoRebuild` で抑制可）。

`-Manual <json>` を指定すると provider を呼ばず、手動翻訳 JSON をキャッシュに直接書き込む（validator チェックなし）。

#### 使用方法

| 引数 | 必須 | 既定値 | 説明 |
|---|---|---|---|
| `[Position 0] $InputPath` | ◎ | — | 対象 EPUB |
| `-Provider <ollama\|openai\|claude\|deepseek>` | | `ollama` | provider |
| `-Model <name>` | | provider 既定 | model 名 |
| `-Concurrency <n>` | | 0（provider 既定） | 並列数 |
| `-CacheRoot <path>` | | `.cache` | cache root |
| `-Glossary <path>` | | 自動検出 | 用語集 |
| `-Manual <path>` | | — | 手動翻訳 JSON（指定時 API 未呼出） |
| `-Limit <n>` / `-Page <n>` / `-Block <n>` / `-Reason <list>` | | | 対象フィルタ |
| `-Output <path>` | | `<input>_jp.epub` | 出力 EPUB |
| `-NoRebuild` | | | recover 後に EPUB 再構築をスキップ |
| `-List` | | | 対象一覧を表示するだけ |
| `-KindleFixedLayout` / `-NoKindleFixedLayout` | | | Kindle 固定レイアウト指定 |
| `-DevBuild` / `-NoRun` | | | |

#### 使用例

```powershell
# DeepSeek で再翻訳
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek

# 別 provider で追い翻訳（cache root は同じ .cache/ なので自動で正しい cache_key を引く）
.\scripts\recover-from-cache.ps1 .\book.epub -Provider openai

# 対象一覧の確認
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -List

# 手動翻訳 JSON で API を呼ばずキャッシュ直書き
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Manual .\book.manual.json

# 上位モデル（pro）で再試行
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Model deepseek-v4-pro
```

#### 手動 JSON 形式

```json
{
  "entries": [
    {
      "page": 4,
      "block": 150,
      "href": "chapter2.xhtml",
      "text": "翻訳後のテキスト"
    }
  ]
}
```

詳細は [detailed-examples.ja.md](detailed-examples.ja.md) 参照。

### batch-recover-local.ps1 / .sh（Batch 専用）

#### 概要

OpenAI Batch API ワークフロー専用。**未完了の Batch ジョブをローカルで救済し、EPUB を再構築**する複合スクリプト。
通常リカバリとは入力ソース（Batch ワークスペース内の状態）と実行ステップが大きく異なるため、独立スクリプトとして提供。

#### 実行ステップ

`batch health → fetch → import → reroute-local → translate-local → verify → batch health → rebuild` の 8 ステップを順次実行。

| 引数 | 既定 | 説明 |
|---|---|---|
| `[Position 0] $InputPath` | — | 対象 EPUB |
| `-CacheRoot` | `.cache` | Batch ワークスペース |
| `-BatchModel` | `gpt-5-mini` | Batch 側の model |
| `-LocalProvider <ollama\|openai\|claude>` | `ollama` | ローカル fallback provider |
| `-LocalModel` | `qwen3:14b` | ローカル fallback model |
| `-Limit <n>` | 100 | translate-local の処理上限 |
| `-Priority <page-order\|failed-first\|hard-first\|short-first\|oldest-first>` | `short-first` | 優先順位 |
| `-SkipFetchImport` / `-SkipLocal` / `-SkipRebuild` / `-StrictVerify` | | 各ステップ制御 |

#### 使用例

```powershell
.\scripts\batch-recover-local.ps1 .\book.epub
.\scripts\batch-recover-local.ps1 .\book.epub -LocalModel qwen3:14b -Limit 100
.\scripts\batch-recover-local.ps1 .\book.epub -SkipFetchImport
.\scripts\batch-recover-local.ps1 .\book.epub -LocalProvider claude -LocalModel claude-sonnet-4-5
```

---

## スキャン

### scan-and-recover.ps1 / .sh

#### 概要

完成した出力 EPUB を再走査し「翻訳済みのように見えるが実は怪しいブロック」を検出する。
translate コマンド時の validator が見逃した「unchanged_source」「英語残り」「placeholder 消失」などを後付けで救済する補完ツール。

`-ScanOnly` で検出のみ、無指定で検出 + recover + 再構築まで実行。

#### 使用方法

| 引数 | 必須 | 既定値 | 説明 |
|---|---|---|---|
| `[Position 0] $InputPath` | ◎ | — | 元の入力 EPUB |
| `[Position 1] $OutputPath` | | `<input>_jp.epub` | 翻訳済み EPUB |
| `-Provider <ollama\|openai\|claude\|deepseek>` | ◎ | — | provider（必須） |
| `-Model` | | provider 別 | model 名 |
| `-CacheRoot` | | `.cache` | cache root |
| `-Glossary` | | 自動検出 | 用語集 |
| `-Limit <n>` | | | 検出上限 |
| `-ScanOnly` | | | 検出のみ（API 未呼出） |
| `-NoRebuild` | | | recover 後の再構築をスキップ |
| `-KindleFixedLayout` / `-NoKindleFixedLayout` | | | Kindle 固定レイアウト指定 |
| `-NoRun` | | | コマンド表示のみ |

#### 使用例

```powershell
# 検出のみ(API 未呼出)
.\scripts\scan-and-recover.ps1 .\book.epub -Provider deepseek -ScanOnly

# 検出 + recover + 再構築
.\scripts\scan-and-recover.ps1 .\book.epub -Provider deepseek

# 別 provider で
.\scripts\scan-and-recover.ps1 .\book.epub -Provider claude
```

#### 通常リカバリとの違い

| | recover-from-cache | scan-and-recover |
|---|---|---|
| 検出タイミング | translate 中に validator がエラー判定 | translate 完了後に出力 EPUB を再検査 |
| 検出元 | translate が書いた `recovery.jsonl` | 入力 EPUB と出力 EPUB の差分 |
| 主な対象 | provider が API でエラー / placeholder 不一致 等 | validator を通って書き込まれたが実際は怪しい翻訳 |

---

## 再構築

### rebuild-from-cache.ps1 / .sh

#### 概要

キャッシュにある翻訳結果から **API を呼ばずに EPUB を再生成**する。
provider/model は **キャッシュから自動検出**するため、`-Provider` を指定する必要がない（手動指定も可能）。

#### 使用方法

```powershell
.\scripts\rebuild-from-cache.ps1 <入力EPUB> [auto|fixed|reflow]
```

| 引数 | 既定値 | 説明 |
|---|---|---|
| `[Position 0] $InputPath` | — | 対象 EPUB のパス（必須） |
| `[Position 1] $Layout` | `auto` | Kindle 固定レイアウト指定（`auto` / `fixed` / `reflow`） |
| `-Provider` | 自動検出 | キャッシュから検出される最頻値を上書き |
| `-Model` | 自動検出 | 同上 |
| `-CacheRoot` | `.cache` | cache root を上書き |
| `-Glossary` | 自動検出 | EPUB と同名 `.json` を自動検出 |
| `-DevBuild` / `-NoRun` | | |

#### 自動検出ロジック

1. 入力 EPUB の SHA-256 先頭 16 バイト（32 hex）を計算
2. `<.cache>/<input_hash>/translations.jsonl` を探す
3. 各エントリの `(provider, model)` 組合せを集計し、最頻値を採用
4. 該当ファイル無し / エントリ無しの場合はエラー（`-Provider` / `-Model` 明示指定を促す）

#### 使用例

```powershell
# キャッシュから自動検出して再構築
.\scripts\rebuild-from-cache.ps1 .\book.epub

# Kindle 固定レイアウト強制
.\scripts\rebuild-from-cache.ps1 .\book.epub fixed

# Kindle 固定レイアウト抑制
.\scripts\rebuild-from-cache.ps1 .\book.epub reflow

# 複数 provider 混在時に明示指定
.\scripts\rebuild-from-cache.ps1 .\book.epub -Provider claude
```

---

## キャッシュクリア

### clear-all-caches.ps1 / .sh

#### 概要

OS 既定 + プロジェクト内の `.cache/`（および従来の provider 別 `.deepseek-cache/` 等）を一括削除する。

#### 使用方法

| 引数 | 説明 |
|---|---|
| `-DryRun` | 削除予定のディレクトリを表示するだけ |
| `-Yes` | 確認なしで削除 |
| `-Include <list>` | 追加で削除するパス |

#### 使用例

```powershell
.\scripts\clear-all-caches.ps1 -DryRun
.\scripts\clear-all-caches.ps1 -Yes
.\scripts\clear-all-caches.ps1 -Include .\some-other-cache -Yes
```

- 実行中の epubicus プロセスは停止しない
- ロックされたファイルは報告のみで残る

---

## 関連ドキュメント

- [script-cleanup-plan.ja.md](script-cleanup-plan.ja.md) — スクリプト整理の計画と進捗
- [translation-workflow.ja.md](translation-workflow.ja.md) — 翻訳ワークフロー全体像
- [operation-guide.ja.md](operation-guide.ja.md) — 日常運用ガイド
- [detailed-examples.ja.md](detailed-examples.ja.md) — 詳細な実行例
