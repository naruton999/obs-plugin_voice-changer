# ボイスチェンジャー（OBS 音声フィルタ）

OBS Studio 用の音声フィルタプラグイン。ピッチとフォルマントを独立に制御する。

## 状態

v0.1.0 開発中。現在は OS 非依存の DSP コア `vc-core` の骨格のみ。
OBS プラグイン本体（`obs-filter`）は未実装。

## 構成

| パス | 内容 |
|---|---|
| `crates/vc-core/` | DSP コア（OBS 非依存） |
| `docs/DESIGN.md` | 確定仕様 |
| `presets/` | プリセット |

## ビルド・テスト

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## ライセンス

GPL-2.0-or-later
