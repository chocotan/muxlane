use muxlane_term::VTerm;

#[test]
fn vterm_tracks_terminal_mouse_modes() {
    let vt = VTerm::new(80, 24);
    assert!(!vt.mouse_reporting());
    vt.feed(b"\x1b[?1000h\x1b[?1006h");
    assert!(vt.mouse_reporting());
    assert!(vt.sgr_mouse());
    vt.feed(b"\x1b[?1000l\x1b[?1006l");
    assert!(!vt.mouse_reporting());
    assert!(!vt.sgr_mouse());
}

#[test]
fn vterm_tracks_application_cursor_mode() {
    let vt = VTerm::new(80, 24);
    assert!(!vt.modes().app_cursor);
    vt.feed(b"\x1b[?1h");
    assert!(vt.modes().app_cursor);
    vt.feed(b"\x1b[?1l");
    assert!(!vt.modes().app_cursor);
}

#[test]
fn wide_character_forces_following_text_into_next_run() {
    let vt = VTerm::new(10, 3);
    vt.feed("你a".as_bytes());
    let snap = vt.render_snapshot();
    let wide = snap.rows[0]
        .runs
        .iter()
        .find(|run| run.text.contains('你'))
        .unwrap();
    let ascii = snap.rows[0]
        .runs
        .iter()
        .find(|run| run.text.contains('a'))
        .unwrap();
    assert_eq!(wide.start_col, 0);
    assert_eq!(wide.cells, 2);
    assert_eq!(ascii.start_col, 2);
}

#[test]
fn combining_marks_remain_with_their_base_cell() {
    let vt = VTerm::new(10, 3);
    vt.feed("e\u{301}x".as_bytes());
    let text = vt.render_snapshot().rows[0]
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    assert!(text.starts_with("e\u{301}x"), "rendered={text:?}");
}

#[test]
fn logical_cursor_survives_hidden_visual_cursor() {
    let vt = VTerm::new(10, 3);
    vt.feed(b"abc\x1b[?25l");
    let snap = vt.render_snapshot();
    assert!(snap.cursor.is_none());
    assert_eq!(snap.logical_cursor.as_ref().unwrap().col, 3);
}

#[test]
fn local_scrollback_changes_rendered_viewport() {
    let vt = VTerm::new(10, 3);
    for line in 1..=8 {
        vt.feed(format!("{line}\r\n").as_bytes());
    }
    let live = vt.text_lines();
    assert!(vt.scroll_metrics().0 > 0);
    assert!(vt.scroll_display(2));
    let older = vt.text_lines();
    assert_ne!(older, live);
    assert!(vt.scroll_display(-100));
    assert_eq!(vt.scroll_metrics().1, 0);
}

#[test]
fn selection_copies_selected_cells_and_marks_render_runs() {
    let vt = VTerm::new(20, 4);
    vt.feed(b"hello world");
    vt.begin_selection(0, 0, false);
    vt.update_selection(0, 4, true);
    assert_eq!(vt.selection_to_string().as_deref(), Some("hello"));
    assert!(vt.render_snapshot().rows[0]
        .runs
        .iter()
        .any(|run| run.style.selected && run.text.contains("hello")));
    vt.stop_selection();
    assert!(!vt.selection_active());
}

#[test]
fn vterm_renders_text() {
    let vt = VTerm::new(80, 24);
    vt.feed(b"echo hello\r\nhello world\r\n");
    let lines = vt.text_lines();
    assert_eq!(lines[0], "echo hello");
    assert_eq!(lines[1], "hello world");
}

#[test]
fn vterm_handles_ansi() {
    let vt = VTerm::new(80, 24);
    vt.feed(b"\x1b[31mred text\x1b[0m plain");
    let lines = vt.text_lines();
    assert!(
        lines[0].contains("red text"),
        "ansi colors stripped in text mode: {:?}",
        lines[0]
    );
}

#[test]
fn vterm_cursor_movement() {
    let vt = VTerm::new(10, 5);
    // 移到第 2 行第 3 列写 X
    vt.feed(b"\x1b[2;3HX");
    let lines = vt.text_lines();
    assert!(
        lines[1].starts_with("  X"),
        "cursor positioning works: {:?}",
        lines
    );
}

#[test]
fn vterm_preserves_truecolor_and_cursor() {
    let vt = VTerm::new(20, 5);
    vt.feed(b"\x1b[38;2;255;0;0mRED\x1b[0m");
    let snap = vt.render_snapshot();
    let run = snap.rows[0]
        .runs
        .iter()
        .find(|r| r.text.contains("RED"))
        .expect("RED run");
    assert_eq!(run.style.fg, 0xff0000ff, "truecolor preserved");
    let cursor = snap.cursor.as_ref().expect("cursor visible");
    assert_eq!(cursor.col, 3);
    assert_eq!(cursor.row, 0);
}

#[test]
fn vterm_preserves_background_and_bold() {
    let vt = VTerm::new(20, 5);
    vt.feed(b"\x1b[1;48;2;1;2;3mX\x1b[0m");
    let snap = vt.render_snapshot();
    let run = snap.rows[0]
        .runs
        .iter()
        .find(|r| r.text.contains('X'))
        .unwrap();
    assert!(run.style.bold);
    assert_eq!(run.style.bg, 0x010203ff);
}

#[tokio::test]
async fn pty_input_echo_latency_under_100ms() {
    use muxlane_core::model::AgentType;
    let cfg = muxlane_term::LaunchCfg {
        agent: "shell_latency".into(),
        agent_type: AgentType::Shell,
        cwd: std::env::temp_dir(),
        env: vec![("PS1".into(), "$ ".into())],
        program_override: Some("bash".into()),
        args: vec!["--norc".into(), "--noprofile".into()],
        cols: 80,
        rows: 24,
        tmux_session: None,
    };
    let session = muxlane_term::PtySession::spawn(cfg).unwrap();
    let (_snap, mut rx) = session.subscribe();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    while rx.try_recv().is_ok() {}
    let start = std::time::Instant::now();
    session.write_input(b"x");
    let bytes = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
        .await
        .expect("PTY echo within 100ms")
        .expect("channel alive");
    assert!(bytes.contains(&b'x'));
    assert!(start.elapsed() < std::time::Duration::from_millis(100));
    session.kill();
}

#[test]
fn partial_damage_rebuild_is_fast() {
    let vt = VTerm::new(240, 80);
    // 填满屏幕并建初始缓存。
    for i in 0..200 {
        vt.feed(
            format!(
                "\x1b[38;2;{};{};{}mline-{i:03} {}\x1b[0m\r\n",
                i % 255,
                (i * 3) % 255,
                (i * 7) % 255,
                "x".repeat(180)
            )
            .as_bytes(),
        );
    }
    let _ = vt.render_snapshot();
    // 单字符只 damage 光标行；不应重建 80x240 全网格。
    vt.feed(b"x");
    let start = std::time::Instant::now();
    let snap = vt.render_snapshot();
    assert_eq!(snap.rows.len(), 80);
    assert!(
        start.elapsed() < std::time::Duration::from_millis(50),
        "partial damage snapshot too slow: {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn kill_does_not_block_behind_child_wait() {
    use muxlane_core::model::AgentType;
    let cfg = muxlane_term::LaunchCfg {
        agent: "shell_kill_lock".into(),
        agent_type: AgentType::Shell,
        cwd: std::env::temp_dir(),
        env: vec![],
        program_override: Some("bash".into()),
        args: vec!["-c".into(), "exec 0>&- 1>&- 2>&-; sleep 60".into()],
        cols: 80,
        rows: 24,
        tmux_session: None,
    };
    let session = muxlane_term::PtySession::spawn(cfg).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let start = std::time::Instant::now();
    session.kill();
    assert!(start.elapsed() < std::time::Duration::from_millis(200));
}
