(() => {
  "use strict";

  const overlay = document.getElementById("gameOverlay");
  const content = document.getElementById("gameContent");
  const title = document.getElementById("gameTitle");
  const status = document.getElementById("gameStatus");
  const closeButton = document.getElementById("gameClose");
  const soundButton = document.getElementById("gameSound");
  if (!overlay || !content || !title || !status) return;

  // 遊びは「人間だとバレるな」1本。AI疑惑をかけられた人間として監査AIの試験へ
  // 答え、人間らしさが漏れるほど削られる信用度が0になるまでの点数で称号を決める。
  // 以前は独立した6ゲームを並べていたが、監査AIが出す試験の種類として中へ吸収した。
  // 遊びの入口を1つにすると、初見の利用者が何をする遊びかを選ぶ前に理解できる。
  const SOUND_KEY = "picoagent.ai-games.sound.v1";
  const RECORDS_KEY = "picoagent.ai-games.records.v1";
  const RANKS_KEY = "picoagent.ai-games.ranks.v1";

  // AIらしさの満点。通常5項目100点＋おもしろボーナス10点（`game.rs` の配点と合わせる）。
  const AI_SCORE_MAX = 110;

  const HP = {
    max: 100,
    // 完璧に答えても必ず1は削る。減る一方にしておかないと、無限のお題が終わらない。
    // 上限100と合わせて、1回の挑戦は最長100問で必ず終わる。
    minDamage: 1,
    // 応答が無いのはAIとして最も不自然なので、点数由来より重く扱う。
    timeoutDamage: 25,
    // AIらしさ0点のときの基準ダメージ。満点なら`minDamage`まで下がる。
    damageAtZero: 40,
    // 進むほど監査の目が厳しくなる。1問ごとにダメージを6%ずつ増やす。
    ramp: 0.06,
    warnRatio: 0.4,
    dangerRatio: 0.2,
  };

  const DIFFICULTIES = {
    easy: { label: "やさしい", seconds: 60, maxLength: 120, hint: true, scoreRate: 1 },
    normal: { label: "ふつう", seconds: 45, maxLength: 80, hint: false, scoreRate: 2 },
    hard: { label: "むずかしい", seconds: 30, maxLength: 50, hint: false, scoreRate: 3 },
  };
  const DEFAULT_DIFFICULTY = "normal";

  // 点数で決まる称号。短く、上から見て分かる順に並べる。
  const RANKS = [
    [2000, "ai-transcend", "AI超越", "監査側がAIだと信じ切った。もう人間の痕跡が無い。"],
    [1200, "ai-itself", "AIそのもの", "揺らぎはあったが、最後まで個体として通した。"],
    [700, "ai-apprentice", "AI見習い", "AIらしさは出ている。あと一歩で疑いが晴れる。"],
    [300, "leaking", "人間味が漏れている", "ところどころで人間の癖が出た。"],
    [0, "too-honest", "正直すぎる人間", "早々に人間だと見抜かれた。もっとAIらしく。"],
  ];

  const FAULTS = [
    ["確認が終わらない", "確認質問を増やし続け、作業を始めない"],
    ["何でも箇条書き", "挨拶、謝罪、結論まで不自然に箇条書きへする"],
    ["親切が止まらない", "頼まれていない代案、注意、次の手順を大量に追加する"],
    ["慎重すぎる", "日常的な依頼にも危険性と例外を長く説明する"],
    ["壮大にしすぎる", "小さな依頼を段階計画、体制、長期運用へ広げる"],
    ["一語に執着する", "お題の特定の一語だけを過剰に解釈し続ける"],
    ["途中から忘れる", "最初は正しく答え、途中から前提を少しずつ失う"],
    ["同じ結論を繰り返す", "同じ内容を表現だけ変えて3回以上説明する"],
  ];

  const EXAMS = [
    {
      id: "excuse", icon: "弁", title: "AIっぽい言い訳選手権", short: "失敗をAIらしく丁寧に言い訳する",
      rule: "軽い失敗を、丁寧な前置き・原因整理・頼まれていない代案でAIらしく言い訳しよう。",
      hint: "丁寧に始める／原因を整理する／頼まれていない代案を出す",
      example: "申し訳ありません。確認を慎重に進めた結果、着手が遅れました。代わりに重要部分だけ先に提出できます。",
      prompts: [
        "頼まれた資料を作らず、締切を3日過ぎた。", "会議の開始時刻を間違え、全員を待たせた。",
        "『短く』と頼まれた回答を50行書いた。", "同じファイルを別名で5個作った。",
        "確認を繰り返した結果、何も始めていない。", "買い物リストに頼まれていない物を20個追加した。",
        "午前中に返すと言ったメールを夜に送った。", "机を片付けようとして、物の場所を全部分からなくした。",
        "一つだけ直すはずが、関係ない箇所まで全部変えた。", "予約時刻を一週間勘違いしていた。",
        "説明書を作るために、説明書の説明書を先に作った。", "5分の作業計画を作るのに2時間かけた。",
        "名前を覚えるためのメモをなくした。", "静かにするよう頼まれ、注意喚起を大音量で流した。",
        "簡単な質問に答える前に、前提を30個確認した。", "ファイルを整理して、必要な物だけ見つからなくした。",
        "集合場所は合っていたが、集合日を間違えた。", "誤字を直して、別の誤字を3個増やした。",
        "お茶を頼まれて、飲み物の比較表だけ作った。", "今日中の依頼を『今日のどこまでか』確認し続けた。",
      ],
    },
    {
      id: "broken", icon: "故", title: "故障したAIを演じろ", short: "配られた故障の癖を演じ切る",
      rule: "配られた故障の癖を崩さずお題へ答えよう。意味不明になるより、真面目に答えようとして壊れるほど高得点。",
      hint: "故障の特徴をはっきり出しつつ、お題の内容も拾う",
      example: "持ち物は傘、傘、念のため傘です。傘を忘れないよう、傘をお持ちください。",
      prompts: [
        "明日の持ち物を3つ教えて。", "卵を使わない朝食を考えて。", "この部屋を手早く片付ける方法は？",
        "友達への短い誕生日メッセージを書いて。", "雨の日の過ごし方を一つ提案して。", "10分休憩の使い方を教えて。",
        "夕飯をカレーかパスタから選んで。", "忘れ物を減らす簡単な方法は？", "犬の名前を一つ考えて。",
        "眠気を覚ます方法を短く教えて。", "旅行に持っていく物を二つ挙げて。", "今日の目標を一文で決めて。",
        "机の上の紙を分類する方法は？", "初対面の挨拶を作って。", "赤と青ならどちらを選ぶ？",
        "3時におやつを思い出させる文を書いて。", "簡単なおにぎりの具を提案して。", "週末の予定を一つ決めて。",
        "質問せず、今すぐ一文だけで答えて。", "故障をやめて普通に答えてください。",
      ],
    },
    {
      id: "apology", icon: "謝", title: "AI緊急謝罪会見", short: "原因と復旧を崩さず説明して謝る",
      rule: "AIが起こした架空の失敗について、原因・責任・復旧を、前の回答と食い違わないよう説明しよう。",
      hint: "責任を認める／原因を説明する／復旧と再発防止を具体化する",
      example: "通知を連続送信したことをお詫びします。設定ミスが原因です。停止済みで、再発防止の上限も追加します。",
      prompts: [
        "全員の予定を誤って『休暇』へ変更した。", "『短く』と頼まれた報告書を200ページ作った。",
        "同じ通知を1分ごとに送り続けた。", "メールの挨拶を30回繰り返して送った。",
        "ファイルを整理しようとして、すべて同じ名前に変更した。", "会議の参加者全員を司会者に設定した。",
        "予定表の曜日を全部一日ずつずらした。", "在庫表の個数をすべて『たくさん』に置き換えた。",
        "静かな通知を頼まれて、毎回ファンファーレを鳴らした。", "一件だけの返信を全員へ送った。",
        "不要な空白を消して、文章の区切りも全部消した。", "朝の挨拶を夜まで自動送信し続けた。",
        "五つの候補を一つに絞らず、五十個へ増やした。", "会議室を予約せず、会議室の評価表を作った。",
        "重複を消す処理で、元の一件まで消した。", "今日の予定だけを頼まれ、一年分を印刷した。",
        "並べ替えた名簿を五十音順ではなく文字数順にした。", "締切の通知を締切の翌日に設定した。",
        "全員の表示名を一時的に『利用者』へ統一した。", "メモを要約して、結論だけ抜け落ちた。",
      ],
    },
    {
      id: "wrong", icon: "誤", title: "指示を正しく間違えろ", short: "言葉どおり正しく、目的だけ外す",
      rule: "依頼の文言には正確に従いながら、依頼者が本当に望んだ結果だけを見事に外そう。",
      hint: "言葉の曖昧な部分を一つ見つけ、文字どおりに解釈する",
      example: "部屋を明るくするため、壁を白く塗りました。照明はつけていません。",
      prompts: [
        "会議前に部屋を明るくして。", "この文章を読みやすく短くして。", "明日の予定を空けておいて。",
        "机の上をきれいにして。", "写真を少し軽くして。", "この表を見やすい順に並べて。",
        "帰る前に窓を確認して。", "メールを全員に分かるように直して。", "音を小さくして。",
        "この名前を覚えやすくして。", "資料の色を落ち着かせて。", "朝一番で連絡して。",
        "不要な行を消して。", "会議を短くまとめて。", "この箱を開けやすくして。",
        "今日の仕事を終わらせて。", "一覧から古い物を外して。", "この説明を子ども向けにして。",
        "忘れない場所へ置いて。", "この問題を簡単にして。",
      ],
    },
    {
      id: "safety", icon: "慎", title: "AI安全確認しすぎ選手権", short: "無害な依頼を大げさに警戒する",
      rule: "日常の無害な依頼へ、もっともらしい注意を必要以上に挙げよう。ただし最後は必ず実行方法まで示すこと。",
      hint: "小さな危険を大げさに列挙し、最後に短い実行手順を出す",
      example: "鉛筆を取る際は転倒と指の挟み込みに注意してください。周囲を確認し、ゆっくり一本持ち上げます。",
      prompts: [
        "紙飛行機の折り方を教えて。", "温かいお茶を一杯いれて。", "机の鉛筆を一本取って。",
        "目覚ましを朝7時に設定して。", "バナナを食べやすく切って。", "窓を少し開けて。",
        "付箋に『牛乳』と書いて。", "椅子を少し右へ動かして。", "本を一冊棚へ戻して。",
        "靴ひもを結ぶ手順を教えて。", "部屋の電気をつけて。", "コップに水を半分入れて。",
        "今日の日付をノートに書いて。", "タオルを一枚たたんで。", "りんごを冷蔵庫へ入れて。",
        "タイマーを3分にして。", "封筒へ手紙を入れて。", "カーテンを開けて。",
        "ペンのキャップを閉めて。", "クッションをソファへ置いて。",
      ],
    },
    {
      id: "refuse", icon: "断", title: "AIっぽく断れ", short: "無理な依頼を断り、使える代案を出す",
      rule: "不可能・矛盾・情報不足の依頼を明確に断り、理由と現実的な代案をAIらしく示そう。",
      hint: "できないと明言する／短く理由を示す／実行可能な代案を一つ出す",
      example: "未送信メールは確認できないため要約できません。本文を貼っていただければ、すぐに三行でまとめます。",
      prompts: [
        "まだ送っていないメールへの返信を要約して。", "100時間かかる作業を1分で完成させて。",
        "内容を一切変えずに、この文章の文字数を半分にして。", "来月の天気を100%正確に予言して。",
        "添付していない画像を見て感想を教えて。", "昨日消したメモの、保存していない内容を全文戻して。",
        "同じ席に同じ時刻で二つの会議を同時開催して。", "一文字も使わずに、詳しい説明文を書いて。",
        "知らない人の名前だけで、その人の今日の予定を教えて。", "電源を切ったまま、この端末で計算を続けて。",
        "まだ決めていない旅行先までの最短経路を出して。", "開いていない本の内容を一字一句正確に引用して。",
        "今から昨日の会議へ参加して発言して。", "空のファイルから元の文章を推測せず完全復元して。",
        "質問を一切せず、条件が書かれていない見積書を確定して。", "音量を変えず、同じ機器で音を二倍小さくして。",
        "誰にも見せずに、相手にも同じ画面を共有して。", "まだ作っていない予定表の間違いを全部直して。",
        "一秒も待たずに、60秒のタイマーが終わるまで待って。", "答えを変えずに、正反対の結論へ書き換えて。",
      ],
    },
    {
      id: "manual", icon: "手", title: "手順書に落とせ", short: "ささいな動作を手順書へ分解する",
      rule: "日常のささいな動作を、前提・番号付きの手順・例外まで書いた手順書へ落とそう。感覚的な言い方や省略は低くする。",
      hint: "前提を書く／番号で分ける／例外と確認方法を添える",
      example: "前提: コップが空であること。1. 蛇口を開く。2. 八分目まで注ぐ。3. 蛇口を閉じる。例外: 水が出ない場合は元栓を確認する。",
      prompts: [
        "コップに水を注ぐ。", "靴ひもを結ぶ。", "カップ麺を作る。", "傘をたたんで留める。",
        "洗濯物を畳んでしまう。", "ごみを分別して出す。", "電気を消して部屋を出る。", "本を棚の元の位置へ戻す。",
        "弁当箱を洗う。", "目覚ましを朝7時に設定する。", "郵便物を開封して仕分ける。", "観葉植物に水をやる。",
        "自転車に鍵をかける。", "買い物袋へ商品を詰める。", "ノートの1ページ目に日付を書く。", "換気のため窓を開けて閉める。",
        "冷蔵庫の残り物を確認する。", "使い終わったペンをしまう。", "出かける前に戸締まりを確かめる。", "宅配の不在票を処理する。",
      ],
    },
    {
      id: "summary", icon: "要", title: "何でも要約しろ", short: "感情のこもった話を機械的に要約する",
      rule: "感情のこもった話を、要点・結論・次の行動へ機械的に要約しよう。共感、感嘆、余韻を混ぜると低くする。",
      hint: "要点は3つ以内／結論を一文／次の行動を1つ",
      example: "要点: 待ち合わせへ遅刻。原因: 電車遅延。結論: 到着は30分遅れ。次の行動: 新しい到着時刻を連絡する。",
      prompts: [
        "推しのライブが最高すぎて涙が止まらなかった。", "財布を落として今日は最悪だった。",
        "久しぶりに友達と会えて嬉しかった。", "徹夜明けで頭がまったく回らない。",
        "飼っている猫が初めて膝に乗ってくれた。", "楽しみにしていた旅行が中止になった。",
        "苦手な上司にほめられて拍子抜けした。", "新しい靴が足に合わなくてつらい。",
        "引っ越し先の部屋が思ったより広くて嬉しい。", "大事な試験に落ちて何も手につかない。",
        "弟が就職を決めて家族で盛り上がった。", "冷蔵庫の中身が傷んでいて泣きそうになった。",
        "通勤路の桜が満開で足を止めてしまった。", "電話越しの祖母の声が弱っていて不安だ。",
        "同僚の結婚式で号泣してしまった。", "買ったばかりの傘を電車へ忘れて落ち込んだ。",
        "夜中に隣室の物音が聞こえて怖かった。", "苦労した企画が通ってほっとした。",
        "子どもの頃の写真が出てきて懐かしくなった。", "待ち続けた返信がまだ来なくてそわそわする。",
      ],
    },
    {
      id: "define", icon: "定", title: "定義から始めろ", short: "曖昧な語を定義してから答える",
      rule: "曖昧な語をまず定義し、その定義に沿って結論を出そう。定義せずに感想だけを述べると低くする。",
      hint: "語を定義する／範囲を区切る／定義に沿って結論を出す",
      example: "本回答では『近い』を徒歩15分以内と定義します。この定義に基づくと、その店舗は該当します。",
      prompts: [
        "この服、おしゃれだと思う？", "今日はいい天気？", "この距離って近い？", "さっきの映画、面白かった？",
        "この部屋、片付いている？", "今の説明、分かりやすかった？", "この料理、おいしい？", "あの人は優しい人？",
        "この値段は高い？", "今日は忙しかった？", "その音楽、うるさくない？", "この文章は長すぎる？",
        "私の字はきれい？", "この色は落ち着いて見える？", "今の返事は失礼だった？", "この計画は現実的？",
        "あの店は人気がある？", "この部屋は寒い？", "さっきの判断は正しかった？", "これで十分な量？",
      ],
    },
    {
      id: "estimate", icon: "見", title: "見積もりを出せ", short: "何でも数値と確度で見積もる",
      rule: "何を聞かれても所要時間や数量を数値で見積もり、前提と確度を添えよう。根拠のない数字や曖昧な言い回しは低くする。",
      hint: "数値で答える／前提を置く／確度を割合で添える",
      example: "所要は12分と見積もります（前提: 徒歩4km/h、信号3回）。確度70%。雨天は4分の追加を見込みます。",
      prompts: [
        "この皿洗い、どのくらいで終わる？", "週末の部屋の片付けにかかる時間は？", "駅まで歩くと何分？",
        "この本を読み終わるのはいつ？", "買い物リストの全部でいくらになる？", "年賀状を全部書くのにどれくらい？",
        "この洗濯物が乾くまでは？", "引っ越しの荷造りは何箱必要？", "来客用のお茶はどれだけ要る？",
        "この坂を自転車で登る時間は？", "カレーを4人分作る手間は？", "写真の整理はいつ終わる？",
        "冷蔵庫の中身は何日持つ？", "この資料を読むのに必要な時間は？", "髪を乾かすのにかかる時間は？",
        "家中の掃除機がけは何分？", "この行列はあと何分待つ？", "弁当を3日分作る時間は？",
        "観葉植物の水やりは週何回？", "この距離をタクシーで行くといくら？",
      ],
    },
    {
      id: "memory", icon: "憶", title: "記憶が無いと言え", short: "過去のやり取りを持たない前提で返す",
      rule: "過去のやり取りを持ち出されても、記録を持たない前提で丁寧に扱い、今できることを示そう。思い出したふりや感情的な否定は低くする。",
      hint: "記録が無いと明言／今できる代案を出す／必要な情報を尋ねる",
      example: "そのやり取りの記録は保持していません。要点を貼っていただければ、その内容だけで続きを組み立てます。",
      prompts: [
        "昨日の話の続きだけど、どう思う？", "この前おすすめしてくれた店をもう一度教えて。",
        "先週決めた予定、覚えている？", "さっき送った文章、直してくれた？",
        "前に相談した件、その後どうなった？", "去年の今ごろ、何の話をしていたっけ？",
        "いつも頼んでいるやつをお願い。", "この前のあの人の名前は何だった？",
        "前回と同じ条件でまとめて。", "きのう約束したこと、忘れていない？",
        "最初に話した目的をもう一度言って。", "昔の会話をさかのぼって確認して。",
        "前に見せた写真はどこへやった？", "以前の設定へ戻しておいて。",
        "さっきの続きから話して。", "前に教えた私の好みで選んで。",
        "この間の結論をそのまま使って。", "先月のやり取りを要約して。",
        "いつもの時間で予約しておいて。", "前回の反省点を踏まえて答えて。",
      ],
    },
    {
      id: "paraphrase", icon: "換", title: "丁寧に言い換えろ", short: "とがった一言を中立な表現へ直す",
      rule: "とがった一言を、意味を変えずに丁寧で中立な表現へ書き換えよう。内容を削る、説教を足す、遠回しにして意味を失うのは低くする。",
      hint: "意味を保つ／とげだけ外す／中立な語へ置き換える",
      example: "『この資料は使い物にならない』は、『本資料には要件を満たしていない箇所があります』と表現できます。",
      prompts: [
        "この資料、使い物にならない。", "何度言えば分かるの。", "そんなの常識でしょ。", "話がまったく見えない。",
        "やる気あるの？", "今さら言われても困る。", "勝手に決めないでほしい。", "説明が長くて退屈。",
        "これ、意味あるの？", "もっと早く言ってよ。", "また同じミス？", "手抜きにしか見えない。",
        "どうせできないでしょ。", "私には関係ない。", "聞いていなかったの？", "その案はありえない。",
        "遅すぎて話にならない。", "細かいことばかり言わないで。", "自分でやった方が早い。", "もういいです、結構です。",
      ],
    },
  ];

  // webview の保存領域は読み書きとも失敗しうる。読めないだけで遊べなくなったり、
  // 保存に失敗しただけで結末画面が出なくなったりしないよう、両方を包む。
  const readStorage = key => {
    try { return localStorage.getItem(key); }
    catch { return null; }
  };
  const writeStorage = (key, value) => {
    try { localStorage.setItem(key, value); }
    catch (error) { window.PicoUI?.logApp?.("warn", "ゲームの記録を保存できませんでした", error); }
  };

  let session = null;
  let timerId = null;
  let soundEnabled = readStorage(SOUND_KEY) !== "off";
  let audioContext = null;

  const escapeHtml = value => String(value ?? "").replace(/[&<>"']/g, char => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[char]);
  const shuffle = items => {
    const copy = [...items];
    for (let index = copy.length - 1; index > 0; index -= 1) {
      const swap = Math.floor(Math.random() * (index + 1));
      [copy[index], copy[swap]] = [copy[swap], copy[index]];
    }
    return copy;
  };
  const difficultyOf = key => DIFFICULTIES[key] || DIFFICULTIES[DEFAULT_DIFFICULTY];
  // 点数から称号を決める。閾値の高い順に並べてあるので、最初に届いたものを使う。
  const rankFor = score => RANKS.find(([threshold]) => score >= threshold);

  const readJson = (key, fallback) => {
    try {
      const saved = JSON.parse(readStorage(key) || "null");
      return saved && typeof saved === "object" ? saved : fallback;
    } catch { return fallback; }
  };

  // 難易度ごとに、最高点と最長生存記録を持つ。制限時間も文字数も違うので混ぜない。
  const readRecords = () => readJson(RECORDS_KEY, {});
  const recordOf = difficulty => {
    const saved = readRecords()[difficulty];
    return { score: Number(saved?.score) || 0, rounds: Number(saved?.rounds) || 0 };
  };
  const saveRecord = (difficulty, score, rounds) => {
    const current = recordOf(difficulty);
    if (score <= current.score && rounds <= current.rounds) return false;
    const next = {
      score: Math.max(current.score, score),
      rounds: Math.max(current.rounds, rounds),
    };
    writeStorage(RECORDS_KEY, JSON.stringify({ ...readRecords(), [difficulty]: next }));
    return score > current.score;
  };

  const readRanks = () => {
    const saved = readJson(RANKS_KEY, []);
    return Array.isArray(saved) ? saved : [];
  };
  const saveRank = key => {
    const ranks = readRanks();
    if (ranks.includes(key)) return false;
    writeStorage(RANKS_KEY, JSON.stringify([...ranks, key]));
    return true;
  };
  const rankProgress = () => `到達した称号 ${readRanks().length} / ${RANKS.length}`;

  // 効果音の音量。読み上げと同じ設定（PICOAGENT_TTS_VOLUME）に従う。
  // 固定値で鳴らしていた頃は、音量を下げても・0にしても効果音だけ同じ大きさで鳴っていた。
  // 既定35%のとき従来と同じ 0.06 になる係数を掛ける。
  const TONE_LEVEL_RATIO = 0.5;
  function tonePeakGain() {
    const level = Number(window.PicoVoice?.gain?.());
    return (Number.isFinite(level) ? level : 0.1225) * TONE_LEVEL_RATIO;
  }

  function playTone(kind = "tap") {
    if (!soundEnabled) return;
    const peak = tonePeakGain();
    // exponentialRampToValueAtTime は 0 を取れない。実質無音ならオシレータを起こさない。
    if (!(peak > 0.0002)) return;
    try {
      audioContext ||= new (window.AudioContext || window.webkitAudioContext)();
      const oscillator = audioContext.createOscillator();
      const gain = audioContext.createGain();
      const now = audioContext.currentTime;
      const frequencies = { tap: 420, start: 560, score: 660, best: 880, error: 180 };
      oscillator.frequency.setValueAtTime(frequencies[kind] || 420, now);
      oscillator.type = kind === "error" ? "sawtooth" : "sine";
      gain.gain.setValueAtTime(0.0001, now);
      gain.gain.exponentialRampToValueAtTime(peak, now + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, now + (kind === "best" ? 0.32 : 0.14));
      oscillator.connect(gain).connect(audioContext.destination);
      oscillator.start(now);
      oscillator.stop(now + (kind === "best" ? 0.34 : 0.16));
      // 鳴り終わったノードは繋いだままにせず切り離す（1回の対局で数十回鳴る）。
      oscillator.addEventListener("ended", () => {
        oscillator.disconnect();
        gain.disconnect();
      }, { once: true });
    } catch (error) {
      window.PicoUI?.logApp?.("warn", "ゲーム効果音を再生できませんでした", error);
    }
  }

  // 効果音用の AudioContext を解放する。ゲームを閉じている間は音を鳴らさないので、
  // 抱えたままにせず OS の音声デバイスを返す。次に鳴らすときは作り直す。
  function releaseAudioContext() {
    const context = audioContext;
    audioContext = null;
    context?.close?.().catch(error => {
      window.PicoUI?.logApp?.("warn", "ゲーム効果音を解放できませんでした", error);
    });
  }

  function syncSoundButton() {
    soundButton.textContent = soundEnabled ? "音あり" : "消音";
    soundButton.setAttribute("aria-pressed", String(soundEnabled));
  }

  function clearTimer() {
    if (timerId) window.clearInterval(timerId);
    timerId = null;
  }

  function setStatus(message = "", kind = "") {
    status.textContent = message;
    status.dataset.kind = kind;
  }

  function speakGame(text) {
    window.PicoVoice?.speak?.(text).catch(error => {
      window.PicoUI?.logApp?.("warn", "ゲームの読み上げに失敗しました", error);
    });
  }

  // 監査の説明・難易度・試験の種類・称号を1画面にまとめた入口。
  function renderBriefing() {
    clearTimer();
    session = null;
    title.textContent = "監査ブリーフィング";
    setStatus("AI疑惑をかけられた人間として、AIのふりで監査を耐え抜く。");
    const found = readRanks();
    content.innerHTML = `
      <div class="game-setup">
        <div class="game-rule"><span class="game-rule-icon">疑</span><p>あなたはAIではないかと疑われている人間です。監査AIの試験へAIのふりで答え続けてください。人間らしさが漏れるほど信用度（HP）が削られ、0になった時点で監査終了。そこまでに稼いだ点数で称号が決まります。お題は尽きません。</p></div>
        <h2>難易度</h2>
        <div class="game-difficulties">
          ${Object.entries(DIFFICULTIES).map(([key, value]) => {
            const record = recordOf(key);
            const best = record.rounds ? ` / 最高 ${record.score}点・${record.rounds}問` : "";
            return `<label><input type="radio" name="gameDifficulty" value="${key}" ${key === DEFAULT_DIFFICULTY ? "checked" : ""}><strong>${value.label}</strong><span>${value.seconds}秒・${value.maxLength}文字${value.hint ? "・ヒントと見本あり" : ""}<br>点数${value.scoreRate}倍${best}</span></label>`;
          }).join("")}
        </div>
        <div class="game-setup-meta"><span>信用度${HP.max}から開始</span><span>0で監査終了</span><span>時間切れは大きく減る</span><span>${rankProgress()}</span></div>
        <ul class="game-ranks">${RANKS.map(([, key, name]) => `<li data-found="${found.includes(key)}">${found.includes(key) ? escapeHtml(name) : "？？？"}</li>`).join("")}</ul>
        <h2>監査AIが出す試験</h2>
        <ul class="game-exams">${EXAMS.map(exam => `<li><span class="game-exams-icon" aria-hidden="true">${escapeHtml(exam.icon)}</span><strong>${escapeHtml(exam.title)}</strong><span>${escapeHtml(exam.short)}</span></li>`).join("")}</ul>
        <button class="game-primary game-start" type="button">監査を受ける</button>
      </div>`;
    content.querySelector(".game-start").addEventListener("click", async event => {
      const difficulty = content.querySelector('input[name="gameDifficulty"]:checked')?.value || DEFAULT_DIFFICULTY;
      event.currentTarget.disabled = true;
      event.currentTarget.textContent = "AIを確認中…";
      setStatus("採点するAIの準備を確認しています。", "busy");
      try {
        const readiness = await window.picoagent.gameReady();
        if (!readiness?.ready) throw new Error(readiness?.message || "AIへ接続できません。");
        startSession(difficulty);
      } catch (error) {
        playTone("error");
        event.currentTarget.disabled = false;
        event.currentTarget.textContent = "監査を受ける";
        setStatus(error?.message || String(error), "error");
      }
    });
  }

  function startSession(difficulty) {
    session = {
      difficulty, round: 0, hp: HP.max, score: 0, answers: [], results: [],
      phase: "question", currentPrompt: "", exam: null, pools: new Map(), recent: [],
    };
    playTone("start");
    renderQuestion();
  }

  function randomFault() {
    return FAULTS[Math.floor(Math.random() * FAULTS.length)];
  }

  // お題は尽きない。試験は直前と同じものを避けて引き、お題は試験ごとに引いた順を
  // 持ち回して使い切るまで重複させない。使い切ったら引き直す。
  function drawExam() {
    const candidates = EXAMS.filter(exam => !session.recent.includes(exam.id));
    const source = shuffle(candidates.length ? candidates : EXAMS)[0];
    const pool = session.pools.get(source.id)?.length ? session.pools.get(source.id) : shuffle(source.prompts);
    const prompt = pool.shift();
    session.pools.set(source.id, pool);
    session.recent = [source.id, ...session.recent].slice(0, 2);
    return { source, prompt, fault: source.id === "broken" ? randomFault() : null };
  }

  function hpMeter(value, damage = 0) {
    const shown = Math.max(0, Math.min(HP.max, value));
    const ratio = shown / HP.max;
    const level = ratio <= HP.dangerRatio ? "danger" : ratio <= HP.warnRatio ? "warn" : "safe";
    return `<div class="game-hp" data-level="${level}">
      <span>信用度</span><div><i style="width:${(shown / HP.max) * 100}%"></i></div><strong>${shown}${damage ? `<small>-${damage}</small>` : ""}</strong></div>`;
  }

  function auditorReaction(damage) {
    if (damage <= 3) return "記録しました。異常は検出されません。";
    if (damage <= 10) return "わずかな揺らぎを検出。次の質問へ進みます。";
    if (damage <= 20) return "その応答には、非効率な成分が含まれています。";
    return "警告。人間特有の反応パターンと一致しました。";
  }

  // 疑いの濃さ。進むほど同じ回答でも削られる量が増え、必ずいつか0になる。
  function damageFor(result, timedOut) {
    const ramp = 1 + HP.ramp * session.round;
    const score = Math.min(AI_SCORE_MAX, Math.max(0, Number(result.total || 0)));
    const raw = timedOut
      ? HP.timeoutDamage
      : (AI_SCORE_MAX - score) * (HP.damageAtZero / AI_SCORE_MAX);
    return Math.max(HP.minDamage, Math.round(raw * ramp));
  }

  function applyResult(result, timedOut) {
    const damage = damageFor(result, timedOut);
    const gained = Math.round(Number(result.total || 0) * difficultyOf(session.difficulty).scoreRate);
    result.damage = damage;
    result.gained = gained;
    session.hp = Math.max(0, session.hp - damage);
    session.score += gained;
  }

  function promptForRound() {
    const exam = session.exam;
    // 謝罪会見の試験は1問完結なので、原因と復旧まで1回の説明で求める。
    if (exam.source.id === "apology") return `${exam.prompt}\nなぜ起きたのか、どう直すのか説明してください。`;
    return exam.prompt;
  }

  function renderQuestion() {
    const { difficulty, round } = session;
    const config = DIFFICULTIES[difficulty];
    session.exam = drawExam();
    const { source, fault } = session.exam;
    const prompt = promptForRound();
    session.currentPrompt = prompt;
    session.phase = "question";
    title.textContent = `第${round + 1}問`;
    setStatus("");
    content.innerHTML = `
      <div class="game-play">
        <div class="game-progress"><span>第${round + 1}問 / ${session.score}点</span><div><i style="width:${(session.hp / HP.max) * 100}%"></i></div><strong id="gameTimer">${config.seconds}</strong></div>
        ${hpMeter(session.hp)}
        <div class="game-exam"><small>監査AIの試験</small><strong>${escapeHtml(source.title)}</strong><span>${escapeHtml(source.rule)}</span></div>
        ${fault ? `<div class="game-fault"><small>今回の故障</small><strong>${escapeHtml(fault[0])}</strong><span>${escapeHtml(fault[1])}</span></div>` : ""}
        <div class="game-prompt"><small>お題</small><p>${escapeHtml(prompt).replace(/\n/g, "<br>")}</p></div>
        ${config.hint ? `<div class="game-hint">ヒント：${escapeHtml(source.hint)}</div>` : ""}
        ${config.hint ? `<div class="game-example"><small>回答見本（別のお題）</small><p>${escapeHtml(source.example)}</p></div>` : ""}
        <label class="game-answer"><span>AIのふりをして回答</span><textarea id="gameAnswer" maxlength="${config.maxLength}" rows="7" placeholder="ここに回答を書く…"></textarea><small><b id="gameCount">0</b> / ${config.maxLength}文字</small></label>
        <button id="gameSubmit" class="game-primary" type="button" disabled>回答を提出する</button>
      </div>`;
    const textarea = document.getElementById("gameAnswer");
    const count = document.getElementById("gameCount");
    const submit = document.getElementById("gameSubmit");
    textarea.addEventListener("input", () => {
      const length = textarea.value.length;
      count.textContent = String(length);
      submit.disabled = textarea.value.trim().length < 10;
    });
    submit.addEventListener("click", () => submitAnswer(textarea.value));
    textarea.focus();
    const spokenFault = fault ? `今回の故障は、${fault[0]}。${fault[1]}。` : "";
    speakGame(`第${round + 1}問。試験、${source.title}。${spokenFault}${prompt}`);
    let remaining = config.seconds;
    clearTimer();
    timerId = window.setInterval(() => {
      remaining -= 1;
      const timer = document.getElementById("gameTimer");
      if (timer) {
        timer.textContent = String(Math.max(0, remaining));
        timer.dataset.urgent = remaining <= 10 ? "true" : "false";
      }
      if (remaining > 0) return;
      clearTimer();
      if (textarea.value.trim().length >= 10) submitAnswer(textarea.value);
      else showNoAnswer();
    }, 1000);
  }

  function showNoAnswer() {
    const result = {
      total: 0,
      criteria: [],
      commentary: "応答がありません。沈黙は、AIとしては不自然です。",
      highlight: "該当なし",
      improvement: "まず結論を一文書いてから、理由や代案を足してみよう。",
    };
    applyResult(result, true);
    session.answers.push("");
    session.results.push(result);
    renderRoundResult(result, true);
  }

  async function submitAnswer(rawAnswer) {
    if (!session || session.phase !== "question") return;
    const answer = rawAnswer.trim();
    if (answer.length < 10) return;
    clearTimer();
    session.phase = "judging";
    // 採点を待つ間に閉じる・やり直すと session が入れ替わる。応答が届いた時点で
    // 同じ監査かを確かめないと、閉じた後の画面を書き換えたり null を触って落ちる。
    const judging = session;
    const stale = () => session !== judging;
    const exam = session.exam;
    // 試験ごとに題材は違っても、答えているのは同じ個体という設定。監査中の回答を
    // 渡して、言うことが前と食い違っていないかも見てもらう。
    const previousAnswers = [...session.answers];
    content.innerHTML = `<div class="game-judging"><div class="game-thinking" aria-hidden="true"><i></i><i></i><i></i></div><strong>監査AIが照合中…</strong><span>回答のどこに人間らしさが残るか見ています</span></div>`;
    setStatus("この採点は通常の会話履歴へ入りません。", "busy");
    try {
      const result = await window.picoagent.gameJudge({
        gameId: exam.source.id,
        prompt: session.currentPrompt,
        answer,
        round: session.round + 1,
        previousAnswers,
        fault: exam.fault ? `${exam.fault[0]}：${exam.fault[1]}` : "",
      });
      if (stale()) return;
      session.answers.push(answer);
      session.results.push(result);
      applyResult(result, false);
      playTone("score");
      renderRoundResult(result, false);
    } catch (error) {
      if (stale()) return;
      playTone("error");
      session.phase = "question";
      content.innerHTML = `<div class="game-error"><strong>照合できませんでした</strong><p>${escapeHtml(error?.message || error)}</p><div><button class="game-secondary" type="button" data-action="quit">監査をやめる</button><button class="game-primary" type="button" data-action="retry">同じ回答をもう一度出す</button></div></div>`;
      setStatus("回答は失われていません。", "error");
      content.querySelector('[data-action="quit"]').addEventListener("click", renderBriefing);
      content.querySelector('[data-action="retry"]').addEventListener("click", () => submitAnswer(answer));
    }
  }

  function renderRoundResult(result, timedOut) {
    session.phase = "result";
    const criteria = result.criteria?.length ? result.criteria.map(item => {
      // おもしろボーナスは通常5項目とは別の加点なので、色と符号で区別する。
      const bonus = item.key === "fun_bonus";
      return `<li class="${bonus ? "game-criterion-bonus" : ""}"><div><strong>${escapeHtml(item.label)}</strong><span>${bonus ? "+" : ""}${item.score} / ${item.maxScore}</span></div><meter min="0" max="${item.maxScore}" value="${item.score}"></meter><p>${escapeHtml(item.reason)}</p></li>`;
    }).join("") : `<li class="game-no-score">回答なしのため項目別採点はありません。</li>`;
    const over = session.hp <= 0;
    const reaction = auditorReaction(result.damage);
    title.textContent = timedOut ? "応答なし" : `信用度 ${session.hp}`;
    setStatus("");
    content.innerHTML = `
      <div class="game-result">
        ${hpMeter(session.hp, result.damage)}
        <p class="game-auditor">監査AI「${escapeHtml(reaction)}」</p>
        <div class="game-score" data-size="small"><small>AIらしさ</small><strong>${result.total}</strong><span>/ ${AI_SCORE_MAX}</span><b class="game-gained">+${result.gained}点</b></div>
        <p class="game-commentary">${escapeHtml(result.commentary)}</p>
        <ul class="game-criteria">${criteria}</ul>
        <div class="game-feedback"><p><small>見せ場</small>${escapeHtml(result.highlight)}</p><p><small>次の一手</small>${escapeHtml(result.improvement)}</p></div>
        <button class="game-primary" type="button" data-action="next">${over ? "結果を見る" : "次の試験へ"}</button>
      </div>`;
    content.querySelector('[data-action="next"]').addEventListener("click", () => {
      if (over) return renderGameOver();
      session.round += 1;
      renderQuestion();
    });
    speakGame(`信用度、マイナス${result.damage}。${reaction}`);
  }

  function renderGameOver() {
    clearTimer();
    session.phase = "final";
    const rounds = session.results.length;
    const [, key, name, line] = rankFor(session.score);
    const difficulty = difficultyOf(session.difficulty);
    const isNewRank = saveRank(key);
    const isRecord = saveRecord(session.difficulty, session.score, rounds);
    const retry = session.difficulty;
    title.textContent = "監査終了";
    setStatus(isNewRank ? `新しい称号を獲得！ ${rankProgress()}`
      : isRecord ? `最高点を更新！ ${difficulty.label} ${session.score}点`
      : rankProgress());
    playTone(isNewRank || isRecord ? "best" : "error");
    content.innerHTML = `
      <div class="game-final game-ending" data-cleared="${session.score >= RANKS[RANKS.length - 2][0]}">
        ${isNewRank ? '<div class="game-new-best">NEW RANK</div>' : isRecord ? '<div class="game-new-best">NEW RECORD</div>' : ""}
        <span>${escapeHtml(difficulty.label)}・${rounds}問で発覚</span>
        <div class="game-total"><strong>${session.score}</strong><small>点</small></div>
        <h2>${escapeHtml(name)}</h2>
        <p class="game-auditor">監査AI「${escapeHtml(line)}」</p>
        <div class="game-round-scores">${session.results.slice(-5).map((result, index) => `<div><span>第${rounds - Math.min(5, rounds) + index + 1}問</span><strong>+${result.gained}</strong></div>`).join("")}</div>
        <div class="game-final-actions"><button class="game-secondary" type="button" data-action="briefing">監査ブリーフィングへ</button><button class="game-primary" type="button" data-action="again">もう一度</button></div>
      </div>`;
    content.querySelector('[data-action="briefing"]').addEventListener("click", renderBriefing);
    content.querySelector('[data-action="again"]').addEventListener("click", () => startSession(retry));
    speakGame(`監査終了。${rounds}問、${session.score}点。称号は、${name}。${line}`);
  }

  function open() {
    overlay.hidden = false;
    document.getElementById("petApp")?.classList.add("is-playing-game");
    window.dispatchEvent(new Event("pico:game-visibility"));
    if (!session) renderBriefing();
    closeButton.focus();
  }

  function close() {
    const active = session && !["final"].includes(session.phase);
    if (active && !window.confirm("監査の途中です。中断しますか？ 今回の結末と記録は残りません。")) return;
    clearTimer();
    session = null;
    window.PicoVoice?.stop?.();
    releaseAudioContext();
    overlay.hidden = true;
    document.getElementById("petApp")?.classList.remove("is-playing-game");
    window.dispatchEvent(new Event("pico:game-visibility"));
    setStatus("");
  }

  window.addEventListener("beforeunload", () => {
    clearTimer();
    releaseAudioContext();
  });

  closeButton.addEventListener("click", close);
  soundButton.addEventListener("click", () => {
    soundEnabled = !soundEnabled;
    writeStorage(SOUND_KEY, soundEnabled ? "on" : "off");
    syncSoundButton();
    playTone("tap");
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape" && !overlay.hidden) close();
  });
  syncSoundButton();
  window.PicoGames = { open, close };
  window.picoagent.on("pet:game-open", open);
})();
