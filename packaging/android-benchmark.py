"""Capture a bounded Android debug benchmark and its platform frame statistics."""
import pathlib
import json
import shlex
import selectors
import subprocess
import sys
import time

if len(sys.argv) > 1 and sys.argv[1] == "throughput":
    project, scene, frames = sys.argv[2:]
    if int(frames) < 2:
        raise SystemExit("FRAMES must be at least two")
    package = "dev.shrimply.app"
    def shell(arguments):
        return subprocess.run(["adb", "shell", shlex.join(arguments)], capture_output=True, text=True)
    shell(["am", "force-stop", package]).check_returncode()
    shell(["input", "keyevent", "KEYCODE_WAKEUP"]).check_returncode()
    shell(["wm", "dismiss-keyguard"]).check_returncode()
    shell(["run-as", package, "rm", "-f", "cache/pipeline-throughput.json"]).check_returncode()
    shell(["am", "start", "-W", "-n", f"{package}/.MainActivity", "--es", "throughput_project", project,
        "--es", "throughput_scene", scene, "--ei", "throughput_frames", frames]).check_returncode()
    deadline = time.monotonic() + 240
    while time.monotonic() < deadline:
        result = shell(["run-as", package, "cat", "cache/pipeline-throughput.json"])
        if result.returncode == 0:
            try:
                report = json.loads(result.stdout)
            except json.JSONDecodeError:
                time.sleep(0.5)
                continue
            if not report["ok"]:
                raise SystemExit(report["error"])
            output = pathlib.Path("target") / f"android-throughput-{scene}.txt"
            output.parent.mkdir(exist_ok=True)
            output.write_text(report["report"])
            print(report["report"], end="")
            raise SystemExit(0)
        if not shell(["pidof", package]).stdout.strip():
            raise SystemExit("Throughput benchmark process exited")
        time.sleep(0.5)
    raise SystemExit("Throughput benchmark timed out")

stress = sys.argv[1] if len(sys.argv) > 1 else "false"
if stress not in {"true", "false"}:
    raise SystemExit("STRESS must be true or false")
output = pathlib.Path("target") / f"android-benchmark-{'stress' if stress == 'true' else 'normal'}.txt"
output.parent.mkdir(parents=True, exist_ok=True)
subprocess.run(["adb", "shell", "am", "force-stop", "dev.shrimply.app"], check=True)
subprocess.run(["adb", "shell", "am", "start", "-W", "-n", "dev.shrimply.app/.MainActivity", "--ez", "benchmark", "true", "--ez", "stress", stress], check=True)
pid = subprocess.check_output(["adb", "shell", "pidof", "-s", "dev.shrimply.app"], text=True).strip()
if not pid.isdecimal():
    raise SystemExit("Shrimply did not start")
process = subprocess.Popen(["adb", "logcat", "-v", "raw", "-T", "1", "--pid", pid, "-s", "ShrimplyTimeline:I", "AndroidRuntime:E"], stdout=subprocess.PIPE, bufsize=0)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
deadline = time.monotonic() + 180
try:
    with output.open("w") as report:
        while time.monotonic() < deadline:
            if not selector.select(timeout=1):
                continue
            line = process.stdout.readline().decode("utf-8", errors="replace")
            print(line, end="", flush=True)
            if "native editor panic" in line or "FATAL EXCEPTION" in line:
                raise SystemExit("Android benchmark crashed")
            if line.startswith(("benchmark frames=", "frame_interval_p50_us=", "scene_record_p50_us=")):
                report.write(line)
                report.flush()
            if "rendering idle" in line:
                print(f"Saved {output}")
                break
        else:
            raise SystemExit("Android benchmark timed out; check the device and logcat")
finally:
    process.terminate()
    process.wait(timeout=10)
    selector.close()
