mod media;
mod server;
mod store;

use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), String> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() == 1 && arguments[0] == "--version" {
        println!("shrimply-project-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let [address, database]: [std::ffi::OsString; 2] = arguments
        .try_into()
        .map_err(|_| "usage: shrimply-project-server LISTEN_ADDRESS DATABASE_PATH")?;
    let address = address
        .to_str()
        .ok_or("listen address must be UTF-8")?
        .parse()
        .map_err(|error| format!("invalid listen address: {error}"))?;
    let token = std::env::var("SHRIMPLY_PROJECT_SERVER_TOKEN")
        .map_err(|_| "set SHRIMPLY_PROJECT_SERVER_TOKEN")?;
    server::serve(address, Path::new(&database), &token).await
}
