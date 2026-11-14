import { commands, ExtensionContext, workspace } from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';
import { constants } from 'node:fs';
import { access, stat } from 'node:fs/promises';
import { homedir } from 'node:os';
import * as path from 'node:path';

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
    const configuration = workspace.getConfiguration('sanim');
    const configured = configuration.get<string>('server.path', '').trim();
    const suffix = process.platform === 'win32' ? '.exe' : '';
    const directories = (process.env.PATH ?? '').split(path.delimiter).filter(Boolean);
    const installed = [path.join(homedir(), '.local', 'bin', `shrimply${suffix}`)];
    if (process.platform === 'darwin') {
        installed.push('/Applications/Shrimply.app/Contents/MacOS/shrimply', path.join(homedir(), 'Applications/Shrimply.app/Contents/MacOS/shrimply'));
    }
    const candidates = configured
        ? path.isAbsolute(configured) || configured.includes('/') || configured.includes('\\') ? [configured] : directories.map(directory => path.join(directory, process.platform === 'win32' && !path.extname(configured) ? `${configured}.exe` : configured))
        : [...directories.map(directory => path.join(directory, `shrimply${suffix}`)), ...installed, ...directories.map(directory => path.join(directory, `sanim-lsp${suffix}`))];
    let command: string | undefined;
    for (const candidate of candidates) {
        try {
            await access(candidate, constants.X_OK);
            if (!(await stat(candidate)).isFile()) continue;
            command = candidate;
            break;
        } catch (error) {
            if (!['ENOENT', 'ENOTDIR', 'EACCES'].includes((error as NodeJS.ErrnoException).code ?? '')) throw error;
        }
    }
    if (!command) throw new Error('Sanim language server not found. Install Shrimply and enable its CLI, or set sanim.server.path to Shrimply or a standalone sanim-lsp executable.');
    const args = configuration.get<string[]>('server.arguments') ?? (/^shrimply(?:\.exe)?$/i.test(path.basename(command)) ? ['sanim', 'lsp'] : []);
    const serverOptions: ServerOptions = { command, args };
    const watcher = workspace.createFileSystemWatcher('**/*.sanim');
    context.subscriptions.push(watcher);
    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'sanim' }],
        synchronize: { fileEvents: watcher },
        outputChannelName: 'Sanim Language Server',
    };
    client = new LanguageClient('sanim', 'Sanim', serverOptions, clientOptions);
    context.subscriptions.push(workspace.registerTextDocumentContentProvider('sanim-stdlib', {
        provideTextDocumentContent: async () => {
            if (!client) throw new Error('Sanim language server is not running.');
            return client.sendRequest<string>('sanim/stdlib', {});
        },
    }));
    context.subscriptions.push(commands.registerCommand('sanim.restartServer', async () => {
        await client?.restart();
    }));
    await client.start();
}

export async function deactivate(): Promise<void> {
    await client?.dispose();
    client = undefined;
}
