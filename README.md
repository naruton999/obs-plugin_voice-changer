# ボイスチェンジャー（OBS 音声フィルタ）

OBS Studio 用の音声フィルタプラグイン。ピッチとフォルマントを独立に制御する。

## 状態

v0.1.0 開発中。

| 部分 | 状態 |
|---|---|
| `vc-core` | DSP チェーン（ゲイン／HPF／ピッチ／フォルマント／トーン EQ／コンプ） |
| `obs-filter` | macOS arm64 / Windows x64。`vc-core` を接続済み |
| ピッチ | WSOLA（既定）と位相ロック付きフェーズボコーダを切替可能 |
| フォルマント | 独立 STFT 段（テーパー付きリフタ、必要なときだけ実行） |
| ステレオ | 2ch は Mid を1回処理して左右へコピー（左右の位相ずれ・揺れ防止） |
| ゲート | ダウンワードエキスパンダ実装済み（既定 OFF） |
| UI・プリセット読込 | 実装済み（`high-quality-female` ほか） |

## 構成

| パス | 内容 |
|---|---|
| `crates/vc-core/` | 音声処理コア（OBS 非依存） |
| `crates/obs-filter/` | OBS バインディング（cdylib） |
| `docs/DESIGN.md` | 確定仕様 |
| `presets/` | プリセット |

## ビルド・テスト

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## OBS プラグイン

### 共通仕様

| 項目 | 値 |
|---|---|
| フィルタ ID | `obs_plugin_voice_changer_filter` |
| 表示名 | ボイスチェンジャー |
| 対象 OBS | 32.2.1 以上（ビルドは OBS 31.1.1 API） |
| 音声処理 | `vc-core` を接続済み |

`filter_audio` は `vc-core` のチェーン
（入力ゲイン → HPF → ゲート → ピッチ → フォルマント → トーン EQ → コンプ → 出力ゲイン → ソフトクリップ）を呼ぶ。
既定は WSOLA（時間伸縮＋リサンプル、正規化相関、Catmull-Rom 補間、アップシフト時のリサンプル前ローパス）で、フォルマントは独立 STFT 段（テーパー付きケプストラムリフタ、無音ビンを増幅しない床、ゆっくりしたレベル補正）で動かす。高品質が必要な場合は位相ロック付きフェーズボコーダに切り替えられる。ゲートはダウンワードエキスパンダで実装済み（既定 OFF）。2ch は Mid を1回処理して左右へコピーする。
設定は OBS のプロパティで変更し、シーンコレクションに保存される。

### プロパティ

| 項目 | 内容 |
|---|---|
| プリセット | `presets/*.toml` を自動列挙し、「プリセットを適用」で反映 |
| フォルマント独立制御 | ON のとき、フォルマントをピッチと別に設定 |
| フォルマントを有効にする | OFF で STFT 処理をスキップ（遅延が約 21ms 減る） |
| ピッチ / フォルマント | -12〜+12 半音（0.1 刻み） |
| 高品質モード（フェーズボコーダ・任意） | 既定 OFF（WSOLA）。ON で位相ロック付き PV に切り替え |
| 入力ゲイン / HPF / 出力ゲイン | 各スライダー |
| トーン EQ | 低域 (250Hz) / プレゼンス (2.5kHz) / 高域 (5kHz) の ±12dB |
| コンプレッサー | ON/OFF と Threshold / Ratio / Attack / Release / Makeup |
| ゲート | ON/OFF と Threshold / Ratio / Attack / Release / Hold |
| 遅延維持バイパス | 処理を回したまま乾音を出力（乾音も同じ遅延で出力） |
| 概算遅延 / バージョン | 情報ラベル |

### プリセット

`data/obs-plugins/obs-plugin_voice-changer/presets/*.toml`（macOS は
`.plugin/Contents/Resources/presets/*.toml`）を起動時に列挙します。
新しいプリセットは TOML を置いてプロパティを開き直すと一覧に追加されます。

同梱:

| ファイル | 内容 |
|---|---|
| `high-quality-female.toml` | **推奨（高品質モード/PV）**（ピッチ +12 / フォルマント +4.4 / トーン EQ / コンプ / 出力 +2.0dB） |
| `standard-female.toml` | WSOLA 版の標準（同じ音色設定、`phase_vocoder=false`） |
| `sharp-female.toml` | 控えめな女性化（ピッチ +3.5 / フォルマント +2.5） |
| `natural-female.toml` | 別案（ピッチ +11 / フォルマント +6.0 / トーン控えめ） |
| `bright-female.toml` | 別案（ピッチ +12 / フォルマント +7.0 / 軽いコンプ） |
| `max-female.toml` | 別案・限界寄り（ピッチ +12 / フォルマント +10.0） |
| `migrated-female.toml` | `standard-female` と同設定（旧名・互換用） |

### 自然な女性声に近づける

男性声を女性声に近づけるとき、**ピッチだけを上げてフォルマントを据え置くと「匿名ニュース声」**になりやすい。
ピッチとフォルマントの差が大きいほど人工的に聞こえるため、次の目安で調整する。

| 項目 | 目安 | 備考 |
|---|---|---|
| ピッチ | +10〜+12 半音 | 低い男性声（F0 100〜110Hz）から 180〜210Hz へ |
| フォルマント | +4〜+7 半音 | +4.4 が標準、+7 で明るめ、+10 は限界寄り |
| 高域 EQ | +0〜+6 dB | 標準は +6。歯擦音が気になる場合は +2〜+3 に下げる |
| コンプ | ratio 2〜3 | かけすぎると「放送声」になる |

**`high-quality-female` が推奨プリセット**（高品質モード/PV）。`standard-female` は同じ音色設定の
WSOLA 版、`natural-female` / `bright-female` / `max-female` は別案、`migrated-female` は
`standard-female` の旧名（互換用）。

オフラインで聴き比べる場合（既定は WSOLA、`--pv` でフェーズボコーダ）:

```bash
cargo run -p vc-core --example render -- \
  wav/dry.wav wav/high-quality-female.wav --pv \
  --pitch 12 --formant 4.4 --tone-low -3 --tone-mid 2 --tone-high 6 \
  --comp --threshold -18 --ratio 3 --attack 8 --release 150 --makeup 2 \
  --out-gain 2 --hpf 100
```

内部遅延の目安: **WSOLA（既定）はピッチ 40ms＋フォルマント 21ms で約 61ms**。
高品質モード（PV）はピッチ 43ms＋フォルマント 21ms で約 64ms。ピッチ 0 のときはピッチ段をスキップする。
OBS の音声ブロック（約 21ms）を含めると 82〜85ms。フォルマントを OFF にすると約 21ms 減る。

> 高品質モードは WSOLA より CPU を使います（FFT 2048 点＋位相ロック＋包絡推定）。

libobs はビルド時にリンクしない。macOS は OBS がプラグインをロードするときにシンボルを解決し、
Windows は `raw-dylib` で `obs.dll` にリンクする。
そのため、ビルド時に OBS のフレームワークや SDK ライブラリは不要。

### macOS（arm64）

#### ビルド

```bash
cargo build -p obs-filter --release
```

#### パッケージ

```bash
./build-aux/package-macos.sh
```

`dist/obs-plugin_voice-changer.plugin` が生成される（ad-hoc 署名済み）。

配布用に Developer ID で署名する場合（公証はしない）:

```bash
CODESIGN_IDENTITY="Developer ID Application: ..." ./build-aux/package-macos.sh
```

#### 配置

`dist/obs-plugin_voice-changer.plugin` を
`~/Library/Application Support/obs-studio/plugins/` にコピーし、OBS を再起動する。

### Windows（x64）

#### ビルド・パッケージ

```powershell
cargo build -p obs-filter --release --target x86_64-pc-windows-msvc
./build-aux/package-windows.ps1 -Configuration Release -SkipBuild
```

`-SkipBuild` を省略すると、スクリプトがビルドから実行する。

`dist/obs-plugin_voice-changer-windows-x64.zip` が生成される。

#### 配置（OBS 33 以降の新配置）

1. zip を展開する。
2. `obs-plugin_voice-changer` フォルダを `C:\ProgramData\obs-studio\plugins\` にコピーする。
3. OBS を再起動する。

```
C:\ProgramData\obs-studio\plugins\obs-plugin_voice-changer\
├─ obs-plugin_voice-changer.dll
└─ data\presets\sharp-female.toml
```

`OBS_PLUGINS_PATH` を設定している場合は、そのフォルダ配下に置いてもよい。

#### 配置（OBS 32.x / 旧配置）

| 種類 | コピー先 |
|---|---|
| DLL | `C:\Program Files\obs-studio\obs-plugins\64bit\` |
| data | `C:\Program Files\obs-studio\data\obs-plugins\obs-plugin_voice-changer\` |

管理者権限が必要。アンインストールはコピーしたファイルとフォルダの削除のみ。

#### 注意

- 署名なしのため、SmartScreen の警告が出る場合がある。zip のプロパティで「ブロックの解除」を行う。
- CI は GitHub Actions の手動実行（`Windows Build` / `macOS Build`）。artifact 自体が zip なので、ダウンロードして 1 回展開すると `obs-plugin_voice-changer/`（macOS は `.plugin`）が入っている。

## ライセンス

GPL-2.0-or-later
