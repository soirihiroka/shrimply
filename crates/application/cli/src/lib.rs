mod args;
mod render;

const HELP: &str = "Usage: shrimply render PROJECT -o OUTPUT [OPTIONS]

Render a saved .shrimp, .sjson, or .json project without opening the editor.
The saved file is used even if the project is open in another process.

Options:
  -o, --output PATH           Destination file (required)
      --codec CODEC          h264, h265, gif (default: h264, or gif for GIF output)
      --container FORMAT     mp4, mkv, gif (default: output extension)
      --fps RATE             Positive integer, decimal, or ratio such as 30000/1001
                             (default: project frame rate)
      --video-bitrate KBPS   Video bitrate (default: 10000)
      --audio-encoder NAME   aac, fdk-aac, opus; macOS also supports audiotoolbox-aac
                             (default: aac)
      --audio-bitrate KBPS   Audio bitrate (default: 192)
      --audio-sample-rate HZ Audio sample rate (default: 48000)
      --overwrite           Replace an existing output file
  -h, --help                Show this help

Uses the project dimensions and existing GPU export backend. GIF has no audio.
Use -- before a project path beginning with '-'. Bitrates are in Kbps.
Exit codes: 0 success, 1 loading/rendering failure, 2 invalid arguments.";

/// Handle CLI commands before any GUI startup, or leave normal launching alone.
pub fn run_if_requested() -> Option<u8> {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments.next()?;
    if command == "--help" || command == "-h" {
        println!(
            "Usage: shrimply [PROJECT]\n       shrimply render PROJECT -o OUTPUT [OPTIONS]\n\nUse 'shrimply render --help' for rendering options."
        );
        return Some(0);
    }
    if command != "render" {
        return None;
    }
    let arguments: Vec<_> = arguments.collect();
    if arguments
        .iter()
        .take_while(|argument| *argument != "--")
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{HELP}");
        return Some(0);
    }
    let options = match args::parse(arguments) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\nUse 'shrimply render --help' for usage.");
            return Some(2);
        }
    };
    #[cfg(windows)]
    if std::env::var_os("CUDA_MODULE_LOADING").is_none() {
        // Before diagnostics or the renderer can start worker threads.
        unsafe { std::env::set_var("CUDA_MODULE_LOADING", "EAGER") };
    }
    shrimply_process_reporting::diagnostics::init();
    Some(match render::run(options) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}
