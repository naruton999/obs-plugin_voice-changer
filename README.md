# ボイスチェンジャー（OBS 音声フィルタ）

OBS Studio 用の音声フィルタプラグイン。ピッチとフォルマントを独立に制御する。

## 状態

v0.1.0 開発中。

| 部分 | 状態 |
|---|---|
| `vc-core` | DSP チェーン（ゲイン／HPF／ソフトクリップ／バイパス／WSOLA ピッチ／フォルマント） |
| `obs-filter` | macOS arm64 / Windows x64。`vc-core` を接続済み |
| ピッチ | WSOLA 実装済み（UI から設定可能） |
| フォルマント | STFT 実装済み（既定 ON、独立制御に対応） |
| ゲート | ダウンワードエキスパンダ実装済み（既定 OFF） |
| UI・プリセット読込 | 実装済み（`sharp-female`） |

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
（入力ゲイン → HPF → ゲート → ピッチ → フォルマント → 出力ゲイン → ソフトクリップ）を呼ぶ。
ピッチは WSOLA、フォルマントは STFT の包絡置換で実装済み。ゲートはダウンワードエキスパンダで実装済み（既定 OFF）。
設定は OBS のプロパティで変更し、シーンコレクションに保存される。

### プロパティ

| 項目 | 内容 |
|---|---|
| プリセット | `sharp-female` を選択し「プリセットを適用」で反映 |
| フォルマント独立制御 | ON のとき、フォルマントをピッチと別に設定 |
| フォルマントを有効にする | OFF で STFT 処理をスキップ（遅延が約 21ms 減る） |
| ピッチ / フォルマント | -12〜+12 半音（0.1 刻み）。フォルマントは ±7 半音でクランプ |
| 入力ゲイン / HPF / 出力ゲイン | 各スライダー |
| ゲート | ON/OFF と Threshold / Ratio / Attack / Release / Hold |
| 遅延維持バイパス | 処理を回したまま乾音を出力（乾音も同じ遅延で出力） |
| 概算遅延 / バージョン | 情報ラベル |

内部遅延の目安（フォルマント ON）: ピッチ 0 で約 31ms、+3.5 半音で約 50ms、+12 半音で約 81ms。
OBS の音声ブロック（約 21ms）を含めた合計はこれに +21ms した値になる。
フォルマントを OFF にすると約 21ms 減る。

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
- CI は GitHub Actions の手動実行（`Windows Build`）で、artifact から zip を取得する。

## ライセンス

GPL-2.0-or-later
