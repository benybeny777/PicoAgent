# UI契約（フロントとバックエンドの約束）

画面（`ui/`）とバックエンド（`src-tauri/`）の間で交わす command・イベントの正本です。**画面を作り直すときは、旧フロントのコードを読まずにこの文書だけで同じ機能を組み立てられる**ことを目的にします。仕様の説明は [SPEC.md](../SPEC.md)、設定キーは [SETTINGS.md](SETTINGS.md) が正本で、ここは「何を呼べて、何が届くか」だけを持ちます。

## 守ること

- メイン画面のモジュール（`ui/app/`・`ui/character/`・`ui/chat/`・`ui/sources/`）から Tauri を呼ぶのは `ui/app/bridge.js` だけにする。`window.picoagent` や `window.__TAURI__` を他のモジュールが直接触ると `tools/tests/ui-architecture-contract.mjs` が落とす。
- `window.picoagent`（`ui/picoagent.js`）はメイン画面・設定画面・キャラ確認画面が共有する低レベルの入口で、invoke 失敗を `app.log` へ残す。`bridge.js` はその上に名前付きの関数を並べるだけで、業務処理を持たない。
- command を足す・消す・引数を変えるときは、この文書の表を同じ作業で直す。`src-tauri/src/ui_contract.rs` のテストが、`invoke_handler` に登録された command とこの文書の表を双方向に突き合わせる（表に無い command、登録されていない command のどちらも失敗）。
- 引数名は JavaScript 側の名前（camelCase）で書く。Tauri が Rust の snake_case へ変換する。
- 戻り値の `Value` は JSON オブジェクト。キーは下の各行に書いたものが最低限の約束で、増やすのは互換、消す・意味を変えるのは非互換として扱う。

## command

### メイン画面

| command | 引数 | 戻り値・役割 |
|---|---|---|
| `pet_frontend_ready` | なし | なし。フロントを読み込めた合図。届かないと起動監視が診断ページへ切り替える |
| `pet_boot_progress` | なし | なし。起動中の生存通知（`pet_init` の `bootProgressIntervalMs` 間隔） |
| `pet_init` | なし | 起動時の状態一式。`layout`・`scenes`（`PICOAGENT_SCENES` を正規化したもの）・`characterId`・`characterOverrides`・`live2dSdkOverrides`・`history`・`historySaveBlocked`・`providerId`・`up`・`theme`・`uiTheme`・`fontPx`・`voiceVolume`・`ttsEnabled`・`gazeTrackingEnabled`・`quickActions`・`napMinutes`・`stream*`（背景・ディスプレイ・字幕色）・`chatter*`・`onboardingDone`・`relationship`・`bondLabel`・`relationshipMemoryEnabled`・`relationshipError`（読込失敗理由、成功時null）・`bootProgressIntervalMs`・`betaBuild` |
| `pet_finish_boot` | なし | なし。保存したウィンドウ寸法・位置を戻す（起動画面から通常画面へ移る直前） |
| `pet_reveal_main` | なし | なし。窓を表示する唯一の入口 |
| `pet_send` | `text` | 返答 `{ reply, error?, choices?, quitApp? }`。生成中は `pet:stream_chunk` などのイベントが届く |
| `pet_send_image` | `text`, `images`（data URL 配列） | `pet_send` と同じ |
| `pet_cancel` | なし | なし。実行中の会話を次の区切りで止める |
| `pet_capture_screen` | なし | `{ dataUrl }` |
| `pet_read_file_content` | `name`, `dataB64` | 抽出した本文（文字列） |
| `pet_read_dropped_path` | `path` | `{ kind: "image" \| "file", name, dataUrl, path }` |
| `pet_transcribe_audio` | `dataB64`（WAV の data URL） | `{ text }` |
| `pet_speak` | `text` | `{ enabled, skipped?, audio?（バイト配列）, mime?, lipSync? }` |
| `aivis_status` | なし | `{ enabled, ... }` |
| `pet_prepare_voice_model` | なし | 選択中キャラの声を準備 |
| `pet_export_history` | `markdown` | `{ path }` |
| `pet_idle_chatter` | なし | `{ enabled, reply? }` |
| `pet_daily_greeting` | なし | `{ reply? }` |
| `pet_daily_events` | なし | `{ events: [{ text }], broken: [{ text?, label?, due_at? }] }` |
| `pet_alarms` | なし | `{ alarms: [...] }` |
| `pet_alarm_wake` | `id` | `{ reply? }` |
| `pet_set_character_profile` | `characterId`, `profile`, `greeting`, `voice` | 確定したキャラの配信表示 `{ streamBackground, streamBackgroundImage, streamDisplay, streamDisplayImage, streamCaptionColor }` |
| `pet_user_characters` | なし | `{ characters: [{ id, name, description, path }] }`（利用者が追加したキャラ） |
| `pet_save_scene` | `scenes` | なし。`PICOAGENT_SCENES` を正規化して保存する。壊れた入力は保存せずエラー |
| `pet_save_ui_pref` | `key`, `value` | なし。メイン画面で変えた見た目だけを保存する。対象は `PICOAGENT_BUBBLE_FONT_PX`（10〜28の整数文字列）、`PICOAGENT_STREAM_CHAT_FONT_PX`（18〜48）、`PICOAGENT_TITLE_BAR_PINNED`（真偽値）。それ以外のキー・範囲外は保存せずエラー |
| `pet_menu` | `context`（`{ selection, editable, link, character }` の真偽値）、`x`・`y`（窓内の論理座標） | なし。右クリック位置に統一メニューを出す |
| `pet_layout_menu` | `x`, `y`（ウィンドウ左上基準の論理座標） | なし。上端メニュー「表示」の一覧 |
| `pet_character_menu` | `x`, `y`, `characters`（`[{ id, name }]`） | なし。上端メニュー「キャラ選択」の一覧 |
| `pet_start_window_drag` | なし | なし。OS 標準のウィンドウ移動を始める |
| `pet_place_window` | `position`（`left` / `right` / `current` / `primary`） | なし |
| `pet_reset_layout` | なし | なし。ウィンドウ位置・寸法と現在の形式のシーン保存値を既定へ戻し、`pet:layout-reset` を送る |
| `pet_open_settings` | `tab`（省略可） | なし |
| `pet_open_external_url` | `url` | なし（http / https だけ） |
| `pet_open_generated_media` | `path` | なし（生成物の管理領域だけ） |
| `pet_config_presence` | `keys`（`PICOAGENT_` で始まるキー、32件まで） | `{ キー: 値が入っているか }` |
| `pet_relationship_state` | なし | `{ relationship, bondLabel }` |
| `pet_relationship_event` | `kind`, `detail` | `{ relationship, bondLabel, memoryEnabled? }` |
| `pet_relationship_pin_moment` | `momentId`, `pinned` | `{ relationship, bondLabel }` |
| `pet_relationship_delete_moment` | `momentId` | `{ relationship, bondLabel }` |
| `pet_relationship_clear` | なし | `{ relationship, bondLabel }` |
| `pet_complete_onboarding` | なし | `{ relationship, bondLabel }` |
| `pet_show_onboarding` | なし | なし。`pet:show-onboarding` を送る |
| `pet_schedule_runs` | なし | スケジュール実行履歴の配列 |
| `pet_rerun_schedule_task` | `taskId` | 実行結果 |
| `pet_durable_jobs` | なし | 永続ジョブの配列 |
| `pet_durable_job_retry` | `id` | なし |
| `pet_durable_job_cancel` | `id` | なし |
| `pet_project_rule_undo` | `rule` | 取り消せたか（真偽値） |
| `pet_request_gpu_cache_reset` | `reason` | なし。次回起動で描画キャッシュを消す予約 |
| `pet_game_ready` | なし | ゲームを始められるか |
| `pet_game_judge` | `request` | 採点結果 |
| `pet_emotion` | なし | 感情状態 |
| `pet_quit` | なし | なし |
| `pet_log` | `level`, `message`, `detail` | なし。`app.log` へ書く |
| `engine_status` | なし | 同梱推論エンジンの状態 `{ phase, percent?, receivedBytes?, totalBytes?, message? }` |

### 設定画面・キャラ確認画面

| command | 引数 | 戻り値・役割 |
|---|---|---|
| `settings_load` | なし | 設定一式と各コネクタのスキーマ |
| `settings_save` | `patch`, `baseRevisions` | 保存結果 |
| `settings_list_models` | `id` | モデル名の配列 |
| `settings_test_tool` | `name` | 接続確認の結果 |
| `settings_memory_list` | なし | 長期メモリの一覧 |
| `settings_memory_delete` | `category`, `text`, `updatedAt` | 削除結果 |
| `settings_components_status` | なし | 追加機能の状態の配列 |
| `settings_components_install` | `ids`, `enableTts`, `acceptedLicenseIds` | 導入結果。進捗は `component:progress` |
| `settings_components_remove` | `id` | 削除結果 |
| `settings_generation_environment` | なし | 画像・動画生成の実行環境 |
| `settings_check_update` | なし | 更新の有無 |
| `settings_install_update` | なし | 更新の実行結果 |
| `settings_character_pick` | `folder` | 選んだパス |
| `settings_character_discover_downloads` | なし | ダウンロードフォルダ内のMMD・Live2D候補 |
| `settings_character_install` | `path` | 導入結果 |
| `settings_character_install_live2d` | `path`, `id`, `name` | 導入結果 |
| `settings_character_install_mmd` | `path`, `id`, `name` | 導入結果 |
| `settings_character_rename` | `id`, `name` | 変更結果 |
| `settings_character_remove` | `id` | 削除結果 |
| `settings_character_maker_pick_source` | なし | 選んだ元画像 |
| `settings_character_maker_list` | なし | 作成中キャラの一覧 |
| `settings_character_maker_create` | `input` | 作成したキャラの定義 |
| `settings_character_maker_run` | `characterId`, `fromStage` | 実行後のキャラの定義。進捗は `character-maker:progress` |
| `settings_character_maker_cancel` | なし | 中止結果 |
| `settings_character_maker_install` | `characterId` | 導入結果 |
| `settings_character_maker_preview` | `characterId` | 確認画面を開く |
| `settings_character_maker_preview_data` | `characterId` | 確認画面の描画データ |
| `settings_character_learning_pick` | `folder` | 音声素材または既存学習環境の選択。取消はpathなし |
| `settings_character_learning_list` | なし | Windows対応・管理環境の確認済み状態・学習一覧・実行中ID |
| `settings_character_learning_start` | `input`, `resumeId` | 素材から候補を作成、または同一ジョブを再開。進捗は一覧に含める |
| `settings_character_learning_cancel` | `id` | 実行中の学習の停止要求 |
| `settings_character_learning_setup` | なし | Windows/NVIDIAの専用学習環境を明示導入・確認 |
| `settings_character_learning_check_voice` | `id` | 候補をAivisへ登録し、30文の品質検査結果を保存 |
| `settings_character_learning_sample` | `id`, `index` | 学習ジョブ内の試聴WAVのdata URL・文・件数 |
| `settings_pick_stream_image` | `kind` | 取り込んだ画像のパス |
| `settings_export_diagnostics` | なし | 書き出した診断ファイル |
| `restart_app` | `restartEngine`, `restartAivis` | 再初期化の結果。メインへ `pet:reload` を送る |
| `pet_list_downloaded_models` | なし | 取得済みモデルの一覧 |
| `pet_delete_downloaded_model` | `name` | 削除結果 |
| `pet_usage` | なし | トークン使用量 |
| `pet_traces` | なし | 実行記録 |

## イベント（バックエンド → 画面）

| イベント | 中身 | 受け手の役割 |
|---|---|---|
| `pet:ui` | 変わった設定だけのオブジェクト（`theme`・`uiTheme`・`fontPx`・`voiceVolume`・`ttsEnabled`・`gazeTrackingEnabled`・`characterId`・`characterOverrides`・`live2dSdkOverrides`・`layout`・`stream*`・`chatter*`・`quickActions`・`relationshipMemoryEnabled`） | 設定保存を画面へ反映する |
| `pet:layout` | 形式ID | 表示形式を切り替える（ウィンドウ寸法はバックエンドが当て済み） |
| `pet:layout-reset` | `{ scenes }` | 位置とサイズのリセット後のシーンへ置き換える |
| `pet:layout-edit` | `"all"` \| `"character"` | 配置の編集（全ソース）またはキャラの直接調整を始める |
| `pet:context-action` | `{ action, framing? }`。`action` は `copy` / `cut` / `paste` / `select-all` / `open-link` / `copy-link` / `edit-character` / `set-framing` / `reset-character` | 右クリックした時点に控えた選択・要素に対して実行する |
| `pet:reset` | なし | 会話の表示と控えを捨てる |
| `pet:reload` | なし | 画面を読み込み直す（設定反映後の再初期化） |
| `pet:notice` | `{ kind, title, body, ok, taskId?, runId?, jobId?, state?, progress?, rule?, key?, at? }` | 通知センターへ積む。`kind=durable_job` は `jobId` で上書き、`kind=background_failure` は原因ごとに1度だけトースト。`ok=true`で復旧を解除し次の失敗を知らせる |
| `pet:model` | 表示用のモデル名 | 状態行に短く出す |
| `pet:emotion` | `{ mood, energy }` | キャラの感情表示 |
| `pet:hotkey` | `"summon"` | 入力欄を開く |
| `pet:game-open` | なし | ゲーム画面を開く |
| `pet:show-onboarding` | なし | 初回案内を出し直す |
| `pet:agent_status` | 進捗の文字列 | 状態行・実行中タスクへ出す |
| `pet:agent_activity` | `{ phase, tools, failedTools?, labels?, success? }` | 実行中タスクの行を組み立てる |
| `pet:stream_chunk` | 文字列 | 生成途中の本文 |
| `pet:stream_reset` | なし | 途中本文を捨てる（ツール実行を挟んだ再生成） |
| `engine:status` | `engine_status` と同じ | 送信可否と状態行 |
| `component:progress` | `{ id, downloaded, total }` | 追加機能の取得進捗 |
| `settings:navigate` | タブID | 設定画面のタブ移動 |
| `character-maker:state` / `character-maker:progress` | 作成状態 | 設定画面のキャラ作成 |
| `tauri://drag-enter` / `tauri://drag-leave` / `tauri://drag-drop` | `{ paths }` | ファイルのドロップ添付 |

学習input.asrDeviceはcuda/cpu。新規画面はcudaを既定に送信し、旧保存ジョブの省略だけcpuとして解釈する。URL・音声ファイルは同じ選択を使い、GPU失敗時の自動CPU切替は行わない。

通知の組み立てと送信は `notice.rs` が所有し、`at` はローカル時刻のRFC 3339。種類固有のキーは維持する。起動中の通知は待機し、メイン画面が `pet:notice` の購読完了を待ってから `pet_init` の末尾で待機通知を `pet:notice` へ順番に送る。待機列と新着通知は同じロック内で直列化し、失敗と復旧の順序を守る。画面の再読込時は未復旧の失敗も再送する。復旧の無い一度きりの出来事（状態ファイルの破損退避 `kind=state_quarantined`、起動時の更新確認の失敗 `kind=update` の `ok=false`）は `background_failure` にせず、通知一覧へ1件残す。会話履歴の保存停止はキャラごとに管理し、止めているキャラが1体も無くなったときだけ解除（`ok=true`）を送る。`relationshipError` は `relationship-load` の原因キーで画面へ知らせる。

`pet_set_character_profile` の `voice` は読み込めた `character.json` の `voice` オブジェクトまたは `null`。ID・人格・挨拶・声を同じスナップショットとして登録し、声の検証・関係性・履歴の読込に失敗した場合は旧組を温存する。Aivisの `speakerUuid` と `modelUuid` はUUID形式、`styleId` は数値文字列を保持する。声の解決に失敗した `pet_prepare_voice_model` と `pet_speak` は理由付きエラーを返し、別の声で合成しない。
