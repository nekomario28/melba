<p align="center">
  <img width="740" src="docs/assets/melba-album.png" alt="Melba のアルバム画面、曲一覧とプレイヤー操作（デモ用ライブラリ）" />
</p>

# melba — Linux 向けネイティブ Apple Music クライアント

[English](README.md) | **日本語**

Melba は Rust 製の非公式 Apple Music クライアントです。Electron を使わず、独立した再生デーモン `melbad` とネイティブ UI で動作します。

## 主な機能

- **Apple Music:** カタログ検索、アルバム、ライブラリ、プレイリスト、購読曲の再生
- **音声:** ローカルの Widevine CDM による AAC-256、利用者が別途用意する復号サービスによる ALAC/lossless
- **出力:** ALSA の直接出力とフォーマット検証、または PipeWire/PulseAudio の共有出力
- **デスクトップ:** egui、再生キューの保存、MPRIS、16 言語 UI
- **更新:** 対応するポータブルインストール向けの署名付き更新

音源がロスレスであることと、出力が bit-perfect であることは別です。詳しくは [音声仕様（英語）](docs/audio.md) を参照してください。

## インストール

**Linux x86_64 / aarch64 対応。** ソースからのビルドには Rust 1.98 以上や開発用ライブラリが必要です。先に [ディストリビューション別の依存関係（英語）](docs/installation.md) を導入してください。

```sh
git clone https://github.com/nekomario28/melba.git
cd melba
sh packaging/install.sh
melba
```

既定では `~/.local/opt/melba` に `melba` と `melbad` をセットでインストールし、`~/.local/bin` にコマンドへのリンクを作成します。Arch パッケージや別の導入方法は [インストール詳細](docs/installation.md) を参照してください。

ALAC 用の外部サービスもセットアップする場合は、別リポジトリの [bootstrap](https://github.com/nekomario28/melba-wrapper-bootstrap) を利用できます。

```sh
curl -fsSL https://raw.githubusercontent.com/nekomario28/melba-wrapper-bootstrap/v1.2.3/install.sh | bash
```

**外部スクリプトの内容を確認してから実行してください。** ビルド依存が必要です。Melba 本体に復号サービスや rootfs を同梱するものではなく、`sudo` や Apple ID / パスワードの収集、規約への同意も行いません。[Wrapper の制御](docs/wrapper.ja.md) も参照してください。

## 再生

`melba` を起動して **Sign in with Apple Music** を選択します。フルトラックには有効な認証と Apple Music の有料契約が必要です。カタログのプレビューは契約なしでも利用できます。

- **AAC-256（既定）:** 端末に既にある Widevine CDM を使用
- **ALAC / ロスレス（任意）:** 別途用意して起動する互換復号サービスが必要。利用できない場合は AAC にフォールバック可能
- **音声デバイス:** Settings または `melbad setup` で選択。`melbad doctor` で再生に必要な環境を診断

認証とトークンの保存方式は [認証仕様（英語）](docs/authentication.md) を参照してください。

## bit-perfect 出力

直接接続する `hw:X,Y` がソースと互換のあるフォーマットで開かれた場合に限り、Melba は `BITPERFECT` と判定します。`default` / PipeWire / PulseAudio は共有出力であり、bit-perfect と表示しません。

```sh
melbad probe --json
melbad setup
melbad play fixtures/96k-24bit.flac --device hw:0,0 --require-bitperfect --verify
```

`hw:0,0` は環境に応じて変更してください。例にはソース checkout 内の fixture と対応ハードウェアが必要です。判定条件と注意事項は [音声仕様（英語）](docs/audio.md) にあります。

## 制限事項

- Apple Inc. による公式アプリではなく、提携・承認も受けていません。
- AAC には利用可能なローカル CDM と認証が必要です。ALAC の復号サービスは Melba に含まれません。
- 公開 aarch64 バイナリはビルド検証済みですが、実機での動作保証とは異なります。
- ログイン、各曲の再生可否、音質と出力経路は利用環境で確認する必要があります。

## ドキュメント

- [インストール・パッケージ（英語）](docs/installation.md)
- [音声と再生・bit-perfect 検証（英語）](docs/audio.md)
- [認証・トークン管理（英語）](docs/authentication.md)
- [署名と更新（英語）](docs/updates.md)
- [外部連携（英語）](docs/integrations.md)
- [開発・テスト（英語）](docs/development.md)
- [Wrapper の制御・トラブルシューティング（日本語）](docs/wrapper.ja.md)
- [ロードマップ](docs/roadmap.md) · [公開範囲](PUBLICATION.md) · [第三者ライセンス](NOTICE.md)

## クレジット・ライセンス

デスクトップ UI は Carmine Paolino 氏の [Spotifast](https://github.com/crmne/spotifast) を基にしています。原著作権表示は [`crates/melba/LICENSE`](crates/melba/LICENSE) に保持しています。

デーモンと共通コードは **MIT または Apache-2.0**、Spotifast 由来のデスクトップ crate は **MIT のみ**です。依存関係・画像・外部サービスにはそれぞれの利用条件があります（[NOTICE.md](NOTICE.md)）。

Apple Music などの商標は Apple Inc. に帰属します。Apple のサービス利用には同社の規約が適用されます。認証情報やトークン、サインイン用プロファイルを共有しないでください。
