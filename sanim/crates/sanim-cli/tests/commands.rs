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
    command.args(["--width", "32", "--height", "32", "--dpi", "72"]);
    if arguments[0] == "video" {
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
}
#[test]
fn render_writes_actual_png() {
    let (output, png) = run(
        "add(Circle(radius: 4pt))\n",
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
        &["check", "--set", "duration=\"2s\"", "--json"],
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
        &["check", "--set", "count=\"bad\""],
    );
    assert!(!output.status.success());
}
#[test]
fn bounded_watch_produces_initial_revision() {
    let (output, png) = run(
        "add(Circle(radius: 2pt))\n",
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
        "let marker=Circle(radius:4pt,fill:Color(\"red\"))\nplay(MoveTo(marker,Vec2(25%,0%)),duration:300ms)\n",
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
