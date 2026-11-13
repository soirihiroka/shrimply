import { commands, ExtensionContext, workspace } from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
    const command = workspace.getConfiguration('sanim').get<string>('server.path', 'sanim-lsp');
    const serverOptions: ServerOptions = { command, args: [] };
    const watcher = workspace.createFileSystemWatcher('**/*.sanim');
    context.subscriptions.push(watcher);
    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'sanim' }],
        synchronize: { fileEvents: watcher },
        outputChannelName: 'Sanim Language Server',
    };
    client = new LanguageClient('sanim', 'Sanim', serverOptions, clientOptions);
    context.subscriptions.push(commands.registerCommand('sanim.restartServer', async () => {
        await client?.restart();
    }));
    await client.start();
}

export async function deactivate(): Promise<void> {
    await client?.dispose();
    client = undefined;
}
