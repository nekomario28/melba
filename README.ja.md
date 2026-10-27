<p align="center">
  <img width="1000" src="docs/assets/melba-album.png" alt="Melba の全画面。アルバム、曲一覧、サイドバー、プレイヤー（デモ用ライブラリ）" />
  <br/><sub>実際のアプリ全画面 · デモ用ライブラリ</sub>
</p>

# melba — Linux 向けネイティブ Apple Music クライアント

[English](README.md) | **日本語**

Melba は Rust 製の非公式 Apple Music クライアントです。Electron を使わず、独立した再生デーモン `melbad` とネイティブ UI で動作します。

## 主な機能

- egui のネイティブ UI、キュー、歌詞、MPRIS、16 言語
- カタログ検索・ライブラリ・プレイリスト。AAC-256 はローカルの Widevine CDM、ALAC は利用者が用意する復号サービス経由（任意）
- ALSA 直接出力とフォーマット検証、対応環境向けの署名付きポータブル更新

音源がロスレスであることと、出力が bit-perfect であることは別です。詳しくは [音声仕様（英語）](docs/audio.md) を参照してください。

## インストール

Linux x86_64 / aarch64 対応。Rust 1.98 以上と開発用ライブラリが必要です。先に [ディストリビューション別の依存関係（英語）](docs/installation.md) を導入してください。

```sh
git clone https://github.com/nekomario28/melba.git
cd melba
sh packaging/install.sh
melba
```

`~/.local/opt/melba` に `melba` と `melbad` をセットで配置し、`~/.local/bin` にコマンドリンクとデスクトップエントリを作成します。`melbad` はデスクトップが必要時に起動します。パッケージや別の導入方法は [インストール詳細](docs/installation.md) を参照してください。

ALAC 用の外部サービスは [source-only bootstrap](docs/installation.md#optional-source-only-wrapper-bootstrap) を参照してください。wrapper・rootfs・アカウント情報は Melba に同梱されません。

## 再生

`melba` を起動し **Sign in with Apple Music** を選択します。フルトラックには有効な認証と Apple Music の有料契約が必要です。プレビューは契約なしでも再生できます。出力は Settings または `melbad setup` で選択します。

- **AAC-256（既定）:** 端末に既にある Widevine CDM を使用
- **ALAC / ロスレス（任意）:** 別途用意して起動する互換復号サービスが必要。利用できない場合は AAC にフォールバック可能
- **出力:** 直接の `hw:X,Y`、または共有の `default` / PipeWire / PulseAudio

認証とトークン管理は [認証仕様（英語）](docs/authentication.md)、サービスモードは [Wrapper の制御](docs/wrapper.ja.md) を参照してください。

## bit-perfect

`default` / PipeWire / PulseAudio は共有出力であり、bit-perfect とは表示しません。ソースのレート・チャンネル数・有効ビット深度に一致する `hw:` 直接オープンだけが `BITPERFECT` になり得ます。

```sh
melbad probe --json
melbad play fixtures/96k-24bit.flac --device hw:0,0 --require-bitperfect --verify
```

`hw:0,0` は環境に応じて変更してください。判定表・注意事項・receipt は [音声仕様（英語）](docs/audio.md) にあります。

## 状態

Apple Inc. の公式アプリではなく、提携・承認も受けていません。aarch64 リリースはビルド検証済みで、arm64 実機での動作検証は未実施です。ログイン、曲の再生可否、出力ハードウェア、ロスレス再生は利用環境で確認してください。[更新](docs/updates.md) と [ロードマップ](docs/roadmap.md) も参照してください。

## ドキュメント

- [インストール](docs/installation.md) · [音声と再生](docs/audio.md) · [認証](docs/authentication.md)
- [更新](docs/updates.md) · [外部連携](docs/integrations.md) · [開発](docs/development.md)
- [Wrapper の制御](docs/wrapper.ja.md) · [Roadmap](docs/roadmap.md)
- [公開範囲](PUBLICATION.md) · [第三者ライセンス](NOTICE.md) · [Agent-facing specs](specs/README.md)

## クレジット・ライセンス

デスクトップ UI は Carmine Paolino 氏の [Spotifast](https://github.com/crmne/spotifast) を基にしています。原著作権表示は [`crates/melba/LICENSE`](crates/melba/LICENSE) に保持しています。デーモンと共通コードは **MIT または Apache-2.0**、Spotifast 由来のデスクトップ crate は **MIT のみ**です。詳しくは [NOTICE.md](NOTICE.md) を参照してください。

Apple Music などの商標は Apple Inc. に帰属します。Apple のサービス利用には同社の規約が適用されます。認証情報やトークン、サインイン用プロファイルを共有しないでください。
