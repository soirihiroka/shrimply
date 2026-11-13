use std::{
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

fn run(source: &str, arguments: &[&str]) -> (Output, Vec<u8>) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "sanim-cli-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("main script.sanim");
    std::fs::write(&path, source).unwrap();
    let output_path = directory.join(if arguments[0] == "video" {
        "clip.mp4"
    } else {
        "frame.png"
    });
    let mut command = Command::new(env!("CARGO_BIN_EXE_sanim"));
    command.arg(arguments[0]).arg(&path);
    for (flag, value) in [("--width", "32"), ("--height", "32"), ("--dpi", "72")] {
        if arguments[0] != "fmt" && !arguments.contains(&flag) {
            command.args([flag, value]);
        }
    }
    if arguments[0] == "video" && !arguments.contains(&"--fps") {
        command.args(["--fps", "10"]);
    }
    for argument in &arguments[1..] {
        if *argument == "OUTPUT" {
            command.arg(&output_path);
        } else {
            command.arg(argument);
        }
    }
    let output = command.output().unwrap();
    let png = std::fs::read(&output_path).unwrap_or_default();
    std::fs::remove_dir_all(&directory).unwrap();
    (output, png)
}
#[test]
fn check_reports_structured_success() {
    let (output, _) = run("wait(1s)\n", &["check", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["ok"], true);
}
#[test]
fn check_reports_structured_type_errors() {
    let (output, _) = run("let value: Color = 1\n", &["check", "--json"]);
    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(json["diagnostic"]["code"], "S106");
    assert_eq!(json["diagnostic"]["span"]["line"], 1);
    assert_eq!(json["diagnostic"]["severity"], "error");
}

#[test]
fn lint_is_static_and_exposes_unused_expensive_work() {
    let source = "let unused=read<List<Number>>(\"absent.json\")\nwait(1s)";
    let (output, _) = run(source, &["lint", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
    assert!(
        result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "L001" && diagnostic["severity"] == "warning")
    );
    let (denied, _) = run(source, &["lint", "--json", "--deny-warnings"]);
    assert_eq!(denied.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&denied.stdout).unwrap()["ok"],
        false
    );
    let (checked, _) = run(source, &["check", "--json"]);
    assert_eq!(checked.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&checked.stderr).contains("absent.json"));
}

#[test]
fn profile_measures_requested_frames_and_warm_cache_reuse() {
    let (output, bytes) = run(
        "add(Circle(radius=4pt))\nwait(1s)",
        &["profile", "--json", "--samples", "5"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(bytes.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let passes = result["profile"]["passes"].as_array().unwrap();
    assert_eq!(passes[0]["samples"], 5);
    assert_eq!(passes[0]["geometries_prepared"], 1);
    assert_eq!(passes[1]["states_computed"], 0);
    assert_eq!(passes[1]["geometries_prepared"], 0);
    assert_eq!(passes[1]["recording_cache_hits"], 5);
}

#[test]
fn performance_warnings_are_actionable_and_can_fail_check() {
    let (output, _) = run(
        "repeat 3 {let n=sin(1)}\nwait(1s)",
        &[
            "check",
            "--json",
            "--warn-operations",
            "1",
            "--deny-warnings",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert!(
        result["performance"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "P001"
                && warning["severity"] == "warning"
                && warning["span"]["line"].as_u64().unwrap() > 0)
    );
}

#[test]
fn malformed_profile_and_duplicate_options_have_argument_exit_status() {
    for args in [
        vec!["profile", "--samples", "0", "--json"],
        vec!["check", "--dpi", "72", "--dpi", "96", "--json"],
        vec![
            "render", "--output", "OUTPUT", "--time", "1s", "--frame", "0", "--json",
        ],
    ] {
        let (output, bytes) = run("wait(1s)", &args);
        assert_eq!(output.status.code(), Some(2));
        assert!(bytes.is_empty());
        let result: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(result["diagnostic"]["severity"], "error");
    }
}
#[test]
fn render_writes_actual_png() {
    let (output, png) = run(
        "add(Circle(radius= 4pt))\n",
        &["render", "--output", "OUTPUT"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}
#[test]
fn reflected_override_is_validated() {
    let (output, _) = run(
        "let duration: Duration=reflect(1s)\nwait(duration)\n",
        &[
            "check",
            "--set",
            "main script.sanim::duration=\"2s\"",
            "--json",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn invalid_override_fails() {
    let (output, _) = run(
        "let count: Number=reflect(1)\n",
        &["check", "--set", "main script.sanim::count=\"bad\""],
    );
    assert!(!output.status.success());
}
#[test]
fn bounded_watch_produces_initial_revision() {
    let (output, png) = run(
        "add(Circle(radius= 2pt))\n",
        &["watch", "--output", "OUTPUT", "--cycles", "1"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!png.is_empty());
    let lines = String::from_utf8(output.stdout).unwrap();
    assert_eq!(lines.lines().count(), 1);
    let event: serde_json::Value = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
    assert_eq!(event["event"], "updated");
}
#[test]
fn help_is_available_without_script() {
    let output = Command::new(env!("CARGO_BIN_EXE_sanim"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("watch"));
}
#[test]
fn render_requires_output() {
    let (output, _) = run("wait(1s)\n", &["render"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--output")
    );
}
#[test]
fn unknown_option_fails() {
    let (output, _) = run("wait(1s)\n", &["check", "--imaginary", "1"]);
    assert!(!output.status.success());
}
#[test]
fn file_root_blocks_escape() {
    let (output, _) = run("let data = read<String>(\"../outside.json\")\n", &["check"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("escapes")
    );
}
#[test]
fn script_filename_with_spaces() {
    let (output, _) = run("let label = \"hello world\"\n", &["inspect"]);
    assert!(output.status.success());
}

#[test]
fn video_renders_complete_independent_h264_stream() {
    let (output, mp4) = run(
        "let marker=Circle(radius=4pt,fill=Color(\"red\"))\nMoveTo(marker, Vec2(25%,0%), duration=300ms, ease=\"linear\")\n",
        &["video", "--output", "OUTPUT"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(&mp4[4..8], b"ftyp");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["frames"], 3);
}
#[test]
fn video_reports_missing_encoder_and_rejects_invalid_dimensions() {
    let (output, bytes) = run(
        "wait(100ms)",
        &[
            "video",
            "--output",
            "OUTPUT",
            "--ffmpeg",
            "/missing/sanim-ffmpeg",
        ],
    );
    assert!(!output.status.success());
    assert!(bytes.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not start FFmpeg"));
    let (output, _) = run(
        "wait(100ms)",
        &["video", "--output", "OUTPUT", "--width", "15"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("even width"));
}

#[test]
fn cli_duration_reports_exact_time_without_rendering() {
    let (output, bytes) = run("repeat 100000{wait(1ms)}", &["duration", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(bytes.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["seconds"], "100");
}

#[test]
fn cli_defaults_are_external_hd_30fps_and_css_dpi() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("defaults.sanim");
    std::fs::write(&path, "wait(1s)").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sanim"))
        .arg("duration")
        .arg(path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["frames"], 30);
    assert_eq!(result["viewport"]["width"], 1920);
    assert_eq!(result["viewport"]["height"], 1080);
    assert_eq!(result["viewport"]["dpi"], 96);
}

#[test]
fn progress_is_opt_in_structured_stderr_without_changing_results() {
    let (output, _) = run("wait(1s)", &["check", "--json", "--progress"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
    let stderr = String::from_utf8(output.stderr).unwrap();
    let events: Vec<serde_json::Value> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event["event"] == "progress"));
    assert_eq!(events.last().unwrap()["progress"]["state"], "ready");
    let (without, _) = run("wait(1s)", &["check", "--json"]);
    assert!(without.stderr.is_empty());
    let (failed, _) = run("let broken = (", &["check", "--json", "--progress"]);
    assert!(!failed.status.success());
    let stderr = String::from_utf8(failed.stderr).unwrap();
    let events: Vec<serde_json::Value> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events[events.len() - 2]["progress"]["state"], "failed");
    assert_eq!(events.last().unwrap()["ok"], false);
}

#[test]
fn event_watch_reports_repeated_failure_after_recovery_to_same_good_source() {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;
    struct Watch(Child);
    impl Drop for Watch {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.sanim");
    let good = "add(Circle(radius=2pt))\nwait(1s)\n";
    std::fs::write(&source, good).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_sanim"))
        .arg("watch")
        .arg(&source)
        .args([
            "--cycles", "4", "--width", "16", "--height", "16", "--output",
        ])
        .arg(dir.path().join("watch.png"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut child = Watch(child);
    let (sender, receiver) = mpsc::channel();
    let output = std::thread::spawn({
        let sender = sender.clone();
        move || {
            for line in BufReader::new(stdout).lines() {
                sender.send(line.unwrap()).unwrap();
            }
        }
    });
    let errors = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            sender.send(line.unwrap()).unwrap();
        }
    });
    let next = || {
        let line = receiver
            .recv_timeout(Duration::from_secs(8))
            .expect("event-driven watch stopped reporting transitions");
        serde_json::from_str::<serde_json::Value>(&line).unwrap()
    };
    assert_eq!(next()["event"], "updated");
    std::fs::write(&source, "let broken=(\n").unwrap();
    assert_eq!(next()["event"], "failed");
    std::fs::write(&source, good).unwrap();
    assert_eq!(next()["event"], "updated");
    std::fs::write(&source, "let broken=(\n").unwrap();
    assert_eq!(next()["event"], "failed");
    assert!(child.0.wait().unwrap().success());
    output.join().unwrap();
    errors.join().unwrap();
}

#[test]
fn formatter_is_syntax_only_and_check_lint_are_explicit() {
    let source = "let input=read<List<Number>>(\"absent.json\")\nwait(1s)\n";
    let (output, _) = run(source, &["fmt"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = String::from_utf8(output.stdout).unwrap();
    assert!(formatted.contains("let input = read<List<Number>>(\"absent.json\")"));
    let (output, _) = run(source, &["fmt", "--check", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["changed"],
        true
    );
    let (output, _) = run(&formatted, &["fmt", "--check"]);
    assert!(output.status.success());
    let (output, _) = run(
        "let size=12pt\nwait(1s)",
        &["lint", "--check-format", "--deny-warnings", "--json"],
    );
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "F101")
    );
}

#[test]
fn formatter_atomic_write_preserves_permissions_and_rejects_invalid_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("format.sanim");
    std::fs::write(&source, "#123abc\nlet a=1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_sanim"))
        .args(["fmt"])
        .arg(&source)
        .args(["--write", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "#123abc\nlet a = 1\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&source).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    std::fs::write(&source, "let a=(").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sanim"))
        .args(["fmt"])
        .arg(&source)
        .arg("--write")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "let a=(");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn formatter_supports_stdin_and_validates_its_options() {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_sanim"))
        .args(["fmt", "-", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"let value=1\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["formatted"], "let value = 1\n");
    for arguments in [
        ["--write", "--check"],
        ["--indent-width", "0"],
        ["--line-width", "2"],
    ] {
        let (output, _) = run("wait(1s)", &["fmt", arguments[0], arguments[1]]);
        assert_eq!(output.status.code(), Some(2));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_sanim"))
        .args(["fmt", "-", "--write"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
