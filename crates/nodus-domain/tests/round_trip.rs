//! Each type reads the JSON the TypeScript writes and writes it back the same,
//! byte for byte: field names, order, absent fields and nulls.

use nodus_domain::json;
use nodus_domain::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Reads `text` as `T` and checks it's written back exactly as it was.
fn round_trip<T: Serialize + DeserializeOwned>(text: &str) -> T {
    let value: T = json::from_str(text).unwrap_or_else(|err| panic!("{err}: {text}"));
    assert_eq!(json::to_string(&value).unwrap(), text);
    value
}

fn rejects<T: DeserializeOwned>(text: &str) {
    assert!(json::from_str::<T>(text).is_err(), "expected {text} to be refused");
}

#[test]
fn project() {
    let project: Project = round_trip(concat!(
        r#"{"id":"7c0e2f4a-1b3d-4e5f-8a9b-0c1d2e3f4a5b","name":"Example","repoPath":"/Users/example/repo","#,
        r#""color":"purple","createdAt":1790000000000,"openedAt":1790000123456.25,"#,
        r#""groupId":"5d6e7f80-9a1b-4c2d-8e3f-4a5b6c7d8e9f","actions":[{"id":"a1","name":"Ship it","#,
        r#""icon":"rocket","description":"Build and release","prompt":"Release the app.","createdAt":1790000000500}],"#,
        r#""focused":true}"#
    ));
    assert_eq!(project.color, FolderColor::Purple);
    assert_eq!(project.opened_at, Some(1_790_000_123_456.25));
    assert_eq!(project.archived, None);

    // The least a project has.
    let bare: Project = round_trip(r#"{"id":"p","name":"p","repoPath":"/tmp/p","color":"graphite","createdAt":1}"#);
    assert!(bare.group_id.is_none() && bare.actions.is_none() && bare.focused.is_none());
}

#[test]
fn project_keeps_fields_it_does_not_know() {
    let project: Project = round_trip(concat!(
        r#"{"id":"p","name":"p","repoPath":"/tmp/p","color":"blue","createdAt":1,"#,
        r#""actions":[{"id":"a","name":"a","icon":"zap","description":"","prompt":"go","createdAt":2,"allowedTools":["Bash"]}],"#,
        r#""pinned":{"at":3,"by":"someone"}}"#
    ));
    assert_eq!(project.extra["pinned"]["by"], "someone");
    assert_eq!(project.actions.unwrap()[0].extra["allowedTools"][0], "Bash");
}

#[test]
fn project_refuses_a_colour_it_does_not_know() {
    rejects::<Project>(r#"{"id":"p","name":"p","repoPath":"/tmp/p","color":"teal","createdAt":1}"#);
}

#[test]
fn group() {
    round_trip::<ProjectGroup>(r#"{"id":"g","name":"Infra","createdAt":1790000000000}"#);
    let group: ProjectGroup = round_trip(r#"{"id":"g","name":"Infra","createdAt":1790000000000,"collapsed":false}"#);
    assert_eq!(group.collapsed, Some(false));
}

#[test]
fn project_stats() {
    round_trip::<ProjectStats>(
        r#"{"total":12,"active":4,"working":1,"waiting":2,"running":{"sessions":3,"background":1,"terminals":2}}"#,
    );
}

#[test]
fn project_patch_tells_null_from_absent() {
    let leave: ProjectPatch = round_trip(r#"{"name":"Renamed"}"#);
    assert_eq!(leave.group_id, None);
    let ungroup: ProjectPatch = round_trip(r#"{"groupId":null}"#);
    assert_eq!(ungroup.group_id, Some(None));
    let regroup: ProjectPatch = round_trip(r#"{"color":"green","archived":true,"groupId":"g","focused":false}"#);
    assert_eq!(regroup.group_id, Some(Some("g".into())));
    round_trip::<GroupPatch>(r#"{"collapsed":true}"#);
}

#[test]
fn actions() {
    let action: ProjectAction = round_trip(
        r#"{"id":"a","name":"Tidy","icon":"brush-cleaning","description":"Clean up","prompt":"Tidy the repo.","global":true,"createdAt":5}"#,
    );
    assert_eq!(ActionIcon::parse(&action.icon), Some(ActionIcon::BrushCleaning));
    round_trip::<ActionDraft>(r#"{"name":"Test","icon":"test-tube","prompt":"Run the tests."}"#);
    round_trip::<ActionRunResult>(r#"{"projectId":"p","sessionId":"s","actionName":"Test","message":"All green."}"#);
    round_trip::<ActionRunResult>(r#"{"projectId":"p","sessionId":"s","actionName":"Test"}"#);
    round_trip::<ActionLists>(concat!(
        r#"{"project":{"id":"p","name":"p","repoPath":"/tmp/p","color":"red","createdAt":1},"#,
        r#""global":[{"id":"g","name":"Commit","icon":"git-commit-horizontal","description":"","prompt":"Commit.","global":true,"createdAt":2}]}"#
    ));
}

#[test]
fn action_icons_are_the_typescript_list() {
    let names: Vec<&str> = ActionIcon::ALL.iter().map(|icon| icon.as_str()).collect();
    assert_eq!(
        names,
        [
            "zap",
            "refresh-cw",
            "git-merge",
            "git-pull-request",
            "git-commit-horizontal",
            "upload",
            "download",
            "archive",
            "brush-cleaning",
            "eraser",
            "sparkles",
            "wand-sparkles",
            "test-tube",
            "bug",
            "rocket",
            "package",
            "file-text",
            "book-open",
            "list-checks",
            "search",
            "shield-check",
            "hammer",
            "square-terminal",
            "gauge",
            "lightbulb",
            "message-square",
        ]
    );
    assert_eq!(ActionIcon::DEFAULT.as_str(), "zap");
    assert_eq!(ActionIcon::parse("not-an-icon"), None);
}

#[test]
fn colours_and_kinds_are_the_typescript_lists() {
    let strings = |all: &[&str]| all.join(",");
    assert_eq!(
        strings(&FolderColor::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>()),
        "blue,purple,pink,red,orange,yellow,green,graphite"
    );
    assert_eq!(
        strings(&InkColor::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>()),
        "graphite,red,orange,green,blue,purple"
    );
    assert_eq!(
        strings(&InkKind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>()),
        "pen,arrow,text,rect"
    );
    assert_eq!(
        strings(&CaptureKind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>()),
        "session,into,note,view"
    );
    assert_eq!(
        strings(&MenuCommand::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>()),
        "close,new-session,new-terminal,new-project,settings,voice-settings,update-settings,search,toggle-focus,report-problem"
    );
    assert_eq!(CaptureKind::parse("into"), Some(CaptureKind::Into));
    assert_eq!(CaptureKind::parse("Into"), None);
    assert_eq!(FolderColor::Graphite.to_string(), "graphite");
}

#[test]
fn board_node_as_session_create_writes_it() {
    let node: BoardNode = round_trip(concat!(
        r#"{"sessionId":"0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d","kind":"terminal","x":120,"y":-48.5,"archived":false,"#,
        r#""title":"Logs","createdByApp":true,"expanded":true,"createdAt":1790000000000}"#
    ));
    assert_eq!(node.kind, Some(SessionKind::Terminal));
    assert_eq!(node.y, -48.5);
}

#[test]
fn board_node_with_everything() {
    round_trip::<BoardNode>(concat!(
        r#"{"sessionId":"s","x":0,"y":0,"archived":true,"title":"Merged","parents":["a","b"],"createdByApp":true,"#,
        r#""runOf":"act","expanded":false,"viewOf":"v","autoMode":true,"fromView":"v2","createdAt":1790000000000,"#,
        r#""w":720,"h":480,"archivedAt":1790000999999.5,"sectionId":"sec"}"#
    ));
}

#[test]
fn imported_board_node_keeps_its_fractional_time() {
    // An imported card takes the transcript file's mtime, fraction and all.
    let node: BoardNode =
        round_trip(r#"{"sessionId":"s","x":40,"y":372,"archived":true,"createdAt":1790000000000.452}"#);
    assert_eq!(node.kind, None);
    assert_eq!(node.created_at, 1_790_000_000_000.452);
}

#[test]
fn board_node_keeps_fields_it_does_not_know() {
    let node: BoardNode = round_trip(r#"{"sessionId":"s","x":1,"y":2,"archived":false,"createdAt":3,"pinnedAt":4}"#);
    assert_eq!(node.extra["pinnedAt"], 4);
}

#[test]
fn board_node_model_is_kept_whatever_its_shape() {
    let mut node: BoardNode =
        round_trip(r#"{"sessionId":"s","x":1,"y":2,"archived":false,"createdAt":3,"model":"opus"}"#);
    assert_eq!(node.model(), Some("opus"));
    node.set_model(Some(SessionModel::Haiku));
    assert_eq!(
        json::to_string(&node).unwrap(),
        r#"{"sessionId":"s","x":1,"y":2,"archived":false,"createdAt":3,"model":"haiku"}"#
    );
    node.set_model(None);
    assert_eq!(node.model(), None);
    // A newer version's model, of another shape: no model here, written back as it was.
    let newer: BoardNode =
        round_trip(r#"{"sessionId":"s","x":1,"y":2,"archived":false,"createdAt":3,"model":{"name":"Opus 9"}}"#);
    assert_eq!(newer.model(), None);
}

#[test]
fn node_patch_tells_null_from_absent() {
    let moved: NodePatch = round_trip(r#"{"sessionId":"s","x":10.5,"y":20}"#);
    assert_eq!(moved.section_id, None);
    let ungrouped: NodePatch = round_trip(r#"{"sessionId":"s","sectionId":null}"#);
    assert_eq!(ungrouped.section_id, Some(None));
    let grouped: NodePatch = round_trip(r#"{"sessionId":"s","sectionId":"sec","expanded":true,"w":600,"h":400}"#);
    assert_eq!(grouped.section_id, Some(Some("sec".into())));
}

#[test]
fn sections_notes_and_ink() {
    round_trip::<BoardSection>(r#"{"id":"sec","name":"","color":"orange","x":-200,"y":80,"createdAt":1790000000000}"#);
    round_trip::<BoardNote>(
        r#"{"id":"n","text":"Remember the\nmigration","color":"yellow","x":10,"y":20,"sessionId":"s","createdAt":1}"#,
    );
    round_trip::<BoardNote>(concat!(
        r#"{"id":"n","text":"Found it","color":"green","x":10,"y":20,"w":320,"h":200,"from":"s","#,
        r#""fromProject":"p","fromTitle":"Hunting the bug","createdAt":1}"#
    ));
    let stroke: InkStroke =
        round_trip(r#"{"id":"i","kind":"pen","color":"graphite","points":[0,0,10.25,12.5,20,-3],"size":1.5}"#);
    assert_eq!(stroke.points.len(), 6);
    round_trip::<InkStroke>(r#"{"id":"t","kind":"text","color":"red","points":[5,5],"size":1,"text":"Here"}"#);
    rejects::<InkStroke>(r#"{"id":"i","kind":"marker","color":"red","points":[],"size":1}"#);
}

#[test]
fn rect_and_viewport() {
    round_trip::<Rect>(r#"{"x":0,"y":0,"w":1928,"h":320}"#);
    round_trip::<Viewport>(r#"{"x":-273.4262580407277,"y":325.7206949380824,"zoom":0.5857424799943102}"#);
}

#[test]
fn view_as_the_ui_creates_it() {
    let view: BoardView = round_trip(concat!(
        r#"{"id":"v","title":"New view","description":"Open PRs","x":100,"y":200,"version":0,"#,
        r#""createdAt":1790000000000,"updatedAt":1790000000000}"#
    ));
    assert!(view.commands.is_none() && view.refresh.is_none());
}

#[test]
fn view_with_everything() {
    let view: BoardView = round_trip(concat!(
        r#"{"id":"v","title":"CI","description":"Build status","x":100,"y":200,"version":7,"#,
        r#""createdAt":1790000000000,"updatedAt":1790000500000,"sessionId":"brain","from":"s","#,
        r#""commands":[{"name":"runs","run":"gh run list --json status","every":5,"approved":"gh run list --json status"},"#,
        r#"{"name":"rm","run":"rm -rf build","denied":"rm -rf build"}],"#,
        r#""refresh":{"every":30,"prompt":"Check the deploys."},"w":480,"h":360,"startsSessions":true,"#,
        r#""allowedRuns":["gh run list --json status"],"deniedRuns":["rm -rf build"]}"#
    ));
    assert_eq!(view.commands.unwrap()[1].approved, None);
}

#[test]
fn view_results_and_patches() {
    let result: CommandResult = round_trip(
        r#"{"name":"runs","code":null,"stdout":"","stderr":"","at":1790000000000,"durationMs":15000,"error":"Timed out"}"#,
    );
    assert_eq!(result.code, None);
    round_trip::<CommandResult>(r#"{"name":"runs","code":0,"stdout":"ok\n","stderr":"","at":1,"durationMs":42}"#);
    // Windows' exit codes don't fit in 32 bits.
    round_trip::<CommandResult>(r#"{"name":"crash","code":3221225477,"stdout":"","stderr":"","at":1,"durationMs":2}"#);
    round_trip::<ViewPatch>(r#"{"id":"v","x":1,"y":2,"w":300,"h":200,"title":"Renamed","startsSessions":false}"#);
    round_trip::<ViewPatch>(r#"{"id":"v"}"#);
    round_trip::<CreateViewOptions>(r#"{"description":"A clock","x":10,"y":20}"#);
    round_trip::<ViewReport>(r#"{"error":"ReferenceError: x is not defined"}"#);
    round_trip::<ViewReport>(r#"{"loaded":3}"#);
    assert_eq!(json::to_string(&ViewEvent::Result).unwrap(), r#""result""#);
}

#[test]
fn session_meta_and_usage() {
    round_trip::<SessionMeta>(concat!(
        r#"{"sessionId":"s","title":"Fix the login bug","firstPrompt":"The login fails","lastPrompt":"Ship it","#,
        r#""lastAssistantText":"Done.","gitBranch":"main","messageCount":14,"createdAt":1790000000000,"#,
        r#""updatedAt":1790000360000.75,"transcriptPath":"/Users/example/.claude/projects/-Users-example-repo/s.jsonl"}"#
    ));
    round_trip::<SessionMeta>(
        r#"{"sessionId":"s","title":"","messageCount":0,"createdAt":1,"updatedAt":1,"transcriptPath":"/t"}"#,
    );
    round_trip::<SessionUsage>(concat!(
        r#"{"model":"Opus 5.5","contextPercent":42.5,"contextTokens":85000,"contextSize":200000,"#,
        r#""costUsd":1.2345,"durationMs":3600000,"at":1790000000000}"#
    ));
    round_trip::<SessionUsage>(r#"{"at":1790000000000}"#);
}

#[test]
fn session_card_is_its_node_and_more() {
    let card: SessionCard = round_trip(concat!(
        r#"{"sessionId":"s","x":0,"y":0,"archived":false,"createdByApp":true,"createdAt":1,"#,
        r#""meta":{"sessionId":"s","title":"t","messageCount":2,"createdAt":1,"updatedAt":2,"transcriptPath":"/t"},"#,
        r#""status":"waiting","live":true,"agents":[{"agentId":"ag","agentType":"Explore","description":"Find the tests","#,
        r#""status":"working","parentAgentId":"p","tool":"Grep","activity":"login","background":1,"toolCount":3,"#,
        r#""startedAt":1,"updatedAt":2}],"agentCount":4,"usage":{"model":"Opus 5.5","at":3}}"#
    ));
    assert_eq!(card.node.session_id, "s");
    assert_eq!(card.status, SessionStatus::Waiting);
    assert!(card.node.extra.is_empty());
    round_trip::<SessionCard>(
        r#"{"sessionId":"s","kind":"terminal","x":0,"y":0,"archived":false,"createdAt":1,"status":"done","live":false}"#,
    );
}

#[test]
fn session_requests_and_results() {
    round_trip::<CreateSessionOptions>(concat!(
        r#"{"x":10,"y":20,"kind":"claude","name":"Merge","initialPrompt":"Carry on","parents":["a","b"],"#,
        r#""actionId":"act","expanded":true,"autoMode":true,"fromView":"v"}"#
    ));
    round_trip::<CreateSessionOptions>(r#"{"x":10,"y":20}"#);
    round_trip::<SummaryResult>(r#"{"sessionId":"s","title":"t","summary":"It works.","error":"Timed out"}"#);
    round_trip::<SummarizeProgress>(r#"{"requestId":"r","sessionId":"s","phase":"running"}"#);
    round_trip::<OpenSessionResult>(r#"{"replay":"\u001b[2J$ ","live":true}"#);
    round_trip::<TranscriptMessage>(concat!(
        r#"{"role":"assistant","text":"Reading it","timestamp":1790000000000,"tools":["Read"],"#,
        r#""calls":[{"name":"Read","detail":"src/main.rs"},{"name":"TodoWrite"}]}"#
    ));
    round_trip::<TranscriptMessage>(r#"{"role":"user","text":"Background task done","timestamp":1,"notice":true}"#);
}

#[test]
fn search_hits() {
    round_trip::<SessionSearchHit>(concat!(
        r#"{"sessionId":"s","matches":3,"snippets":[{"kind":"tool","tool":"Edit","file":"login.ts","agent":true,"#,
        r#""text":"…the login form…","ranges":[[4,9],[15,19]],"timestamp":1790000000000}]}"#
    ));
    round_trip::<SessionSearchHit>(r#"{"sessionId":"s","matches":0,"snippets":[]}"#);
}

#[test]
fn git() {
    let status: GitStatus = round_trip(concat!(
        r#"{"branch":null,"oid":null,"ahead":0,"behind":0,"files":[{"path":"new.rs","oldPath":"old.rs","status":"renamed"},"#,
        r#"{"path":"notes.md","status":"untracked"}]}"#
    ));
    assert_eq!(status.branch, None);
    round_trip::<GitStatus>(
        r#"{"branch":"main","oid":"abc123","upstream":"origin/main","ahead":2,"behind":1,"files":[]}"#,
    );
    round_trip::<GitFileDiff>(r#"{"patch":"","binary":true}"#);
    round_trip::<GitFileDiff>(r#"{"patch":"","tooLarge":true}"#);
    round_trip::<GitCommit>(r#"{"oid":"abc","subject":"Fix it","author":"Example Person","time":1790000000000}"#);
}

#[test]
fn update_status_flattens_its_state() {
    let cases = [
        r#"{"state":"off","reason":"A development build","current":"0.9.4"}"#,
        r#"{"state":"idle","current":"0.9.4"}"#,
        r#"{"state":"checking","current":"0.9.4"}"#,
        r#"{"state":"latest","current":"0.9.4"}"#,
        r#"{"state":"downloading","version":"0.9.5","percent":42,"current":"0.9.4"}"#,
        r#"{"state":"ready","version":"0.9.5","current":"0.9.4"}"#,
        r#"{"state":"error","message":"Offline","current":"0.9.4"}"#,
    ];
    for text in cases {
        round_trip::<UpdateStatus>(text);
    }
    let status: UpdateStatus = json::from_str(cases[4]).unwrap();
    assert_eq!(
        status.state,
        UpdateState::Downloading {
            version: "0.9.5".into(),
            percent: 42
        }
    );
}

#[test]
fn settings() {
    let settings: AppSettings = round_trip(concat!(
        r#"{"theme":"dark","autoUpdate":true,"notify":false,"dockBadge":true,"pinProjectRail":true,"#,
        r#""focusFeature":true,"focusMode":false,"capture":true,"wheelZoom":false}"#
    ));
    assert_eq!(settings.theme, ThemePreference::Dark);
    assert!(!settings.wheel_zoom);
    // On, the mouse wheel switch isn't written; a file without it reads it as on.
    let written = json::to_string(&AppSettings::default()).unwrap();
    assert_eq!(
        written,
        concat!(
            r#"{"theme":"system","autoUpdate":true,"notify":true,"dockBadge":true,"pinProjectRail":false,"#,
            r#""focusFeature":false,"focusMode":false,"capture":true}"#
        )
    );
    assert!(json::from_str::<AppSettings>(&written).unwrap().wheel_zoom);
    round_trip::<SettingsPatch>(r#"{"theme":"light","autoUpdate":false}"#);
}

#[test]
fn app_info_and_open_requests() {
    round_trip::<AppInfo>(r#"{"claudePath":null,"claudeVersion":null,"platform":"darwin"}"#);
    round_trip::<AppInfo>(
        r#"{"claudePath":"/usr/local/bin/claude","claudeVersion":"2.1.0 (Claude Code)","platform":"win32"}"#,
    );
    round_trip::<OpenCardRequest>(r#"{"projectId":"p"}"#);
    round_trip::<OpenCardRequest>(r#"{"projectId":"p","sessionId":"s"}"#);
    assert_eq!(json::to_string(&MenuCommand::ToggleFocus).unwrap(), r#""toggle-focus""#);
}

#[test]
fn voice() {
    assert_eq!(json::to_string(&Some(VoiceKeySource::Env)).unwrap(), r#""env""#);
    assert_eq!(json::to_string(&None::<VoiceKeySource>).unwrap(), "null");
    let text: TranscribeResult = round_trip(r#"{"text":"Add a dark mode"}"#);
    assert_eq!(
        text,
        TranscribeResult::Text {
            text: "Add a dark mode".into()
        }
    );
    let failed: TranscribeResult = round_trip(r#"{"error":"Unauthorized","keyProblem":true}"#);
    assert_eq!(
        failed,
        TranscribeResult::Failed {
            error: "Unauthorized".into(),
            key_problem: Some(true)
        }
    );
    round_trip::<TranscribeResult>(r#"{"error":"Offline"}"#);
}

#[test]
fn capture_attachments() {
    let attachments: Vec<CaptureAttachment> = round_trip(concat!(
        r#"[{"kind":"tab","url":"https://example.com/pr/1","title":"PR 1","app":"Safari"},"#,
        r#"{"kind":"image","path":"/tmp/captures/a.png","thumb":"data:image/png;base64,AAAA","copied":true},"#,
        r#"{"kind":"image","path":"/tmp/captures/b.png","thumb":"data:image/png;base64,BBBB"},"#,
        r#"{"kind":"file","path":"/tmp/report.pdf","name":"report.pdf","size":48213},"#,
        r#"{"kind":"text","text":"copied words"}]"#
    ));
    assert!(matches!(&attachments[3], CaptureAttachment::File(file) if file.size == 48_213));
    rejects::<CaptureAttachment>(r#"{"kind":"video","path":"/tmp/a.mov"}"#);
}

#[test]
fn capture_context_writes_its_tab_as_an_attachment() {
    let context: CaptureContext = round_trip(concat!(
        r#"{"app":"Safari","tab":{"kind":"tab","url":"https://example.com","title":"Example","app":"Safari"},"#,
        r#""clipboard":{"kind":"image","thumb":"data:image/png;base64,AAAA"},"shortcut":"needs-permission"}"#
    ));
    assert_eq!(context.tab.unwrap().title, "Example");
    round_trip::<CaptureContext>(r#"{"clipboard":{"kind":"text","text":"hello"},"shortcut":"ready"}"#);
    round_trip::<CaptureContext>(r#"{"shortcut":"off"}"#);
    rejects::<CaptureContext>(r#"{"tab":{"kind":"text","text":"not a tab"},"shortcut":"ready"}"#);
}

#[test]
fn capture_targets_guesses_and_receipts() {
    round_trip::<CaptureTargets>(concat!(
        r#"{"projects":[{"id":"p","name":"Example","color":"pink"}],"#,
        r#""sessions":[{"projectId":"p","sessionId":"s","title":"t","status":"working","live":true,"updatedAt":1790000000000}],"#,
        r#""recents":[{"id":"c","kind":"note","projectId":"p","title":"Buy milk","createdAt":1,"noteId":"n"},"#,
        r#"{"id":"d","kind":"session","projectId":"p","title":"Fix it","createdAt":2,"sessionId":"s","status":"done"}],"#,
        r#""defaultProjectId":null}"#
    ));
    round_trip::<CaptureGuess>(
        r#"{"kind":"into","projectId":"p","sessionId":"s","reason":"It's about the login bug."}"#,
    );
    round_trip::<CaptureSubmission>(
        r#"{"text":"Look at this","kind":"session","projectId":"p","attachments":[{"kind":"text","text":"x"}]}"#,
    );
    round_trip::<CaptureReceipt>(
        r#"{"kind":"view","projectId":"p","projectName":"Example","title":"Clock","viewId":"v"}"#,
    );
    assert_eq!(MAX_COPIED_CHARS, 20_000);
}
