import * as path from 'path';
import { workspace, ExtensionContext } from 'vscode';

import {
	Executable,
	LanguageClient,
	LanguageClientOptions,
	Middleware,
	ServerOptions,
	Trace,
	TransportKind
} from 'vscode-languageclient/node';

let client: LanguageClient;

export function activate(context: ExtensionContext) {
	const serverModule = context.asAbsolutePath(
		path.join('server', 'target', 'debug', 'server.exe')
	);

	const serverOptions: ServerOptions = {
		run: { 
			command: serverModule, 
			transport: TransportKind.stdio,
			options: { shell: true, detached: true }
		},
		debug: {
			command: serverModule,
			transport: TransportKind.stdio
		}
	};

	const clientOptions: LanguageClientOptions = {
		documentSelector: [{ scheme: 'file', pattern: '**/*.desc' }],
		synchronize: {
			fileEvents: workspace.createFileSystemWatcher('**/.clientrc')
		}
	};

	client = new LanguageClient(
		'DescendServer',
		'Descent Server',
		serverOptions,
		clientOptions
	);

	client.setTrace(Trace.Verbose);
	client.start();
}

export function deactivate(): Thenable<void> | undefined {
	if (!client) {
		return undefined;
	}
	return client.stop();
}
