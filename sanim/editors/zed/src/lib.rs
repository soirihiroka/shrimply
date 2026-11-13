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
        let command = binary
            .as_ref()
            .and_then(|binary| binary.path.clone())
            .or_else(|| worktree.which("sanim-lsp"))
            .filter(|path| !path.trim().is_empty())
            .ok_or("Sanim language server not found. Install sanim-lsp on PATH or set lsp.sanim-lsp.binary.path in Zed settings.")?;
        let (args, env) = binary.map_or_else(
            || (Vec::new(), Vec::new()),
            |binary| {
                (
                    binary.arguments.unwrap_or_default(),
                    binary.env.unwrap_or_default().into_iter().collect(),
                )
            },
        );
        Ok(zed::Command { command, args, env })
    }
}

zed::register_extension!(Sanim);
