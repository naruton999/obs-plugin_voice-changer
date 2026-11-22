# ボイスチェンジャー 設計仕様

| 項目 | 内容 |
|---|---|
| 文書種別 | 確定仕様 |
| 対象 | OBS Studio 音声フィルタプラグイン |
| バージョン | 0.1.0 |
| 対象 OBS | 32.2.1 以上 |
| 実装言語 | Rust（cdylib、C++ シムなし） |
| 対応 OS | Windows x86_64 / macOS arm64 |
| ライセンス | GPL-2.0-or-later |

---

## 1. 概要

マイク音声の**ピッチ**と**フォルマント**を独立に制御する OBS 音声フィルタ。
配信中の安定動作と、実用的な遅延（合計 40〜70ms）を両立する。

想定チェーン:

```
[マイク]
  → ノイズ抑制（OBS 標準または別プロジェクト）
  → ボイスチェンジャー（本プラグイン）
  → EQ / Compressor / Limiter（OBS 標準）
  → 配信
```

ピッチシフトはノイズ抑制の**後**に置く。

### 1.1 目的

| 優先 | 目的 |
|---|---|
| G1 | 声を「シャープな女性」に近づけられる |
| G2 | 配信中に破綻しない（音切れ・クラッシュ・CPU 過多がない） |
| G3 | 追加遅延が 40〜70ms に収まる |
| G4 | プラグインの障害が OBS を巻き込まない |

### 1.2 対象外

| # | 項目 |
|---|---|
| N1 | 仮想オーディオデバイス出力 |
| N2 | Discord・ゲーム内ボイスでの利用 |
| N3 | ノイズ抑制 |
| N4 | トレイ常駐・単体ウィンドウ・単一インスタンス |
| N5 | 解析コマンド・プリセット自動生成 |
| N6 | ループバック（OBS のモニタリングで代替） |
| N7 | プリセットの書き出し（v1 は読込のみ） |

---

## 2. アーキテクチャ

```
OBS 音声スレッド
  └─ filter_audio(parent, context, obs_audio_data)
        │  planar f32 / 可変フレーム数
        ▼
  ┌──────────────── vc-core ────────────────┐
  │ InputGain → HPF → Gate → Pitch(PV/WSOLA)│
  │           → Formant(任意) → OutGain     │
  │           → SoftClip                    │
  └─────────────────────────────────────────┘
        │
        ▼
  OBS のミキサ・配信経路
```

| 層 | 実装 |
|---|---|
| プラグイン境界 | Rust の `cdylib` |
| DSP | OS API 非依存の純 Rust クレート `vc-core` |
| OBS バインディング | `obs-filter` クレート（手書き FFI） |
| UI | OBS プロパティ（`obs_properties_t`） |
| 設定保存 | OBS のシーンコレクション（`obs_data_t`） |

### 2.1 クレート構成

```
<repo>/
├─ Cargo.toml                  # workspace
├─ crates/
│  ├─ vc-core/                 # DSP 本体（FFI なし・テスト可能）
│  └─ obs-filter/              # libobs バインディング + filter_audio（cdylib）
├─ presets/
│  └─ general-female.toml
│  └─ general-male.toml
├─ data/obs-plugins/<module>/
│  ├─ presets/
│  └─ locale/
├─ docs/
└─ .github/workflows/
```

`vc-core` はサンプルレートと最大ブロック長を外部から受け取る。
OBS 固有の都合（ブロック長・チャンネル配置）を持ち込まない。

---

## 3. OBS 統合仕様

| 項目 | 値 |
|---|---|
| フィルタ種別 | `OBS_SOURCE_TYPE_FILTER` |
| `output_flags` | `OBS_SOURCE_AUDIO` |
| 実装コールバック | `create` / `destroy` / `update` / `get_properties` / `get_defaults` / `filter_audio` |
| 対象 OBS | 32.2.1 以上 |
| エントリポイント | `obs_module_load` / `obs_module_unload` / `obs_module_ver` など |

### 3.1 音声フォーマット

| 項目 | 扱い |
|---|---|
| 形式 | planar f32（`obs_audio_data.data[ch]`） |
| フレーム数 | 可変（64〜2048 を想定。固定前提にしない） |
| チャンネル | 1ch / 2ch |
| 2ch の扱い | Mid を 1 回処理して左右へ複製（ボイスチェンジャーはモノラル効果。左右別処理は位相ずれ・揺れの原因になる） |
| サンプルレート | 48kHz 既定、44.1kHz も対応。処理は ms 基準 |
| タイムスタンプ | `obs_audio_data.timestamp` は変更しない |

### 3.2 リアルタイム制約

音声スレッドで実行するため、以下を禁止する。

| 禁止 | 代替 |
|---|---|
| `malloc` / `Vec` の拡張 | `create` / `update`（メインスレッド）で最大ブロック長分を確保 |
| mutex / `RwLock` | atomic（f32 はビット保持） |
| ログ・`format!` | atomic カウンタに記録し UI 側で読む |
| ファイル I/O | プリセットは `update` で読む |
| panic を FFI 境界へ通す | `catch_unwind` で全エントリを保護 |

### 3.3 命名

| 項目 | 値 |
|---|---|
| モジュール / DLL | `obs-plugin_voice-changer` |
| フィルタ ID | `obs_plugin_voice_changer_filter` |
| 表示名 | ボイスチェンジャー |
| データフォルダ | `data/obs-plugins/obs-plugin_voice-changer/` |
| macOS バンドル | `obs-plugin_voice-changer.plugin` |

Cargo の cdylib 出力名はアンダースコア区切りになるため、パッケージ時に上記のモジュール名へリネームする。

---

## 4. DSP 仕様

### 4.1 処理チェーン

```
in ─► InputGain ─► HPF ─► Gate(既定 OFF) ─► Pitch(WSOLA 既定 / PV 任意)
   ─► Formant(任意) ─► OutGain ─► SoftClip ─► out
```

- 順序は固定。
- 各段は `trait DspStage { fn process(&mut self, buf: &mut [f32]); }` を実装する。
- パラメータ変更は**毎ブロック先頭**で反映し、10〜20ms でスムージングする。
- 無効な段はスキップする（関数ポインタは使わない）。

### 4.2 パラメータ

| 段 | パラメータ | 範囲 | 既定 |
|---|---|---|---|
| InputGain | 入力ゲイン | -24〜+24 dB | 0 dB |
| HPF | カットオフ | 20〜200 Hz（2 次 Butterworth） | 80 Hz |
| Gate | 有効/無効 | on/off | OFF |
| | Threshold | -60〜-20 dBFS | -45 dBFS |
| | Ratio | 2〜20 | 4 |
| | Attack | 1〜20 ms | 5 ms |
| | Release | 50〜500 ms | 120 ms |
| | Hold | 0〜200 ms | 40 ms |
| Pitch | シフト量 | -12〜+12 半音（0.1 刻み） | 0 半音 |
| | 方式 | WSOLA / 位相ロック PV | WSOLA |
| Formant | 有効/無効 | on/off | ON |
| | 独立制御 | on/off | OFF（ピッチに連動） |
| | シフト量 | -12〜+12 半音（0.1 刻み） | ピッチと同じ |
| OutGain | 出力ゲイン | -24〜+24 dB | 0 dB |
| SoftClip | 閾値 | -1 dBFS 固定（ルックアヘッドなし） | 固定 |

### 4.3 Pitch（PSOLA / 位相ロック付きフェーズボコーダ / WSOLA）

既定は PSOLA（`general-female`）。F0 追跡、固定ピッチマーク、2 周期グレインの重畳で
ピッチを変え、グレインのリサンプルでフォルマントを独立に動かす。
独立 STFT フォルマント段を使わないため、二重声感が出にくい。

| 項目 | 値 |
|---|---|
| 方式 | PSOLA（pitch-synchronous overlap-add） |
| F0 解析 | 40ms 窓 / 10ms ホップ、500Hz 低域通過、正規化自己相関 |
| F0 平滑化 | 前後 2 フレームのメディアン |
| ピッチマーク | F0 から固定間隔 |
| グレイン | 入力 2 周期、Hann |
| フォルマント | グレインを `2^(formant/12)` でリサンプル |
| 遅延（48kHz） | 92ms |

既定プリセット `general-female` はピッチ +11 / グレインF +5、トーン・コンプ OFF、
出力 +1.0dB、HPF 20Hz。既存ボイチェンとの比較（48kHz）:

| 指標 | wet | PSOLA 統合 |
|---|---|---|
| F0 | 216Hz | 212Hz |
| スペクトル重心 | 636Hz | 604Hz |
| 0–500Hz | 42.4% | 42.0% |
| 500–1500Hz | 56.2% | 56.2% |
| 二重声指標 | 35.5% | 24.2% |
| HNR | 0.814 | 0.822 |

既知の制限: グレインのフォルマントリサンプルによる僅かなガラつき、無声区間も
ピッチシフトする、パラメータ変更は即時反映（平滑化なし）。

従来の WSOLA（時間伸縮＋リサンプル、正規化相関、Catmull-Rom 補間）と、
位相ロック付きフェーズボコーダ（FFT 2048 点 / hop 512 / トランジェントで位相リセット）
もフォールバックとして残す。下表は WSOLA の値。

| 項目 | 値 |
|---|---|
| 方式 | WSOLA（波形類似オーバーラップ加法） |
| 窓長 | 40ms 相当（48kHz で 1920 サンプル） |
| ホップ | 窓長 / 4 |
| 類似探索 | ±10ms |
| 窓関数 | Hann |
| 変更のスムージング | 半音値の変更は 50ms かけて補間 |

低い声でも 2 周期以上を確保できる窓長とする。

### 4.4 Formant（独立制御）

| 項目 | 値 |
|---|---|
| 方式 | STFT（1024 点 / hop 256 / Hann）→ パワースペクトル → ケプストラムリフタで包絡推定 → 周波数軸を伸縮 → 元の包絡と入れ替えて逆 FFT |
| 位相 | WSOLA 出力の位相を引き継ぐ（位相復元はしない） |
| OFF 時 | 完全にスキップ（遅延 +0ms） |
| ON 時 | 遅延 +21ms |
| 過大シフト | ±12 半音超は自動クランプする |
| 方針 | 品質より破綻の少なさを優先 |

フォルマントは既定でピッチに連動する。
「独立制御」が ON のときのみ、ピッチとは別のシフト量を適用する。
PSOLA モードではこの STFT 段を使わず、グレイン内でフォルマントを動かす。

### 4.5 浮動小数の扱い

- 非正規化数を避ける（`+1e-20` を足す、または FTZ）。
- NaN / Inf を検出したブロックは無音にし、カウンタを加算する。
- 入力が NaN / Inf でも OBS を壊さない。

---

## 5. 遅延

| 構成 | 内部遅延 |
|---|---|
| **PSOLA（既定 `general-female`）** | **92ms** |
| 高品質モード（PV、フォルマント OFF） | 43ms |
| WSOLA（フォルマント OFF） | 40ms |
| ＋フォルマント ON | +21ms |
| ＋OBS の音声ブロック | 約 21ms |
| **合計の目安（PSOLA）** | **約 113ms** |
| **合計の目安（WSOLA/PV＋フォルマント）** | **約 82〜85ms** |

PSOLA は既存ボイチェンの単一音声感を優先したため遅延が大きい。低遅延が必要な場合は
WSOLA / PV を使い、フォルマントを OFF にする（約 40〜43ms）。

- OBS の「音声同期オフセット」で補正する前提とし、**推奨値をプロパティに表示**する。
- フィルタの有効/無効で遅延が変わるため、**遅延維持バイパス**を用意する。
  - 処理は回したまま、乾音を出力する。
- 既定は品質優先でフォルマント込み約 85ms（OBS 込み）。低遅延用途ではフォルマント OFF を推奨する。

---

## 6. UI 仕様

```
┌ ボイスチェンジャー ──────────────────────────┐
│ プリセット      [ 一般女性 ▼ ]                │
│                          [プリセットを適用]   │
│ ─────────────────────────────────────────── │
│ ☑ フォルマント独立制御                        │
│    ピッチ      [────────●] +11.0 半音         │
│    フォルマント [──●─────] +5.0 半音          │
│ ─────────────────────────────────────────── │
│ 入力ゲイン [──●──] 0.0 dB   HPF [──●──] 20 Hz │
│ ☐ ゲート  Threshold [-45.0] dB                │
│ 出力ゲイン [──●──] +1.0 dB                    │
│ ─────────────────────────────────────────── │
│ ☑ 自然な声（PSOLA）                           │
│ ☐ 遅延維持バイパス                            │
│ 概算遅延 約 92ms      v0.1.0                  │
└──────────────────────────────────────────────┘
```

| 項目 | 形式 | 備考 |
|---|---|---|
| プリセット | ドロップダウン＋適用ボタン | 選択しただけでは適用しない |
| ピッチ / フォルマント | スライダー＋数値入力 | 半音。0.1 刻み |
| ゲート | チェック＋数値 | 既定 OFF |
| 概算遅延 | ラベル | フォルマント ON/OFF で変わる |
| バージョン | ラベル | 不具合報告用 |

図は既定プリセット `一般女性` 適用時の例。PSOLA とフォルマント独立制御は ON、
「自然な声（PSOLA）」を OFF にすると WSOLA / PV を手動選択できる。

- 数値は必ず単位付き。
- 変更は即時反映（`update`）。
- 設定は OBS のシーンコレクション（`obs_data_t`）に保存する。独自ファイルは作らない。

---

## 7. プリセット仕様

| 項目 | 仕様 |
|---|---|
| 形式 | TOML |
| 同梱 | `general-female.toml`（一般女性・既定）、`general-male.toml`（一般男性） |
| 配置 | `data/obs-plugins/<module>/presets/*.toml`（`obs_module_file` で解決） |
| 読込 | プラグイン読込時と `update` 時（メインスレッド） |
| 未知フィールド | 無視（前方互換） |
| `schema_version` 不一致 | 読まずにエラー表示（壊さない） |
| 保存 | v1 では実装しない |

### 7.1 スキーマ

```toml
schema_version = 1
name = "一般女性"
description = "自然な女性声。PSOLA +11 / グレインF +5"
created = "2026-11-22"

[input]
gain_db = 0.0
hpf_hz = 20.0

[gate]
enabled = false
threshold_db = -45.0
ratio = 4.0
attack_ms = 5.0
release_ms = 120.0
hold_ms = 40.0

[pitch]
semitones = 11.0
phase_vocoder = false
psola = true

[formant]
enabled = true
independent = true
semitones = 5.0

[tone]
low_db = 0.0
mid_db = 0.0
high_db = 0.0

[compressor]
enabled = false
threshold_db = -18.0
ratio = 3.0
attack_ms = 8.0
release_ms = 150.0
makeup_db = 2.0

[output]
gain_db = 1.0

# 本プラグインでは適用しない（OBS 標準フィルタ用の参考値）
[reference]
low_shelf = { freq = 250.0, gain_db = -3.0 }
peak = { freq = 2500.0, q = 1.0, gain_db = 2.0 }
high_shelf = { freq = 5000.0, gain_db = 6.0 }
lowpass_hz = 16000.0
comp = { threshold_db = -18.0, ratio = 3.0, attack_ms = 8.0, release_ms = 150.0, makeup_db = 2.0 }
```

- `[reference]` は「OBS 標準の EQ / コンプを手動設定するときの目安」。プラグインは読むだけで適用しない。
- 値はすべて「未設定なら既定値」で読む。

---

## 8. ビルド・配布仕様

### 8.1 ビルド構成

| 項目 | 内容 |
|---|---|
| ワークスペース | Cargo workspace |
| DSP | `crates/vc-core` |
| プラグイン | `crates/obs-filter`（`crate-type = ["cdylib"]`） |
| OBS SDK | `obs-plugintemplate` を git submodule として固定し、CMake preset で取得 |
| リンク | `build.rs` で SDK のヘッダ／インポートライブラリを指定 |
| FFI | 手書きの最小定義。構造体サイズをコンパイル時に検証する |
| panic | FFI 境界ごとに `catch_unwind` |

### 8.2 成果物

```
Windows:
  obs-plugins/64bit/obs-plugin_voice-changer.dll
  data/obs-plugins/obs-plugin_voice-changer/
    presets/general-female.toml
    locale/en-US.ini
    locale/ja-JP.ini

macOS:
  obs-plugin_voice-changer.plugin/
    Contents/MacOS/obs-plugin_voice-changer
    Contents/Info.plist
  data/obs-plugins/obs-plugin_voice-changer/
    presets/general-female.toml
    locale/en-US.ini
    locale/ja-JP.ini
```

### 8.3 対応プラットフォーム

| OS | 対象 | 配置先 |
|---|---|---|
| Windows | x86_64 | `C:\Program Files\obs-studio\obs-plugins\64bit\` ＋ `data\obs-plugins\` |
| macOS | **arm64 のみ** | `OBS.app/Contents/PlugIns/` または `~/Library/Application Support/obs-studio/plugins/` |

### 8.4 CI

| 項目 | 内容 |
|---|---|
| Windows | GitHub Actions `windows-latest` |
| macOS | GitHub Actions `macos-latest` |
| 実行タイミング | **手動（`workflow_dispatch`）のみ** |
| 対象 OBS | 32.2.1 以上 |

### 8.5 署名

| 場面 | 方式 |
|---|---|
| 開発中 | ad-hoc 署名 |
| 配布時 | Developer ID 署名（公証なし） |
| 未署名時の利用 | README に quarantine 解除手順を記載 |

### 8.6 配布

- v1 は **zip（手動配置）**。インストーラは作らない。
- アンインストールは DLL（または `.plugin`）と data フォルダの削除のみ。
- レジストリ・独自設定ファイルは一切作らない。

---

## 9. 性能・信頼性要件

### 9.1 CPU 予算（1 ストリーム、48kHz）

| 段 | 目安 |
|---|---|
| Gate / HPF / SoftClip | 合計 0.5% 未満 |
| WSOLA pitch | 1.5% |
| Formant STFT（ON 時） | 2.0% |
| コピー・その他 | 0.3% |
| **合計** | **約 2.3%（連動） / 約 4.3%（独立 ON）** |

目標は 1 コアの 5% 以下。超える場合は FFT 点数と窓長を下げる。

### 9.2 安定性

- 音声スレッドでメモリ確保・ロック・ファイル I/O を行わない。
- FFI 境界で panic を捕捉する。
- NaN / Inf を検出しても OBS を壊さない。
- フィルタの追加・削除・無効化を繰り返しても OBS が落ちない。

---

## 10. 受け入れ基準

| ID | 基準 |
|---|---|
| AC-01 | マイクソースに追加すると声が変わる（ピッチ +3.5 半音で明確に変化） |
| AC-02 | ピッチ ±12 半音で破綻（金属音・欠落）が実用範囲に収まる |
| AC-03 | フォルマント ON/OFF で音色が変わり、遅延差を実測して記録できる |
| AC-04 | 1ch ソース・2ch ソースの両方で動作する |
| AC-05 | 44.1kHz のシーンでも動作する（音程が狂わない） |
| AC-06 | 8 時間連続配信で音切れ・OBS の音声ドロップが 0 |
| AC-07 | CPU 使用率が 1 コアの 5% 以下（フォルマント ON） |
| AC-08 | フィルタの追加・削除・無効化を 50 回繰り返しても OBS が落ちない |
| AC-09 | シーンコレクションを切り替えても設定が保持される |
| AC-10 | OBS 終了後に DLL / `.plugin` のロックが解放され、削除できる |
