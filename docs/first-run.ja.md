# Linuxで初めて再生する

[English](first-run.md) · [README](../README.ja.md) · [インストール](installation.md)

MelbaにはApple Musicのログイン情報、Widevine CDM、オプションのALAC復号サービスは同梱されません。**インストールやGUI起動の成功だけでは、フルトラック再生の確認にはなりません。**

## まずAAC再生を確認

1. アプリケーションメニューからMelbaを起動するか、通常のポータブル版なら `~/.local/opt/melba/melba` を実行します。**Sign in with Apple Music**を選びます。フルトラック再生には有効なApple Music契約が必要です。
2. 最初はSettingsで共有のシステム音声出力を選びます。`default` はPipeWire/PulseAudio向けで、`hw:` は排他使用のための出力です。必要なら `melbad config audio-device default` を実行します。
3. **Settings → Stream quality → AAC 256**、または `melbad config audio-quality aac` を選びます。これでオプションのALACサービスに依存せず確認できます。
4. カタログの曲を**30秒を超えて**再生します。検索、アートワーク表示、30秒のプレビュー、ログイン成功だけではフルトラック再生の証拠にはなりません。
5. 再生できない場合は `~/.local/opt/melba/melbad doctor` を実行し、認証・開発者トークン・Widevine CDM・音声出力の診断結果を確認します。秘密情報は共有しないでください。

## 主な問題と確認方法

| 症状 | 確認すること |
| --- | --- |
| 30秒しか再生されない | 契約・セッション・Widevine CDMを確認。MelbaはCDMをダウンロードしません。`melbad doctor` を実行します。 |
| ブラウザではログインできる | `melbad auth open` と `melbad auth status` を確認。一部のブラウザ保存形式は非対応です（[認証仕様](authentication.md)）。 |
| 無音・デバイス使用中・停止 | まず `default` を使い、`melbad probe` と `melbad doctor` を確認。`hw:` は他のアプリと競合します。 |
| ALACが使えない | 外部ランタイムの任意導入が必要です（[ALAC手順](installation.md#enable-alac-lossless)）。配置できても再生成功は保証されません。 |
| `melbad` が見つからない | 通常のポータブル版なら `~/.local/opt/melba/melbad` を使うか、`~/.local/bin` をPATHに追加します。 |

## 不具合の報告

[GitHub Issue](https://github.com/nekomario28/melba/issues/new/choose)にMelbaのバージョン（`melba --version`）、ディストリビューション、Wayland/X11、再現手順、問題の種類（ログイン・AACフルトラック・音声出力）を記入してください。診断の失敗項目は記載できますが、**トークン・パスワード・Cookie・アカウント識別子・認証ファイル・署名付き配信URL・秘密情報を含むログを公開しないでください**。

開発者向けには、初回ログイン・セッション復元・期限切れ/拒否・ログアウト・フルトラック再生・曲送り・復帰について、同意を得た検証用アカウントによる実機確認がまだ必要です。CIやコンテナ起動テストだけでは代替できません。
