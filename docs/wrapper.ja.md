# Wrapper（ALAC 復号サービス）の制御

[English](wrapper.md) | **日本語**

Melba は wrapper を**ダウンロード・インストール・同梱・再配布しません**。
この文書は、利用者が用意して既にインストールしたサービスの制御だけを扱います。
payload とその利用条件はこのリポジトリの外です（[NOTICE.md](../NOTICE.md)、
[PUBLICATION.md](../PUBLICATION.md)）。

## モード

| 設定 | 意味 |
|---|---|
| `"wrapper_mode": "managed"` | インストール済みの managed service を必要時に起動・停止する。インストールや取得はしない。 |
| `"wrapper_url": "tcp://…"` | 利用者が起動したサービスへ接続する。プロセス制御はしない。 |
| どちらもなし | managed service があれば自動検出して起動。なければ ALAC は AAC にフォールバックする。 |

`managed` は**プロセスの所有**を意味し、インストールを意味しません。staging
済みの managed service は key がなくても検出されるため、bootstrap を先に
入れて設定を編集せずに使えます。

## 対応サービス

managed path は、Melba の managed directory に staging された upstream の
**WorldObservationLog/wrapper** bundle を使います。GUI のネイティブ認証
bridge を検証済みなのはこのサービスだけです。他のサービスは drop-in
代替ではありません。

| サービス | Melba との互換性 |
|---|---|
| WorldObservationLog/wrapper (managed) | 対応 managed path。GUI ネイティブログイン ABI |
| WorldObservationLog/wrapper-lite (HTTP key) | `wrapper_url` 経由の再生。GUI ネイティブログインは未確立 |
| glomatico/wrapper-v2 (TCP decrypt) | `wrapper_url` 経由の再生。GUI ネイティブログインは未確立 |
| aria / downloader 系 | download pipeline であり、Melba 用のライブ復号 endpoint ではない |
| 自作プロトコル shell | 再生は可能。ネイティブ GUI ログイン ABI はない |

サービスの取得・インストールは明示的で、自動では行いません。source-only の
bootstrap は
<https://github.com/nekomario28/melba-wrapper-bootstrap> にあり、upstream
bundle をローカルに取得するだけで再配布はしません。この表は互換性の話で、
利用許諾ではありません。bootstrap 側の README・license・security も参照して
ください。

## 実行の流れ

1. ALAC stream を開くと、サービスの key port を確認する。
2. managed で port が閉じていれば、インストール済みサービスを起動して port を
   待つ。サービスがない、または早期終了した場合は AAC にフォールバックし、
   理由を記録する。
3. サインインは GUI 内で行う。対応 managed service があれば、ネイティブの
   prompt を private な GUI auth path 経由で中継する。なければ browser flow で
   AAC にサインインする。独立した CLI login 手順はない。
4. `--require-bitperfect` と output picker は wrapper とは独立している。ALSA
   path が exact か shared かだけを決める。

## コマンド

```sh
melbad wrapper status   # 配置先、binary、key port の到達性
melbad wrapper start    # インストール済みサービスを起動（debug/foreground parent）
melbad wrapper stop     # managed process を停止し、key port を確認
melbad auth status      # ローカル token の有無/期限。token は表示しない
melbad setup            # audio output のみ。wrapper には触れない
```

transport は `melbad ctl ...` または MPRIS を使います。live の account /
wrapper database は編集しないでください。

## トラブルシューティング

- key port が到達可能なのに `running: no` の場合、別のサービスが port を所有
  しています。Melba は管理外のサービスを kill しません。
- `complete GUI sign-in before using the managed wrapper` は、キャッシュされた
  binding が tombstone であることを意味します。GUI サインインを完了するか、
  `melbad auth clear` を実行してください。
- `wrapper exited early` は managed な `wrapper.log` に記録されます。lifecycle
  行だけを確認し、account identifier・token・signed URL を貼らないでください。
- ALAC が失敗した場合、再生はフォールバックを報告して AAC を継続します。

## 境界

- サービスは loopback に bind してください。port は通常、認証されません。
- account / wrapper database はローカルの owner-only ファイルです。diagnostics
  や log に添付しないでください。
- サービスの license、Apple の規約、Melba の license は別です。
- Melba は client/protocol と ALAC decode path を含みますが、FairPlay key
  derivation は含みません。インストールを法的な clearance として説明しないで
  ください。
