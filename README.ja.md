<p align="center">
  <img width="1000" src="docs/assets/melba-album.png" alt="Melba の全画面。アルバム、曲一覧、サイドバー、プレイヤー（デモ用ライブラリ）" />
  <br/><sub>実際のアプリ全画面 · デモ用ライブラリ</sub>
</p>

<h1 align="center">melba</h1>
<p align="center"><strong>Linux 向けネイティブ Apple Music クライアント</strong></p>
<p align="center">
  <a href="https://github.com/nekomario28/melba/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/nekomario28/melba/actions/workflows/ci.yml/badge.svg?branch=main" /></a>
  <a href="https://github.com/nekomario28/melba/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/nekomario28/melba?label=release" /></a>
</p>

[English](README.md) | **日本語**

Melba は Rust 製の非公式 Apple Music デスクトッププレイヤー（`melba`）と独立再生デーモン（`melbad`）です。Electron を使わず、ネイティブ egui UI でカタログ検索・ライブラリ・プレイリスト・キュー・同期歌詞・MPRIS・16 言語・署名付きポータブル更新に対応しています。

## インストール

Linux `x86_64` / `aarch64` 対応。署名付きポータブル版アーカイブ（`melba` + `melbad`）は [Releases](https://github.com/nekomario28/melba/releases) で配布しています。ソースからビルド・導入する場合は、Rust 1.98 以上と[ディストリビューション別の開発用ライブラリ](docs/installation.md)を導入した上で以下を実行します。

```sh
git clone https://github.com/nekomario28/melba.git
cd melba
sh packaging/install.sh
melba
```

`packaging/install.sh` は `~/.local/opt/melba` に `melba` と `melbad` を配置し、`~/.local/bin` にコマンドリンクとデスクトップエントリを作成します（`melbad` はデスクトップ起動時に自動起動）。別レイアウトやパッケージ手順、および ALAC 用の外部サービス導入（[source-only bootstrap](docs/installation.md#optional-source-only-wrapper-bootstrap)）は [インストール詳細](docs/installation.md) を参照してください。wrapper・rootfs・アカウント情報は Melba に同梱されません。

## 再生

`melba` を起動して **Sign in with Apple Music** を選び、Settings または `melbad setup` で出力先を選択します。フルトラック再生には Apple Music の有料契約と有効なセッションが必要です（プレビューは契約不要）。

- **AAC-256（既定）:** 端末に既存の Widevine CDM を使用
- **ALAC / ロスレス（任意）:** 利用者が別途用意・起動する互換復号サービスを使用（未接続時は AAC にフォールバック可能）
- **出力:** ALSA 直接出力（`hw:X,Y`）、または共有システム出力（`default` / PipeWire / PulseAudio）
- **診断:** `melbad doctor` でトークン、CDM、wrapper 疎通、音声出力を一括確認

音源がロスレスであることと、出力が bit-perfect であることは別です。認証管理は [認証仕様](docs/authentication.md)、サービスモードは [Wrapper の制御](docs/wrapper.ja.md) を参照してください。

## bit-perfect

`default` / PipeWire / PulseAudio は共有出力であり、bit-perfect とは判定しません。ソースのサンプルレート・チャンネル数・有効ビット深度に一致する `hw:` 直接オープンのみが `BITPERFECT` を返します。

```sh
melbad probe --json
melbad play fixtures/96k-24bit.flac --device hw:0,0 --require-bitperfect --verify
```

`hw:0,0` は実環境に合わせて置き換えてください。判定表・注意事項・receipt 形式は [音声仕様](docs/audio.md) に記載しています。OS ミキサー（PipeWire）を使う場合の無改変パススルー条件（単独ストリーム・リサンプリングなし・音量 100%）や、サンプルレート切り替え時に HDMI AV アンプで曲頭が切れる現象の回避設定（WirePlumber `api.alsa.start-delay`）は [Shared output (PipeWire) on Linux](docs/audio.md#shared-output-pipewire-on-linux) を参照してください。

## 状態

Apple Inc. の非公式クライアントであり、提携・承認は受けていません。`aarch64` リリースはビルド検証済みですが、arm64 実機での動作検証は未実施です。ログイン・再生可否・出力機器・ロスレス再生は利用環境で確認してください。[更新仕様](docs/updates.md) と [ロードマップ](docs/roadmap.md) も参照してください。

## ドキュメント

- [インストール](docs/installation.md) · [音声と再生](docs/audio.md) · [認証](docs/authentication.md)
- [更新](docs/updates.md) · [外部連携](docs/integrations.md) · [開発](docs/development.md)
- [Wrapper の制御](docs/wrapper.ja.md) · [Roadmap](docs/roadmap.md)
- [公開範囲](PUBLICATION.md) · [第三者ライセンス](NOTICE.md) · [Agent-facing specs](specs/README.md)

## クレジット・ライセンス

デスクトップ UI は Carmine Paolino 氏の [Spotifast](https://github.com/crmne/spotifast) を基にしています（原著作権表示は [`crates/melba/LICENSE`](crates/melba/LICENSE)）。デーモンと共通コードは **MIT または Apache-2.0**、Spotifast 由来のデスクトップ crate は **MIT のみ**です。詳しくは [NOTICE.md](NOTICE.md) を参照してください。

Apple Music などの商標は Apple Inc. に帰属します。Apple のサービス利用には同社の規約が適用されます。認証情報やトークン、サインイン用プロファイルを共有しないでください。
