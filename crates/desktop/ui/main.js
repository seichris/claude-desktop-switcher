// ============================================================================
// Claude Desktop Switcher — settings UI logic (Tauri v2)
// The WebView is a "dumb terminal": all profile logic lives in Rust.
// When window.__TAURI__ is absent (plain browser, e.g. screenshots) a dev mock
// stands in. The shipped app sets withGlobalTauri:true and always reaches Rust.
//
// All dynamic UI is built with createElement + textContent (never innerHTML),
// so backend/user strings are never parsed as HTML — XSS is structurally
// impossible.
// ============================================================================

const HAS_TAURI = !!(window.__TAURI__ && window.__TAURI__.core);
const invoke = HAS_TAURI ? window.__TAURI__.core.invoke : devInvoke;

// --- Internationalization (ja default, en fallback) -------------------------
// The app ships Japanese copy inline. When the OS/browser locale is not Japanese
// we render English. Standalone strings are translated by matching the exact
// Japanese text against EN below (applied to the static HTML at load and to every
// node the app builds afterwards, via a MutationObserver). Strings that embed a
// name or a number are translated at construction with T(ja, en).
// Locale detection: Japanese when the OS/browser locale is Japanese, else English.
// A ?lang=ja|en query override makes screenshot capture deterministic regardless of
// the headless browser locale; the shipped app is loaded without query params.
const LANG = (() => {
  const forced = new URLSearchParams(location.search).get('lang');
  if (forced === 'ja' || forced === 'en') return forced;
  return String(navigator.language || navigator.userLanguage || 'en').toLowerCase().startsWith('ja') ? 'ja' : 'en';
})();
const T = (ja, en) => (LANG === 'ja' ? ja : en);
// Japanese -> English for every standalone visible string. Keys must match the
// rendered text exactly (trimmed). Terminology follows the shipped EN landing page
// and the established English screenshots (Existing Claude / In use / Work etc.).
const EN = {
  'ローカルCodeを共有': 'Share local Code',
  'Code・メモリ': 'Code and memory',
  'Codeのスレッド・メモリ・入力履歴を引き継ぎます。初回サインイン後に、バックアップしてサイドバー・リポジトリ・プリセットの共有を有効にします。ログインとクラウドChatは別々です。共通ルール・CLI権限・スキル・プラグインは分離します。':
    'Carry over Code threads, memory and input history. After the first sign-in, back up and enable shared sidebar, repos and presets. Logins and cloud Chat stay separate. Global rules, CLI permissions, skills and plugins stay isolated.',
  // Sidebar / chrome
  'Claude Desktop Switcher': 'Claude Desktop Switcher',
  '既存のClaude': 'Existing Claude',
  'あなたの標準環境である既存のClaude': 'Existing Claude, your standard environment',
  '利用中': 'In use',
  '環境を作る': 'New environment',
  'テーマ': 'Theme',
  'ユーザーガイド': 'User guide',
  '問題を報告': 'Report an issue',
  'このアプリについて': 'About',
  '更新を確認': 'Check for updates',
  'ブルー': 'Blue', 'ティール': 'Teal', 'インディゴ': 'Indigo', 'テラコッタ': 'Terracotta',
  // Empty / onboarding
  'はじめまして': 'Welcome',
  '環境を分けて、混ぜない。': 'Separate environments. Never mixed.',
  '新しい環境ではローカルCodeの共有を既定とし、ログインを分離します。初回サインイン後、詳細画面でバックアップして共有を有効にしてください。':
    'New environments default to shared local Code with separate logins. After the first sign-in, back up and enable sharing in the environment details.',
  'ログインは環境ごとに保持します': 'Sign-ins stay with each environment',
  '普段使っている環境は「既存のClaude」として保持します。ローカルCode全体の共有を明示的に有効にした場合だけCodeストアを同期し、ログインは変更しません。':
    'Your everyday setup remains Existing Claude. Only explicitly enabling complete local Code sharing synchronizes its Code stores; sign-ins stay unchanged.',
  '環境を切り替えて使えます': 'Switch between environments',
  '「利用中」は、いまClaudeが起動している環境です。Claudeを終了すれば「利用中」は外れ、その環境はまた「この環境を起動」から開き直せます。設定を共有する環境は衝突を防ぐため一度に1つずつ開きます。次の環境を選んで起動を押すと、Claudeを手動ですべて終了するまで待ってから開きます。「すべて分ける」で作った環境は何も共有しないので、起動中のClaudeを終了せず、新しいウィンドウで並べて開けます。':
    '"In use" marks the environment Claude is currently running for. Quit Claude and it clears, so you can reopen that environment from "Launch this environment". Environments that share settings open one at a time to avoid conflicts. Choose the next environment and press Launch; it opens after you manually quit every Claude instance. An environment set to "separate everything" shares nothing, so you can open it in a new window alongside a running Claude, without quitting.',
  'ターミナルのClaude Codeも同じ環境で使えます': 'Claude Code in the terminal uses the same environment',
  // Where to get csw: shown wherever the GUI names a csw command.
  'cswコマンドはアプリの.dmgに含まれておらず、アプリを更新しても更新されません。GitHubのリリースページから最新のcswを入手できます。':
    "The csw command is not included in the app's .dmg and is not updated with the app. You can get the latest csw from the GitHub releases page.",
  'リリースページを開く': 'Open releases page',
  // Detail: taglines, sections, sharing
  'あなた自身の環境': 'Your own setup',
  '利用中の環境': 'Environment in use',
  '作成した環境': 'Created environment',
  'Claudeデスクトップアプリ': 'Claude Desktop App',
  'Claude Code': 'Claude Code',
  'この環境が引き継いでいるもの': 'What this environment inherits',
  '「共有」は既存のClaudeと中身を共通にすること、「コピー」は作成時に一度だけ写して以後は別々、「分離」はこの環境だけで持つことです。':
    '"Shared" keeps the contents in common with your existing Claude; "Copied" duplicates once at creation and they diverge after; "Isolated" means this environment keeps its own.',
  'ワークツリー一覧': 'Worktrees', 'Gitワークツリーとrepoの対応': 'Mapping of Git worktrees to repos',
  '共通ルール': 'Global rules', 'CLAUDE.mdに書いた常時ルール': 'Always-on rules in CLAUDE.md',
  'ルールファイル': 'Rule files', 'rules/に分けて置いた常時ルール': 'Always-on rules kept as files in rules/',
  'プラグイン': 'Plugins', '導入したプラグイン': 'Installed plugins',
  'スキル': 'Skills', 'カスタムスキル': 'Custom skills',
  'ツール権限・フック': 'Tool permissions & hooks', 'ツールの実行可否とフック': 'Whether tools may run, and hooks',
  'プロジェクトの会話・メモリ': 'Project conversations & memory', 'プロジェクトごとの会話履歴と自動メモリ': 'Per-project conversation history and auto-memory',
  '入力履歴': 'Input history', '入力したプロンプトの履歴': 'History of entered prompts',
  '端末ID': 'Device ID', '端末を識別するためのID': 'ID that identifies your device',
  '共有': 'Shared', 'コピー': 'Copied', '分離': 'Isolated',
  'この環境のデータの場所': "Location of this environment's data",
  'あなたのClaudeフォルダの場所': 'Location of your Claude folder',
  'これはCSWが作った場所ではなく、あなた自身のClaudeフォルダです。': 'This is not a folder CSW created; it is your own Claude folder.',
  'ターミナルでClaude Codeを使う': 'Use Claude Code in the terminal',
  'CSWから開いたアプリの中で使うターミナルは、最初からこの環境です。コマンドは要りません。自分で別に開いたiTerm2などのターミナルで対象の環境に揃えるには、環境名を渡して次を実行します。この設定はそのタブだけに効き、普段の環境には影響しません。':
    "A terminal opened inside the app you launched from CSW is already in this environment, so no command is needed. To switch a terminal you opened yourself, such as iTerm2, to a target environment, pass the environment name and run the command below. It applies to that tab only and never affects your usual environment.",
  'あなた自身のClaudeデスクトップアプリとClaude Codeの環境です。ローカルCode全体の共有を明示的に有効にした場合だけCodeストアを同期し、ログインは変更しません。':
    'This is your own Claude Desktop App and Claude Code setup. Only explicitly enabling complete local Code sharing synchronizes its Code stores; sign-ins stay unchanged.',
  // Buttons / actions
  '複製': 'Duplicate', '削除': 'Delete',
  'この環境の複製': 'Duplicate this environment',
  'この環境の設定とデータをそのまま写して、別の名前の環境を作ります。Claudeデスクトップアプリのサインインの情報もそのまま写すので、複製した環境は元の環境と同じサインインの状態を持ちます。別のアカウントで使う環境は「環境を作る」から作ってください。':
    "Copies this environment's settings and data as they are into a new environment under a different name. The Claude Desktop App sign-in is copied too, so the duplicate carries the same sign-in state as the original. For a different account, create the environment with \"New environment\" instead.",
  '起動のしかた': 'How to launch',
  'この環境を起動': 'Launch this environment', '重複して起動': 'Launch alongside', '前面に表示': 'Bring to front',
  'この環境でClaudeを開きます。ほかのClaudeが起動中なら、手動ですべて終了するまで待ってから開きます。':
    'Opens Claude for this environment. If another Claude is running, CSW waits for you to quit every instance, then opens the selected environment.',
  '起動中のClaudeを終了せずに、この環境を新しいウィンドウで同時に開きます。':
    'Opens this environment in a new window at the same time, without quitting a running Claude.',
  '既存のClaudeを起動': 'Launch Existing Claude',
  '既存のClaudeは変更・削除できません': 'Existing Claude cannot be changed or deleted',
  '完全に削除': 'Delete permanently',
  'ゴミ箱へ移動': 'Move to Trash',
  '完全に削除する': 'Delete permanently',
  '完全に削除すると、ゴミ箱を経由せずにすぐ消え、戻せません。':
    'Permanent deletion skips the Trash, takes effect immediately, and cannot be undone.',
  'ゴミ箱へ移動できませんでした。すぐに消す場合は「完全に削除」を選んでください。':
    'Could not move it to the Trash. To remove it now, choose "Delete permanently".',
  'やめる': 'Cancel', '複製を作る': 'Duplicate', '戻る': 'Back',
  '閉じる': 'Close', 'もう一度試す': 'Try again', '開き直す': 'Reopen',
  // Create flow
  '新しい環境を作る': 'Create a new environment',
  'この環境を作る': 'Create this environment', '作成する': 'Create',
  '各項目を、既存のClaudeと共有するか、この環境だけにするかを選びます。': 'For each item, choose whether to share it with your existing Claude or keep it only in this environment.',
  '共有 = いまのを使う ／ 分離 = この環境だけ ／ コピー = 最初だけ写す': 'Shared = use the current one / Isolated = only this environment / Copied = copied once at the start',
  'アカウントのサインイン情報（config.json）と、コネクタ・アプリ設定（claude_desktop_config.json）は、アカウント別の情報を含むため、どのモードでも必ずこの環境だけに分離します。':
    'The account sign-in (config.json) and the connector/app settings (claude_desktop_config.json) contain per-account data, so they are always isolated to this environment in every mode.',
  '2つのアカウントが同じ端末として結び付かないよう、端末IDは常に分離します。': 'The device ID is always isolated so two accounts are not linked as the same device.',
  'カスタム設定': 'Custom', 'モードの既定どおり': 'Mode defaults',
  // Misc / toasts (standalone)
  'コピーしました': 'Copied', 'コピーできませんでした。': 'Could not copy.',
  '環境を読み込めませんでした。': 'Could not load the environment.',
  '起動できませんでした。もう一度お試しください。': 'Could not launch. Please try again.',
  '削除できませんでした。利用中の環境は、起動中のClaudeを終了してから削除してください。': 'Could not delete. Quit the running Claude for an in-use environment before deleting it.',
  '複製できませんでした。同じ名前がすでにあるか確認してください。': 'Could not duplicate. Check whether the same name already exists.',
  '作成できませんでした。同じ名前がすでにあるか確認してください。': 'Could not create. Check whether the same name already exists.',
  'モードを変えたので高度設定をリセットしました': 'Changed the mode, so advanced settings were reset',
  'リンクを開けませんでした。': 'Could not open the link.',
  '取り出す': 'Eject', 'あとで': 'Later',
  'ディスクイメージを取り出しました': 'Disk image ejected',
  '取り出せませんでした。ディスクイメージを使用中のウィンドウを閉じてから、もう一度お試しください。': 'Could not eject. Close any window using the disk image, then try again.',
  'パスをコピー': 'Copy path', 'コマンドをコピー': 'Copy command',
  '複製先の名前。例：仕事用-控え': 'Name for the duplicate. e.g. Work-backup',
  '名前を入力してください。': 'Enter a name.',
  '"default"は使えません。いまの環境を指す予約名です。': '"default" is reserved for the current environment and cannot be used.',
  '名前は64文字までにしてください。': 'Use at most 64 characters.',
  '使えるのは文字・数字・ハイフン・アンダースコアだけです。空白や記号は使えません。': 'Use letters, digits, hyphens and underscores only. No spaces or symbols.',
  '既存のClaudeが標準の場所に見つかりません。引き継げるものが無いため、「すべて分ける」だけで作成できます。':
    'Your existing Claude was not found at the standard locations. There is nothing to carry over, so you can create only with "Separate everything".',
  'Claude Code（CLI）の設定が標準の場所に見つかりません。引き継ぐモードを選んでも、ルールやスキル、会話などのCLI側は引き継がれません。':
    'No Claude Code (CLI) settings were found at the standard location. Even if you choose a mode that carries things over, the CLI side (rules, skills, and conversations) is not carried over.',
  // About dialog
  '非公式のオープンソースのコミュニティプロジェクトです。': 'An unofficial, open-source community project.',
  '無保証': 'No warranty',
  '本ソフトウェアはMITライセンスのもとで、そのままの状態で提供されます。動作や品質について、いかなる保証もありません。': 'This software is provided as is under the MIT License, with no warranty of any kind as to operation or quality.',
  '自己責任でのご利用': 'Use at your own risk',
  'ご利用は自己責任でお願いします。データの損失や不具合などについて、作者は責任を負いません。大切なデータは、お試しになる前にバックアップしておくことをおすすめします。': 'Use at your own risk. The author is not responsible for data loss or malfunctions. Back up important data before trying it.',
  'プライバシー': 'Privacy',
  'CSWはインターネット通信も、利用状況の送信も行いません。パスワードを保管するmacOSのキーチェーンにも触れず、すべて手元のフォルダと設定の操作だけで動きます。': 'CSW makes no network requests and sends no usage data. It never touches the macOS Keychain that stores your passwords; it works only with local folders and settings.',
  '何を読み書きするか': 'What it reads and writes',
  '通常はCSW専用のフォルダに書き込みます。ローカルCode全体の共有を明示的に有効にすると、既存のClaudeを含む参加環境のCodeストアも置き換えます。サインイン情報は対象外です。読み書きの範囲と通信しないことの確かめ方は、プライバシーと透明性の文書で公開しています。':
    'Writes normally stay in CSW\'s own folder. Explicitly enabling complete local Code sharing also replaces Code stores in participating environments, including Existing Claude. Sign-in storage is excluded. Privacy and Transparency documents the read/write scope and how to verify the absence of network requests.',
  '確かめ方を見る': 'See how to verify',
  '非公式プロジェクト': 'Unofficial project',
  // Isolation check (csw doctor)
  '分離の検査': 'Isolation check',
  '共有と分離が設定どおりに保たれているかを、この場で点検します。ファイルの中身は読まず、何も書き換えません。':
    'Checks on the spot that sharing and isolation still match this environment\'s settings. It reads no file contents and changes nothing.',
  '検査する': 'Check now',
  '検査できませんでした。': 'The check could not be run.',
  'この環境のClaudeデスクトップアプリが起動中です。書き換えの途中を検査した可能性があるため、問題が出た場合はClaudeを終了してからもう一度検査してください。':
    'The Claude Desktop App is running in this environment. The check may have caught a rewrite in progress; if issues appear, quit Claude and check again.',
  '正常に共有': 'Shared (OK)',
  '正常に分離': 'Isolated (OK)',
  '正常にコピー': 'Copied (OK)',
  '共有元がまだありません': 'Nothing to share yet',
  '要確認': 'Needs attention',
  '共有元が見つかりません。': 'The shared source is missing.',
  'リンク先が想定と異なります。ターミナルで「csw doctor --fix」を実行すると張り直せます。':
    'The link points somewhere unexpected. Run csw doctor --fix in a terminal to re-point it.',
  'リンク先が想定と異なり、共有元も見つかりません。':
    'The link points somewhere unexpected, and the shared source is missing.',
  '共有のリンクが実体のファイルに置き換わっています。手元の内容を失わないよう、自動では直しません。':
    'The share link has been replaced by a regular file. To avoid losing your local contents, it is not repaired automatically.',
  '共有のリンクの位置に実体のフォルダがあります。フォルダが空なら、ターミナルで「csw doctor --fix」を実行するとリンクに置き換えられます。この置き換えにはcsw 0.24.2以降が必要です。中身があるときは、手元の内容を失わないよう、自動では直しません。':
    'A regular folder sits where the share link belongs. If the folder is empty, run csw doctor --fix in a terminal to replace it with the link. This replacement needs csw 0.24.2 or later. If it has contents, it is not repaired automatically, to avoid losing your local contents.',
  '共有のリンクがありません。ターミナルで「csw doctor --fix」を実行すると作れます。':
    'The share link is missing. Run csw doctor --fix in a terminal to create it.',
  '常に分離する項目がリンクになっています。': 'An always-isolated item has become a link.',
  'コピーの項目がリンクになっています。': 'An item set to copy has become a link.',
  '分離の項目がリンクになっています。': 'An isolated item has become a link.',
  // Data map (場所のデータの内訳)
  '読み込んでいます…': 'Loading…',
  '内訳を読み込めませんでした。': 'The breakdown could not be loaded.',
  'Finderで表示': 'Show in Finder',
  'Finderで表示できませんでした。': 'Could not show it in Finder.',
  '常に分離': 'Always isolated',
  '既存のClaudeと共有': 'Shared with your existing Claude',
  'まだありません': 'Not present yet',
  '状態を判定できませんでした。': 'The state could not be determined.',
  'セッション状態': 'Session state',
  'アカウントのサインイン情報': 'Account sign-in',
  'コネクタ・アプリ設定': 'Connectors & app settings',
  '本プロジェクトは非公式のコミュニティ製で、Anthropic社とは関係ありません。「Claude」はAnthropicの商標です。': 'This is an unofficial community project, not affiliated with Anthropic. "Claude" is a trademark of Anthropic.',
  // Avatar picker labels
  '仕事': 'Work', '個人': 'Personal', '検証': 'Testing', '開発': 'Dev', '会社': 'Company',
  '学習': 'Study', 'デザイン': 'Design', 'ローンチ': 'Launch', 'プロジェクト': 'Project',
  'お気に入り': 'Favorite', 'グローバル': 'Global', 'ハート': 'Heart', 'ツール': 'Tool',
  'アイデア': 'Idea', '資料': 'Docs', '会話': 'Chat',
  // Static create-view strings and chrome aria-labels.
  '環境一覧': 'Environments', 'テーマカラー': 'Accent color', 'ヘルプ': 'Help', 'アイコンを選ぶ': 'Choose an icon', '分け方': 'How to separate',
  '名前': 'Name', '例：仕事用、検証用': 'e.g. Work, Testing', 'アイコン（任意）': 'Icon (optional)',
  'メモ（任意）': 'Note (optional)', 'この環境のメモ': 'Note for this environment',
  '編集': 'Edit', '保存': 'Save', 'メモを書く': 'Add a note',
  '例：仕事用。会社のGoogleアカウントでサインイン': 'e.g. Work. Signs in with the company Google account',
  '既存のClaudeから、どう分けますか？': 'How do you want to separate this from your existing Claude?',
  'どのモードでも、サインインするアカウントは環境ごとに分かれます。課金や利用量はそのアカウントにひも付きます。違いは、会話・メモリ・設定をどこまで引き継ぐかです。':
    'In every mode the signed-in account is separate per environment, and billing and usage follow that account. What differs is how much of your conversations, memory, and settings carry over.',
  'アカウントだけ分ける': 'Separate the account only',
  '会話履歴も自動メモリも、共通ルールやスキルもそのまま引き継ぎます。分けるのは課金アカウントだけなので、研究と開発で利用量を分けつつ作業を続けられます。':
    'Conversation history, auto-memory, global rules and skills all carry over. Only the billing account is separated, so you can keep working while splitting usage between, say, research and development.',
  'アカウント': 'Account', '会話・メモリ': 'Conversations & memory',
  '会話とメモリも分ける': 'Separate conversations and memory too',
  '共通ルール・スキル・プラグインとツール権限は引き継ぎ、会話履歴と自動メモリはこの環境だけにします。慣れた設定はそのままに、用途ごとに会話を分けたいときに向いています。':
    'Global rules, skills, plugins and tool permissions carry over, while conversation history and auto-memory stay in this environment only. Good when you want your familiar setup but separate conversations per purpose.',
  'ルール・設定': 'Rules & settings',
  'すべて分ける': 'Separate everything', '推奨': 'Recommended',
  'ルールも設定も会話履歴もメモリも、何も引き継がずにまっさらな環境を作ります。既存のClaudeには一切触れないので、案件やクライアント、仕事と個人を完全に分けられます。':
    'Creates a blank environment that carries nothing over: no rules, settings, conversation history, or memory. It never touches your existing Claude, so you can fully separate projects, clients, or work and personal use.',
  '設定・会話': 'Settings & conversations',
  '項目ごとに詳しく設定する': 'Configure in detail, item by item',
  'モードの既定に戻す': 'Reset to mode defaults',
};

// Elements holding user data (environment names, filesystem paths, shell command)
// must never be run through the dictionary: a user could name an environment
// "仕事" and it must not become "Work". The default row's "既存の Claude" is
// translated at construction with T() instead, so these can be skipped wholesale.
const I18N_SKIP = '.profile-name, .detail-name, .profile-note, .detail-note, .usage-profile-name, .path-code, .code-line, code';
const setEN = (v) => (Object.prototype.hasOwnProperty.call(EN, v) ? EN[v] : null);

function applyI18nText(node) {
  if (LANG === 'ja') return;
  if (node.nodeType === 3) {
    if (node.parentElement && node.parentElement.closest(I18N_SKIP)) return;
    const en = setEN(node.textContent.trim());
    if (en != null) node.textContent = node.textContent.replace(node.textContent.trim(), en);
    return;
  }
  if (node.nodeType !== 1) return;
  // Elements carrying an explicit English phrase (composed markup: <strong>+text,
  // inline <code>) are replaced whole and not descended into.
  const whole = [];
  if (node.hasAttribute('data-en')) whole.push(node);
  node.querySelectorAll('[data-en]').forEach((e) => whole.push(e));
  for (const e of whole) e.textContent = e.getAttribute('data-en');
  // Attributes that surface as visible text, on this node and every descendant.
  const attrEls = [node, ...node.querySelectorAll('[placeholder], [aria-label], [title]')];
  for (const e of attrEls) {
    for (const attr of ['placeholder', 'aria-label', 'title']) {
      if (!e.hasAttribute || !e.hasAttribute(attr)) continue;
      const en = setEN(e.getAttribute(attr).trim());
      if (en != null) e.setAttribute(attr, en);
    }
  }
  // Remaining text nodes, skipping user-data containers and data-en subtrees.
  const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT, {
    acceptNode: (tn) =>
      tn.parentElement && tn.parentElement.closest(I18N_SKIP + ', [data-en]')
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT,
  });
  const texts = [];
  let n;
  while ((n = walker.nextNode())) texts.push(n);
  for (const tn of texts) {
    const en = setEN(tn.textContent.trim());
    if (en != null) tn.textContent = tn.textContent.replace(tn.textContent.trim(), en);
  }
}

if (LANG === 'en') {
  document.documentElement.lang = 'en';
  // main.js runs at end of <body>, so the static HTML already exists: translate it,
  // then observe for everything the app builds afterwards.
  applyI18nText(document.body);
  new MutationObserver((muts) => {
    for (const m of muts) for (const added of m.addedNodes) applyI18nText(added);
  }).observe(document.body, { childList: true, subtree: true });
}

// --- The sharing components the user can tune, grouped and described in plain
// language. config.json (OAuth tokens) and claude_desktop_config.json (account
// state, rewritten on launch) are deliberately absent: they are always isolated
// and never offered as a choice. -------------------------------------------
const SHARE_GROUPS = [
  {
    label: 'Claudeデスクトップアプリ',
    items: [
      { key: 'desktop_worktrees', name: 'ワークツリー一覧', desc: 'Gitワークツリーとrepoの対応' },
    ],
  },
  {
    label: 'Claude Code',
    items: [
      { key: 'cli_claude_md', name: '共通ルール', desc: 'CLAUDE.mdに書いた常時ルール' },
      { key: 'cli_rules', name: 'ルールファイル', desc: 'rules/に分けて置いた常時ルール' },
      { key: 'cli_plugins', name: 'プラグイン', desc: '導入したプラグイン' },
      { key: 'cli_skills', name: 'スキル', desc: 'カスタムスキル' },
      { key: 'cli_settings', name: 'ツール権限・フック', desc: 'ツールの実行可否とフック' },
      { key: 'cli_project_memory', name: 'プロジェクトの会話・メモリ', desc: 'プロジェクトごとの会話履歴と自動メモリ' },
      { key: 'cli_history', name: '入力履歴', desc: '入力したプロンプトの履歴' },
    ],
  },
];
const DEVICE_ID = { key: 'desktop_device_id', name: '端末ID', desc: '端末を識別するためのID' };
const ALL_KEYS = [...SHARE_GROUPS.flatMap((g) => g.items.map((i) => i.key)), DEVICE_ID.key];

// Labels for the isolation check (csw doctor). Reuses the share-UI vocabulary;
// the always-isolated items the share UI deliberately hides get their
// SPECIFICATION.md §3 names here.
const DOCTOR_LABELS = {
  ...Object.fromEntries(SHARE_GROUPS.flatMap((g) => g.items).map((i) => [i.key, i.name])),
  [DEVICE_ID.key]: DEVICE_ID.name,
  cli_sessions: 'セッション状態',
  desktop_app_config: 'アカウントのサインイン情報',
  desktop_config: 'コネクタ・アプリ設定',
};

// Mode presets — must mirror SharingConfig::{share_settings,share_workspace}_preset
// and build_sharing_config in main.rs. config.json / claude_desktop_config.json are
// always isolated and are not part of these maps.
const PRESETS = {
  share_code: {
    ...Object.fromEntries(ALL_KEYS.map((k) => [k, 'isolate'])),
    cli_project_memory: 'share',
    cli_history: 'share',
    desktop_worktrees: 'copy',
  },
  // すべて分ける: a fully separated environment, nothing carried over.
  isolate: Object.fromEntries(ALL_KEYS.map((k) => [k, 'isolate'])),
  // 会話とメモリも分ける: reuse the common setup, keep conversation history + memory and the account separate.
  share_settings: {
    cli_claude_md: 'share',
    cli_rules: 'share',
    cli_plugins: 'share',
    cli_skills: 'share',
    cli_settings: 'copy',
    desktop_worktrees: 'copy',
    cli_project_memory: 'isolate',
    cli_history: 'isolate',
    desktop_device_id: 'isolate',
  },
  // アカウントだけ分ける: also carry conversation history + memory across, separating only the account.
  share_workspace: {
    cli_claude_md: 'share',
    cli_rules: 'share',
    cli_plugins: 'share',
    cli_skills: 'share',
    cli_settings: 'copy',
    desktop_worktrees: 'copy',
    cli_project_memory: 'share',
    cli_history: 'share',
    desktop_device_id: 'isolate',
  },
};

// --- App state --------------------------------------------------------------
let profiles = [];
let activeName = 'default';
let desktopRunning = false; // whether any Claude Desktop is running right now
let runningProfiles = []; // names of environments whose Claude is running (can be several)
let selectedName = null;
let currentMode = 'share_code';
let overrides = { ...PRESETS.share_code };
let advancedCustomized = false;
let usageReports = null;
let usageReadPending = false;
let pendingLaunch = { request_id: 0, target: null, phase: 'idle', error: null, wait_for_exit: true };
let pendingRead = false;

// --- DOM builder helpers (no innerHTML) -------------------------------------
const SVG_NS = 'http://www.w3.org/2000/svg';

function h(tag, props, ...kids) {
  const node = document.createElement(tag);
  if (props) {
    for (const k in props) {
      const v = props[k];
      if (v == null || v === false) continue;
      if (k === 'class') node.className = v;
      else if (k === 'text') node.textContent = v;
      else if (k === 'dataset') Object.assign(node.dataset, v);
      else if (k.startsWith('on') && typeof v === 'function') node.addEventListener(k.slice(2), v);
      else if (k === 'disabled' || k === 'checked' || k === 'hidden') { if (v) node[k] = true; }
      else node.setAttribute(k, v);
    }
  }
  appendKids(node, kids);
  return node;
}

function appendKids(node, kids) {
  for (const c of kids.flat(Infinity)) {
    if (c == null || c === false) continue;
    node.appendChild(typeof c === 'object' ? c : document.createTextNode(String(c)));
  }
}

function icon(id, cls) {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('class', cls || 'icon');
  const use = document.createElementNS(SVG_NS, 'use');
  use.setAttribute('href', '#' + id);
  svg.appendChild(use);
  return svg;
}

const $ = (id) => document.getElementById(id);

// Avatar icons: a curated set drawn from the app's Phosphor sprite (never
// hand-rolled). A profile's `icon` is either one of these slugs (rendered as the
// SVG glyph) or a short emoji/character (rendered as text); empty falls back to
// the first letter of the name.
const AVATAR_ICONS = [
  { slug: 'briefcase', label: '仕事' },
  { slug: 'user', label: '個人' },
  { slug: 'flask', label: '検証' },
  { slug: 'code', label: '開発' },
  { slug: 'buildings', label: '会社' },
  { slug: 'graduation-cap', label: '学習' },
  { slug: 'palette', label: 'デザイン' },
  { slug: 'rocket', label: 'ローンチ' },
  { slug: 'folder', label: 'プロジェクト' },
  { slug: 'star', label: 'お気に入り' },
  { slug: 'globe', label: 'グローバル' },
  { slug: 'heart', label: 'ハート' },
  { slug: 'wrench', label: 'ツール' },
  { slug: 'lightbulb', label: 'アイデア' },
  { slug: 'book-open', label: '資料' },
  { slug: 'chat-circle', label: '会話' },
];
const ICON_SLUGS = AVATAR_ICONS.map((i) => i.slug);

function avatarContent(iconVal, name) {
  if (iconVal && ICON_SLUGS.includes(iconVal)) return icon('i-' + iconVal, 'icon avatar-glyph');
  if (iconVal) return iconVal;
  return (name || '?').charAt(0).toUpperCase();
}
const reduceMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
function withTransition(update) {
  if (document.startViewTransition && !reduceMotion()) document.startViewTransition(update);
  else update();
}

// --- DOM refs ---------------------------------------------------------------
const el = {
  profileList: $('profileList'),
  dmgBanner: $('dmgBanner'),
  pendingLaunchBanner: $('pendingLaunchBanner'),
  pendingLaunchTarget: $('pendingLaunchTarget'),
  pendingLaunchText: $('pendingLaunchText'),
  btnCancelPendingLaunch: $('btnCancelPendingLaunch'),
  btnRecoverPendingLaunch: $('btnRecoverPendingLaunch'),
  btnDmgEject: $('btnDmgEject'),
  btnDmgLater: $('btnDmgLater'),
  takeoverBanner: $('takeoverBanner'),
  reopenBanner: $('reopenBanner'),
  reopenText: $('reopenText'),
  btnReopen: $('btnReopen'),
  btnReopenClose: $('btnReopenClose'),
  takeoverText: $('takeoverText'),
  btnTakeoverClose: $('btnTakeoverClose'),
  btnCreate: $('btnCreate'),
  btnCreateEmpty: $('btnCreateEmpty'),
  viewEmpty: $('viewEmpty'),
  viewDetail: $('viewDetail'),
  viewCreate: $('viewCreate'),
  viewUsage: $('viewUsage'),
  btnUsage: $('btnUsage'),
  btnUsageRefresh: $('btnUsageRefresh'),
  usageReports: $('usageReports'),
  usageMessage: $('usageMessage'),
  usageScroll: $('usageScroll'),
  usageFade: $('usageFade'),
  detailContent: $('detailContent'),
  detailFooter: $('detailFooter'),
  emptyFirstRun: $('emptyFirstRun'),
  firstRunExtra: $('firstRunExtra'),
  inputName: $('inputName'),
  inputNote: $('inputNote'),
  iconPicker: $('iconPicker'),
  createNotice: $('createNotice'),
  nameError: $('nameError'),
  advancedToggle: $('advancedToggle'),
  advancedState: $('advancedState'),
  advancedReset: $('advancedReset'),
  advancedGroups: $('advancedGroups'),
  createFooter: $('createFooter'),
  emptyScroll: $('emptyScroll'),
  emptyFade: $('emptyFade'),
  detailScroll: $('detailScroll'),
  detailFade: $('detailFade'),
  createScroll: $('createScroll'),
  createFade: $('createFade'),
  toastArea: $('toastArea'),
  appVersion: $('appVersion'),
  btnAbout: $('btnAbout'),
};

// --- Toast ------------------------------------------------------------------
function showToast(msg, isError) {
  const t = h('div', { class: 'toast' + (isError ? ' error' : '') },
    icon(isError ? 'i-info' : 'i-check'), h('span', { text: msg }));
  if (isError) t.setAttribute('role', 'alert'); // announce failures assertively
  el.toastArea.appendChild(t);
  setTimeout(() => {
    t.style.transition = 'opacity .2s';
    t.style.opacity = '0';
    setTimeout(() => t.remove(), 220);
  }, 3200);
}

// --- View switching ---------------------------------------------------------
function setView(view) {
  el.viewEmpty.hidden = view !== 'empty';
  el.viewDetail.hidden = view !== 'detail';
  el.viewCreate.hidden = view !== 'create';
  el.viewUsage.hidden = view !== 'usage';
  el.btnUsage.classList.toggle('selected', view === 'usage');
  el.btnUsage.setAttribute('aria-pressed', view === 'usage' ? 'true' : 'false');
  requestAnimationFrame(refreshFades);
}

// An environment is "in use" (利用中) when a Claude Desktop instance is actually
// running for it. Fully-isolated environments can run side by side, so several may
// be in use at once. Resolved per environment by the backend from the live
// processes' --user-data-dir (csw_core::switcher::desktop_dir_running); quitting a
// Claude clears its marker so that environment can be launched again.
const inUse = (name) => runningProfiles.includes(name);

// --- Sidebar ----------------------------------------------------------------
function renderSidebar() {
  // One rail: "既存の Claude" (default) pinned first, then created environments.
  // default is a selectable row like any other, not a separate card. Pin it
  // explicitly so order never depends on the backend's list_profiles order.
  const created = profiles.filter((p) => p.name !== 'default');
  const defaultP = profiles.find((p) => p.name === 'default') || { name: 'default', icon: '', is_default: true };
  const ordered = [defaultP, ...created];

  const rows = ordered.map((p) => {
    const isDefault = p.name === 'default';
    const isInUse = inUse(p.name);
    const open = () => withTransition(() => showDetail(p.name));
    // One pill at most: the "利用中" row, shown only while Claude Desktop is
    // actually running for it. The default row needs no extra marker — its name
    // ("既存の Claude") + monitor icon + first position already signal the baseline.
    const pill = isInUse
      ? h('span', { class: 'pill pill-active', text: '利用中' })
      : null;
    // Sub lines under the name: the user's own note first (never translated),
    // then the launch stamp. Rows without them keep the compact height.
    const subs = [];
    if (!isDefault && p.note) subs.push(h('span', { class: 'profile-note', text: p.note }));
    if (!isDefault && p.last_launched_at) {
      subs.push(h('span', {
        class: 'profile-meta',
        text: T(`最終起動：${relTime(p.last_launched_at)}`, `Last launched ${relTime(p.last_launched_at)}`),
      }));
    }
    return h('li', {
      class: 'profile-item' + (p.name === selectedName ? ' selected' : ''),
      role: 'button', tabindex: '0', onclick: open,
      'aria-label': isDefault ? 'あなたの標準環境である既存のClaude' : null,
      'aria-current': isInUse ? 'true' : null, // current = the running environment
      onkeydown: (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); open(); } },
    },
      h('span', { class: 'profile-avatar' }, isDefault ? icon('i-monitor') : avatarContent(p.icon, p.name)),
      h('span', { class: 'profile-text' },
        h('span', { class: 'profile-top' },
          h('span', { class: 'profile-name', text: isDefault ? T('既存のClaude', 'Existing Claude') : p.name }),
          pill),
        ...subs));
  });
  el.profileList.replaceChildren(...rows);
}

// --- Detail view ------------------------------------------------------------

// These names describe known limit windows; identifiers and profile names from
// disk are never used as markup, translated, or evaluated.
const USAGE_WINDOW_LABELS = {
  five_hour: ['5時間の上限', '5-hour limit'],
  seven_day: ['7日間の上限', '7-day limit'],
  seven_day_opus: ['Opusの7日間の上限', 'Opus 7-day limit'],
  seven_day_sonnet: ['Sonnetの7日間の上限', 'Sonnet 7-day limit'],
  seven_day_oauth_apps: ['OAuthアプリの7日間の上限', 'OAuth apps 7-day limit'],
  seven_day_cowork: ['Coworkの7日間の上限', 'Cowork 7-day limit'],
  seven_day_omelette: ['追加の7日間の上限', 'Additional 7-day limit'],
  omelette_promotional: ['追加のプロモーション上限', 'Additional promotional limit'],
};

function usageDate(ms) {
  if (!Number.isFinite(ms) || ms <= 0) return null;
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleString(LANG === 'ja' ? 'ja-JP' : 'en-US', {
    year: 'numeric', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit', timeZoneName: 'short',
  });
}

function usageStateLabel(state) {
  return ({
    fresh: T('最近の記録', 'Recent reading'),
    stale: T('古いデータ', 'Stale data'),
    missing: T('記録なし', 'No cached data'),
    unavailable: T('読み取り不可', 'Unavailable'),
  })[state] || T('読み取り不可', 'Unavailable');
}

function renderUsageWindow(window) {
  const labels = Object.hasOwn(USAGE_WINDOW_LABELS, window.key) ? USAGE_WINDOW_LABELS[window.key] : null;
  if (!labels) return null;
  const label = T(...labels);
  const percent = Number.isFinite(window.used_percent) && window.used_percent >= 0 && window.used_percent <= 100
    ? window.used_percent : null;
  const formatted = percent == null ? T('不明', 'Unknown')
    : T(`${new Intl.NumberFormat('ja-JP', { maximumFractionDigits: 1 }).format(percent)}%使用`, `${new Intl.NumberFormat('en-US', { maximumFractionDigits: 1 }).format(percent)}% used`);
  const row = h('div', { class: 'usage-window' },
    h('div', { class: 'usage-window-heading' },
      h('span', { class: 'usage-window-label', text: label }),
      h('span', { class: 'usage-value', text: formatted })));
  if (percent != null) {
    const bar = h('div', {
      class: 'usage-bar', role: 'progressbar', 'aria-label': label,
      'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': String(percent),
      'aria-valuetext': formatted,
    });
    const fill = h('div', { class: 'usage-bar-fill' + (percent >= 90 ? ' usage-high' : percent >= 75 ? ' usage-elevated' : '') });
    fill.style.width = `${percent}%`;
    bar.appendChild(fill);
    row.appendChild(bar);
  }
  const reset = usageDate(window.resets_at_ms);
  const resetText = reset == null ? T('リセット時刻：記録なし', 'Reset time: not recorded')
    : window.state === 'reset_passed' || window.resets_at_ms <= Date.now()
      ? T(`リセット予定時刻：${reset}。新しい記録が必要です。`, `Reset was scheduled for ${reset}. A newer reading is needed.`)
      : T(`リセット予定時刻：${reset}`, `Scheduled reset: ${reset}`);
  row.appendChild(h('p', { class: 'usage-reset', text: resetText }));
  return row;
}

function renderUsageReports() {
  const reports = Array.isArray(usageReports) ? usageReports : [];
  el.usageReports.replaceChildren(...reports.map((report) => {
    const windows = Array.isArray(report.windows) ? report.windows : [];
    let state = ['fresh', 'stale', 'missing', 'unavailable'].includes(report.state) ? report.state : 'unavailable';
    // A retained snapshot can age while a later read fails. Only downgrade its
    // presentation; never synthesize a new reading or reset a percentage.
    if (state === 'fresh' && (Date.now() - report.captured_at_ms > 30 * 60000
      || windows.some((w) => Number.isFinite(w.resets_at_ms) && w.resets_at_ms <= Date.now()))) state = 'stale';
    const captured = usageDate(report.captured_at_ms);
    const age = captured ? relTime(new Date(report.captured_at_ms).toISOString()) : null;
    const checkedText = captured
      ? T(`最終確認：${captured}・${age}`, `Last checked: ${captured} · ${age}`)
      : T('最終確認：記録なし', 'Last checked: not recorded');
    return h('article', { class: 'usage-report usage-' + state },
      h('div', { class: 'usage-report-heading' },
        h('h2', { class: 'usage-profile-name', text: report.profile === 'default' ? T('既存のClaude', 'Existing Claude') : report.profile }),
        h('span', { class: 'usage-state', text: usageStateLabel(state) })),
      h('p', { class: 'usage-checked', text: checkedText }),
      ...windows.map(renderUsageWindow),
      report.source === 'history'
        ? h('p', { class: 'usage-source', text: T('利用量の履歴から表示しています。リセット時刻は保存されていません。', 'Shown from usage history. Reset times are not saved in that history.') }) : null,
      state === 'missing'
        ? h('p', { class: 'usage-source', text: T('この環境には利用量の記録がありません。環境を使うとClaudeが記録を保存する場合があります。', 'No usage reading is saved in this environment. Claude may save one when you use it.') }) : null,
      state === 'unavailable'
        ? h('p', { class: 'usage-source', text: T('対応する形式の記録を安全に読み取れませんでした。', 'A supported reading could not be read safely.') }) : null);
  }));
  requestAnimationFrame(refreshFades);
}

async function loadUsageReports() {
  if (usageReadPending) return;
  usageReadPending = true;
  el.btnUsageRefresh.disabled = true;
  el.usageReports.setAttribute('aria-busy', 'true');
  el.usageMessage.textContent = T('手元の保存内容を確認しています。', 'Checking the local cache.');
  try {
    const reports = await invoke('get_usage_reports');
    if (!Array.isArray(reports)) throw new Error('usage_unavailable');
    usageReports = reports;
    renderUsageReports();
    el.usageMessage.textContent = reports.length ? '' : T('環境が見つかりませんでした。', 'No environments were found.');
  } catch (error) {
    renderUsageReports();
    el.usageMessage.textContent = String(error) === 'usage_read_pending'
      ? T('保存内容の確認中です。しばらくしてから再確認してください。', 'A local check is already running. Recheck in a moment.')
      : T('保存内容を再確認できませんでした。以前の記録がある場合は表示を残しています。', 'The local cache could not be rechecked. Any previous readings remain visible.');
  } finally {
    usageReadPending = false;
    el.btnUsageRefresh.disabled = false;
    el.usageReports.removeAttribute('aria-busy');
    refreshFades();
  }
}

function showUsage() {
  selectedName = null;
  renderSidebar();
  setView('usage');
  renderUsageReports();
  return loadUsageReports();
}

async function showDetail(name) {
  selectedName = name;
  let d;
  try {
    d = await invoke('get_profile_details', { name });
  } catch (err) {
    showToast('環境を読み込めませんでした。', true);
    return;
  }
  renderSidebar();
  const isDefault = !!d.is_default;
  const isInUse = inUse(name);

  const header = h('div', { class: 'detail-header' },
    h('div', { class: 'detail-bezel' },
      h('div', { class: 'detail-avatar' }, isDefault ? icon('i-monitor') : avatarContent(d.icon, name))),
    h('div', { class: 'detail-titles' },
      h('div', { class: 'detail-name', text: isDefault ? T('既存のClaude', 'Existing Claude') : name }),
      h('div', { class: 'detail-tagline', text: isDefault ? 'あなた自身の環境' : (isInUse ? '利用中の環境' : '作成した環境') })),
    isInUse ? h('span', { class: 'pill pill-active', style: 'margin-left:auto', 'aria-label': '利用中', text: '利用中' }) : null);

  const nodes = [header];
  const codeSharing = codeSharingSection(d);
  if (codeSharing) nodes.push(codeSharing);

  if (!isDefault) {
    // The sign-in signpost comes first: it exists only during the environment's
    // very first launch, exactly when the empty Claude window is on screen.
    const signpost = signpostSection(d);
    if (signpost) nodes.push(signpost);
    // Identity next: the user's own note and the environment's record (created,
    // duplicated from, last launched) answer "which environment is this?" before
    // the sharing state below answers "what does it inherit?".
    nodes.push(noteSection(d));
  }

  if (isDefault) {
    // Keep only the reassurance (the user's real worry: "will this break my
    // setup?"). The paths are collapsed; the "all 11 shared" line is dropped as
    // self-evident for the baseline.
    nodes.push(section('', [
      h('div', { class: 'note-card', dataset: { en: 'This is your own Claude Desktop App and Claude Code setup. Only explicitly enabling complete local Code sharing synchronizes its Code stores; sign-ins stay unchanged.' } },
        'あなたが普段使っている', h('strong', { text: 'Claudeデスクトップアプリ' }), 'と',
        h('strong', { text: 'Claude Code' }),
        'の環境です。ローカルCode全体の共有を明示的に有効にした場合だけCodeストアを同期し、ログインは変更しません。'),
    ]));
    nodes.push(pathsSection(d, true));
  } else {
    // State first (what this environment inherits), then collapsed detail, then
    // a one-line switch hint by the action. The "利用中"/multi-window concept lives
    // in onboarding, not repeated as an always-on block here.
    nodes.push(sharingDisclosure(d.sharing, !!d.cloned_from, !!d.code_sharing?.enabled));
    nodes.push(pathsSection(d, false));
    nodes.push(doctorSection(name));
    nodes.push(terminalSection(name));
    // Cloning is a rare management action, so it sits here as a quiet button rather
    // than taking a prominent footer slot next to the launch actions.
    nodes.push(cloneSection(name));
    // Explain the footer's launch buttons, right above them (last content block), so
    // each short-label button reads unambiguously.
    nodes.push(launchHelp(!!d.supports_concurrent_windows));
  }

  el.detailContent.replaceChildren(...nodes);
  renderDetailFooter(name, isDefault, isInUse, !!d.supports_concurrent_windows);
  setView('detail');
}

function codeSharingSection(d) {
  const status = d.code_sharing;
  if (status?.unavailable) return section(T('ローカルCodeの共有', 'Shared local Code'), [
    h('p', { class: 'note-card', text: T('共有の状態を確認できません。設定を変更せず、CLIで状態を確認してください。', 'Sharing status is unavailable. Leave its configuration unchanged and inspect it with the CLI.') }),
  ]);
  if (!status || (!status.enabled && !status.eligible)) return null;
  if (status.enabled) {
    return section(T('ローカルCodeの共有', 'Shared local Code'), [
      h('p', { class: 'note-card', text: T(
        `${status.participants}環境で有効です。Codeのスレッド・メモリ・履歴・リポジトリ・プリセットは、Claudeを手動で終了した後の切り替え時に引き継ぎます。ログインとクラウドChatは別々です。`,
        `Enabled across ${status.participants} environments. Code threads, memory, history, repos and presets carry over when you switch after quitting Claude manually. Logins and cloud Chat remain separate.`) }),
    ]);
  }
  const source = status.source === 'default' ? T('既存のClaude', 'Existing Claude') : status.source;
  const controls = h('div', { class: 'footer-group code-sharing-controls' });
  const canSetUp = !desktopRunning && !['waiting', 'switching'].includes(pendingLaunch.phase);
  const setup = h('button', {
    type: 'button', class: 'btn btn-ghost', disabled: !canSetUp, dataset: { action: 'setup-code' },
    onclick: () => {
      controls.replaceChildren(
        h('p', { class: 'firstrun-body', text: T(
          `「${d.name}」の現在のローカルCodeデータをバックアップして、「${source}」から共有を開始します。Code内に保存された権限・コネクタ参照・スケジュールタスクも含みます。既存の共有グループがあれば最新の変更を引き継ぎます。`,
          `Back up and replace "${d.name}"'s complete local Code data using "${source}", including saved Code permissions, connector references and scheduled tasks. If sharing is already enabled, carry forward the group's latest changes first.`) }),
        h('button', { type: 'button', class: 'btn btn-ghost', dataset: { action: 'cancel-code' }, onclick: () => showDetail(d.name) }, T('やめる', 'Cancel')),
        h('button', { type: 'button', class: 'btn btn-primary', dataset: { action: 'confirm-code' }, onclick: async (event) => {
          event.currentTarget.disabled = true;
          try {
            const backup = await invoke('enable_code_sharing', { name: d.name, replace: true });
            await showDetail(d.name);
            showToast(T(`共有を有効にしました。バックアップ：${backup}`, `Shared Code enabled. Backup: ${backup}`));
          } catch (error) {
            showToast(String(error), true);
            await showDetail(d.name);
          }
        } }, T('バックアップして共有を有効にする', 'Back up and enable sharing')));
    },
  }, T('Codeの共有を設定', 'Set up shared Code'));
  controls.append(setup);
  return section(T('ローカルCodeの共有', 'Shared local Code'), [
    h('p', { class: 'note-card', text: T(
      'サイドバーの共有はまだ有効ではありません。両方の環境で通常どおりサインインしてCodeを一度開き、作業と入力待ちが終わってからClaudeを手動で終了し、ここで共有を有効にしてください。CLIへのサインインは不要です。',
      'Sidebar sharing is not enabled yet. Sign in normally and open Code once in both environments. Finish all tasks and input waits, quit Claude manually, then enable sharing here. No CLI sign-in is needed.') }),
    controls,
  ]);
}

// Clone is a quiet, infrequent management action. It is presented as a labeled row
// in a flat bordered card (minimalist-ui: clear hierarchy, no orphan button) with a
// one-line explanation of what it does, so it never reads as a mystery button. It
// stays out of the footer so the footer's slots are reserved for launching.
// The Claude Desktop App sign-in lives inside the environment folder and is
// copied as is, so the card says so before the user duplicates and points a
// different account to the create button (its label, 環境を作る).
function cloneSection(name) {
  return section('この環境の複製', [
    h('div', { class: 'manage-row' },
      h('p', { class: 'firstrun-body manage-text', text: 'この環境の設定とデータをそのまま写して、別の名前の環境を作ります。Claudeデスクトップアプリのサインインの情報もそのまま写すので、複製した環境は元の環境と同じサインインの状態を持ちます。別のアカウントで使う環境は「環境を作る」から作ってください。' }),
      h('button', { type: 'button', class: 'btn btn-ghost manage-action', onclick: () => showCloneRow(name) },
        icon('i-duplicate'), h('span', { text: '複製' }))),
  ]);
}

// The isolation check (csw doctor) is a quiet, on-demand, read-only action.
// Same presentation as cloneSection: a labeled row with a one-line explanation
// and a ghost button; the report renders below when run. The GUI never repairs
// anything (auto-fixes are the classic source of profile-switcher accidents);
// re-pointing broken links stays in the CLI as `csw doctor --fix`.
function doctorSection(name) {
  const results = h('div', { class: 'doctor-results' });
  const btn = h('button', {
    type: 'button', class: 'btn btn-ghost manage-action',
    onclick: () => runDoctor(name, results, btn),
  }, icon('i-check'), h('span', { text: '検査する' }));
  return section('分離の検査', [
    h('div', { class: 'manage-row' },
      h('p', { class: 'firstrun-body manage-text', text: '共有と分離が設定どおりに保たれているかを、この場で点検します。ファイルの中身は読まず、何も書き換えません。' }),
      btn),
    results,
  ]);
}

async function runDoctor(name, results, btn) {
  btn.disabled = true;
  try {
    const report = await invoke('inspect_profile', { name });
    results.replaceChildren(renderDoctorReport(report));
  } catch (err) {
    showToast('検査できませんでした。', true);
  } finally {
    btn.disabled = false;
  }
}

// The four link points that are structurally isolated in every mode
// (SPECIFICATION.md §3「常に分離する項目」). Mirrors LINK_ITEMS' fixed_mode.
const ALWAYS_ISOLATED_KEYS = ['desktop_config', 'cli_sessions', 'desktop_device_id', 'desktop_app_config'];

// Status text per item. Healthy states are short labels; issues add one full
// explanatory sentence below the row. Copy stays a distinct word from 分離
// (csw_product_canon: 共有 / 分離 / コピー only). item.mode carries the serde
// form of SharingMode ('share' / 'copy' / 'isolate', lowercase).
function doctorStatus(item) {
  const st = item.health.state;
  if (st === 'shared_ok') return { status: '正常に共有', issue: false };
  if (st === 'isolated_ok') {
    return { status: item.mode === 'copy' ? '正常にコピー' : '正常に分離', issue: false };
  }
  if (st === 'source_absent') return { status: '共有元がまだありません', issue: false };
  const unexpectedLink = ALWAYS_ISOLATED_KEYS.includes(item.key)
    ? '常に分離する項目がリンクになっています。'
    : (item.mode === 'copy' ? 'コピーの項目がリンクになっています。' : '分離の項目がリンクになっています。');
  const detail = {
    source_missing: '共有元が見つかりません。',
    wrong_target: item.health.fixable
      ? 'リンク先が想定と異なります。ターミナルで「csw doctor --fix」を実行すると張り直せます。'
      : 'リンク先が想定と異なり、共有元も見つかりません。',
    // csw doctor --fix replaces a folder only if it is empty (rmdir); the check
    // never lists it, so it cannot say which case applies here. The empty-folder
    // replacement exists only in csw 0.24.2 and later, and csw is updated
    // separately from the app, so the message names the version.
    materialized: item.health.is_directory
      ? '共有のリンクの位置に実体のフォルダがあります。フォルダが空なら、ターミナルで「csw doctor --fix」を実行するとリンクに置き換えられます。この置き換えにはcsw 0.24.2以降が必要です。中身があるときは、手元の内容を失わないよう、自動では直しません。'
      : '共有のリンクが実体のファイルに置き換わっています。手元の内容を失わないよう、自動では直しません。',
    missing_link: '共有のリンクがありません。ターミナルで「csw doctor --fix」を実行すると作れます。',
    unexpected_link: unexpectedLink,
  }[st] || '状態を判定できませんでした。';
  // The three repairs csw doctor --fix performs (inspector.rs): re-point a
  // fixable wrong link, replace an empty folder, create a missing link. The
  // report adds where to get csw when any of them applies.
  const usesCsw = (st === 'wrong_target' && !!item.health.fixable)
    || (st === 'materialized' && !!item.health.is_directory)
    || st === 'missing_link';
  return { status: '要確認', detail, issue: true, usesCsw };
}

function renderDoctorReport(report) {
  const wrap = h('div', { class: 'doctor-report' });
  if (report.running) {
    wrap.appendChild(h('p', { class: 'doctor-note', text: 'この環境のClaudeデスクトップアプリが起動中です。書き換えの途中を検査した可能性があるため、問題が出た場合はClaudeを終了してからもう一度検査してください。' }));
  }
  const n = report.items.length;
  const c = report.issue_count;
  const summary = c === 0
    ? T(`問題は見つかりませんでした。${n}項目すべてが設定どおりです。`, `No issues found. All ${n} items match the settings.`)
    : T(`${c}件の問題が見つかりました。`, c === 1 ? '1 issue found.' : `${c} issues found.`);
  wrap.appendChild(h('p', {
    class: report.issue_count === 0 ? 'doctor-summary' : 'doctor-summary doctor-summary-issue',
    text: summary,
  }));
  const list = h('div', { class: 'doctor-list' });
  let usesCsw = false;
  for (const item of report.items) {
    const label = DOCTOR_LABELS[item.key] || item.key;
    const s = doctorStatus(item);
    if (s.usesCsw) usesCsw = true;
    list.appendChild(h('div', { class: 'doctor-item' },
      h('span', { class: 'doctor-item-label', text: label }),
      h('span', { class: s.issue ? 'doctor-item-state doctor-item-issue' : 'doctor-item-state', text: s.status })));
    if (s.detail) list.appendChild(h('p', { class: 'doctor-item-detail', text: s.detail }));
  }
  wrap.appendChild(list);
  // The GUI never repairs, so a csw doctor --fix hint is the only repair path:
  // say once, under the results, where the csw command comes from.
  if (usesCsw) wrap.appendChild(cswGetRow());
  return wrap;
}

// Describe the footer's launch buttons just above them. Uses the same bordered
// head+body card the onboarding uses to explain multiple items (minimalist-ui:
// flat, 1px border, clear hierarchy) rather than a bare muted paragraph. Copy is
// plain prose with the reason before the instruction, not a term:definition list.
// "重複して起動" only appears for fully-isolated environments (they run concurrently).
function launchHelp(supportsConcurrent) {
  const way = (head, body) => h('div', { class: 'launch-way' },
    h('p', { class: 'firstrun-head', text: head }),
    h('p', { class: 'firstrun-body', text: body }));
  const ways = [
    way('この環境を起動', 'この環境でClaudeを開きます。ほかのClaudeが起動中なら、手動ですべて終了するまで待ってから開きます。'),
  ];
  if (supportsConcurrent) {
    ways.push(way('重複して起動', '起動中のClaudeを終了せずに、この環境を新しいウィンドウで同時に開きます。'));
  }
  return section('起動のしかた', [h('div', { class: 'firstrun-card' }, ...ways)]);
}

function section(label, children) {
  return h('div', { class: 'section' },
    label ? h('div', { class: 'section-label', text: label }) : null, ...children);
}

function pathsSection(d, isDefault) {
  if (isDefault) {
    // The default is the user's real Claude data: paths only, never aggregated.
    const rows = [
      pathRow('Claudeデスクトップアプリ', d.desktop_path, 'default', 'desktop'),
      pathRow('Claude Code', d.cli_path, 'default', 'cli'),
      h('p', { class: 'path-caption', text: 'これはCSWが作った場所ではなく、あなた自身のClaudeフォルダです。' }),
    ];
    return section('', [disclosure('あなたのClaudeフォルダの場所', null, rows)]);
  }
  const name = d.name;
  const rows = [
    pathRow('Claudeデスクトップアプリ', d.desktop_path, name, 'desktop'),
    pathRow('Claude Code', d.cli_path, name, 'cli'),
  ];
  // The breakdown (sizes, link targets) is aggregated lazily on first open:
  // it stats files but never reads contents and never follows symlinks.
  const details = h('div', { class: 'datamap' });
  // Loaded on open; retried on the next open after a failure.
  let loading = false;
  let loaded = false;
  const loadMap = async () => {
    if (loading || loaded) return;
    loading = true;
    details.replaceChildren(h('p', { class: 'datamap-loading', text: '読み込んでいます…' }));
    try {
      const map = await invoke('get_profile_data_map', { name });
      details.replaceChildren(renderDataMap(map));
      loaded = true;
    } catch (err) {
      details.replaceChildren(h('p', { class: 'datamap-loading', text: '内訳を読み込めませんでした。' }));
    } finally {
      loading = false;
    }
  };
  return section('', [disclosure('この環境のデータの場所', null, [...rows, details], loadMap)]);
}

// Item labels for the data map: same vocabulary as the isolation check.
function fmtBytes(n) {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) { v /= 1024; u += 1; }
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[u]}`;
}

function fmtEpoch(epoch) {
  const d = new Date(epoch * 1000);
  return d.toLocaleDateString(LANG === 'ja' ? 'ja-JP' : 'en-US', { year: 'numeric', month: 'short', day: 'numeric' });
}

// RFC 3339 string (created_at / last_launched_at) to a localized date.
function fmtDate(rfc3339) {
  const t = Date.parse(rfc3339);
  if (Number.isNaN(t)) return '';
  return new Date(t).toLocaleDateString(LANG === 'ja' ? 'ja-JP' : 'en-US', { year: 'numeric', month: 'short', day: 'numeric' });
}

// Relative time for the launch stamp. Dynamic strings are built per language
// with T() because runtime-concatenated text never matches the EN dictionary.
function relTime(rfc3339) {
  const then = Date.parse(rfc3339);
  if (Number.isNaN(then)) return '';
  const mins = Math.floor(Math.max(0, Date.now() - then) / 60000);
  const hours = Math.floor(mins / 60);
  const days = Math.floor(hours / 24);
  if (mins < 1) return T('たった今', 'just now');
  if (hours < 1) return T(`${mins}分前`, mins === 1 ? '1 minute ago' : `${mins} minutes ago`);
  if (days < 1) return T(`${hours}時間前`, hours === 1 ? '1 hour ago' : `${hours} hours ago`);
  if (days < 30) return T(`${days}日前`, days === 1 ? '1 day ago' : `${days} days ago`);
  return fmtDate(rfc3339);
}

// The sign-in signpost: a one-time card shown while a newly created
// environment has been launched exactly once, which is precisely when the
// freshly opened Claude is empty and asks for a sign-in. When it applies is
// decided in core (ProfileManager::signin_signpost_due): environments from
// before the creation stamp existed and duplicates (they open signed in to
// the source's account) never show it. Sign-in completion is deliberately not
// detected: that would require reading file contents (docs/PRIVACY.md), so
// the card simply stays until dismissed or until the second launch.
// All lines are built with T(): they embed the user's note, and runtime-
// concatenated text never matches the EN dictionary.
function signpostSection(d) {
  if (!d.signin_signpost_due) return null;
  // Keyed by name + creation stamp: deleting an environment and recreating it
  // under the same name is a new environment whose first launch needs the
  // card again, so an old dismissal must not suppress it.
  const key = `csw_signpost_${d.name}_${d.created_at}`;
  if (localStorage.getItem(key)) return null;

  const wrap = h('div', { class: 'section' });
  const card = h('div', { class: 'note-card signpost-card' },
    h('p', {
      text: T('開いたClaudeがまっさらでサインインを求めてくるのは正常です。この環境には専用のデータ領域が用意されています。',
        'It is normal that the Claude you just opened is empty and asks you to sign in. This environment starts with its own dedicated data.'),
    }),
    h('p', {
      text: T('この環境で使うアカウントでサインインしてください。',
        'Sign in with the account you use for this environment.'),
    }));
  if (d.note) {
    // The user's own words, quoted so they never read as part of CSW's copy.
    card.append(h('p', {
      text: T(`メモには「${d.note}」と書いてあります。`, `Your note says: "${d.note}".`),
    }));
  }
  card.append(h('p', {
    text: T('既存のClaudeのクラウドChatとログインは維持します。ローカルCode全体の共有を有効にした場合は、Codeストアが同期されます。',
      'Existing Claude keeps its cloud Chat history and sign-in. When complete local Code sharing is enabled, its Code stores are synchronized.'),
  }));
  if (d.sharing && d.sharing.cli_project_memory !== 'isolate') {
    // Shared or copied: either way the local Claude Code history is here,
    // while the empty desktop window still needs its sign-in.
    card.append(h('p', {
      text: T('Claude Codeの会話履歴と自動メモリは、この環境に引き継ぎ済みです。デスクトップには、サインインしたアカウントの会話が表示されます。',
        'Your Claude Code conversation history and auto-memory have carried over into this environment. Desktop conversations follow the account you sign in with.'),
    }));
  }
  card.append(h('div', { class: 'signpost-actions' },
    h('button', {
      type: 'button', class: 'btn btn-ghost',
      onclick: () => { localStorage.setItem(key, '1'); wrap.remove(); },
    }, '閉じる')));
  wrap.append(card);
  return wrap;
}

// The note and record card: the free-form note (editable in place) with the
// environment's record underneath. Never shown for the default environment.
function noteSection(d) {
  const name = d.name;
  const box = h('div', { class: 'note-box' });

  const facts = [];
  if (d.created_at) facts.push(T(`作成：${fmtDate(d.created_at)}`, `Created ${fmtDate(d.created_at)}`));
  if (d.cloned_from) facts.push(T(`複製元：${d.cloned_from}`, `Duplicated from ${d.cloned_from}`));
  facts.push(d.last_launched_at
    ? T(`最終起動：${relTime(d.last_launched_at)}`, `Last launched ${relTime(d.last_launched_at)}`)
    : T('最終起動：まだありません', 'Never launched'));
  const factsRow = h('div', { class: 'detail-facts' }, ...facts.map((f) => h('span', { text: f })));

  const renderRead = () => {
    box.replaceChildren(
      h('div', { class: 'manage-row' },
        h('p', {
          class: 'manage-text detail-note' + (d.note ? '' : ' note-empty'),
          text: d.note || T('メモはまだありません。用途や、サインインに使うアカウントを書いておけます。',
            'No note yet. You can write down what this environment is for and which account signs in.'),
        }),
        h('button', { type: 'button', class: 'btn btn-ghost manage-action', onclick: renderEdit },
          d.note ? '編集' : 'メモを書く')),
      factsRow);
  };

  const renderEdit = () => {
    const save = async (val) => {
      try {
        await invoke('set_profile_note', { name, note: val });
        d.note = val;
        await refreshProfiles();
        // Rebuild the whole detail view, not just this card: the sign-in
        // signpost quotes the note and must follow the edit immediately.
        await showDetail(name);
        showToast(T('メモを保存しました', 'Saved the note'));
      } catch (err) {
        showToast(T('メモを保存できませんでした。', 'Could not save the note.'), true);
      }
    };
    const input = h('input', {
      type: 'text', class: 'input', value: d.note || '', maxlength: '200',
      placeholder: '例：仕事用。会社のGoogleアカウントでサインイン',
      autocomplete: 'off', spellcheck: 'false', style: 'flex:1',
      // IME guard: Enter while composing Japanese must confirm the conversion,
      // not submit (e.isComposing, plus keyCode 229 for older WebKit).
      onkeydown: (e) => {
        if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) save(input.value.trim());
      },
    });
    box.replaceChildren(
      h('div', { class: 'manage-row note-edit-row' },
        input,
        h('button', { type: 'button', class: 'btn btn-ghost', onclick: renderRead }, 'やめる'),
        h('button', { type: 'button', class: 'btn btn-primary', onclick: () => save(input.value.trim()) }, '保存')),
      factsRow);
    input.focus();
  };

  renderRead();
  return section('この環境のメモ', [box]);
}

function renderDataMap(map) {
  const wrap = h('div', { class: 'datamap-report' });
  wrap.appendChild(h('p', {
    class: 'datamap-total',
    text: T(
      `この環境が専有している容量は${fmtBytes(map.total_size_bytes)}です。内訳はClaudeデスクトップアプリが${fmtBytes(map.desktop_size_bytes)}、Claude Codeが${fmtBytes(map.cli_size_bytes)}で、共有リンクの先の実体は含みません。`,
      `This environment occupies ${fmtBytes(map.total_size_bytes)} by itself: ${fmtBytes(map.desktop_size_bytes)} for the Claude Desktop App and ${fmtBytes(map.cli_size_bytes)} for Claude Code. Shared originals are not included.`
    ),
  }));
  const list = h('div', { class: 'datamap-list' });
  for (const item of map.items) {
    const label = DOCTOR_LABELS[item.key] || item.key;
    let state;
    let detail = null;
    if (item.key === 'desktop_app_config') {
      state = '常に分離';
    } else if (item.mode === 'share' && item.link_target) {
      state = '既存のClaudeと共有';
      detail = item.link_target;
    } else if (item.link_target) {
      // A non-share item drifted into a link: state the fact with the same
      // wording the isolation check uses, and show where it points.
      state = ALWAYS_ISOLATED_KEYS.includes(item.key)
        ? '常に分離する項目がリンクになっています。'
        : (item.mode === 'copy' ? 'コピーの項目がリンクになっています。' : '分離の項目がリンクになっています。');
      detail = item.link_target;
    } else if (!item.exists) {
      state = 'まだありません';
    } else {
      const size = fmtBytes(item.size_bytes || 0);
      state = item.modified_epoch
        ? `${size}${T(' ・ 最終更新', ' · updated ')}${fmtEpoch(item.modified_epoch)}`
        : size;
    }
    list.appendChild(h('div', { class: 'datamap-item' },
      h('span', { class: 'datamap-item-label', text: label }),
      h('span', { class: 'datamap-item-state', text: state })));
    if (detail) {
      list.appendChild(h('p', { class: 'datamap-item-detail' },
        h('code', { class: 'path-code', text: detail })));
    }
  }
  wrap.appendChild(list);
  return wrap;
}

function pathRow(label, value, revealName, revealWhich) {
  const reveal = revealName ? h('button', {
    type: 'button', class: 'icon-btn', title: 'Finderで表示',
    onclick: () => invoke('reveal_profile_dir', { name: revealName, which: revealWhich })
      .catch(() => showToast('Finderで表示できませんでした。', true)),
  }, icon('i-folder')) : null;
  return h('div', { class: 'path-row' },
    h('div', { class: 'path-meta' },
      h('span', { class: 'path-label', text: label }),
      h('code', { class: 'path-code', text: value })),
    reveal,
    copyButton(value, 'パスをコピー'));
}

function copyButton(value, title) {
  return h('button', {
    type: 'button', class: 'icon-btn', title,
    onclick: () => copyText(value),
  }, icon('i-copy'));
}

// Where to get csw. The .dmg ships the menu-bar app only; csw is a separate
// release asset, so every place that shows a csw command (the first-run card
// in index.html, the terminal section, the isolation check) says so and offers
// the releases page. Same markup as the static first-run copy.
function cswGetRow() {
  return h('div', { class: 'csw-get' },
    h('p', { class: 'firstrun-body', text: 'cswコマンドはアプリの.dmgに含まれておらず、アプリを更新しても更新されません。GitHubのリリースページから最新のcswを入手できます。' }),
    h('button', { type: 'button', class: 'btn btn-ghost', onclick: () => openExternal(RELEASES_URL) },
      'リリースページを開く'));
}

function terminalSection(name) {
  const cmd = `eval $(csw env ${name})`;
  // Collapsed by default: most users drive everything from the GUI; the CLI
  // command is only needed when opening a separate terminal yourself.
  const inner = [
    h('p', { class: 'share-basis', text: T(
      'CSWから開いたClaudeデスクトップアプリの中のターミナルは、最初からこの環境です。コマンドは要りません。自分で別に開いたiTerm2などのターミナルでこの環境に揃えるには、次を実行します。この指定はそのタブだけに効き、普段の環境には影響しません。',
      'A terminal inside the Claude Desktop App you opened from CSW is already in this environment, so no command is needed. To match a terminal you opened yourself, such as iTerm2, run the command below. It applies to that tab only and never affects your usual environment.') }),
    h('div', { class: 'path-row' },
      h('div', { class: 'path-meta' }, h('code', { class: 'path-code', text: cmd })),
      copyButton(cmd, 'コマンドをコピー')),
    cswGetRow(),
  ];
  return section('', [disclosure('ターミナルでClaude Codeを使う', null, inner)]);
}

// Generic collapsible disclosure: a summary row stays visible, the panel opens
// on click. Keeps secondary detail (paths, the terminal command, the full
// sharing breakdown) off the always-on surface so the detail view shows state +
// actions first and discloses the rest only when asked.
function disclosure(label, sub, innerNodes, onOpen) {
  const wrap = h('div', { class: 'disclosure' });
  const innerWrap = h('div', { class: 'disclosure-inner' }, ...innerNodes);
  innerWrap.inert = true; // keep collapsed content out of the tab order
  const toggle = h('button', {
    type: 'button', class: 'sharing-summary', 'aria-expanded': 'false',
    onclick: () => {
      const open = wrap.classList.toggle('open');
      toggle.setAttribute('aria-expanded', String(open));
      innerWrap.inert = !open;
      if (open && onOpen) onOpen(); // callbacks self-guard (e.g. load once, retry on failure)
      requestAnimationFrame(refreshFades);
    },
  },
    h('span', { class: 'summary-text' }, label,
      sub ? h('span', { class: 'summary-sub', text: sub }) : null),
    icon('i-chevron', 'icon summary-chevron'));
  appendKids(wrap, [toggle, h('div', { class: 'disclosure-panel' }, innerWrap)]);
  return wrap;
}

function sharingDisclosure(sharing, cloned, nativeCodeShared = false) {
  // The linker copies this file at creation, but the enabled native Code group
  // then synchronizes it at every switch. Show the resulting sharing behavior.
  if (nativeCodeShared) sharing = { ...sharing, desktop_worktrees: 'share' };
  const shareCount = ALL_KEYS.filter((k) => sharing[k] === 'share').length;
  const copyCount = ALL_KEYS.filter((k) => sharing[k] === 'copy').length;
  const isoCount = ALL_KEYS.length - shareCount - copyCount;
  const summary = copyCount
    ? T(`共有${shareCount}・コピー${copyCount}・分離${isoCount}件`, `${shareCount} shared · ${copyCount} copied · ${isoCount} isolated`)
    : T(`共有${shareCount}・分離${isoCount}件`, `${shareCount} shared · ${isoCount} isolated`);

  const inner = [
    // Name the reference point so "shared/copied/isolated" is never ambiguous.
    h('p', { class: 'share-basis', text: '「共有」は既存のClaudeと中身を共通にすること、「コピー」は作成時に一度だけ写して以後は別々、「分離」はこの環境だけで持つことです。' }),
  ];
  if (nativeCodeShared) inner.push(h('p', { class: 'share-basis', text: T(
    'ワークツリー一覧は、ローカルCode全体の共有により切り替え時に同期します。',
    'The repo/worktree list is synchronized at each switch by complete local Code sharing.') }));
  for (const g of SHARE_GROUPS) {
    inner.push(h('div', { class: 'share-group' },
      h('div', { class: 'share-group-label', text: g.label }),
      ...g.items.map((it) => sharingReadRow(it, sharing[it.key]))));
  }
  inner.push(h('div', { class: 'share-group' }, sharingReadRow(DEVICE_ID, sharing[DEVICE_ID.key])));

  // A duplicate got the Claude Desktop App sign-in (desktop-data, Cookies
  // included) from its source, so "account always separate" would be untrue
  // there. The label states where that sign-in came from, which stays true
  // even after either side signs out or in again; the terminal Claude Code
  // sign-in is per environment in the Keychain and is not part of it.
  const sub = cloned
    ? T(' ／ デスクトップのサインインは複製元から写したもの', ' · Desktop sign-in duplicated from the original')
    : T(' ／ アカウントは常に分離', ' · Account always separate');
  return section('この環境が引き継いでいるもの', [disclosure(summary, sub, inner)]);
}

function sharingReadRow(item, mode) {
  return h('div', { class: 'share-line' },
    h('div', { class: 'share-line-meta' },
      h('div', { class: 'share-line-name', text: item.name }),
      h('div', { class: 'share-line-desc', text: item.desc })),
    badge(mode));
}

function badge(mode) {
  if (mode === 'share') return h('span', { class: 'badge badge-share' }, icon('i-link'), '共有');
  if (mode === 'copy') return h('span', { class: 'badge badge-copy' }, icon('i-copy'), 'コピー');
  return h('span', { class: 'badge badge-isolate' }, icon('i-lock'), '分離');
}

function renderDetailFooter(name, isDefault, isInUse, supportsConcurrent) {
  el.detailFooter.className = 'view-footer split';
  // The primary button's role depends on whether this environment's Claude is
  // running. While it runs, it brings that Claude's window to the front (the way
  // to tell side-by-side instances apart, since the Dock shows one shared icon).
  // While it is not running, it launches: the existing Claude opens the standard
  // data dir ("既存の Claude を起動"), other non-default environments launch.
  // The "利用中" state is carried by the pill, so the button stays an action.
  // Text-only: a switch/arrow glyph on the label only added noise; launchHelp()
  // in the body above spells out the meaning of each short label.
  const switchBtn = h('button', {
    type: 'button', class: 'btn btn-primary',
    onclick: () => isInUse ? doBringToFront(name) : doSwitch(name, supportsConcurrent),
  }, h('span', { text: isInUse ? '前面に表示' : (isDefault ? '既存のClaudeを起動' : 'この環境を起動') }));

  if (isDefault) {
    el.detailFooter.replaceChildren(
      h('span', { class: 'confirm-text', style: 'color:var(--ink-muted)', text: '既存のClaudeは変更・削除できません' }),
      switchBtn);
    return;
  }
  // Footer holds the launch actions. For fully-isolated environments the dedicated
  // "重複して起動" sits right next to the primary launch (a frequent action for these);
  // clone/delete are management actions kept out of this row.
  const launchGroup = h('div', { class: 'footer-group' }, switchBtn);
  if (supportsConcurrent) {
    launchGroup.appendChild(h('button', {
      type: 'button', class: 'btn btn-ghost', onclick: () => doLaunchNewWindow(name),
    }, h('span', { text: '重複して起動' })));
  }
  el.detailFooter.replaceChildren(
    launchGroup,
    h('button', { type: 'button', class: 'btn btn-danger', onclick: () => showDeleteRow(name) },
      icon('i-trash'), h('span', { text: '削除' })));
}

// --- Inline confirm / clone rows (no window.confirm) ------------------------
// Deleting moves the whole folder to the Trash (restorable, links move as
// links). The copy states that honestly, including that the sign-in state
// stays restorable until the Trash is emptied; users who want the data gone
// now get an explicit, separately confirmed "完全に削除".
function showDeleteRow(name) {
  el.detailFooter.className = 'view-footer confirm-stack';
  const cancel = h('button', { type: 'button', class: 'btn btn-ghost', onclick: () => showDetail(name) }, 'やめる');
  const text = h('span', {
    class: 'confirm-text',
    text: T(
      'この環境をゴミ箱へ移動します。ゴミ箱を空にするまでは、サインイン状態を含むすべてのデータを戻せます。既存のClaudeには影響しません。',
      'This environment will be moved to the Trash. Until you empty the Trash, everything including the sign-in state can be restored. Your existing Claude is not affected.'
    ),
  });
  el.detailFooter.replaceChildren(
    text,
    h('div', { class: 'footer-group' },
      cancel,
      h('button', { type: 'button', class: 'btn btn-danger', onclick: () => showPurgeRow(name) }, '完全に削除'),
      h('button', { type: 'button', class: 'btn btn-danger-solid', onclick: () => doDelete(name) }, 'ゴミ箱へ移動')));
  cancel.focus(); // default focus on the safe action
  // The occupied size arrives whenever the read-only aggregation resolves; the
  // confirmation never waits for it. Built with T(), so the observer-based
  // dictionary is not involved and the sentence stays language-correct.
  invoke('get_profile_data_map', { name }).then((map) => {
    text.textContent += T(
      `この環境の専有容量は${fmtBytes(map.total_size_bytes)}です。`,
      ` This environment occupies ${fmtBytes(map.total_size_bytes)} by itself.`
    );
  }).catch(() => { /* size stays out of the copy */ });
}

// Second, separate confirmation for the non-restorable path.
function showPurgeRow(name) {
  el.detailFooter.className = 'view-footer split';
  const cancel = h('button', { type: 'button', class: 'btn btn-ghost', onclick: () => showDetail(name) }, 'やめる');
  el.detailFooter.replaceChildren(
    h('span', { class: 'confirm-text', text: '完全に削除すると、ゴミ箱を経由せずにすぐ消え、戻せません。' }),
    h('div', { class: 'footer-group' },
      cancel,
      h('button', { type: 'button', class: 'btn btn-danger-solid', onclick: () => doPurge(name) }, '完全に削除する')));
  cancel.focus();
}

function showCloneRow(name) {
  el.detailFooter.className = 'view-footer';
  const input = h('input', {
    type: 'text', class: 'input', placeholder: '複製先の名前。例：仕事用-控え',
    autocomplete: 'off', spellcheck: 'false', style: 'flex:1',
    onkeydown: (e) => { if (e.key === 'Enter') doClone(name, input.value.trim()); },
  });
  el.detailFooter.replaceChildren(
    input,
    h('button', { type: 'button', class: 'btn btn-ghost', onclick: () => showDetail(name) }, 'やめる'),
    h('button', { type: 'button', class: 'btn btn-primary', onclick: () => doClone(name, input.value.trim()) }, '複製を作る'));
  input.focus();
}

// --- Backend operations -----------------------------------------------------
// The backend owns the one volatile request. Rendering never decides whether
// Claude is idle or authorizes a launch; it only shows the process-exit wait.
function launchError(code) {
  if (code === 'launch_busy') return T('ほかの起動操作が進行中です。完了してから操作してください。', 'Another launch action is in progress. Wait for it to finish.');
  if (code === 'launch_pending') return T('現在の起動予約を確認してください。終了待ちの間はキャンセルできます。', 'Check the current pending launch. You can cancel while it is waiting for you to quit Claude.');
  if (code === 'launch_committed') return T('起動処理が始まったため、キャンセルできません。', 'Launching has started and can no longer be canceled.');
  if (code === 'launch_unconfirmed') return T('Claudeの起動を確認できません。Claudeの状態を確認し、すべて終了してから「終了後の起動予約を解除」を選んでください。起動要求は再送していません。', 'Claude startup could not be confirmed. Check Claude, quit every instance, then choose Recover closed launch. No retry was sent.');
  if (code === 'history_conflict') return T('別の環境のCode履歴にも変更があります。作業を終えてClaudeを終了し、利用ガイドで元環境を選び直す回復手順を確認してください。未確認の起動予約が残る場合は、再設定の前に共有を解除する必要があります。', 'Code history has conflicting changes in another environment. Finish all work, quit Claude, then follow the User guide to choose a source. If an unconfirmed launch remains, disable sharing before reconfiguring it.');
  if (code === 'history_recovery_needed') return T('中断したCode履歴の引き継ぎ後に別の変更があり、復元を停止しています。バックアップと処理記録を確認してから復元してください。', 'Code history changed after an interrupted handoff. Recovery is paused. Inspect the backups and transaction journal before repairing it.');
  if (code === 'history_unavailable') return T('共有するCode履歴の検証または復元が必要なため、起動していません。共有設定とバックアップを確認してください。', 'Shared Code history needs validation or recovery. Check the sharing setup and backups before launching.');
  if (code === 'history_launch_pending') return T('前回の起動を確認できず、Code履歴の同期を停止しています。Claudeを終了し、「終了後の起動予約を解除」を選んでください。履歴をバックアップして予約だけを解除します。', 'A previous launch is unconfirmed, so Code history synchronization is blocked. Quit Claude, then choose Recover closed launch. This backs up history and clears only the saved request.');
  if (code === 'launch_failed') return T('Claudeの起動要求に失敗しました。Claudeの状態を確認し、終了してから「終了後の起動予約を解除」を選んでください。', 'The Claude launch request failed. Check Claude, then quit it before using Recover closed launch.');
  if (code === 'desktop_reappeared') return T('Claudeが再び起動したため、追加の起動要求を送りませんでした。Claudeを終了してから操作してください。', 'Claude started again before launch. Quit Claude before trying again.');
  if (code === 'launch_process_unavailable') return T('Claudeへ起動を要求しましたが、プロセス状態を確認できません。Claudeの状態を確認し、終了してから「終了後の起動予約を解除」を選んでください。', 'Claude was asked to launch, but its process state could not be verified. Check Claude, quit it, then choose Recover closed launch.');
  if (code === 'process_unavailable') return T('Claudeの起動状態を確認できません。追加の起動要求は送りません。', 'Could not verify Claude’s process state. No further launch request will be sent.');
  if (code === 'target_changed' || code === 'target_unavailable') return T('予約した環境が変更または削除されたため、起動していません。環境を確認してから予約し直してください。', 'The requested environment changed or was removed. Check it before requesting another launch.');
  return T('環境を起動できませんでした。Claudeと環境の状態を確認してください。', 'Could not launch the environment. Check Claude and the environment.');
}

function reopenFailureMessage(result) {
  const errors = [...new Set(Object.values(result.errors || {}))];
  const detail = errors.filter(code => code !== 'desktop_reappeared');
  if (detail.length) return detail.map(launchError).join(' ');
  return launchError('desktop_reappeared');
}

function renderPendingLaunch() {
  const status = pendingLaunch;
  const visible = ['waiting', 'switching', 'failed'].includes(status.phase);
  el.pendingLaunchBanner.hidden = !visible;
  if (!visible) return;
  const name = (status.targets || [status.target]).map(n => n === 'default' ? T('既存のClaude', 'Existing Claude') : n).join(T('、', ', '));
  el.pendingLaunchTarget.textContent = T(`起動先：${name}`, `Launch target: ${name}`);
  el.pendingLaunchText.textContent = status.phase === 'waiting' && status.automatic_return
    ? T('Claudeの更新後に元の環境へ戻します。開き直されたClaudeの終了を待っています。起動予約はキャンセルできます。', 'Returning to your environment after the Claude update. Waiting for the restarted Claude to exit. You can cancel this launch.')
    : status.phase === 'waiting' && status.error === 'process_unavailable'
    ? T('Claudeの起動状態を再確認しています。確認できるまで起動せず、予約はキャンセルできます。CSWはClaudeを終了しません。', 'Rechecking Claude’s process state. No launch will begin until it can be verified. You can cancel this reservation. CSW does not quit Claude.')
    : status.error ? launchError(status.error)
    : status.phase === 'waiting' && status.wait_for_exit
      ? T('作業と質問への回答を終えて、ClaudeをCmd+Qで終了してください。すべてのClaudeが終了した後にこの環境を開きます。CSWはClaudeを終了しません。', 'Finish your work and answer pending questions, then quit Claude with Cmd+Q. This environment opens after every Claude instance has exited. CSW does not quit Claude.')
      : T('Claudeの起動を確認しています。起動要求は再送しません。', 'Confirming Claude startup. The launch request will not be repeated.');
  el.btnCancelPendingLaunch.hidden = status.phase !== 'waiting';
  el.btnRecoverPendingLaunch.hidden = status.phase !== 'failed' || !['history_launch_pending', 'launch_unconfirmed', 'launch_failed', 'launch_process_unavailable'].includes(status.error);
}

function acceptPendingLaunch(status) {
  if (!status || status.request_id < pendingLaunch.request_id) return;
  // A command response can arrive after its matching event. Terminal states
  // cannot be rolled back to an earlier Waiting/Switching response.
  if (status.request_id === pendingLaunch.request_id) {
    const rank = { idle: 0, waiting: 1, switching: 2, failed: 3, completed: 4, canceled: 4 };
    if (rank[status.phase] < rank[pendingLaunch.phase]) return;
  }
  const before = pendingLaunch;
  pendingLaunch = status;
  renderPendingLaunch();
  if (before.request_id === status.request_id && before.phase === status.phase) return;
  if (status.phase === 'completed') {
    showToast(T('選んだ環境のClaudeを開きました。', 'Opened Claude for the selected environment.'));
    refreshProfiles();
  } else if (status.phase === 'canceled') {
    showToast(T('起動予約をキャンセルしました。', 'Pending launch canceled.'));
  }
}

async function refreshPendingLaunch() {
  if (pendingRead) return;
  pendingRead = true;
  try { acceptPendingLaunch(await invoke('get_pending_launch')); }
  catch (e) { /* Preserve the last known status; never infer that it is idle. */ }
  finally { pendingRead = false; }
}

async function doCancelPendingLaunch() {
  const requestId = pendingLaunch.request_id;
  try { acceptPendingLaunch(await invoke('cancel_pending_launch', { requestId })); }
  catch (err) { showToast(launchError(String(err)), true); await refreshPendingLaunch(); }
}

let recoveryInProgress = false;
async function doRecoverPendingLaunch() {
  if (recoveryInProgress) return;
  recoveryInProgress = true;
  el.btnRecoverPendingLaunch.disabled = true;
  const requestId = pendingLaunch.request_id;
  try {
    acceptPendingLaunch(await invoke('recover_pending_code_launch', { requestId, abandon: true }));
    showToast(T('起動予約を解除しました。履歴と共有設定は維持しています。使いたい環境を選んで起動してください。', 'Saved launch cleared. History and sharing preserved. Select the environment you want to launch.'));
    await refreshProfiles();
  } catch (err) {
    showToast(launchError(String(err)), true);
    await refreshPendingLaunch();
  } finally {
    recoveryInProgress = false;
    el.btnRecoverPendingLaunch.disabled = false;
  }
}

async function doSwitch(name) {
  try { acceptPendingLaunch(await invoke('request_environment_launch', { name })); }
  catch (err) { showToast(launchError(String(err)), true); await refreshPendingLaunch(); }
}

// Open a fully-isolated environment in an additional window, alongside whatever is
// already running. Does not switch the active environment or require quitting.
async function doLaunchNewWindow(name) {
  try {
    acceptPendingLaunch(await invoke('launch_additional_window', { name }));
  } catch (err) {
    showToast(launchError(String(err)), true); await refreshPendingLaunch();
  }
}

// Bring the running Claude for this environment to the front. The button only
// appears while the environment is in use; if its Claude was quit in the
// meantime the backend returns false, so refresh to return the button to a
// launch action instead of leaving a dead "前面に表示".
async function doBringToFront(name) {
  try {
    const brought = await invoke('activate_environment', { name });
    if (brought) {
      showToast(T('前面に表示しました', 'Brought to front'));
      return;
    }
    await refreshProfiles();
    withTransition(() => showDetail(name));
    showToast(T('この環境のClaudeは動いていないようです。一覧を更新しました。',
      'That environment’s Claude does not seem to be running. Refreshed the list.'), true);
  } catch (err) {
    showToast(T('前面に表示できませんでした。もう一度お試しください。',
      'Could not bring it to front. Please try again.'), true);
  }
}

async function doDelete(name) {
  try {
    await invoke('delete_profile', { name });
    selectedName = null;
    await refreshProfiles();
    const remaining = profiles.filter((p) => p.name !== 'default');
    withTransition(() => (remaining.length ? showDetail(remaining[0].name) : showEmpty()));
    showToast(T(`「${name}」をゴミ箱へ移動しました`, `Moved "${name}" to the Trash`));
  } catch (err) {
    // The backend never falls back to permanent deletion. When the Trash move
    // itself failed (CswError::TrashMoveFailed), say so and point at the
    // explicit permanent path; otherwise the usual cause is an in-use profile.
    if (String(err).includes('could not move to the Trash')) {
      showToast('ゴミ箱へ移動できませんでした。すぐに消す場合は「完全に削除」を選んでください。', true);
    } else {
      showToast('削除できませんでした。利用中の環境は、起動中のClaudeを終了してから削除してください。', true);
    }
  }
}

async function doPurge(name) {
  try {
    await invoke('purge_profile', { name });
    selectedName = null;
    await refreshProfiles();
    const remaining = profiles.filter((p) => p.name !== 'default');
    withTransition(() => (remaining.length ? showDetail(remaining[0].name) : showEmpty()));
    showToast(T(`「${name}」を完全に削除しました`, `Permanently deleted "${name}"`));
  } catch (err) {
    showToast('削除できませんでした。利用中の環境は、起動中のClaudeを終了してから削除してください。', true);
  }
}

async function doClone(source, target) {
  const err = validateName(target);
  if (err) { showToast(err, true); return; }
  try {
    await invoke('clone_profile', { source, target });
    selectedName = target;
    await refreshProfiles();
    withTransition(() => showDetail(target));
    // Name both sides: which environment was duplicated, under which new name.
    showToast(T(`「${source}」を「${target}」として複製しました。`, `Duplicated "${source}" as "${target}".`));
  } catch (e) {
    showToast(String(e), true);
  }
}

// --- Empty / create flow ----------------------------------------------------
let createIcon = ''; // selected avatar: an AVATAR_ICONS slug, an emoji/char, or '' (none)

// Render the icon picker grid (designed glyphs) for the create view.
function renderIconPicker() {
  el.iconPicker.replaceChildren(...AVATAR_ICONS.map((it) =>
    h('button', {
      type: 'button', class: 'icon-tile', role: 'radio', 'data-icon': it.slug,
      'aria-label': it.label, 'aria-checked': 'false', title: it.label,
      onclick: () => selectIcon(it.slug),
    }, icon('i-' + it.slug))));
  syncIconPicker();
}
function selectIcon(value) {
  createIcon = createIcon === value ? '' : value; // click again to clear
  syncIconPicker();
}
function syncIconPicker() {
  document.querySelectorAll('.icon-tile').forEach((b) => {
    const on = b.dataset.icon === createIcon && createIcon !== '';
    b.classList.toggle('selected', on);
    b.setAttribute('aria-checked', on ? 'true' : 'false');
  });
}

function showEmpty() {
  selectedName = null;
  renderSidebar();
  setView('empty');
}

function showCreate() {
  localStorage.setItem('csw_onboarded', '1');
  el.inputName.value = '';
  el.inputNote.value = '';
  createIcon = '';
  renderIconPicker();
  el.nameError.hidden = true;
  setMode('share_code');
  closeAdvanced();
  renderCreateFooter();
  setView('create');
  setTimeout(() => el.inputName.focus(), 0);
}

// --- Create-view footer: deliberate two-step (review → confirm) -------------
// A stray Enter (e.g. confirming a Japanese IME kanji conversion) must never
// create an environment, so creating always goes through an explicit confirm step.
function cancelCreate() {
  withTransition(() => (selectedName ? showDetail(selectedName) : showEmpty()));
}
function renderCreateFooter() {
  el.createFooter.className = 'view-footer';
  el.createFooter.replaceChildren(
    h('button', { type: 'button', class: 'btn btn-ghost', onclick: cancelCreate }, 'やめる'),
    h('button', { type: 'button', class: 'btn btn-primary', onclick: requestCreate }, 'この環境を作る'));
}
// Validate the name, then ask for an explicit confirmation before creating.
function requestCreate() {
  const name = el.inputName.value.trim();
  const err = validateName(name);
  if (err) { el.nameError.textContent = err; el.nameError.hidden = false; el.inputName.focus(); return; }
  el.nameError.hidden = true;
  el.createFooter.className = 'view-footer split';
  el.createFooter.replaceChildren(
    h('span', { class: 'confirm-text', text: T(`「${name}」を作成します。`, `Create "${name}".`) }),
    h('div', { class: 'footer-group' },
      h('button', { type: 'button', class: 'btn btn-ghost', onclick: renderCreateFooter }, '戻る'),
      h('button', { type: 'button', class: 'btn btn-primary', onclick: submitCreate }, '作成する')));
}

// Gate the "share" mode by what the existing Claude actually has at the standard
// locations. Sharing symlinks from the default dirs, so a root that is missing has
// nothing to share: both missing → only "すべて分ける"; one missing → that side
// won't carry over, so warn but keep sharing available for the present side.
async function applyRootsStatus() {
  let st = { desktop_present: true, cli_present: true };
  try { st = await invoke('get_default_roots_status'); } catch (e) { /* keep optimistic default */ }
  // Both share modes carry over from the existing Claude, so both need its data.
  const shareModes = ['share_settings', 'share_workspace'];
  const bothAbsent = !st.desktop_present && !st.cli_present;
  for (const mode of shareModes) {
    const input = document.querySelector(`input[name="mode"][value="${mode}"]`);
    if (input) input.disabled = bothAbsent;
    const card = input ? input.closest('.mode-card') : null;
    if (card) card.classList.toggle('is-disabled', bothAbsent);
  }
  // The Code default can initialize its empty local content stores without a
  // CLI login. Existing environments and their settings are never migrated.

  let msg = '';
  if (currentMode === 'share_code') {
    msg = T('初回サインイン後、両方の環境でCodeを開いてClaudeを手動で終了し、詳細画面の「Codeの共有を設定」でバックアップして有効にしてください。クラウドChatの会話は共有されません。',
      'After the first sign-in, open Code in both environments and quit Claude manually. Use "Set up shared Code" in the environment details to back up and enable it. Cloud Chat conversations are not shared.');
  } else if (bothAbsent) {
    msg = '既存のClaudeが標準の場所に見つかりません。引き継げるものが無いため、「すべて分ける」だけで作成できます。';
  } else if (!st.cli_present) {
    msg = 'Claude Code（CLI）の設定が標準の場所に見つかりません。引き継ぐモードを選んでも、ルールやスキル、会話などのCLI側は引き継がれません。';
  }
  el.createNotice.textContent = msg;
  el.createNotice.hidden = !msg;
}

function setMode(mode) {
  currentMode = mode;
  const radio = document.querySelector(`input[name="mode"][value="${mode}"]`);
  if (radio) radio.checked = true;
  overrides = { ...PRESETS[mode] };
  advancedCustomized = false;
  renderAdvancedRows();
  updateAdvancedState();
  applyRootsStatus();
}

function renderAdvancedRows() {
  // Name the reference point + define the words once, so every row reads clearly.
  const nodes = [
    h('p', { class: 'share-basis', text: '各項目を、既存のClaudeと共有するか、この環境だけにするかを選びます。' }),
    h('p', { class: 'share-legend', text: '共有 = いまのを使う ／ 分離 = この環境だけ ／ コピー = 最初だけ写す' }),
    h('p', { class: 'path-caption', text: 'アカウントのサインイン情報（config.json）と、コネクタ・アプリ設定（claude_desktop_config.json）は、アカウント別の情報を含むため、どのモードでも必ずこの環境だけに分離します。' }),
  ];
  for (const g of SHARE_GROUPS) {
    nodes.push(h('div', { class: 'share-group' },
      h('div', { class: 'share-group-label', text: g.label }),
      ...g.items.map((it) => segRow(it, overrides[it.key], false))));
  }
  nodes.push(h('div', { class: 'share-group' },
    segRow(DEVICE_ID, 'isolate', true),
    h('p', { class: 'path-caption', text: '2つのアカウントが同じ端末として結び付かないよう、端末IDは常に分離します。' })));
  el.advancedGroups.replaceChildren(...nodes);
}

function segRow(item, value, fixed) {
  const opt = (val, iconId, label) => {
    const input = h('input', { type: 'radio', name: 'seg-' + item.key, value: val });
    if (value === val) input.checked = true;
    if (fixed) input.disabled = true;
    return h('label', { class: 'seg-opt' }, input, icon(iconId), label);
  };
  return h('div', { class: 'share-line' + (fixed ? ' fixed' : '') },
    h('div', { class: 'share-line-meta' },
      h('div', { class: 'share-line-name', text: item.name }),
      h('div', { class: 'share-line-desc', text: item.desc })),
    // Composed at construction with T(): a runtime-concatenated label can never
    // match the exact-text EN dictionary, so both languages are built here.
    h('div', { class: 'seg', role: 'radiogroup', 'aria-label': T(
      item.name + '：既存のClaudeと共有・分離・コピーのどれにするか',
      (setEN(item.name) || item.name) + ': whether to share with, isolate from, or copy from your existing Claude'
    ) },
      opt('share', 'i-link', '共有'), opt('isolate', 'i-lock', '分離'), opt('copy', 'i-copy', 'コピー')));
}

function updateAdvancedState() {
  el.advancedState.textContent = advancedCustomized ? 'カスタム設定' : 'モードの既定どおり';
  el.advancedState.classList.toggle('custom', advancedCustomized);
  el.advancedReset.hidden = !advancedCustomized;
}

function openAdvanced() {
  const adv = el.advancedToggle.closest('.advanced');
  adv.classList.add('open');
  el.advancedToggle.setAttribute('aria-expanded', 'true');
  adv.querySelector('.advanced-inner').inert = false;
  requestAnimationFrame(refreshFades);
}
function closeAdvanced() {
  const adv = el.advancedToggle.closest('.advanced');
  adv.classList.remove('open');
  el.advancedToggle.setAttribute('aria-expanded', 'false');
  adv.querySelector('.advanced-inner').inert = true; // keep collapsed rows out of the tab order
}

function validateName(name) {
  if (!name) return '名前を入力してください。';
  if (name.toLowerCase() === 'default') return '"default"は使えません。いまの環境を指す予約名です。';
  if ([...name].length > 64) return '名前は64文字までにしてください。';
  // Allow Unicode letters/numbers (including Japanese) plus hyphen/underscore.
  // Spaces, slashes, dots and symbols are rejected (the name becomes a folder
  // name and is passed to the shell in `csw env <name>`). Mirrors the core
  // validate_profile_name guard.
  if (!/^[\p{L}\p{N}_-]+$/u.test(name)) return '使えるのは文字・数字・ハイフン・アンダースコアだけです。空白や記号は使えません。';
  return null;
}

async function submitCreate() {
  const name = el.inputName.value.trim();
  const iconVal = createIcon;
  const err = validateName(name);
  if (err) { el.nameError.textContent = err; el.nameError.hidden = false; el.inputName.focus(); return; }
  el.nameError.hidden = true;

  const args = { name, mode: currentMode, icon: iconVal || null, note: el.inputNote.value.trim() || null };
  if (advancedCustomized) args.sharingOverrides = { ...overrides };

  const confirmBtn = el.createFooter.querySelector('.btn-primary');
  if (confirmBtn) confirmBtn.disabled = true;
  try {
    await invoke('create_profile', args);
    localStorage.setItem('csw_onboarded', '1');
    selectedName = name;
    await refreshProfiles();
    withTransition(() => showDetail(name));
    showToast(T(`「${name}」を作成しました`, `Created "${name}"`));
  } catch (e) {
    showToast(String(e), true);
    renderCreateFooter(); // restore the form footer so the user can fix and retry
  }
}

// --- Scroll affordance ------------------------------------------------------
function fadeFor(scrollEl, fadeEl) {
  if (!scrollEl || !fadeEl) return;
  fadeEl.hidden = !(scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight > 4);
}
function refreshFades() {
  fadeFor(el.emptyScroll, el.emptyFade);
  fadeFor(el.detailScroll, el.detailFade);
  fadeFor(el.createScroll, el.createFade);
  fadeFor(el.usageScroll, el.usageFade);
}

// --- Clipboard --------------------------------------------------------------
async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    showToast('コピーしました');
  } catch (e) {
    showToast('コピーできませんでした。', true);
  }
}

// --- Data refresh -----------------------------------------------------------
async function refreshProfiles() {
  try {
    profiles = await invoke('list_profiles');
    activeName = await invoke('get_active_profile');
  } catch (e) {
    profiles = [{ name: 'default', icon: '', is_default: true }];
    activeName = 'default';
  }
  await refreshRunning();
  renderSidebar();
}

// Re-read which environments are running (per environment) and whether any Claude
// is running at all. Kept separate so it can refresh on window focus without
// re-listing profiles, since the user typically quits/starts Claude in another app
// and returns to CSW expecting the markers and actions to reflect reality.
async function refreshRunning() {
  try { runningProfiles = await invoke('get_running_profiles'); }
  catch (e) { runningProfiles = []; }
  try { desktopRunning = await invoke('get_desktop_running_status'); }
  catch (e) { desktopRunning = false; }
  try { takeoverFrom = await invoke('get_takeover_notice'); }
  catch (e) { takeoverFrom = null; }
  try { reopenOffer = await invoke('get_reopen_offer'); }
  catch (e) { reopenOffer = []; }
  renderTakeover();
  renderReopen();
  await refreshPendingLaunch();
}

// The reopen-previous-set offer. The watcher records which environments were
// open together before the last all-quit; this banner offers to reopen the ones
// not already running. The names are user data, built with T(). Suppressed
// while the takeover banner is up so only one banner shows at a time.
let reopenOffer = [];
function renderReopen() {
  const takeoverShowing = takeoverFrom && profiles.some((p) => p.name === takeoverFrom);
  if (!reopenOffer.length || takeoverShowing) {
    el.reopenBanner.hidden = true;
    return;
  }
  const disp = (n) => (n === 'default' ? T('既存のClaude', 'Existing Claude') : n);
  const listJa = reopenOffer.map((n) => `「${disp(n)}」`).join('');
  const listEn = reopenOffer.map((n) => `"${disp(n)}"`).join(', ');
  el.reopenText.textContent = T(
    `前回開いていた${listJa}を開き直せます。更新などで全部閉じたときに使えます。`,
    `You can reopen ${listEn}, which you had open last time. Use it after you quit everything for an update.`);
  el.reopenBanner.hidden = false;
}

// Answer to the tray's "いまの環境を確認". Payload kinds: "claude" with the
// resolved environment (or null for a data directory CSW does not manage),
// "self" (this settings window was frontmost), "other". Text is built with
// T() because it embeds the environment name (user data).
function handleWhereami(p) {
  if (!p || !p.kind) return;
  if (p.kind === 'claude' && p.environment) {
    const name = p.environment;
    if (profiles.some((x) => x.name === name)) withTransition(() => showDetail(name));
    showToast(name === 'default'
      ? T('前面のClaudeは既存のClaudeです。', 'The Claude in front is Existing Claude.')
      : T(`前面のClaudeは「${name}」です。`, `The Claude in front is "${name}".`));
    return;
  }
  if (p.kind === 'claude') {
    showToast(T('前面のClaudeは、CSWのどの環境でもないデータで動いています。',
      "The Claude in front runs on data outside CSW's environments."));
    return;
  }
  if (p.kind === 'self') {
    showToast(T('この設定ウィンドウが前面です。確かめたいClaudeのウィンドウをクリックしてから、もう一度お試しください。',
      'This settings window is in front. Click the Claude window you want to check, then try again.'));
    return;
  }
  showToast(T('前面のウィンドウはClaudeではありません。', 'The window in front is not Claude.'));
}

// The update-takeover notice. The watcher (Rust) records the transition; this
// banner explains it and points back to the environment the user was in. All
// text is built with T(): it embeds the environment name (user data).
let takeoverFrom = null;
function renderTakeover() {
  const name = takeoverFrom;
  if (!name || !profiles.some((p) => p.name === name)) {
    el.takeoverBanner.hidden = true;
    return;
  }
  el.takeoverText.textContent = T(
    `Claudeがアプリの更新などで、CSWを経由せずに既存のClaudeとして開き直されました。「${name}」で使い直すには、起動中のClaudeを一度終了してから、CSWで「${name}」を選んで起動し直してください。`,
    `Claude was relaunched outside CSW as Existing Claude, which can happen after an app update. To use "${name}" again, quit the running Claude, then select "${name}" in CSW and launch it.`);
  el.takeoverBanner.hidden = false;
}

// When the window regains focus or becomes visible again, the user may have just
// quit (or started) a Claude Desktop elsewhere, or moved an environment folder
// back from the Trash. Re-list the environments and the running state so the
// sidebar and the launch actions reflect reality without a restart
// (list_profiles is a single directory scan, so this stays cheap).
async function revalidateRunning() {
  const before = JSON.stringify([desktopRunning, runningProfiles, profiles.map((p) => p.name)]);
  await refreshProfiles();
  if (!el.viewUsage.hidden) {
    await loadUsageReports();
    return;
  }
  if (JSON.stringify([desktopRunning, runningProfiles, profiles.map((p) => p.name)]) === before) return;
  if (!el.viewDetail.hidden && selectedName) {
    if (profiles.some((p) => p.name === selectedName)) {
      showDetail(selectedName);
    } else {
      // The shown environment disappeared outside CSW: fall back gracefully.
      selectedName = null;
      const remaining = profiles.filter((p) => p.name !== 'default');
      withTransition(() => (remaining.length ? showDetail(remaining[0].name) : showEmpty()));
    }
  }
}

// --- Init -------------------------------------------------------------------
function wireEvents() {
  el.btnCancelPendingLaunch.addEventListener('click', doCancelPendingLaunch);
  el.btnRecoverPendingLaunch.addEventListener('click', doRecoverPendingLaunch);
  setInterval(refreshPendingLaunch, 1000);
  el.btnCreate.addEventListener('click', () => withTransition(showCreate));
  el.btnCreateEmpty.addEventListener('click', () => withTransition(showCreate));
  el.btnUsage.addEventListener('click', () => withTransition(showUsage));
  el.btnUsageRefresh.addEventListener('click', loadUsageReports);
  el.usageScroll.addEventListener('scroll', refreshFades, { passive: true });
  setInterval(() => {
    if (!document.hidden && !el.viewUsage.hidden) loadUsageReports();
  }, 60000);

  document.querySelectorAll('input[name="mode"]').forEach((r) =>
    r.addEventListener('change', () => {
      if (advancedCustomized) showToast('モードを変えたので高度設定をリセットしました');
      setMode(r.value);
    }));

  el.advancedToggle.addEventListener('click', () => {
    const open = el.advancedToggle.closest('.advanced').classList.contains('open');
    if (open) closeAdvanced(); else openAdvanced();
  });
  el.advancedReset.addEventListener('click', () => setMode(currentMode));
  el.advancedGroups.addEventListener('change', (e) => {
    const input = e.target.closest('input[type="radio"]');
    if (!input || input.disabled) return;
    overrides[input.name.replace('seg-', '')] = input.value;
    advancedCustomized = ALL_KEYS.some((k) => overrides[k] !== PRESETS[currentMode][k]);
    updateAdvancedState();
  });

  window.addEventListener('focus', revalidateRunning);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) revalidateRunning(); });

  // The tray's "いまの環境を確認": the Rust side resolves the frontmost app
  // before showing this window, then emits the result here.
  if (HAS_TAURI && window.__TAURI__.event) {
    window.__TAURI__.event.listen('whereami', (e) => handleWhereami(e.payload));
    window.__TAURI__.event.listen('pending-launch-changed', (e) => acceptPendingLaunch(e.payload));
    window.__TAURI__.event.listen('pending-launch-error', (e) => showToast(launchError(e.payload), true));
  } else {
    // Browser QA drives the same handler directly.
    window.__DEV_WHEREAMI = handleWhereami;
  }

  el.btnTakeoverClose.addEventListener('click', async () => {
    try { await invoke('dismiss_takeover_notice'); } catch (e) { /* the banner hides regardless */ }
    takeoverFrom = null;
    el.takeoverBanner.hidden = true;
  });
  el.btnReopenClose.addEventListener('click', async () => {
    try { await invoke('dismiss_reopen'); } catch (e) { /* the banner hides regardless */ }
    reopenOffer = [];
    el.reopenBanner.hidden = true;
  });
  el.btnReopen.addEventListener('click', async () => {
    let res;
    try { res = await invoke('reopen_previous_set'); }
    catch (e) { showToast(launchError(String(e)), true); return; }
    reopenOffer = [];
    el.reopenBanner.hidden = true;
    if (res.launched && res.launched.length) {
      showToast(T(`${res.launched.length}個の環境を開き直しました。`,
        `Reopened ${res.launched.length} environment${res.launched.length === 1 ? '' : 's'}.`));
    }
    if (res.blocked && res.blocked.length) {
      showToast(reopenFailureMessage(res), true);
    }
    await refreshRunning();
  });

  el.emptyScroll.addEventListener('scroll', () => fadeFor(el.emptyScroll, el.emptyFade), { passive: true });
  el.detailScroll.addEventListener('scroll', () => fadeFor(el.detailScroll, el.detailFade), { passive: true });
  el.createScroll.addEventListener('scroll', () => fadeFor(el.createScroll, el.createFade), { passive: true });
  window.addEventListener('resize', refreshFades);
  el.inputName.addEventListener('keydown', (e) => {
    // Ignore Enter while an IME composition is active (e.g. confirming a Japanese
    // kanji conversion) so it never creates; otherwise go to the confirmation step.
    if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) { e.preventDefault(); requestCreate(); }
  });
}

function checkFirstRun() {
  const show = !localStorage.getItem('csw_onboarded');
  el.emptyFirstRun.hidden = !show;
  el.firstRunExtra.hidden = !show;
}

// --- Accent theme (user-selectable, persisted) ------------------------------
const ACCENTS = ['blue', 'teal', 'indigo', 'claude'];
function applyAccent(name) {
  const a = ACCENTS.includes(name) ? name : 'blue';
  document.documentElement.dataset.accent = a;
  try { localStorage.setItem('csw_accent', a); } catch (e) {}
  document.querySelectorAll('.theme-dot').forEach((d) => {
    const on = d.dataset.accent === a;
    d.classList.toggle('active', on);
    if (on) d.setAttribute('aria-current', 'true'); else d.removeAttribute('aria-current');
  });
}
function initAccent() {
  let saved = 'blue';
  try { saved = localStorage.getItem('csw_accent') || 'blue'; } catch (e) {}
  applyAccent(saved);
  const picker = document.getElementById('themePicker');
  if (picker) picker.addEventListener('click', (e) => {
    const dot = e.target.closest('.theme-dot');
    if (dot) applyAccent(dot.dataset.accent);
  });
}

// --- Footer: help links, version, about dialog ------------------------------
// Same disclaimer the LP shows under "免責・利用について" (kept in sync by hand).
const DISCLAIMER = [
  ['無保証', '本ソフトウェアはMITライセンスのもとで、そのままの状態で提供されます。動作や品質について、いかなる保証もありません。'],
  ['自己責任でのご利用', 'ご利用は自己責任でお願いします。データの損失や不具合などについて、作者は責任を負いません。大切なデータは、お試しになる前にバックアップしておくことをおすすめします。'],
  ['プライバシー', 'CSWはインターネット通信も、利用状況の送信も行いません。パスワードを保管するmacOSのキーチェーンにも触れず、すべて手元のフォルダと設定の操作だけで動きます。'],
  ['何を読み書きするか', '通常はCSW専用のフォルダに書き込みます。ローカルCode全体の共有を明示的に有効にすると、既存のClaudeを含む参加環境のCodeストアも置き換えます。サインイン情報は対象外です。読み書きの範囲と通信しないことの確かめ方は、プライバシーと透明性の文書で公開しています。'],
  ['非公式プロジェクト', '本プロジェクトは非公式のコミュニティ製で、Anthropic社とは関係ありません。「Claude」はAnthropicの商標です。'],
];

// Fixed GitHub URLs for the privacy document (ja/en). Listed, like the footer
// links (data-url, and data-url-en for the English UI), in docs/PRIVACY.md as
// the complete set of URLs open_url may receive.
const PRIVACY_URL_JA = 'https://github.com/seichris/claude-desktop-switcher/blob/main/docs/PRIVACY.md';
const PRIVACY_URL_EN = 'https://github.com/seichris/claude-desktop-switcher/blob/main/docs/PRIVACY_EN.md';
// The releases page, where csw is published. The same URL as the footer's
// update check (index.html), so it adds nothing to that list.
const RELEASES_URL = 'https://github.com/seichris/claude-desktop-switcher/releases';

// Hand a fixed https GitHub URL to the OS (Rust open_url) so it opens in the
// default browser. CSW makes no network requests itself.
async function openExternal(url) {
  try {
    await invoke('open_url', { url });
  } catch (e) {
    showToast('リンクを開けませんでした。', true);
  }
}

function showAbout() {
  let overlay;
  const close = () => {
    overlay.remove();
    document.removeEventListener('keydown', onKey);
    el.btnAbout.focus();
  };
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  const closeBtn = h('button', { type: 'button', class: 'btn btn-ghost', onclick: close }, '閉じる');
  const privacyBtn = h('button', {
    type: 'button', class: 'btn btn-ghost',
    onclick: () => openExternal(T(PRIVACY_URL_JA, PRIVACY_URL_EN)),
  }, '確かめ方を見る');
  const card = h('div', { class: 'about-card', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'このアプリについて' },
    h('div', { class: 'about-title', text: 'Claude Desktop Switcher' }),
    h('div', { class: 'about-version', text: el.appVersion.textContent || '' }),
    h('p', { class: 'about-intro', text: '非公式のオープンソースのコミュニティプロジェクトです。' }),
    h('div', { class: 'about-list' },
      ...DISCLAIMER.map(([t, b]) => h('div', { class: 'about-item' },
        h('div', { class: 'about-item-title', text: t }),
        h('div', { class: 'about-item-body', text: b })))),
    h('div', { class: 'about-foot' }, privacyBtn, closeBtn));
  overlay = h('div', { class: 'about-overlay', onclick: (e) => { if (e.target === overlay) close(); } }, card);
  document.addEventListener('keydown', onKey);
  document.body.appendChild(overlay);
  closeBtn.focus();
}

function wireFooter() {
  // Static link buttons: the footer links and the first-run card's releases
  // button. data-url-en, when present, is the English page for the English UI.
  document.querySelectorAll('button[data-url]').forEach((b) =>
    b.addEventListener('click', () => openExternal(T(b.dataset.url, b.dataset.urlEn || b.dataset.url))));
  el.btnAbout.addEventListener('click', showAbout);
  // Version from tauri.conf.json (release-please keeps it current). Hide on failure.
  invoke('app_version')
    .then((v) => { el.appVersion.textContent = 'v' + v; })
    .catch(() => { el.appVersion.hidden = true; });
}

// --- Leftover installer disk image (SPECIFICATION.md §5.A) ------------------
// Prompt to eject a still-mounted CSW .dmg. "あとで" hides the banner for this
// session only; the next launch prompts again while the image stays mounted.
let dmgMounts = [];
let dmgDismissed = false;

async function checkDmgLeftover() {
  try { dmgMounts = (await invoke('get_dmg_mount_status')) || []; }
  catch (e) { dmgMounts = []; }
  el.dmgBanner.hidden = dmgDismissed || dmgMounts.length === 0;
}

async function doEjectDmg() {
  el.btnDmgEject.disabled = true;
  try {
    for (const m of dmgMounts) await invoke('eject_dmg', { mountPoint: m });
    el.dmgBanner.hidden = true;
    showToast('ディスクイメージを取り出しました');
  } catch (err) {
    showToast('取り出せませんでした。ディスクイメージを使用中のウィンドウを閉じてから、もう一度お試しください。', true);
  } finally {
    el.btnDmgEject.disabled = false;
  }
}

function wireDmgBanner() {
  el.btnDmgEject.addEventListener('click', doEjectDmg);
  el.btnDmgLater.addEventListener('click', () => {
    dmgDismissed = true;
    el.dmgBanner.hidden = true;
  });
}

async function init() {
  initAccent();
  wireEvents();
  wireFooter();
  wireDmgBanner();
  checkDmgLeftover();
  await refreshProfiles();
  checkFirstRun();
  showEmpty();
}

document.addEventListener('DOMContentLoaded', init);

// ============================================================================
// Dev-only mock (used only when window.__TAURI__ is absent, e.g. screenshots).
// The shipped app sets withGlobalTauri:true and never reaches this path.
// ============================================================================
// Browser-QA state: notes saved through the mock survive within the session,
// so flows that re-fetch details (note edit -> signpost quote) behave like the
// real backend, which re-reads profile.toml.
const DEV_NOTES = new Map();
const DEV_CODE_MEMBERS = new Set();
const DEV_CODE_CALLS = [];
let devPending = { request_id: 0, target: null, phase: 'idle', error: null, wait_for_exit: true };
let devRunning = ['default'];
if (!HAS_TAURI) window.__DEV_PENDING = {
  complete() {
    if (!['waiting', 'switching'].includes(devPending.phase)) return;
    devRunning = [devPending.target];
    devPending = { ...devPending, phase: 'completed', error: null };
    acceptPendingLaunch({ ...devPending });
  },
  fail(code = 'history_launch_pending') {
    devRunning = [];
    devPending = { ...devPending, phase: 'failed', error: code };
    acceptPendingLaunch({ ...devPending });
  },
  switching() { devPending = { ...devPending, phase: 'switching' }; acceptPendingLaunch({ ...devPending }); },
};

function devInvoke(cmd, args) {
  // Locale-aware sample names so English screenshots read naturally. Real user
  // environment names are never translated (they are not run through the dictionary).
  const NM = LANG === 'ja' ? { work: '仕事用', research: '研究用', testing: '検証用' } : { work: 'Work', research: 'Research', testing: 'Testing' };
  // Sample notes are user data in the real app; here they are locale-aware so
  // screenshots read naturally in both languages.
  const NOTE = LANG === 'ja'
    ? { work: '会社のGoogleアカウントでサインイン', research: '研究室のAnthropicアカウントでサインイン' }
    : { work: 'Signs in with the company Google account', research: 'Signs in with the lab Anthropic account' };
  const ago = (hours) => new Date(Date.now() - hours * 3600 * 1000).toISOString();
  const prof = (name, iconVal, concurrent, sharing, extra) => ({
    name, icon: iconVal, is_default: false, supports_concurrent_windows: concurrent,
    desktop_path: `~/.context-switcher-claude/profiles/${name}/desktop-data`,
    cli_path: `~/.context-switcher-claude/profiles/${name}/cli-data`,
    sharing,
    note: '', created_at: null, cloned_from: null, last_launched_at: null,
    signin_signpost_due: false,
    ...extra,
  });
  const sample = {
    default: { name: 'default', icon: '', is_default: true, desktop_path: '~/Library/Application Support/Claude', cli_path: '~/.claude', supports_concurrent_windows: false, sharing: Object.fromEntries(ALL_KEYS.map((k) => [k, 'share'])) },
    [NM.work]: prof(NM.work, 'briefcase', false, { ...PRESETS.share_code },
      { note: NOTE.work, created_at: ago(24 * 40), first_launched_at: ago(24 * 40), last_launched_at: ago(3) }),
    // Launched exactly once: the sign-in signpost state, for browser QA. One
    // shared timestamp: three ago() calls could straddle a millisecond and
    // break the first == last equality this fixture exists to represent.
    [NM.research]: (() => {
      const t = ago(24 * 3);
      return prof(NM.research, 'graduation-cap', false, { ...PRESETS.share_workspace },
        { note: NOTE.research, created_at: t, first_launched_at: t, last_launched_at: t, signin_signpost_due: true });
    })(),
    [NM.testing]: prof(NM.testing, 'flask', true, { ...PRESETS.isolate },
      { created_at: ago(2), cloned_from: NM.work }),
  };
  for (const profile of Object.values(sample)) {
    profile.code_sharing = { enabled: false, eligible: false, source: 'default', participants: 0 };
  }
  sample[NM.research].sharing = { ...PRESETS.share_code };
  sample[NM.research].code_sharing.eligible = true;
  sample.default.code_sharing = { enabled: true, eligible: false, source: 'default', participants: 2 };
  sample[NM.work].code_sharing = { enabled: true, eligible: true, source: 'default', participants: 2 };
  if (DEV_CODE_MEMBERS.has(NM.research)) {
    sample[NM.research].code_sharing = { enabled: true, eligible: true, source: 'default', participants: 3 };
  }
  for (const [name, note] of DEV_NOTES) {
    if (sample[name]) sample[name].note = note;
  }
  switch (cmd) {
    case 'enable_code_sharing':
      DEV_CODE_CALLS.push({ ...args });
      if (!args.replace || desktopRunning || ['waiting', 'switching'].includes(pendingLaunch.phase)) return Promise.reject('Claude must be stopped and replacement confirmed');
      DEV_CODE_MEMBERS.add(args.name);
      return Promise.resolve('~/.context-switcher-claude/native-code-backups/synthetic-backup');
    case 'recover_pending_code_launch': {
      if (args.requestId !== devPending.request_id) return Promise.reject('launch_stale');
      if (!args.abandon || devPending.phase !== 'failed') return Promise.reject('launch_refused');
      if (devRunning.length) return Promise.reject('desktop_reappeared');
      devPending = { request_id: devPending.request_id + 1, target: null, targets: [], phase: 'idle', error: null, wait_for_exit: true };
      return Promise.resolve({ ...devPending });
    }
    case 'get_pending_launch': return Promise.resolve({ ...devPending });
    case 'request_environment_launch':
    case 'launch_additional_window': {
      if (['waiting', 'switching'].includes(devPending.phase)) {
        if (devPending.target === args.name) return Promise.resolve({ ...devPending });
        return Promise.reject('launch_pending');
      }
      devPending = { request_id: devPending.request_id + 1, target: args.name, phase: 'waiting', error: null, wait_for_exit: cmd !== 'launch_additional_window' };
      return Promise.resolve({ ...devPending });
    }
    case 'cancel_pending_launch': {
      if (args.requestId !== devPending.request_id) return Promise.reject('launch_stale');
      if (devPending.phase !== 'waiting') return Promise.reject('launch_committed');
      devPending = { ...devPending, phase: 'canceled' };
      return Promise.resolve({ ...devPending });
    }
    case 'get_usage_reports': {
      const now = Date.now();
      const window = (key, used_percent, resets_at_ms, state = 'available') => ({ key, used_percent, resets_at_ms, state });
      return Promise.resolve([
        { profile: 'default', state: 'fresh', source: 'cache', captured_at_ms: now - 4 * 60000,
          windows: [window('five_hour', 24, now + 2 * 3600000), window('seven_day', 58, now + 3 * 86400000)] },
        { profile: NM.work, state: 'stale', source: 'cache', captured_at_ms: now - 8 * 3600000,
          windows: [window('five_hour', 78, now - 4 * 3600000, 'reset_passed'), window('seven_day', 92, now + 2 * 86400000)] },
        { profile: NM.research, state: 'stale', source: 'history', captured_at_ms: now - 45 * 60000,
          windows: [window('five_hour', 12, null), window('seven_day', null, null, 'missing')] },
        { profile: NM.testing, state: 'missing', source: null, captured_at_ms: null,
          windows: [window('five_hour', null, null, 'missing'), window('seven_day', null, null, 'missing')] },
      ]);
    }
    case 'list_profiles':
      return Promise.resolve([sample.default, sample[NM.work], sample[NM.research], sample[NM.testing]]
        .map((p) => ({
          name: p.name, icon: p.icon, is_default: p.is_default,
          note: p.note || '', last_launched_at: p.last_launched_at || null,
        })));
    case 'get_active_profile':
      return Promise.resolve('default');
    case 'get_profile_details':
      return Promise.resolve(sample[args.name] || sample[NM.testing]);
    case 'get_desktop_running_status':
      // Depict the active environment as actually running so screenshots show the
      // "利用中" marker (the realistic state when Claude is open for that account).
      return Promise.resolve(devRunning.length > 0);
    case 'get_running_profiles':
      // The existing Claude is depicted as running (mirrors get_desktop_running_status).
      return Promise.resolve([...devRunning]);
    case 'activate_environment':
      // Dev mock: pretend the environment's Claude was found and raised.
      return Promise.resolve(true);
    case 'get_profile_data_map': {
      // Plausible healthy map shaped like the Rust serde output (lowercase
      // modes, LINK_ITEMS order, share_settings-style environment).
      const base = `~/.context-switcher-claude/profiles/${args.name}`;
      const t = 1782900000;
      const items = [
        { key: 'desktop_config', mode: 'isolate', link_target: null, size_bytes: 2048, modified_epoch: t, exists: true },
        { key: 'cli_settings', mode: 'copy', link_target: null, size_bytes: 6144, modified_epoch: t, exists: true },
        { key: 'cli_claude_md', mode: 'share', link_target: '~/.claude/CLAUDE.md', size_bytes: null, modified_epoch: null, exists: true },
        { key: 'cli_rules', mode: 'share', link_target: '~/.claude/rules', size_bytes: null, modified_epoch: null, exists: true },
        { key: 'cli_project_memory', mode: 'isolate', link_target: null, size_bytes: 48 * 1024 * 1024, modified_epoch: t, exists: true },
        { key: 'cli_plugins', mode: 'share', link_target: '~/.claude/plugins', size_bytes: null, modified_epoch: null, exists: true },
        { key: 'cli_skills', mode: 'share', link_target: '~/.claude/skills', size_bytes: null, modified_epoch: null, exists: true },
        { key: 'cli_sessions', mode: 'isolate', link_target: null, size_bytes: 12288, modified_epoch: t, exists: true },
        { key: 'cli_history', mode: 'isolate', link_target: null, size_bytes: null, modified_epoch: null, exists: false },
        { key: 'desktop_worktrees', mode: 'copy', link_target: null, size_bytes: 1024, modified_epoch: t, exists: true },
        { key: 'desktop_device_id', mode: 'isolate', link_target: null, size_bytes: 64, modified_epoch: t, exists: true },
        { key: 'desktop_app_config', mode: 'isolate', link_target: null, size_bytes: null, modified_epoch: null, exists: true },
      ];
      return Promise.resolve({
        profile: args.name,
        desktop_dir: `${base}/desktop-data`,
        cli_dir: `${base}/cli-data`,
        desktop_size_bytes: 96 * 1024 * 1024,
        cli_size_bytes: 49 * 1024 * 1024,
        total_size_bytes: 145 * 1024 * 1024,
        items,
      });
    }
    case 'reveal_profile_dir':
      return Promise.resolve(null);
    case 'set_profile_note':
      DEV_NOTES.set(args.name, args.note);
      return Promise.resolve(null);
    case 'get_takeover_notice':
      // Browser QA: drive the takeover scenario via window.__DEV_TAKEOVER.
      return Promise.resolve(window.__DEV_TAKEOVER || null);
    case 'dismiss_takeover_notice':
      window.__DEV_TAKEOVER = null;
      return Promise.resolve(null);
    case 'get_reopen_offer':
      // Browser QA: drive the reopen scenario via window.__DEV_REOPEN (array of names).
      return Promise.resolve(window.__DEV_REOPEN || []);
    case 'reopen_previous_set': {
      const offer = window.__DEV_REOPEN || [];
      window.__DEV_REOPEN = [];
      // Mirror the plan: the existing Claude / a shared env is "blocked" here
      // for QA when marked, isolated ones "launched".
      const blocked = (window.__DEV_REOPEN_BLOCKED || []).filter((n) => offer.includes(n));
      return Promise.resolve({ launched: offer.filter((n) => !blocked.includes(n)), blocked });
    }
    case 'dismiss_reopen':
      window.__DEV_REOPEN = [];
      return Promise.resolve(null);
    case 'inspect_profile': {
      // Healthy sample report for browser QA / screenshots, shaped exactly like
      // the Rust inspector's serde output: lowercase SharingMode values and the
      // LINK_ITEMS order (share_settings-style environment).
      const modes = { ...PRESETS.share_settings, cli_sessions: 'isolate', desktop_app_config: 'isolate', desktop_config: 'isolate' };
      const order = ['desktop_config', 'cli_settings', 'cli_claude_md', 'cli_rules', 'cli_project_memory', 'cli_plugins',
        'cli_skills', 'cli_sessions', 'cli_history', 'desktop_worktrees', 'desktop_device_id', 'desktop_app_config'];
      const items = order.map((key) => (modes[key] === 'share'
        ? { key, mode: 'share', health: { state: 'shared_ok', target: '~/.claude/' + key }, is_issue: false }
        : { key, mode: modes[key], health: { state: 'isolated_ok' }, is_issue: false }));
      return Promise.resolve({ profile: args.name, items, issue_count: 0, running: false });
    }
    case 'get_default_roots_status':
      return Promise.resolve({ desktop_present: true, cli_present: true });
    case 'get_dmg_mount_status':
      // Browser/screenshot mode never shows the banner; QA forces it via eval.
      return Promise.resolve([]);
    case 'eject_dmg':
      return Promise.resolve(null);
    case 'app_version': {
      // The real version lives in tauri.conf.json, which the browser mock cannot
      // read. A manual ?appver= shows one; without it the footer version is hidden
      // rather than showing a stale hardcoded one. Screenshot capture
      // (scripts/appshot) leaves it out: screenshots are taken before
      // release-please bumps the version, so any injected value would be stale.
      const appver = new URLSearchParams(location.search).get('appver');
      return appver ? Promise.resolve(appver) : Promise.reject(new Error('no appver'));
    }
    case 'open_url':
      return Promise.resolve(null);
    default:
      return Promise.resolve(null);
  }
}
