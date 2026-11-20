# Physical Modeling Piano F1

弦や駒、響板や空気への放射などの物理モデルをリアルタイムに計算する 88 鍵ピアノ音源と、その Web アプリです。

## クイックスタート

[piano.yoshiyukikoyanagi.com](https://piano.yoshiyukikoyanagi.com) にアクセスすると、その場でピアノ音源を試すことができます。

ローカルの開発環境で動作を試したい場合は、ターミナルを開いて以下のコマンドを実行してください。

```bash
node tools/serve.mjs 8080
```

- 上記のコマンドを実行したら、ブラウザで `http://localhost:8080` を開いてください。
- 動作には最新の Node.js が必要です。

## UIの操作方法

ページを開いたら、最初に「Start Playing」ボタンを押してください。音が出せる状態になります。

### PC キーボードで演奏する

- `Z X C …` の段と `Q W E …` の段: 鍵盤を弾きます。`Q W E …` の段は、`Z X C …` の段より 1 オクターブ高い音です。
- `↑` `↓`: オクターブを移動します。
- `←` `→`: 打鍵の強さ（Velocity）を調整します。
- `Shift`: 押している間、サステインペダルを踏んだ状態になります。
- `Esc`: 鳴っている音をすべて止めます。

### MIDI ファイルを再生する

- 「Open MIDI File」ボタンで標準 MIDI ファイル（`.mid`）を選ぶと、その演奏を再生します。
  - ショートカットキーは、macOS では `Command+U`、それ以外では `Ctrl+U` です。
- `Space`: MIDI ファイルの再生と一時停止を切り替えます。

## このプロジェクトについて

Anthropic の Fable 5.2 と対話しながら作成したプロジェクトです。物理モデルは Fable 5.2 の持っている知識に基づいて作成されたものです。私は指示だけを行い、手作業での実装は全く行っていません。

なお本プロジェクトについて、既存の有名な商用ピアノ音源（例えば Synthogy の Ivory など）をリバースエンジニアリングして挙動を復元したものではありません。また既存のオープンソース実装を丸ごとコピーしたものでもありません。

### コントリビューションと Fork

あくまでホビープロジェクトなので、 PR は受け付けていません。改善点が見つかった場合は、このリポジトリを Fork して、あなた自身の新しいピアノ音源としてリリースいただいて構いません。クレジット表記も不要です。もっとも、プロジェクトは [@moutend](https://github.com/moutend) が起源であると示していただけると嬉しいです。

### プライバシーとセキュリティ

この Web アプリはクライアントサイドで動作するものであり、訪問者の個人情報を外部のサーバーに送信することはありません。外部の CDN から動的に生成された JavaScript を読み込むといったこともありません。

従って、現時点でのセキュリティについては万全であると認識しています。ただし、もしセキュリティの欠陥が発見されたとしても、私は責任を負わないものとします。

## その他

<details>
<summary>開発者向けのドキュメントとコマンド</summary>

- 作業内容・モデルの説明・計測結果は [docs/report.md](docs/report.md) にあります。
- 計測ログは [docs/experiments/](docs/experiments/) にあります。
- MIDI ファイルを WAV に書き出します（録音との比較用）: `cd engine && cargo run --release --bin render -- midi out.wav ../midi/fur_Elise_WoO59.mid`
- ペダルを踏んだままの同音連打を検査します（全 88 鍵）: `cd engine && cargo run --release --bin repeat -- scan`
- エンジンを再ビルドします: `./build.sh`（Rust と `wasm32-unknown-unknown` ターゲットが必要です）
- 負荷を計測します: `cd engine && cargo run --release --bin bench`（ネイティブ）、`node tools/bench-wasm.mjs`（WASM。デモ曲の再生も計測します）。遅いマシンの代わりに高効率コアで計測するには、macOS では `taskpolicy -b` を前に付けます。
- エンジンを変更する前後のレンダリングを比較します: `cd engine && cargo run --release --bin abcmp -- before.wav after.wav`（差信号のレベルと、時間窓ごとの 1/3 オクターブ帯域レベルの差を表示します）
- 響板モードを再計算します: `cd engine && cargo run --release --bin gen_board`
- 音量補正テーブルを再計算します（モデルを変えたら必要です）: `cd engine && cargo run --release --bin gen_leveling`
- モデルを変えたときは、補正テーブル一式を `build.sh` 冒頭の順で再計算します（`gen_sustain` → `gen_voicing` → `gen_leveling` → `gen_morph` → `gen_leveling`。大きく変えたときは 2 巡します）。C3 以下の鍵の欄は作り直さず保持されます。
- 基準鍵盤の音色を他の鍵盤へ補間するテーブルを再計算します: `cd engine && cargo run --release --bin gen_morph`、続けてもう一度 `gen_leveling` を実行します（基準鍵盤の一覧は `engine/src/morph.rs` の `ANCHORS` にあります。結果は `cargo run --release --bin analyze -- morph` で確認できます）。

</details>

## ライセンス

MIT

**補足情報**

リポジトリのオーナーである私は、この実装は Fable 5.2 がピアノの物理的な特性に基づいて独自に考えて実装したものであると認識しています。しかし、もし既存ソフトウェアのライセンスを侵害しているような実装が含まれていたらご連絡ください。連絡先は `Yoshiyuki Koyanagi <moutend@gmail.com>` です。
