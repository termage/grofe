-- Luau declarations for the grofe scripting API (version 1).
-- Point luau-lsp at this file; see docs/book/src/lua.md.

export type SystemResult = {
	code: number,
	stdout: string,
	stderr: string,
}

export type BindOptions = {
	desc: string?,
	["repeat"]: boolean?,
}

export type SystemOptions = {
	cwd: string?,
	timeout_ms: number?,
	env: { [string]: string }?,
	stdin: string?,
	on_line: ((line: string) -> ())?,
	agent: HeadlessAgent?,
}

export type HeadlessAgent = {
	kind: string,
	label: string,
	state: string?,
}

export type OnOptions = {
	pane: string?,
}

export type NotifyOptions = {
	pane: string?,
	level: ("info" | "warn" | "error")?,
}

declare grofe: {
	api: number,
	cmd: (command: { string } | string) -> string,
	fmt: (format: string, target: string?) -> string,
	bind: (table: string, key: string, fn: () -> (), opts: BindOptions?) -> number,
	on: (event: string, fn: (payload: { [string]: any }) -> (), opts: OnOptions?) -> number,
	every: (ms: number, fn: () -> ()) -> number,
	system: (argv: { string }, fn: (result: SystemResult) -> (), opts: SystemOptions?) -> number,
	cancel: (id: number) -> (),
	agent: {
		history: (pane: string, n: number) -> { string },
	},
	log: (msg: string) -> (),
	plugin: (name: string) -> any,
	notify: (text: string, opts: NotifyOptions?) -> (),
	json: {
		encode: (value: any) -> string,
		decode: (text: string) -> any,
	},
	config: (name: string, dir: string?) -> { [string]: any },
	pack_dir: () -> string,
	options: () -> { [string]: any },
	store: {
		get: (key: string) -> any,
		set: (key: string, value: any) -> (),
		delete: (key: string) -> (),
	},
}
