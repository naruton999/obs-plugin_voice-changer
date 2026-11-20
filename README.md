# ボイスチェンジャー（OBS 音声フィルタ）

OBS Studio 用の音声フィルタプラグイン。ピッチとフォルマントを独立に制御する。

## 状態

v0.1.0 開発中。

| 部分 | 状態 |
|---|---|
| `vc-core` | DSP チェーン骨格（ゲイン／HPF／ソフトクリップ／バイパス） |
| `obs-filter` | macOS arm64 向けの**素通しフィルタ** |
| ピッチ／フォルマント | 未実装（枠のみ） |
| UI・プリセット読込 | 未実装 |

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

## OBS プラグイン（macOS / 素通し）

### ビルド

```bash
cargo build -p obs-filter --release
```

### パッケージ

```bash
./build-aux/package-macos.sh
```

`dist/obs-plugin_voice_changer.plugin` が生成される（ad-hoc 署名済み）。

### 配置

`dist/obs-plugin_voice_changer.plugin` を
`~/Library/Application Support/obs-studio/plugins/` にコピーし、OBS を再起動する。

### 仕様

| 項目 | 値 |
|---|---|
| フィルタ ID | `obs_plugin_voice_changer_filter` |
| 表示名 | ボイスチェンジャー |
| 対象 OBS | 32.2.1 以上（ビルドは OBS 31.1.1 API） |
| 対象 OS | macOS arm64 |
| 音声処理 | 現時点では素通し |

libobs はビルド時にリンクせず、OBS がプラグインをロードするときにシンボルを解決する。
そのため、ビルド時に OBS のフレームワークや SDK ライブラリは不要。

## ライセンス

GPL-2.0-or-later
