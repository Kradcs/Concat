// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
use super::*;
use concat_project::editor::Editor;

// A native subprocess exercises the same argument, stdin and file protocol on
// Windows and Unix. It never contacts OpenAI or uses a real credential.
fn fake() -> (tempfile::TempDir, Codex) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("fake.rs");
    let executable = dir.path().join(if cfg!(windows) {
        "fake codex.exe"
    } else {
        "fake codex"
    });
    fs::write(&source, r###"
use std::{env,fs,io::{self,Read},path::PathBuf};
fn main() {
    let args:Vec<String> = env::args().skip(1).collect();
    if args.contains(&"--hang".into()) {
        std::thread::sleep(std::time::Duration::from_secs(10)); return;
    }
    let home=PathBuf::from(env::var_os("CODEX_HOME").unwrap());
    let marker=home.join("test-signed-in");
    assert!(env::var_os("OPENAI_API_KEY").is_none());
    assert!(env::var_os("CODEX_API_KEY").is_none());
    for flag in ["forced_login_method=\"chatgpt\"", "features.shell_tool=false",
                 "features.request_permissions_tool=false", "sandbox_mode=\"read-only\""] {
        assert!(args.iter().any(|v| v==flag));
    }
    if args.contains(&"login".into()) {
        if args.contains(&"status".into()) {
            if marker.exists() { eprintln!("Logged in using ChatGPT"); }
            else { std::process::exit(1); }
        } else { fs::write(marker, "test-only").unwrap(); }
    } else if args.contains(&"logout".into()) {
        fs::remove_file(marker).unwrap();
    } else {
        for flag in ["--ephemeral", "--ignore-user-config", "--ignore-rules", "--strict-config"] {
            assert!(args.contains(&flag.into()));
        }
        let mut input=String::new(); io::stdin().read_to_string(&mut input).unwrap();
        assert!(input.contains("selectedClipIds"));
        assert!(input.contains("addTextClip"));
        assert!(!args.iter().any(|v| v.contains("Adicione")));
        let schema=&args[args.iter().position(|v| v=="--output-schema").unwrap()+1];
        assert!(fs::read_to_string(schema).unwrap().contains("commandsJson"));
        let output=&args[args.iter().position(|v| v=="--output-last-message").unwrap()+1];
        let response=if input.contains("invalid-plan") {
            r##"{"message":"invalid","commandsJson":"[{\"op\":\"removeMedia\",\"mediaId\":\"m1\"}]"}"##
        } else if input.contains("service-failure") {
            std::process::exit(1);
        } else {
            r##"{"message":"Título proposto","commandsJson":"[{\"op\":\"addTextClip\",\"trackId\":null,\"start\":0.0,\"style\":null,\"duration\":3.0}]"}"##
        };
        fs::write(output,response).unwrap();
    }
}
"###).unwrap();
    let status = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success());
    let codex = Codex {
        executable,
        home: dir.path().join("private home"),
    };
    codex.prepare().unwrap();
    (dir, codex)
}

#[test]
fn chatgpt_login_edit_review_apply_undo_and_logout() {
    let (_dir, codex) = fake();
    let cancel = AtomicBool::new(false);
    assert!(!codex.signed_in(&cancel).unwrap());
    assert!(
        codex
            .propose("Adicione título", &json!({}), &cancel)
            .is_err()
    );
    codex.login(&cancel).unwrap();
    assert!(codex.signed_in(&cancel).unwrap());
    let mut editor = Editor::new();
    let before = editor.project().clone();
    let plan = codex
        .propose(
            "Adicione um título; $(this is literal user text)",
            &json!({"selectedClipIds":[], "playheadSeconds":0.0}),
            &cancel,
        )
        .unwrap();
    assert_eq!(editor.project(), &before);
    assert_eq!(plan.commands.len(), 1);
    editor.apply(plan.into_command()).unwrap();
    assert_eq!(editor.project().active().clips[0].duration, 3.0);
    assert!(editor.undo());
    assert_eq!(editor.project(), &before);
    assert!(!fs::read_dir(&codex.home).unwrap().any(|f| {
        f.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("request-")
    }));
    codex.logout(&cancel).unwrap();
    assert!(!codex.signed_in(&cancel).unwrap());
}

#[test]
fn codex_failures_and_unsupported_commands_never_produce_a_plan() {
    let (_dir, codex) = fake();
    let cancel = AtomicBool::new(false);
    codex.login(&cancel).unwrap();
    for request in ["invalid-plan", "service-failure"] {
        assert!(
            codex
                .propose(request, &json!({"selectedClipIds":[]}), &cancel)
                .is_err()
        );
    }
}

#[test]
fn hung_processes_time_out_and_can_be_cancelled() {
    let (_dir, codex) = fake();
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let command = || {
        let mut command = Command::new(&codex.executable);
        command.arg("--hang");
        command
    };
    let start = Instant::now();
    assert!(run(command(), None, Duration::from_millis(80), &cancel).is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
    let signal = cancel.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        signal.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    assert!(
        run(command(), None, Duration::from_secs(10), &cancel)
            .unwrap_err()
            .contains("cancelada")
    );
    assert!(start.elapsed() < Duration::from_secs(3));
    worker.join().unwrap();
}

#[test]
fn pinned_official_cli_accepts_config_and_requires_login() {
    // Set by CI to the checksum-verified native Codex package. No account login
    // is attempted in automation. The subprocess tests above always run.
    let Some(executable) = std::env::var_os("CONCAT_TEST_CODEX") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let codex = Codex {
        executable: executable.into(),
        home: dir.path().join("codex"),
    };
    codex.prepare().unwrap();
    let runtime = codex.executable.parent().unwrap().join(if cfg!(windows) {
        "codex-code-mode-host.exe"
    } else {
        "codex-code-mode-host"
    });
    assert!(
        runtime.is_file(),
        "The official native model runtime must ship beside Codex"
    );
    assert!(
        Command::new(runtime)
            .arg("--help")
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut command = codex.command();
    command.args(["features", "list"]);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing = String::from_utf8_lossy(&output.stdout);
    for feature in [
        "shell_tool",
        "request_permissions_tool",
        "browser_use",
        "apps",
    ] {
        assert!(
            listing
                .lines()
                .any(|line| line.split_whitespace().next() == Some(feature)
                    && line.ends_with("false"))
        );
    }
    assert!(!codex.signed_in(&AtomicBool::new(false)).unwrap());
}
