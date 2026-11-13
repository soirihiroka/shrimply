use zed::settings::LspSettings;
use zed_extension_api as zed;

struct Sanim;

impl zed::Extension for Sanim {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let binary = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?.binary;
        let configured = binary
            .as_ref()
            .and_then(|binary| binary.path.clone())
            .filter(|path| !path.trim().is_empty());
        let mut installed = Vec::new();
        if let Some((_, home)) = worktree.shell_env().iter().find(|(key, _)| key == "HOME") {
            installed.push(format!("{home}/.local/bin/shrimply"));
            installed.push(format!("{home}/Applications/Shrimply.app/Contents/MacOS/shrimply"));
        }
        installed.push("/Applications/Shrimply.app/Contents/MacOS/shrimply".into());
        let command = configured
            .or_else(|| worktree.which("shrimply"))
            .or_else(|| installed.iter().find_map(|path| worktree.which(path)))
            .or_else(|| worktree.which("sanim-lsp"))
            .ok_or("Sanim language server not found. Install Shrimply and enable its CLI, or set lsp.sanim-lsp.binary.path to Shrimply or a standalone sanim-lsp executable.")?;
        let name = command.rsplit(['/', '\\']).next().unwrap_or(&command);
        let default_args = if name.eq_ignore_ascii_case("shrimply") || name.eq_ignore_ascii_case("shrimply.exe") {
            vec!["sanim".into(), "lsp".into()]
        } else {
            Vec::new()
        };
        let (args, env) = binary.map_or_else(
            || (default_args.clone(), Vec::new()),
            |binary| {
                (
                    binary.arguments.clone().unwrap_or_else(|| default_args.clone()),
                    binary.env.unwrap_or_default().into_iter().collect(),
                )
            },
        );
        Ok(zed::Command { command, args, env })
    }
}

zed::register_extension!(Sanim);
