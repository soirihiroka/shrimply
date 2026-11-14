"""Capture a bounded Android debug benchmark and its platform frame statistics."""
import pathlib
import selectors
import subprocess
import sys
import time

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
