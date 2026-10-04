interface Api {
	name(name: string): void
	template(fn: (args: unknown[], opts: { compileCommands(c: string[]): string[] }) => string | string[]): void
}

export default ({ name, template }: Api) => {
	name('boom')
	template((args, { compileCommands }) => [
		'particle minecraft:explosion ~ ~ ~',
		...compileCommands(['greet boom 2', '/tp @s ~ ~1 ~']),
		`# args: ${JSON.stringify(args)}`,
	])
}
