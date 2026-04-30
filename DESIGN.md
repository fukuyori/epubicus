# epubicus — 設計書

英語 EPUB を体裁を保持したまま日本語に翻訳するツール。
翻訳エンジンはローカルの Ollama を使用する。

---

## 1. 目的とスコープ

### 1.1 目的
- 英語の EPUB（小説・技術書・エッセイ等）を、レイアウト・書式・ナビゲーション・画像・脚注を保持したまま日本語化する。
- ローカル LLM（Ollama）で完結させ、外部 API 課金や情報漏洩を避ける。

### 1.2 スコープ内
- EPUB 2.0.1 / EPUB 3.x の入力対応（実用上は 3.x を主対象）。
- XHTML 本文の翻訳（インライン書式、リンク、強調、ルビ等の保持）。
- 章タイトル・目次・メタデータ（title, description）の翻訳。
- 翻訳結果のキャッシュと再開（途中中断後の再実行）。
- 用語集（glossary）による固有名詞の訳ぶれ防止。

### 1.3 スコープ外（v1）
- 画像内テキスト（OCR）、SVG 内テキスト、埋め込みフォント差し替え。
- 縦書き化、ルビ自動付与、漢字レベル調整。
- DRM 付き EPUB の解除。
- GUI（v2 以降に検討。v1 は CLI のみ）。

---

## 2. 全体アーキテクチャ

```
┌─────────────┐   ┌──────────┐   ┌──────────────┐   ┌─────────────┐
│  EPUB (en)  │──>│ Unpacker │──>│ XHTML Parser │──>│  Extractor  │
└─────────────┘   └──────────┘   └──────────────┘   └──────┬──────┘
                                                            │ segments
                                                            ▼
                                                    ┌──────────────┐
                                                    │   Chunker    │
                                                    └──────┬───────┘
                                                           │ batches
                                                           ▼
┌─────────────┐   ┌──────────┐   ┌──────────────┐   ┌──────────────┐
│  EPUB (ja)  │<──│  Packer  │<──│  Reassembler │<──│Ollama Client │
└─────────────┘   └──────────┘   └──────┬───────┘   └──────┬───────┘
                                        ▲                  │
                                        │                  │
                                        │  ┌────────────┐  │
                                        └──│   Cache    │<─┘
                                           │  (sled)    │
                                           └────────────┘
                                                  ▲
                                           ┌──────┴──────┐
                                           │  Glossary   │
                                           └─────────────┘
```

### 2.1 主要コンポーネント

| コンポーネント | 責務 |
|--|--|
| Unpacker | EPUB（ZIP）を一時ディレクトリに展開、`META-INF/container.xml` から OPF を特定 |
| OPF Reader | manifest と spine を解析、対象 XHTML 一覧を生成 |
| XHTML Parser | XHTML をパース（DOM ツリー化） |
| Extractor | DOM を走査し、翻訳対象テキストを「セグメント」として抽出。インラインタグはプレースホルダ化 |
| Chunker | セグメント群を LLM に渡せるサイズのバッチに分割 |
| Glossary Builder | 全本文をスキャンし、固有名詞候補を抽出して訳を確定（事前パス） |
| Ollama Client | Ollama HTTP API へリクエスト、リトライとタイムアウト管理 |
| Cache | 入力ハッシュ → 訳文 のローカル KV キャッシュ |
| Reassembler | 翻訳済みセグメントをプレースホルダ復元しつつ XHTML に書き戻す |
| Packer | OPF を更新（言語属性、タイトル等）し、EPUB として再パック |

---

## 3. EPUB 取り扱い

### 3.1 入力ファイル種別の分類

OPF の `manifest` を走査し、`media-type` と spine 包含状況で分類する。

| media-type | 扱い |
|--|--|
| `application/xhtml+xml` | 翻訳対象（spine 内） |
| `application/xhtml+xml`（spine 外） | 通常コピー（カバーページ等は別判定） |
| `text/css` | コピー（フォントスタックに日本語フォールバックを追加） |
| `application/x-dtbncx+xml`（NCX） | 目次テキストを翻訳 |
| `application/oebps-package+xml`（OPF） | メタデータ翻訳して再生成 |
| `image/*`, `font/*`, `application/font-*` | コピー |
| `application/x-dtbook+xml`, その他 | コピー |

### 3.2 パッケージメタデータ更新
- `<dc:language>` を `ja` に書き換え。
- `<dc:title>` を翻訳（オリジナルは `<dc:title id="orig">` として保持しても良い）。
- `<dc:contributor id="epubicus-translator">epubicus (model: <model_name>)</dc:contributor>` と `meta refines` の role を追加。
- `<meta property="dcterms:modified">` を更新。

### 3.3 ナビゲーション
- EPUB3 `nav.xhtml`（`epub:type="toc"`）内のリンクテキストを翻訳。
- EPUB2 NCX の `<navLabel><text>` を翻訳。
- 両方存在する場合は両方処理。

---

## 4. XHTML 翻訳の核心戦略

ここがツールの品質を決める最重要パート。

### 4.1 セグメント抽出単位

**ブロック要素単位**で翻訳する。ブロックの定義:
- `p`, `h1`〜`h6`, `li`, `blockquote`, `figcaption`, `dt`, `dd`, `caption`, `td`, `th`, `summary`
- `div` のうちテキストを直接含むもの（子がブロックなら子に降りる）

ブロック単位の理由:
- 1 文だけだと文脈不足で日本語訳の質が落ちる。
- 章丸ごとだと LLM のコンテキスト長を超え、欠落リスク。
- ブロック単位なら原文と訳文の対応が 1:1 で取れ、復元失敗時のリカバリが容易。

### 4.2 インラインタグのプレースホルダ化

ブロック内に `<em>`, `<strong>`, `<a>`, `<span>`, `<code>`, `<i>`, `<b>`, `<sup>`, `<sub>`, `<ruby>`, `<br>` 等が混在する。日本語は語順が大きく変わるため、タグを LLM にそのまま渡すと壊れやすい。

**方式: タグ → 番号付きプレースホルダ**

```xhtml
<p>This is <em>very</em> <a href="#n1">important</a>.</p>
```

を、抽出時に:

```text
This is ⟦E1⟧very⟦/E1⟧ ⟦E2⟧important⟦/E2⟧.
```

に変換し（タグ ID と属性は別途マップに退避）、LLM に「⟦…⟧ マーカは順序が変わってもよいが必ず元の数だけ残す」ルールで翻訳依頼。

復元時:

```text
これは⟦E2⟧重要⟦/E2⟧で⟦E1⟧とても⟦/E1⟧。
   ↓
<p>これは<a href="#n1">重要</a>で<em>とても</em>。</p>
```

**プレースホルダ仕様:**
- 開始: `⟦E{n}⟧`、終了: `⟦/E{n}⟧`、空要素（`<br/>`, `<img/>`）: `⟦S{n}⟧`
- `n` はブロック内連番。
- `⟦` `⟧` は U+27E6 / U+27E7。通常の文章にまず出現せず、トークナイザで分割されにくい記号を選択。

**バリデーション:**
1. 訳文中のプレースホルダ集合が原文と一致するか（数・ID）。
2. 開始・終了の対応が取れているか。
3. 不一致なら `temperature` を下げて 1 回再試行。再試行も失敗ならフォールバック（後述）。

**フォールバック:**
- インラインタグを全て剥がした平文で再翻訳し、訳文をブロックの単一テキストノードとして配置。書式は失われるが本文は欠損しない。
- ログに warning を残し、最終レポートで該当箇所を一覧化。

### 4.3 翻訳しない要素

`skip_tags` で指定（デフォルト）:
- `code`, `pre`, `kbd`, `samp`, `var`, `tt`, `script`, `style`
- `epub:type="pagebreak"` を持つ要素
- `lang` 属性が `en` 以外（多言語混在原書対策）
- 数式 (`math`, MathML)
- `aria-hidden="true"` の装飾要素

ただし `<code>` 内に英文コメント等があり翻訳すべきケースもあるため、`--translate-code` オプションで上書き可能にする。

### 4.4 特殊ケース

| ケース | 扱い |
|--|--|
| 脚注（`<aside epub:type="footnote">`） | 通常翻訳。リンクテキスト（脚注番号）は訳さず維持 |
| 詩・対話 | ブロック単位で訳すが、`<br/>` を `⟦S⟧` で保持し改行を保つ |
| テーブル | セル単位で翻訳 |
| 画像の alt | 翻訳 |
| 章タイトル | 専用プロンプト（短く・体言止め志向） |
| 目次 | 翻訳済みタイトルを参照（重複翻訳しない） |

---

## 5. Ollama 連携

### 5.1 推奨モデル

| モデル | サイズ | 日本語品質 | コンテキスト | 備考 |
|--|--|--|--|--|
| `qwen2.5:14b` | 9 GB | ◎ | 32K | デフォルト推奨。日本語が最も安定 |
| `qwen2.5:7b` | 4.7 GB | ○ | 32K | 軽量機向け |
| `gemma3:12b` | 8 GB | ○ | 128K | 長文章向き |
| `aya-expanse:8b` | 5 GB | ◎ | 8K | 多言語特化 |
| `command-r:35b` | 20 GB | ◎ | 128K | 高品質、要 GPU 24GB+ |

設定で切替可能、デフォルトは `qwen2.5:14b`。

### 5.2 API

Ollama の REST API を使用:

```
POST http://localhost:11434/api/chat
Content-Type: application/json

{
  "model": "qwen2.5:14b",
  "messages": [
    {"role": "system", "content": "<system_prompt>"},
    {"role": "user", "content": "<source_block>"}
  ],
  "stream": false,
  "options": {
    "temperature": 0.3,
    "top_p": 0.9,
    "num_ctx": 8192,
    "num_predict": 2048,
    "seed": 42
  }
}
```

レスポンス: `{message: {content: "..."}}`

### 5.3 プロンプト設計

**システムプロンプト（テンプレート）:**

`{style_block}` は §5.4 の文体プリセットから組み立てて差し込む。

```
あなたは英日翻訳の専門家です。出版物として通用する自然で読みやすい日本語に翻訳してください。

【絶対遵守ルール】
1. 入力中の ⟦…⟧ で囲まれたマーカ（例: ⟦E1⟧, ⟦/E1⟧, ⟦S2⟧）は、形を一切変えずに訳文に含めてください。
2. マーカの順序は日本語として自然になるように入れ替えて構いませんが、原文に現れた全てのマーカを過不足なく残してください。
3. マーカの中身（タグ ID 番号）を改変・追加・削除しないでください。
4. 翻訳のみを出力し、説明・前置き・括弧書きの注釈を一切付けないでください。
5. 用語集が与えられた場合、その訳語を必ず使用してください。

{style_block}
```

**ユーザプロンプト構造:**
```
<glossary>
- Frodo Baggins => フロド・バギンズ
- Shire => ホビット庄
</glossary>

<context>
（直前ブロックの訳文 1〜2 個を参考として提示。文脈連続性のため）
</context>

<source>
The hobbits hurried through ⟦E1⟧the Shire⟦/E1⟧ at dusk.
</source>
```

期待出力:
```
ホビットたちは夕暮れの⟦E1⟧ホビット庄⟦/E1⟧を急いで進んだ。
```

### 5.4 文体プリセット

`--style` または設定ファイル `[style]` で指定。プロンプトに差し込まれる文体ブロックを切り替える。

**プリセット一覧:**

| ID | 用途 | 地の文 | 会話 | 章タイトル | その他特徴 |
|--|--|--|--|--|--|
| `novel` | 小説（一般文芸） | である調 | 話者に合わせた口語 | 簡潔・体言止め可 | 比喩は意訳寄り |
| `novel-polite` | 児童向け・ライト文芸 | です・ます調 | 話者に合わせた口語 | 簡潔 | 漢字を控えめに |
| `tech` | 技術書・リファレンス | である調 | 該当なし | 命令形可（手順章で「〜する」） | 専門用語はカタカナ優先、カッコ内に原語併記を許可 |
| `essay` | エッセイ・ノンフィクション | である調 | 該当なし | 体言止め | 一人称は原文を尊重（I → 私／僕／俺は文脈判定） |
| `academic` | 学術書・論文系 | である調（硬め） | 該当なし | 名詞句 | 受動態を許容、訳語の厳密さ優先 |
| `business` | ビジネス書 | です・ます調 | 該当なし | 体言止め | 専門用語は一般化しすぎない |
| `custom` | 設定ファイルで自由記述 | — | — | — | `[style.custom]` の文字列をそのまま挿入 |

**プリセット → プロンプトブロック例（`novel`）:**

```text
【文体】
- 地の文: である調（だ・である）。一文を長くしすぎず、句読点でリズムを整えてください。
- 会話文（クォート内）: 話者の人物像にふさわしい口語。砕けた語尾も可。
- 章タイトル: 簡潔。体言止めを基本とし、必要に応じて短い動詞句も可。
- 比喩・修辞表現は意味が通る範囲で日本語として自然な表現に置き換えてください。
```

**プリセット → プロンプトブロック例（`tech`）:**

```text
【文体】
- 地の文: である調。事実を淡々と述べる調子。
- 専門用語: 一般に流通しているカタカナ訳を優先（例: implementation → 実装、callback → コールバック）。初出または訳が定着していない用語は「日本語訳（原語）」の形式で書いてください（例: 投機実行（speculative execution））。
- コードや識別子: 翻訳しないでください（プレースホルダで保護されている場合はそのまま）。
- 章タイトル: 名詞句または「〜する」形の動詞句。
- 手順・箇条書きの指示: 「〜してください」ではなく「〜する」で統一。
```

**自動判定（オプション）:**
- `--style auto` 指定時、書籍冒頭 5,000 単語をサンプリングして判定:
  - dialogue ratio（クォート行の比率） > 15% → `novel`
  - コード／`<pre>` 比率 > 5% → `tech`
  - それ以外 → `essay`
- 判定結果はログに出力。確信度が低い場合は `essay` にフォールバックし warning。
- 自動判定は v0.3 以降の機能。v0.1 は明示指定必須、未指定なら `essay` をデフォルト。

**設定ファイルでの個別指定（書籍ごと）:**

書籍ごとに `epubicus.toml` を置く運用、または `--config <path>` で指定する運用を想定。設定ファイル側の `[style]` がコマンドライン `--style` より弱く、`--style` で上書き可能（CLI > config > デフォルト）。

### 5.5 バッチング戦略

- 1 リクエスト = 1 ブロック（v1 のシンプル版）。
- v1.1 で「複数ブロックを区切り記号で連結して 1 リクエスト」方式も検討（スループット向上）。
- リクエスト並列度はデフォルト 1（ローカル GPU は 1 つしかなく逐次の方が速い）。CPU 推論時は並列化しない。

### 5.6 リトライ・タイムアウト

- HTTP タイムアウト: 60 秒（短文）〜 300 秒（長文）。`num_predict * 0.2 秒 + 30 秒` で動的算出。
- リトライ: ネットワークエラーは 3 回まで指数バックオフ。
- バリデーション失敗（プレースホルダ不一致）は temperature を 0.3 → 0.1 → 0 と下げて最大 3 回。

---

## 6. 用語集 (Glossary)

固有名詞の訳ぶれ（Frodo が「フロド」だったり「フロドー」になる）を防ぐ仕組み。

### 6.1 構築フロー（事前パス）

```
1. 全 XHTML から固有名詞候補を抽出
   - 大文字始まり連語の頻度集計
   - 既知ストップワードを除外
   - 出現回数 N 回以上を候補に
2. Ollama に candidates をまとめて投げ「日本語訳を JSON で返せ」
   - format: "json" モードを使用
3. ユーザに編集機会を与える（--review-glossary フラグで対話モード）
4. glossary.json として保存
```

### 6.2 形式

```json
{
  "model": "qwen2.5:14b",
  "source_lang": "en",
  "target_lang": "ja",
  "entries": [
    {"src": "Frodo Baggins", "dst": "フロド・バギンズ", "kind": "person"},
    {"src": "Shire", "dst": "ホビット庄", "kind": "place"},
    {"src": "Bag End", "dst": "袋小路屋敷", "kind": "place"}
  ]
}
```

### 6.3 翻訳時の使用
- 各リクエストに「現在のブロックに登場する用語のみ」を抽出して付与（プロンプト肥大化を回避）。
- ブロック内テキストに対し substring 検索でヒットした用語だけ含める。

---

## 7. キャッシュ

### 7.1 目的
- 中断後の再開（数千ブロックある本で 8 割完了時点でクラッシュ → やり直し回避）
- パラメータ変更時の挙動を予測可能にする
- 異なる EPUB を扱う際の独立性確保

### 7.2 識別方法

入力 EPUB ファイル全体の SHA-256 を取り、その先頭 16 バイト hex（32 文字）を **ラン ID** とする。

```text
input_hash = hex(sha256(input.epub bytes)[:16])
```

設計上の含意:

- 異なる EPUB は確実に別 ID（衝突確率 = 2^-128 で無視できる）
- 同じ EPUB の中断・再実行は同じ ID → 自動再開
- バイト単位で差があれば別扱い（Calibre 等での再保存・1 文字修正でも別キャッシュ）。**割り切り**: 起動時のオーバーヘッドや誤判定の複雑さを避けるため、コンテンツ指紋の併用はしない（必要になれば schema_version を上げて拡張）

### 7.3 ストレージ構造

```text
%LOCALAPPDATA%\epubicus\cache\         (Windows)
~/.cache/epubicus/                      (Linux/macOS)
└── <input_hash>/
    ├── manifest.json        メタ情報・進捗
    └── translations.jsonl   セグメント単位の翻訳結果（append-only）
```

ルートディレクトリは `[cache].dir` で上書き可能。デフォルトは OS のキャッシュ規約に従う。

### 7.4 manifest.json

```json
{
  "schema_version": 1,
  "input": {
    "sha256": "a1b2c3d4e5f60718",
    "path_when_started": "D:\\books\\alice.epub",
    "size_bytes": 312456,
    "mtime": "2026-04-28T10:00:00Z"
  },
  "params": {
    "model": "qwen2.5:14b",
    "prompt_version": "v1",
    "style_id": "novel",
    "style_overrides_sha": "0000000000000000",
    "glossary_sha": "abc1234567890def"
  },
  "progress": {
    "total_segments": 4302,
    "completed": 3104,
    "completed_ids": ["c01:0", "c01:1", "..."],
    "fallback_count": 2
  },
  "timestamps": {
    "started_at": "2026-04-28T20:00:00Z",
    "last_updated_at": "2026-04-29T03:14:00Z"
  },
  "last_output_path": "D:\\books\\alice.ja.epub"
}
```

manifest は N セグメント完了ごと（デフォルト 50 件）に同期保存。中断時の損失上限はその件数まで。

### 7.5 translations.jsonl

1 行 1 セグメント、append-only:

```json
{"segment_id":"c01:0","key":"<sha16>","translated":"...","fallback":false,"at":"2026-04-28T20:01:13Z"}
```

- `key = sha256(model || prompt_version || style_id || style_overrides_sha || glossary_subset_sha || source_with_placeholders)[:16].hex()`
- 起動時に `manifest.progress.completed_ids` を読んで O(1) ルックアップ表を作る。jsonl は監査・復旧用。

### 7.6 起動時のフロー

```text
1. 入力 EPUB を読み、SHA-256 を計算 → input_hash
2. cache/<input_hash>/ を探す
   ├─ 存在する: manifest.json を読み込む
   │   ├─ params が今回の指定と一致
   │   │   → 自動再開
   │   │     "Resuming 3104/4302 from previous run (started 2 days ago)"
   │   └─ 不一致（model/style/glossary が変更されている）
   │       → 対話プロンプト
   │         [r] resume: 残りだけ新パラメータで翻訳（混在を許容）
   │         [c] clear : このランを削除して最初から
   │         [a] abort : 中断
   └─ 存在しない: 別 EPUB or 初回 → 新規ディレクトリで開始
3. 異なる EPUB は別ハッシュ別ディレクトリなので 100% 分離
```

### 7.7 完了時の自動削除

成功完了の定義（**通常の `translate` 実行のみ**が対象）:
- `progress.completed == progress.total_segments`
- 出力 EPUB が最終パスへ atomic move 完了
- パックエラーなし

挙動:
- 上記条件を満たした直後、`cache/<input_hash>/` を **自動削除**
- レポート JSON（フォールバック詳細等）は出力ディレクトリ側に `<output>.report.json` として別途保存されるため、キャッシュ削除で情報は失われない
- `--keep-cache` フラグで明示保持可能（デフォルト無効、デバッグ・調査用）

**自動削除が発火しないケース:**
- `--partial-from-cache` 実行時（§7.10 参照）。キャッシュは読み取りのみで進捗更新もしないため、完了条件を満たさない
- `--dry-run`（そもそも何も書き込まない）
- パック失敗

### 7.8 削除タイミング一覧

| ラン状態 | キャッシュ | 削除タイミング |
|--|--|--|
| 進行中 | 保持 | なし |
| 中断（クラッシュ・Ctrl-C） | 保持 | 手動 `cache clear --hash` または `cache prune --older-than` |
| 成功完了（通常 translate） | 削除 | 完了直後・自動 |
| パック失敗 | 保持 | 中断扱い。再実行で続きから |
| `--keep-cache` 指定での完了 | 保持 | 手動削除 |
| `--partial-from-cache` 実行後 | **保持・無変更** | 手動削除（読み取り専用なので自動削除なし） |
| `--dry-run` 実行後 | **保持・無変更** | 手動削除（読み書きなし） |

### 7.9 蓄積時のヒント

起動時、未完ランが多数（デフォルト > 5 件）または総容量が大きい（デフォルト > 500 MB）場合、1 行のみ案内する:

```text
Note: 12 cached runs (~640 MB). Run `epubicus cache prune --older-than 30d` to clean up.
```

しきい値は `[cache].hint_threshold_mb` / `hint_threshold_count` で調整可能。

### 7.10 無効化と例外操作

- `--no-cache`: 読み書き両方無効（既存キャッシュも無視、新規書き込みもしない）
- `--clear-cache`: **この入力 EPUB のキャッシュ**を削除して新規開始（`cache clear --hash <自動算出>` の糖衣構文）
- `--partial-from-cache`: provider を呼ばず、cache hit のブロックだけ訳文に置換、miss は原文を維持して EPUB を作成（プレビュー・部分配布用）。**キャッシュは読み取りのみで一切変更されない**（manifest 更新も jsonl 追記もしない）。後で通常 `translate` を実行すると残りの未訳ブロックがそこから埋まる
- `--keep-cache`: 完了後の自動削除を抑止

---

## 8. CLI 仕様

```text
epubicus translate <INPUT.epub> [-o OUTPUT.epub] [OPTIONS]
epubicus glossary  <INPUT.epub> [-o glossary.json] [OPTIONS]
epubicus inspect   <INPUT.epub>                       # 構造解析のみ
epubicus models                                       # Ollama にあるモデル一覧
epubicus cache list                                   # 全ラン一覧
epubicus cache show <hash|input.epub>                 # 詳細
epubicus cache resume <hash|input.epub>               # 中断ランを再開
epubicus cache prune --older-than <DAYS>              # 古い未完ランを削除
epubicus cache clear --hash <HASH>                    # 単一削除
epubicus cache clear --all [--yes] [--dry-run]        # 全削除（要確認）
```

### 8.1 translate サブコマンド

| オプション | デフォルト | 説明 |
|--|--|--|
| `-o, --output PATH` | `<input>.ja.epub` | 出力ファイル |
| `-m, --model NAME` | `qwen2.5:14b` | Ollama モデル名 |
| `--ollama-host URL` | `http://localhost:11434` | Ollama エンドポイント |
| `--glossary PATH` | なし | 用語集 JSON |
| `--build-glossary` | false | 翻訳前に用語集を自動生成 |
| `--review-glossary` | false | 用語集を対話レビュー |
| `--temperature F` | 0.3 | サンプリング温度 |
| `--num-ctx N` | 8192 | コンテキスト長 |
| `--concurrency N` | 1 | XHTML ファイル単位で未キャッシュ provider リクエストを最大 N 件並列実行 |
| `--style ID` | `essay`（v0.1）／`auto`（v0.3 以降） | 文体プリセット（§5.4）。`novel` / `novel-polite` / `tech` / `essay` / `academic` / `business` / `custom` / `auto` |
| `--skip-chapters RANGES` | なし | `1,3,5-7` 形式で章スキップ |
| `--only-chapters RANGES` | なし | 指定章のみ翻訳（デバッグ用） |
| `--translate-code` | false | `<code>` も翻訳 |
| `--keep-original` | false | 訳文の後ろに原文を併記（**デフォルト無効**） |
| `--no-cache` | false | キャッシュ読み書き両方無効 |
| `--clear-cache` | false | この入力 EPUB のキャッシュを削除して新規開始 |
| `--keep-cache` | false | 完了後もキャッシュを保持（デバッグ用） |
| `--partial-from-cache` | false | キャッシュ済み訳文だけを使い、未訳ブロックは原文維持（LLM 呼び出しなし） |
| `--dry-run` | false | LLM を呼ばず統計のみ表示 |
| `-v, --verbose` | false | 詳細ログ |
| `-c, --config PATH` | `./epubicus.toml` 等 | 設定ファイル |

実装済み CLI では、主要な共通オプションを `EPUBICUS_PROVIDER` / `EPUBICUS_MODEL` / `EPUBICUS_CONCURRENCY` などの環境変数からも読める。CLI 引数を指定した場合は CLI 引数を優先する。

### 8.1.1 cache サブコマンド

| サブコマンド | 説明 |
|--|--|
| `cache list` | 全ラン一覧を表示。列: hash / 入力ファイル / 進捗 (%) / 最終更新 / サイズ |
| `cache show <hash\|input.epub>` | 指定ランの manifest 内容を整形表示 |
| `cache resume <hash\|input.epub>` | 中断ランを明示的に再開（通常は translate コマンドで自動再開される） |
| `cache prune --older-than <DAYS>` | 最終更新から N 日以上経過した未完ランを削除（要確認） |
| `cache clear --hash <HASH>` | 指定ハッシュのキャッシュを削除（確認なし。明示的な指定なので安全） |
| `cache clear --all` | 全キャッシュを削除。**`yes` を全文入力して確認**。`--yes` で省略、`--dry-run` でプレビュー |

`cache clear --all` の対話例:

```text
$ epubicus cache clear --all
About to delete all 7 cached runs (total 412 MB):
  - alice.epub          78% done   started 3 days ago    52 MB
  - moby_dick.epub      12% done   started 8 hours ago   18 MB
  - 1984.epub           45% done   started 2 weeks ago   31 MB
  - sherlock.epub       91% done   started 1 month ago   84 MB
  ...
  (output EPUB files are NOT touched)

Type 'yes' to confirm: _
```

### 8.2 進捗表示

`indicatif` で:
```
Translating  [chapter 7/24] ███████░░░░░░  29% │ 1,243/4,302 blocks │ ETA 18m
Current: "It was a dark and stormy night..."
```

### 8.3 終了レポート

```
Done. Output: book.ja.epub (3.4 MB)
Translated:    4,302 blocks
Cache hits:      0 (fresh run)
Tag fallbacks:   3   (see book.ja.epub.report.json)
Skipped:         12 (code blocks)
Elapsed:         2h 14m
Model:           qwen2.5:14b
```

`*.report.json` にフォールバック箇所の詳細（ファイル、ブロック ID、原文、訳文）を出力。

---

## 9. 設定ファイル (`epubicus.toml`)

```toml
[ollama]
host = "http://localhost:11434"
model = "qwen2.5:14b"
temperature = 0.3
top_p = 0.9
num_ctx = 8192
num_predict = 2048
seed = 42
timeout_base_secs = 60

[translation]
parallelism = 1
context_window_blocks = 2  # 直前何ブロックを context として渡すか
keep_original_as_aside = false  # 併記モード。デフォルト無効
skip_tags = ["code", "pre", "kbd", "samp", "script", "style"]
preserve_lang_attrs = ["zh", "fr", "de", "la"]

[style]
# プリセット: novel / novel-polite / tech / essay / academic / business / custom / auto
preset = "essay"

# preset = "custom" の場合、システムプロンプトにそのまま挿入される文体ブロック。
# 改行は LF で、【文体】見出しから書く。
custom_prompt = """
【文体】
- 地の文: である調。
- 章タイトル: 体言止め。
"""

# プリセットの一部だけ上書きしたい場合に使う（v0.3 以降）。
# 指定したキーのみプリセット文体に上書き適用される。
[style.overrides]
# narrative_form = "polite"      # である / polite
# title_form     = "noun_phrase" # noun_phrase / verb_phrase / free
# allow_original_in_parens = true # 専門用語の原語併記を許可

[glossary]
min_occurrences = 3
max_entries = 200
auto_build = false

[output]
language = "ja"
add_translator_meta = true
add_japanese_font_fallback = true
font_fallback = "\"Hiragino Mincho ProN\", \"Yu Mincho\", serif"

[cache]
enabled = true
# dir のデフォルトは OS のキャッシュ規約に従う
#   Windows: %LOCALAPPDATA%\epubicus\cache
#   Linux:   ~/.cache/epubicus
#   macOS:   ~/Library/Caches/epubicus
# dir = "D:\\epubicus-cache"

# 完了後にキャッシュを自動削除（デフォルト true）。
# false にすると完了ランも残り、手動削除が必要。
auto_delete_on_completion = true

# 未完ランの自動 prune（最終更新から N 日経過で削除）。0 = 無効。
auto_prune_after_days = 0

# manifest の同期保存頻度（N セグメント完了ごと）。中断時の損失上限。
sync_every_n_segments = 50

# 蓄積警告のしきい値（起動時に 1 行ヒントを表示する条件）。
hint_threshold_count = 5
hint_threshold_mb = 500
```

---

## 10. ディレクトリ構成

```
epubicus/
├─ Cargo.toml
├─ Cargo.lock
├─ README.md
├─ DESIGN.md            (本書)
├─ LICENSE
├─ epubicus.toml.example
├─ src/
│  ├─ main.rs           # CLI エントリ
│  ├─ cli.rs            # clap 定義
│  ├─ config.rs         # toml 設定
│  ├─ error.rs          # thiserror による Error 型
│  ├─ epub/
│  │   ├─ mod.rs
│  │   ├─ unpack.rs     # zip 展開
│  │   ├─ pack.rs       # zip 再パック (EPUB 仕様: mimetype を最初に無圧縮で)
│  │   ├─ container.rs  # META-INF/container.xml
│  │   ├─ opf.rs        # package document
│  │   ├─ spine.rs
│  │   └─ nav.rs        # nav.xhtml + NCX
│  ├─ xhtml/
│  │   ├─ mod.rs
│  │   ├─ parse.rs      # quick-xml ベースの DOM
│  │   ├─ extract.rs    # ブロック → セグメント抽出
│  │   ├─ placeholder.rs # ⟦E1⟧ などの符号化/復号
│  │   └─ assemble.rs   # 訳文 → DOM 書き戻し
│  ├─ translate/
│  │   ├─ mod.rs
│  │   ├─ ollama.rs     # HTTP クライアント
│  │   ├─ prompt.rs     # プロンプト組立て
│  │   ├─ validate.rs   # プレースホルダ検証
│  │   ├─ glossary.rs
│  │   └─ pipeline.rs   # 翻訳全体の流れ
│  ├─ cache/
│  │   ├─ mod.rs
│  │   └─ jsonl.rs
│  ├─ progress.rs       # indicatif ラッパ
│  └─ report.rs         # 最終レポート出力
└─ tests/
   ├─ fixtures/
   │   ├─ minimal.epub
   │   └─ alice.epub        # 著作権切れの実書籍
   ├─ epub_roundtrip.rs
   ├─ placeholder_idempotent.rs
   └─ glossary_extract.rs
```

---

## 11. 主要クレート

| クレート | 用途 | 備考 |
|--|--|--|
| `clap` (v4, derive) | CLI パース | |
| `serde`, `serde_json` | シリアライズ | |
| `toml` | 設定ファイル | |
| `zip` | EPUB 入出力 | EPUB 仕様: `mimetype` を最初・無圧縮で格納する点に注意 |
| `quick-xml` | XHTML パース | ホイル & 高速。XML として処理（HTML パーサだと自己終了タグ等で問題） |
| `reqwest` (blocking + json) | Ollama HTTP | v1 はブロッキングで十分 |
| `tokio` | 非同期ランタイム | v1.1 並列化時に導入。v1 は不要 |
| `anyhow` | アプリエラー伝播 | |
| `thiserror` | ライブラリ層エラー型 | |
| `tracing`, `tracing-subscriber` | ログ | |
| `indicatif` | 進捗バー | |
| `sha2` | ハッシュ | キャッシュキー |
| `regex` | プレースホルダ検出 | |
| `unicode-segmentation` | 文字数カウント | LLM トークン推定 |
| `tempfile` | 一時ディレクトリ | |
| `walkdir` | 展開後ディレクトリ走査 | |

---

## 12. 主要データ型（抜粋）

```rust
// ブロックから抽出されるセグメント
pub struct Segment {
    pub id: SegmentId,            // (file_id, block_index)
    pub source: String,           // プレースホルダ入りの英文
    pub tag_map: TagMap,          // ⟦E1⟧ -> 元のタグ + 属性
    pub kind: SegmentKind,        // Heading | Paragraph | ListItem | TableCell | Caption | Title | NavLabel | Meta
    pub lang_hint: Option<String>,
}

pub struct TagMap {
    pub entries: Vec<TagEntry>,
}

pub struct TagEntry {
    pub n: u32,
    pub kind: TagKind,            // Open | Close | SelfClosing
    pub tag: String,              // "em", "a", ...
    pub attrs: Vec<(String, String)>,
}

pub struct TranslationRequest {
    pub segment: Segment,
    pub glossary_subset: Vec<GlossaryEntry>,
    pub context: Vec<TranslatedBlock>,
}

pub struct TranslationResult {
    pub segment_id: SegmentId,
    pub translated: String,
    pub fallback_used: bool,
    pub elapsed_ms: u32,
}

pub struct OllamaClient {
    base: Url,
    http: reqwest::blocking::Client,
    model: String,
    options: OllamaOptions,
}
```

---

## 13. アルゴリズム詳細：抽出と復元

### 13.1 抽出（DOM → Segment）

```
fn extract_block(node: &Element, ctx: &mut ExtractCtx) -> Segment {
    let mut buf = String::new();
    let mut tag_map = TagMap::new();
    let mut counter = 0u32;
    walk(node, &mut buf, &mut tag_map, &mut counter);
    Segment { source: buf, tag_map, ... }
}

fn walk(node, buf, map, counter) {
    for child in node.children {
        match child {
            Text(t) => buf.push_str(&escape_for_placeholder(&t)),
            Element(e) if is_skip_tag(e) => {
                // タグごと丸ごと SelfClosing プレースホルダに退避
                let n = next(counter);
                map.add_self(n, e.serialize());
                buf.push_str(&format!("⟦S{n}⟧"));
            }
            Element(e) if is_void(e) => {
                let n = next(counter);
                map.add_self(n, e.tag, e.attrs);
                buf.push_str(&format!("⟦S{n}⟧"));
            }
            Element(e) => {
                let n = next(counter);
                map.add_open(n, e.tag, e.attrs);
                buf.push_str(&format!("⟦E{n}⟧"));
                walk(e, buf, map, counter);
                buf.push_str(&format!("⟦/E{n}⟧"));
                map.add_close(n);
            }
        }
    }
}
```

### 13.2 復元（訳文 + TagMap → DOM）

```
fn restore(translated: &str, map: &TagMap) -> Result<Vec<Node>, RestoreError> {
    let tokens = tokenize_with_placeholders(translated)?;
    // tokens: [Text, Open(1), Text, Close(1), Text, ...]
    validate_balanced(&tokens, map)?;
    let mut stack = vec![Vec::new()];  // ノードスタック
    for tok in tokens {
        match tok {
            Text(s) => stack.last_mut().push(Node::Text(s)),
            Open(n) => stack.push(Vec::new()),
            Close(n) => {
                let children = stack.pop();
                let entry = map.get_open(n);
                let elem = Element { tag: entry.tag, attrs: entry.attrs, children };
                stack.last_mut().push(Node::Element(elem));
            }
            SelfClose(n) => stack.last_mut().push(map.get_self(n).into_node()),
        }
    }
    assert_eq!(stack.len(), 1);
    Ok(stack.pop().unwrap())
}
```

### 13.3 検証ルール

`validate_balanced`:
1. 訳文のプレースホルダ集合が原文と完全一致（ID 単位）。
2. Open/Close が対応しており、入れ子（または並び）が DOM として再構成可能。
   - 注: 日本語語順入れ替えにより `⟦E1⟧⟦E2⟧⟦/E1⟧⟦/E2⟧` のような交差は理論上発生しうる。
   - 交差発生時は「兄弟関係に変換」(`⟦E1⟧X⟦/E1⟧⟦E2⟧Y⟦/E2⟧` 化) を試み、それも無理ならフォールバック。
3. SelfClosing は ID ごとに 1 回ずつ。

---

## 14. テスト戦略

| 種別 | 内容 |
|--|--|
| 単体 | placeholder の符号化/復号がラウンドトリップする（property test、`proptest` 推奨） |
| 単体 | OPF/NCX/nav パーサが minimal fixture を読み書きできる |
| 単体 | EPUB 再パック後、`unzip -l` 出力で mimetype が先頭・無圧縮 |
| 統合 | `--dry-run` で著作権切れ書籍 1 冊が完走（LLM はモック） |
| 統合 | Ollama モック（ローカル HTTP サーバ）で full pipeline |
| ゴールデン | 既知の難所（ルビ、複雑な脚注、テーブル）を含む小型 EPUB の差分テスト |
| 手動 | 実際の Ollama + 実際の英書 1 章を翻訳し目視 QA |

`tests/fixtures/` に最小 EPUB を 1 つ手書きで作っておく。

---

## 15. エラー処理とロギング

- ライブラリ層（`xhtml`, `epub`, `translate`）は `thiserror` で型付き Error。
- アプリ層は `anyhow` で集約。
- `tracing` で構造化ログ（`-v` で `debug`、`-vv` で `trace`）。
- 致命的でない問題（フォールバック発動、用語集未定義の固有名詞検出）は最終レポートに集約。
- 中断時はキャッシュにそこまでの結果が書き込まれているため、再実行で続きから。

---

## 16. パフォーマンス想定

| 項目 | 見積 |
|--|--|
| 平均英文ブロック | 60 単語 |
| qwen2.5:14b 推論速度 (RTX 4070 12GB, num_ctx=4096) | 約 30 tok/s |
| 1 ブロックあたり所要 | 5〜15 秒（生成 100〜300 tok 仮定） |
| 中編書籍（4,000 ブロック） | 6〜16 時間 |
| CPU 推論（GPU なし） | 上記の 5〜10 倍 |

→ 「夜寝る前に走らせて朝完成」が現実的なライン。バッチング（複数ブロック連結）で 2〜3 倍高速化できる可能性あり、v1.1 で検証。

---

## 17. ロードマップ

### v0.1 (MVP)
- [x] EPUB 展開 / 再パック
- [x] OPF / 単一 XHTML パース
- [x] プレースホルダ符号化 / 復号
- [x] Ollama クライアント（同期、リトライなし）
- [x] 章単位 translate サブコマンド
- [x] minimal fixture でのラウンドトリップテスト

### v0.2
- [x] キャッシュ（JSONL、ブロック単位）
- [x] キャッシュ済みブロックだけで部分翻訳 EPUB を作成する `--partial-from-cache`
- [x] glossary 構築 / 適用（候補抽出 + 手動編集 JSON + 翻訳時適用）
- [ ] nav.xhtml / NCX 翻訳
- [ ] フォールバック処理 + 最終レポート

### v0.3
- [ ] context_window_blocks 対応
- [x] 進捗バー
- [ ] 設定ファイル
- [ ] バッチング（複数ブロック / リクエスト）

### v0.4
- [ ] tokio 化と並列度オプション
- [ ] `--keep-original` 併記モード
- [ ] epubcheck 連携（あれば実行、警告表示）

### v1.0
- [ ] 著作権切れ実書籍 5 冊での実証
- [ ] ドキュメント整備
- [ ] **Inno Setup による Windows インストーラ作成**（`installer/epubicus.iss`、署名は v1.1 以降）
- [ ] GitHub Releases への成果物アップロード（`epubicus-x.y.z-windows-x64.exe`、`epubicus-x.y.z-windows-x64.zip`）
- [ ] `cargo install epubicus` でも入る（crates.io 公開）

### v2 候補
- [ ] GUI（tauri or egui）— v1 では CLI のみ。GUI ニーズが顕在化してから着手
- [ ] 縦書き化オプション
- [ ] ルビ自動付与
- [ ] PDF → EPUB 変換との接続

---

## 18. 既知のリスクと対策

| リスク | 影響 | 対策 |
|--|--|--|
| ローカル LLM の品質ばらつき | 訳文の不自然さ | プロンプト調整、temperature=0.3、用語集、context |
| プレースホルダ破壊 | 書式喪失 | 検証 + 再試行 + フォールバック |
| 推論時間の長さ | 1 冊数時間〜十数時間 | キャッシュ、再開可能性、バッチング |
| EPUB 仕様逸脱した入力 | パース失敗 | 寛容パーサ + 失敗時はファイルをスキップしてコピー |
| 日本語フォント未指定 | リーダで豆腐 | CSS にフォールバックを注入 |
| 中断後ファイル破損 | 出力不可 | tempdir で作業、最後に atomic に move |
| 大型書籍のメモリ | OOM | XHTML をストリーミング/個別処理（章ごとにメモリ解放） |

---

## 19. 決定事項（2026-04-28 確定）

| # | 項目 | 決定 |
|--|--|--|
| 1 | バイナリ名 | **`epubicus`** で確定 |
| 2 | 既訳併記モード | **デフォルト無効**。`--keep-original` または `[translation].keep_original_as_aside = true` で有効化 |
| 3 | 文体選択 | **ジャンル別プリセット + 設定ファイルでの個別指定**。`--style` および `[style]` セクションで切替（§5.4）。プリセット未指定時は `essay`（v0.1）または `auto` 判定（v0.3 以降） |
| 4 | UI | **CLI のみ**。GUI は v2 候補として保留 |
| 5 | 配布形態 | **Inno Setup による Windows インストーラ**を v1.0 で配布。`cargo install` も併用可（§17 ロードマップ） |
| 6 | キャッシュ識別 | **入力 EPUB ファイル全体の SHA-256 先頭 16 バイト**をラン ID に使用（§7.2）。コンテンツ指紋等の併用はしない（再保存・微修正でも別キャッシュ扱い、割り切り） |
| 7 | キャッシュ自動削除 | **成功完了時に自動削除**。中断・`--partial-from-cache`・`--dry-run` では削除しない。`--keep-cache` で完了後保持可能（§7.7-§7.8） |
| 8 | キャッシュ全削除 | `epubicus cache clear --all` を提供。`yes` 全文入力での確認必須、`--yes` でスキップ、`--dry-run` でプレビュー（§8.1.1） |

### 19.1 v0.1 着手にあたっての残課題（実装中に判断）

- TOML スキーマの厳密化（`serde` で deserialize、未知キーで warning）
- インストーラのインストール先（`%ProgramFiles%\epubicus\` の予定）と PATH 追加可否のオプション
- Inno Setup の `.iss` テンプレートを `installer/` に置くか、`tools/installer/` に置くか
- ライセンス（v1 までに決定。MIT / Apache-2.0 デュアルが Rust エコシステムでは無難）

---

## 20. 参考

- EPUB 3.3 仕様: https://www.w3.org/TR/epub-33/
- Ollama API: https://github.com/ollama/ollama/blob/main/docs/api.md
- `quick-xml` ドキュメント
- 既存の類似 OSS（参考用、コピーはしない）: `ebook-translator-server` (Python), `epub-translator` (Python)

---

## 21. 実装ステータス（2026-04-28）

現在の実装は `README.md` / `README.ja.md` に利用手順を集約している。設計書との差分として、v0.1 の縦切りに加えて一部 v0.2 / v0.3 相当の補助機能も先行実装済み。

### 実装済み

- `translate`: 指定 spine 範囲を翻訳して EPUB を再パックする。一部範囲だけ翻訳し、それ以外は元のまま残せる。
- `test`: 指定 spine 範囲を翻訳し、標準出力に表示する。
- `inspect`: OPF spine の順序、`linear`、参照先ファイル有無、サイズ、翻訳対象ブロック概算を表示する。
- `toc`: EPUB3 `nav.xhtml` または EPUB2 NCX の目次を表示する。
- `glossary`: 固有名詞・専門用語候補を JSON に出力し、手動編集した `dst` を翻訳時の用語集として使う。
- provider: Ollama / OpenAI Responses API / Claude Messages API を `--provider` で切替。
- API key: OpenAI / Claude は環境変数、明示オプション、`--prompt-api-key` に対応。
- progress: 本番 `translate` で翻訳対象ブロック数ベースのプログレスバーを表示。
- `--dry-run`: provider を呼ばず、EPUB 展開・XHTML 走査・再パックの確認ができる。

### 未実装・注意点

- JSONL キャッシュは実装済み。中断後の再実行で翻訳済みブロックを再利用でき、`--partial-from-cache` で途中までの訳文と以降の原文をつないだ EPUB を作成できる。
- 目次表示は実装済みだが、`nav.xhtml` / NCX の翻訳は未実装。
- OPF metadata の翻訳は未実装。`dc:language` と contributor 追加のみ。
- プレースホルダ検証失敗時の再試行、フォールバック詳細レポートは未実装。
- 設定ファイル `epubicus.toml` は未実装。
- バッチング、context window、並列翻訳は未実装。
- `<code>` / `<pre>` / `script` / `style` / MathML などは翻訳対象外として扱う。
