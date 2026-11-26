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
}

export type PromptOptions = {
	-- states that end the wait; default { "idle", "blocked", "done" }
	["until"]: (string | { string })?,
	timeout_ms: number?,
}

export type OnOptions = {
	-- a pane target only; a session or window target is an error
	pane: string?,
}

export type NotifyOptions = {
	pane: string?,
	level: ("info" | "warn" | "error")?,
	-- Enter on the tray entry opens grofe.input and sends the text to `pane`
	reply: boolean?,
}

export type ChooseItem = string | { label: string, value: any }

export type ChooseOptions = {
	title: string?,
	-- text shown for the selected item
	preview: ((item: any) -> string)?,
}

-- Client, pane and cwd of the key press; every field is nil outside one.
export type Context = {
	client: string?,
	pane: string?,
	cwd: string?,
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
		prompt: (pane: string, text: string, opts: PromptOptions?, fn: (state: string?) -> ()) -> number,
		read: (pane: string, n: number?) -> string,
	},
	choose: (items: { ChooseItem }, fn: (item: any?) -> (), opts: ChooseOptions?) -> (),
	input: (prompt: string, fn: (text: string?) -> ()) -> (),
	context: () -> Context,
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
